//! Startup pre-flight checks.
//!
//! These run *once* in the binary's `run_subcommand` after the `Package`
//! is built and before the first worker spawns. Each check fails fast
//! with an actionable error so users see "your venv has no editable
//! install, run `pip install -e .`" instead of N copies of
//! `ModuleNotFoundError` mid-run.
//!
//! Cross-platform notes: every subprocess invocation uses
//! `Command::new(bin)` which on Windows resolves PATH (with PATHEXT)
//! the same way `cmd /c bin` would. Path comparisons go through
//! `Path::is_dir` / `Path::is_file` which work on all platforms.
//! Executable-bit checks live in `project::hooks::validate_script`
//! and are already gated by `#[cfg(unix)]`.
//!
//! Every check is opt-out via `[preflight] enabled = false` (master)
//! or per-check flags (`[preflight] python_imports = false`, etc.).

use std::io::ErrorKind;
use std::path::Path;
use std::process::Command;

use anyhow::{Context, Result};

use crate::project::config::PreflightConfig;
use crate::project::package::Package;

/// Run every enabled pre-flight check. Returns the first failure (with
/// an actionable message) or Ok when everything passes / is disabled.
pub fn run_all(pkg: &Package, cfg: &PreflightConfig) -> Result<()> {
    if !cfg.enabled {
        return Ok(());
    }
    if cfg.suite_roots {
        check_suite_roots(pkg)?;
    }
    if cfg.run_globs {
        check_run_globs(pkg)?;
    }
    if cfg.command_tools {
        check_command_tools(pkg)?;
    }
    if cfg.python_imports {
        check_python_imports(pkg)?;
    }
    if cfg.r_pkgload {
        check_r_pkgload(pkg)?;
    }
    Ok(())
}

// ── Check 1: suite roots exist ─────────────────────────────────────────────

fn check_suite_roots(pkg: &Package) -> Result<()> {
    for suite in &pkg.test_suites {
        if !suite.root.is_dir() {
            anyhow::bail!(
                "[[suite]] {}: root {:?} is not an existing directory.\n\
                 Hint: typo in the `root` field, or the directory hasn't been created yet.\n\
                 Disable this check with `[preflight] suite_roots = false`.",
                suite.plugin.name(),
                suite.root.display(),
            );
        }
    }
    Ok(())
}

// ── Check 2: run globs match at least one file ─────────────────────────────

fn check_run_globs(pkg: &Package) -> Result<()> {
    for suite in &pkg.test_suites {
        let dirs = suite.run_search_dirs();
        let hit = dirs.iter().filter(|d| d.is_dir()).any(|dir| {
            !crate::analysis::walk::collect_files(dir, |p| suite.owns_test_file(p)).is_empty()
        });
        if !hit {
            anyhow::bail!(
                "[[suite]] {}: `run` globs matched zero files under {}.\n\
                 Patterns: {:?}\n\
                 Hint: check the patterns; remember `tests/foo` is a literal path, \
                 not a recursive walk (write `tests/foo/**/*.py` instead).\n\
                 Disable this check with `[preflight] run_globs = false`.",
                suite.plugin.name(),
                suite.root.display(),
                suite.run,
            );
        }
    }
    Ok(())
}

// ── Check 3: CLI tools on PATH for command-mode plugins ────────────────────

fn check_command_tools(pkg: &Package) -> Result<()> {
    for suite in &pkg.test_suites {
        let spec = match suite.plugin.command_spec(&suite.root, pkg) {
            Some(s) => s,
            None => continue,
        };
        let bin = match spec.argv.first() {
            Some(b) => b,
            None => continue,
        };
        if !is_executable_on_path(bin) {
            anyhow::bail!(
                "[[suite]] {suite}: command-line tool {bin:?} not found on PATH.\n\
                 \n  {hint}\n\
                 \n  Once installed, ensure it is on PATH.\n\
                 \n  Disable this check with `[preflight] command_tools = false`.",
                suite = suite.plugin.name(),
                bin = bin,
                hint = install_hint(bin),
            );
        }
    }
    Ok(())
}

fn install_hint(bin: &str) -> String {
    match bin {
        "jarl" => "Install instructions: https://jarl.etiennebacher.com/".into(),
        "ruff" => "Install with `pip install ruff` or `brew install ruff`.".into(),
        "skyspell" => "Install instructions: https://codeberg.org/your-tools/skyspell".into(),
        "typos" => "Install instructions: https://github.com/crate-ci/typos".into(),
        _ => format!("Install `{bin}` and ensure it is on PATH."),
    }
}

/// Cross-platform "is `bin` runnable from PATH?" probe. Spawns
/// `bin --version` (most CLI tools support this) and treats only
/// `ErrorKind::NotFound` from the spawn as a hard miss. Any successful
/// spawn — even non-zero exit, even malformed --version handling —
/// counts as "found": the binary exists, it just may not like our args.
fn is_executable_on_path(bin: &str) -> bool {
    match Command::new(bin)
        .arg("--version")
        .stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .status()
    {
        Ok(_) => true,
        Err(e) => e.kind() != ErrorKind::NotFound,
    }
}

// ── Check 4: Python project module imports ─────────────────────────────────

fn check_python_imports(pkg: &Package) -> Result<()> {
    for suite in &pkg.test_suites {
        if suite.plugin.language() != "python" {
            continue;
        }
        let module = match suite.plugin.project_module_name(&suite.root) {
            Some(m) if !m.is_empty() => m,
            _ => continue,
        };
        check_python_import_one(pkg, &suite.root, &module, suite.plugin.name())?;
    }
    Ok(())
}

