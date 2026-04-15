//! typos `Plugin` impl (command mode).
//!
//! typos is a spell checker, not a test framework, so plugin semantics
//! mirror the other linter/checker plugins (jarl, ruff, skyspell):
//!
//! - `supported_outcomes` is `[Pass, Warn, Error]`: misspellings are warn,
//!   not fail, so they do not dominate the red count next to real failures.
//! - `subject_label` is "typo"; every misspelling becomes one warn event
//!   carrying the word, its 1-based column range, and the single suggested
//!   correction via the protocol's `corrections` field.
//!
//! typos writes newline-delimited JSON (one object per line) when invoked
//! with `--format json`. Each object has a `type` field; we care about
//! `typo` (misspellings) and treat anything else as either a parse error
//! or a typos internal warning to surface.

use std::path::Path;

use serde::Deserialize;

use crate::engine::protocol::{
    Correction, Counts, Event, Message, Outcome, Subject, Summary,
};
use crate::project::plugin::{ActionScope, CommandSpec, Plugin, PluginAction};

pub struct TyposPlugin;

impl Plugin for TyposPlugin {
    fn name(&self) -> &'static str {
        "typos"
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
        typos_cli_version()
    }
    fn default_run(&self) -> Vec<String> {
        // typos is language-agnostic: it checks any UTF-8 file against a
        // curated misspelling list. Default to common source + prose
        // extensions; users override via [[suite]].run in config.
        vec![
            "**/*.md".into(),
            "**/*.markdown".into(),
            "**/*.txt".into(),
            "**/*.rst".into(),
            "**/*.qmd".into(),
            "**/*.Rmd".into(),
            "**/*.R".into(),
            "**/*.r".into(),
            "**/*.py".into(),
            "**/*.rs".into(),
            "**/*.js".into(),
            "**/*.ts".into(),
            "**/*.go".into(),
            "**/*.c".into(),
            "**/*.h".into(),
            "**/*.cpp".into(),
            "**/*.hpp".into(),
            "**/*.java".into(),
            "**/*.rb".into(),
            "**/*.sh".into(),
            "**/*.toml".into(),
            "**/*.yaml".into(),
            "**/*.yml".into(),
        ]
    }
    fn default_watch(&self) -> Vec<String> {
        Vec::new()
    }
    fn is_test_file(&self, path: &Path) -> bool {
        // Accept any file whose extension appears in the default run set.
        // Users in file-mode are already exempt (TestSuite::explicit_files).
        let ext = match path.extension().and_then(|e| e.to_str()) {
            Some(e) => e.to_ascii_lowercase(),
            None => return false,
        };
        matches!(
            ext.as_str(),
            "md" | "markdown"
                | "txt"
                | "rst"
                | "qmd"
                | "rmd"
                | "r"
                | "py"
                | "rs"
                | "js"
                | "ts"
                | "go"
                | "c"
                | "h"
                | "cpp"
                | "hpp"
                | "java"
                | "rb"
                | "sh"
                | "toml"
                | "yaml"
                | "yml"
        )
    }
    fn is_source_file(&self, _path: &Path) -> bool {
        false
    }
    fn supported_outcomes(&self) -> &'static [Outcome] {
        &[Outcome::Pass, Outcome::Warn, Outcome::Error]
    }
    fn subject_label(&self) -> &'static str {
        "typo"
    }
    fn actions(&self) -> Vec<PluginAction> {
        let fix: Vec<String> = vec![
            "typos".into(),
            "--write-changes".into(),
        ];
        vec![
            PluginAction {
                name: "fix",
                key: 'f',
                label: "Typos: fix",
                command: fix.clone(),
                rerun: true,
                scope: ActionScope::File,
            },
            PluginAction {
                name: "fix_all",
                key: 'a',
                label: "Typos: fix all",
                command: fix,
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
            argv: vec!["typos".into(), "--format".into(), "json".into()],
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
        parse_typos_output(file, stdout, stderr, exit_code, duration_ms)
    }
}

// ── typos NDJSON output parsing ───────────────────────────────────────────

#[derive(Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
enum TyposRecord {
    Typo(TyposTypo),
    #[serde(other)]
    Other,
}

#[derive(Deserialize)]
struct TyposTypo {
    #[serde(default)]
    line_num: u32,
    #[serde(default)]
    byte_offset: u32,
    #[serde(default)]
    typo: String,
    #[serde(default)]
    corrections: Vec<String>,
}

fn parse_typos_output(
    file: &str,
    stdout: &str,
    stderr: &str,
    exit_code: Option<i32>,
    duration_ms: u64,
) -> Vec<Message> {
    let mut messages = Vec::new();
    let mut counts = Counts::default();

    // typos exit codes: 0 = no typos, 2 = typos found, anything else = error
    // (bad args, unreadable file, internal failure). Stdout is empty on
    // error; the real message is on stderr.
    let unexpected_exit = !matches!(exit_code, Some(0) | Some(2) | None);
    if unexpected_exit {
        let msg = if stderr.trim().is_empty() {
            format!("typos exited with code {:?}", exit_code)
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

    let mut total_typos = 0usize;
    for line in stdout.lines() {
        if line.trim().is_empty() {
            continue;
        }
        let record: TyposRecord = match serde_json::from_str(line) {
            Ok(r) => r,
            Err(e) => {
                counts.bump(Outcome::Error);
                messages.push(Message::Event(Event::engine_error(
                    file,
                    format!("failed to parse typos output: {e}"),
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
        let t = match record {
            TyposRecord::Typo(t) => t,
            TyposRecord::Other => continue,
        };
        total_typos += 1;
        // typos reports line 1-based and byte_offset 0-based within that
        // line. scrutin's Correction uses 1-based inclusive byte-column
        // bounds (matching skyspell, which `apply_correction_to_file` is
        // written against): col_start is the first byte of the word,
        // col_end is the last.
        let len = t.typo.len() as u32;
        if len == 0 {
            continue;
        }
        let col_start = t.byte_offset + 1;
        let col_end = t.byte_offset + len;
        let correction = Correction {
            word: t.typo.clone(),
            line: t.line_num,
            col_start,
            col_end,
            suggestions: t.corrections,
        };
        counts.bump(Outcome::Warn);
        messages.push(Message::Event(Event {
            file: file.to_string(),
            outcome: Outcome::Warn,
            subject: Subject {
                kind: "typo".into(),
                name: t.typo.clone(),
                parent: None,
            },
            metrics: None,
            failures: Vec::new(),
            message: Some(format!("misspelled: {}", t.typo)),
            line: Some(t.line_num),
            duration_ms: 0,
            corrections: vec![correction],
        }));
    }

    if total_typos == 0 {
        counts.bump(Outcome::Pass);
        messages.push(Message::Event(Event {
            file: file.to_string(),
            outcome: Outcome::Pass,
            subject: Subject {
                kind: "typo".into(),
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

/// Query `typos --version`. Output is `typos-cli 1.45.1`; strip the leading
/// `typos-cli ` so callers get just the version string. Any failure returns
/// `None`.
fn typos_cli_version() -> Option<String> {
    use std::process::Command;
    let out = Command::new("typos").arg("--version").output().ok()?;
    if !out.status.success() {
        return None;
    }
    let s = String::from_utf8(out.stdout).ok()?;
    let trimmed = s.trim();
    if trimmed.is_empty() {
        return None;
    }
    let v = trimmed
        .strip_prefix("typos-cli ")
        .or_else(|| trimmed.strip_prefix("typos "))
        .unwrap_or(trimmed)
        .trim()
        .to_string();
    if v.is_empty() { None } else { Some(v) }
}

#[cfg(test)]
mod tests {
    use super::*;

    const SAMPLE: &str = r#"{"type":"typo","path":"/tmp/t.py","line_num":2,"byte_offset":31,"typo":"misspeled","corrections":["misspelled"]}
{"type":"typo","path":"/tmp/t.py","line_num":3,"byte_offset":14,"typo":"coment","corrections":["comment"]}
"#;

    #[test]
    fn parse_emits_one_warn_per_typo_with_correction() {
        let msgs = parse_typos_output("t.py", SAMPLE, "", Some(2), 42);
        let events: Vec<&Event> = msgs
            .iter()
            .filter_map(|m| if let Message::Event(e) = m { Some(e) } else { None })
            .collect();
        assert_eq!(events.len(), 2);
        let first = events[0];
        assert_eq!(first.outcome, Outcome::Warn);
        assert_eq!(first.subject.name, "misspeled");
        assert_eq!(first.line, Some(2));
        assert_eq!(first.corrections.len(), 1);
        assert_eq!(first.corrections[0].word, "misspeled");
        // byte_offset 31, word len 9 → col_start 32 (1-based inclusive),
        // col_end 40 (1-based inclusive last byte).
        assert_eq!(first.corrections[0].col_start, 32);
        assert_eq!(first.corrections[0].col_end, 40);
        assert_eq!(
            first.corrections[0].suggestions,
            vec!["misspelled".to_string()]
        );
    }

    #[test]
    fn parse_emits_pass_on_clean_file() {
        let msgs = parse_typos_output("clean.R", "", "", Some(0), 10);
        let events: Vec<&Event> = msgs
            .iter()
            .filter_map(|m| if let Message::Event(e) = m { Some(e) } else { None })
            .collect();
        assert_eq!(events.len(), 1);
        assert_eq!(events[0].outcome, Outcome::Pass);
    }

    #[test]
    fn parse_treats_unexpected_exit_as_error() {
        let msgs = parse_typos_output("x.txt", "", "some failure", Some(64), 1);
        let events: Vec<&Event> = msgs
            .iter()
            .filter_map(|m| if let Message::Event(e) = m { Some(e) } else { None })
            .collect();
        assert_eq!(events.len(), 1);
        assert_eq!(events[0].outcome, Outcome::Error);
    }

    /// Verifies that the col_start/col_end bounds we emit round-trip
    /// through `apply_correction_to_file`: the TUI digit-key fix flow
    /// feeds the Correction straight into that helper, and if our
    /// bounds are off by one the word-mismatch guard fires and the
    /// fix silently fails.
    #[test]
    fn correction_bounds_round_trip_through_apply() {
        use crate::prose::skyspell::apply_correction_to_file;

        let tmp = tempfile::tempdir().unwrap();
        let path = tmp.path().join("sample.R");
        std::fs::write(
            &path,
            "foo <- function() {\n  # this comment has a recieve typo\n}\n",
        )
        .unwrap();

        // Match what `typos --format json` emits for that file.
        let record = r#"{"type":"typo","path":"x","line_num":2,"byte_offset":23,"typo":"recieve","corrections":["receive"]}"#;
        let msgs = parse_typos_output("sample.R", record, "", Some(2), 1);
        let event = msgs
            .iter()
            .find_map(|m| if let Message::Event(e) = m { Some(e) } else { None })
            .expect("expected one typo event");
        let correction = event.corrections.first().expect("correction attached");

        apply_correction_to_file(&path, correction, &correction.suggestions[0]).unwrap();
        let after = std::fs::read_to_string(&path).unwrap();
        assert!(
            after.contains("a receive typo"),
            "apply_correction_to_file did not rewrite the word; file is:\n{after}"
        );
    }

    #[test]
    fn parse_skips_non_typo_records() {
        let mixed = r#"{"type":"note","message":"something else"}
{"type":"typo","path":"t.R","line_num":1,"byte_offset":0,"typo":"recieve","corrections":["receive"]}
"#;
        let msgs = parse_typos_output("t.R", mixed, "", Some(2), 1);
        let warns = msgs
            .iter()
            .filter_map(|m| if let Message::Event(e) = m { Some(e) } else { None })
            .filter(|e| e.outcome == Outcome::Warn)
            .count();
        assert_eq!(warns, 1);
    }
}
