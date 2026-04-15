//! Pytest plugin: any project with pyproject.toml / setup.py / setup.cfg
//! and a `tests/`, `test/`, or top-level `test_*.py`.

use std::path::Path;

use crate::project::plugin::Plugin;
use crate::python::{
    py_env_vars, py_is_source_path, py_is_test_filename, py_is_test_path, py_module_version,
    py_parse_pyproject_name, py_parse_pyproject_version, py_project_name_or_dir, py_subprocess_cmd,
};

const PYTEST_RUNNER: &str = include_str!("runner.py");

pub struct PytestPlugin;

impl Plugin for PytestPlugin {
    fn name(&self) -> &'static str {
        "pytest"
    }
    fn language(&self) -> &'static str {
        "python"
    }
    fn detect(&self, root: &Path) -> bool {
        let has_marker = root.join("pyproject.toml").is_file()
            || root.join("setup.py").is_file()
            || root.join("setup.cfg").is_file();
        if !has_marker {
            return false;
        }
        if root.join("tests").is_dir() || root.join("test").is_dir() {
            return true;
        }
        if let Ok(entries) = std::fs::read_dir(root) {
            for entry in entries.flatten() {
                if let Some(name) = entry.file_name().to_str()
                    && py_is_test_filename(name)
                {
                    return true;
                }
            }
        }
        false
    }
    fn subprocess_cmd(&self, root: &Path) -> Vec<String> {
        py_subprocess_cmd(root, &self.runner_basename())
    }
    fn runner_script(&self) -> &'static str {
        PYTEST_RUNNER
    }
    fn script_extension(&self) -> &'static str {
        "py"
    }
    fn runner_basename(&self) -> String {
        "runner_pytest.py".into()
    }
    fn project_name(&self, root: &Path) -> String {
        py_project_name_or_dir(root)
    }
    fn project_module_name(&self, root: &Path) -> Option<String> {
        // Mirror runner.py::_warm_up: project name with `-` → `_`.
        py_parse_pyproject_name(root).map(|n| n.replace('-', "_"))
    }
    fn project_version(&self, root: &Path) -> Option<String> {
        py_parse_pyproject_version(root)
    }
    fn tool_version(&self, root: &Path) -> Option<String> {
        py_module_version(root, "pytest")
    }
    fn default_run(&self) -> Vec<String> {
        vec![
            format!("{}/**/test_*.py", super::TEST_DIR),
            format!("{}/**/*_test.py", super::TEST_DIR),
            "test/**/test_*.py".into(),
            "test/**/*_test.py".into(),
            "test_*.py".into(),
            "*_test.py".into(),
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
        py_env_vars("pytest", root)
    }
}
