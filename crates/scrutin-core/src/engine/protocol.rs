//! NDJSON wire protocol between scrutin and the worker subprocesses.
//!
//! IMPORTANT: this schema is implemented by **two** runners that hand-encode
//! JSON independently — `crate::r::runner.R` (R/testthat/tinytest) and
//! `crate::python::pytest::runner.py` (Python/pytest). Any change to the
//! message shapes here must be mirrored in both files, or one language will
//! silently drop fields. There is no shared serialization layer.
//!
//! See `docs/reporting-spec.md` for the full taxonomy and design rationale.
//! Three message types:
//!
//! - `event` — one per test/assertion/validation step. Carries an [`Outcome`]
//!   from the six-value taxonomy plus optional [`Subject`], [`Metrics`], and
//!   structured [`FailureDetail`] rows.
//! - `summary` — emitted once per file at end-of-run. Carries authoritative
//!   wall time and counts; consumers prefer it over a tally of `event`
//!   messages when both are present.
//! - `done` — end-of-stream marker.
//!
//! Cancellation is **not** a wire message. The engine attaches a `cancelled`
//! flag to [`crate::engine::run_events::FileResult`] when it kills a worker
//! mid-file; workers never need to know they were cancelled.

use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};

// ── Outcome taxonomy ────────────────────────────────────────────────────────

/// The six-value outcome taxonomy. Every `Event` carries exactly one.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Outcome {
    /// Assertion held / validation step passed its threshold.
    Pass,
    /// Assertion broken / threshold violated.
    Fail,
    /// Could not evaluate (exception, missing column, broken setup).
    Error,
    /// Intentionally not run (user `skip()`, platform mismatch, precondition).
    Skip,
    /// Failed but predicted; does *not* count as a regression.
    Xfail,
    /// Soft failure: surfaced to the user but does *not* break the build.
    Warn,
}

impl Outcome {
    /// Display order for status-sorted views: failures first so the user's
    /// eye lands on the problems. Single source of truth shared across TUI
    /// and web frontends.
    pub fn rank(self) -> u8 {
        match self {
            Outcome::Fail => 0,
            Outcome::Error => 1,
            Outcome::Warn => 2,
            Outcome::Pass => 3,
            Outcome::Skip => 4,
            Outcome::Xfail => 5,
        }
    }

    /// Single-glyph icon used by the TUI, the plain reporter, the web
    /// counts bar, and the file status pills.
    pub fn icon(self) -> &'static str {
        match self {
            Outcome::Pass => "\u{25cf}",  // \u{25cf}
            Outcome::Fail => "\u{2717}",  // \u{2717}
            Outcome::Error => "\u{26a0}", // \u{26a0}
            Outcome::Skip => "\u{2298}",  // \u{2298}
            Outcome::Xfail => "\u{2299}", // \u{2299}
            Outcome::Warn => "\u{26a1}",  // \u{26a1}
        }
    }

    /// Iteration order for status-sorted emissions. Matches `rank()` so the
    /// frontend can sort by "index of this outcome in all_by_rank".
    pub fn all_by_rank() -> [Outcome; 6] {
        [
            Outcome::Fail,
            Outcome::Error,
            Outcome::Warn,
            Outcome::Pass,
            Outcome::Skip,
            Outcome::Xfail,
        ]
    }
}

// ── Event payloads ──────────────────────────────────────────────────────────

/// What was tested. Decoupled from a single `test: String` because data
/// validation needs to identify a column inside a table inside a database,
/// not just a function name.
#[derive(Debug, Clone, Deserialize)]
pub struct Subject {
    /// Freeform vocabulary: `function`, `step`, `expectation`, `check`,
    /// `field`, …. Plugins choose; the core crate doesn't enumerate.
    pub kind: String,
    /// Local identifier of the thing tested.
    pub name: String,
    /// Optional containing scope (table for a column, model for a field).
    #[serde(default)]
    pub parent: Option<String>,
}

/// Quantitative outcome data — populated by data-validation plugins, absent
/// for unit tests.
#[derive(Debug, Clone, Default, Deserialize)]
pub struct Metrics {
    /// Rows / elements considered.
    #[serde(default)]
    pub total: Option<u64>,
    /// Rows / elements that failed.
    #[serde(default)]
    pub failed: Option<u64>,
    /// Precomputed `failed / total` (the source library is the authority).
    #[serde(default)]
    pub fraction: Option<f64>,
    /// Freeform plugin-defined observations.
    #[serde(default)]
    pub observed: Option<serde_json::Value>,
}

