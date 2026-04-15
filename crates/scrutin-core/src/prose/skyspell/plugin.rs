//! skyspell `Plugin` impl (command mode).
//!
//! skyspell is a spell checker, not a test framework, so plugin semantics
//! mirror the jarl and ruff linter plugins:
//!
//! - `supported_outcomes` is `[Pass, Warn, Error]`: misspellings are warn,
//!   not fail, so they do not dominate the red count next to real failures.
//! - `subject_label` is "word"; every misspelling becomes one warn event
//!   carrying the word, its 1-based position, and ranked suggestions via
//!   the protocol's `corrections` field.
//!
//! Opt-in: detection requires `skyspell.toml` at the project root (mirrors
//! `jarl.toml`). The marker file may declare `lang = "en_US"` (default:
//! `en_US`).

use std::collections::BTreeMap;
use std::path::Path;

use serde::Deserialize;

use crate::analysis::walk;
use crate::engine::protocol::{
    Correction, Counts, Event, Message, Outcome, Subject, Summary,
};
use crate::project::plugin::{CommandSpec, Plugin};

pub struct SkyspellPlugin;

impl Plugin for SkyspellPlugin {
    fn name(&self) -> &'static str {
        "skyspell"
    }
    fn language(&self) -> &'static str {
        "prose"
    }
    fn detect(&self, _root: &Path) -> bool {
        false
    }
    fn project_name(&self, root: &Path) -> String {
        root.file_name()
            .and_then(|n| n.to_str())
            .unwrap_or("<unknown>")
            .to_string()
    }
    fn tool_version(&self, _root: &Path) -> Option<String> {
        skyspell_cli_version()
    }
    fn default_run(&self) -> Vec<String> {
        vec![
            "**/*.md".into(),
            "**/*.markdown".into(),
            "**/*.txt".into(),
            "**/*.rst".into(),
            "**/*.qmd".into(),
            "**/*.Rmd".into(),
        ]
    }
    fn default_watch(&self) -> Vec<String> {
        Vec::new()
    }
    fn is_test_file(&self, path: &Path) -> bool {
        walk::has_extension(path, &["md", "markdown", "txt", "rst", "qmd"])
            || path
                .extension()
                .and_then(|e| e.to_str())
                .is_some_and(|e| e.eq_ignore_ascii_case("rmd"))
    }
    fn is_source_file(&self, _path: &Path) -> bool {
        false
    }
    fn supported_outcomes(&self) -> &'static [Outcome] {
        &[Outcome::Pass, Outcome::Warn, Outcome::Error]
    }
    fn subject_label(&self) -> &'static str {
        "word"
    }

    fn command_spec(
        &self,
        root: &Path,
        pkg: &crate::project::package::Package,
    ) -> Option<CommandSpec> {
        let mut argv: Vec<String> = vec![
            "skyspell".into(),
            "--project-path".into(),
            root.to_string_lossy().into_owned(),
        ];
        argv.extend(pkg.skyspell_extra_args.iter().cloned());
        argv.extend([
            "check".into(),
            "--output-format".into(),
            "json".into(),
        ]);
        Some(CommandSpec { argv })
    }

    fn parse_command_output(
        &self,
        file: &str,
        stdout: &str,
        stderr: &str,
        exit_code: Option<i32>,
        duration_ms: u64,
    ) -> Vec<Message> {
        parse_skyspell_output(file, stdout, stderr, exit_code, duration_ms)
    }
}

// ── skyspell JSON output parsing ─────────────────────────────────────────

#[derive(Deserialize)]
struct SkyspellOutput {
    #[serde(default)]
    errors: BTreeMap<String, Vec<SkyspellError>>,
    #[serde(default)]
    suggestions: BTreeMap<String, Vec<String>>,
}

#[derive(Deserialize)]
struct SkyspellError {
    word: String,
    range: SkyspellRange,
}

#[derive(Deserialize)]
struct SkyspellRange {
    line: u32,
    start_column: u32,
    end_column: u32,
}

