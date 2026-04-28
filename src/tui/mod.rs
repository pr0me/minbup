pub mod review;
pub mod state;
pub mod view;

use std::io::{self, IsTerminal, Write};
use std::thread::JoinHandle;
use std::time::Duration;

use crossbeam_channel::{Receiver, Sender};
use crossterm::event::{self, Event, KeyCode, KeyEventKind, KeyModifiers};
use crossterm::execute;
use crossterm::terminal::{
    disable_raw_mode, enable_raw_mode, EnterAlternateScreen, LeaveAlternateScreen,
};
use ratatui::backend::CrosstermBackend;
use ratatui::Terminal;

use crate::archive::{LargeFileEntry, ReviewOutcome, ReviewProvider};
use crate::error::{Error, Result};

use self::state::ProgressState;
use self::view::ViewModel;

pub struct TuiHandle {
    review_tx: Sender<Vec<LargeFileEntry>>,
    review_rx: Receiver<ReviewOutcome>,
    shutdown_tx: Sender<()>,
    join: Option<JoinHandle<Result<()>>>,
}

impl TuiHandle {
    pub fn shutdown(mut self) -> Result<()> {
        let _ = self.shutdown_tx.send(());
        if let Some(j) = self.join.take() {
            match j.join() {
                Ok(r) => r,
                Err(_) => Err(Error::Other(anyhow::anyhow!("tui thread panicked"))),
            }
        } else {
            Ok(())
        }
    }
}

impl ReviewProvider for TuiHandle {
    fn decide(&mut self, queue: &[LargeFileEntry]) -> ReviewOutcome {
        if self.review_tx.send(queue.to_vec()).is_err() {
            return ReviewOutcome::KeepAll;
        }
        self.review_rx.recv().unwrap_or(ReviewOutcome::KeepAll)
    }
}

pub fn is_tty() -> bool {
    io::stderr().is_terminal()
}

pub fn spawn(state: ProgressState) -> Result<TuiHandle> {
    let (review_tx, review_req_rx) = crossbeam_channel::bounded::<Vec<LargeFileEntry>>(1);
    let (review_resp_tx, review_rx) = crossbeam_channel::bounded::<ReviewOutcome>(1);
    let (shutdown_tx, shutdown_rx) = crossbeam_channel::bounded::<()>(1);

    let join = std::thread::Builder::new()
        .name("minbup-tui".into())
        .spawn(move || run_tui(state, review_req_rx, review_resp_tx, shutdown_rx))
        .map_err(|e| Error::Other(anyhow::anyhow!("spawn tui: {e}")))?;

    Ok(TuiHandle {
        review_tx,
        review_rx,
        shutdown_tx,
        join: Some(join),
    })
}

fn run_tui(
    state: ProgressState,
    review_req_rx: Receiver<Vec<LargeFileEntry>>,
    review_resp_tx: Sender<ReviewOutcome>,
    shutdown_rx: Receiver<()>,
) -> Result<()> {
    enable_raw_mode().map_err(io_err)?;
    let mut stderr = io::stderr();
    execute!(stderr, EnterAlternateScreen).map_err(io_err)?;
    let backend = CrosstermBackend::new(stderr);
    let mut term = Terminal::new(backend).map_err(io_err)?;

    let mut vm = ViewModel::new(state);
    let outcome = run_loop(&mut term, &mut vm, &review_req_rx, &review_resp_tx, &shutdown_rx);

    let _ = disable_raw_mode();
    let _ = execute!(term.backend_mut(), LeaveAlternateScreen);
    let _ = term.show_cursor();

    outcome
}

fn run_loop<B: ratatui::backend::Backend>(
    term: &mut Terminal<B>,
    vm: &mut ViewModel,
    review_req_rx: &Receiver<Vec<LargeFileEntry>>,
    review_resp_tx: &Sender<ReviewOutcome>,
    shutdown_rx: &Receiver<()>,
) -> Result<()> {
    loop {
        if shutdown_rx.try_recv().is_ok() || vm.state.is_aborted() {
            term.draw(|f| view::draw(f, vm)).map_err(io_err)?;
            return Ok(());
        }
        if let Ok(queue) = review_req_rx.try_recv() {
            let outcome = review::run(term, &queue).map_err(io_err)?;
            let _ = review_resp_tx.send(outcome);
        }
        if event::poll(Duration::from_millis(0)).map_err(io_err)? {
            if let Event::Key(k) = event::read().map_err(io_err)? {
                if k.kind == KeyEventKind::Press && is_abort_key(&k) {
                    vm.state.signal_abort();
                    return Ok(());
                }
            }
        }
        vm.tick();
        term.draw(|f| view::draw(f, vm)).map_err(io_err)?;
        std::thread::sleep(Duration::from_millis(33));
    }
}

fn is_abort_key(k: &crossterm::event::KeyEvent) -> bool {
    (k.modifiers.contains(KeyModifiers::CONTROL) && k.code == KeyCode::Char('c'))
        || k.code == KeyCode::Esc
        || k.code == KeyCode::Char('q')
}

fn io_err(e: io::Error) -> Error {
    Error::Io(e)
}

/// Lightweight stderr fallback when stdout is not a TTY.
pub struct PlainProgress {
    state: ProgressState,
    handle: Option<JoinHandle<()>>,
    shutdown_tx: Sender<()>,
}

impl PlainProgress {
    pub fn spawn(state: ProgressState) -> Self {
        let (shutdown_tx, shutdown_rx) = crossbeam_channel::bounded::<()>(1);
        let s = state.clone();
        let handle = std::thread::Builder::new()
            .name("minbup-progress".into())
            .spawn(move || plain_loop(s, shutdown_rx))
            .ok();
        Self { state, handle, shutdown_tx }
    }

    pub fn shutdown(mut self) {
        let _ = self.shutdown_tx.send(());
        if let Some(h) = self.handle.take() {
            let _ = h.join();
        }
        let _ = self.state;
    }
}

fn plain_loop(state: ProgressState, shutdown_rx: Receiver<()>) {
    use std::sync::atomic::Ordering;
    let mut last_compressed = 0u64;
    let mut last_emit = std::time::Instant::now();
    loop {
        if shutdown_rx.try_recv().is_ok() {
            break;
        }
        std::thread::sleep(Duration::from_millis(500));
        if last_emit.elapsed() < Duration::from_secs(2) {
            continue;
        }
        last_emit = std::time::Instant::now();
        let phase = state.phase();
        let archived_c = state.bytes_archived_compressed.load(Ordering::Relaxed);
        let files_done = state.files_done.load(Ordering::Relaxed);
        let files_total = state.files_total.load(Ordering::Relaxed);
        let delta = archived_c.saturating_sub(last_compressed);
        last_compressed = archived_c;
        let throughput = (delta as f64 / 2.0) as u64;
        let mut stderr = io::stderr();
        let _ = writeln!(
            stderr,
            "[{}] files {}/{}  archive {}  ({}/s)",
            phase.label(),
            files_done,
            files_total,
            crate::util::human_bytes(archived_c),
            crate::util::human_bytes(throughput),
        );
    }
}
