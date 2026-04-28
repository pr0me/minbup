use std::collections::VecDeque;
use std::sync::atomic::Ordering;
use std::time::Duration;

use ratatui::layout::{Constraint, Direction, Layout, Rect};
use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Borders, Gauge, Paragraph, Sparkline};
use ratatui::{Frame, symbols};

use crate::tui::state::{Phase, ProgressState};
use crate::util::human_bytes;

const ACCENT: Color = Color::Cyan;
const DIM: Color = Color::DarkGray;

pub struct ViewModel {
    pub state: ProgressState,
    pub spark: VecDeque<u64>,
    pub spark_capacity: usize,
    pub last_compressed: u64,
}

impl ViewModel {
    pub fn new(state: ProgressState) -> Self {
        Self {
            state,
            spark: VecDeque::with_capacity(120),
            spark_capacity: 120,
            last_compressed: 0,
        }
    }

    pub fn tick(&mut self) {
        let now = self.state.bytes_archived_compressed.load(Ordering::Relaxed);
        let delta = now.saturating_sub(self.last_compressed);
        self.last_compressed = now;
        if self.spark.len() == self.spark_capacity {
            self.spark.pop_front();
        }
        self.spark.push_back(delta);
    }
}

pub fn draw(f: &mut Frame, vm: &ViewModel) {
    let area = f.area();
    let outer = Block::default()
        .borders(Borders::ALL)
        .border_style(Style::default().fg(ACCENT))
        .border_set(symbols::border::ROUNDED)
        .title(Span::styled(
            " minbup ",
            Style::default().fg(ACCENT).add_modifier(Modifier::BOLD),
        ));
    f.render_widget(outer, area);

    let inner = Rect {
        x: area.x + 1,
        y: area.y + 1,
        width: area.width.saturating_sub(2),
        height: area.height.saturating_sub(2),
    };

    let rows = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Length(1), // phase line
            Constraint::Length(3), // progress gauge
            Constraint::Min(6),    // body (stats + spark)
            Constraint::Length(1), // current path
        ])
        .split(inner);

    draw_phase(f, rows[0], vm);
    draw_gauge(f, rows[1], vm);
    draw_body(f, rows[2], vm);
    draw_current(f, rows[3], vm);
}

fn draw_phase(f: &mut Frame, area: Rect, vm: &ViewModel) {
    let phases = [
        Phase::Preflight,
        Phase::Discover,
        Phase::Stream,
        Phase::Review,
        Phase::StreamLarge,
        Phase::Manifest,
        Phase::Done,
    ];
    let active = vm.state.phase();
    let sep = Span::styled(" › ", Style::default().fg(DIM));
    let mut spans: Vec<Span<'_>> = Vec::with_capacity(phases.len() * 2);
    for (i, p) in phases.iter().enumerate() {
        if i > 0 {
            spans.push(sep.clone());
        }
        let style = if *p == active {
            Style::default().fg(ACCENT).add_modifier(Modifier::BOLD)
        } else if (*p as u8) < (active as u8) {
            Style::default().fg(Color::Green)
        } else {
            Style::default().fg(DIM)
        };
        spans.push(Span::styled(p.label(), style));
    }
    f.render_widget(Paragraph::new(Line::from(spans)), area);
}

fn draw_gauge(f: &mut Frame, area: Rect, vm: &ViewModel) {
    let total = vm.state.bytes_total.load(Ordering::Relaxed).max(1);
    let done = vm.state.bytes_archived_uncompressed.load(Ordering::Relaxed);
    let pct = ((done as f64 / total as f64) * 100.0).min(100.0) as u16;

    let elapsed = vm.state.start.elapsed();
    let eta = compute_eta(done, total, elapsed);
    let label = format!(
        "{} / {}  ·  {:>3}%  ·  eta {}",
        human_bytes(done),
        human_bytes(total),
        pct,
        eta
    );

    let g = Gauge::default()
        .block(
            Block::default()
                .borders(Borders::TOP | Borders::BOTTOM)
                .border_style(Style::default().fg(DIM))
                .title(Span::styled(" progress ", Style::default().fg(DIM))),
        )
        .gauge_style(Style::default().fg(ACCENT).bg(Color::Reset))
        .percent(pct)
        .label(label);
    f.render_widget(g, area);
}

