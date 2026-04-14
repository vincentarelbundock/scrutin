//! jarl `Plugin` impl (command mode).
//!
//! jarl is a linter, not a test framework, so several plugin methods have
//! different semantics than testthat/tinytest:
//!
//! - `source_dirs` is empty: there's no separate source/test split for a
//!   linter; each file depends only on itself.
//! - `discover_test_files` walks `R/` for *all* `.R` files (no prefix).
//! - `supported_outcomes` is `[Pass, Warn, Error]`: violations are warn,
//!   not fail, so a lint issue doesn't dominate the red count.
//!
//! Because jarl is an external CLI tool (not an R library), this plugin
//! uses command mode: the engine calls `jarl check --output-format json`
//! directly and this module parses the JSON output in Rust. No Rscript
//! subprocess needed.

use std::path::{Path, PathBuf};

use anyhow::Result;
use serde::Deserialize;

use crate::analysis::walk;
use crate::engine::protocol::{Counts, Event, Message, Outcome, Subject, Summary};
use crate::project::plugin::{CommandSpec, Plugin, PluginAction};
use crate::r::{parse_r_package_name, parse_r_package_version, r_env_vars, r_is_noise_line};

pub struct JarlPlugin;

impl Plugin for JarlPlugin {
    fn name(&self) -> &'static str {
        "jarl"
    }
    fn language(&self) -> &'static str {
        "r"
    }
    fn detect(&self, root: &Path) -> bool {
        root.join(super::MARKER).is_file()
            && root.join("DESCRIPTION").is_file()
            && root.join(super::LINT_DIR).is_dir()
    }
    // Worker-mode methods: not called for command plugins, but the trait
    // requires them. Return inert values.
    fn subprocess_cmd(&self, _root: &Path) -> Vec<String> {
        vec![]
    }
    fn runner_script(&self) -> &'static str {
        ""
    }
    fn script_extension(&self) -> &'static str {
        "R"
    }
    fn runner_basename(&self) -> String {
        "runner_jarl.R".into()
    }
    fn project_name(&self, root: &Path) -> String {
        parse_r_package_name(root)
    }
    fn project_version(&self, root: &Path) -> Option<String> {
        parse_r_package_version(root)
    }
    fn tool_version(&self, _root: &Path) -> Option<String> {
        // jarl is an external CLI binary, not an R package. Query the CLI.
        jarl_cli_version()
    }
    fn source_dirs(&self) -> Vec<&'static str> {
        vec![]
    }
    fn test_dirs(&self) -> Vec<&'static str> {
        vec![super::LINT_DIR]
    }
    fn discover_test_files(&self, _root: &Path, test_dir: &Path) -> Result<Vec<PathBuf>> {
        if !test_dir.is_dir() {
            return Ok(Vec::new());
        }
        Ok(walk::collect_files(test_dir, |p| {
            walk::has_extension(p, &["r"])
        }))
    }
    fn is_test_file(&self, path: &Path) -> bool {
        walk::has_extension(path, &["r"])
    }
    fn is_source_file(&self, _path: &Path) -> bool {
        false
    }
    fn test_file_candidates(&self, stem: &str) -> Vec<String> {
        vec![format!("{stem}.R"), format!("{stem}.r")]
    }
    fn env_vars(&self, root: &Path) -> Vec<(String, String)> {
        r_env_vars("jarl", root)
    }
    fn is_noise_line(&self, line: &str) -> bool {
        r_is_noise_line(line)
    }
    fn supported_outcomes(&self) -> &'static [Outcome] {
        &[Outcome::Pass, Outcome::Warn, Outcome::Error]
    }
    fn subject_label(&self) -> &'static str {
        "rule"
    }
    fn actions(&self) -> Vec<PluginAction> {
        use crate::project::plugin::ActionScope;
        let base_fix = vec![
            "jarl".into(),
            "check".into(),
            "--fix".into(),
            "--allow-dirty".into(),
            "--allow-no-vcs".into(),
        ];
        let base_fix_unsafe = vec![
            "jarl".into(),
            "check".into(),
            "--fix".into(),
            "--unsafe-fixes".into(),
            "--allow-dirty".into(),
            "--allow-no-vcs".into(),
        ];
        vec![
            PluginAction {
                name: "fix",
                key: 'f',
                label: "Jarl: fix",
                command: base_fix.clone(),
                rerun: true,
                scope: ActionScope::File,
            },
            PluginAction {
                name: "fix_unsafe",
                key: 'F',
                label: "Jarl: fix (unsafe)",
                command: base_fix_unsafe.clone(),
                rerun: true,
                scope: ActionScope::File,
            },
            PluginAction {
                name: "fix_all",
                key: 'a',
                label: "Jarl: fix all",
                command: base_fix,
                rerun: true,
                scope: ActionScope::All,
            },
            PluginAction {
                name: "fix_unsafe_all",
                key: 'A',
                label: "Jarl: fix all (unsafe)",
                command: base_fix_unsafe,
                rerun: true,
                scope: ActionScope::All,
            },
        ]
    }

    fn command_spec(&self, _root: &Path) -> Option<CommandSpec> {
        Some(CommandSpec {
            argv: vec![
                "jarl".into(),
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
        _stderr: &str,
        _exit_code: Option<i32>,
        duration_ms: u64,
    ) -> Vec<Message> {
        parse_jarl_output(file, stdout, duration_ms)
    }
}

// ── jarl JSON output parsing ──────────────────────────────────────────────

#[derive(Deserialize)]
struct JarlOutput {
    #[serde(default)]
    diagnostics: Vec<JarlDiagnostic>,
    #[serde(default)]
    errors: Vec<String>,
}

#[derive(Deserialize)]
struct JarlDiagnostic {
    message: JarlMessage,
    location: Option<JarlLocation>,
}

#[derive(Deserialize)]
struct JarlMessage {
    #[serde(default)]
    name: Option<String>,
    #[serde(default)]
    body: Option<String>,
}

#[derive(Deserialize)]
struct JarlLocation {
    row: Option<u32>,
}

fn parse_jarl_output(file: &str, stdout: &str, duration_ms: u64) -> Vec<Message> {
    let mut messages = Vec::new();
    let mut counts = Counts::default();

    let parsed: JarlOutput = match serde_json::from_str(stdout) {
        Ok(p) => p,
        Err(e) => {
            // If stdout is empty or unparseable, emit a single error.
            counts.bump(Outcome::Error);
            messages.push(Message::Event(Event::engine_error(
                file,
                format!("failed to parse jarl output: {e}"),
            )));
            messages.push(Message::Summary(Summary {
                file: file.to_string(),
                duration_ms,
                counts,
            }));
            messages.push(Message::Done);
            return messages;
        }
    };

    for err in &parsed.errors {
        counts.bump(Outcome::Error);
        messages.push(Message::Event(Event {
            file: file.to_string(),
            outcome: Outcome::Error,
            subject: Subject {
                kind: "engine".into(),
                name: "<jarl>".into(),
                parent: None,
            },
            metrics: None,
            failures: Vec::new(),
            message: Some(err.clone()),
            line: None,
            duration_ms: 0,
        }));
    }

    if parsed.diagnostics.is_empty() && parsed.errors.is_empty() {
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
        }));
    }

    for d in &parsed.diagnostics {
        let rule_name = d
            .message
            .name
            .as_deref()
            .unwrap_or("unknown")
            .to_string();
        let body = d.message.body.clone().unwrap_or_default();
        let line = d.location.as_ref().and_then(|loc| loc.row);

        counts.bump(Outcome::Warn);
        messages.push(Message::Event(Event {
            file: file.to_string(),
            outcome: Outcome::Warn,
            subject: Subject {
                kind: "rule".into(),
                name: rule_name,
                parent: None,
            },
            metrics: None,
            failures: Vec::new(),
            message: if body.is_empty() { None } else { Some(body) },
            line,
            duration_ms: 0,
        }));
    }

    messages.push(Message::Summary(Summary {
        file: file.to_string(),
        duration_ms,
        counts,
    }));
    messages.push(Message::Done);
    messages
}

/// Query `jarl --version`. Any failure (not installed, bad exit, empty
/// output) returns `None`.
fn jarl_cli_version() -> Option<String> {
    use std::process::Command;
    let out = Command::new("jarl").arg("--version").output().ok()?;
    if !out.status.success() {
        return None;
    }
    let s = String::from_utf8(out.stdout).ok()?;
    let trimmed = s.trim();
    if trimmed.is_empty() {
        return None;
    }
    // Strip common `<name> <version>` prefix, like `jarl 0.1.0`.
    let v = trimmed
        .strip_prefix("jarl ")
        .unwrap_or(trimmed)
        .trim()
        .to_string();
    if v.is_empty() { None } else { Some(v) }
}
