use std::fs::{self, File};
use std::io::{self, BufWriter, IsTerminal, Write};
use std::process::ExitCode;
use std::sync::atomic::Ordering;

use age::secrecy::SecretString;
use camino::Utf8PathBuf;
use clap::Parser;
use tracing::{info, warn};
use tracing_subscriber::EnvFilter;

mod archive;
mod cli;
mod config;
mod error;
mod hashing;
mod restore;
mod tui;
mod util;
mod walk;

use crate::archive::{
    ArchiveStats, ArchiveWriter, CountingWriter, LargeFileEntry, ReviewOutcome, ReviewProvider,
};
use crate::cli::{BackupArgs, Cli, Commands, RestoreArgs};
use crate::config::{
    BackupSettings, LargeFilePolicy, RestoreSettings, PASSPHRASE_ENV, WRITE_BUFFER_BYTES,
};
use crate::error::{Error, Result};
use crate::tui::state::{Phase, ProgressState};
use crate::tui::{spawn as spawn_tui, PlainProgress, TuiHandle};
use crate::util::{
    append_partial_suffix, default_output_name, ensure_output_outside_target, human_bytes,
    human_duration, normalize_exclude,
};
use crate::walk::{DiscoverReport, WalkEvent};

fn main() -> ExitCode {
    let cli = Cli::parse();
    init_tracing(cli.verbose);
    let result = match cli.command {
        Commands::Backup(a) => run_backup(a),
        Commands::Restore(a) => run_restore(a),
    };
    match result {
        Ok(()) => ExitCode::SUCCESS,
        Err(e) => {
            eprintln!("error: {e}");
            ExitCode::FAILURE
        }
    }
}

fn init_tracing(verbose: u8) {
    let level = match verbose {
        0 => "warn",
        1 => "info",
        2 => "debug",
        _ => "trace",
    };
    let filter = EnvFilter::try_from_default_env()
        .unwrap_or_else(|_| EnvFilter::new(format!("minbup={level}")));
    tracing_subscriber::fmt()
        .with_env_filter(filter)
        .with_writer(std::io::stderr)
        .init();
}

fn run_backup(args: BackupArgs) -> Result<()> {
    let (settings, excluded_subtrees) = build_backup_settings(args)?;
    ensure_output_outside_target(
        settings.output.as_std_path(),
        settings.target.as_std_path(),
        &excluded_subtrees,
    )?;

    let state = ProgressState::new();
    state.set_phase(Phase::Discover);

    let state_for_signal = state.clone();
    let _ = ctrlc::set_handler(move || state_for_signal.signal_abort());

    let interactive = tui::is_tty();
    let mut tui_handle = if interactive {
        Some(spawn_tui(state.clone())?)
    } else {
        None
    };
    let plain = if !interactive {
        Some(PlainProgress::spawn(state.clone()))
    } else {
        None
    };

    info!("scanning {}", settings.target);
    let report = walk::discover::run(&settings.target, &settings, &state)?;
    info!(
        "discover: {} file(s), {} ({} project(s))",
        report.total_files,
        human_bytes(report.total_bytes),
        report.projects.len()
    );

    state.set_phase(Phase::Stream);

    let partial = append_partial_suffix(&settings.output);
    let stats_result = open_and_run(&settings, &report, &state, &partial, tui_handle.as_mut());

    match stats_result {
        Ok(stats) => {
            state.set_phase(Phase::Done);
            fs::rename(partial.as_std_path(), settings.output.as_std_path())?;
            shutdown_progress(tui_handle, plain);
            let elapsed = state.start.elapsed();
            print_backup_summary(&settings, &stats, report.projects.len(), elapsed);
            Ok(())
        }
        Err(e) => {
            shutdown_progress(tui_handle, plain);
            let _ = fs::remove_file(partial.as_std_path());
            Err(e)
        }
    }
}

fn shutdown_progress(tui: Option<TuiHandle>, plain: Option<PlainProgress>) {
    if let Some(t) = tui {
        if let Err(e) = t.shutdown() {
            warn!("tui shutdown: {e}");
        }
    }
    if let Some(p) = plain {
        p.shutdown();
    }
}

