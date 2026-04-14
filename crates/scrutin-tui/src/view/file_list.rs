//! The Files-mode left pane: list of test files with status icon, name,
//! per-file detail (counts + ms), and an optional duration bar.

use ratatui::layout::Rect;
use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{List, ListItem};

use crate::state::*;
use super::icons::format_file_detail;
use super::layout::{adjust_scroll, list_focused, pane_block};

pub(super) fn draw_file_list(f: &mut ratatui::Frame, state: &mut AppState, area: Rect) {
    let block = pane_block("Files", list_focused(state.mode()));
    let inner = block.inner(area);
    f.render_widget(block, area);
    let area = inner;
    let visible = state.visible_files();
    let height = area.height as usize;
    state.nav.file_list_height = height;

    if !visible.is_empty() && state.nav.file_cursor >= visible.len() {
        state.nav.file_cursor = visible.len() - 1;
    }
    adjust_scroll(state.nav.file_cursor, &mut state.nav.file_scroll, height);

    // Below FILE_DETAIL_MIN_COLS we drop the right-aligned per-file detail
    // (counts + bar) so the filename can use the full width instead of
    // being truncated.
    let show_detail = area.width >= FILE_DETAIL_MIN_COLS;
    let icon_cols: usize = 4; // " * \u{25cc} "
    let detail_reserve: usize = if show_detail { 30 } else { 0 };
    let name_width = visible
        .iter()
        .map(|&i| state.files[i].name.len())
        .max()
        .unwrap_or(20)
        .max(10)
        .min((area.width as usize).saturating_sub(icon_cols + detail_reserve).max(10));

    use unicode_width::UnicodeWidthStr;
    let detail_width = visible
        .iter()
        .map(|&i| match &state.files[i].status {
            FileStatus::Pending   => 0,
            FileStatus::Running   => "running...".width(),
            FileStatus::Cancelled => "cancelled".width(),
            FileStatus::Skipped { skipped, ms } => format!("{} skip  {}ms", skipped, ms).width(),
            FileStatus::Passed { passed, warned, ms } =>
                format_file_detail(*passed, 0, 0, *warned, *ms).width(),
            FileStatus::Failed { passed, failed, errored, warned, ms } =>
                format_file_detail(*passed, *failed, *errored, *warned, *ms).width(),
        })
        .max()
        .unwrap_or(0);

    let max_ms = if state.display.show_duration_bars {
        visible
            .iter()
            .map(|&i| match &state.files[i].status {
                FileStatus::Passed { ms, .. } | FileStatus::Failed { ms, .. } | FileStatus::Skipped { ms, .. } => *ms,
                _ => 0,
            })
            .max()
            .unwrap_or(1)
            .max(1)
    } else { 1 };

    let end = (state.nav.file_scroll + height).min(visible.len());
    let items: Vec<ListItem> = visible[state.nav.file_scroll..end]
        .iter()
        .enumerate()
        .map(|(row, &i)| {
            let vi = state.nav.file_scroll + row;
            let entry = &state.files[i];
            let (icon, color) = match &entry.status {
                FileStatus::Pending => ("\u{25cb}", Color::DarkGray),
                FileStatus::Running => ("\u{25cc}", Color::Yellow),
                FileStatus::Passed { warned, .. } if *warned > 0 => ("\u{25cf}", Color::Yellow),
                FileStatus::Passed   { .. } => ("\u{25cf}", Color::Green),
                FileStatus::Failed   { .. } => ("\u{25cf}", Color::Red),
                FileStatus::Skipped  { .. } => ("\u{25cb}", Color::DarkGray),
                FileStatus::Cancelled       => ("\u{2298}", Color::DarkGray),
            };

            let ms = match &entry.status {
                FileStatus::Passed { ms, .. } | FileStatus::Failed { ms, .. } | FileStatus::Skipped { ms, .. } => *ms,
                _ => 0,
            };

            let detail = match &entry.status {
                FileStatus::Pending   => String::new(),
                FileStatus::Running   => "running...".to_string(),
                FileStatus::Cancelled => "cancelled".to_string(),
                FileStatus::Skipped { skipped, ms } => format!("{} skip  {}ms", skipped, ms),
                FileStatus::Passed { passed, warned, ms } =>
                    format_file_detail(*passed, 0, 0, *warned, *ms),
                FileStatus::Failed { passed, failed, errored, warned, ms } =>
                    format_file_detail(*passed, *failed, *errored, *warned, *ms),
            };

            let bar = if state.display.show_duration_bars && ms > 0 {
                let width = ((ms as f64 / max_ms as f64) * 15.0).ceil() as usize;
                let full = width.min(15);
                format!(" {}", "\u{2588}".repeat(full))
            } else { String::new() };

            let is_selected = state.multi.selected.contains(&entry.path);
            let mut style = if vi == state.nav.file_cursor {
                Style::default().add_modifier(Modifier::REVERSED)
            } else { Style::default() };
            if is_selected {
                style = style.fg(Color::Cyan).add_modifier(Modifier::BOLD);
            }
            let marker = if is_selected { "*" } else { " " };
            let mut spans = vec![
                Span::styled(
                    format!(" {}{} ", marker, icon),
                    Style::default().fg(if is_selected { Color::Cyan } else { color }),
                ),
                Span::styled(format!("{:<width$}", entry.name, width = name_width), style),
            ];
            if entry.attempt > 0 {
                spans.push(Span::styled(
                    format!(" [r{}/{}]", entry.attempt, state.rerun_max),
                    Style::default().fg(Color::Yellow),
                ));
            }
            if entry.flaky {
                spans.push(Span::styled(
                    " ~flaky",
                    Style::default().fg(Color::Yellow).add_modifier(Modifier::BOLD),
                ));
            }
            if show_detail {
                let dw = detail.width();
                let pad = if dw < detail_width { detail_width - dw } else { 0 };
                spans.push(Span::styled(
                    format!(" {}{}", detail, " ".repeat(pad)),
                    Style::default().fg(Color::DarkGray),
                ));
                spans.push(Span::styled(bar, Style::default().fg(color)));
            }
            ListItem::new(Line::from(spans))
        })
        .collect();

    f.render_widget(List::new(items), area);
}
