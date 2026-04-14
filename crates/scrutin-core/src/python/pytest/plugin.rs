//! Pytest plugin: any project with pyproject.toml / setup.py / setup.cfg
//! and a `tests/`, `test/`, or top-level `test_*.py`.

use std::path::{Path, PathBuf};

use anyhow::Result;

use crate::project::plugin::Plugin;
use crate::python::{
    py_env_vars, py_is_source_path, py_is_test_filename, py_is_test_path, py_module_version,
    py_parse_pyproject_name, py_parse_pyproject_version, py_subprocess_cmd, py_walk_tests,
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
        py_module_version(root, "pytest")
    }
    fn source_dirs(&self) -> Vec<&'static str> {
        vec!["src", "lib"]
    }
    fn test_dirs(&self) -> Vec<&'static str> {
        vec![super::TEST_DIR, "test"]
    }
    fn discover_test_files(&self, root: &Path, test_dir: &Path) -> Result<Vec<PathBuf>> {
        let mut out = py_walk_tests(test_dir);
        // Also pick up top-level test_*.py at the project root, regardless
        // of which test_dir was chosen — pytest's own discovery does the
        // same thing.
        if let Ok(entries) = std::fs::read_dir(root) {
            for entry in entries.flatten() {
                let path = entry.path();
                if path.is_file() && py_is_test_path(&path) {
                    out.push(path);
                }
            }
        }
        out.sort();
        out.dedup();
        Ok(out)
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
