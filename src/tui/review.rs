use std::io;
use std::time::Duration;

use crossterm::event::{self, Event, KeyCode, KeyEventKind};
use ratatui::layout::{Constraint, Direction, Layout, Margin, Rect};
use ratatui::prelude::Backend;
use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Borders, List, ListItem, ListState, Paragraph};
use ratatui::{Frame, Terminal, symbols};

use crate::archive::{LargeFileEntry, ReviewOutcome};
use crate::util::{human_bytes, systemtime_to_rfc3339};

const ACCENT: Color = Color::Cyan;
const WARN: Color = Color::Yellow;
const DIM: Color = Color::DarkGray;

pub fn run<B: Backend>(
    term: &mut Terminal<B>,
    queue: &[LargeFileEntry],
) -> io::Result<ReviewOutcome> {
    if queue.is_empty() {
        return Ok(ReviewOutcome::SkipAll);
    }

    let mut keep = vec![true; queue.len()];
    let mut list_state = ListState::default();
    list_state.select(Some(0));

    loop {
        term.draw(|f| draw(f, queue, &keep, &mut list_state))?;
        if event::poll(Duration::from_millis(150))? {
            if let Event::Key(k) = event::read()? {
                if k.kind != KeyEventKind::Press {
                    continue;
                }
                match k.code {
                    KeyCode::Char('q') | KeyCode::Char('Q') => {
                        let kept: Vec<usize> = keep
                            .iter()
                            .enumerate()
                            .filter_map(|(i, k)| k.then_some(i))
                            .collect();
                        return Ok(if kept.len() == queue.len() {
                            ReviewOutcome::KeepAll
                        } else if kept.is_empty() {
                            ReviewOutcome::SkipAll
                        } else {
                            ReviewOutcome::KeepSelected(kept)
                        });
                    }
                    KeyCode::Char('a') => {
                        return Ok(ReviewOutcome::KeepAll);
                    }
                    KeyCode::Char('s') => {
                        return Ok(ReviewOutcome::SkipAll);
                    }
                    KeyCode::Esc => {
                        return Ok(ReviewOutcome::SkipAll);
                    }
                    KeyCode::Up | KeyCode::Char('k') => move_sel(&mut list_state, queue.len(), -1),
                    KeyCode::Down | KeyCode::Char('j') => move_sel(&mut list_state, queue.len(), 1),
                    KeyCode::PageUp => move_sel(&mut list_state, queue.len(), -10),
                    KeyCode::PageDown => move_sel(&mut list_state, queue.len(), 10),
                    KeyCode::Home => list_state.select(Some(0)),
                    KeyCode::End => list_state.select(Some(queue.len().saturating_sub(1))),
                    KeyCode::Char(' ') | KeyCode::Enter => {
                        if let Some(i) = list_state.selected() {
                            keep[i] = !keep[i];
                        }
                    }
                    _ => {}
                }
            }
        }
    }
}

fn move_sel(state: &mut ListState, len: usize, delta: i32) {
    if len == 0 {
        return;
    }
    let cur = state.selected().unwrap_or(0) as i32;
    let next = (cur + delta).clamp(0, len as i32 - 1);
    state.select(Some(next as usize));
}

