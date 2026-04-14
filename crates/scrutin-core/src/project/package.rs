use anyhow::{Context, Result, bail};
use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::sync::Arc;

use crate::project::config::SuiteConfig;
use crate::project::plugin::{self, Plugin};

/// Per-suite worker hook script paths, resolved by the caller against the
/// project root before a `TestSuite` is built.
#[derive(Debug, Clone, Default)]
pub struct WorkerHookPaths {
    pub startup: Option<PathBuf>,
    pub teardown: Option<PathBuf>,
}

/// One tool's slice of a project: which plugin runs it, which test
/// directories hold its files. A `Package` can hold multiple suites,
/// supporting more than one tool or language in a single project root.
#[derive(Clone)]
pub struct TestSuite {
    pub plugin: Arc<dyn Plugin>,
    /// Resolved test directories for this suite.
    pub test_dirs: Vec<PathBuf>,
    /// Source directories for this suite (relative paths, resolved at
    /// query time against `Package::root`).
    pub source_dir_names: Vec<String>,
    /// Worker hooks scoped to this suite, resolved from
    /// `[hooks.<lang>.<tool>]` (with `[hooks.<lang>]` fallback).
    pub worker_hooks: WorkerHookPaths,
    /// Optional path to a user-provided runner script. When `Some`, the
    /// engine reads this file instead of the embedded default. Set from
    /// `[[suite]].runner` or `[<tool>].runner` in .scrutin/config.toml.
    pub runner_override: Option<PathBuf>,
}

impl TestSuite {
    /// Does this suite claim `path` as one of its test files? A suite owns
    /// a path iff the path lives under one of its `test_dirs` AND its
    /// plugin's `is_test_file` predicate accepts it.
    pub fn owns_test_file(&self, path: &Path) -> bool {
        if !self.plugin.is_test_file(path) {
            return false;
        }
        self.test_dirs.iter().any(|d| path.starts_with(d))
    }
}

#[derive(Clone)]
pub struct Package {
    pub name: String,
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
    /// where their files live.
    pub fn from_suites(
        root: PathBuf,
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
            if name.is_none() {
                name = Some(plugin.project_name(&root));
            }
            let test_dirs: Vec<PathBuf> = sc.test_dirs.iter().map(|d| root.join(d)).collect();
            let worker_hooks = resolve_hooks(plugin.as_ref())?;
            test_suites.push(TestSuite {
                plugin,
                test_dirs,
                source_dir_names: sc.source_dirs.clone(),
                worker_hooks,
                runner_override: sc.runner.clone(),
            });
        }

        Ok(Package {
            name: name.unwrap_or_else(|| dir_name(&root)),
            root,
            test_suites,
            pytest_extra_args: pytest_extra_args.to_vec(),
            python_interpreter,
            env,
        })
    }

    /// Build a `Package` via auto-detection. Scans `root` for tool
    /// marker files and builds suites from plugin defaults. When
    /// `tool_filter` is not "auto", only that single tool is
    /// considered.
    pub fn from_auto_detect(
        root: PathBuf,
        tool_filter: &str,
        pytest_extra_args: &[String],
        python_interpreter: Vec<String>,
        mut resolve_hooks: impl FnMut(&dyn Plugin) -> Result<WorkerHookPaths>,
        mut resolve_runner: impl FnMut(&dyn Plugin) -> Option<PathBuf>,
        env: BTreeMap<String, String>,
    ) -> Result<Self> {
        let plugins = plugin::detect_plugins(&root, tool_filter)?;
        let name = plugins[0].project_name(&root);

        let mut test_suites: Vec<TestSuite> = Vec::with_capacity(plugins.len());
        for plugin in plugins {
            let test_dirs = resolve_dirs(&root, &plugin.test_dirs());
            let source_dir_names: Vec<String> =
                plugin.source_dirs().iter().map(|s| s.to_string()).collect();
            let worker_hooks = resolve_hooks(plugin.as_ref())?;
            let runner_override = resolve_runner(plugin.as_ref());
            test_suites.push(TestSuite {
                plugin,
                test_dirs,
                source_dir_names,
                worker_hooks,
                runner_override,
            });
        }

        Ok(Package {
            name,
            root,
            test_suites,
            pytest_extra_args: pytest_extra_args.to_vec(),
            python_interpreter,
            env,
        })
    }

    /// Resolved source dirs across every active suite, deduplicated. Only
    /// directories that exist on disk are returned.
    pub fn resolved_source_dirs(&self) -> Vec<PathBuf> {
        self.resolved_source_dirs_for_language(None)
    }

    /// Like `resolved_source_dirs`, but if `language` is `Some(lang)` only
    /// suites whose plugin reports that language are walked. Used by per-
    /// language analyzers (e.g. `r::depmap`) so they don't pick up unrelated
    /// directories from co-resident suites in other languages.
    pub fn resolved_source_dirs_for_language(&self, language: Option<&str>) -> Vec<PathBuf> {
        let mut out: Vec<PathBuf> = Vec::new();
        let mut seen: std::collections::HashSet<PathBuf> = std::collections::HashSet::new();
        for suite in &self.test_suites {
            if let Some(lang) = language
                && suite.plugin.language() != lang
            {
                continue;
            }
            for c in &suite.source_dir_names {
                let p = self.root.join(c);
                if p.is_dir() && seen.insert(p.clone()) {
                    out.push(p);
                }
            }
        }
        out
    }

    /// Resolved test dirs across every active suite, deduplicated.
    pub fn resolved_test_dirs(&self) -> Vec<PathBuf> {
        let mut out: Vec<PathBuf> = Vec::new();
        for suite in &self.test_suites {
            for td in &suite.test_dirs {
                if td.is_dir() && !out.contains(td) {
                    out.push(td.clone());
                }
            }
        }
        out
    }

    /// Discover every test file across every active suite (sorted, deduped).
    pub fn test_files(&self) -> Result<Vec<PathBuf>> {
        let mut out = Vec::new();
        for suite in &self.test_suites {
            for td in &suite.test_dirs {
                let files = suite
                    .plugin
                    .discover_test_files(&self.root, td)
                    .with_context(|| {
                        format!(
                            "discovering test files for {} in {}",
                            suite.plugin.name(),
                            td.display()
                        )
                    })?;
                out.extend(files);
            }
        }
        out.sort();
        out.dedup();
        Ok(out)
    }

    /// Find which suite owns a given test file. The first matching suite
    /// wins; suites are checked in registration order (tinytest, testthat,
    /// pytest). Because each plugin's predicate keys on extension/prefix,
    /// the same file is never claimed by two suites in practice.
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

    /// Does any active suite recognize `path` as a source file?
    pub fn is_any_source_file(&self, path: &Path) -> bool {
        self.test_suites
            .iter()
            .any(|s| s.plugin.is_source_file(path))
    }
}

/// Resolve directory candidates against `root`. Returns all candidates
/// that exist on disk. If none exist, returns the first candidate joined
/// to root (so the path is still useful for error messages and watcher
/// initialization).
fn resolve_dirs(root: &Path, candidates: &[&str]) -> Vec<PathBuf> {
    let existing: Vec<PathBuf> = candidates
        .iter()
        .map(|c| root.join(c))
        .filter(|p| p.is_dir())
        .collect();
    if existing.is_empty() && !candidates.is_empty() {
        vec![root.join(candidates[0])]
    } else {
        existing
    }
}

fn dir_name(root: &Path) -> String {
    root.file_name()
        .and_then(|n| n.to_str())
        .unwrap_or("<unknown>")
        .to_string()
}
