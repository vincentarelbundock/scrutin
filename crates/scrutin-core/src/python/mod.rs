//! Python-language slice of scrutin: pytest plugin, shared static analysis
//! (`imports`), and the embedded Python worker companion script.
//!
//! Adding a second Python tool (e.g. unittest) means dropping a
//! sibling directory next to `pytest/` and registering the plugin in
//! `plugins()`. Both would share `imports` for static dep-map analysis,
//! plus the `py_*` helpers below, symmetric with `r/mod.rs`.

use std::path::{Path, PathBuf};
use std::sync::Arc;

use crate::project::plugin::Plugin;

pub mod great_expectations;
pub mod imports;
pub mod pytest;
pub mod ruff;

/// Every Python plugin compiled into the binary. Called by the central
/// plugin registry in `project::plugin::all_plugins()`.
pub fn plugins() -> Vec<Arc<dyn Plugin>> {
    vec![
        Arc::new(pytest::plugin::PytestPlugin),
        Arc::new(great_expectations::plugin::GreatExpectationsPlugin),
        Arc::new(ruff::plugin::RuffPlugin),
    ]
}

// ── Shared Python plugin helpers ────────────────────────────────────────────
//
// Mirrors the layout in `r/mod.rs`. Anything any Python tool would want
// (interpreter discovery, project-name parsing, test-file walking, env vars,
// subprocess command shape) lives here so the per-tool plugin files
// stay thin.

#[cfg(windows)]
const VENV_PY_REL: [&str; 2] = ["Scripts", "python.exe"];
#[cfg(not(windows))]
const VENV_PY_REL: [&str; 2] = ["bin", "python"];

#[cfg(windows)]
const PATH_PY: &str = "python";
#[cfg(not(windows))]
const PATH_PY: &str = "python3";

/// Locate a Python interpreter to spawn workers with.
///
/// Resolution order:
///   1. Explicit `venv_override` from `[python].venv` in .scrutin/config.toml
///   2. `$VIRTUAL_ENV` environment variable
///   3. `<root>/.venv` and `<root>/venv` directories
///   4. `$CONDA_PREFIX`
///   5. `python3` (or `python` on Windows) on PATH
pub(crate) fn py_find_python(root: &Path, venv_override: Option<&Path>) -> String {
    let venv_subpath: PathBuf = VENV_PY_REL.iter().collect();

    // 1. Explicit venv from config
    if let Some(venv_dir) = venv_override {
        let abs = if venv_dir.is_relative() {
            root.join(venv_dir)
        } else {
            venv_dir.to_path_buf()
        };
        let candidate = abs.join(&venv_subpath);
        if candidate.is_file() {
            return candidate.display().to_string();
        }
    }

    // 2. $VIRTUAL_ENV
    if let Ok(venv) = std::env::var("VIRTUAL_ENV") {
        let candidate = Path::new(&venv).join(&venv_subpath);
        if candidate.is_file() {
            return candidate.display().to_string();
        }
    }

    // 3. Conventional .venv / venv directories
    for dir in &[".venv", "venv"] {
        let candidate = root.join(dir).join(&venv_subpath);
        if candidate.is_file() {
            return candidate.display().to_string();
        }
    }

    // 4. $CONDA_PREFIX
    if let Ok(conda) = std::env::var("CONDA_PREFIX") {
        let candidate = Path::new(&conda).join(&venv_subpath);
        if candidate.is_file() {
            return candidate.display().to_string();
        }
    }

    // 5. Fallback to PATH
    PATH_PY.into()
}

/// Resolve a Python project's display name: `pyproject.toml` if
/// parseable, else the directory basename. Used by every Python
/// plugin's `Plugin::project_name` so the fallback logic lives in
/// one place.
pub(crate) fn py_project_name_or_dir(root: &Path) -> String {
    py_parse_pyproject_name(root).unwrap_or_else(|| {
        root.file_name()
            .and_then(|n| n.to_str())
            .unwrap_or("<unknown>")
            .to_string()
    })
}

/// Parse the project's package name from `pyproject.toml`. Looks under
/// `[project]` first, then `[tool.poetry]`. Returns `None` if neither
/// section has a string `name` field.
pub(crate) fn py_parse_pyproject_name(root: &Path) -> Option<String> {
    let contents = std::fs::read_to_string(root.join("pyproject.toml")).ok()?;
    let value: toml::Value = toml::from_str(&contents).ok()?;
    let from_section = |section: &str| -> Option<String> {
        value
            .get(section)
            .and_then(|t| t.get("name"))
            .and_then(|n| n.as_str())
            .map(|s| s.to_string())
    };
    from_section("project").or_else(|| {
        value
            .get("tool")
            .and_then(|t| t.get("poetry"))
            .and_then(|p| p.get("name"))
            .and_then(|n| n.as_str())
            .map(|s| s.to_string())
    })
}

