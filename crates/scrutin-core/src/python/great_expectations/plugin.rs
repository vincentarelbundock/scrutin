//! great_expectations `Plugin` impl.
//!
//! GE test files live under `tests/great_expectations/test_*.py`. Shares
//! the Python helpers in `python/mod.rs` with pytest; differs in detect
//! marker (the test directory itself, no pyproject requirement), test
//! directory, runner basename, `SCRUTIN_TOOL` env var, and the
//! outcome vocabulary it can emit (`xfail` via `meta.expected_to_fail`,
//! no `skip` or `warn`).

use std::path::{Path, PathBuf};

use anyhow::Result;

use crate::engine::protocol::Outcome;
use crate::project::plugin::Plugin;
use crate::python::{
    py_env_vars, py_is_source_path, py_is_test_path, py_module_version, py_parse_pyproject_name,
    py_parse_pyproject_version, py_subprocess_cmd, py_walk_tests,
};

const GE_RUNNER: &str = include_str!("runner.py");

pub struct GreatExpectationsPlugin;

impl Plugin for GreatExpectationsPlugin {
    fn name(&self) -> &'static str {
        "great_expectations"
    }
    fn language(&self) -> &'static str {
        "python"
    }
    fn detect(&self, root: &Path) -> bool {
        // Mirrors pointblank's detect: presence of the conventional test
        // directory is sufficient. We don't require pyproject.toml because
        // GE projects often live standalone.
        root.join(super::TEST_DIR).is_dir()
    }
    fn subprocess_cmd(&self, root: &Path) -> Vec<String> {
        py_subprocess_cmd(root, &self.runner_basename())
    }
    fn runner_script(&self) -> &'static str {
        GE_RUNNER
    }
    fn script_extension(&self) -> &'static str {
        "py"
    }
    fn runner_basename(&self) -> String {
        "runner_great_expectations.py".into()
    }
    fn project_name(&self, root: &Path) -> String {
        py_parse_pyproject_name(root).unwrap_or_else(|| {
            root.file_name()
                .and_then(|n| n.to_str())
                .unwrap_or("<unknown>")
                .to_string()
        })
    }
    fn project_version(&self, root: &Path) -> Option<String> {
        py_parse_pyproject_version(root)
    }
    fn tool_version(&self, root: &Path) -> Option<String> {
        py_module_version(root, "great_expectations")
    }
    fn source_dirs(&self) -> Vec<&'static str> {
        vec!["src", "lib"]
    }
    fn test_dirs(&self) -> Vec<&'static str> {
        vec![super::TEST_DIR]
    }
    fn discover_test_files(&self, _root: &Path, test_dir: &Path) -> Result<Vec<PathBuf>> {
        Ok(py_walk_tests(test_dir))
    }
    fn is_test_file(&self, path: &Path) -> bool {
        py_is_test_path(path)
    }
    fn is_source_file(&self, path: &Path) -> bool {
        py_is_source_path(path)
    }
    fn test_file_candidates(&self, stem: &str) -> Vec<String> {
        vec![format!("test_{stem}.py"), format!("{stem}_test.py")]
    }
    fn env_vars(&self, root: &Path) -> Vec<(String, String)> {
        py_env_vars("great_expectations", root)
    }
    fn supported_outcomes(&self) -> &'static [Outcome] {
        // No `skip` or `warn`. `xfail` is supported via
        // `meta.expected_to_fail` on the expectation config.
        &[Outcome::Pass, Outcome::Fail, Outcome::Error, Outcome::Xfail]
    }
    fn subject_label(&self) -> &'static str {
        "expectation"
    }
}
