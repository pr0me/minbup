use camino::Utf8PathBuf;
use clap::{Args, Parser, Subcommand, ValueEnum};

use crate::config::{LargeFilePolicy, LARGE_FILE_THRESHOLD_DEFAULT, ZSTD_LEVEL_DEFAULT};

#[derive(Debug, Parser)]
#[command(name = "minbup", version, about = "minimal migration backup tool", long_about = None)]
pub struct Cli {
    #[command(subcommand)]
    pub command: Commands,

    /// Verbose logging (-v info, -vv debug, -vvv trace).
    #[arg(short, long, action = clap::ArgAction::Count, global = true)]
    pub verbose: u8,
}

#[derive(Debug, Subcommand)]
pub enum Commands {
    /// Scan a directory and produce a single compressed (optionally encrypted) archive.
    Backup(BackupArgs),
    /// Decompress an archive and reconstitute git projects via .gitmeta sidecars.
    Restore(RestoreArgs),
}

#[derive(Debug, Args)]
pub struct BackupArgs {
    /// Directory to back up.
    pub target: Utf8PathBuf,

    /// Output archive path. Defaults to ./<target-name>-<timestamp>.tar.zst[.age].
    #[arg(short, long)]
    pub output: Option<Utf8PathBuf>,

    /// Encrypt with a passphrase via the age cipher.
    #[arg(long)]
    pub encrypt: bool,

    /// Skip the bundled default exclusion patterns.
    #[arg(long)]
    pub no_default_excludes: bool,

    /// Extra exclusion patterns. accepts gitignore-style globs (e.g. `*.bak`) or
    /// filesystem paths (e.g. `../binarly`, `./vendor`) that resolve inside the target.
    /// May be passed multiple times.
    #[arg(short = 'e', long = "exclude", value_name = "PATTERN")]
    pub exclude: Vec<String>,

    /// Threshold above which a file is queued for review (in bytes).
    #[arg(long, default_value_t = LARGE_FILE_THRESHOLD_DEFAULT)]
    pub large_threshold: u64,

    /// Zstd compression level (1..=22).
    #[arg(long, default_value_t = ZSTD_LEVEL_DEFAULT)]
    pub zstd_level: i32,

    /// Number of zstd worker threads (0 = single-threaded).
    #[arg(long)]
    pub zstd_workers: Option<u32>,

    /// Behavior for large files when stdin is not a TTY (or always).
    #[arg(long, value_enum, default_value_t = LargeFilePolicyArg::Prompt)]
    pub large_files: LargeFilePolicyArg,
}

#[derive(Debug, Args)]
pub struct RestoreArgs {
    /// Archive path (.tar.zst or .tar.zst.age).
    pub archive: Utf8PathBuf,

    /// Destination directory. Created if absent.
    pub dest: Utf8PathBuf,

    /// Don't run `git init` / fetch from .gitmeta files; leave sidecars in place.
    #[arg(long)]
    pub no_git_rehydrate: bool,

    /// `git fetch` the full history instead of `--depth=1`.
    #[arg(long)]
    pub full_history: bool,

    /// Skip blake3 manifest verification (not recommended).
    #[arg(long)]
    pub skip_verify: bool,
}

#[derive(Clone, Copy, Debug, ValueEnum)]
pub enum LargeFilePolicyArg {
    Prompt,
    Keep,
    Skip,
}

impl From<LargeFilePolicyArg> for LargeFilePolicy {
    fn from(v: LargeFilePolicyArg) -> Self {
        match v {
            LargeFilePolicyArg::Prompt => LargeFilePolicy::Prompt,
            LargeFilePolicyArg::Keep => LargeFilePolicy::KeepAll,
            LargeFilePolicyArg::Skip => LargeFilePolicy::SkipAll,
        }
    }
}
