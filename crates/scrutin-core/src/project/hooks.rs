//! Process- and worker-level hook scripts.
//!
//! Process hooks (`startup` / `teardown`) run from the Rust binary once per
//! scrutin invocation. Worker hooks are resolved here but actually executed
//! by the runner subprocesses (R / pytest) on boot / shutdown via the
//! `SCRUTIN_WORKER_STARTUP` / `SCRUTIN_WORKER_TEARDOWN` env vars.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::process::Command;

use anyhow::{Context, Result, bail};

use crate::project::config::Config;
use crate::project::plugin::Plugin;

/// Process-level hooks, resolved against a concrete project root.
#[derive(Debug, Clone)]
pub struct ProcessHooks {
    project_root: PathBuf,
    startup: Option<PathBuf>,
    teardown: Option<PathBuf>,
    /// `[env]` from .scrutin/config.toml. Applied to both startup and teardown
    /// scripts so they see the same env as test workers — consistency
    /// across every subprocess that runs user code.
    env: BTreeMap<String, String>,
}

impl ProcessHooks {
    pub fn from_config(cfg: &Config, project_root: &Path) -> Self {
        Self {
            project_root: project_root.to_path_buf(),
            startup: cfg.hooks.startup.clone(),
            teardown: cfg.hooks.teardown.clone(),
            env: cfg.env.clone(),
        }
    }

    /// Run the startup script if configured. Errors abort the run.
    pub fn run_startup(&self) -> Result<()> {
        let Some(rel) = self.startup.as_ref() else {
            return Ok(());
        };
        let abs = absolute_under(&self.project_root, rel);
        validate_script(&abs, "startup")?;
        let status = Command::new(&abs)
            .current_dir(&self.project_root)
            .envs(self.env.iter())
            .status()
            .with_context(|| format!("failed to spawn startup hook {}", abs.display()))?;
        if !status.success() {
            let code = status.code().unwrap_or(1);
            bail!(
                "startup hook {} exited with status {}",
                abs.display(),
                code
            );
        }
        Ok(())
    }

    /// Run the teardown script if configured. Logs warnings on failure;
    /// never errors — teardown must not mask the test exit code.
    pub fn run_teardown(&self) {
        let Some(rel) = self.teardown.as_ref() else {
            return;
        };
        let abs = absolute_under(&self.project_root, rel);
        if let Err(e) = validate_script(&abs, "teardown") {
            eprintln!("warning: {}", e);
            return;
        }
        match Command::new(&abs)
            .current_dir(&self.project_root)
            .envs(self.env.iter())
            .status()
        {
            Ok(status) if status.success() => {}
            Ok(status) => eprintln!(
                "warning: teardown hook {} exited with status {}",
                abs.display(),
                status.code().unwrap_or(-1)
            ),
            Err(e) => eprintln!(
                "warning: failed to spawn teardown hook {}: {}",
                abs.display(),
                e
            ),
        }
    }
}

/// Resolved absolute paths of the worker_startup / worker_teardown scripts
/// for a given language+tool pair.
#[derive(Debug, Clone, Default)]
pub struct WorkerHooks {
    pub startup: Option<PathBuf>,
    pub teardown: Option<PathBuf>,
}

/// Resolve worker hooks for a given plugin using per-field override
/// semantics: tool-level entries override language-level entries on a
/// per-field basis.
pub fn resolve_worker_hooks(
    cfg: &Config,
    plugin: &dyn Plugin,
    project_root: &Path,
) -> Result<WorkerHooks> {
    let lang = plugin.language();
    let tool = plugin.name();
    let lang_hooks = cfg.hooks.by_language.get(lang);
    let tool_hooks = lang_hooks.and_then(|lh| lh.by_tool.get(tool));

    let startup_rel = tool_hooks
        .and_then(|f| f.worker_startup.clone())
        .or_else(|| lang_hooks.and_then(|l| l.worker_startup.clone()));
    let teardown_rel = tool_hooks
        .and_then(|f| f.worker_teardown.clone())
        .or_else(|| lang_hooks.and_then(|l| l.worker_teardown.clone()));

    let startup = match startup_rel {
        Some(rel) => {
            let abs = absolute_under(project_root, &rel);
            validate_source_file(&abs, "worker_startup")?;
            Some(abs)
        }
        None => None,
    };
    let teardown = match teardown_rel {
        Some(rel) => {
            let abs = absolute_under(project_root, &rel);
            validate_source_file(&abs, "worker_teardown")?;
            Some(abs)
        }
        None => None,
    };

    Ok(WorkerHooks { startup, teardown })
}

fn absolute_under(root: &Path, rel: &Path) -> PathBuf {
    if rel.is_absolute() {
        rel.to_path_buf()
    } else {
        root.join(rel)
    }
}

