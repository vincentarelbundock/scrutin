use anyhow::{Context, Result, bail};
use globset::{Glob, GlobSet, GlobSetBuilder};
use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::sync::Arc;

use crate::analysis::walk;
use crate::project::config::SuiteConfig;
use crate::project::plugin::{self, Plugin};

/// Per-suite worker hook script paths, resolved by the caller against the
/// project root before a `TestSuite` is built.
#[derive(Debug, Clone, Default)]
pub struct WorkerHookPaths {
    pub startup: Option<PathBuf>,
    pub teardown: Option<PathBuf>,
}

/// One tool's slice of a project: which plugin runs it, which directory
/// it runs from (the *suite root*), which files it operates on, and which
/// files to watch for reruns. A `Package` can hold multiple suites,
/// supporting more than one tool or language side-by-side — including
/// monorepos where R lives under `r/` and Python under `python/`.
#[derive(Clone)]
pub struct TestSuite {
    pub plugin: Arc<dyn Plugin>,
    /// Suite root: absolute, canonicalized. Drives the subprocess CWD and
    /// the `SCRUTIN_PKG_DIR` env var. `DESCRIPTION` / `pyproject.toml` /
    /// `.venv` / tool-specific configs are discovered relative to this.
    pub root: PathBuf,
    /// Glob patterns for files the tool operates on, stored as absolute
    /// pattern strings (each joined under `root`). These drive test
    /// discovery and are matched by `owns_test_file` for routing.
    pub run: Vec<String>,
    /// Glob patterns for files watched to trigger reruns. Drives the
    /// watcher registration + dep-map staleness detection.
    pub watch: Vec<String>,
    /// Compiled `run` patterns for fast matching in `owns_test_file` and
    /// in discovery.
    pub run_set: GlobSet,
    /// Compiled `watch` patterns.
    pub watch_set: GlobSet,
    /// Worker hooks scoped to this suite, resolved from
    /// `[hooks.<lang>.<tool>]` (with `[hooks.<lang>]` fallback).
    pub worker_hooks: WorkerHookPaths,
    /// Optional path to a user-provided runner script. When `Some`, the
    /// engine reads this file instead of the embedded default. Set from
    /// `[[suite]].runner` or `[<tool>].runner` in .scrutin/config.toml.
    pub runner_override: Option<PathBuf>,
    /// True when the caller enumerated an explicit file list (file-mode).
    /// In that case `owns_test_file` skips the plugin's `is_test_file`
    /// predicate: the user already chose which files to run, so the
    /// predicate (which is meant for auto-walked trees) would only drop
    /// legitimate inputs (e.g. skyspell on `.R`, `.py`, `.rs` sources).
    pub explicit_files: bool,
}

impl TestSuite {
    /// Build a `TestSuite` from a plugin, root, and optional user-supplied
    /// run/watch globs. When `run` is empty the plugin's defaults are used;
    /// when `watch` is empty it falls back to the plugin's watch defaults,
    /// or to the resolved `run` patterns if no watch defaults exist.
    ///
    /// The root is cleaned of Windows `\\?\` verbatim prefixes and all
    /// patterns are anchored under it, so glob matching works uniformly
    /// across platforms.
    pub fn new(
        plugin: Arc<dyn Plugin>,
        root: PathBuf,
        run: Vec<String>,
        watch: Vec<String>,
        worker_hooks: WorkerHookPaths,
        runner_override: Option<PathBuf>,
    ) -> Result<Self> {
        let root = strip_verbatim_prefix(root);

        let run_patterns = if run.is_empty() {
            default_patterns(&root, &plugin.default_run())
        } else {
            anchor_patterns(&root, &run)
        };

        let watch_patterns = if !watch.is_empty() {
            anchor_patterns(&root, &watch)
        } else {
            let defaults = plugin.default_watch();
            if defaults.is_empty() {
                run_patterns.clone()
            } else {
                default_patterns(&root, &defaults)
            }
        };

        let run_set = build_globset(&run_patterns)
            .with_context(|| format!("compiling run globs for {}", plugin.name()))?;
        let watch_set = build_globset(&watch_patterns)
            .with_context(|| format!("compiling watch globs for {}", plugin.name()))?;

        Ok(Self {
            plugin,
            root,
            run: run_patterns,
            watch: watch_patterns,
            run_set,
            watch_set,
            worker_hooks,
            runner_override,
            explicit_files: false,
        })
    }