fn build_backup_settings(
    args: BackupArgs,
) -> Result<(BackupSettings, Vec<std::path::PathBuf>)> {
    let target = args
        .target
        .canonicalize_utf8()
        .map_err(|e| Error::Config(format!("canonicalize target: {e}")))?;
    let output = match args.output {
        Some(o) => o,
        None => default_output_name(&target, args.encrypt),
    };
    let zstd_workers = args
        .zstd_workers
        .unwrap_or_else(|| (num_cpus::get() as u32).min(4));

    let normalized = args
        .exclude
        .iter()
        .filter_map(|raw| normalize_exclude(&target, raw))
        .collect::<Vec<_>>();
    let extra_excludes = normalized.iter().map(|n| n.pattern.clone()).collect();
    let excluded_subtrees = normalized
        .into_iter()
        .filter_map(|n| n.canonical_path)
        .collect();

    Ok((
        BackupSettings {
            target,
            output,
            encrypt: args.encrypt,
            large_threshold: args.large_threshold,
            zstd_level: args.zstd_level,
            zstd_workers,
            large_files: args.large_files.into(),
            no_default_excludes: args.no_default_excludes,
            extra_excludes,
        },
        excluded_subtrees,
    ))
}

fn open_and_run(
    settings: &BackupSettings,
    report: &DiscoverReport,
    state: &ProgressState,
    partial: &Utf8PathBuf,
    review: Option<&mut TuiHandle>,
) -> Result<ArchiveStats> {
    let f = File::create(partial.as_std_path())?;
    let buf = BufWriter::with_capacity(WRITE_BUFFER_BYTES, f);
    let counting = CountingWriter::new(buf, state.bytes_archived_compressed.clone());

    if settings.encrypt {
        let secret = read_passphrase()?;
        let enc = age::Encryptor::with_user_passphrase(secret);
        let stream = enc
            .wrap_output(counting)
            .map_err(|e| Error::Encrypt(format!("wrap: {e}")))?;
        let zstd_enc = make_zstd_encoder(stream, settings)?;
        let aw = ArchiveWriter::new(zstd_enc, state.clone());
        let (inner, stats) = run_pipeline(aw, settings, report, state, review)?;
        let counting_back = inner
            .finish()
            .map_err(|e| Error::Encrypt(format!("age finish: {e}")))?;
        finalize_file(counting_back)?;
        Ok(stats)
    } else {
        let zstd_enc = make_zstd_encoder(counting, settings)?;
        let aw = ArchiveWriter::new(zstd_enc, state.clone());
        let (counting_back, stats) = run_pipeline(aw, settings, report, state, review)?;
        finalize_file(counting_back)?;
        Ok(stats)
    }
}

fn make_zstd_encoder<W: Write>(
    w: W,
    settings: &BackupSettings,
) -> Result<zstd::Encoder<'static, W>> {
    let mut enc = zstd::Encoder::new(w, settings.zstd_level)
        .map_err(|e| Error::Archive(format!("zstd init: {e}")))?;
    if settings.zstd_workers > 0 {
        enc.multithread(settings.zstd_workers)
            .map_err(|e| Error::Archive(format!("zstd multithread: {e}")))?;
    }
    Ok(enc)
}

fn finalize_file(c: CountingWriter<BufWriter<File>>) -> Result<()> {
    let mut buf = c.into_inner();
    buf.flush()?;
    let file = buf.into_inner().map_err(|e| Error::Io(e.into_error()))?;
    file.sync_all()?;
    Ok(())
}

fn read_passphrase() -> Result<SecretString> {
    if let Ok(s) = std::env::var(PASSPHRASE_ENV) {
        return Ok(SecretString::from(s));
    }
    if !io::stdin().is_terminal() {
        return Err(Error::Config(format!(
            "stdin is not a tty; set {PASSPHRASE_ENV} to the passphrase"
        )));
    }
    let p = rpassword::prompt_password("passphrase: ")
        .map_err(|e| Error::Encrypt(format!("read passphrase: {e}")))?;
    if p.is_empty() {
        return Err(Error::Encrypt("empty passphrase".into()));
    }
    Ok(SecretString::from(p))
}

