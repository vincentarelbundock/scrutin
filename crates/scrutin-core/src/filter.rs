//! Glob matching for test-file include/exclude lists.
//!
//! ## Dialect
//!
//! A small, deliberately narrow shell-style glob:
//!
//! - `*` matches any (possibly empty) sequence of characters
//! - `?` matches exactly one character
//! - all other characters match literally (no `[…]` ranges, no escape)
//!
//! Matching is anchored at both ends — the pattern must consume the whole
//! input — and runs in O(p + t) via standard backtracking pointers (no
//! recursion, no exponential worst case). Patterns are matched against
//! file basenames, not full paths.

use std::path::PathBuf;

/// Match `text` against a `*`/`?` glob pattern. Anchored at both ends.
///
/// Examples (verified by unit tests):
/// - `glob_match("test_*", "test_foo.py")` → true
/// - `glob_match("*slow*", "test_slow_db.py")` → true
/// - `glob_match("test-?.R", "test-1.R")` → true
/// - `glob_match("*ab*abc", "abababc")` → true (correct backtracking)
pub fn glob_match(pattern: &str, text: &str) -> bool {
    let p: Vec<char> = pattern.chars().collect();
    let t: Vec<char> = text.chars().collect();

    let (mut pi, mut ti) = (0usize, 0usize);
    // When we hit a `*`, remember where it was and how much of `text` was
    // consumed at that point. On a later mismatch we rewind to just-after
    // the star and let it consume one more character. This is the standard
    // linear-time glob algorithm — O(|p| + |t|) for typical inputs, with
    // the worst case bounded by `|p| * |t|` (no exponential blowup).
    let mut star: Option<usize> = None;
    let mut match_ti: usize = 0;

    while ti < t.len() {
        if pi < p.len() && (p[pi] == '?' || p[pi] == t[ti]) {
            pi += 1;
            ti += 1;
        } else if pi < p.len() && p[pi] == '*' {
            star = Some(pi);
            match_ti = ti;
            pi += 1;
        } else if let Some(s) = star {
            pi = s + 1;
            match_ti += 1;
            ti = match_ti;
        } else {
            return false;
        }
    }
    // Trailing stars in the pattern match the empty tail.
    while pi < p.len() && p[pi] == '*' {
        pi += 1;
    }
    pi == p.len()
}

/// Apply include + exclude filters in a single retain pass over `files`.
/// Computes the basename once per path, instead of twice (which the
/// previous `apply_filters` + `apply_excludes` pair did).
///
/// - Empty `includes` means "include everything" (no positive filter).
/// - A path is kept iff it matches *any* include pattern (or includes is
///   empty) AND matches *no* exclude pattern.
pub fn apply_include_exclude(
    files: &mut Vec<PathBuf>,
    includes: &[String],
    excludes: &[String],
) {
    if includes.is_empty() && excludes.is_empty() {
        return;
    }
    files.retain(|f| {
        let name = f.file_name().unwrap_or_default().to_string_lossy();
        let included = includes.is_empty() || includes.iter().any(|p| glob_match(p, &name));
        if !included {
            return false;
        }
        !excludes.iter().any(|p| glob_match(p, &name))
    });
}

/// Convenience for callers that have a single pattern and a basename
/// already in hand (e.g. the TUI's active filter). Equivalent to
/// `glob_match(pattern, name)` — exposed under a clearer name so the
/// dialect's "match basenames" convention is enforced at one entry point.
pub fn matches_name(pattern: &str, name: &str) -> bool {
    glob_match(pattern, name)
}

#[cfg(test)]
mod tests {
    use super::*;

    // ── glob_match basics ───────────────────────────────────────────────────

    #[test]
    fn literal_exact_match() {
        assert!(glob_match("test_foo.py", "test_foo.py"));
        assert!(!glob_match("test_foo.py", "test_bar.py"));
    }

    #[test]
    fn star_anchors() {
        assert!(glob_match("test_*", "test_foo.py"));
        assert!(!glob_match("test_*", "atest_foo.py"));
        assert!(glob_match("*foo*", "test_foo.py"));
        assert!(glob_match("*.py", "test_foo.py"));
        assert!(!glob_match("*.py", "test_foo.R"));
    }

    #[test]
    fn star_matches_empty() {
        assert!(glob_match("a*b", "ab"));
        assert!(glob_match("*", ""));
        assert!(glob_match("**", ""));
    }

    #[test]
    fn question_mark_matches_one() {
        assert!(glob_match("test-?.R", "test-1.R"));
        assert!(!glob_match("test-?.R", "test-12.R"));
        assert!(!glob_match("test-?.R", "test-.R"));
    }

    #[test]
    fn backtracking_works() {
        // Regression: the old leftmost-find matcher returned false for
        // these because the first `*` segment greedily consumed a prefix
        // the later segment needed.
        assert!(glob_match("*ab*abc", "abababc"));
        assert!(glob_match("*a*a", "aa"));
        assert!(glob_match("a*a*a", "aaaa"));
    }

    #[test]
    fn no_match_returns_false() {
        assert!(!glob_match("a*b", "ba"));
        assert!(!glob_match("foo", "foobar"));
        assert!(!glob_match("foobar", "foo"));
    }

    // ── apply_include_exclude ──────────────────────────────────────────────

    fn pb(s: &str) -> PathBuf {
        PathBuf::from(s)
    }

    #[test]
    fn empty_filters_keep_all() {
        let mut files = vec![pb("a.py"), pb("b.py")];
        apply_include_exclude(&mut files, &[], &[]);
        assert_eq!(files, vec![pb("a.py"), pb("b.py")]);
    }

    #[test]
    fn includes_keep_only_matches() {
        let mut files = vec![pb("test_a.py"), pb("test_b.py"), pb("conftest.py")];
        apply_include_exclude(&mut files, &["test_*".into()], &[]);
        assert_eq!(files, vec![pb("test_a.py"), pb("test_b.py")]);
    }

    #[test]
    fn excludes_drop_matches() {
        let mut files = vec![pb("test_slow.py"), pb("test_fast.py")];
        apply_include_exclude(&mut files, &[], &["*slow*".into()]);
        assert_eq!(files, vec![pb("test_fast.py")]);
    }

    #[test]
    fn includes_and_excludes_compose() {
        let mut files = vec![
            pb("test_a.py"),
            pb("test_slow.py"),
            pb("conftest.py"),
        ];
        apply_include_exclude(
            &mut files,
            &["test_*".into()],
            &["*slow*".into()],
        );
        assert_eq!(files, vec![pb("test_a.py")]);
    }
}