    /// Does this suite claim `path` as one of its input files? A suite
    /// owns a path iff `path` matches any of the suite's `run` globs
    /// AND the plugin's `is_test_file` predicate accepts it.
    ///
    /// The glob-match is authoritative for which suite (monorepo routing);
    /// the predicate rejects files that happen to live in the tree but
    /// aren't inputs (e.g. `conftest.py` under a pytest `run = "tests/**/*.py"`).
    ///
    /// In file-mode (`explicit_files = true`) the predicate is bypassed
    /// because the caller already enumerated exactly which files to run.
    pub fn owns_test_file(&self, path: &Path) -> bool {
        if !self.run_set.is_match(path) {
            return false;
        }
        self.explicit_files || self.plugin.is_test_file(path)
    }

    /// Directories to walk when discovering files. Computed as the
    /// longest non-glob prefix of each `run` pattern, deduped.
    pub fn run_search_dirs(&self) -> Vec<PathBuf> {
        glob_prefix_dirs(&self.run)
    }

    /// Directories to watch. Computed as the longest non-glob prefix of
    /// each `watch` pattern, deduped.
    pub fn watch_search_dirs(&self) -> Vec<PathBuf> {
        glob_prefix_dirs(&self.watch)
    }
}

#[derive(Clone)]
pub struct Package {
    pub name: String,
    /// Project root: where `.scrutin/config.toml` lives. Anchors shared
    /// state (state.db, runner scripts, hooks, .gitignore, log buffer,
    /// git metadata). Distinct from any suite's `root`.
    pub root: PathBuf,
    /// All active test suites for this project. Always non-empty.
    pub test_suites: Vec<TestSuite>,
    /// Verbatim extra args appended to `pytest.main()` in the runner
    /// subprocess. Plumbed via the `SCRUTIN_PYTEST_EXTRA_ARGS` env var.
    pub pytest_extra_args: Vec<String>,
    /// Args spliced between `skyspell` and the subcommand on every skyspell
    /// invocation. Includes `--lang` (skyspell requires it).
    pub skyspell_extra_args: Vec<String>,
    /// Args appended to `skyspell add` after the subcommand, before the
    /// `<WORD>`. Controls whitelist scope (default: `["--project"]`).
    pub skyspell_add_args: Vec<String>,
    /// Resolved Python interpreter command (may be multiple tokens, e.g.
    /// `["uv", "run", "python"]`). When non-empty, replaces the interpreter
    /// that `py_subprocess_cmd` would auto-detect.
    pub python_interpreter: Vec<String>,
    /// User-declared env vars from `[env]`.
    pub env: BTreeMap<String, String>,
}