fn run_pipeline<W: Write>(
    mut aw: ArchiveWriter<W>,
    settings: &BackupSettings,
    report: &DiscoverReport,
    state: &ProgressState,
    review: Option<&mut TuiHandle>,
) -> Result<(W, ArchiveStats)> {
    for project in &report.projects {
        if let Err(e) = aw.add_gitmeta(project) {
            warn!("write .gitmeta for {}: {e}", project.root_rel);
            state.errors_skipped.fetch_add(1, Ordering::Relaxed);
        }
    }

    let (tx, rx) = crossbeam_channel::bounded::<WalkEvent>(config::CHANNEL_CAPACITY);
    let target = settings.target.clone();
    let settings_clone = settings.clone();
    let projects = report.projects.clone();
    let state_for_walker = state.clone();
    let walker_join = std::thread::Builder::new()
        .name("minbup-walker".into())
        .spawn(move || {
            walk::stream::run(&target, &settings_clone, &projects, tx, &state_for_walker)
        })
        .map_err(|e| Error::Other(anyhow::anyhow!("spawn walker: {e}")))?;

    let mut large_queue = Vec::<LargeFileEntry>::new();

    while let Ok(ev) = rx.recv() {
        if state.is_aborted() {
            return Err(Error::UserAbort);
        }
        match ev {
            WalkEvent::Small { abs, rel, size, mtime } => {
                state.set_current_path(abs.as_str());
                aw.add_file(&abs, rel.as_std_path(), mtime, size)?;
            }
            WalkEvent::Large { abs, rel, size, mtime, tracked_by_git } => {
                large_queue.push(LargeFileEntry { abs, rel, size, mtime, tracked_by_git });
            }
            WalkEvent::EndOfSmall => break,
        }
    }
    if state.is_aborted() {
        return Err(Error::UserAbort);
    }

    walker_join
        .join()
        .map_err(|_| Error::Walk("walker thread panicked".into()))??;

    let kept_indices: Vec<usize> = if large_queue.is_empty() {
        Vec::new()
    } else {
        state.set_phase(Phase::Review);
        match settings.large_files {
            LargeFilePolicy::SkipAll => Vec::new(),
            LargeFilePolicy::KeepAll => (0..large_queue.len()).collect(),
            LargeFilePolicy::Prompt => match review {
                Some(h) => match h.decide(&large_queue) {
                    ReviewOutcome::SkipAll => Vec::new(),
                    ReviewOutcome::KeepAll => (0..large_queue.len()).collect(),
                    ReviewOutcome::KeepSelected(v) => v,
                },
                None => {
                    warn!("no review provider available; defaulting to keep-all");
                    (0..large_queue.len()).collect()
                }
            },
        }
    };

    if !kept_indices.is_empty() {
        state.set_phase(Phase::StreamLarge);
        for i in kept_indices {
            if state.is_aborted() {
                return Err(Error::UserAbort);
            }
            let e = &large_queue[i];
            state.set_current_path(e.abs.as_str());
            aw.add_file(&e.abs, e.rel.as_std_path(), e.mtime, e.size)?;
        }
    }

    state.set_phase(Phase::Manifest);
    let (inner, stats) = aw.finalize()?;
    Ok((inner, stats))
}

fn print_backup_summary(
    settings: &BackupSettings,
    stats: &ArchiveStats,
    projects: usize,
    elapsed: std::time::Duration,
) {
    let on_disk = std::fs::metadata(settings.output.as_std_path()).map(|m| m.len()).ok();
    eprintln!();
    eprintln!("✓ {}", settings.output);
    eprintln!("  files     {}", stats.files);
    eprintln!("  projects  {}", projects);
    eprintln!(
        "  source    {}  (extracted size)",
        human_bytes(stats.uncompressed_bytes)
    );
    if let Some(d) = on_disk {
        let ratio = if stats.uncompressed_bytes > 0 {
            d as f64 / stats.uncompressed_bytes as f64
        } else {
            0.0
        };
        eprintln!(
            "  archive   {}  ({:.0}% of source)",
            human_bytes(d),
            ratio * 100.0
        );
    }
    eprintln!("  elapsed   {}", human_duration(elapsed));
}

fn run_restore(args: RestoreArgs) -> Result<()> {
    let settings = RestoreSettings {
        archive: args.archive,
        dest: args.dest,
        no_git_rehydrate: args.no_git_rehydrate,
        full_history: args.full_history,
        skip_verify: args.skip_verify,
    };
    fs::create_dir_all(settings.dest.as_std_path())?;

    let passphrase = if is_age(&settings.archive) {
        Some(read_passphrase()?)
    } else {
        None
    };

    info!("extracting {} → {}", settings.archive, settings.dest);
    let mut summary = restore::extract::extract(
        &settings.archive,
        &settings.dest,
        passphrase,
        settings.skip_verify,
    )?;

    if !settings.no_git_rehydrate {
        info!("rehydrating git projects");
        restore::rehydrate::rehydrate_all(&settings.dest, settings.full_history, &mut summary)?;
    }

    eprintln!();
    eprintln!("✓ restored to {}", settings.dest);
    eprintln!(
        "  {} file(s), {}",
        summary.files_extracted,
        human_bytes(summary.bytes_extracted)
    );
    if !settings.skip_verify {
        eprintln!("  manifest entries verified: {}", summary.manifest_entries);
    }
    if !settings.no_git_rehydrate {
        eprintln!(
            "  git projects: {} rehydrated, {} failed",
            summary.projects_rehydrated, summary.projects_failed
        );
    }
    if summary.projects_failed > 0 {
        return Err(Error::Restore(format!(
            "{} project(s) failed to rehydrate",
            summary.projects_failed
        )));
    }
    Ok(())
}

fn is_age(p: &Utf8PathBuf) -> bool {
    p.as_str().ends_with(".age")
}
