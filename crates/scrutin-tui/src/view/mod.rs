//! TUI rendering. Top-level dispatch + small chrome (draw_too_small);
//! everything else lives in topic-scoped sibling modules.
//!
//! Module map:
//!   layout.rs      \u{2014} split_panes, pane_block, centered_rect, scroll math
//!   icons.rs       \u{2014} outcome icon+color, file detail formatter
//!   source.rs      \u{2014} syntect-highlighted source windows
//!   sort.rs        \u{2014} test-list sort
//!   overlays.rs    \u{2014} palette/help/action-output overlays + ANSI strip
//!   file_list.rs   \u{2014} Files-mode left pane
//!   counts.rs      \u{2014} bottom counts bar
//!   hints.rs       \u{2014} bottom hints bar + mode chip
//!   breadcrumb.rs  \u{2014} top breadcrumb bar (consistent across all modes)
//!   log.rs         \u{2014} Mode::Log full-screen viewer
//!   normal.rs      \u{2014} Mode::Normal (Files level)
//!   detail.rs      \u{2014} Mode::Detail (per-test view)
//!   failure.rs     \u{2014} Mode::Failure (3-pane drill-in)

mod layout;
mod icons;
mod source;
mod sort;
mod overlays;
mod file_list;
mod counts;
mod hints;
mod breadcrumb;
mod log;
mod normal;
mod detail;
mod failure;

// External re-exports: only the entry points lib.rs/state.rs need.
pub(crate) use sort::sort_tests;

use ratatui::layout::Rect;
use ratatui::style::{Color, Style};
use ratatui::text::Line;
use ratatui::widgets::Paragraph;

use crate::keymap::PaletteKind;
use crate::state::*;

pub(super) fn draw(f: &mut ratatui::Frame, state: &mut AppState) {
    let area = f.area();
    if area.width < MIN_TERMINAL_COLS || area.height < MIN_TERMINAL_ROWS {
        draw_too_small(f, area);
        return;
    }
    // Per-frame: forget last frame's pane bounds. Each draw_* fn refills
    // whichever rects it actually places on screen this frame.
    state.pane_rects = PaneRects::default();
    match state.mode().clone() {
        Mode::Normal  => normal::draw_normal(f, state),
        Mode::Detail  => detail::draw_detail(f, state),
        Mode::Failure => failure::draw_failure(f, state),
        Mode::Help => {
            // Render Normal beneath \u{2014} the overlay covers most of the area.
            normal::draw_normal(f, state);
            overlays::draw_help_overlay(f, state);
        }
        Mode::Log => log::draw_log(f, state),
        Mode::ActionOutput => {
            normal::draw_normal(f, state);
            overlays::draw_action_output_overlay(f, state);
        }
        Mode::Palette(kind) => {
            // Draw the underlying drill level beneath the palette overlay.
            // `state.level()` ignores any overlay frames on top, so this
            // is just the level enum match.
            match state.level() {
                Level::Detail  => detail::draw_detail(f, state),
                Level::Failure => failure::draw_failure(f, state),
                Level::Normal  => normal::draw_normal(f, state),
            }
            match kind {
                PaletteKind::Filter => {} // filter input rides in the hints bar
                PaletteKind::Run    => overlays::draw_run_menu_overlay(f, state),
                PaletteKind::Sort   => overlays::draw_sort_menu_overlay(f, state),
                PaletteKind::Action => overlays::draw_action_menu_overlay(f, state),
            }
        }
    }
}

/// Rendered when the terminal is below the minimum usable size. Avoids
/// the crammed-borders garbled-output failure mode.
fn draw_too_small(f: &mut ratatui::Frame, area: Rect) {
    let msg = format!(
        "terminal too small\nneed {}\u{00d7}{}\nhave {}\u{00d7}{}",
        MIN_TERMINAL_COLS, MIN_TERMINAL_ROWS, area.width, area.height
    );
    let lines: Vec<Line> = msg.lines().map(|l| Line::from(l.to_string())).collect();
    let h = lines.len() as u16;
    let y = area.y + area.height.saturating_sub(h) / 2;
    let rect = Rect { x: area.x, y, width: area.width, height: h.min(area.height) };
    f.render_widget(
        Paragraph::new(lines).style(Style::default().fg(Color::Yellow)),
        rect,
    );
}