impl Package {
    /// Build a `Package`. When `suite_configs` is non-empty, use those
    /// explicit declarations (each suite root resolved against `pkg_root`).
    /// Otherwise auto-detect plugins by scanning `pkg_root` for marker
    /// files, filtered by `tool_filter` ("auto" = all detected tools).
    pub fn new(
        pkg_root: PathBuf,
        suite_configs: &[SuiteConfig],
        tool_filter: &str,
        pytest_extra_args: &[String],
        skyspell_extra_args: &[String],
        skyspell_add_args: &[String],
        python_interpreter: Vec<String>,
        mut resolve_hooks: impl FnMut(&dyn Plugin) -> Result<WorkerHookPaths>,
        env: BTreeMap<String, String>,
    ) -> Result<Self> {
        let mut test_suites = Vec::new();
        let mut name: Option<String> = None;

        if suite_configs.is_empty() {
            // Auto-detect: scan for marker files.
            let plugins = plugin::detect_plugins(&pkg_root, tool_filter)?;
            name = Some(plugins[0].project_name(&pkg_root));
            for plugin in plugins {
                let worker_hooks = resolve_hooks(plugin.as_ref())?;
                test_suites.push(TestSuite::new(
                    plugin,
                    pkg_root.clone(),
                    Vec::new(),
                    Vec::new(),
                    worker_hooks,
                    None,
                )?);
            }
        } else {
            // Explicit [[suite]] declarations.
            for sc in suite_configs {
                let plugin = plugin::plugin_by_name(&sc.tool).ok_or_else(|| {
                    anyhow::anyhow!("Unknown tool {:?} in [[suite]]", sc.tool)
                })?;
                let suite_root = resolve_suite_root(&pkg_root, &sc.root);
                if name.is_none() {
                    name = Some(plugin.project_name(&suite_root));
                }
                let worker_hooks = resolve_hooks(plugin.as_ref())?;
                test_suites.push(TestSuite::new(
                    plugin,
                    suite_root,
                    sc.run.clone(),
                    sc.watch.clone(),
                    worker_hooks,
                    sc.runner.clone(),
                )?);
            }
        }

        Ok(Package {
            name: name.unwrap_or_else(|| dir_name(&pkg_root)),
            root: pkg_root,
            test_suites,
            pytest_extra_args: pytest_extra_args.to_vec(),
            skyspell_extra_args: skyspell_extra_args.to_vec(),
            skyspell_add_args: skyspell_add_args.to_vec(),
            python_interpreter,
            env,
        })
    }

    /// Build a `Package` for file-mode: a handful of explicit file paths run
    /// through a single named command-mode plugin. `pkg_root` is a synthetic
    /// scratch dir (typically a tempdir) that anchors `.scrutin/` state; it
    /// does not need to contain `files`. The suite's own root is set to the
    /// files' common ancestor so tool-specific config (`pyproject.toml`,
    /// `ruff.toml`) is still discoverable.
    pub fn from_files(
        pkg_root: PathBuf,
        files: &[PathBuf],
        tool_name: &str,
        pytest_extra_args: &[String],
        skyspell_extra_args: &[String],
        skyspell_add_args: &[String],
        python_interpreter: Vec<String>,
        env: BTreeMap<String, String>,
    ) -> Result<Self> {
        if files.is_empty() {
            bail!("file-mode requires at least one file");
        }
        let plugin = plugin::plugin_by_name(tool_name)
            .ok_or_else(|| anyhow::anyhow!("Unknown tool: {}", tool_name))?;

        // Refuse worker-mode plugins in file-mode: without a project root,
        // their runner subprocess has nowhere to live.
        let stub = Package {
            name: String::new(),
            root: pkg_root.clone(),
            test_suites: Vec::new(),
            pytest_extra_args: pytest_extra_args.to_vec(),
            skyspell_extra_args: skyspell_extra_args.to_vec(),
            skyspell_add_args: skyspell_add_args.to_vec(),
            python_interpreter: python_interpreter.clone(),
            env: env.clone(),
        };
        if plugin.command_spec(&pkg_root, &stub).is_none() {
            bail!(
                "{} requires a project root; run scrutin from the project directory instead",
                plugin.name()
            );
        }

        let suite_root = common_ancestor(files)
            .unwrap_or_else(|| files[0].parent().unwrap_or(Path::new(".")).to_path_buf());

        // Build a run glob matching each input file as a literal path.
        // globset treats paths without metachars (*, ?, [, {) as exact
        // matches, which is what we want. Paths containing glob metachars
        // are a rare edge case: the engine invokes the tool with the
        // literal path regardless (see `engine::command_pool`), so suite
        // routing is the only thing that could misroute; document that
        // users with exotic filenames use `[[suite]] run = [...]` instead.
        let run_patterns: Vec<String> = files
            .iter()
            .map(|f| f.to_string_lossy().into_owned())
            .collect();
        let mut gsb = GlobSetBuilder::new();
        for pat in &run_patterns {
            gsb.add(
                Glob::new(pat)
                    .with_context(|| format!("compiling exact-path glob for {}", pat))?,
            );
        }
        let run_set = gsb.build().context("compiling file-mode run globset")?;
        // watch isn't used in file-mode, but TestSuite keeps a non-empty
        // watch_set invariant; alias it to the run set.
        let watch_set = run_set.clone();

        let suite = TestSuite {
            plugin: plugin.clone(),
            root: suite_root.clone(),
            run: run_patterns.clone(),
            watch: run_patterns,
            run_set,
            watch_set,
            worker_hooks: WorkerHookPaths::default(),
            runner_override: None,
            explicit_files: true,
        };

        Ok(Package {
            name: plugin.project_name(&suite_root),
            root: pkg_root,
            test_suites: vec![suite],
            pytest_extra_args: pytest_extra_args.to_vec(),
            skyspell_extra_args: skyspell_extra_args.to_vec(),
            skyspell_add_args: skyspell_add_args.to_vec(),
            python_interpreter,
            env,
        })
    }

