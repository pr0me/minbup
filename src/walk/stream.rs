use std::sync::atomic::Ordering;

use camino::Utf8Path;
use crossbeam_channel::Sender;
use tracing::warn;

use crate::config::BackupSettings;
use crate::error::Result;
use crate::tui::state::ProgressState;
use crate::walk::discover::{build_walker, is_tracked};
use crate::walk::{Project, WalkEvent};

/// Second pass: emit WalkEvents through `tx`. Closes by sending EndOfSmall.
pub fn run(
    target: &Utf8Path,
    settings: &BackupSettings,
    projects: &[Project],
    tx: Sender<WalkEvent>,
    state: &ProgressState,
) -> Result<()> {
    let walker = build_walker(target, settings)?;

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

        let abs = match Utf8Path::from_path(entry.path()) {
            Some(p) => p.to_path_buf(),
            None => {
                warn!("non-utf8 path skipped: {:?}", entry.path());
                state.errors_skipped.fetch_add(1, Ordering::Relaxed);
                continue;
            }
        };
        let rel = match abs.strip_prefix(target) {
            Ok(r) => r.to_path_buf(),
            Err(_) => continue,
        };
        let md = match entry.metadata() {
            Ok(m) => m,
            Err(e) => {
                warn!("metadata: {e}");
                state.errors_skipped.fetch_add(1, Ordering::Relaxed);
                continue;
            }
        };
        let size = md.len();
        let mtime = md.modified().unwrap_or(std::time::SystemTime::UNIX_EPOCH);

        let ev = if size >= settings.large_threshold {
            state.large_queued.fetch_add(1, Ordering::Relaxed);
            WalkEvent::Large {
                abs: abs.clone(),
                rel,
                size,
                mtime,
                tracked_by_git: is_tracked(projects, &abs),
            }
        } else {
            WalkEvent::Small { abs, rel, size, mtime }
        };

        if tx.send(ev).is_err() {
            // archiver dropped; abort walk
            break;
        }
    }
    let _ = tx.send(WalkEvent::EndOfSmall);
    Ok(())
}
