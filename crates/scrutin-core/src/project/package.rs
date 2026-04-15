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
}

impl TestSuite {
    /// Build a `TestSuite` by compiling the given glob patterns into
    /// `GlobSet`s. `run` / `watch` entries are anchored under `root`
    /// when relative, matching the `from_suites` semantics. Empty
    /// `watch` aliases to `run`.
    ///
    /// Intended for tests and internal builders. Production code should
    /// go through `Package::from_suites` / `from_auto_detect`.
    pub fn new(
        plugin: Arc<dyn Plugin>,
        root: PathBuf,
        run: Vec<String>,
        watch: Vec<String>,
        worker_hooks: WorkerHookPaths,
        runner_override: Option<PathBuf>,
    ) -> Result<Self> {
        let run_patterns = anchor_patterns(&root, &run);
        let watch_patterns = if watch.is_empty() {
            run_patterns.clone()
        } else {
            anchor_patterns(&root, &watch)
        };
        let run_set = build_globset(&run_patterns)?;
        let watch_set = build_globset(&watch_patterns)?;
        Ok(Self {
            plugin,
            root,
            run: run_patterns,
            watch: watch_patterns,
            run_set,
            watch_set,
            worker_hooks,
            runner_override,
        })
    }

    /// Does this suite claim `path` as one of its input files? A suite
    /// owns a path iff `path` matches any of the suite's `run` globs
    /// AND the plugin's `is_test_file` predicate accepts it.
    ///
    /// The glob-match is authoritative for which suite (monorepo routing);
    /// the predicate rejects files that happen to live in the tree but
    /// aren't inputs (e.g. `conftest.py` under a pytest `run = "tests/**/*.py"`).
    pub fn owns_test_file(&self, path: &Path) -> bool {
        self.plugin.is_test_file(path) && self.run_set.is_match(path)
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
    /// Resolved Python interpreter command (may be multiple tokens, e.g.
    /// `["uv", "run", "python"]`). When non-empty, replaces the interpreter
    /// that `py_subprocess_cmd` would auto-detect.
    pub python_interpreter: Vec<String>,
    /// User-declared env vars from `[env]`.
    pub env: BTreeMap<String, String>,
}

impl Package {
    /// Build a `Package` from explicit `[[suite]]` declarations. No
    /// auto-detection: the caller says exactly which tools run and
    /// where. Each suite's `root` is resolved against `pkg_root`;
    /// `run` / `watch` default to the plugin's defaults.
    pub fn from_suites(
        pkg_root: PathBuf,
        suite_configs: &[SuiteConfig],
        pytest_extra_args: &[String],
        python_interpreter: Vec<String>,
        mut resolve_hooks: impl FnMut(&dyn Plugin) -> Result<WorkerHookPaths>,
        env: BTreeMap<String, String>,
    ) -> Result<Self> {
        if suite_configs.is_empty() {
            bail!("No [[suite]] entries in config");
        }
        let mut test_suites = Vec::with_capacity(suite_configs.len());
        let mut name: Option<String> = None;

        for sc in suite_configs {
            let plugin = plugin::plugin_by_name(&sc.tool).ok_or_else(|| {
                anyhow::anyhow!("Unknown tool {:?} in [[suite]]", sc.tool)
            })?;
            let suite_root = resolve_suite_root(&pkg_root, &sc.root);
            if name.is_none() {
                name = Some(plugin.project_name(&suite_root));
            }

            let run_patterns: Vec<String> = if sc.run.is_empty() {
                default_patterns(&suite_root, &plugin.default_run())
            } else {
                anchor_patterns(&suite_root, &sc.run)
            };
            let watch_patterns: Vec<String> = if !sc.watch.is_empty() {
                anchor_patterns(&suite_root, &sc.watch)
            } else {
                let defaults = plugin.default_watch();
                if defaults.is_empty() {
                    run_patterns.clone()
                } else {
                    default_patterns(&suite_root, &defaults)
                }
            };

            let run_set = build_globset(&run_patterns)
                .with_context(|| format!("compiling run globs for [[suite]] {}", sc.tool))?;
            let watch_set = build_globset(&watch_patterns)
                .with_context(|| format!("compiling watch globs for [[suite]] {}", sc.tool))?;

            let worker_hooks = resolve_hooks(plugin.as_ref())?;
            test_suites.push(TestSuite {
                plugin,
                root: suite_root,
                run: run_patterns,
                watch: watch_patterns,
                run_set,
                watch_set,
                worker_hooks,
                runner_override: sc.runner.clone(),
            });
        }

        Ok(Package {
            name: name.unwrap_or_else(|| dir_name(&pkg_root)),
            root: pkg_root,
            test_suites,
            pytest_extra_args: pytest_extra_args.to_vec(),
            python_interpreter,
            env,
        })
    }

    /// Build a `Package` via auto-detection. Scans `pkg_root` for tool
    /// marker files and builds suites from plugin defaults, with every
    /// suite's root set to `pkg_root`. When `tool_filter` is not "auto",
    /// only that single tool is considered.
    pub fn from_auto_detect(
        pkg_root: PathBuf,
        tool_filter: &str,
        pytest_extra_args: &[String],
        python_interpreter: Vec<String>,
        mut resolve_hooks: impl FnMut(&dyn Plugin) -> Result<WorkerHookPaths>,
        mut resolve_runner: impl FnMut(&dyn Plugin) -> Option<PathBuf>,
        env: BTreeMap<String, String>,
    ) -> Result<Self> {
        let plugins = plugin::detect_plugins(&pkg_root, tool_filter)?;
        let name = plugins[0].project_name(&pkg_root);

        let mut test_suites: Vec<TestSuite> = Vec::with_capacity(plugins.len());
        for plugin in plugins {
            let suite_root = pkg_root.clone();
            let run_patterns = default_patterns(&suite_root, &plugin.default_run());
            let watch_defaults = plugin.default_watch();
            let watch_patterns = if watch_defaults.is_empty() {
                run_patterns.clone()
            } else {
                default_patterns(&suite_root, &watch_defaults)
            };

            let run_set = build_globset(&run_patterns)
                .with_context(|| format!("compiling run globs for {}", plugin.name()))?;
            let watch_set = build_globset(&watch_patterns)
                .with_context(|| format!("compiling watch globs for {}", plugin.name()))?;

            let worker_hooks = resolve_hooks(plugin.as_ref())?;
            let runner_override = resolve_runner(plugin.as_ref());
            test_suites.push(TestSuite {
                plugin,
                root: suite_root,
                run: run_patterns,
                watch: watch_patterns,
                run_set,
                watch_set,
                worker_hooks,
                runner_override,
            });
        }

        Ok(Package {
            name,
            root: pkg_root,
            test_suites,
            pytest_extra_args: pytest_extra_args.to_vec(),
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
    std::fs::canonicalize(&joined).unwrap_or(joined)
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
    let root_str = root.to_string_lossy();
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
    // (we build it that way in `anchor_pattern`), so we can scan
    // segments directly.
    let (absolute, rest) = match pattern.strip_prefix('/') {
        Some(rest) => (true, rest),
        None => (false, pattern),
    };
    let literal: Vec<&str> = rest
        .split('/')
        .take_while(|seg| !contains_glob_meta(seg))
        .collect();

    let mut cur = if absolute {
        PathBuf::from("/")
    } else if literal.is_empty() {
        // Top-level glob like `**/*.py`: walk from `.`.
        PathBuf::from(".")
    } else {
        PathBuf::new()
    };
    for seg in literal {
        if !seg.is_empty() {
            cur.push(seg);
        }
    }

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
}