    /// Resolved source/watch dirs across every active suite, deduplicated.
    /// Only directories that exist on disk are returned. Drives the
    /// watcher registration and the hash/dep-map walker.
    pub fn resolved_source_dirs(&self) -> Vec<PathBuf> {
        let mut out: Vec<PathBuf> = Vec::new();
        let mut seen: std::collections::HashSet<PathBuf> = std::collections::HashSet::new();
        for suite in &self.test_suites {
            for dir in suite.watch_search_dirs() {
                if dir.is_dir() && seen.insert(dir.clone()) {
                    out.push(dir);
                }
            }
        }
        out
    }

    /// Resolved "run" dirs across every active suite, deduplicated.
    pub fn resolved_test_dirs(&self) -> Vec<PathBuf> {
        let mut out: Vec<PathBuf> = Vec::new();
        let mut seen: std::collections::HashSet<PathBuf> = std::collections::HashSet::new();
        for suite in &self.test_suites {
            for dir in suite.run_search_dirs() {
                if dir.is_dir() && seen.insert(dir.clone()) {
                    out.push(dir);
                }
            }
        }
        out
    }

    /// Discover every file the active suites will run against.
    /// Walks each suite's `run_search_dirs` and keeps files whose path
    /// matches the suite's `run_set` AND its plugin's `is_test_file`.
    pub fn test_files(&self) -> Result<Vec<PathBuf>> {
        let mut out = Vec::new();
        for suite in &self.test_suites {
            for dir in suite.run_search_dirs() {
                if !dir.is_dir() {
                    continue;
                }
                let files = walk::collect_files(&dir, |p| {
                    suite.run_set.is_match(p) && suite.plugin.is_test_file(p)
                });
                out.extend(files);
            }
        }
        out.sort();
        out.dedup();
        Ok(out)
    }

    /// Find which suite owns a given test file. The first matching suite
    /// wins; suites are checked in registration order.
    pub fn suite_for(&self, path: &Path) -> Option<&TestSuite> {
        self.test_suites.iter().find(|s| s.owns_test_file(path))
    }

    /// `+`-joined list of active tool names (e.g. `tinytest+testthat+pytest`).
    /// Used by headers and the TUI label.
    pub fn tool_names(&self) -> String {
        let names: Vec<&str> = self.test_suites.iter().map(|s| s.plugin.name()).collect();
        names.join("+")
    }

    /// Does any active suite recognize `path` as a test file?
    pub fn is_any_test_file(&self, path: &Path) -> bool {
        self.suite_for(path).is_some()
    }

    /// Does any active suite recognize `path` as a source/watch file?
    /// (Matches when `path` is in a suite's `watch_set` — the thing the
    /// watcher's noise filter actually cares about.)
    pub fn is_any_source_file(&self, path: &Path) -> bool {
        self.test_suites.iter().any(|s| s.watch_set.is_match(path))
    }
}

