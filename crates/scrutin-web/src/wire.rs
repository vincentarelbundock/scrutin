//! Stable wire types for the browser frontend.
//!
//! These are **not** the same types as `scrutin-core::engine::protocol`:
//! the core protocol is the R/Python worker ↔ engine contract, the wire
//! types are the server ↔ browser contract. Both evolve independently;
//! translation happens in [`crate::state`].
//!
//! The six-outcome taxonomy (pass/fail/error/skip/xfail/warn) is preserved
//! verbatim per `docs/reporting-spec.md`. `bad_file = failed > 0 || errored > 0`
//! is computed server-side and never recomputed in the browser.

use std::path::{Path, PathBuf};

use scrutin_core::engine::protocol::{Event as CoreEvent, Message as CoreMessage, Outcome};
use serde::{Deserialize, Serialize};

/// Stable file identifier: an xxhash64 of the path relative to the project
/// root, hex-encoded. Paths are display-only — the id is what crosses the
/// wire and what clients store in maps / URLs.
///
/// Serialized as a hex **string** (not a bare u64) so browser-side maps,
/// URL paths, and JSON document keys work naturally.
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub struct FileId(pub u64);

impl FileId {
    pub fn of(rel_path: &Path) -> Self {
        use xxhash_rust::xxh64::xxh64;
        let bytes = rel_path.to_string_lossy();
        FileId(xxh64(bytes.as_bytes(), 0))
    }
    pub fn as_hex(self) -> String {
        format!("{:016x}", self.0)
    }
}

impl std::fmt::Display for FileId {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{:016x}", self.0)
    }
}

impl std::str::FromStr for FileId {
    type Err = std::num::ParseIntError;
    fn from_str(s: &str) -> Result<Self, Self::Err> {
        u64::from_str_radix(s, 16).map(FileId)
    }
}

impl serde::Serialize for FileId {
    fn serialize<S: serde::Serializer>(&self, s: S) -> Result<S::Ok, S::Error> {
        s.serialize_str(&self.as_hex())
    }
}

impl<'de> serde::Deserialize<'de> for FileId {
    fn deserialize<D: serde::Deserializer<'de>>(d: D) -> Result<Self, D::Error> {
        let s = <String as serde::Deserialize>::deserialize(d)?;
        s.parse().map_err(serde::de::Error::custom)
    }
}

/// Stable run identifier. A UUID v4 per run.
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq, Serialize, Deserialize)]
pub struct RunId(pub uuid::Uuid);

impl RunId {
    pub fn new() -> Self {
        RunId(uuid::Uuid::new_v4())
    }
}

impl std::fmt::Display for RunId {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.0)
    }
}

impl Default for RunId {
    fn default() -> Self {
        Self::new()
    }
}

// ── Outcome + status ────────────────────────────────────────────────────────

/// Mirrors the six-outcome taxonomy. Serialized as snake_case strings.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum WireOutcome {
    Pass,
    Fail,
    Error,
    Skip,
    Xfail,
    Warn,
}

impl From<Outcome> for WireOutcome {
    fn from(o: Outcome) -> Self {
        match o {
            Outcome::Pass => Self::Pass,
            Outcome::Fail => Self::Fail,
            Outcome::Error => Self::Error,
            Outcome::Skip => Self::Skip,
            Outcome::Xfail => Self::Xfail,
            Outcome::Warn => Self::Warn,
        }
    }
}

/// File-level status: the aggregate of a file's events. `Running` during
/// the run, terminal states afterwards. `Unknown` before the file has
/// ever been touched by a run.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum WireStatus {
    Unknown,
    Pending,
    Running,
    Passed,
    Failed,
    Errored,
    Skipped,
    Cancelled,
}

/// Per-outcome counts. Re-exported from core so the wire layer doesn't
/// maintain a parallel struct.
pub type WireCounts = scrutin_core::engine::protocol::Counts;