impl Metrics {
    /// Single-line human-readable summary of the metrics. Returns `None`
    /// when there's nothing quantitative to show, so callers can inline
    /// this into a message field without producing empty leading lines.
    /// Both the TUI detail pane and the plain-mode reporter prepend this
    /// to the event's `message` so data-validation results actually surface.
    pub fn display_summary(&self) -> Option<String> {
        match (self.total, self.failed, self.fraction) {
            (Some(t), Some(f), frac) => {
                let pct = frac
                    .map(|x| x * 100.0)
                    .or_else(|| (t > 0).then(|| (f as f64 / t as f64) * 100.0))
                    .unwrap_or(0.0);
                Some(format!("{f} of {t} failed ({pct:.2}%)"))
            }
            (Some(t), None, _) => Some(format!("{t} checked")),
            (None, Some(f), _) => Some(format!("{f} failed")),
            (None, None, _) => None,
        }
    }
}

/// One failing element's structured detail. Plugins use snake_case keys so
/// the TUI can render columns consistently across rebuilds.
#[derive(Debug, Clone, Deserialize)]
pub struct FailureDetail {
    #[serde(flatten)]
    pub fields: BTreeMap<String, serde_json::Value>,
}

/// A spell-check correction attached to a `warn` event. Carries the
/// misspelled word's byte/column span inside the file plus the ranked list
/// of candidate replacements. Populated only by spell-check plugins (today:
/// skyspell); every other plugin leaves [`Event::corrections`] empty.
#[derive(Debug, Clone, Default, Deserialize)]
pub struct Correction {
    pub word: String,
    /// 1-based line number inside the file.
    pub line: u32,
    /// 1-based start column of the misspelled word.
    pub col_start: u32,
    /// 1-based exclusive end column of the misspelled word.
    pub col_end: u32,
    /// Candidate replacements, ranked best-first by the backend.
    #[serde(default)]
    pub suggestions: Vec<String>,
}

/// One per-test event.
#[derive(Debug, Clone, Deserialize)]
pub struct Event {
    pub file: String,
    pub outcome: Outcome,
    pub subject: Subject,
    #[serde(default)]
    pub metrics: Option<Metrics>,
    #[serde(default)]
    pub failures: Vec<FailureDetail>,
    #[serde(default)]
    pub message: Option<String>,
    #[serde(default)]
    pub line: Option<u32>,
    #[serde(default)]
    pub duration_ms: u64,
    /// Spell-check corrections attached to this event. Empty for all
    /// non-spell-check plugins.
    #[serde(default)]
    pub corrections: Vec<Correction>,
}

// ── Summary ─────────────────────────────────────────────────────────────────

/// Per-outcome counts. Mirrors the six-value taxonomy.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Counts {
    #[serde(default)]
    pub pass: u32,
    #[serde(default)]
    pub fail: u32,
    #[serde(default)]
    pub error: u32,
    #[serde(default)]
    pub skip: u32,
    #[serde(default)]
    pub xfail: u32,
    #[serde(default)]
    pub warn: u32,
}

impl Counts {
    /// Increment the counter for `outcome`.
    pub fn bump(&mut self, outcome: Outcome) {
        match outcome {
            Outcome::Pass => self.pass += 1,
            Outcome::Fail => self.fail += 1,
            Outcome::Error => self.error += 1,
            Outcome::Skip => self.skip += 1,
            Outcome::Xfail => self.xfail += 1,
            Outcome::Warn => self.warn += 1,
        }
    }

    /// True iff `fail > 0 || error > 0`. This is the shared "bad file" rule
    /// used by every reporter.
    pub fn bad(&self) -> bool {
        self.fail > 0 || self.error > 0
    }

    /// Subtract another `Counts` from self, clamping at zero.
    pub fn saturating_sub(&mut self, other: &Counts) {
        self.pass = self.pass.saturating_sub(other.pass);
        self.fail = self.fail.saturating_sub(other.fail);
        self.error = self.error.saturating_sub(other.error);
        self.skip = self.skip.saturating_sub(other.skip);
        self.xfail = self.xfail.saturating_sub(other.xfail);
        self.warn = self.warn.saturating_sub(other.warn);
    }