fn draw(f: &mut Frame, queue: &[LargeFileEntry], keep: &[bool], state: &mut ListState) {
    let area = f.area();
    let outer = Block::default()
        .borders(Borders::ALL)
        .border_style(Style::default().fg(WARN))
        .border_set(symbols::border::ROUNDED)
        .title(Span::styled(
            " large files — review ",
            Style::default().fg(WARN).add_modifier(Modifier::BOLD),
        ));
    f.render_widget(outer, area);

    let inner = area.inner(Margin { horizontal: 1, vertical: 1 });

    let rows = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Length(2), // header
            Constraint::Min(5),    // body (list + detail)
            Constraint::Length(2), // help
        ])
        .split(inner);

    let kept = keep.iter().filter(|k| **k).count();
    let total_size: u64 = queue.iter().map(|e| e.size).sum();
    let kept_size: u64 = queue
        .iter()
        .zip(keep)
        .filter_map(|(e, k)| k.then_some(e.size))
        .sum();
    f.render_widget(
        Paragraph::new(vec![
            Line::from(vec![
                Span::styled(
                    format!("{} files queued", queue.len()),
                    Style::default().fg(WARN).add_modifier(Modifier::BOLD),
                ),
                Span::raw("  ·  "),
                Span::styled(
                    format!("{} keep / {} skip", kept, queue.len() - kept),
                    Style::default().fg(ACCENT),
                ),
                Span::raw("  ·  "),
                Span::styled(
                    format!("{} of {}", human_bytes(kept_size), human_bytes(total_size)),
                    Style::default().fg(Color::White),
                ),
            ]),
            Line::from(""),
        ]),
        rows[0],
    );

    let body = Layout::default()
        .direction(Direction::Horizontal)
        .constraints([Constraint::Percentage(50), Constraint::Percentage(50)])
        .split(rows[1]);

    draw_list(f, body[0], queue, keep, state);
    draw_detail(f, body[1], queue, state);

    f.render_widget(
        Paragraph::new(Line::from(vec![
            Span::styled("[space/enter]", Style::default().fg(ACCENT)),
            Span::raw(" toggle  "),
            Span::styled("[a]", Style::default().fg(ACCENT)),
            Span::raw(" keep all  "),
            Span::styled("[s]", Style::default().fg(ACCENT)),
            Span::raw(" skip all  "),
            Span::styled("[q]", Style::default().fg(ACCENT)),
            Span::raw(" confirm"),
        ])),
        rows[2],
    );
}

fn draw_list(
    f: &mut Frame,
    area: Rect,
    queue: &[LargeFileEntry],
    keep: &[bool],
    state: &mut ListState,
) {
    let items: Vec<ListItem<'_>> = queue
        .iter()
        .zip(keep)
        .map(|(e, k)| {
            let mark = if *k { "[x]" } else { "[ ]" };
            let mark_style = if *k {
                Style::default().fg(Color::Green)
            } else {
                Style::default().fg(DIM)
            };
            let name = e.rel.file_name().unwrap_or_else(|| e.rel.as_str());
            ListItem::new(Line::from(vec![
                Span::styled(format!("{} ", mark), mark_style),
                Span::raw(format!("{:>10}  ", human_bytes(e.size))),
                Span::styled(name.to_owned(), Style::default().fg(Color::White)),
            ]))
        })
        .collect();

    let list = List::new(items)
        .block(
            Block::default()
                .borders(Borders::RIGHT)
                .border_style(Style::default().fg(DIM)),
        )
        .highlight_style(Style::default().bg(Color::DarkGray).add_modifier(Modifier::BOLD))
        .highlight_symbol("▸ ");
    f.render_stateful_widget(list, area, state);
}

fn draw_detail(f: &mut Frame, area: Rect, queue: &[LargeFileEntry], state: &ListState) {
    let i = state.selected().unwrap_or(0);
    let e = match queue.get(i) {
        Some(e) => e,
        None => return,
    };
    let lines = vec![
        kv("path", e.rel.as_str().to_owned()),
        kv("size", human_bytes(e.size)),
        kv("modified", systemtime_to_rfc3339(e.mtime)),
        kv("git tracked", if e.tracked_by_git { "yes" } else { "no" }),
    ];
    f.render_widget(Paragraph::new(lines).block(Block::default().borders(Borders::NONE)), area);
}

fn kv(k: impl Into<String>, v: impl Into<String>) -> Line<'static> {
    Line::from(vec![
        Span::styled(format!("  {:<14}", k.into()), Style::default().fg(DIM)),
        Span::styled(v.into(), Style::default().fg(Color::White)),
    ])
}