fn draw_body(f: &mut Frame, area: Rect, vm: &ViewModel) {
    let cols = Layout::default()
        .direction(Direction::Horizontal)
        .constraints([Constraint::Min(36), Constraint::Min(20)])
        .split(area);

    draw_stats(f, cols[0], vm);
    draw_spark(f, cols[1], vm);
}

fn draw_stats(f: &mut Frame, area: Rect, vm: &ViewModel) {
    let s = &vm.state;
    let scanned = s.bytes_scanned.load(Ordering::Relaxed);
    let archived_u = s.bytes_archived_uncompressed.load(Ordering::Relaxed);
    let archived_c = s.bytes_archived_compressed.load(Ordering::Relaxed);
    let files_done = s.files_done.load(Ordering::Relaxed);
    let files_total = s.files_total.load(Ordering::Relaxed);
    let projects = s.projects_found.load(Ordering::Relaxed);
    let large = s.large_queued.load(Ordering::Relaxed);
    let errors = s.errors_skipped.load(Ordering::Relaxed);

    let ratio = if archived_u > 0 {
        archived_c as f64 / archived_u as f64
    } else {
        0.0
    };

    let lines = vec![
        kv("scanned", human_bytes(scanned)),
        kv("uncompressed", human_bytes(archived_u)),
        kv(
            "compressed",
            format!("{}  ({:.0}% of source)", human_bytes(archived_c), ratio * 100.0),
        ),
        kv("files", format!("{} / {}", files_done, files_total)),
        kv("projects", projects.to_string()),
        kv("large queued", large.to_string()),
        kv("errors skipped", errors.to_string()),
    ];

    let block = Block::default()
        .borders(Borders::RIGHT)
        .border_style(Style::default().fg(DIM))
        .title(Span::styled(" stats ", Style::default().fg(DIM)));
    f.render_widget(Paragraph::new(lines).block(block), area);
}

fn draw_spark(f: &mut Frame, area: Rect, vm: &ViewModel) {
    let data: Vec<u64> = vm.spark.iter().copied().collect();
    let block = Block::default()
        .borders(Borders::NONE)
        .title(Span::styled(" throughput ", Style::default().fg(DIM)));
    let s = Sparkline::default()
        .block(block)
        .data(&data)
        .style(Style::default().fg(ACCENT));
    f.render_widget(s, area);
}

fn draw_current(f: &mut Frame, area: Rect, vm: &ViewModel) {
    let path = vm.state.current_path.lock().map(|s| s.clone()).unwrap_or_default();
    let text = if path.is_empty() {
        Line::from(Span::styled("idle", Style::default().fg(DIM)))
    } else {
        let max = area.width.saturating_sub(4) as usize;
        let truncated = if path.len() > max && max > 1 {
            format!("…{}", &path[path.len() - (max - 1)..])
        } else {
            path
        };
        Line::from(vec![
            Span::styled("▸ ", Style::default().fg(ACCENT)),
            Span::styled(truncated, Style::default().fg(Color::Gray)),
        ])
    };
    f.render_widget(Paragraph::new(text), area);
}

fn kv(k: impl Into<String>, v: impl Into<String>) -> Line<'static> {
    Line::from(vec![
        Span::styled(format!("{:<14}", k.into()), Style::default().fg(DIM)),
        Span::styled(v.into(), Style::default().fg(Color::White)),
    ])
}

fn compute_eta(done: u64, total: u64, elapsed: Duration) -> String {
    if done == 0 || total == 0 || done >= total {
        return "—".into();
    }
    let rate = done as f64 / elapsed.as_secs_f64().max(0.001);
    if rate <= 0.0 {
        return "—".into();
    }
    let remaining = (total - done) as f64 / rate;
    format_secs(remaining as u64)
}

fn format_secs(s: u64) -> String {
    if s < 60 {
        format!("{s}s")
    } else if s < 3600 {
        format!("{}m{:02}s", s / 60, s % 60)
    } else {
        format!("{}h{:02}m", s / 3600, (s % 3600) / 60)
    }
}