// ── Helpers ─────────────────────────────────────────────────────────────

fn resolve_suite_root(pkg_root: &Path, suite_root: &Path) -> PathBuf {
    let joined = if suite_root.is_absolute() {
        suite_root.to_path_buf()
    } else {
        pkg_root.join(suite_root)
    };
    let canonical = std::fs::canonicalize(&joined).unwrap_or(joined);
    strip_verbatim_prefix(canonical)
}

/// On Windows, `std::fs::canonicalize` returns a verbatim path with the
/// `\\?\` prefix. That prefix breaks glob matching because the rest of
/// scrutin walks non-verbatim paths, so patterns built from a verbatim
/// root never match. This strips `\\?\` for ordinary drive paths (leaving
/// UNC-style `\\?\UNC\...` and any non-prefixed path alone).
#[cfg(windows)]
fn strip_verbatim_prefix(p: PathBuf) -> PathBuf {
    let s = p.to_string_lossy();
    if let Some(rest) = s.strip_prefix(r"\\?\")
        && !rest.starts_with("UNC\\")
    {
        return PathBuf::from(rest);
    }
    p
}

#[cfg(not(windows))]
fn strip_verbatim_prefix(p: PathBuf) -> PathBuf {
    p
}

/// Anchor each pattern under `root`. Absolute patterns stay as-is;
/// relative patterns get joined under `root` so matching is done against
/// absolute paths (and a pattern like `tests/**/*.py` applies only to
/// files under this suite's root).
fn anchor_patterns(root: &Path, patterns: &[String]) -> Vec<String> {
    patterns
        .iter()
        .map(|p| anchor_pattern(root, p))
        .collect()
}

fn default_patterns(root: &Path, defaults: &[String]) -> Vec<String> {
    defaults
        .iter()
        .map(|p| anchor_pattern(root, p))
        .collect()
}

/// Join a glob pattern under `root`, producing an absolute glob string.
///
/// `globset` matches against forward-slash-separated paths on every
/// platform (it treats `\` as a literal character, not a separator),
/// so we build the result as a string with explicit `/` rather than
/// going through `Path::join`, which would emit `\` on Windows.
/// Absolute patterns pass through unchanged.
fn anchor_pattern(root: &Path, pattern: &str) -> String {
    if Path::new(pattern).is_absolute() {
        return pattern.to_string();
    }
    let root_raw = root.to_string_lossy();
    let root_str: std::borrow::Cow<'_, str> = if root_raw.contains('\\') {
        std::borrow::Cow::Owned(root_raw.replace('\\', "/"))
    } else {
        root_raw
    };
    let sep = if root_str.ends_with('/') { "" } else { "/" };
    format!("{root_str}{sep}{pattern}")
}

fn build_globset(patterns: &[String]) -> Result<GlobSet> {
    let mut builder = GlobSetBuilder::new();
    for p in patterns {
        let glob = Glob::new(p).with_context(|| format!("invalid glob {p:?}"))?;
        builder.add(glob);
    }
    builder.build().context("building glob set")
}

/// Longest non-glob prefix of a pattern — the dir we need to walk to find
/// files the pattern could match. Returns an existing absolute path when
/// possible; otherwise the closest ancestor that does exist (so the
/// watcher initialization still has something to register).
fn glob_prefix_dirs(patterns: &[String]) -> Vec<PathBuf> {
    let mut out: Vec<PathBuf> = Vec::new();
    let mut seen: std::collections::HashSet<PathBuf> = std::collections::HashSet::new();
    for pat in patterns {
        let dir = longest_literal_prefix(pat);
        if seen.insert(dir.clone()) {
            out.push(dir);
        }
    }
    out
}

