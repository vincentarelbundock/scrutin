//! Python static import analysis with transitive resolution.
//!
//! Walks `.py` files under the project root, extracts `import` / `from ...
//! import` statements, resolves them to local files, and inverts the result
//! into a reverse dependency map: `source_file -> [test_files]`.
//!
//! **Transitive resolution**: the map traces the full import graph. If
//! `test_x.py` imports `helpers.py` which imports `core.py`, then `core.py`
//! maps to `test_x.py`. Cycles (allowed by Python) are handled safely.
//!
//! This is a pragmatic line-based parser, not a full Python parser. It
//! handles:
//!   - `import foo` / `import foo.bar` / `import foo as x` / `import foo, bar`
//!   - `from foo import x` / `from foo.bar import y, z` / `from foo import x as y`
//!   - `from . import x` / `from .foo import y` / `from ..foo import y` (relative)
//!   - Inline `# comments` after the import statement
//!   - Indented `import` lines (e.g. inside `if TYPE_CHECKING:`)
//!   - Multi-line parenthesized imports: `from foo import (\n  a,\n  b,\n)`
//!   - Backslash continuation lines: `from foo import a, \\\n    b, c`
//!
//! What it deliberately does **not** handle:
//!   - `importlib.import_module(...)` and `__import__("...")` (string-ref
//!     imports: there is no static way to resolve them).
//!
//! All misses fall back to the filename heuristic and ultimately to a full
//! suite re-run, so the cost of a missed import is "slower watch loop," not
//! "missed test failure."

use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};

use crate::analysis::walk;
use crate::project::package::Package;

/// Build the reverse dep map for `pkg`. Keys are source-file paths relative
/// to `pkg.root`; values are test-file *basenames* (to match the existing
/// dep-map format consumed by `deps::resolve_tests`).
///
/// Multi-suite aware: in a monorepo with an R suite rooted at `r/` and a
/// Python suite rooted at `python/`, only the Python suite's `watch`
/// dir-prefixes are scanned (so R files do not leak into the Python import
/// graph and vice versa).
pub fn build_import_map(pkg: &Package) -> HashMap<String, Vec<String>> {
    // Gather .py files from every Python suite's watch + run dir-prefixes
    // so the walker stays bounded by what Python actually cares about.
    let mut py_files: Vec<PathBuf> = Vec::new();
    let mut seen_py: HashSet<PathBuf> = HashSet::new();
    for suite in &pkg.test_suites {
        if suite.plugin.language() != "python" {
            continue;
        }
        let dirs = suite
            .watch_search_dirs()
            .into_iter()
            .chain(suite.run_search_dirs());
        for dir in dirs {
            if !dir.is_dir() {
                continue;
            }
            for f in walk::collect_files(&dir, |p| walk::has_extension(p, &["py"])) {
                if seen_py.insert(f.clone()) {
                    py_files.push(f);
                }
            }
        }
    }
    let test_set: HashSet<PathBuf> = pkg.test_files().unwrap_or_default().into_iter().collect();

    // Module-name → source-file index. Best-effort: handles flat layout
    // (`pkg/foo.py`) and src layout (`src/pkg/foo.py`) by stripping a
    // leading "src" segment if present.
    let mut module_to_file: HashMap<String, PathBuf> = HashMap::new();
    for file in &py_files {
        if test_set.contains(file) {
            continue;
        }
        if let Some(module) = file_to_module(&pkg.root, file) {
            module_to_file.insert(module, file.clone());
        }
    }

    // Source-to-source adjacency graph (direct deps only) for transitive
    // resolution. If test_x.py imports helpers.py which imports core.py,
    // editing core.py should trigger test_x.py.
    let mut src_graph: HashMap<PathBuf, HashSet<PathBuf>> = HashMap::new();
    for file in &py_files {
        if test_set.contains(file) {
            continue;
        }
        let contents = match std::fs::read_to_string(file) {
            Ok(s) => s,
            Err(_) => continue,
        };
        let imports = scan_imports_str(&contents);
        let resolved = resolve_imports(file, &pkg.root, &imports, &module_to_file);
        let resolved: HashSet<PathBuf> = resolved
            .into_iter()
            .filter(|p| !test_set.contains(p))
            .collect();
        if !resolved.is_empty() {
            src_graph.insert(file.clone(), resolved);
        }
    }

    // For each test file, scan imports and resolve them to source files.
    let mut forward: HashMap<PathBuf, HashSet<PathBuf>> = HashMap::new();
    for test_file in &test_set {
        let contents = match std::fs::read_to_string(test_file) {
            Ok(s) => s,
            Err(e) => {
                // Surface unreadable test files: an empty import set would
                // silently exclude this file from the reverse map and edits
                // to its dependencies wouldn't trigger it on watch.
                eprintln!(
                    "[scrutin] python::imports: failed to read {}: {e}",
                    test_file.display()
                );
                continue;
            }
        };
        let imports = scan_imports_str(&contents);
        let resolved = resolve_imports(test_file, &pkg.root, &imports, &module_to_file);
        forward.insert(test_file.clone(), resolved);
    }

    // Expand each test file's deps transitively through the source graph.
    // Cycle-safe: HashSet::insert returns false for already-seen nodes.
    for sources in forward.values_mut() {
        let mut queue: Vec<PathBuf> = sources.iter().cloned().collect();
        while let Some(src) = queue.pop() {
            if let Some(neighbors) = src_graph.get(&src) {
                for neighbor in neighbors {
                    if sources.insert(neighbor.clone()) {
                        queue.push(neighbor.clone());
                    }
                }
            }
        }
    }

    // Invert. Keys relative to pkg.root; values are test basenames.
    let mut reverse: HashMap<String, Vec<String>> = HashMap::new();
    for (test_file, sources) in forward {
        let test_name = test_file
            .file_name()
            .and_then(|n| n.to_str())
            .unwrap_or("")
            .to_string();
        if test_name.is_empty() {
            continue;
        }
        for src in sources {
            // Skip anything outside the project root rather than storing
            // an absolute path that the dep-map consumer wouldn't recognize.
            let Ok(rel) = src.strip_prefix(&pkg.root) else {
                continue;
            };
            reverse
                .entry(rel.to_string_lossy().into_owned())
                .or_default()
                .push(test_name.clone());
        }
    }
    for v in reverse.values_mut() {
        v.sort();
        v.dedup();
    }
    reverse
}

