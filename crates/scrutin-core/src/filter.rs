//! Glob matching for test-file include/exclude lists.
//!
//! ## Dialect
//!
//! Powered by the `globset` crate (same matcher as ripgrep / `ignore`):
//!
//! - `*` matches any run of characters except `/`
//! - `**` matches any run of characters including `/` (recursive)
//! - `?` matches exactly one character except `/`
//! - `[abc]` / `[!abc]` character classes
//! - `{a,b}` alternation
//! - backslash escapes a metacharacter
//!
//! Patterns are matched against file **basenames** (not full paths), so
//! `/` in a pattern effectively never matches. That's intentional: scrutin's
//! filters always scope to a single filename.

use globset::{Glob, GlobSet, GlobSetBuilder};
use std::path::PathBuf;

/// Match a basename against a single glob pattern. Anchored at both ends.
///
/// Invalid patterns return `false` rather than erroring: filter patterns
/// come from user config and a typo should not crash the runner.
///
/// Prefer `apply_include_exclude` when matching a whole list of files
/// against many patterns: it compiles a `GlobSet` once and checks all
/// patterns in one pass per file.
pub fn matches_name(pattern: &str, name: &str) -> bool {
    match Glob::new(pattern) {
        Ok(g) => g.compile_matcher().is_match(name),
        Err(_) => false,
    }
}

/// Apply include + exclude filters in a single retain pass over `files`.
///
/// - Empty `includes` means "include everything" (no positive filter).
/// - A path is kept iff it matches *any* include pattern (or includes is
///   empty) AND matches *no* exclude pattern.
///
/// Compiles each list into a `GlobSet` once, then matches all patterns in
/// one pass per file. Invalid patterns are silently skipped (same rationale
/// as `matches_name`).
pub fn apply_include_exclude(
    files: &mut Vec<PathBuf>,
    includes: &[String],
    excludes: &[String],
) {
    if includes.is_empty() && excludes.is_empty() {
        return;
    }
    let include_set = build_glob_set(includes);
    let exclude_set = build_glob_set(excludes);
    files.retain(|f| {
        let name = f.file_name().unwrap_or_default().to_string_lossy();
        let included = includes.is_empty() || include_set.is_match(name.as_ref());
        if !included {
            return false;
        }
        !exclude_set.is_match(name.as_ref())
    });
}

fn build_glob_set(patterns: &[String]) -> GlobSet {
    let mut builder = GlobSetBuilder::new();
    for p in patterns {
        if let Ok(g) = Glob::new(p) {
            builder.add(g);
        }
    }
    builder.build().unwrap_or_else(|_| GlobSet::empty())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn literal_exact_match() {
        assert!(matches_name("test_foo.py", "test_foo.py"));
        assert!(!matches_name("test_foo.py", "test_bar.py"));
    }

    #[test]
    fn star_anchors() {
        assert!(matches_name("test_*", "test_foo.py"));
        assert!(!matches_name("test_*", "atest_foo.py"));
        assert!(matches_name("*foo*", "test_foo.py"));
        assert!(matches_name("*.py", "test_foo.py"));
        assert!(!matches_name("*.py", "test_foo.R"));
    }

    #[test]
    fn star_matches_empty() {
        assert!(matches_name("a*b", "ab"));
        assert!(matches_name("*", ""));
    }

    #[test]
    fn question_mark_matches_one() {
        assert!(matches_name("test-?.R", "test-1.R"));
        assert!(!matches_name("test-?.R", "test-12.R"));
        assert!(!matches_name("test-?.R", "test-.R"));
    }

    #[test]
    fn backtracking_works() {
        assert!(matches_name("*ab*abc", "abababc"));
        assert!(matches_name("*a*a", "aa"));
        assert!(matches_name("a*a*a", "aaaa"));
    }

    #[test]
    fn no_match_returns_false() {
        assert!(!matches_name("a*b", "ba"));
        assert!(!matches_name("foo", "foobar"));
        assert!(!matches_name("foobar", "foo"));
    }

    #[test]
    fn character_class_and_alternation() {
        assert!(matches_name("test-[abc].R", "test-a.R"));
        assert!(!matches_name("test-[abc].R", "test-d.R"));
        assert!(matches_name("test-[!abc].R", "test-d.R"));
        assert!(matches_name("test-{foo,bar}.R", "test-foo.R"));
        assert!(matches_name("test-{foo,bar}.R", "test-bar.R"));
        assert!(!matches_name("test-{foo,bar}.R", "test-baz.R"));
    }

    #[test]
    fn invalid_pattern_returns_false() {
        assert!(!matches_name("test-[abc.R", "test-a.R"));
    }

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
        let mut files = vec![pb("test_a.py"), pb("test_slow.py"), pb("conftest.py")];
        apply_include_exclude(&mut files, &["test_*".into()], &["*slow*".into()]);
        assert_eq!(files, vec![pb("test_a.py")]);
    }

    #[test]
    fn alternation_in_include() {
        let mut files = vec![
            pb("test_model.py"),
            pb("test_plot.py"),
            pb("test_slow.py"),
        ];
        apply_include_exclude(&mut files, &["test_{model,plot}.py".into()], &[]);
        assert_eq!(files, vec![pb("test_model.py"), pb("test_plot.py")]);
    }
}
