use std::sync::atomic::Ordering;

use camino::{Utf8Path, Utf8PathBuf};
use ignore::overrides::OverrideBuilder;
use ignore::{DirEntry, WalkBuilder};
use tracing::warn;

use crate::config::{BackupSettings, DEFAULT_EXCLUDES};
use crate::error::{Error, Result};
use crate::tui::state::ProgressState;
use crate::walk::gitmeta;
use crate::walk::{DiscoverReport, Project};

/// True for `target/` directories whose sibling has a `Cargo.toml` (cargo build artifact).
fn is_rust_target(entry: &DirEntry) -> bool {
    if entry.file_name() != "target" {
        return false;
    }
    if !entry.file_type().map(|t| t.is_dir()).unwrap_or(false) {
        return false;
    }
    entry
        .path()
        .parent()
        .map(|p| p.join("Cargo.toml").is_file())
        .unwrap_or(false)
}

/// Find every git project under `target` (skip subtrees beneath each .git).
/// updates `state.projects_found` as projects are discovered.
pub fn find_projects(target: &Utf8Path, state: &ProgressState) -> Vec<Project> {
    let (tx, rx) = crossbeam_channel::unbounded::<Project>();
    let target_owned = target.to_path_buf();
    let tx_clone = tx.clone();
    let state_clone = state.clone();

    let walker = WalkBuilder::new(target.as_std_path())
        .standard_filters(false)
        .hidden(false)
        .follow_links(false)
        .filter_entry(move |entry: &DirEntry| {
            if is_rust_target(entry) {
                return false;
            }
            let is_git = entry.file_name() == ".git"
                && entry.file_type().map(|t| t.is_dir()).unwrap_or(false);
            if is_git {
                if let Some(parent) = entry.path().parent() {
                    if let Some(utf8parent) = Utf8Path::from_path(parent) {
                        match gitmeta::gather(utf8parent, &target_owned) {
                            Ok(p) => {
                                state_clone.projects_found.fetch_add(1, Ordering::Relaxed);
                                let _ = tx_clone.send(p);
                            }
                            Err(e) => warn!("gitmeta gather failed: {e}"),
                        }
                    }
                }
                return false;
            }
            true
        })
        .build();

    for r in walker {
        if let Err(e) = r {
            warn!("walk error during project discovery: {e}");
        }
    }
    drop(tx);
    rx.into_iter().collect()
}

/// Build the configured walker (overrides + .bupignore + .git skip).
pub fn build_walker(
    target: &Utf8Path,
    settings: &BackupSettings,
) -> Result<ignore::Walk> {
    let mut wb = WalkBuilder::new(target.as_std_path());
    wb.standard_filters(false)
        .git_ignore(false)
        .git_global(false)
        .git_exclude(false)
        .ignore(false)
        .parents(false)
        .hidden(false)
        .follow_links(false)
        .add_custom_ignore_filename(".bupignore");

    let mut ob = OverrideBuilder::new(target.as_std_path());
    ob.add("!.git/")
        .map_err(|e| Error::Walk(format!("override .git/: {e}")))?;
    ob.add("!**/.git/")
        .map_err(|e| Error::Walk(format!("override **/.git/: {e}")))?;

    if !settings.no_default_excludes {
        for pat in DEFAULT_EXCLUDES {
            ob.add(&format!("!{pat}"))
                .map_err(|e| Error::Walk(format!("override {pat}: {e}")))?;
        }
    }
    for pat in &settings.extra_excludes {
        let p = if pat.starts_with('!') { pat.clone() } else { format!("!{pat}") };
        ob.add(&p).map_err(|e| Error::Walk(format!("override {pat}: {e}")))?;
    }

    let ov = ob.build().map_err(|e| Error::Walk(format!("override build: {e}")))?;
    wb.overrides(ov);
    wb.filter_entry(|e| !is_rust_target(e));
    Ok(wb.build())
}

/// First pass: count files + sum bytes, gather projects.
pub fn run(
    target: &Utf8Path,
    settings: &BackupSettings,
    state: &ProgressState,
) -> Result<DiscoverReport> {
    let projects = find_projects(target, state);

    let walker = build_walker(target, settings)?;
    let mut total_files = 0u64;
    let mut total_bytes = 0u64;

    for r in walker {
        let entry = match r {
            Ok(e) => e,
            Err(e) => {
                warn!("walk error: {e}");
                state.errors_skipped.fetch_add(1, Ordering::Relaxed);
                continue;
            }
        };
        let ft = match entry.file_type() {
            Some(t) => t,
            None => continue,
        };
        if !ft.is_file() || ft.is_symlink() {
            continue;
        }
        let md = match entry.metadata() {
            Ok(m) => m,
            Err(e) => {
                warn!("metadata: {e}");
                state.errors_skipped.fetch_add(1, Ordering::Relaxed);
                continue;
            }
        };
        total_files += 1;
        total_bytes += md.len();
        state.bytes_scanned.fetch_add(md.len(), Ordering::Relaxed);
    }

    state.bytes_total.store(total_bytes, Ordering::Relaxed);
    state
        .files_total
        .store(total_files + projects.len() as u64, Ordering::Relaxed);
    Ok(DiscoverReport {
        total_files,
        total_bytes,
        projects,
    })
}

/// Lookup whether a file is tracked by some project's git index.
pub fn is_tracked(projects: &[Project], abs: &Utf8Path) -> bool {
    projects.iter().any(|p| {
        abs.strip_prefix(&p.root_abs)
            .map(|rel| p.tracked.contains(&Utf8PathBuf::from(rel)))
            .unwrap_or(false)
    })
}