fn parse_skyspell_output(
    file: &str,
    stdout: &str,
    stderr: &str,
    exit_code: Option<i32>,
    duration_ms: u64,
) -> Vec<Message> {
    let mut messages = Vec::new();
    let mut counts = Counts::default();

    // skyspell exits non-zero for internal errors (bad lang, missing
    // dictionaries, etc). In those cases stdout is typically empty and the
    // real message is on stderr.
    if !matches!(exit_code, Some(0) | None) && stdout.trim().is_empty() {
        let msg = if stderr.trim().is_empty() {
            format!("skyspell exited with code {:?}", exit_code)
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

    let parsed: SkyspellOutput = if stdout.trim().is_empty() {
        SkyspellOutput {
            errors: BTreeMap::new(),
            suggestions: BTreeMap::new(),
        }
    } else {
        match serde_json::from_str(stdout) {
            Ok(p) => p,
            Err(e) => {
                counts.bump(Outcome::Error);
                messages.push(Message::Event(Event::engine_error(
                    file,
                    format!("failed to parse skyspell output: {e}"),
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

    // skyspell keys `errors` by the path it received; we flatten across all
    // keys since the engine only ever asks about one file at a time.
    let mut total_errors = 0usize;
    for per_file in parsed.errors.values() {
        for err in per_file {
            total_errors += 1;
            let suggestions = parsed
                .suggestions
                .get(&err.word)
                .cloned()
                .unwrap_or_default();
            let correction = Correction {
                word: err.word.clone(),
                line: err.range.line,
                col_start: err.range.start_column,
                col_end: err.range.end_column,
                suggestions,
            };
            counts.bump(Outcome::Warn);
            messages.push(Message::Event(Event {
                file: file.to_string(),
                outcome: Outcome::Warn,
                subject: Subject {
                    kind: "word".into(),
                    name: err.word.clone(),
                    parent: None,
                },
                metrics: None,
                failures: Vec::new(),
                message: Some(format!("misspelled: {}", err.word)),
                line: Some(err.range.line),
                duration_ms: 0,
                corrections: vec![correction],
            }));
        }
    }

    if total_errors == 0 {
        counts.bump(Outcome::Pass);
        messages.push(Message::Event(Event {
            file: file.to_string(),
            outcome: Outcome::Pass,
            subject: Subject {
                kind: "word".into(),
                name: "spelling".into(),
                parent: None,
            },
            metrics: None,
            failures: Vec::new(),
            message: None,
            line: None,
            duration_ms,
            corrections: Vec::new(),
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

/// Query `skyspell --version`. Any failure returns `None`.
fn skyspell_cli_version() -> Option<String> {
    use std::process::Command;
    let out = Command::new("skyspell").arg("--version").output().ok()?;
    if !out.status.success() {
        return None;
    }
    let s = String::from_utf8(out.stdout).ok()?;
    let trimmed = s.trim();
    if trimmed.is_empty() {
        return None;
    }
    let v = trimmed
        .strip_prefix("skyspell ")
        .unwrap_or(trimmed)
        .trim()
        .to_string();
    if v.is_empty() { None } else { Some(v) }
}

#[cfg(test)]
mod tests {
    use super::*;

    const SAMPLE: &str = r#"{
      "errors": {
        "/tmp/sample.txt": [
          {"word": "tset", "range": {"line": 1, "start_column": 11, "end_column": 14}},
          {"word": "teh",  "range": {"line": 1, "start_column": 37, "end_column": 39}}
        ]
      },
      "suggestions": {
        "tset": ["test", "stet"],
        "teh":  ["the", "tea"]
      }
    }"#;

    #[test]
    fn parse_emits_one_warn_per_error_with_suggestions() {
        let msgs = parse_skyspell_output("sample.txt", SAMPLE, "", Some(0), 42);
        let events: Vec<&Event> = msgs
            .iter()
            .filter_map(|m| if let Message::Event(e) = m { Some(e) } else { None })
            .collect();
        assert_eq!(events.len(), 2);
        let first = events[0];
        assert_eq!(first.outcome, Outcome::Warn);
        assert_eq!(first.subject.name, "tset");
        assert_eq!(first.line, Some(1));
        assert_eq!(first.corrections.len(), 1);
        assert_eq!(first.corrections[0].word, "tset");
        assert_eq!(first.corrections[0].col_start, 11);
        assert_eq!(first.corrections[0].col_end, 14);
        assert_eq!(
            first.corrections[0].suggestions,
            vec!["test".to_string(), "stet".to_string()]
        );
    }

    #[test]
    fn parse_emits_pass_on_clean_file() {
        let msgs = parse_skyspell_output(
            "clean.txt",
            r#"{"errors":{},"suggestions":{}}"#,
            "",
            Some(0),
            10,
        );
        let events: Vec<&Event> = msgs
            .iter()
            .filter_map(|m| if let Message::Event(e) = m { Some(e) } else { None })
            .collect();
        assert_eq!(events.len(), 1);
        assert_eq!(events[0].outcome, Outcome::Pass);
    }

    #[test]
    fn parse_handles_malformed_json() {
        let msgs = parse_skyspell_output("x.txt", "not json", "", Some(0), 1);
        let events: Vec<&Event> = msgs
            .iter()
            .filter_map(|m| if let Message::Event(e) = m { Some(e) } else { None })
            .collect();
        assert_eq!(events.len(), 1);
        assert_eq!(events[0].outcome, Outcome::Error);
    }
}