    /// Fold another `Counts` into self (for accumulating run totals).
    pub fn merge(&mut self, other: &Counts) {
        self.pass += other.pass;
        self.fail += other.fail;
        self.error += other.error;
        self.skip += other.skip;
        self.xfail += other.xfail;
        self.warn += other.warn;
    }
}

// ── File status ────────────────────────────────────────────────────────────

/// Terminal status of a file after a run. Shared decision tree used by all
/// reporters (TUI, web, plain, JUnit) so they never drift.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FileStatus {
    /// Engine cancelled this file (TUI cancel, `--max-fail`, etc.).
    Cancelled,
    /// `fail > 0 || error > 0`.
    Failed,
    /// At least one pass/xfail/warn, no failures.
    Passed,
    /// Every test was skipped (pass=0, fail=0, error=0, skip>0).
    Skipped,
    /// No events at all (should not happen for a completed file).
    Pending,
}

impl FileStatus {
    /// Derive from counts + the engine's cancelled flag.
    pub fn from_counts(counts: &Counts, cancelled: bool) -> Self {
        if cancelled {
            Self::Cancelled
        } else if counts.error > 0 || counts.fail > 0 {
            Self::Failed
        } else if counts.pass + counts.xfail + counts.warn > 0 {
            Self::Passed
        } else if counts.skip > 0 {
            Self::Skipped
        } else {
            Self::Pending
        }
    }
}

// ── File tally ─────────────────────────────────────────────────────────────

/// Result of tallying one file's messages. Every reporter converts a
/// `&[Message]` into this via [`tally_messages`], then maps it into its
/// own domain types.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct FileTally {
    /// Per-outcome counts (from events; Summary is only used for duration).
    pub counts: Counts,
    /// Wall-clock duration from the Summary message (0 if absent).
    pub duration_ms: u64,
    /// Terminal status derived from `counts` + `cancelled`.
    pub status: FileStatus,
    /// Whether the file has warnings (convenience: `counts.warn > 0`).
    pub warned: bool,
    /// Whether the file is bad (convenience: `counts.bad()`).
    pub bad: bool,
}

impl Default for FileStatus {
    fn default() -> Self {
        Self::Pending
    }
}

/// The single tally function all reporters share. Walks a file's messages,
/// counts outcomes from events, and takes `duration_ms` from the Summary.
///
/// **Counting policy**: events are authoritative for per-outcome counts.
/// The Summary is consulted only for `duration_ms` (worker wall time is
/// more accurate than the sum of per-event ms). This is the only policy
/// that lets the JUnit writer's `tests=N` attribute equal the `<testcase>`
/// count, which the JUnit schema requires.
pub fn tally_messages(messages: &[Message], cancelled: bool) -> FileTally {
    let mut counts = Counts::default();
    let mut duration_ms: u64 = 0;

    for msg in messages {
        match msg {
            Message::Event(e) => {
                counts.bump(e.outcome);
            }
            Message::Summary(s) => {
                duration_ms = s.duration_ms;
            }
            Message::Deps(_) | Message::Done => {}
        }
    }

    let status = FileStatus::from_counts(&counts, cancelled);
    let warned = counts.warn > 0;
    let bad = counts.bad();

    FileTally {
        counts,
        duration_ms,
        status,
        warned,
        bad,
    }
}

/// Authoritative end-of-file summary. Consumers prefer this over tallying
/// the `event` stream when both are present.
#[derive(Debug, Clone, Deserialize)]
pub struct Summary {
    pub file: String,
    pub duration_ms: u64,
    pub counts: Counts,
}

/// Runtime dependency observation: which source files' functions were called
/// during a single test file's execution. Emitted by the R runner's
/// `trace()`-based instrumentation after the test completes and before
/// the `done` marker.
#[derive(Debug, Clone, Deserialize)]
pub struct Deps {
    pub file: String,
    pub sources: Vec<String>,
}

// ── Top-level message ───────────────────────────────────────────────────────

#[derive(Debug, Clone, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum Message {
    Event(Event),
    Summary(Summary),
    Deps(Deps),
    Done,
}

// ── Engine-side helpers ─────────────────────────────────────────────────────