// ── Internals ───────────────────────────────────────────────────────────────

/// Convert a file path to a dotted module name, dropping any leading
/// "src" segment and trimming "__init__.py".
pub(crate) fn file_to_module(root: &Path, file: &Path) -> Option<String> {
    let rel = file.strip_prefix(root).ok()?;
    let mut parts: Vec<String> = rel
        .components()
        .filter_map(|c| match c {
            std::path::Component::Normal(s) => s.to_str().map(|s| s.to_string()),
            _ => None,
        })
        .collect();
    if parts.is_empty() {
        return None;
    }
    if parts[0] == "src" {
        parts.remove(0);
    }
    let last = parts.pop()?;
    let stem = last.strip_suffix(".py").unwrap_or(&last);
    if stem == "__init__" {
        if parts.is_empty() {
            return None;
        }
        return Some(parts.join("."));
    }
    parts.push(stem.to_string());
    Some(parts.join("."))
}

/// A single parsed import statement.
#[derive(Debug, PartialEq, Eq)]
pub(crate) enum Import {
    /// `import foo` or `import foo.bar`
    Absolute(String),
    /// `from foo.bar import x, y` — `module = "foo.bar"`, `symbols = ["x","y"]`.
    /// We keep the symbols so the resolver can probe both `foo.bar` and
    /// `foo.bar.x` / `foo.bar.y` for the case where `x` is a submodule
    /// rather than a name re-exported from `foo/bar.py`.
    FromAbsolute { module: String, symbols: Vec<String> },
    /// `from .foo import x` — `dots = 1`, `tail = "foo"`, plus the imported
    /// symbols for the same submodule-probing reason.
    Relative {
        dots: usize,
        tail: String,
        symbols: Vec<String>,
    },
}

