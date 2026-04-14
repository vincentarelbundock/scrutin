//! Source-context rendering with syntect-based syntax highlighting,
//! vertical and horizontal scroll, and configurable context window.

use std::path::Path;

use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span};

/// Source window with the gutter prefix and `>` arrow at `line` (if any).
pub(super) fn load_source_context(path: &Path, line: Option<u32>, max_lines: usize) -> Vec<Line<'static>> {
    load_source_context_ex(path, line, max_lines, 0, 0)
}

/// Cheap line count for clamping `source_scroll` against EOF. Re-reads
/// the file each frame (acceptable for test files); add a cache here if
/// profiling ever flags it.
pub(super) fn file_line_count(path: &Path) -> usize {
    std::fs::read_to_string(path)
        .map(|c| c.lines().count())
        .unwrap_or(0)
}

/// Height-aware source context with syntect-based syntax highlighting,
/// vertical and horizontal scroll, and configurable context window.
///
/// `extra_scroll` shifts the visible window down (lines), `hscroll`
/// shifts each rendered line left by N columns. Both clamp to valid
/// ranges.
pub(super) fn load_source_context_ex(
    path: &Path,
    line: Option<u32>,
    max_lines: usize,
    extra_scroll: usize,
    hscroll: usize,
) -> Vec<Line<'static>> {
    use std::sync::LazyLock;
    use syntect::easy::HighlightLines;
    use syntect::highlighting::{Style as SynStyle, ThemeSet};
    use syntect::parsing::SyntaxSet;

    static SYNTAX_SET: LazyLock<SyntaxSet> = LazyLock::new(SyntaxSet::load_defaults_newlines);
    static THEME_SET: LazyLock<ThemeSet> = LazyLock::new(ThemeSet::load_defaults);

    let content = match std::fs::read_to_string(path) {
        Ok(c) => c,
        Err(_) => return vec![Line::from("  (could not read file)")],
    };

    let all_lines: Vec<&str> = content.lines().collect();
    let half = max_lines / 2;

    let (mut start, mut end) = if let Some(target_line) = line {
        let target = target_line as usize;
        let s = target.saturating_sub(half);
        let e = (target + half).min(all_lines.len());
        (s, e)
    } else {
        (0, max_lines.min(all_lines.len()))
    };
    // Apply manual scroll offset (Tab \u2192 Main pane, j/k).
    let total = all_lines.len();
    start = (start + extra_scroll).min(total);
    end = (end + extra_scroll).min(total);

    // Pick syntax by extension (R, Py, etc.); fall back to plain text.
    let ext = path.extension().and_then(|e| e.to_str()).unwrap_or("");
    let syntax = SYNTAX_SET
        .find_syntax_by_extension(ext)
        .unwrap_or_else(|| SYNTAX_SET.find_syntax_plain_text());
    let theme = &THEME_SET.themes["base16-eighties.dark"];
    let mut h = HighlightLines::new(syntax, theme);

    // Syntect is stateful \u{2014} feed every line from the top of the file so
    // the parser state is correct at `start`, but only emit lines in
    // [start, end).
    let mut out: Vec<Line<'static>> = Vec::with_capacity(end.saturating_sub(start));
    for (i, raw) in all_lines.iter().enumerate().take(end) {
        let line_with_nl = format!("{}\n", raw);
        let regions: Vec<(SynStyle, &str)> = h
            .highlight_line(&line_with_nl, &SYNTAX_SET)
            .unwrap_or_default();
        if i < start { continue; }
        let line_no = i + 1;
        let is_target = line.is_some_and(|t| line_no == t as usize);
        let prefix = if is_target { ">" } else { " " };
        let gutter_style = if is_target {
            Style::default().fg(Color::Red).add_modifier(Modifier::BOLD)
        } else {
            Style::default().fg(Color::DarkGray)
        };
        let mut spans: Vec<Span<'static>> = Vec::with_capacity(regions.len() + 1);
        spans.push(Span::styled(format!("{} {:>4} \u{2502} ", prefix, line_no), gutter_style));
        let mut skip = hscroll;
        for (sty, text) in regions {
            let text = text.trim_end_matches('\n');
            if text.is_empty() { continue; }
            let text: String = if skip >= text.chars().count() {
                skip -= text.chars().count();
                continue;
            } else if skip > 0 {
                let s: String = text.chars().skip(skip).collect();
                skip = 0;
                s
            } else {
                text.to_string()
            };
            let fg = Color::Rgb(sty.foreground.r, sty.foreground.g, sty.foreground.b);
            let mut s = Style::default().fg(fg);
            if is_target {
                s = s.add_modifier(Modifier::BOLD);
            }
            spans.push(Span::styled(text, s));
        }
        out.push(Line::from(spans));
    }
    out
}