impl Event {
    /// Synthesize an engine-side error event for things the worker couldn't
    /// emit (timeouts, crashes, internal pool errors). The subject is
    /// `kind="engine"` so consumers can tell it apart from a worker-emitted
    /// error.
    pub fn engine_error(file_basename: impl Into<String>, message: impl Into<String>) -> Event {
        Event {
            file: file_basename.into(),
            outcome: Outcome::Error,
            subject: Subject {
                kind: "engine".into(),
                name: "<error>".into(),
                parent: None,
            },
            metrics: None,
            failures: Vec::new(),
            message: Some(message.into()),
            line: None,
            duration_ms: 0,
            corrections: Vec::new(),
        }
    }

    /// Display name for the test. Returns `"<anon>"` when the subject name
    /// is empty.
    pub fn display_name(&self) -> String {
        if self.subject.name.is_empty() {
            "<anon>".to_string()
        } else {
            self.subject.name.clone()
        }
    }

    /// Format the event body for display. Inlines metrics summary into the
    /// message so data-validation results surface in all reporters.
    pub fn display_body(&self) -> String {
        let original = self.message.clone().unwrap_or_default();
        match self.metrics.as_ref().and_then(|m| m.display_summary()) {
            Some(summary) if original.is_empty() => summary,
            Some(summary) => format!("{summary}\n{original}"),
            None => original,
        }
    }
}

// ── Findings ───────────────────────────────────────────────────────────────

/// A notable event (failure, error, or warning) extracted from a file's
/// message stream. Shared across TUI and plain reporters for post-run
/// summary rendering.
#[derive(Debug, Clone)]
pub struct Finding {
    /// Display name of the file (basename).
    pub file: String,
    /// Full path to the file.
    pub file_path: std::path::PathBuf,
    pub test: String,
    pub message: String,
    pub line: Option<u32>,
    pub outcome: Outcome,
}

/// Extract findings (fail/error/warn) from a file's messages.
pub fn collect_findings(messages: &[Message], file_name: &str, file_path: &std::path::Path) -> (Vec<Finding>, Vec<Finding>) {
    let mut failures = Vec::new();
    let mut warnings = Vec::new();
    for msg in messages {
        if let Message::Event(e) = msg {
            match e.outcome {
                Outcome::Fail | Outcome::Error => {
                    failures.push(Finding {
                        file: file_name.to_string(),
                        file_path: file_path.to_path_buf(),
                        test: e.display_name(),
                        message: e.display_body(),
                        line: e.line,
                        outcome: e.outcome,
                    });
                }
                Outcome::Warn => {
                    warnings.push(Finding {
                        file: file_name.to_string(),
                        file_path: file_path.to_path_buf(),
                        test: e.display_name(),
                        message: e.display_body(),
                        line: e.line,
                        outcome: e.outcome,
                    });
                }
                _ => {}
            }
        }
    }
    (failures, warnings)
}

// ── Processed event ────────────────────────────────────────────────────────

/// A test event with display-ready fields. Produced by [`process_events`]
/// so reporters don't each reimplement name/body extraction.
#[derive(Debug, Clone)]
pub struct ProcessedEvent {
    pub name: String,
    pub outcome: Outcome,
    pub message: String,
    pub line: Option<u32>,
    pub duration_ms: u64,
    /// Spell-check corrections (empty for non-spell-check plugins). Carried
    /// through from [`Event::corrections`] so frontends like the TUI can
    /// read suggestions off the current-file test list without re-parsing.
    pub corrections: Vec<Correction>,
}

impl ProcessedEvent {
    /// True when the outcome is a hard failure (fail or error).
    pub fn is_bad(&self) -> bool {
        matches!(self.outcome, Outcome::Fail | Outcome::Error)
    }
}

/// Extract display-ready events from a file's messages.
pub fn process_events(messages: &[Message]) -> Vec<ProcessedEvent> {
    messages
        .iter()
        .filter_map(|m| match m {
            Message::Event(e) => Some(ProcessedEvent {
                name: e.display_name(),
                outcome: e.outcome,
                message: e.display_body(),
                line: e.line,
                duration_ms: e.duration_ms,
                corrections: e.corrections.clone(),
            }),
            _ => None,
        })
        .collect()
}

// ── Utilities ──────────────────────────────────────────────────────────────