/// Worker hooks are sourced into the interpreter, not exec'd, so only
/// existence + file-ness matter. No executable-bit check.
fn validate_source_file(path: &Path, label: &str) -> Result<()> {
    if !path.exists() {
        bail!("hook script not found: {} ({})", path.display(), label);
    }
    if !path.is_file() {
        bail!(
            "hook script is not a regular file: {} ({})",
            path.display(),
            label
        );
    }
    Ok(())
}

fn validate_script(path: &Path, label: &str) -> Result<()> {
    if !path.exists() {
        bail!("hook script not found: {} ({})", path.display(), label);
    }
    if !path.is_file() {
        bail!(
            "hook script is not a regular file: {} ({})",
            path.display(),
            label
        );
    }
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let meta = std::fs::metadata(path)
            .with_context(|| format!("stat hook script {}", path.display()))?;
        let mode = meta.permissions().mode();
        if mode & 0o111 == 0 {
            bail!(
                "hook script not executable: {} ({}), run: chmod +x {}",
                path.display(),
                label,
                path.display()
            );
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    // Helper: write a script file to `root/rel`, mark it executable, return
    // the relative path suitable for `ProcessHooks::from_config`.
    #[cfg(unix)]
    fn write_script(root: &Path, rel: &str, body: &str) -> PathBuf {
        use std::os::unix::fs::PermissionsExt;
        let abs = root.join(rel);
        std::fs::create_dir_all(abs.parent().unwrap()).unwrap();
        std::fs::write(&abs, body).unwrap();
        let mut perms = std::fs::metadata(&abs).unwrap().permissions();
        perms.set_mode(0o755);
        std::fs::set_permissions(&abs, perms).unwrap();
        PathBuf::from(rel)
    }

    fn process_hooks(
        root: &Path,
        startup: Option<PathBuf>,
        teardown: Option<PathBuf>,
    ) -> ProcessHooks {
        ProcessHooks {
            project_root: root.to_path_buf(),
            startup,
            teardown,
            env: BTreeMap::new(),
        }
    }

    // ── Process hook execution (spec §3.13) ────────────────────────────────

    #[cfg(unix)]
    #[test]
    fn startup_hook_runs_successfully_when_exit_zero() {
        let dir = tempfile::tempdir().unwrap();
        let rel = write_script(dir.path(), "scripts/ok.sh", "#!/bin/sh\nexit 0\n");
        let h = process_hooks(dir.path(), Some(rel), None);
        assert!(h.run_startup().is_ok());
    }

    #[cfg(unix)]
    #[test]
    fn startup_hook_aborts_run_on_non_zero_exit() {
        let dir = tempfile::tempdir().unwrap();
        let rel = write_script(dir.path(), "scripts/fail.sh", "#!/bin/sh\nexit 7\n");
        let h = process_hooks(dir.path(), Some(rel), None);
        let err = h.run_startup().unwrap_err();
        let s = format!("{err}");
        assert!(
            s.contains("exited with status 7"),
            "error must surface the exit code so users can diagnose: {s}"
        );
    }

    #[test]
    fn startup_hook_errors_when_script_missing() {
        let dir = tempfile::tempdir().unwrap();
        let h = process_hooks(
            dir.path(),
            Some(PathBuf::from("scripts/nonexistent.sh")),
            None,
        );
        let err = h.run_startup().unwrap_err();
        let s = format!("{err}");
        assert!(
            s.contains("not found"),
            "missing script error must say 'not found'; got: {s}"
        );
    }

    #[cfg(unix)]
    #[test]
    fn startup_hook_errors_when_not_executable() {
        // On unix, validate_script checks the executable bit (`chmod +x`).
        // This protects against the common mistake of forgetting to mark
        // a script executable and getting a confusing spawn error.
        let dir = tempfile::tempdir().unwrap();
        let abs = dir.path().join("scripts/plain.sh");
        std::fs::create_dir_all(abs.parent().unwrap()).unwrap();
        std::fs::write(&abs, "#!/bin/sh\nexit 0\n").unwrap();
        // Deliberately NOT chmod +x: permissions are 0644 by default.

        let h = process_hooks(
            dir.path(),
            Some(PathBuf::from("scripts/plain.sh")),
            None,
        );
        let err = h.run_startup().unwrap_err();
        let s = format!("{err}");
        assert!(
            s.contains("not executable") && s.contains("chmod +x"),
            "non-executable script error must mention chmod +x: {s}"
        );
    }

    #[test]
    fn startup_hook_is_noop_when_unconfigured() {
        // No startup script configured: run_startup is a no-op, not an error.
        // This is the default shape for most projects.
        let dir = tempfile::tempdir().unwrap();
        let h = process_hooks(dir.path(), None, None);
        assert!(h.run_startup().is_ok());
    }

    #[cfg(unix)]
    #[test]
    fn teardown_hook_never_errors_on_failure() {
        // Spec §3.13: post-run always runs, even on failure, and must NOT
        // mask the test exit code. Even a teardown script that exits
        // non-zero is only a warning on stderr; the function returns ().
        let dir = tempfile::tempdir().unwrap();
        let rel = write_script(dir.path(), "scripts/td_fail.sh", "#!/bin/sh\nexit 9\n");
        let h = process_hooks(dir.path(), None, Some(rel));
        // Compiles: return type is (), not Result. Explicit about the contract.
        h.run_teardown();
    }

    #[test]
    fn teardown_hook_is_noop_when_unconfigured() {
        let dir = tempfile::tempdir().unwrap();
        let h = process_hooks(dir.path(), None, None);
        h.run_teardown();
    }

    #[test]
    fn teardown_hook_missing_file_warns_does_not_panic() {
        let dir = tempfile::tempdir().unwrap();
        let h = process_hooks(
            dir.path(),
            None,
            Some(PathBuf::from("scripts/nonexistent.sh")),
        );
        h.run_teardown(); // must not panic or unwrap-fail
    }

    // ── resolve_worker_hooks: tool-over-language override ──────────────────

    #[cfg(unix)]
    #[test]
    fn worker_hooks_tool_level_overrides_language_level_per_field() {
        // [hooks.python] sets worker_startup.
        // [hooks.python.pytest] sets worker_teardown only.
        // Resolver must return:
        //   startup  = python.worker_startup (inherited)
        //   teardown = pytest.worker_teardown (tool-specific)
        let dir = tempfile::tempdir().unwrap();
        write_script(dir.path(), "scripts/py_any.py", "");
        write_script(dir.path(), "scripts/py_pytest_td.py", "");

        let toml_src = r#"
[hooks.python]
worker_startup = "scripts/py_any.py"

[hooks.python.pytest]
worker_teardown = "scripts/py_pytest_td.py"
"#;
        let cfg: Config = toml::from_str(toml_src).expect("parse");

        // A minimal plugin stub: only language() and name() are used by
        // resolve_worker_hooks. Construct it inline without touching the
        // registry.
        struct FakePyPytest;
        impl Plugin for FakePyPytest {
            fn name(&self) -> &'static str {
                "pytest"
            }
            fn language(&self) -> &'static str {
                "python"
            }
            fn detect(&self, _: &Path) -> bool {
                false
            }
            fn subprocess_cmd(&self, _: &Path) -> Vec<String> {
                Vec::new()
            }
            fn runner_script(&self) -> &'static str {
                ""
            }
            fn script_extension(&self) -> &'static str {
                "py"
            }
            fn project_name(&self, _: &Path) -> String {
                "fake".into()
            }
            fn default_run(&self) -> Vec<String> {
                Vec::new()
            }
            fn default_watch(&self) -> Vec<String> {
                Vec::new()
            }
            fn is_test_file(&self, _: &Path) -> bool {
                false
            }
            fn is_source_file(&self, _: &Path) -> bool {
                false
            }
        }

        let hooks = resolve_worker_hooks(&cfg, &FakePyPytest, dir.path()).unwrap();
        assert!(
            hooks
                .startup
                .as_ref()
                .is_some_and(|p| p.ends_with("py_any.py")),
            "startup must inherit from [hooks.python]; got {:?}",
            hooks.startup
        );
        assert!(
            hooks
                .teardown
                .as_ref()
                .is_some_and(|p| p.ends_with("py_pytest_td.py")),
            "teardown must come from [hooks.python.pytest] (tool-level wins); got {:?}",
            hooks.teardown
        );
    }

    #[test]
    fn config_flatten_hooks_parses() {
        let toml_src = r#"
[hooks]
startup = "scripts/startup.sh"
teardown = "scripts/teardown.sh"

[hooks.python]
worker_startup = "scripts/py_any.py"

[hooks.python.pytest]
worker_startup = "scripts/py_pytest.py"
worker_teardown = "scripts/py_pytest_td.py"

[hooks.r.testthat]
worker_startup = "scripts/r_tt.R"
"#;
        let cfg: Config = toml::from_str(toml_src).expect("parse");
        assert_eq!(
            cfg.hooks.startup.as_deref(),
            Some(Path::new("scripts/startup.sh"))
        );
        let py = cfg.hooks.by_language.get("python").expect("python lang");
        assert_eq!(
            py.worker_startup.as_deref(),
            Some(Path::new("scripts/py_any.py"))
        );
        let pyt = py.by_tool.get("pytest").expect("pytest tool");
        assert_eq!(
            pyt.worker_startup.as_deref(),
            Some(Path::new("scripts/py_pytest.py"))
        );
        let r = cfg.hooks.by_language.get("r").expect("r lang");
        let tt = r.by_tool.get("testthat").expect("testthat tool");
        assert_eq!(
            tt.worker_startup.as_deref(),
            Some(Path::new("scripts/r_tt.R"))
        );
    }
}
