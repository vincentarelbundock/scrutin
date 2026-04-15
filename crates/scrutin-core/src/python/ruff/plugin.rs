//! ruff `Plugin` impl (command mode).
//!
//! ruff is a linter, not a test framework, so several plugin methods have
//! different semantics than pytest:
//!
//! - `source_dirs` is empty: there is no separate source/test split for a
//!   linter; each file depends only on itself.
//! - `discover_test_files` walks for *all* `.py` files (no prefix filter).
//! - `supported_outcomes` is `[Pass, Warn, Error]`: violations are warn,
//!   not fail, so a lint issue does not dominate the red count next to a
//!   real test failure.
//!
//! Because ruff is an external CLI tool (not a Python library), this plugin
//! uses command mode: the engine calls `ruff check --output-format json`
//! directly and this module parses the JSON output in Rust. No Python
//! subprocess needed.
//!
//! Modeled on `r/jarl/plugin.rs`.

use std::path::Path;

use serde::Deserialize;

use crate::analysis::walk;
use crate::engine::protocol::{Counts, Event, Message, Outcome, Subject, Summary};
use crate::project::plugin::{CommandSpec, Plugin, PluginAction};
use crate::python::{py_env_vars, py_parse_pyproject_version, py_project_name_or_dir};

pub struct RuffPlugin;

impl Plugin for RuffPlugin {
    fn name(&self) -> &'static str {
        "ruff"
    }
    fn language(&self) -> &'static str {
        "python"
    }
    fn detect(&self, _root: &Path) -> bool {
        false
    }
    fn project_name(&self, root: &Path) -> String {
        py_project_name_or_dir(root)
    }
    fn project_version(&self, root: &Path) -> Option<String> {
        py_parse_pyproject_version(root)
    }
    fn tool_version(&self, _root: &Path) -> Option<String> {
        ruff_cli_version()
    }
    fn default_run(&self) -> Vec<String> {
        // Lint every .py under the suite root; ruff's own config handles
        // exclusions.
        vec!["**/*.py".into()]
    }
    fn default_watch(&self) -> Vec<String> {
        Vec::new()
    }
    fn is_test_file(&self, path: &Path) -> bool {
        walk::has_extension(path, &["py"])
    }
    fn is_source_file(&self, _path: &Path) -> bool {
        false
    }
    fn test_file_candidates(&self, stem: &str) -> Vec<String> {
        vec![format!("{stem}.py")]
    }
    fn env_vars(&self, root: &Path) -> Vec<(String, String)> {
        py_env_vars("ruff", root)
    }
    fn is_noise_line(&self, _line: &str) -> bool {
        false
    }
    fn supported_outcomes(&self) -> &'static [Outcome] {
        &[Outcome::Pass, Outcome::Warn, Outcome::Error]
    }
    fn subject_label(&self) -> &'static str {
        "rule"
    }
    fn actions(&self) -> Vec<PluginAction> {
        use crate::project::plugin::ActionScope;
        let base_fix: Vec<String> = vec![
            "ruff".into(),
            "check".into(),
            "--fix".into(),
        ];
        let base_fix_unsafe: Vec<String> = vec![
            "ruff".into(),
            "check".into(),
            "--fix".into(),
            "--unsafe-fixes".into(),
        ];
        vec![
            PluginAction {
                name: "fix",
                key: 'f',
                label: "Ruff: fix",
                command: base_fix.clone(),
                rerun: true,
                scope: ActionScope::File,
            },
            PluginAction {
                name: "fix_unsafe",
                key: 'F',
                label: "Ruff: fix (unsafe)",
                command: base_fix_unsafe.clone(),
                rerun: true,
                scope: ActionScope::File,
            },
            PluginAction {
                name: "fix_all",
                key: 'a',
                label: "Ruff: fix all",
                command: base_fix,
                rerun: true,
                scope: ActionScope::All,
            },
            PluginAction {
                name: "fix_unsafe_all",
                key: 'A',
                label: "Ruff: fix all (unsafe)",
                command: base_fix_unsafe,
                rerun: true,
                scope: ActionScope::All,
            },
        ]
    }

    fn command_spec(
        &self,
        _root: &Path,
        _pkg: &crate::project::package::Package,
    ) -> Option<CommandSpec> {
        Some(CommandSpec {
            argv: vec![
                "ruff".into(),
                "check".into(),
                "--output-format".into(),
                "json".into(),
            ],
        })
    }

    fn parse_command_output(
        &self,
        file: &str,
        stdout: &str,
        stderr: &str,
        exit_code: Option<i32>,
        duration_ms: u64,
    ) -> Vec<Message> {
        parse_ruff_output(file, stdout, stderr, exit_code, duration_ms)
    }
}