/// Parse the project's version string from `pyproject.toml`. Looks under
/// `[project]` first, then `[tool.poetry]`. Returns `None` if neither
/// section declares a string `version`.
pub(crate) fn py_parse_pyproject_version(root: &Path) -> Option<String> {
    let contents = std::fs::read_to_string(root.join("pyproject.toml")).ok()?;
    let value: toml::Value = toml::from_str(&contents).ok()?;
    let from_section = |section: &str| -> Option<String> {
        value
            .get(section)
            .and_then(|t| t.get("version"))
            .and_then(|v| v.as_str())
            .map(|s| s.to_string())
    };
    from_section("project").or_else(|| {
        value
            .get("tool")
            .and_then(|t| t.get("poetry"))
            .and_then(|p| p.get("version"))
            .and_then(|v| v.as_str())
            .map(|s| s.to_string())
    })
}

/// Query the installed version of a Python module via a short subprocess
/// call. Uses the same interpreter resolution as `py_subprocess_cmd`, so
/// the version reported is the one the test runners will actually import.
/// Any failure returns `None`.
pub(crate) fn py_module_version(root: &Path, module: &str) -> Option<String> {
    use std::process::Command;
    let py = py_find_python(root, None);
    let code = format!("import {m}; print({m}.__version__)", m = module);
    let out = Command::new(&py).arg("-c").arg(code).output().ok()?;
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

/// Is this a pytest-style test filename? `test_*.py` or `*_test.py`.
pub(crate) fn py_is_test_filename(name: &str) -> bool {
    name.ends_with(".py") && (name.starts_with("test_") || name.ends_with("_test.py"))
}

pub(crate) fn py_is_test_path(path: &Path) -> bool {
    path.file_name()
        .and_then(|n| n.to_str())
        .is_some_and(py_is_test_filename)
}

pub(crate) fn py_is_source_path(path: &Path) -> bool {
    path.extension().and_then(|e| e.to_str()) == Some("py")
}

/// Build the default subprocess command for a Python plugin using
/// auto-detected interpreter. The `[python].interpreter` / `[python].venv`
/// overrides are applied later in the engine (runner.rs), not here.
pub(crate) fn py_subprocess_cmd(root: &Path, runner_path: &str) -> Vec<String> {
    vec![
        py_find_python(root, None),
        "-u".into(),
        runner_path.into(),
    ]
}

/// Runner filename for a Python plugin. Prefixed with `scrutin_` so the
/// on-disk file never collides with an importable top-level module name:
/// Python prepends the script's directory to `sys.path[0]`, so a runner
/// called `pytest.py` would shadow `import pytest` from inside the runner
/// itself. Every Python plugin overrides `Plugin::runner_filename` via this
/// helper so the shadowing stays impossible by construction.
pub(crate) fn py_runner_filename(tool_name: &str) -> String {
    format!("scrutin_{tool_name}.py")
}

pub(crate) fn py_env_vars(tool: &'static str, root: &Path) -> Vec<(String, String)> {
    vec![
        ("SCRUTIN_TOOL".into(), tool.into()),
        ("SCRUTIN_PKG_DIR".into(), root.display().to_string()),
    ]
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_pyproject_project_section() {
        let tmp = tempfile::tempdir().unwrap();
        std::fs::write(
            tmp.path().join("pyproject.toml"),
            "[project]\nname = \"my_pkg\"\ndynamic = [\"version\"]\n",
        )
        .unwrap();
        assert_eq!(py_parse_pyproject_name(tmp.path()).as_deref(), Some("my_pkg"));
    }

    #[test]
    fn parse_pyproject_dynamic_name_is_not_misread() {
        // The hand-rolled parser used to greedy-match `dynamic = ["name"]`
        // and produce a garbage name. The toml-crate-based version reads
        // `name` as a real key only.
        let tmp = tempfile::tempdir().unwrap();
        std::fs::write(
            tmp.path().join("pyproject.toml"),
            "[project]\ndynamic = [\"name\", \"version\"]\n",
        )
        .unwrap();
        assert_eq!(py_parse_pyproject_name(tmp.path()), None);
    }

    #[test]
    fn parse_pyproject_tool_poetry() {
        let tmp = tempfile::tempdir().unwrap();
        std::fs::write(
            tmp.path().join("pyproject.toml"),
            "[tool.poetry]\nname = \"poetry_pkg\"\n",
        )
        .unwrap();
        assert_eq!(
            py_parse_pyproject_name(tmp.path()).as_deref(),
            Some("poetry_pkg")
        );
    }

    #[test]
    fn py_is_test_filename_basics() {
        assert!(py_is_test_filename("test_foo.py"));
        assert!(py_is_test_filename("foo_test.py"));
        assert!(!py_is_test_filename("test_data"));
        assert!(!py_is_test_filename("foo.py"));
    }
}
