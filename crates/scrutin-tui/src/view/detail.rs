//! Detail mode: per-test list with source/error preview.

use ratatui::layout::{Constraint, Direction, Layout, Rect};
use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Borders, List, ListItem, Paragraph};

use crate::state::*;
use super::breadcrumb::draw_breadcrumb_bar;
use super::hints::{draw_notice_bar, format_key};
use super::icons::test_icon_color;
use super::layout::{adjust_scroll, split_panes, PaneLayout};
use super::sort::sort_tests;
use super::source::load_source_context_ex;

pub(super) fn draw_detail(f: &mut ratatui::Frame, state: &mut AppState) {
    let list_pct = state.current_list_pct();
    let horizontal = state.current_horizontal();
    let notice = state.active_notice();
    let show_notice = f.area().height >= HINTS_BAR_MIN_ROWS && notice.is_some();

    let mut constraints: Vec<Constraint> = vec![
        Constraint::Length(1), // header
        Constraint::Min(3),    // body
        Constraint::Length(0), // preview (unused)
    ];
    if show_notice { constraints.push(Constraint::Length(1)); }
    constraints.push(Constraint::Length(1)); // status bar

    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints(constraints)
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
        let bar_base = 3 + usize::from(show_notice);
        if show_notice { draw_notice_bar(f, notice.as_deref().unwrap_or(""), chunks[3]); }
        draw_detail_status_bar(f, chunks[bar_base]);
        return;
    }

    let test_end = (state.nav.test_scroll + test_height).min(tests.len());
    let items = build_detail_test_items(&tests, state.nav.test_scroll, test_end, state.nav.test_cursor);

    match split_panes(chunks[1], list_pct, horizontal) {
        PaneLayout::Split { list, main } => {
            state.pane_rects.list = Some(list);
            state.pane_rects.main = Some(main);
            state.nav.test_list_top = list.y;
            f.render_widget(List::new(items), list);
            render_detail_main_pane(f, state, &tests, main);
        }
        PaneLayout::Single { area } => {
            state.pane_rects.main = Some(area);
            render_detail_main_pane(f, state, &tests, area);
        }
    }

    let bar_base = 3 + usize::from(show_notice);
    if show_notice { draw_notice_bar(f, notice.as_deref().unwrap_or(""), chunks[3]); }
    draw_detail_status_bar(f, chunks[bar_base]);
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
    // Spell-check: suggestions rendered as a horizontal chip grid wrapping
    // to pane width, so a word like `excercise` with 9 candidates fits in
    // one or two lines instead of a 10-row stack. Press [N] to accept,
    // [0] to whitelist.
    let has_corrections = cur_test.is_some_and(|t| !t.corrections.is_empty());
    if let Some(correction) = cur_test.and_then(|t| t.corrections.first()) {
        content.push(Line::from(Span::styled(
            "Replace with:",
            Style::default().fg(Color::DarkGray),
        )));
        let pane_width = inner.width as usize;
        let indent = "  ";
        let mut spans: Vec<Span> = vec![Span::raw(indent)];
        let mut width: usize = indent.len();
        for (i, sug) in correction.suggestions.iter().take(9).enumerate() {
            let n = i + 1;
            let is_best = i == 0;
            // "[N] word" + optional " \u{2605}" star for the best match.
            let chip_text_len = 4 + sug.chars().count() + if is_best { 2 } else { 0 };
            let sep_len = if spans.len() > 1 { 3 } else { 0 };
            if width + sep_len + chip_text_len > pane_width && spans.len() > 1 {
                content.push(Line::from(std::mem::take(&mut spans)));
                spans.push(Span::raw(indent));
                width = indent.len();
            }
            if spans.len() > 1 {
                spans.push(Span::raw("   "));
                width += 3;
            }
            spans.push(Span::styled(
                format!("[{}]", n),
                Style::default().fg(Color::Cyan),
            ));
            spans.push(Span::raw(" "));
            let word_style = if is_best {
                Style::default()
                    .fg(Color::Green)
                    .add_modifier(Modifier::BOLD)
            } else {
                Style::default()
            };
            spans.push(Span::styled(sug.clone(), word_style));
            if is_best {
                spans.push(Span::styled(
                    " \u{2605}",
                    Style::default().fg(Color::Green),
                ));
            }
            width += chip_text_len;
        }
        if spans.len() > 1 {
            content.push(Line::from(spans));
        }
        content.push(Line::from(""));
        content.push(Line::from(vec![
            Span::styled("[0]", Style::default().fg(Color::Cyan)),
            Span::raw(" "),
            Span::styled(
                format!("Add \u{201c}{}\u{201d} to dictionary", correction.word),
                Style::default().fg(Color::Magenta),
            ),
        ]));
        content.push(Line::from(""));
    }

    // Plugin-level actions (ruff/jarl fix variants) as a chip grid. Only
    // shown when the cursor file's suite has actions AND the current event
    // isn't a spell-check one (whose digits 1-9 are already bound). Each
    // chip corresponds to `[N] <label>` and is invoked by pressing N.
    let actions = state
        .selected_file()
        .and_then(|f| state.suite_actions.get(&f.suite))
        .filter(|v| !v.is_empty())
        .filter(|_| !has_corrections);
    if let Some(actions) = actions {
        content.push(Line::from(Span::styled(
            "Actions:",
            Style::default().fg(Color::DarkGray),
        )));
        let pane_width = inner.width as usize;
        let indent = "  ";
        let mut spans: Vec<Span> = vec![Span::raw(indent)];
        let mut width: usize = indent.len();
        for (i, action) in actions.iter().take(9).enumerate() {
            let n = i + 1;
            let chip_text_len = 4 + action.label.chars().count();
            let sep_len = if spans.len() > 1 { 3 } else { 0 };
            if width + sep_len + chip_text_len > pane_width && spans.len() > 1 {
                content.push(Line::from(std::mem::take(&mut spans)));
                spans.push(Span::raw(indent));
                width = indent.len();
            }
            if spans.len() > 1 {
                spans.push(Span::raw("   "));
                width += 3;
            }
            spans.push(Span::styled(
                format!("[{}]", n),
                Style::default().fg(Color::Cyan),
            ));
            spans.push(Span::raw(" "));
            spans.push(Span::raw(action.label.to_string()));
            width += chip_text_len;
        }
        if spans.len() > 1 {
            content.push(Line::from(spans));
        }
        content.push(Line::from(""));
    }
    // Source-context window. Warnings (skyspell suggestions, ruff/jarl
    // lint diagnostics) get a compact 7-line snippet centered on the
    // flagged line: just enough to see the quote in situ without pushing
    // the chip row off screen. Failures and errors instead fill whatever
    // pane space is left, which is what the user wants when they're
    // reading failing code.
    let is_warning = cur_test.is_some_and(|t| matches!(
        t.outcome,
        scrutin_core::engine::protocol::Outcome::Warn
    ));
    let auto = (inner.height as usize).saturating_sub(content.len());
    let remaining = if state.nav.source_context_lines > 0 {
        state.nav.source_context_lines.min(auto)
    } else if is_warning {
        7.min(auto)
    } else {
        auto
    };
    if let Some(path) = file_path
        && remaining > 0
    {
        if is_warning {
            content.push(Line::from(Span::styled(
                "Context:",
                Style::default().fg(Color::DarkGray),
            )));
        }
        content.extend(load_source_context_ex(
            &path, cur_line, remaining,
            state.nav.source_scroll, state.nav.source_hscroll,
        ));
    }
    f.render_widget(Paragraph::new(content), inner);
}

fn draw_detail_status_bar(f: &mut ratatui::Frame, area: Rect) {
    let line = Line::from(Span::styled(
        " \u{2191}\u{2193}/jk navigate  Enter expand failure  1-9 accept  0 add-to-dict  e edit  d run file  Esc back  ? help",
        Style::default().fg(Color::DarkGray),
    ));
    f.render_widget(Paragraph::new(line), area);
}
