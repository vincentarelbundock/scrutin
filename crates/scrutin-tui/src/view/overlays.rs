//! Overlay rendering: palette menus (Run/Sort/Action), text overlays
//! (Help/ActionOutput), and ANSI-stripping for action output.

use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Borders, Clear, Paragraph};

use crate::state::*;
use super::layout::{centered_rect, pane_block};

/// A row in a palette overlay. All three palette types (Run, Sort, Action)
/// share this structure and the same rendering logic.
pub(super) struct PaletteRow {
    pub label: String,
    pub desc: String,
    pub enabled: bool,
    pub active: bool,
}

/// Draw a centered palette overlay with a title, rows, and a footer.
pub(super) fn draw_palette_overlay(
    f: &mut ratatui::Frame,
    title: &str,
    rows: &[PaletteRow],
    footer: &str,
    cursor: usize,
) {
    if rows.is_empty() {
        return;
    }
    let dim = Style::default().fg(Color::DarkGray);
    let active_style = Style::default().fg(Color::Green);
    let max_label = rows.iter().map(|r| r.label.len()).max().unwrap_or(0);
    let max_row_w = rows
        .iter()
        .map(|r| 1 + max_label + 2 + r.desc.len() + 1)
        .max()
        .unwrap_or(0)
        .max(footer.len() + 1);
    let area = f.area();
    let w = (max_row_w as u16 + 2).min(area.width);
    let h = (rows.len() as u16 + 4).min(area.height);
    let x = area.x + area.width.saturating_sub(w) / 2;
    let y = area.y + area.height.saturating_sub(h) / 2;
    let rect = ratatui::layout::Rect { x, y, width: w, height: h };
    f.render_widget(Clear, rect);
    let block = Block::default().borders(Borders::ALL).title(format!(" {} ", title));
    let mut lines: Vec<Line> = rows
        .iter()
        .enumerate()
        .map(|(i, row)| {
            let base = if !row.enabled { dim }
                else if row.active { active_style }
                else { Style::default() };
            let style = if i == cursor { base.add_modifier(Modifier::REVERSED) } else { base };
            Line::from(vec![
                Span::styled(format!(" {:<width$}", row.label, width = max_label), style),
                Span::styled(format!("  {}", row.desc), dim),
            ])
        })
        .collect();
    lines.push(Line::from(""));
    lines.push(Line::from(Span::styled(format!(" {}", footer), dim)));
    f.render_widget(Paragraph::new(lines).block(block), rect);
}

pub(super) fn draw_run_menu_overlay(f: &mut ratatui::Frame, state: &AppState) {
    let visible = state.visible_files();
    let n_visible = visible.len();
    let n_all = state.files.len();
    let n_selected = visible
        .iter()
        .filter(|&&i| state.multi.selected.contains(&state.files[i].path))
        .count();
    let n_failed = state
        .files
        .iter()
        .filter(|e| matches!(e.status, FileStatus::Failed { .. }))
        .count();
    let git_reason = state.git.disabled_reason();

    let mut rows = vec![
        PaletteRow { label: "a  all".into(), desc: format!("({})", n_all), enabled: n_all > 0, active: false },
        PaletteRow {
            label: "v  visible".into(),
            desc: if n_visible == n_all { "(no filter active)".into() } else { format!("({})", n_visible) },
            enabled: n_visible > 0, active: false,
        },
        PaletteRow {
            label: "s  selected".into(),
            desc: if n_selected == 0 { "(none selected)".into() } else { format!("({})", n_selected) },
            enabled: n_selected > 0, active: false,
        },
        PaletteRow {
            label: "f  failed".into(),
            desc: if n_failed == 0 { "(no failures)".into() } else { format!("({})", n_failed) },
            enabled: n_failed > 0, active: false,
        },
        PaletteRow {
            label: "u  uncommitted".into(),
            desc: git_reason.unwrap_or_default(),
            enabled: state.git.disabled_reason().is_none(), active: false,
        },
    ];
    for g in &state.run_groups {
        rows.push(PaletteRow { label: format!("   group: {}", g.name), desc: String::new(), enabled: true, active: false });
    }
    draw_palette_overlay(f, "Run", &rows, "j/k move  Enter run  Esc cancel", state.overlay.cursor_pos());
}

pub(super) fn draw_sort_menu_overlay(f: &mut ratatui::Frame, state: &AppState) {
    let modes = SortMode::ALL;
    let in_detail = state.nav.mode_stack.len() >= 2
        && matches!(state.nav.mode_stack[state.nav.mode_stack.len() - 2], Mode::Detail);
    let active_mode = if in_detail { state.display.test_sort_mode } else { state.display.sort_mode };
    let reversed = if in_detail { state.display.test_sort_reversed } else { state.display.sort_reversed };
    let title = if in_detail { "Sort tests" } else { "Sort files" };

    let rows: Vec<PaletteRow> = modes
        .iter()
        .map(|m| {
            let is_active = *m == active_mode;
            let arrow = if is_active { if reversed { " \u{2193}" } else { " \u{2191}" } } else { "" };
            PaletteRow {
                label: m.label().into(),
                desc: format!("{}{}", m.description(), arrow),
                enabled: true,
                active: is_active,
            }
        })
        .collect();
    draw_palette_overlay(f, title, &rows, "j/k move  Enter select/reverse  Esc close", state.overlay.cursor_pos());
}

