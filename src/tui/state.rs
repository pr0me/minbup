use std::ops::Deref;
use std::sync::atomic::{AtomicU64, AtomicU8, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Instant;

#[derive(Debug, Copy, Clone, PartialEq, Eq)]
#[repr(u8)]
pub enum Phase {
    Preflight = 0,
    Discover = 1,
    Stream = 2,
    Review = 3,
    StreamLarge = 4,
    Manifest = 5,
    Done = 6,
}

impl Phase {
    pub fn from_u8(v: u8) -> Self {
        match v {
            0 => Phase::Preflight,
            1 => Phase::Discover,
            2 => Phase::Stream,
            3 => Phase::Review,
            4 => Phase::StreamLarge,
            5 => Phase::Manifest,
            _ => Phase::Done,
        }
    }

    pub fn label(self) -> &'static str {
        match self {
            Phase::Preflight => "preflight",
            Phase::Discover => "discover",
            Phase::Stream => "stream",
            Phase::Review => "review",
            Phase::StreamLarge => "stream-large",
            Phase::Manifest => "manifest",
            Phase::Done => "done",
        }
    }
}

pub struct Inner {
    pub bytes_scanned: AtomicU64,
    pub bytes_total: AtomicU64,
    pub bytes_archived_uncompressed: AtomicU64,
    pub bytes_archived_compressed: Arc<AtomicU64>,
    pub files_done: AtomicU64,
    pub files_total: AtomicU64,
    pub projects_found: AtomicU64,
    pub large_queued: AtomicU64,
    pub errors_skipped: AtomicU64,
    pub phase: AtomicU8,
    pub start: Instant,
    pub current_path: Mutex<String>,
}

#[derive(Clone)]
pub struct ProgressState(Arc<Inner>);

impl Default for ProgressState {
    fn default() -> Self {
        Self(Arc::new(Inner {
            bytes_scanned: AtomicU64::new(0),
            bytes_total: AtomicU64::new(0),
            bytes_archived_uncompressed: AtomicU64::new(0),
            bytes_archived_compressed: Arc::new(AtomicU64::new(0)),
            files_done: AtomicU64::new(0),
            files_total: AtomicU64::new(0),
            projects_found: AtomicU64::new(0),
            large_queued: AtomicU64::new(0),
            errors_skipped: AtomicU64::new(0),
            phase: AtomicU8::new(Phase::Preflight as u8),
            start: Instant::now(),
            current_path: Mutex::new(String::new()),
        }))
    }
}

impl ProgressState {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn set_phase(&self, p: Phase) {
        self.phase.store(p as u8, Ordering::Relaxed);
    }

    pub fn phase(&self) -> Phase {
        Phase::from_u8(self.phase.load(Ordering::Relaxed))
    }

    pub fn set_current_path(&self, p: impl Into<String>) {
        if let Ok(mut g) = self.current_path.lock() {
            *g = p.into();
        }
    }
}

impl Deref for ProgressState {
    type Target = Inner;
    fn deref(&self) -> &Inner {
        &self.0
    }
}
