use camino::Utf8PathBuf;

pub const LARGE_FILE_THRESHOLD_DEFAULT: u64 = 100 * 1024 * 1024;
pub const ZSTD_LEVEL_DEFAULT: i32 = 3;
pub const CHANNEL_CAPACITY: usize = 1024;
pub const READ_BUFFER_BYTES: usize = 1 << 20;
pub const WRITE_BUFFER_BYTES: usize = 1 << 20;
pub const MANIFEST_NAME: &str = "MANIFEST.blake3";
pub const GITMETA_NAME: &str = ".gitmeta";
pub const SCHEMA_VERSION: u32 = 1;
pub const PASSPHRASE_ENV: &str = "MINBUP_PASSPHRASE";

/// Default exclusion globs (gitignore syntax). always-on unless `--no-default-excludes`.
pub const DEFAULT_EXCLUDES: &[&str] = &[
    // language caches
    "**/__pycache__/",
    "**/.venv/",
    "**/venv/",
    "**/.mypy_cache/",
    "**/.pytest_cache/",
    "**/.ruff_cache/",
    "**/.cache/",
    // editor / os junk
    "**/.DS_Store",
    "**/.idea/",
    "**/.vscode/",
    "**/*.swp",
    "**/Thumbs.db",
];

#[derive(Clone, Debug, Copy, PartialEq, Eq)]
pub enum LargeFilePolicy {
    Prompt,
    KeepAll,
    SkipAll,
}

#[derive(Clone, Debug)]
pub struct BackupSettings {
    pub target: Utf8PathBuf,
    pub output: Utf8PathBuf,
    pub encrypt: bool,
    pub large_threshold: u64,
    pub zstd_level: i32,
    pub zstd_workers: u32,
    pub large_files: LargeFilePolicy,
    pub no_default_excludes: bool,
    pub extra_excludes: Vec<String>,
}

#[derive(Clone, Debug)]
pub struct RestoreSettings {
    pub archive: Utf8PathBuf,
    pub dest: Utf8PathBuf,
    pub no_git_rehydrate: bool,
    pub full_history: bool,
    pub skip_verify: bool,
}
