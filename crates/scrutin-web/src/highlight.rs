//! Server-side syntax highlighting for source snippets. Uses syntect's
//! class-based HTML so the client can theme via CSS, matching the
//! existing `data-theme` light/dark toggle.

use std::sync::LazyLock;

use syntect::highlighting::ThemeSet;
use syntect::html::{ClassStyle, css_for_theme_with_class_style, line_tokens_to_classed_spans};
use syntect::parsing::{ParseState, ScopeStack, SyntaxSet};

const CLASS_STYLE: ClassStyle = ClassStyle::Spaced;

static SYNTAX_SET: LazyLock<SyntaxSet> = LazyLock::new(SyntaxSet::load_defaults_newlines);
static THEME_SET: LazyLock<ThemeSet> = LazyLock::new(ThemeSet::load_defaults);

/// Highlight the `[start, end)` slice of `content` using the syntax
/// inferred from `ext`. Returns one HTML fragment per line (no trailing
/// newline). Each fragment is self-contained: scopes that cross line
/// boundaries (block comments, multi-line strings) are re-opened at the
/// start of the line and closed at the end, so dropping any single line
/// into the DOM still produces balanced HTML.
pub fn highlight_slice(ext: &str, content: &str, start: usize, end: usize) -> Vec<String> {
    let syntax = SYNTAX_SET
        .find_syntax_by_extension(ext)
        .unwrap_or_else(|| SYNTAX_SET.find_syntax_plain_text());
    let mut parse_state = ParseState::new(syntax);
    let mut scope_stack = ScopeStack::new();
    let mut out = Vec::with_capacity(end.saturating_sub(start));
    for (i, line) in content.lines().enumerate() {
        if i >= end {
            break;
        }
        let with_nl = format!("{}\n", line);

        let reopen: String = scope_stack
            .as_slice()
            .iter()
            .map(|s| {
                let cls = s.build_string().replace('.', " ");
                format!("<span class=\"{}\">", cls)
            })
            .collect();

        let ops = parse_state
            .parse_line(&with_nl, &SYNTAX_SET)
            .unwrap_or_default();
        let body = line_tokens_to_classed_spans(&with_nl, &ops, CLASS_STYLE, &mut scope_stack)
            .map(|(s, _)| s)
            .unwrap_or_default();
        let close = "</span>".repeat(scope_stack.len());

        if i >= start {
            let mut s = String::with_capacity(reopen.len() + body.len() + close.len());
            s.push_str(&reopen);
            s.push_str(body.trim_end_matches('\n'));
            s.push_str(&close);
            out.push(s);
        }
    }
    out
}

/// CSS that translates syntect class names into theme colors. Scoped by
/// `html[data-theme="dark"]` / `html[data-theme="light"]` so the existing
/// toggle selects the right theme without re-rendering any HTML.
pub fn theme_css() -> &'static str {
    static CSS: LazyLock<String> = LazyLock::new(|| {
        let dark = css_for_theme_with_class_style(
            &THEME_SET.themes["base16-eighties.dark"],
            CLASS_STYLE,
        )
        .unwrap_or_default();
        let light =
            css_for_theme_with_class_style(&THEME_SET.themes["InspiredGitHub"], CLASS_STYLE)
                .unwrap_or_default();
        let mut s = String::with_capacity(dark.len() + light.len() + 256);
        s.push_str(&scope_css(&dark, "html[data-theme=\"dark\"]"));
        s.push('\n');
        s.push_str(&scope_css(&light, "html[data-theme=\"light\"]"));
        s
    });
    &CSS
}

/// Prepend `scope ` in front of every selector in `css`. Splits on the
/// rule terminator `}` and handles comma-separated selector lists. The
/// input comes from syntect and has a simple `.class { ...; }` shape, so
/// a full CSS parser would be overkill. Strips /* ... */ comments first
/// because syntect emits a leading theme-name banner that would
/// otherwise land between the scope prefix and the first selector.
fn scope_css(css: &str, scope: &str) -> String {
    let stripped = strip_block_comments(css);
    let mut out = String::with_capacity(stripped.len() + scope.len() * 8);
    for rule in stripped.split('}') {
        let rule = rule.trim();
        if rule.is_empty() {
            continue;
        }
        let Some(brace) = rule.find('{') else { continue };
        let (sel, body) = rule.split_at(brace);
        let selectors: Vec<&str> = sel.split(',').map(|s| s.trim()).collect();
        // Syntect emits a base `.code` rule for the theme background; our
        // own `.code` span is the per-line container and would pick up
        // the theme background, fighting `.source-snippet`. Drop it and
        // let the existing frontend styles supply the container colors.
        if selectors.iter().all(|s| *s == ".code") {
            continue;
        }
        let scoped: Vec<String> = selectors
            .into_iter()
            .filter(|s| *s != ".code")
            .map(|s| format!("{} {}", scope, s))
            .collect();
        if scoped.is_empty() {
            continue;
        }
        out.push_str(&scoped.join(", "));
        out.push(' ');
        out.push_str(body);
        out.push_str("}\n");
    }
    out
}

fn strip_block_comments(css: &str) -> String {
    let mut out = String::with_capacity(css.len());
    let mut rest = css;
    while let Some(start) = rest.find("/*") {
        out.push_str(&rest[..start]);
        match rest[start + 2..].find("*/") {
            Some(end) => rest = &rest[start + 2 + end + 2..],
            None => return out,
        }
    }
    out.push_str(rest);
    out
}