// ── Files + messages ────────────────────────────────────────────────────────

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct WireLocation {
    pub file: String,
    pub line: Option<u32>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct WireMessage {
    pub outcome: WireOutcome,
    pub test_name: Option<String>,
    pub subject_kind: Option<String>,
    pub subject_parent: Option<String>,
    pub location: Option<WireLocation>,
    pub message: Option<String>,
    pub duration_ms: u64,
    /// Quantitative metrics from data-validation plugins. Optional; `None`
    /// for unit-test events.
    pub metrics: Option<WireMetrics>,
    /// Spell-check corrections (skyspell). Empty for every other plugin.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub corrections: Vec<WireCorrection>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct WireMetrics {
    pub total: Option<u64>,
    pub failed: Option<u64>,
    pub fraction: Option<f64>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct WireCorrection {
    pub word: String,
    pub line: u32,
    pub col_start: u32,
    pub col_end: u32,
    pub suggestions: Vec<String>,
}

impl From<&CoreEvent> for WireMessage {
    fn from(e: &CoreEvent) -> Self {
        let test_name = if e.subject.name.is_empty() {
            None
        } else {
            Some(e.subject.name.clone())
        };
        let location = e.line.map(|l| WireLocation {
            file: e.file.clone(),
            line: Some(l),
        });
        let metrics = e.metrics.as_ref().map(|m| WireMetrics {
            total: m.total,
            failed: m.failed,
            fraction: m.fraction,
        });
        let corrections = e
            .corrections
            .iter()
            .map(|c| WireCorrection {
                word: c.word.clone(),
                line: c.line,
                col_start: c.col_start,
                col_end: c.col_end,
                suggestions: c.suggestions.clone(),
            })
            .collect();
        Self {
            outcome: e.outcome.into(),
            test_name,
            subject_kind: Some(e.subject.kind.clone()),
            subject_parent: e.subject.parent.clone(),
            location,
            message: e.message.clone(),
            duration_ms: e.duration_ms,
            metrics,
            corrections,
        }
    }
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct WireFile {
    pub id: FileId,
    /// Relative path (to project root) with forward slashes for display.
    pub path: String,
    /// Display basename.
    pub name: String,
    /// Owning suite name (e.g. `"testthat"`, `"pytest"`, `"pointblank"`).
    pub suite: String,
    pub status: WireStatus,
    pub last_duration_ms: Option<u64>,
    pub last_run_id: Option<RunId>,
    pub counts: WireCounts,
    #[serde(default)]
    pub messages: Vec<WireMessage>,
    /// Convenience flag: true iff `counts.bad()`. Computed server-side.
    pub bad: bool,
}

impl WireFile {
    pub fn new(abs_path: &Path, root: &Path, suite: String) -> Self {
        let rel: PathBuf = abs_path
            .strip_prefix(root)
            .unwrap_or(abs_path)
            .to_path_buf();
        let path = rel.to_string_lossy().replace('\\', "/");
        let name = abs_path
            .file_name()
            .map(|s| s.to_string_lossy().to_string())
            .unwrap_or_else(|| path.clone());
        WireFile {
            id: FileId::of(&rel),
            path,
            name,
            suite,
            status: WireStatus::Unknown,
            last_duration_ms: None,
            last_run_id: None,
            counts: WireCounts::default(),
            messages: Vec::new(),
            bad: false,
        }
    }
}

// ── Suites + package ────────────────────────────────────────────────────────

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct WireSuiteAction {
    pub name: String,
    pub key: String,
    pub label: String,
    /// "file" or "all".
    pub scope: String,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct WireSuite {
    pub name: String,
    pub language: String,
    pub test_dirs: Vec<String>,
    pub source_dir: Option<String>,
    pub file_count: usize,
    #[serde(default)]
    pub actions: Vec<WireSuiteAction>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct WirePackage {
    pub name: String,
    pub root: String,
    pub tool: String,
    pub suites: Vec<WireSuite>,
}

// ── Run summaries + events ──────────────────────────────────────────────────

/// Totals for an in-progress or completed run, plus a list of bad files.
#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct WireRunSummary {
    pub run_id: Option<RunId>,
    pub started_at: Option<String>,   // ISO-8601
    pub finished_at: Option<String>,  // ISO-8601
    pub in_progress: bool,
    pub totals: WireCounts,
    pub bad_files: Vec<FileId>,
    /// Number of worker subprocesses busy *right now*. Mirrors the TUI's
    /// `busy/total workers` indicator. Zero when idle.
    pub busy: u32,
}

/// The snapshot returned by `GET /api/snapshot`: everything a fresh client
/// needs to render the whole UI.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct WireSnapshot {
    pub pkg: WirePackage,
    pub files: Vec<WireFile>,
    pub current_run: WireRunSummary,
    pub watching: bool,
    pub n_workers: usize,
    pub keymap: serde_json::Value,
    /// Outcome order for status-sorted views, serialized as snake_case
    /// strings. Derived from `Outcome::rank()` in scrutin-core so the web
    /// frontend never hard-codes its own copy.
    pub outcome_order: Vec<WireOutcome>,
}

/// Events fanned out over SSE. Kept as a flat enum so the client can
/// switch on `kind` without juggling nested types.
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum WireEvent {
    RunStarted {
        run_id: RunId,
        started_at: String,
        files: Vec<FileId>,
    },
    FileStarted {
        run_id: RunId,
        file_id: FileId,
    },
    FileFinished {
        run_id: RunId,
        file: WireFile,
    },
    RunComplete {
        run_id: RunId,
        finished_at: String,
        totals: WireCounts,
        bad_files: Vec<FileId>,
    },
    RunCancelled {
        run_id: RunId,
        reason: String,
    },
    WatcherTriggered {
        changed_files: Vec<String>,
        will_rerun: Vec<FileId>,
    },
    Log {
        level: String,
        message: String,
        ts: String,
    },
    Heartbeat {
        ts: String,
        /// Busy worker count at the time of the tick. Zero when idle.
        busy: u32,
        in_progress: bool,
    },
}

impl WireEvent {
    pub fn kind_str(&self) -> &'static str {
        match self {
            Self::RunStarted { .. } => "run_started",
            Self::FileStarted { .. } => "file_started",
            Self::FileFinished { .. } => "file_finished",
            Self::RunComplete { .. } => "run_complete",
            Self::RunCancelled { .. } => "run_cancelled",
            Self::WatcherTriggered { .. } => "watcher_triggered",
            Self::Log { .. } => "log",
            Self::Heartbeat { .. } => "heartbeat",
        }
    }
}

/// Tally a file's messages into wire types. Delegates counting and status
/// to `protocol::tally_messages` (the shared implementation) and builds
/// the web-specific `Vec<WireMessage>` alongside it.
pub fn tally_file_messages(messages: &[CoreMessage], cancelled: bool) -> (WireCounts, Vec<WireMessage>, u64, WireStatus, bool) {
    use scrutin_core::engine::protocol::tally_messages;

    let tally = tally_messages(messages, cancelled);

    // Build per-event wire messages (web-specific).
    let wire_msgs: Vec<WireMessage> = messages
        .iter()
        .filter_map(|m| match m {
            CoreMessage::Event(e) => Some(e.into()),
            _ => None,
        })
        .collect();

    let status = wire_status(tally.status, &tally.counts);

    (tally.counts, wire_msgs, tally.duration_ms, status, tally.bad)
}


/// Convert core `FileStatus` to `WireStatus`. The web frontend
/// distinguishes `errored` (error > 0) from `failed` (fail > 0, error == 0),
/// so we need counts to disambiguate `FileStatus::Failed`.
pub fn wire_status(status: scrutin_core::engine::protocol::FileStatus, counts: &WireCounts) -> WireStatus {
    use scrutin_core::engine::protocol::FileStatus as FS;
    match status {
        FS::Cancelled => WireStatus::Cancelled,
        FS::Failed => {
            if counts.error > 0 {
                WireStatus::Errored
            } else {
                WireStatus::Failed
            }
        }
        FS::Passed => WireStatus::Passed,
        FS::Skipped => WireStatus::Skipped,
        FS::Pending => WireStatus::Pending,
    }
}
