//! Top-of-viewport breadcrumb bar. Same format in every drill level so
//! "where am I" always reads the same way: `[LEVEL] pkg \u203a file \u203a test (3/9)`
//! on the left, `N/M workers \u{00b7} status [filters]` on the right.

use ratatui::layout::{Constraint, Direction, Layout, Rect};
use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::Paragraph;

use crate::keymap;
use crate::state::*;

pub(super) fn draw_breadcrumb_bar(f: &mut ratatui::Frame, state: &AppState, area: Rect) {
    // ── Left: level pill + breadcrumb path. ───────────────────────────
    // Drives off `state.level()` so an overlay (Help/Log/Palette) sitting
    // on top of Detail still shows the DETAIL pill, not the overlay's color.
    let level = state.level();
    let pill_color = keymap::mode_color(state.mode());
    let pill_label = match level {
        Level::Normal  => "FILES",
        Level::Detail  => "DETAIL",
        Level::Failure => "FAILURE",
    };
    let mut left: Vec<Span> = vec![
        Span::raw(" "),
        Span::styled(
            format!(" {} ", pill_label),
            Style::default()
                .fg(Color::Black)
                .bg(pill_color)
                .add_modifier(Modifier::BOLD),
        ),
        Span::raw(" "),
        Span::styled(state.pkg_name.clone(), Style::default().fg(Color::DarkGray)),
    ];
    match level {
        Level::Detail => {
            if let Some(file) = state.selected_file() {
                left.push(Span::styled(" \u{203a} ", Style::default().fg(Color::DarkGray)));
                left.push(Span::styled(
                    file.name.clone(),
                    Style::default().add_modifier(Modifier::BOLD),
                ));
            }
        }
        Level::Failure => {
            if let Some(ff) = state.failures.get(state.nav.failure_cursor) {
                let loc = match ff.line {
                    Some(l) => format!("{}:{}", ff.file, l),
                    None    => ff.file.clone(),
                };
                left.push(Span::styled(" \u{203a} ", Style::default().fg(Color::DarkGray)));
                left.push(Span::styled(loc, Style::default().fg(Color::DarkGray)));
                left.push(Span::styled(" \u{203a} ", Style::default().fg(Color::DarkGray)));
                left.push(Span::styled(
                    ff.test.clone(),
                    Style::default().add_modifier(Modifier::BOLD),
                ));
                left.push(Span::styled(
                    format!("  ({}/{})", state.nav.failure_cursor + 1, state.failures.len()),
                    Style::default().fg(Color::DarkGray),
                ));
            }
        }
        Level::Normal => {}
    }

    // ── Right: metadata (workers, watch state, filters). ──────────────
    let mut right: Vec<Span> = Vec::new();
    let busy = state.run.busy_counter.as_ref().map(|b| b.get()).unwrap_or(0);
    right.push(Span::styled(
        format!("{}/{} workers", busy, state.n_workers),
        Style::default().fg(Color::DarkGray),
    ));
    right.push(Span::styled(" \u{00b7} ", Style::default().fg(Color::DarkGray)));
    let (status_txt, status_color) = if !state.display.watch_active {
        ("idle", Color::DarkGray)
    } else if state.display.watch_paused {
        ("paused", Color::Yellow)
    } else if state.run.running {
        ("running", Color::Yellow)
    } else if state.run.run_totals.fail > 0 || state.run.run_totals.error > 0 {
        ("watching", Color::Red)
    } else {
        ("watching", Color::Green)
    };
    right.push(Span::styled(status_txt, Style::default().fg(status_color)));

    if matches!(level, Level::Normal | Level::Detail) {
        if let Some(ref pat) = state.filter.active {
            right.push(Span::styled(" \u{00b7} ", Style::default().fg(Color::DarkGray)));
            right.push(Span::styled(format!("filter: {}", pat), Style::default().fg(Color::Yellow)));
        }
        if state.filter.status != StatusFilter::All {
            right.push(Span::styled(" \u{00b7} ", Style::default().fg(Color::DarkGray)));
            right.push(Span::styled(
                format!("status: {}", state.filter.status.label()),
                Style::default().fg(Color::Yellow),
            ));
        }
        if state.filter.suite.is_meaningful() && state.filter.suite.current.is_some() {
            right.push(Span::styled(" \u{00b7} ", Style::default().fg(Color::DarkGray)));
            right.push(Span::styled(
                format!("suite: {}", state.filter.suite.label()),
                Style::default().fg(Color::Yellow),
            ));
        }
    }

    // Compose left + spacer + right, truncating left if needed so the
    // metadata cluster always gets its full width.
    let total_w = area.width as usize;
    let right_w: usize = right.iter().map(|s| s.content.chars().count()).sum::<usize>() + 1;
    let left_cap = total_w.saturating_sub(right_w);

    let chunks = Layout::default()
        .direction(Direction::Horizontal)
        .constraints([
            Constraint::Max(left_cap as u16),
            Constraint::Length(right_w as u16),
        ])
        .split(area);
    f.render_widget(Paragraph::new(Line::from(left)), chunks[0]);
    f.render_widget(
        Paragraph::new(Line::from(right)).alignment(ratatui::layout::Alignment::Right),
        chunks[1],
    );
}
