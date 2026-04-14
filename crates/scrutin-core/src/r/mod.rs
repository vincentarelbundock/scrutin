//! R-language slice of scrutin: tool plugins (testthat, tinytest,
//! pointblank, validate), the jarl linter plugin, the dep-map cache loader
//! (`depmap`), and the embedded R worker companion script.
//!
//! The four test/validation tool plugins are instantiated via the
//! data-driven [`RPlugin`] struct. jarl is structurally different (linter,
//! command-mode, custom actions) and keeps its own `Plugin` impl.
//!
//! Dependency mapping is handled by runtime instrumentation in the R runner
//! scripts (`trace()` on package functions). The `depmap` module loads the
//! cached map; it is populated incrementally by the engine as tests run.

use std::path::{Path, PathBuf};
use std::sync::Arc;

use anyhow::Result;

use crate::analysis::walk;
use crate::engine::protocol::Outcome;
use crate::project::package::{Package, TestSuite};
use crate::project::plugin::Plugin;

pub mod depmap;
pub mod jarl;

/// Shared R runner infrastructure sourced by each per-plugin runner.
/// Written to `.scrutin/runner_r.R` alongside the per-plugin script.
pub const R_RUNNER_SHARED: &str = include_str!("runner_r.R");

/// Every R plugin compiled into the binary. Called by the central plugin
/// registry in `project::plugin::all_plugins()`.
pub fn plugins() -> Vec<Arc<dyn Plugin>> {
    vec![
        Arc::new(RPlugin {
            name: "tinytest",
            detect_dir: "inst/tinytest",
            test_dir: "inst/tinytest",
            runner_script: include_str!("runner_tinytest.R"),
            supported_outcomes: &[Outcome::Pass, Outcome::Fail, Outcome::Error, Outcome::Skip],
            subject_label: "test",
        }),
        Arc::new(RPlugin {
            name: "testthat",
            detect_dir: "tests/testthat",
            test_dir: "tests/testthat",
            runner_script: include_str!("runner_testthat.R"),
            supported_outcomes: &[Outcome::Pass, Outcome::Fail, Outcome::Error, Outcome::Skip],
            subject_label: "test",
        }),
        Arc::new(RPlugin {
            name: "pointblank",
            detect_dir: "tests/pointblank",
            test_dir: "tests/pointblank",
            runner_script: include_str!("runner_pointblank.R"),
            supported_outcomes: &[Outcome::Pass, Outcome::Fail, Outcome::Error, Outcome::Warn],
            subject_label: "step",
        }),
        Arc::new(jarl::plugin::JarlPlugin),
        Arc::new(RPlugin {
            name: "validate",
            detect_dir: "tests/validate",
            test_dir: "tests/validate",
            runner_script: include_str!("runner_validate.R"),
            supported_outcomes: &[Outcome::Pass, Outcome::Fail, Outcome::Error, Outcome::Warn],
            subject_label: "rule",
        }),
    ]
}

/// Iterate the R-flavored suites of a `Package`. Plugins report their
/// language via `Plugin::language()`; "r" identifies all R plugins.
/// Used by `r::depmap` and `analysis::hashing` so they can hash
/// and dep-map across every R suite at once.
pub fn r_suites(pkg: &Package) -> impl Iterator<Item = &TestSuite> {
    pkg.test_suites
        .iter()
        .filter(|s| s.plugin.language() == "r")
}

// ── Data-driven R plugin ──────────────────────────────────────────────────
//
// testthat, tinytest, pointblank, and validate differ only in their
// detect/test directory, runner script, supported outcomes, and subject
// label. Everything else (package-name parsing, file predicates,
// subprocess command shape, env vars, noise filtering) is identical.

/// A data-driven R plugin. Covers testthat, tinytest, pointblank, and
/// validate. jarl is structurally different and has its own impl.
pub struct RPlugin {
    pub name: &'static str,
    /// Directory whose existence (alongside DESCRIPTION) triggers detection.
    pub detect_dir: &'static str,
    pub test_dir: &'static str,
    pub runner_script: &'static str,
    pub supported_outcomes: &'static [Outcome],
    pub subject_label: &'static str,
}

impl Plugin for RPlugin {
    fn name(&self) -> &'static str {
        self.name
    }
    fn language(&self) -> &'static str {
        "r"
    }
    fn detect(&self, root: &Path) -> bool {
        root.join("DESCRIPTION").is_file() && root.join(self.detect_dir).is_dir()
    }
    fn subprocess_cmd(&self, _root: &Path) -> Vec<String> {
        r_subprocess_cmd(&self.runner_basename())
    }
    fn runner_script(&self) -> &'static str {
        self.runner_script
    }
    fn script_extension(&self) -> &'static str {
        "R"
    }
    fn runner_basename(&self) -> String {
        format!("runner_{}.R", self.name)
    }
    fn project_name(&self, root: &Path) -> String {
        parse_r_package_name(root)
    }
    fn project_version(&self, root: &Path) -> Option<String> {
        parse_r_package_version(root)
    }
    fn tool_version(&self, root: &Path) -> Option<String> {
        r_package_version(root, self.name)
    }
    fn source_dirs(&self) -> Vec<&'static str> {
        vec!["R"]
    }
    fn test_dirs(&self) -> Vec<&'static str> {
        vec![self.test_dir]
    }
    fn discover_test_files(&self, _root: &Path, test_dir: &Path) -> Result<Vec<PathBuf>> {
        list_r_test_files(test_dir)
    }
    fn is_test_file(&self, path: &Path) -> bool {
        is_r_test_path(path)
    }
    fn is_source_file(&self, path: &Path) -> bool {
        is_r_source_path(path)
    }
    fn test_file_candidates(&self, stem: &str) -> Vec<String> {
        vec![
            format!("test-{stem}.R"),
            format!("test_{stem}.R"),
            format!("test-{stem}.r"),
        ]
    }
    fn env_vars(&self, root: &Path) -> Vec<(String, String)> {
        r_env_vars(self.name, root)
    }
    fn is_noise_line(&self, line: &str) -> bool {
        r_is_noise_line(line)
    }
    fn supported_outcomes(&self) -> &'static [Outcome] {
        self.supported_outcomes
    }
    fn subject_label(&self) -> &'static str {
        self.subject_label
    }
}