fn longest_literal_prefix(pattern: &str) -> PathBuf {
    // Rebuild the longest leading path that contains no glob
    // metacharacters. The pattern is always forward-slash-separated
    // (we build it that way in `anchor_pattern`), so we scan segments
    // and re-join as a string rather than calling `PathBuf::push`
    // segment-by-segment: on Windows the latter mishandles bare drive
    // components (`push("D:")` then `push("a")` does NOT yield `D:\a`).
    let (absolute, rest) = match pattern.strip_prefix('/') {
        Some(rest) => (true, rest),
        None => (false, pattern),
    };
    let literal: Vec<&str> = rest
        .split('/')
        .take_while(|seg| !contains_glob_meta(seg))
        .filter(|seg| !seg.is_empty())
        .collect();

    let cur = if absolute {
        PathBuf::from(format!("/{}", literal.join("/")))
    } else if literal.is_empty() {
        // Top-level glob like `**/*.py`: walk from `.`.
        PathBuf::from(".")
    } else {
        PathBuf::from(literal.join("/"))
    };

    // If the literal prefix points to a file rather than a directory,
    // return the parent; the caller wants a dir for walking.
    if cur.is_file()
        && let Some(parent) = cur.parent()
    {
        return parent.to_path_buf();
    }
    cur
}

fn contains_glob_meta(s: &str) -> bool {
    s.contains('*') || s.contains('?') || s.contains('[') || s.contains('{')
}

fn dir_name(root: &Path) -> String {
    root.file_name()
        .and_then(|n| n.to_str())
        .unwrap_or("<unknown>")
        .to_string()
}