pub(crate) fn scan_imports_str(contents: &str) -> Vec<Import> {
    let mut out = Vec::new();

    // Pre-join backslash continuation lines into logical lines.
    let mut logical_lines: Vec<String> = Vec::new();
    let mut accum = String::new();
    for raw in contents.lines() {
        let trimmed_end = raw.trim_end();
        if trimmed_end.ends_with('\\') {
            accum.push_str(&trimmed_end[..trimmed_end.len() - 1]);
            continue;
        }
        if !accum.is_empty() {
            accum.push_str(raw);
            logical_lines.push(std::mem::take(&mut accum));
        } else {
            logical_lines.push(raw.to_string());
        }
    }
    if !accum.is_empty() {
        logical_lines.push(accum);
    }

    // Track whether the current line is inside a triple-quoted string
    // (docstring or otherwise). Without this, a docstring containing
    // `from foo import bar` would produce phantom dep-map edges.
    let mut in_triple: Option<&'static str> = None;
    let mut idx = 0;
    while idx < logical_lines.len() {
        let raw = &logical_lines[idx];
        idx += 1;
        if let Some(delim) = in_triple {
            if raw.contains(delim) {
                in_triple = None;
            }
            continue;
        }
        // Detect a triple-quote that opens on this line and isn't closed
        // on the same line. Check `"""` first, then `'''`.
        for delim in ["\"\"\"", "'''"] {
            if let Some(first) = raw.find(delim) {
                let after = &raw[first + 3..];
                if !after.contains(delim) {
                    in_triple = Some(delim);
                }
                break;
            }
        }
        if in_triple.is_some() {
            continue;
        }
        let line = raw.trim();
        // `import foo` / `import foo.bar` / `import foo as x, bar`
        if let Some(rest) = line.strip_prefix("import ") {
            let rest = strip_inline_comment(rest);
            for item in rest.split(',') {
                let item = item.trim();
                if item.is_empty() {
                    continue;
                }
                // `foo as x` → take "foo"
                let name = item.split_whitespace().next().unwrap_or("");
                if !name.is_empty() {
                    out.push(Import::Absolute(name.to_string()));
                }
            }
            continue;
        }
        // `from foo import x` / `from .foo import x` / `from . import x`
        if let Some(rest) = line.strip_prefix("from ") {
            let rest = strip_inline_comment(rest);
            let mut it = rest.splitn(2, " import ");
            let src = it.next().unwrap_or("").trim();
            let rhs = it.next().unwrap_or("").trim();
            if src.is_empty() {
                continue;
            }
            // Handle multi-line parenthesized imports:
            //   from foo import (
            //       a,
            //       b,
            //   )
            let full_rhs = if rhs.starts_with('(') && !rhs.contains(')') {
                let mut parts = rhs.to_string();
                while idx < logical_lines.len() {
                    let next = strip_inline_comment(logical_lines[idx].trim());
                    idx += 1;
                    parts.push_str(" ");
                    parts.push_str(next);
                    if next.contains(')') {
                        break;
                    }
                }
                parts
            } else {
                rhs.to_string()
            };
            let symbols = parse_symbol_list(&full_rhs);
            let dots = src.chars().take_while(|c| *c == '.').count();
            let tail = src[dots..].trim();
            if dots > 0 {
                out.push(Import::Relative {
                    dots,
                    tail: tail.to_string(),
                    symbols,
                });
            } else {
                out.push(Import::FromAbsolute {
                    module: tail.to_string(),
                    symbols,
                });
            }
        }
    }
    out
}

fn strip_inline_comment(s: &str) -> &str {
    s.split('#').next().unwrap_or("").trim()
}

/// Parse the right-hand side of a `from X import RHS` line into a list of
/// imported symbols. Handles `a`, `a, b`, `a as b`, `a, b as c`. The leading
/// `(` of `import (a, b,)` is stripped so the very first symbol of a
/// parenthesized multi-line import still parses (the rest of the symbols
/// live on subsequent lines and are missed — see module doc).
fn parse_symbol_list(rhs: &str) -> Vec<String> {
    let rhs = rhs.trim_start_matches('(').trim_end_matches(')');
    rhs.split(',')
        .filter_map(|item| {
            // `a as b` → `a`
            item.split_whitespace()
                .next()
                .map(|s| s.to_string())
                .filter(|s| !s.is_empty())
        })
        .collect()
}