pub(super) fn draw_help_overlay(f: &mut ratatui::Frame, state: &mut AppState) {
    use scrutin_core::keymap::{DEFAULT_KEYMAP, Level};

    let area = centered_rect(70, 95, f.area());
    f.render_widget(Clear, area);

    let heading = |text: &str| -> Line<'static> {
        Line::from(Span::styled(format!(" {}", text), Style::default().fg(Color::Cyan)))
    };

    let help_row = |b: &scrutin_core::keymap::KeyBinding| -> String {
        let sp = b.help.find(' ').unwrap_or(b.help.len());
        format!("   {:<12} {}", &b.help[..sp], &b.help[sp..].trim_start())
    };
    let is_shared = |b: &scrutin_core::keymap::KeyBinding| -> bool {
        b.levels.contains(&Level::Files) && b.levels.contains(&Level::Detail)
    };

    let mut help_text: Vec<Line> = vec![
        Line::from(Span::styled(" Keybindings", Style::default().add_modifier(Modifier::BOLD))),
        Line::from(""),
    ];

    // Shared bindings.
    let mut seen = std::collections::HashSet::new();
    for b in DEFAULT_KEYMAP {
        if b.help.is_empty() || !seen.insert(b.action) { continue; }
        if !is_shared(b) { continue; }
        help_text.push(Line::from(help_row(b)));
    }
    help_text.push(Line::from(""));

    // Per-level sections.
    let sections: &[(&str, Level)] = &[
        ("Files only", Level::Files),
        ("Detail / Failure only", Level::Detail),
    ];
    for (title, level) in sections {
        let mut section_seen = std::collections::HashSet::new();
        let mut rows: Vec<String> = Vec::new();
        for b in DEFAULT_KEYMAP {
            if b.help.is_empty() || !section_seen.insert(b.action) { continue; }
            if !b.levels.contains(level) { continue; }
            if is_shared(b) { continue; }
            rows.push(help_row(b));
        }
        if !rows.is_empty() {
            help_text.push(heading(title));
            for r in rows { help_text.push(Line::from(r)); }
            help_text.push(Line::from(""));
        }
    }

    // Plugin actions.
    let mut has_plugin_header = false;
    for (suite, actions) in &state.suite_actions {
        if actions.is_empty() { continue; }
        if !has_plugin_header {
            help_text.push(heading("Actions (a to open menu)"));
            has_plugin_header = true;
        }
        for a in actions {
            help_text.push(Line::from(format!("   {:<16} ({})", a.label, suite)));
        }
    }
    if has_plugin_header {
        help_text.push(Line::from(""));
    }

    draw_text_overlay(f, &mut state.overlay, "Help", help_text, 70, 95);
}

/// Shared renderer for scrollable text overlays. Handles centering, scroll
/// clamping, title with scroll indicator, and Esc hint.
pub(super) fn draw_text_overlay(
    f: &mut ratatui::Frame,
    ov: &mut OverlayState,
    fallback_title: &str,
    lines: Vec<Line<'_>>,
    width_pct: u16,
    height_pct: u16,
) {
    let area = centered_rect(width_pct, height_pct, f.area());
    f.render_widget(Clear, area);

    let total = lines.len();
    let visible_h = (area.height as usize).saturating_sub(2).max(1);
    ov.view_height = visible_h;
    let max_scroll = total.saturating_sub(visible_h);
    if ov.scroll > max_scroll { ov.scroll = max_scroll; }
    let title = if max_scroll == 0 {
        format!("{fallback_title} (Esc to close)")
    } else {
        format!("{fallback_title} ({}/{} \u{00b7} j/k scroll \u{00b7} Esc to close)",
            (ov.scroll + 1).min(max_scroll + 1), max_scroll + 1)
    };
    let widget = Paragraph::new(lines)
        .block(pane_block(&title, true))
        .scroll((ov.scroll as u16, 0));
    f.render_widget(widget, area);
}

/// Strip ANSI escape sequences (SGR, OSC hyperlinks) from a string.
pub(crate) fn strip_ansi(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    let mut chars = s.chars().peekable();
    while let Some(c) = chars.next() {
        if c != '\x1b' { out.push(c); continue; }
        match chars.peek() {
            // CSI: \x1b[ ... <letter>
            Some('[') => {
                chars.next();
                while let Some(&ch) = chars.peek() {
                    chars.next();
                    if ch.is_ascii_alphabetic() { break; }
                }
            }
            // OSC: \x1b] ... ST (\x1b\\ or \x07)
            Some(']') => {
                chars.next();
                while let Some(&ch) = chars.peek() {
                    if ch == '\x07' { chars.next(); break; }
                    if ch == '\x1b' {
                        chars.next();
                        if chars.peek() == Some(&'\\') { chars.next(); }
                        break;
                    }
                    chars.next();
                }
            }
            _ => {}
        }
    }
    out
}
