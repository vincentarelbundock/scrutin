//! Shared filesystem walker for the analysis layer.
//!
//! Both the R and Python analyzers used to ship their own hand-rolled
//! `read_dir`-recursive implementations with subtly different filtering. This
//! module gives them a single helper that takes a "should I keep this entry?"
//! closure plus a baseline ignore list of noise directories every analyzer
//! wants to skip (`.git`, `node_modules`, build/cache dirs, scrutin's own
//! state dir).

use std::path::{Path, PathBuf};

/// Directory names that every analyzer wants to skip. Centralized so the
/// list grows in one place. Anything matching this list is pruned before the
/// per-analyzer filter ever sees it.
const DEFAULT_IGNORED_DIRS: &[&str] = &[
    ".git",
    ".hg",
    ".svn",
    ".scrutin",
    ".pytest_cache",
    ".mypy_cache",
    ".ruff_cache",
    ".tox",
    ".venv",
    "venv",
    "__pycache__",
    "node_modules",
    "build",
    "dist",
    "target",
];

/// Is `name` one of the noise directories every analyzer / watcher wants
/// to skip? Exposed so the file watcher can apply the same predicate
/// without duplicating the list.
pub fn is_ignored_dir(name: &str) -> bool {
    DEFAULT_IGNORED_DIRS.contains(&name)
}

/// Recursively collect files under `dir` for which `keep(path)` returns true.
/// Symlinks are followed implicitly via `read_dir`'s default behavior; cycles
/// would loop forever, but no project layout we support has self-referential
/// symlinks.
pub fn collect_files<F>(dir: &Path, keep: F) -> Vec<PathBuf>
where
    F: Fn(&Path) -> bool,
{
    let mut out = Vec::new();
    walk(dir, &keep, &mut out);
    out.sort();
    out
}

fn walk<F>(dir: &Path, keep: &F, out: &mut Vec<PathBuf>)
where
    F: Fn(&Path) -> bool,
{
    let Ok(entries) = std::fs::read_dir(dir) else {
        return;
    };
    for entry in entries.flatten() {
        let path = entry.path();
        let Some(name) = path.file_name().and_then(|n| n.to_str()) else {
            continue;
        };
        if path.is_dir() {
            if is_ignored_dir(name) || name.starts_with('.') && name != "." && name != ".." {
                // Hidden dirs are skipped by default. The default ignore list
                // duplicates a few of these for clarity, but the dot-prefix
                // catch-all handles the long tail of tooling caches.
                continue;
            }
            walk(&path, keep, out);
        } else if path.is_file() && keep(&path) {
            out.push(path);
        }
    }
}

/// Convenience: keep files whose extension matches any in `exts`
/// (case-insensitive).
pub fn has_extension(path: &Path, exts: &[&str]) -> bool {
    match path.extension().and_then(|e| e.to_str()) {
        Some(e) => exts.iter().any(|x| x.eq_ignore_ascii_case(e)),
        None => false,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use std::io::Write;

    fn touch(path: &Path) {
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent).unwrap();
        }
        let mut f = fs::File::create(path).unwrap();
        writeln!(f, "x").unwrap();
    }

    #[test]
    fn collect_files_skips_ignored_dirs_and_hidden() {
        let tmp = tempdir();
        let root = tmp.path();
        touch(&root.join("a.py"));
        touch(&root.join("sub/b.py"));
        touch(&root.join(".git/c.py"));
        touch(&root.join("__pycache__/d.py"));
        touch(&root.join(".venv/e.py"));
        touch(&root.join(".hidden_tool/f.py"));

        let files = collect_files(root, |p| has_extension(p, &["py"]));
        let names: Vec<String> = files
            .iter()
            .map(|p| p.file_name().unwrap().to_string_lossy().to_string())
            .collect();
        assert!(names.contains(&"a.py".to_string()));
        assert!(names.contains(&"b.py".to_string()));
        assert!(!names.contains(&"c.py".to_string()), "should skip .git");
        assert!(
            !names.contains(&"d.py".to_string()),
            "should skip __pycache__"
        );
        assert!(!names.contains(&"e.py".to_string()), "should skip .venv");
        assert!(
            !names.contains(&"f.py".to_string()),
            "should skip dot-prefixed dirs"
        );
    }

    #[test]
    fn has_extension_is_case_insensitive() {
        assert!(has_extension(Path::new("foo.R"), &["r"]));
        assert!(has_extension(Path::new("foo.r"), &["R"]));
        assert!(has_extension(Path::new("foo.py"), &["py"]));
        assert!(!has_extension(Path::new("foo.txt"), &["py"]));
        assert!(!has_extension(Path::new("foo"), &["py"]));
    }

    fn tempdir() -> tempfile::TempDir {
        tempfile::tempdir().unwrap()
    }
}