pub(crate) fn resolve_imports(
    test_file: &Path,
    root: &Path,
    imports: &[Import],
    module_to_file: &HashMap<String, PathBuf>,
) -> HashSet<PathBuf> {
    let mut found = HashSet::new();
    let file_module = file_to_module(root, test_file);
    let file_parts: Vec<String> = file_module
        .as_deref()
        .map(|m| m.split('.').map(|s| s.to_string()).collect())
        .unwrap_or_default();

    for imp in imports {
        let candidates: Vec<String> = match imp {
            Import::Absolute(m) => vec![m.clone()],
            Import::FromAbsolute { module, symbols } => {
                // `from foo.bar import baz` could mean either:
                //   - `baz` is a name re-exported from `foo/bar.py`, or
                //   - `baz` is a submodule `foo/bar/baz.py`.
                // We don't know which without running Python, so probe both.
                let mut v = vec![module.clone()];
                for s in symbols {
                    v.push(format!("{module}.{s}"));
                }
                v
            }
            Import::Relative {
                dots,
                tail,
                symbols,
            } => {
                if file_parts.is_empty() {
                    continue;
                }
                // N dots means go N-1 levels up from the current package.
                let drop = dots.saturating_sub(1);
                if drop > file_parts.len().saturating_sub(1) {
                    continue;
                }
                let mut base: Vec<String> = file_parts[..file_parts.len() - 1 - drop].to_vec();
                if !tail.is_empty() {
                    for seg in tail.split('.') {
                        base.push(seg.to_string());
                    }
                }
                if base.is_empty() {
                    continue;
                }
                let module = base.join(".");
                let mut v = vec![module.clone()];
                for s in symbols {
                    v.push(format!("{module}.{s}"));
                }
                v
            }
        };

        for c in candidates {
            // Direct hit.
            if let Some(p) = module_to_file.get(&c) {
                found.insert(p.clone());
            }
            // Parent-prefix walk: if `foo.bar.baz` isn't directly mapped,
            // try `foo.bar`, which might resolve to `foo/bar/__init__.py`.
            let mut parts: Vec<&str> = c.split('.').collect();
            while parts.len() > 1 {
                parts.pop();
                let key = parts.join(".");
                if let Some(p) = module_to_file.get(&key) {
                    found.insert(p.clone());
                    break;
                }
            }
        }
    }

    found
}

#[cfg(all(test, unix))]
mod tests {
    use super::*;
    use std::path::PathBuf;

    // ── file_to_module ──────────────────────────────────────────────────────

    #[test]
    fn file_to_module_flat_layout() {
        let root = PathBuf::from("/r");
        assert_eq!(
            file_to_module(&root, &PathBuf::from("/r/pkg/foo.py")),
            Some("pkg.foo".into())
        );
    }

    #[test]
    fn file_to_module_src_layout_strips_leading_src() {
        let root = PathBuf::from("/r");
        assert_eq!(
            file_to_module(&root, &PathBuf::from("/r/src/pkg/foo.py")),
            Some("pkg.foo".into())
        );
    }

    #[test]
    fn file_to_module_init_drops_filename() {
        let root = PathBuf::from("/r");
        assert_eq!(
            file_to_module(&root, &PathBuf::from("/r/pkg/sub/__init__.py")),
            Some("pkg.sub".into())
        );
    }

    #[test]
    fn file_to_module_top_level_init_returns_none() {
        let root = PathBuf::from("/r");
        // Just `__init__.py` at the project root has no package.
        assert_eq!(
            file_to_module(&root, &PathBuf::from("/r/__init__.py")),
            None
        );
    }

    // ── scan_imports_str ────────────────────────────────────────────────────

