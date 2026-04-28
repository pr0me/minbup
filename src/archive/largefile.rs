use std::time::SystemTime;

use camino::Utf8PathBuf;

#[derive(Debug, Clone)]
pub struct LargeFileEntry {
    pub abs: Utf8PathBuf,
    pub rel: Utf8PathBuf,
    pub size: u64,
    pub mtime: SystemTime,
    pub tracked_by_git: bool,
}

#[derive(Debug, Clone)]
pub enum ReviewOutcome {
    SkipAll,
    KeepAll,
    /// Indices into the original queue that should be kept.
    KeepSelected(Vec<usize>),
}

pub trait ReviewProvider: Send {
    fn decide(&mut self, queue: &[LargeFileEntry]) -> ReviewOutcome;
}
