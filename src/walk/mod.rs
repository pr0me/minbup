pub mod discover;
pub mod gitmeta;
pub mod stream;

use std::time::SystemTime;

use camino::Utf8PathBuf;

pub use gitmeta::Project;

#[derive(Debug)]
pub enum WalkEvent {
    Small {
        abs: Utf8PathBuf,
        rel: Utf8PathBuf,
        size: u64,
        mtime: SystemTime,
    },
    Large {
        abs: Utf8PathBuf,
        rel: Utf8PathBuf,
        size: u64,
        mtime: SystemTime,
        tracked_by_git: bool,
    },
    EndOfSmall,
}

#[derive(Debug, Default)]
pub struct DiscoverReport {
    pub total_files: u64,
    pub total_bytes: u64,
    pub projects: Vec<Project>,
}