    #[test]
    fn scan_plain_import() {
        let v = scan_imports_str("import foo");
        assert_eq!(v, vec![Import::Absolute("foo".into())]);
    }

    #[test]
    fn scan_dotted_and_aliased() {
        let v = scan_imports_str("import foo.bar, baz as b");
        assert_eq!(
            v,
            vec![
                Import::Absolute("foo.bar".into()),
                Import::Absolute("baz".into()),
            ]
        );
    }

    #[test]
    fn scan_from_with_symbols() {
        let v = scan_imports_str("from foo.bar import a, b as c");
        assert_eq!(
            v,
            vec![Import::FromAbsolute {
                module: "foo.bar".into(),
                symbols: vec!["a".into(), "b".into()],
            }]
        );
    }

    #[test]
    fn scan_from_relative() {
        let v = scan_imports_str("from .types import X");
        assert_eq!(
            v,
            vec![Import::Relative {
                dots: 1,
                tail: "types".into(),
                symbols: vec!["X".into()],
            }]
        );
    }

    #[test]
    fn scan_from_relative_double_dot() {
        let v = scan_imports_str("from .. import siblings");
        assert_eq!(
            v,
            vec![Import::Relative {
                dots: 2,
                tail: "".into(),
                symbols: vec!["siblings".into()],
            }]
        );
    }

    #[test]
    fn scan_strips_inline_comment() {
        let v = scan_imports_str("import foo  # noqa: F401");
        assert_eq!(v, vec![Import::Absolute("foo".into())]);
    }

    #[test]
    fn scan_skips_imports_inside_docstring() {
        let src = "\"\"\"Module doc.\nfrom foo import bar\n\"\"\"\nimport real_one\n";
        let v = scan_imports_str(src);
        assert_eq!(v, vec![Import::Absolute("real_one".into())]);
    }

    #[test]
    fn scan_indented_import_under_type_checking() {
        let v = scan_imports_str("if TYPE_CHECKING:\n    from .types import X");
        // The `if TYPE_CHECKING:` line is not an import; the indented
        // `from` line is. We should still pick it up.
        assert_eq!(
            v,
            vec![Import::Relative {
                dots: 1,
                tail: "types".into(),
                symbols: vec!["X".into()],
            }]
        );
    }

    // ── resolve_imports ────────────────────────────────────────────────────

    fn module_index(pairs: &[(&str, &str)]) -> HashMap<String, PathBuf> {
        pairs
            .iter()
            .map(|(m, p)| (m.to_string(), PathBuf::from(p)))
            .collect()
    }

    #[test]
    fn resolve_from_absolute_module_hit() {
        let idx = module_index(&[("pkg.bar", "/r/pkg/bar.py")]);
        let v = resolve_imports(
            &PathBuf::from("/r/tests/test_x.py"),
            &PathBuf::from("/r"),
            &[Import::FromAbsolute {
                module: "pkg.bar".into(),
                symbols: vec!["thing".into()],
            }],
            &idx,
        );
        assert!(v.contains(&PathBuf::from("/r/pkg/bar.py")));
    }

    #[test]
    fn resolve_from_absolute_submodule_hit() {
        // `from pkg.bar import baz` where `baz` is a submodule. The fix
        // for review item m8: probe `pkg.bar.baz` in addition to `pkg.bar`.
        let idx = module_index(&[("pkg.bar.baz", "/r/pkg/bar/baz.py")]);
        let v = resolve_imports(
            &PathBuf::from("/r/tests/test_x.py"),
            &PathBuf::from("/r"),
            &[Import::FromAbsolute {
                module: "pkg.bar".into(),
                symbols: vec!["baz".into()],
            }],
            &idx,
        );
        assert!(
            v.contains(&PathBuf::from("/r/pkg/bar/baz.py")),
            "FromAbsolute should probe submodule paths, found: {:?}",
            v
        );
    }