// ── Shared R plugin helpers ───────────────────────────────────────────────

pub(crate) fn parse_r_package_name(root: &Path) -> String {
    let desc = root.join("DESCRIPTION");
    if let Ok(contents) = std::fs::read_to_string(&desc) {
        for line in contents.lines() {
            if let Some(rest) = line.strip_prefix("Package:") {
                return rest.trim().to_string();
            }
        }
    }
    root.file_name()
        .and_then(|n| n.to_str())
        .unwrap_or("<unknown>")
        .to_string()
}

/// Parse the `Version:` field from a package's DESCRIPTION file.
/// Returns `None` when DESCRIPTION is missing or lacks a Version line.
pub(crate) fn parse_r_package_version(root: &Path) -> Option<String> {
    let contents = std::fs::read_to_string(root.join("DESCRIPTION")).ok()?;
    for line in contents.lines() {
        if let Some(rest) = line.strip_prefix("Version:") {
            let v = rest.trim();
            if !v.is_empty() {
                return Some(v.to_string());
            }
        }
    }
    None
}

/// Query the installed version of an R package by shelling out to R. Uses
/// the same env (R_LIBS_USER etc.) as the test runners so we hit whichever
/// library the tests will actually use. Any error (R not installed, package
/// not installed, non-UTF8 output) returns `None`: tool version is metadata,
/// not a correctness dependency.
pub(crate) fn r_package_version(root: &Path, pkg_name: &str) -> Option<String> {
    use std::process::Command;
    let expr = format!("cat(as.character(packageVersion(\"{}\")))", pkg_name);
    let mut cmd = Command::new("R");
    cmd.arg("--no-save")
        .arg("--no-restore")
        .arg("-s")
        .arg("-e")
        .arg(&expr);
    for (k, v) in r_env_vars(pkg_name, root) {
        cmd.env(k, v);
    }
    let out = cmd.output().ok()?;
    if !out.status.success() {
        return None;
    }
    let s = String::from_utf8(out.stdout).ok()?;
    let trimmed = s.trim();
    if trimmed.is_empty() {
        None
    } else {
        Some(trimmed.to_string())
    }
}

pub(crate) fn is_r_test_filename(name: &str) -> bool {
    // Extension check is case-insensitive (`.R` and `.r` both legal); the
    // prefix check is case-sensitive because every R tool convention
    // uses lowercase `test-` / `test_`.
    let has_r_ext = name.ends_with(".R") || name.ends_with(".r");
    has_r_ext && (name.starts_with("test-") || name.starts_with("test_"))
}

pub(crate) fn is_r_test_path(path: &std::path::Path) -> bool {
    path.file_name()
        .and_then(|n| n.to_str())
        .is_some_and(is_r_test_filename)
}

pub(crate) fn is_r_source_path(path: &std::path::Path) -> bool {
    walk::has_extension(path, &["r"])
}

/// R startup chatter that isn't useful in the log pane: S4 generic masking
/// notices and the "Creating a generic function" / "in method for" / "Found
/// more than one class" follow-ups. All R plugins share this filter via
/// `Plugin::is_noise_line`.
const R_NOISE_PREFIXES: &[&str] = &[
    "Creating a generic function",
    "Creating a new generic function",
    "in method for",
    "Found more than one class",
];

pub(crate) fn r_is_noise_line(line: &str) -> bool {
    let t = line.trim_start();
    R_NOISE_PREFIXES.iter().any(|p| t.starts_with(p))
}

pub(crate) fn r_env_vars(tool: &str, root: &std::path::Path) -> Vec<(String, String)> {
    vec![
        ("SCRUTIN_TOOL".into(), tool.into()),
        ("SCRUTIN_PKG_DIR".into(), root.to_string_lossy().into_owned()),
    ]
}

pub(crate) fn r_subprocess_cmd(runner_basename: &str) -> Vec<String> {
    vec![
        "Rscript".into(),
        "--vanilla".into(),
        format!(".scrutin/{runner_basename}"),
    ]
}

/// List R test files under `dir`, recursively. Routes through the shared
/// walker so the same ignore list (`.git`, `__pycache__`, ...) applies, and
/// nested helper directories under `inst/tinytest/` are picked up.
pub(crate) fn list_r_test_files(dir: &std::path::Path) -> anyhow::Result<Vec<std::path::PathBuf>> {
    if !dir.is_dir() {
        return Ok(Vec::new());
    }
    Ok(walk::collect_files(dir, |p| is_r_test_path(p)))
}
