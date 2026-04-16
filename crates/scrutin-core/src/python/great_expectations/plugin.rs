//! great_expectations `Plugin` impl.
//!
//! GE test files live under `tests/great_expectations/test_*.py`. Shares
//! the Python helpers in `python/mod.rs` with pytest; differs in detect
//! marker (the test directory itself, no pyproject requirement), test
//! directory, runner basename, `SCRUTIN_TOOL` env var, and the
//! outcome vocabulary it can emit (`xfail` via `meta.expected_to_fail`,
//! no `skip` or `warn`).

use std::path::Path;

use crate::engine::protocol::Outcome;
use crate::project::plugin::Plugin;
use crate::python::{
    py_env_vars, py_is_source_path, py_is_test_path, py_module_version, py_parse_pyproject_name,
    py_parse_pyproject_version, py_project_name_or_dir, py_subprocess_cmd,
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
    fn subprocess_cmd(&self, root: &Path, runner_path: &str) -> Vec<String> {
        py_subprocess_cmd(root, runner_path)
    }
    fn runner_script(&self) -> &'static str {
        GE_RUNNER
    }
    fn script_extension(&self) -> &'static str {
        "py"
    }
    fn project_name(&self, root: &Path) -> String {
        py_project_name_or_dir(root)
    }
    fn project_module_name(&self, root: &Path) -> Option<String> {
        py_parse_pyproject_name(root).map(|n| n.replace('-', "_"))
    }
    fn project_version(&self, root: &Path) -> Option<String> {
        py_parse_pyproject_version(root)
    }
    fn tool_version(&self, root: &Path) -> Option<String> {
        py_module_version(root, "great_expectations")
    }
    fn default_run(&self) -> Vec<String> {
        vec![
            format!("{}/**/test_*.py", super::TEST_DIR),
            format!("{}/**/*_test.py", super::TEST_DIR),
        ]
    }
    fn default_watch(&self) -> Vec<String> {
        vec!["src/**/*.py".into(), "lib/**/*.py".into(), "**/*.py".into()]
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
