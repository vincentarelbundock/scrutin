//! Bottom hints bar: mode chip + auto-generated key hints (from the
//! active mode's binding table) + selection counter.

use ratatui::layout::Rect;
use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::Paragraph;

use crate::keymap::{self, Binding, PaletteKind};
use crate::state::*;

pub(super) fn draw_hints_bar(f: &mut ratatui::Frame, state: &AppState, area: Rect) {
    // Filter palette piggybacks on Normal's chrome but rewrites the hints
    // bar into a text-input prompt.
    if matches!(state.mode(), Mode::Palette(PaletteKind::Filter)) {
        let chip = mode_chip_span(state.mode());
        let prompt = format!(
            " Filter: {}\u{258e}    Enter confirm  Esc cancel",
            state.filter.input
        );
        f.render_widget(
            Paragraph::new(Line::from(vec![
                chip,
                Span::raw(" "),
                Span::styled(prompt, Style::default().fg(Color::DarkGray)),
            ])),
            area,
        );
        return;
    }
    let mut spans: Vec<Span> = Vec::new();
    spans.push(mode_chip_span(state.mode()));
    spans.push(Span::raw(" "));
    if state.multi.visual_anchor.is_some() {
        spans.push(Span::styled(
            " -- VISUAL -- ",
            Style::default()
                .fg(Color::Black)
                .bg(Color::Cyan)
                .add_modifier(Modifier::BOLD),
        ));
        spans.push(Span::raw(" "));
    }
    let hints = generate_hints_bar_text(state, state.mode(), state.run.running);
    spans.push(Span::styled(hints, Style::default().fg(Color::DarkGray)));
    if !state.multi.selected.is_empty() {
        let total = state.multi.selected.len();
        let visible_sel = state
            .visible_files()
            .into_iter()
            .filter(|&i| state.multi.selected.contains(&state.files[i].path))
            .count();
        let label = if visible_sel == total {
            format!("  [{} selected]", total)
        } else {
            format!("  [{}/{} selected, {} hidden]", visible_sel, total, total - visible_sel)
        };
        spans.push(Span::styled(label, Style::default().fg(Color::Cyan)));
    }
    f.render_widget(Paragraph::new(Line::from(spans)), area);
}

/// Format a binding's key chord for the help overlay ("Ctrl-d", "j", "\u{21b5}").
pub(super) fn format_key(b: &Binding) -> String {
    use crossterm::event::{KeyCode, KeyModifiers};
    let base = match b.key {
        KeyCode::Char(' ')   => "\u{2423}".to_string(),
        KeyCode::Char(c)     => c.to_string(),
        KeyCode::Enter       => "\u{21b5}".to_string(),
        KeyCode::Esc         => "Esc".to_string(),
        KeyCode::Up          => "\u{2191}".to_string(),
        KeyCode::Down        => "\u{2193}".to_string(),
        KeyCode::Left        => "\u{2190}".to_string(),
        KeyCode::Right       => "\u{2192}".to_string(),
        KeyCode::Home        => "Home".to_string(),
        KeyCode::End         => "End".to_string(),
        KeyCode::PageUp      => "PgUp".to_string(),
        KeyCode::PageDown    => "PgDn".to_string(),
        KeyCode::Backspace   => "BS".to_string(),
        KeyCode::Tab         => "Tab".to_string(),
        _                    => format!("{:?}", b.key),
    };
    if b.mods.contains(KeyModifiers::CONTROL)   { format!("C-{}", base) }
    else if b.mods.contains(KeyModifiers::SHIFT) { format!("S-{}", base) }
    else                                         { base }
}

/// Colored, bold mode chip shown at the start of every hints bar.
pub(super) fn mode_chip_span(mode: &Mode) -> Span<'static> {
    let label = format!(" [{}] ", keymap::mode_label(mode));
    Span::styled(
        label,
        Style::default()
            .fg(Color::Black)
            .bg(keymap::mode_color(mode))
            .add_modifier(Modifier::BOLD),
    )
}

/// Space-joined hints for the footer bar. Uses `bar_desc` (the tight
/// subset), not `desc` (which populates the full help overlay).
pub(super) fn generate_hints_bar_text(state: &AppState, mode: &Mode, running: bool) -> String {
    use keymap::Visibility;
    let mut seen = std::collections::HashSet::new();
    let mut parts: Vec<String> = state
        .effective_bindings(mode)
        .iter()
        .filter(|b| !b.bar_desc.is_empty())
        .filter(|b| match b.visible {
            Visibility::Always       => true,
            Visibility::WhenRunning  => running,
            Visibility::WhenIdle     => !running,
        })
        .filter(|b| seen.insert(b.action.clone()))
        .map(|b| b.bar_desc.to_string())
        .collect();
    // Plugin action menu hint (not in the shared keymap).
    if matches!(mode, Mode::Normal | Mode::Detail)
        && state.selected_plugin_actions().is_some_and(|a| !a.is_empty())
    {
        parts.push("a actions".to_string());
    }
    if parts.is_empty() {
        return String::new();
    }
    parts.insert(0, String::new()); // leading space
    parts.join("  ")
}
