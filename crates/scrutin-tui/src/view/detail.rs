//! Detail mode: per-test list with source/error preview.

use ratatui::layout::{Constraint, Direction, Layout, Rect};
use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Borders, List, ListItem, Paragraph};

use crate::state::*;
use super::breadcrumb::draw_breadcrumb_bar;
use super::hints::format_key;
use super::icons::test_icon_color;
use super::layout::{adjust_scroll, split_panes, PaneLayout};
use super::sort::sort_tests;
use super::source::load_source_context_ex;

pub(super) fn draw_detail(f: &mut ratatui::Frame, state: &mut AppState) {
    let list_pct = state.current_list_pct();
    let horizontal = state.current_horizontal();
    let preview_height = Constraint::Length(0);

    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Length(1),
            Constraint::Min(3),
            preview_height,
            Constraint::Length(1),
        ])
        .split(f.area());

    draw_breadcrumb_bar(f, state, chunks[0]);

    // Clone tests to avoid borrow conflict between body split and item builder.
    let mut tests: Vec<TestEntry> = state
        .selected_file()
        .map(|f| f.tests.clone())
        .unwrap_or_default();
    sort_tests(&mut tests, state.display.test_sort_mode, state.display.test_sort_reversed);
    let test_height = chunks[1].height as usize;
    state.nav.test_list_height = test_height;
    if !tests.is_empty() && state.nav.test_cursor >= tests.len() {
        state.nav.test_cursor = tests.len() - 1;
    }
    adjust_scroll(state.nav.test_cursor, &mut state.nav.test_scroll, test_height);

    // No tests \u2192 informative message + key hints derived from the *effective*
    // Detail-mode bindings so user remappings via [keymap.detail] flow through.
    if tests.is_empty() {
        let file_name = state.selected_file().map(|f| f.name.clone()).unwrap_or_default();
        let status_label = state
            .selected_file()
            .map(|f| match f.status {
                FileStatus::Pending   => "not run yet",
                FileStatus::Cancelled => "run was cancelled",
                _                     => "no tests",
            })
            .unwrap_or("no tests");
        let key_for = |action: &str| -> String {
            state
                .effective_bindings(&Mode::Detail)
                .iter()
                .find(|b| b.action.name() == action)
                .map(format_key)
                .unwrap_or_else(|| "(unbound)".to_string())
        };
        let run_key  = key_for("run_current_file");
        let edit_key = key_for("open_editor");
        let back_key = key_for("pop");
        let body_area = chunks[1];
        let dim = Style::default().fg(Color::DarkGray);
        let lines = vec![
            Line::from(""),
            Line::from(Span::styled(
                format!("  {}  \u{2014}  {}", file_name, status_label),
                Style::default().fg(Color::Yellow).add_modifier(Modifier::BOLD),
            )),
            Line::from(""),
            Line::from(Span::styled(format!("  {:<6} run this file", run_key), dim)),
            Line::from(Span::styled(format!("  {:<6} edit in $EDITOR", edit_key), dim)),
            Line::from(Span::styled(format!("  {:<6} back", back_key), dim)),
        ];
        f.render_widget(Paragraph::new(lines), body_area);
        state.nav.source_scroll_max = 0;
        draw_detail_status_bar(f, chunks[3]);
        return;
    }

    let test_end = (state.nav.test_scroll + test_height).min(tests.len());
    let items = build_detail_test_items(&tests, state.nav.test_scroll, test_end, state.nav.test_cursor);

    match split_panes(chunks[1], list_pct, horizontal) {
        PaneLayout::Split { list, main } => {
            state.pane_rects.list = Some(list);
            state.pane_rects.main = Some(main);
            f.render_widget(List::new(items), list);
            render_detail_main_pane(f, state, &tests, main);
        }
        PaneLayout::Single { area } => {
            state.pane_rects.main = Some(area);
            render_detail_main_pane(f, state, &tests, area);
        }
    }

    draw_detail_status_bar(f, chunks[3]);
}

fn build_detail_test_items(
    tests: &[TestEntry],
    scroll: usize,
    end: usize,
    cursor: usize,
) -> Vec<ListItem<'static>> {
    tests[scroll..end]
        .iter()
        .enumerate()
        .map(|(row, t)| {
            let i = scroll + row;
            let (icon, color) = test_icon_color(t);
            let style = if i == cursor {
                Style::default().add_modifier(Modifier::REVERSED)
            } else {
                Style::default()
            };
            let name = if t.name.is_empty() { format!("test {}", i + 1) } else { t.name.clone() };
            let ms_str = if t.duration_ms > 0 { format!("  {}ms", t.duration_ms) } else { String::new() };
            ListItem::new(Line::from(vec![
                Span::styled(format!("  {} ", icon), Style::default().fg(color)),
                Span::styled(name, style),
                Span::styled(ms_str, Style::default().fg(Color::DarkGray)),
            ]))
        })
        .collect()
}

fn render_detail_main_pane(
    f: &mut ratatui::Frame,
    state: &AppState,
    tests: &[TestEntry],
    area: Rect,
) {
    let file_path = state.selected_file().map(|f| f.path.clone());
    let cur_test = tests.get(state.nav.test_cursor);
    let cur_line = cur_test.and_then(|t| t.line);
    let msg = cur_test.map(|t| t.message.clone()).unwrap_or_default();
    let base_title = if !msg.is_empty() {
        if cur_test.is_some_and(|t| !t.is_bad()) { "Warning" } else { "Failure" }
    } else {
        "Source"
    };
    let block = Block::default().borders(Borders::LEFT).title(format!(" {} ", base_title));
    let inner = block.inner(area);
    f.render_widget(block, area);

    let mut content: Vec<Line> = Vec::new();
    if !msg.is_empty() {
        let color = if cur_test.is_some_and(|t| !t.is_bad()) { Color::Yellow } else { Color::Red };
        for ml in msg.lines() {
            content.push(Line::from(Span::styled(ml.to_string(), Style::default().fg(color))));
        }
        content.push(Line::from(""));
    }
    let auto = (inner.height as usize).saturating_sub(content.len());
    let remaining = if state.nav.source_context_lines > 0 {
        state.nav.source_context_lines.min(auto)
    } else {
        auto
    };
    if let Some(path) = file_path
        && remaining > 0
    {
        content.extend(load_source_context_ex(
            &path, cur_line, remaining,
            state.nav.source_scroll, state.nav.source_hscroll,
        ));
    }
    f.render_widget(Paragraph::new(content), inner);
}

fn draw_detail_status_bar(f: &mut ratatui::Frame, area: Rect) {
    let line = Line::from(Span::styled(
        " j/k navigate  Enter expand failure  e edit  d run file  Esc back  ? help",
        Style::default().fg(Color::DarkGray),
    ));
    f.render_widget(Paragraph::new(line), area);
}
