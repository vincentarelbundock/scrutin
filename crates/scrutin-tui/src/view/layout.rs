//! Pane layout primitives: split rules, focus helpers, pane chrome,
//! centered-rect math, scroll adjustment.

use ratatui::layout::{Constraint, Direction, Layout, Rect};
use ratatui::style::{Color, Modifier, Style};
use ratatui::widgets::{Block, BorderType, Borders};

use crate::state::*;

/// Lazygit-style pane chrome: rounded border with an inline title.
/// Focused panes get a Cyan border; unfocused are DarkGray.
pub(super) fn pane_block<'a>(title: &'a str, focused: bool) -> Block<'a> {
    let color = if focused { Color::Cyan } else { Color::DarkGray };
    Block::default()
        .borders(Borders::ALL)
        .border_type(BorderType::Rounded)
        .border_style(Style::default().fg(color))
        .title(format!(" {title} "))
        .title_style(Style::default().fg(color).add_modifier(Modifier::BOLD))
}

/// Which of the two main panes is focused for the given mode. The file
/// list is focused in Normal; the main pane is focused in Detail and
/// Failure. Help/Log/Palette take over the screen and don't participate.
pub(super) fn list_focused(mode: &Mode) -> bool {
    matches!(mode, Mode::Normal)
}
pub(super) fn main_focused(mode: &Mode) -> bool {
    matches!(mode, Mode::Detail | Mode::Failure)
}

// ── Pane layout ────────────────────────────────────────────────────────
//
// Two-pane (list + main) screens decide how to lay themselves out from
// terminal width, focus, and screen_mode rather than a hardcoded breakpoint.
// On narrow terminals the layout collapses to a single pane (the focused
// one) instead of hiding content. Resize never loses state.

pub(super) enum PaneLayout {
    /// Only the main pane is visible (Full screen mode, or terminal too
    /// narrow to fit both panes side-by-side).
    Single { area: Rect },
    Split  { list: Rect, main: Rect },
}

/// Minimum row counts for a horizontal (stacked) split. Each pane is
/// wrapped in a rounded border, so 2 rows of every minimum are spent on
/// the top/bottom border lines and the inner usable area is `min - 2`.
pub(super) const MIN_LIST_ROWS: u16 = 6;
pub(super) const MIN_MAIN_ROWS: u16 = 7;

/// Decide how to split an area into list + main panes.
///
/// Rules:
///   - `list_pct == 0` \u2192 main pane only (the file/test list is hidden).
///   - Available space below `min_list + min_main` \u2192 main pane only.
///   - Otherwise split with the requested percentage, clamped so neither
///     side falls below its minimum.
pub(super) fn split_panes(area: Rect, list_pct: u16, horizontal: bool) -> PaneLayout {
    if list_pct == 0 {
        return PaneLayout::Single { area };
    }

    let (extent, min_list, min_main, direction) = if horizontal {
        (area.height, MIN_LIST_ROWS, MIN_MAIN_ROWS, Direction::Vertical)
    } else {
        (area.width, MIN_LIST_COLS, MIN_MAIN_COLS, Direction::Horizontal)
    };
    if extent < min_list + min_main {
        return PaneLayout::Single { area };
    }

    let mut list_extent = (extent as u32 * list_pct as u32 / 100) as u16;
    if list_extent < min_list {
        list_extent = min_list;
    }
    let mut main_extent = extent.saturating_sub(list_extent);
    if main_extent < min_main {
        main_extent = min_main;
        list_extent = extent.saturating_sub(main_extent);
    }

    let halves = Layout::default()
        .direction(direction)
        .constraints([Constraint::Length(list_extent), Constraint::Length(main_extent)])
        .split(area);

    PaneLayout::Split { list: halves[0], main: halves[1] }
}

/// Centered popup rect at `percent_x` \u{00d7} `percent_y` of `area`.
pub(super) fn centered_rect(percent_x: u16, percent_y: u16, area: Rect) -> Rect {
    let popup_layout = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Percentage((100 - percent_y) / 2),
            Constraint::Percentage(percent_y),
            Constraint::Percentage((100 - percent_y) / 2),
        ])
        .split(area);
    Layout::default()
        .direction(Direction::Horizontal)
        .constraints([
            Constraint::Percentage((100 - percent_x) / 2),
            Constraint::Percentage(percent_x),
            Constraint::Percentage((100 - percent_x) / 2),
        ])
        .split(popup_layout[1])[1]
}

/// Slide `scroll` so `cursor` stays inside `[scroll, scroll + visible_height)`.
pub(super) fn adjust_scroll(cursor: usize, scroll: &mut usize, visible_height: usize) {
    if visible_height == 0 {
        return;
    }
    if cursor < *scroll {
        *scroll = cursor;
    } else if cursor >= *scroll + visible_height {
        *scroll = cursor - visible_height + 1;
    }
}
