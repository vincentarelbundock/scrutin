//! Normal mode (Files level): file list + side pane.

use ratatui::layout::{Constraint, Direction, Layout, Rect};
use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{List, ListItem, Paragraph, Wrap};

use crate::state::*;
use super::breadcrumb::draw_breadcrumb_bar;
use super::counts::draw_counts_bar;
use super::file_list::draw_file_list;
use super::hints::draw_hints_bar;
use super::icons::test_icon_color;
use super::layout::{main_focused, pane_block, split_panes, PaneLayout};
use super::source::{file_line_count, load_source_context_ex};

pub(super) fn draw_normal(f: &mut ratatui::Frame, state: &mut AppState) {
    // Vertical chrome collapses on short terminals: drop the hints bar
    // first, then the counts bar. The header always stays.
    let total_h = f.area().height;
    let show_counts = total_h >= COUNTS_BAR_MIN_ROWS;
    let show_hints  = total_h >= HINTS_BAR_MIN_ROWS;

    let mut constraints: Vec<Constraint> = vec![
        Constraint::Length(1), // header
        Constraint::Min(3),    // file list / split body
    ];
    if show_counts { constraints.push(Constraint::Length(1)); }
    if show_hints  { constraints.push(Constraint::Length(1)); }

    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints(constraints)
        .split(f.area());

    draw_breadcrumb_bar(f, state, chunks[0]);
    match split_panes(chunks[1], state.current_list_pct(), state.current_horizontal()) {
        PaneLayout::Split { list, main } => {
            state.pane_rects.list = Some(list);
            state.pane_rects.main = Some(main);
            draw_file_list(f, state, list);
            draw_side_pane(f, state, main);
        }
        PaneLayout::Single { area } => {
            state.pane_rects.main = Some(area);
            draw_side_pane(f, state, area);
        }
    }
    let mut idx = 2;
    if show_counts { draw_counts_bar(f, state, chunks[idx]); idx += 1; }
    if show_hints  { draw_hints_bar(f, state, chunks[idx]); }
}

/// Side pane shown next to the file list on wide terminals. Collapses
/// Detail/Failure modes into the Normal view: shows the cursor file's
/// test list, or \u{2014} if it failed \u{2014} the first failure with a source
/// snippet, or \u{2014} if it's running \u{2014} a tail of the global log.
pub(super) fn draw_side_pane(f: &mut ratatui::Frame, state: &mut AppState, area: Rect) {
    let focused = main_focused(state.mode());
    let (path, name, status, tests) = match state.selected_file() {
        Some(e) => (e.path.clone(), e.name.clone(), e.status.clone(), e.tests.clone()),
        None => {
            f.render_widget(pane_block("Details", focused), area);
            state.nav.source_scroll_max = 0;
            return;
        }
    };
    let title = match &status {
        FileStatus::Failed  { .. } => format!("Failure \u{2014} {}", name),
        FileStatus::Running        => format!("Log \u{2014} {}", name),
        _                          => name.clone(),
    };
    let block = pane_block(&title, focused);
    let inner = block.inner(area);
    f.render_widget(block, area);

    match &status {
        FileStatus::Running => {
            let lines = state.log.snapshot();
            let h = inner.height as usize;
            let start = lines.len().saturating_sub(h);
            let body: Vec<Line> = lines[start..]
                .iter()
                .map(|l| Line::from(Span::styled(l.clone(), Style::default().fg(Color::Gray))))
                .collect();
            f.render_widget(Paragraph::new(body).wrap(Wrap { trim: false }), inner);
            state.nav.source_scroll_max = 0;
        }
        FileStatus::Failed { .. } => {
            // First failing test + source snippet around its line.
            let failing = tests.iter().find(|t| t.is_bad());
            let h = inner.height as usize;
            let mut lines: Vec<Line> = Vec::new();
            if let Some(t) = failing {
                let test_name = if t.name.is_empty() { "test".to_string() } else { t.name.clone() };
                let loc = match t.line {
                    Some(l) => format!("{}:{}", name, l),
                    None    => name.clone(),
                };
                lines.push(Line::from(vec![
                    Span::styled(" FAIL ", Style::default().fg(Color::Red).add_modifier(Modifier::BOLD)),
                    Span::styled(loc, Style::default().fg(Color::DarkGray)),
                    Span::raw(" \u{203a} "),
                    Span::raw(test_name),
                ]));
                lines.push(Line::from(""));
                for ml in t.message.lines().take(6) {
                    lines.push(Line::from(Span::styled(ml.to_string(), Style::default().fg(Color::Red))));
                }
                lines.push(Line::from(""));
                let auto = h.saturating_sub(lines.len());
                let remaining = if state.nav.source_context_lines > 0 {
                    state.nav.source_context_lines.min(auto)
                } else {
                    auto
                };
                if remaining > 2 {
                    let src = load_source_context_ex(
                        &path, t.line, remaining,
                        state.nav.source_scroll, state.nav.source_hscroll,
                    );
                    lines.extend(src);
                }
            } else {
                lines.push(Line::from(" (failed file, no test detail)"));
            }
            f.render_widget(Paragraph::new(lines), inner);
            state.nav.source_scroll_max = file_line_count(&path).saturating_sub(inner.height as usize);
        }
        _ => {
            // Pending / Passed / Cancelled \u{2014} show per-test list, or the file
            // source if there are no tests yet (e.g. Pending: not run yet).
            if tests.is_empty() {
                if matches!(status, FileStatus::Pending) {
                    let h = inner.height as usize;
                    let src = load_source_context_ex(
                        &path, None, h,
                        state.nav.source_scroll, state.nav.source_hscroll,
                    );
                    f.render_widget(Paragraph::new(src), inner);
                    state.nav.source_scroll_max = file_line_count(&path).saturating_sub(inner.height as usize);
                    return;
                }
                let msg = match &status {
                    FileStatus::Cancelled => " (cancelled)",
                    _                     => " (no tests)",
                };
                f.render_widget(
                    Paragraph::new(Line::from(Span::styled(msg, Style::default().fg(Color::DarkGray)))),
                    inner,
                );
                state.nav.source_scroll_max = 0;
                return;
            }
            let h = inner.height as usize;
            let max_skip = tests.len().saturating_sub(h.max(1));
            state.nav.source_scroll_max = max_skip;
            let scroll = state.nav.source_scroll.min(max_skip);
            let items: Vec<ListItem> = tests
                .iter()
                .enumerate()
                .skip(scroll)
                .take(h)
                .map(|(i, t)| {
                    let (icon, color) = test_icon_color(t);
                    let name = if t.name.is_empty() { format!("test {}", i + 1) } else { t.name.clone() };
                    let ms_str = if t.duration_ms > 0 { format!("  {}ms", t.duration_ms) } else { String::new() };
                    ListItem::new(Line::from(vec![
                        Span::styled(format!("  {} ", icon), Style::default().fg(color)),
                        Span::raw(name),
                        Span::styled(ms_str, Style::default().fg(Color::DarkGray)),
                    ]))
                })
                .collect();
            f.render_widget(List::new(items), inner);
        }
    }
}