/// Extract the display name (basename) from a file path. Shared across all
/// reporters.
pub fn file_display_name(path: &std::path::Path) -> String {
    path.file_name()
        .unwrap_or_default()
        .to_string_lossy()
        .to_string()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn parse(line: &str) -> Message {
        serde_json::from_str(line).expect("parse")
    }

    #[test]
    fn parses_event_pass() {
        let m = parse(
            r#"{"type":"event","file":"f.R","outcome":"pass",
                "subject":{"kind":"function","name":"adds"},
                "duration_ms":12}"#,
        );
        match m {
            Message::Event(e) => {
                assert_eq!(e.file, "f.R");
                assert_eq!(e.outcome, Outcome::Pass);
                assert_eq!(e.subject.kind, "function");
                assert_eq!(e.subject.name, "adds");
                assert_eq!(e.duration_ms, 12);
                assert!(e.metrics.is_none());
                assert!(e.failures.is_empty());
            }
            _ => panic!("wrong variant"),
        }
    }

    #[test]
    fn parses_event_fail_with_metrics_and_failures() {
        let m = parse(
            r#"{"type":"event","file":"f.py","outcome":"fail",
                "subject":{"kind":"check","name":"not_null","parent":"users.email"},
                "metrics":{"total":1000,"failed":7,"fraction":0.007},
                "failures":[{"row":42,"value":null},{"row":87,"value":null}],
                "message":"7 nulls","line":17,"duration_ms":230}"#,
        );
        match m {
            Message::Event(e) => {
                assert_eq!(e.outcome, Outcome::Fail);
                assert_eq!(e.subject.parent.as_deref(), Some("users.email"));
                let m = e.metrics.unwrap();
                assert_eq!(m.total, Some(1000));
                assert_eq!(m.failed, Some(7));
                assert_eq!(e.failures.len(), 2);
                assert_eq!(e.line, Some(17));
            }
            _ => panic!("wrong variant"),
        }
    }

    #[test]
    fn parses_summary_with_counts() {
        let m = parse(
            r#"{"type":"summary","file":"f.R","duration_ms":42,
                "counts":{"pass":3,"fail":1,"skip":0,"xfail":0,"warn":0,"error":0}}"#,
        );
        match m {
            Message::Summary(s) => {
                assert_eq!(s.duration_ms, 42);
                assert_eq!(s.counts.pass, 3);
                assert_eq!(s.counts.fail, 1);
            }
            _ => panic!("wrong variant"),
        }
    }

    #[test]
    fn parses_summary_with_missing_counts_default_zero() {
        let m = parse(r#"{"type":"summary","file":"f.R","duration_ms":1,"counts":{"pass":2}}"#);
        match m {
            Message::Summary(s) => {
                assert_eq!(s.counts.pass, 2);
                assert_eq!(s.counts.fail, 0);
                assert_eq!(s.counts.xfail, 0);
            }
            _ => panic!("wrong variant"),
        }
    }

    #[test]
    fn parses_done() {
        match parse(r#"{"type":"done"}"#) {
            Message::Done => {}
            _ => panic!("wrong variant"),
        }
    }

    // ── §3.1: outcome taxonomy round-trips ─────────────────────────────────
    //
    // Every outcome in the six-value taxonomy must deserialize from its
    // snake_case wire name. If a runner companion (R or pytest) starts
    // emitting a new string, these tests are what will catch it before
    // reporters silently classify it as the wrong thing.

    fn event_with_outcome(outcome_str: &str) -> Event {
        let line = format!(
            r#"{{"type":"event","file":"f","outcome":"{outcome_str}",
                "subject":{{"kind":"function","name":"x"}}}}"#
        );
        match parse(&line) {
            Message::Event(e) => e,
            _ => panic!("not an event"),
        }
    }

    #[test]
    fn parses_all_six_outcomes() {
        assert_eq!(event_with_outcome("pass").outcome, Outcome::Pass);
        assert_eq!(event_with_outcome("fail").outcome, Outcome::Fail);
        assert_eq!(event_with_outcome("error").outcome, Outcome::Error);
        assert_eq!(event_with_outcome("skip").outcome, Outcome::Skip);
        assert_eq!(event_with_outcome("xfail").outcome, Outcome::Xfail);
        assert_eq!(event_with_outcome("warn").outcome, Outcome::Warn);
    }

    #[test]
    fn rejects_unknown_outcome() {
        // An unknown outcome string must not silently map to a known variant.
        // Runner companions are the source of truth; a typo there should
        // surface as a parse failure, not a misclassification.
        let line = r#"{"type":"event","file":"f","outcome":"erroneous",
                       "subject":{"kind":"function","name":"x"}}"#;
        assert!(serde_json::from_str::<Message>(line).is_err());
    }

    // ── Outcome::rank() display ordering ───────────────────────────────────
    //
    // rank() is the single source of truth used by every sorted view (TUI,
    // web, plain reporter). Locking the order here prevents drift where one
    // frontend shows "fail first" and another shows "error first".

    #[test]
    fn rank_orders_bad_outcomes_first() {
        assert_eq!(Outcome::Fail.rank(), 0);
        assert_eq!(Outcome::Error.rank(), 1);
        assert_eq!(Outcome::Warn.rank(), 2);
        assert_eq!(Outcome::Pass.rank(), 3);
        assert_eq!(Outcome::Skip.rank(), 4);
        assert_eq!(Outcome::Xfail.rank(), 5);
    }

    #[test]
    fn all_by_rank_is_sorted_by_rank() {
        let ordered = Outcome::all_by_rank();
        let ranks: Vec<u8> = ordered.iter().map(|o| o.rank()).collect();
        let mut sorted = ranks.clone();
        sorted.sort();
        assert_eq!(ranks, sorted);
        assert_eq!(ordered.len(), 6);
    }

    // ── Counts operations ──────────────────────────────────────────────────

    #[test]
    fn counts_bump_targets_correct_field() {
        let mut c = Counts::default();
        c.bump(Outcome::Pass);
        c.bump(Outcome::Pass);
        c.bump(Outcome::Fail);
        c.bump(Outcome::Error);
        c.bump(Outcome::Skip);
        c.bump(Outcome::Xfail);
        c.bump(Outcome::Warn);
        assert_eq!(c.pass, 2);
        assert_eq!(c.fail, 1);
        assert_eq!(c.error, 1);
        assert_eq!(c.skip, 1);
        assert_eq!(c.xfail, 1);
        assert_eq!(c.warn, 1);
    }

    #[test]
    fn counts_bad_rule() {
        // "bad" means fail > 0 || error > 0. Every reporter depends on this
        // identity; the exit code, the `max_fail` bookkeeping, and the TUI
        // "bad file" marker all branch off it.
        assert!(!Counts::default().bad());
        assert!(Counts { fail: 1, ..Default::default() }.bad());
        assert!(Counts { error: 1, ..Default::default() }.bad());
        assert!(Counts { fail: 1, error: 1, ..Default::default() }.bad());
        // warnings alone do not make a file bad.
        assert!(!Counts { warn: 5, ..Default::default() }.bad());
        // a passing file with skips and xfails is not bad.
        assert!(!Counts { pass: 10, skip: 2, xfail: 1, ..Default::default() }.bad());
    }

    #[test]
    fn counts_merge_sums_fields() {
        let mut a = Counts { pass: 1, fail: 2, ..Default::default() };
        let b = Counts { pass: 3, error: 4, warn: 1, ..Default::default() };
        a.merge(&b);
        assert_eq!(a.pass, 4);
        assert_eq!(a.fail, 2);
        assert_eq!(a.error, 4);
        assert_eq!(a.warn, 1);
    }

    #[test]
    fn counts_saturating_sub_clamps_at_zero() {
        let mut a = Counts { pass: 3, fail: 1, ..Default::default() };
        let b = Counts { pass: 5, fail: 1, error: 2, ..Default::default() };
        a.saturating_sub(&b);
        assert_eq!(a.pass, 0, "3 - 5 must clamp to 0, not underflow");
        assert_eq!(a.fail, 0);
        assert_eq!(a.error, 0, "0 - 2 must clamp to 0");
    }

    // ── FileStatus::from_counts decision tree ─────────────────────────────
    //
    // The decision tree is: cancelled wins over everything; then error > 0
    // or fail > 0 => Failed; then any pass/xfail/warn => Passed; then any
    // skip => Skipped; then Pending. Reporters (TUI, web, plain, JUnit) all
    // route through this single function, so it must not drift.

    #[test]
    fn filestatus_cancelled_wins() {
        let counts = Counts { pass: 3, ..Default::default() };
        assert_eq!(FileStatus::from_counts(&counts, true), FileStatus::Cancelled);
    }

    #[test]
    fn filestatus_error_and_fail_both_map_to_failed() {
        let only_error = Counts { error: 1, ..Default::default() };
        let only_fail = Counts { fail: 1, ..Default::default() };
        let both = Counts { fail: 1, error: 1, ..Default::default() };
        assert_eq!(FileStatus::from_counts(&only_error, false), FileStatus::Failed);
        assert_eq!(FileStatus::from_counts(&only_fail, false), FileStatus::Failed);
        assert_eq!(FileStatus::from_counts(&both, false), FileStatus::Failed);
    }

    #[test]
    fn filestatus_pass_xfail_warn_map_to_passed() {
        let pass_only = Counts { pass: 1, ..Default::default() };
        let xfail_only = Counts { xfail: 1, ..Default::default() };
        let warn_only = Counts { warn: 1, ..Default::default() };
        assert_eq!(FileStatus::from_counts(&pass_only, false), FileStatus::Passed);
        assert_eq!(FileStatus::from_counts(&xfail_only, false), FileStatus::Passed);
        assert_eq!(FileStatus::from_counts(&warn_only, false), FileStatus::Passed);
    }

    #[test]
    fn filestatus_skip_only_is_skipped() {
        let c = Counts { skip: 3, ..Default::default() };
        assert_eq!(FileStatus::from_counts(&c, false), FileStatus::Skipped);
    }

    #[test]
    fn filestatus_empty_is_pending() {
        assert_eq!(FileStatus::from_counts(&Counts::default(), false), FileStatus::Pending);
    }

    // ── tally_messages policy: events for counts, summary for duration ─────
    //
    // This is pinned in protocol.md as the reconciliation policy. JUnit
    // needs the `tests=N` attribute to equal the count of `<testcase>` rows,
    // which only works if counts come from events (not the summary).

    fn event_msg(outcome: Outcome) -> Message {
        Message::Event(Event {
            file: "f".into(),
            outcome,
            subject: Subject {
                kind: "function".into(),
                name: "x".into(),
                parent: None,
            },
            metrics: None,
            failures: Vec::new(),
            message: None,
            line: None,
            duration_ms: 5,
            corrections: Vec::new(),
        })
    }

    fn summary_msg(duration_ms: u64, counts: Counts) -> Message {
        Message::Summary(Summary {
            file: "f".into(),
            duration_ms,
            counts,
        })
    }

    #[test]
    fn tally_counts_come_from_events_not_summary() {
        // Events say 2 pass, 1 fail. Summary says 99 pass, 99 fail. The
        // tally must trust the events. Duration comes from the summary.
        let bogus_counts = Counts {
            pass: 99,
            fail: 99,
            ..Default::default()
        };
        let msgs = vec![
            event_msg(Outcome::Pass),
            event_msg(Outcome::Pass),
            event_msg(Outcome::Fail),
            summary_msg(42, bogus_counts),
            Message::Done,
        ];
        let tally = tally_messages(&msgs, false);
        assert_eq!(tally.counts.pass, 2);
        assert_eq!(tally.counts.fail, 1);
        assert_eq!(tally.duration_ms, 42);
        assert_eq!(tally.status, FileStatus::Failed);
        assert!(tally.bad);
        assert!(!tally.warned);
    }

    #[test]
    fn tally_accumulates_events_across_batches() {
        // Two separate events in the same file must accumulate, not
        // overwrite. Protocol spec §3.1: "Two events for the same file:
        // counts accumulate; summary replaces timing."
        let msgs = vec![
            event_msg(Outcome::Pass),
            event_msg(Outcome::Pass),
            event_msg(Outcome::Pass),
            summary_msg(10, Counts::default()),
            Message::Done,
        ];
        let tally = tally_messages(&msgs, false);
        assert_eq!(tally.counts.pass, 3);
    }

    #[test]
    fn tally_without_summary_has_zero_duration() {
        // Worker crashed before emitting its summary. Events still count,
        // but duration falls back to 0. No panic.
        let msgs = vec![event_msg(Outcome::Pass), event_msg(Outcome::Fail)];
        let tally = tally_messages(&msgs, false);
        assert_eq!(tally.counts.pass, 1);
        assert_eq!(tally.counts.fail, 1);
        assert_eq!(tally.duration_ms, 0);
        assert_eq!(tally.status, FileStatus::Failed);
    }

    #[test]
    fn tally_cancelled_wins_over_status() {
        let msgs = vec![event_msg(Outcome::Pass), event_msg(Outcome::Pass)];
        let tally = tally_messages(&msgs, true);
        assert_eq!(tally.status, FileStatus::Cancelled);
    }

    #[test]
    fn tally_warned_field_tracks_warn_count() {
        let msgs = vec![event_msg(Outcome::Pass), event_msg(Outcome::Warn)];
        let tally = tally_messages(&msgs, false);
        assert!(tally.warned);
        assert!(!tally.bad, "warn alone is not bad");
        assert_eq!(tally.status, FileStatus::Passed);
    }

    // ── collect_findings: partitions by outcome ────────────────────────────

    #[test]
    fn collect_findings_splits_failures_and_warnings() {
        let msgs = vec![
            event_msg(Outcome::Pass),
            event_msg(Outcome::Fail),
            event_msg(Outcome::Error),
            event_msg(Outcome::Warn),
            event_msg(Outcome::Skip),
            event_msg(Outcome::Xfail),
        ];
        let path = std::path::Path::new("f.R");
        let (failures, warnings) = collect_findings(&msgs, "f.R", path);
        assert_eq!(failures.len(), 2, "fail + error are failures");
        assert_eq!(warnings.len(), 1, "warn is a warning");
        assert!(failures.iter().any(|f| f.outcome == Outcome::Fail));
        assert!(failures.iter().any(|f| f.outcome == Outcome::Error));
        assert_eq!(warnings[0].outcome, Outcome::Warn);
    }

    // ── process_events: display-ready fields, anon fallback ────────────────

    #[test]
    fn process_events_falls_back_to_anon_for_empty_name() {
        let anon_event = Event {
            file: "f".into(),
            outcome: Outcome::Pass,
            subject: Subject {
                kind: "function".into(),
                name: String::new(),
                parent: None,
            },
            metrics: None,
            failures: Vec::new(),
            message: None,
            line: None,
            duration_ms: 0,
            corrections: Vec::new(),
        };
        let processed = process_events(&[Message::Event(anon_event)]);
        assert_eq!(processed.len(), 1);
        assert_eq!(processed[0].name, "<anon>");
    }

    #[test]
    fn process_events_is_bad_matches_fail_or_error() {
        let pe = |outcome| ProcessedEvent {
            name: "x".into(),
            outcome,
            message: String::new(),
            line: None,
            duration_ms: 0,
            corrections: Vec::new(),
        };
        assert!(pe(Outcome::Fail).is_bad());
        assert!(pe(Outcome::Error).is_bad());
        assert!(!pe(Outcome::Pass).is_bad());
        assert!(!pe(Outcome::Skip).is_bad());
        assert!(!pe(Outcome::Xfail).is_bad());
        assert!(!pe(Outcome::Warn).is_bad(), "warn is not bad");
    }

    // ── Metrics display_summary: percentage rendering ──────────────────────

    #[test]
    fn metrics_summary_uses_fraction_when_present() {
        let m = Metrics {
            total: Some(1000),
            failed: Some(7),
            fraction: Some(0.007),
            observed: None,
        };
        assert_eq!(m.display_summary().as_deref(), Some("7 of 1000 failed (0.70%)"));
    }

    #[test]
    fn metrics_summary_computes_fraction_when_missing() {
        let m = Metrics {
            total: Some(200),
            failed: Some(1),
            fraction: None,
            observed: None,
        };
        assert_eq!(m.display_summary().as_deref(), Some("1 of 200 failed (0.50%)"));
    }

    #[test]
    fn metrics_summary_handles_zero_total() {
        let m = Metrics {
            total: Some(0),
            failed: Some(0),
            fraction: None,
            observed: None,
        };
        // Must not divide by zero.
        assert_eq!(m.display_summary().as_deref(), Some("0 of 0 failed (0.00%)"));
    }

    #[test]
    fn metrics_summary_is_none_when_nothing_quantitative() {
        let m = Metrics::default();
        assert!(m.display_summary().is_none(),
            "display_summary must return None when no counts; otherwise \
             reporters produce empty leading lines");
    }
}