/// Longest shared directory prefix of the given paths. Returns `None` if
/// `paths` is empty. Used by file-mode to derive a suite root from a set
/// of file arguments: `["/a/b/c.md", "/a/b/d.md"]` -> `Some("/a/b")`.
fn common_ancestor(paths: &[PathBuf]) -> Option<PathBuf> {
    let first = paths.first()?;
    let mut ancestor = first
        .parent()
        .map(|p| p.to_path_buf())
        .unwrap_or_else(|| PathBuf::from("."));
    for p in &paths[1..] {
        let parent = p.parent().unwrap_or_else(|| Path::new("."));
        while !parent.starts_with(&ancestor) {
            match ancestor.parent() {
                Some(up) => ancestor = up.to_path_buf(),
                None => return Some(PathBuf::from("/")),
            }
        }
    }
    Some(ancestor)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn longest_literal_prefix_plain_glob() {
        assert_eq!(
            longest_literal_prefix("/repo/r/tests/testthat/**/test-*.R"),
            PathBuf::from("/repo/r/tests/testthat"),
        );
    }

    #[test]
    fn longest_literal_prefix_top_level_glob() {
        assert_eq!(
            longest_literal_prefix("/repo/**/*.py"),
            PathBuf::from("/repo"),
        );
    }

    #[test]
    fn longest_literal_prefix_literal_file() {
        // A pure literal path has the whole path as prefix; caller gets
        // the parent when it's a real file (checked at the filesystem
        // layer, not here — since the test path doesn't exist, we keep
        // the whole literal).
        assert_eq!(
            longest_literal_prefix("/repo/r/inst/extdata/greet.txt"),
            PathBuf::from("/repo/r/inst/extdata/greet.txt"),
        );
    }

    #[test]
    fn anchor_pattern_relative_joins_under_root() {
        let s = anchor_pattern(Path::new("/repo/r"), "tests/testthat/**/test-*.R");
        assert_eq!(s, "/repo/r/tests/testthat/**/test-*.R");
    }

    #[test]
    fn anchor_pattern_absolute_passes_through() {
        let s = anchor_pattern(Path::new("/repo/r"), "/elsewhere/foo.R");
        assert_eq!(s, "/elsewhere/foo.R");
    }

    #[test]
    fn contains_glob_meta_detects_wildcards() {
        assert!(contains_glob_meta("**/*.py"));
        assert!(contains_glob_meta("foo?.R"));
        assert!(contains_glob_meta("foo[ab].R"));
        assert!(contains_glob_meta("foo{a,b}.R"));
        assert!(!contains_glob_meta("foo/bar.R"));
    }

    /// Build a `TestSuite` with canonicalized `root` and compiled globs,
    /// panicking on failure. Keeps the routing tests readable.
    fn suite(name: &str, root: &Path, run: Vec<&str>, watch: Vec<&str>) -> TestSuite {
        let plugin = plugin::plugin_by_name(name)
            .unwrap_or_else(|| panic!("{name} plugin registered"));
        TestSuite::new(
            plugin,
            std::fs::canonicalize(root).unwrap(),
            run.into_iter().map(String::from).collect(),
            watch.into_iter().map(String::from).collect(),
            WorkerHookPaths::default(),
            None,
        )
        .expect("compile globs")
    }

    #[test]
    fn monorepo_routing_keeps_suites_separate() {
        // Two suites with different roots under the same project.
        // testthat's run globs only match files under /repo/r/.
        // pytest's run globs only match files under /repo/python/.
        // suite_for should route unambiguously.
        let tmp = tempfile::tempdir().expect("tempdir");
        let pkg_root = tmp.path();
        let r_root = pkg_root.join("r");
        let py_root = pkg_root.join("python");
        std::fs::create_dir_all(r_root.join("tests/testthat")).unwrap();
        std::fs::create_dir_all(py_root.join("tests")).unwrap();
        std::fs::write(r_root.join("DESCRIPTION"), "Package: demo\nVersion: 0.0.1\n").unwrap();
        std::fs::write(py_root.join("pyproject.toml"), "[project]\nname=\"demo\"\n").unwrap();

        let r_suite = suite(
            "testthat",
            &r_root,
            vec!["tests/testthat/**/test-*.R"],
            vec!["R/**/*.R"],
        );
        let py_suite = suite(
            "pytest",
            &py_root,
            vec!["tests/**/test_*.py"],
            vec!["src/**/*.py"],
        );

        let pkg = Package {
            name: "monorepo".into(),
            root: pkg_root.to_path_buf(),
            test_suites: vec![r_suite, py_suite],
            pytest_extra_args: Vec::new(),
            skyspell_extra_args: Vec::new(),
            skyspell_add_args: Vec::new(),
            python_interpreter: Vec::new(),
            env: BTreeMap::new(),
        };

        let r_test = std::fs::canonicalize(&r_root)
            .unwrap()
            .join("tests/testthat/test-load.R");
        let py_test = std::fs::canonicalize(&py_root)
            .unwrap()
            .join("tests/test_load.py");

        let s_r = pkg.suite_for(&r_test).expect("R test routes to a suite");
        assert_eq!(s_r.plugin.name(), "testthat");
        let s_py = pkg.suite_for(&py_test).expect("Py test routes to a suite");
        assert_eq!(s_py.plugin.name(), "pytest");

        // Cross-routing: a .py path under /repo/r/ should not match testthat,
        // nor should an .R path under /repo/python/ match pytest.
        let cross_py = std::fs::canonicalize(&r_root).unwrap().join("tests/test_foo.py");
        let cross_r = std::fs::canonicalize(&py_root).unwrap().join("tests/test-foo.R");
        assert!(pkg.suite_for(&cross_py).is_none_or(|s| s.plugin.name() != "testthat"));
        assert!(pkg.suite_for(&cross_r).is_none_or(|s| s.plugin.name() != "pytest"));
    }

    #[test]
    fn scattered_files_routing_no_shared_ancestor() {
        // Two files in unrelated directories both routed to one suite.
        let tmp = tempfile::tempdir().unwrap();
        let root = tmp.path();
        std::fs::create_dir_all(root.join("a")).unwrap();
        std::fs::create_dir_all(root.join("b")).unwrap();

        let suite = suite(
            "testthat",
            root,
            vec!["a/test-foo.R", "b/test-bar.R"],
            vec![],
        );

        let foo = std::fs::canonicalize(root).unwrap().join("a/test-foo.R");
        let bar = std::fs::canonicalize(root).unwrap().join("b/test-bar.R");
        assert!(suite.owns_test_file(&foo));
        assert!(suite.owns_test_file(&bar));
        // A third path not in the run list doesn't get claimed.
        let other = std::fs::canonicalize(root).unwrap().join("a/test-other.R");
        assert!(!suite.owns_test_file(&other));
    }

    // ── common_ancestor (file-mode suite root derivation) ─────────────────

    #[test]
    fn common_ancestor_same_directory() {
        let a = PathBuf::from("/repo/docs/intro.md");
        let b = PathBuf::from("/repo/docs/advanced.md");
        assert_eq!(common_ancestor(&[a, b]), Some(PathBuf::from("/repo/docs")));
    }

    #[test]
    fn common_ancestor_different_directories() {
        let a = PathBuf::from("/repo/docs/intro.md");
        let b = PathBuf::from("/repo/README.md");
        assert_eq!(common_ancestor(&[a, b]), Some(PathBuf::from("/repo")));
    }

    #[test]
    fn common_ancestor_single_file() {
        let a = PathBuf::from("/repo/docs/intro.md");
        assert_eq!(common_ancestor(&[a]), Some(PathBuf::from("/repo/docs")));
    }

    #[test]
    fn common_ancestor_empty_returns_none() {
        assert_eq!(common_ancestor(&[]), None);
    }

    // ── Package::from_files (file-mode) ────────────────────────────────────

    #[test]
    fn from_files_builds_single_suite_for_command_mode_tool() {
        let tmp = tempfile::tempdir().unwrap();
        let root = tmp.path();
        let file = root.join("prose.md");
        std::fs::write(&file, "sample\n").unwrap();

        let pkg = Package::from_files(
            root.to_path_buf(),
            &[file.clone()],
            "skyspell",
            &[],
            &[],
            &[],
            Vec::new(),
            BTreeMap::new(),
        )
        .expect("skyspell is command-mode");

        assert_eq!(pkg.test_suites.len(), 1);
        let suite = &pkg.test_suites[0];
        assert_eq!(suite.plugin.name(), "skyspell");
        assert!(suite.run_set.is_match(&file));
    }

    #[test]
    fn from_files_rejects_worker_mode_tool() {
        let tmp = tempfile::tempdir().unwrap();
        let file = tmp.path().join("test_foo.py");
        std::fs::write(&file, "pass\n").unwrap();

        let err = Package::from_files(
            tmp.path().to_path_buf(),
            &[file],
            "pytest",
            &[],
            &[],
            &[],
            Vec::new(),
            BTreeMap::new(),
        )
        .map(|_| ())
        .expect_err("pytest is worker-mode and must be refused in file-mode");
        let msg = err.to_string();
        assert!(
            msg.contains("project root"),
            "error should explain why worker-mode plugins need a project root, got: {msg}",
        );
    }

    #[test]
    fn from_files_rejects_unknown_tool() {
        let tmp = tempfile::tempdir().unwrap();
        let file = tmp.path().join("x.md");
        std::fs::write(&file, "x\n").unwrap();
        let err = Package::from_files(
            tmp.path().to_path_buf(),
            &[file],
            "nope",
            &[],
            &[],
            &[],
            Vec::new(),
            BTreeMap::new(),
        )
        .map(|_| ())
        .expect_err("unknown tool name must error");
        assert!(err.to_string().to_lowercase().contains("unknown"));
    }

    #[test]
    fn from_files_rejects_empty_input() {
        let tmp = tempfile::tempdir().unwrap();
        let err = Package::from_files(
            tmp.path().to_path_buf(),
            &[],
            "skyspell",
            &[],
            &[],
            &[],
            Vec::new(),
            BTreeMap::new(),
        )
        .map(|_| ())
        .expect_err("empty file list must error");
        assert!(err.to_string().to_lowercase().contains("at least one"));
    }
}
