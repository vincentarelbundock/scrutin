//! Full-screen log viewer (Mode::Log).

use ratatui::layout::{Constraint, Direction, Layout};
use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::Paragraph;

use crate::state::AppState;
use super::layout::pane_block;

pub(super) fn draw_log(f: &mut ratatui::Frame, state: &mut AppState) {
    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Length(1), // header
            Constraint::Min(3),    // log body
            Constraint::Length(1), // hints
        ])
        .split(f.area());

    f.render_widget(
        Paragraph::new(Line::from(vec![
            Span::styled(" scrutin", Style::default().fg(Color::DarkGray)),
            Span::styled(" \u{203a} ", Style::default().fg(Color::DarkGray)),
            Span::styled("Log", Style::default().add_modifier(Modifier::BOLD)),
            Span::styled("  (Esc/L/q to go back)", Style::default().fg(Color::DarkGray)),
        ])),
        chunks[0],
    );

    let body = chunks[1];
    state.pane_rects.log = Some(body);
    let visible = (body.height as usize).saturating_sub(2).max(1);
    state.nav.log_view_height = visible;

    let lines = state.log.snapshot();
    let max_scroll = lines.len().saturating_sub(visible);
    if state.nav.log_scroll > max_scroll {
        state.nav.log_scroll = max_scroll;
    }
    let rendered: Vec<Line> = lines
        .iter()
        .skip(state.nav.log_scroll)
        .take(visible)
        .map(|l| {
            let style = if l.contains("[error]") || l.contains("Error") {
                Style::default().fg(Color::Red)
            } else {
                Style::default().fg(Color::Gray)
            };
            Line::from(Span::styled(l.clone(), style))
        })
        .collect();

    let title = format!("Log ({} lines)", lines.len());
    let body_widget = Paragraph::new(rendered).block(pane_block(&title, true));
    f.render_widget(body_widget, body);

    f.render_widget(
        Paragraph::new(Line::from(Span::styled(
            " j/k scroll  g/G top/bot  Ctrl-d/u half page  Esc back",
            Style::default().fg(Color::DarkGray),
        ))),
        chunks[2],
    );
}