fn check_python_import_one(
    pkg: &Package,
    suite_root: &Path,
    module: &str,
    suite_name: &str,
) -> Result<()> {
    let interpreter = if !pkg.python_interpreter.is_empty() {
        pkg.python_interpreter.clone()
    } else {
        vec![crate::python::py_find_python(suite_root, None)]
    };

    let (bin, args) = interpreter.split_first().ok_or_else(|| {
        anyhow::anyhow!(
            "python suite {}: empty interpreter command",
            suite_name
        )
    })?;
    let mut cmd = Command::new(bin);
    cmd.args(args);
    cmd.arg("-c");
    cmd.arg(format!("import {module}"));
    cmd.current_dir(suite_root);
    cmd.env("SCRUTIN_PKG_DIR", suite_root);

    // Mirror runner.py:main()'s sys.path setup so the pre-flight sees
    // the same import resolution the test workers will see.
    let mut path_entries: Vec<String> = Vec::new();
    let src = suite_root.join("src");
    if src.is_dir() {
        path_entries.push(src.to_string_lossy().into_owned());
    }
    path_entries.push(suite_root.to_string_lossy().into_owned());
    if let Ok(existing) = std::env::var("PYTHONPATH")
        && !existing.is_empty()
    {
        path_entries.push(existing);
    }
    cmd.env("PYTHONPATH", path_entries.join(path_separator()));

    let output = match cmd.output() {
        Ok(o) => o,
        Err(e) => {
            anyhow::bail!(
                "python suite {} ({}): failed to spawn interpreter {:?}: {}\n\
                 Hint: install Python or set [python].interpreter in .scrutin/config.toml.\n\
                 Disable this check with `[preflight] python_imports = false`.",
                suite_name,
                suite_root.display(),
                bin,
                e,
            );
        }
    };
    if output.status.success() {
        return Ok(());
    }
    anyhow::bail!(
        "python suite {} ({}): cannot import package '{}'.\n\
         Hint: from {}, run `uv pip install -e .` (or `pip install -e .` in your venv).\n\
         Disable this check with `[preflight] python_imports = false`.\n\
         Interpreter output:\n{}",
        suite_name,
        suite_root.display(),
        module,
        suite_root.display(),
        stderr_tail(&output.stderr, 3),
    );
}

// ── Check 5: R pkgload installed ───────────────────────────────────────────

fn check_r_pkgload(pkg: &Package) -> Result<()> {
    let mut seen: std::collections::HashSet<&Path> = std::collections::HashSet::new();
    for suite in &pkg.test_suites {
        if suite.plugin.language() != "r" {
            continue;
        }
        // Skip pure linter R plugins (jarl) which don't load_all.
        if suite.plugin.command_spec(&suite.root, pkg).is_some() {
            continue;
        }
        if !seen.insert(suite.root.as_path()) {
            // Already checked this root for a sibling R suite.
            continue;
        }
        check_r_pkgload_one(&suite.root, suite.plugin.name())?;
    }
    Ok(())
}

fn check_r_pkgload_one(suite_root: &Path, suite_name: &str) -> Result<()> {
    let mut cmd = Command::new("Rscript");
    cmd.arg("--vanilla")
        .arg("-e")
        .arg("if (!requireNamespace(\"pkgload\", quietly = TRUE)) quit(status = 1)")
        .current_dir(suite_root)
        .stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::piped());

    let output = cmd
        .output()
        .with_context(|| {
            format!(
                "[[suite]] {}: failed to spawn Rscript. \
                 Is R installed and on PATH? \
                 Disable this check with `[preflight] r_pkgload = false`.",
                suite_name
            )
        })?;
    if output.status.success() {
        return Ok(());
    }
    let tail = stderr_tail(&output.stderr, 3);
    let extra = if tail.trim().is_empty() {
        String::new()
    } else {
        format!("\nRscript output:\n{tail}")
    };
    anyhow::bail!(
        "[[suite]] {} ({}): R package `pkgload` is not installed.\n\
         Hint: open R and run `install.packages(\"pkgload\")`, \
         or edit .scrutin/<tool>/runner.R to use library() instead.\n\
         Disable this check with `[preflight] r_pkgload = false`.{}",
        suite_name,
        suite_root.display(),
        extra,
    );
}

// ── Cross-platform path separator for PYTHONPATH ──────────────────────────

#[cfg(windows)]
fn path_separator() -> &'static str {
    ";"
}

#[cfg(not(windows))]
fn path_separator() -> &'static str {
    ":"
}

// ── Error-message formatting ───────────────────────────────────────────────

/// Last `n` lines of a subprocess stderr buffer, in original order.
/// Used to surface the root-cause traceback in pre-flight failure
/// messages without flooding the terminal.
fn stderr_tail(bytes: &[u8], n: usize) -> String {
    let stderr = String::from_utf8_lossy(bytes);
    let mut tail: Vec<&str> = stderr.lines().rev().take(n).collect();
    tail.reverse();
    tail.join("\n")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn is_executable_finds_a_universal_command() {
        // Every supported OS has at least one of these.
        let probe = if cfg!(windows) { "cmd" } else { "ls" };
        assert!(is_executable_on_path(probe));
    }

    #[test]
    fn is_executable_returns_false_for_missing() {
        assert!(!is_executable_on_path(
            "definitely_not_a_real_binary_qwertyuiop_12345"
        ));
    }
}
