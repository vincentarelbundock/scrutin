//! Bottom counts bar: status icon, per-outcome counts, pass rate,
//! elapsed time, slowest file. Glyphs come from `Outcome::icon()`.

use ratatui::layout::Rect;
use ratatui::style::{Color, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::Paragraph;

use crate::state::AppState;

pub(super) fn draw_counts_bar(f: &mut ratatui::Frame, state: &AppState, area: Rect) {
    let elapsed = if let Some(d) = state.run.last_duration {
        format!("{:.2}s", d.as_secs_f64())
    } else if let Some(s) = state.run.last_run {
        format!("{:.1}s\u{2026}", s.elapsed().as_secs_f64())
    } else {
        String::new()
    };

    let status_icon = if state.run.running {
        Span::styled(" \u{25cc} ", Style::default().fg(Color::Yellow))
    } else if state.run.run_totals.fail > 0 || state.run.run_totals.error > 0 {
        Span::styled(" \u{25cf} ", Style::default().fg(Color::Red))
    } else if state.run.run_totals.warn > 0 {
        Span::styled(" \u{25cf} ", Style::default().fg(Color::Yellow))
    } else if state.run.run_totals.pass > 0 {
        Span::styled(" \u{25cf} ", Style::default().fg(Color::Green))
    } else {
        Span::styled(" \u{25cb} ", Style::default().fg(Color::DarkGray))
    };

    // Pass-rate over outcomes that actually ran (excludes skipped).
    let executed = state.run.run_totals.pass + state.run.run_totals.fail + state.run.run_totals.error;
    let pass_rate = if executed > 0 {
        format!("  {:.0}%", (state.run.run_totals.pass as f64 / executed as f64) * 100.0)
    } else {
        String::new()
    };

    // Slowest file (after a complete run only).
    let slowest = if !state.run.running && !state.run.file_durations.is_empty() {
        state
            .run.file_durations
            .iter()
            .max_by_key(|(_, ms)| *ms)
            .map(|(name, ms)| format!("  \u{25d1} {} {}ms", name, ms))
            .unwrap_or_default()
    } else {
        String::new()
    };

    let count_span = |icon: &str, n: u32, active_color: Color| -> Vec<Span> {
        let color = if n > 0 { active_color } else { Color::DarkGray };
        vec![
            Span::styled(icon.to_string(), Style::default().fg(color)),
            Span::styled(n.to_string(), Style::default().fg(color)),
            Span::raw(" "),
        ]
    };
    use scrutin_core::engine::protocol::Outcome;
    let mut spans = vec![status_icon];
    spans.extend(count_span(Outcome::Pass.icon(),  state.run.run_totals.pass,  Color::Green));
    spans.extend(count_span(Outcome::Fail.icon(),  state.run.run_totals.fail,  Color::Red));
    spans.extend(count_span(Outcome::Error.icon(), state.run.run_totals.error, Color::Red));
    spans.extend(count_span(Outcome::Warn.icon(),  state.run.run_totals.warn,  Color::Yellow));
    spans.extend(count_span(Outcome::Skip.icon(),  state.run.run_totals.skip,  Color::DarkGray));
    if state.run.run_totals.xfail > 0 {
        spans.extend(count_span(Outcome::Xfail.icon(), state.run.run_totals.xfail, Color::DarkGray));
    }
    spans.push(Span::styled(pass_rate, Style::default().fg(Color::DarkGray)));
    spans.push(Span::raw(format!("  {}", elapsed)));
    spans.push(Span::styled(slowest, Style::default().fg(Color::DarkGray)));
    f.render_widget(Paragraph::new(Line::from(spans)), area);
}
