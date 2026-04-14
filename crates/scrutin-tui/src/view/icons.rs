//! Outcome icons + colors, file-detail formatting. Glyphs come from
//! `Outcome::icon()` in scrutin-core; only the ratatui `Color` mapping
//! lives here (since core can't depend on a frontend's color type).

use ratatui::style::Color;

use crate::state::TestEntry;

/// Pick an icon + color for a test entry.
pub(super) fn test_icon_color(t: &TestEntry) -> (&'static str, Color) {
    use scrutin_core::engine::protocol::Outcome;
    let color = match t.outcome {
        Outcome::Pass  => Color::Green,
        Outcome::Fail  => Color::Red,
        Outcome::Error => Color::Red,
        Outcome::Skip  => Color::DarkGray,
        // xfail: predicted failure, dim so it reads as "fine but noted".
        Outcome::Xfail => Color::DarkGray,
        // warn: yellow, distinct from skip.
        Outcome::Warn  => Color::Yellow,
    };
    (t.outcome.icon(), color)
}

/// One-line per-file detail string. Hides zero-count categories so the
/// common "all-pass" file stays compact.
pub(super) fn format_file_detail(passed: u32, failed: u32, errored: u32, warned: u32, ms: u64) -> String {
    let mut parts: Vec<String> = Vec::new();
    if passed  > 0 { parts.push(format!("\u{25cf}{}", passed)); }
    if failed  > 0 { parts.push(format!("\u{2717}{}", failed)); }
    if errored > 0 { parts.push(format!("\u{26a0}{}", errored)); }
    if warned  > 0 { parts.push(format!("\u{26a1}{}", warned)); }
    parts.push(format!("{}ms", ms));
    parts.join(" ")
}