    #[test]
    fn resolve_relative_one_dot_resolves_to_sibling() {
        // Test file is `pkg.tests.test_x` (`pkg/tests/test_x.py`).
        // `from .types import X` should resolve to `pkg.tests.types`.
        let idx = module_index(&[("pkg.tests.types", "/r/pkg/tests/types.py")]);
        let v = resolve_imports(
            &PathBuf::from("/r/pkg/tests/test_x.py"),
            &PathBuf::from("/r"),
            &[Import::Relative {
                dots: 1,
                tail: "types".into(),
                symbols: vec!["X".into()],
            }],
            &idx,
        );
        assert!(v.contains(&PathBuf::from("/r/pkg/tests/types.py")));
    }

    #[test]
    fn resolve_relative_two_dots_goes_one_level_up() {
        // From `pkg.sub.tests.test_x` doing `from ..helpers import h` should
        // resolve to `pkg.sub.helpers` (one level up from the current package).
        let idx = module_index(&[("pkg.sub.helpers", "/r/pkg/sub/helpers.py")]);
        let v = resolve_imports(
            &PathBuf::from("/r/pkg/sub/tests/test_x.py"),
            &PathBuf::from("/r"),
            &[Import::Relative {
                dots: 2,
                tail: "helpers".into(),
                symbols: vec!["h".into()],
            }],
            &idx,
        );
        assert!(
            v.contains(&PathBuf::from("/r/pkg/sub/helpers.py")),
            "two-dot relative should pop one package level, found: {:?}",
            v
        );
    }

    #[test]
    fn resolve_falls_back_to_parent_prefix() {
        // `from foo.bar.deep import x` where only `foo.bar` exists as a
        // file (e.g. `foo/bar.py`); the parent-prefix walk should land on it.
        let idx = module_index(&[("foo.bar", "/r/foo/bar.py")]);
        let v = resolve_imports(
            &PathBuf::from("/r/tests/test_x.py"),
            &PathBuf::from("/r"),
            &[Import::FromAbsolute {
                module: "foo.bar.deep".into(),
                symbols: vec!["x".into()],
            }],
            &idx,
        );
        assert!(v.contains(&PathBuf::from("/r/foo/bar.py")));
    }

    // ── backslash continuation ────────────────────────────────────────────

    #[test]
    fn scan_backslash_continuation_from_import() {
        let v = scan_imports_str("from foo import a, \\\n    b, c");
        assert_eq!(
            v,
            vec![Import::FromAbsolute {
                module: "foo".into(),
                symbols: vec!["a".into(), "b".into(), "c".into()],
            }]
        );
    }

    #[test]
    fn scan_backslash_continuation_plain_import() {
        let v = scan_imports_str("import foo, \\\n    bar");
        assert_eq!(
            v,
            vec![
                Import::Absolute("foo".into()),
                Import::Absolute("bar".into()),
            ]
        );
    }

    // ── multi-line parenthesized imports ──────────────────────────────────

    #[test]
    fn scan_multiline_paren_import() {
        let src = "from foo import (\n    a,\n    b,\n    c,\n)";
        let v = scan_imports_str(src);
        assert_eq!(
            v,
            vec![Import::FromAbsolute {
                module: "foo".into(),
                symbols: vec!["a".into(), "b".into(), "c".into()],
            }]
        );
    }

    #[test]
    fn scan_multiline_paren_with_aliases() {
        let src = "from foo import (\n    a as x,\n    b,\n)";
        let v = scan_imports_str(src);
        assert_eq!(
            v,
            vec![Import::FromAbsolute {
                module: "foo".into(),
                symbols: vec!["a".into(), "b".into()],
            }]
        );
    }

    #[test]
    fn scan_multiline_paren_with_comments() {
        let src = "from foo import (\n    a,  # the a thing\n    b,  # noqa\n)";
        let v = scan_imports_str(src);
        assert_eq!(
            v,
            vec![Import::FromAbsolute {
                module: "foo".into(),
                symbols: vec!["a".into(), "b".into()],
            }]
        );
    }

    #[test]
    fn scan_single_line_paren_import_still_works() {
        let v = scan_imports_str("from foo import (a, b)");
        assert_eq!(
            v,
            vec![Import::FromAbsolute {
                module: "foo".into(),
                symbols: vec!["a".into(), "b".into()],
            }]
        );
    }
}