// ── ruff JSON output parsing ──────────────────────────────────────────────

#[derive(Deserialize)]
struct RuffDiagnostic {
    #[serde(default)]
    code: Option<String>,
    #[serde(default)]
    message: Option<String>,
    #[serde(default)]
    location: Option<RuffLocation>,
}

#[derive(Deserialize)]
struct RuffLocation {
    row: Option<u32>,
}

fn parse_ruff_output(
    file: &str,
    stdout: &str,
    stderr: &str,
    exit_code: Option<i32>,
    duration_ms: u64,
) -> Vec<Message> {
    let mut messages = Vec::new();
    let mut counts = Counts::default();

    // ruff exits 2 on internal error / bad args.
    if exit_code == Some(2) {
        let msg = if stderr.trim().is_empty() {
            "ruff exited with code 2".to_string()
        } else {
            stderr.trim().to_string()
        };
        counts.bump(Outcome::Error);
        messages.push(Message::Event(Event::engine_error(file, msg)));
        messages.push(Message::Summary(Summary {
            file: file.to_string(),
            duration_ms,
            counts,
        }));
        messages.push(Message::Done);
        return messages;
    }

    let diagnostics: Vec<RuffDiagnostic> = if stdout.trim().is_empty() {
        Vec::new()
    } else {
        match serde_json::from_str(stdout) {
            Ok(d) => d,
            Err(e) => {
                counts.bump(Outcome::Error);
                messages.push(Message::Event(Event::engine_error(
                    file,
                    format!("failed to parse ruff output: {e}"),
                )));
                messages.push(Message::Summary(Summary {
                    file: file.to_string(),
                    duration_ms,
                    counts,
                }));
                messages.push(Message::Done);
                return messages;
            }
        }
    };

    if diagnostics.is_empty() {
        counts.bump(Outcome::Pass);
        messages.push(Message::Event(Event {
            file: file.to_string(),
            outcome: Outcome::Pass,
            subject: Subject {
                kind: "rule".into(),
                name: "lint".into(),
                parent: None,
            },
            metrics: None,
            failures: Vec::new(),
            message: None,
            line: None,
            duration_ms,
            corrections: Vec::new(),
        }));
    } else {
        for d in &diagnostics {
            let code = d.code.as_deref().unwrap_or("unknown").to_string();
            let body = d.message.clone().unwrap_or_default();
            let line = d.location.as_ref().and_then(|loc| loc.row);

            counts.bump(Outcome::Warn);
            messages.push(Message::Event(Event {
                file: file.to_string(),
                outcome: Outcome::Warn,
                subject: Subject {
                    kind: "rule".into(),
                    name: code,
                    parent: None,
                },
                metrics: None,
                failures: Vec::new(),
                message: if body.is_empty() { None } else { Some(body) },
                line,
                duration_ms: 0,
                corrections: Vec::new(),
            }));
        }
    }

    messages.push(Message::Summary(Summary {
        file: file.to_string(),
        duration_ms,
        counts,
    }));
    messages.push(Message::Done);
    messages
}

/// Query `ruff --version`. Output looks like `ruff 0.4.0`; we strip the
/// leading `ruff ` so the caller gets just the version string. Any failure
/// returns `None`.
fn ruff_cli_version() -> Option<String> {
    use std::process::Command;
    let out = Command::new("ruff").arg("--version").output().ok()?;
    if !out.status.success() {
        return None;
    }
    let s = String::from_utf8(out.stdout).ok()?;
    let trimmed = s.trim();
    if trimmed.is_empty() {
        return None;
    }
    let v = trimmed
        .strip_prefix("ruff ")
        .unwrap_or(trimmed)
        .trim()
        .to_string();
    if v.is_empty() { None } else { Some(v) }
}
