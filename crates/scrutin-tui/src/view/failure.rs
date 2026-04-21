//! Failure mode: 3-pane Test source / dep-mapped Source / Error layout
//! with a global failure carousel at the top (j/k navigates).

use std::path::Path;

use ratatui::layout::{Constraint, Direction, Layout, Rect};
use ratatui::style::{Color, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Paragraph, Wrap};

use crate::find_source_for_test;
use crate::state::{self, *};
use super::breadcrumb::draw_breadcrumb_bar;
use super::hints::draw_notice_bar;
use super::layout::pane_block;
use super::source::load_source_context;
use state::{MIN_LIST_COLS, MIN_MAIN_COLS};

pub(super) fn draw_failure(f: &mut ratatui::Frame, state: &mut AppState) {
    let Some(failure) = state.failures.get(state.nav.failure_cursor) else { return };
    let failure_message   = failure.message.clone();
    let failure_line      = failure.line;
    let failure_file_path = failure.file_path.clone();
    let failure_file_name = failure.file.clone();

    let notice = state.active_notice();
    let show_notice = f.area().height >= HINTS_BAR_MIN_ROWS && notice.is_some();
    let show_hints = f.area().height >= HINTS_BAR_MIN_ROWS;
    let mut constraints: Vec<Constraint> = vec![Constraint::Length(1), Constraint::Min(5)];
    if show_notice { constraints.push(Constraint::Length(1)); }
    if show_hints  { constraints.push(Constraint::Length(1)); }
    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints(constraints)
        .split(f.area());

    draw_breadcrumb_bar(f, state, chunks[0]);

    // Three-region body: test source (left) | source function (right) over
    // error (bottom). On narrow terminals the top split collapses (no fn
    // pane). On very short terminals the bottom error pane is dropped.
    let body = chunks[1];
    let show_bottom = body.height >= 10;
    let vert_chunks: Vec<Rect> = if show_bottom {
        Layout::default()
            .direction(Direction::Vertical)
            .constraints([Constraint::Percentage(60), Constraint::Percentage(40)])
            .split(body)
            .to_vec()
    } else {
        vec![body]
    };
    let top = vert_chunks[0];
    let show_fn_pane = top.width >= MIN_LIST_COLS + 1 + MIN_MAIN_COLS;
    let horiz_chunks: Vec<Rect> = if show_fn_pane {
        Layout::default()
            .direction(Direction::Horizontal)
            .constraints([Constraint::Percentage(50), Constraint::Percentage(50)])
            .split(top)
            .to_vec()
    } else {
        vec![top]
    };

    let pane_height = horiz_chunks[0].height.saturating_sub(2) as usize;
    state.nav.failure_view_height = pane_height;
    draw_failure_test_pane(f, horiz_chunks[0], &failure_file_path, failure_line, pane_height);
    if show_fn_pane {
        draw_failure_source_fn_pane(f, state, horiz_chunks[1], &failure_file_name, pane_height);
    }

    if show_bottom {
        state.pane_rects.failure_error = Some(vert_chunks[1]);
        draw_failure_error_pane(f, state, vert_chunks[1], &failure_message);
    } else {
        state.nav.failure_scroll = 0;
    }

    let mut chrome_idx = 2;
    if show_notice {
        draw_notice_bar(f, notice.as_deref().unwrap_or(""), chunks[chrome_idx]);
        chrome_idx += 1;
    }
    if show_hints {
        let line = Line::from(Span::styled(
            " j/k next/prev  e edit test  s edit source  Esc back",
            Style::default().fg(Color::DarkGray),
        ));
        f.render_widget(Paragraph::new(line), chunks[chrome_idx]);
    }
}

fn draw_failure_test_pane(
    f: &mut ratatui::Frame,
    area: Rect,
    file_path: &Path,
    line: Option<u32>,
    pane_height: usize,
) {
    let source_lines = load_source_context(file_path, line, pane_height);
    let widget = Paragraph::new(source_lines).block(pane_block("Test", true));
    f.render_widget(widget, area);
}

fn draw_failure_source_fn_pane(
    f: &mut ratatui::Frame,
    state: &AppState,
    area: Rect,
    file_name: &str,
    pane_height: usize,
) {
    let source_path = find_source_for_test(file_name, &state.pkg_root, &state.reverse_dep_map);
    let (title, lines) = match source_path.as_ref() {
        Some(path) => (
            path.file_name().unwrap_or_default().to_string_lossy().to_string(),
            load_source_context(path, None, pane_height),
        ),
        None => (
            "Source".to_string(),
            vec![Line::from("  (no source mapping)")],
        ),
    };
    let widget = Paragraph::new(lines).block(pane_block(&title, false));
    f.render_widget(widget, area);
}

fn draw_failure_error_pane(
    f: &mut ratatui::Frame,
    state: &mut AppState,
    area: Rect,
    message: &str,
) {
    let error_lines: Vec<Line> = message
        .lines()
        .map(|l| Line::from(Span::styled(l.to_string(), Style::default().fg(Color::Red))))
        .collect();
    let total = error_lines.len();
    let visible_h = (area.height as usize).saturating_sub(2);
    let max_scroll = total.saturating_sub(visible_h.max(1));
    if state.nav.failure_scroll > max_scroll {
        state.nav.failure_scroll = max_scroll;
    }
    let widget = Paragraph::new(error_lines)
        .block(pane_block("Error", true))
        .wrap(Wrap { trim: false })
        .scroll((state.nav.failure_scroll as u16, 0));
    f.render_widget(widget, area);
}
