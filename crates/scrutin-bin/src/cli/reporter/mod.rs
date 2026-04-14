//! Reporter infrastructure: shared types and engine-driving helpers used by
//! the plain and github reporters. Each reporter lives in its own submodule
//! and exposes a top-level `run()` async function called from `cli.rs`.

pub mod github;
pub mod plain;

use std::collections::HashSet;
use std::time::Duration;

use scrutin_core::engine::protocol::{self as proto, Finding, Message};

// ---------------------------------------------------------------------------
// Shared types
// ---------------------------------------------------------------------------

/// One file's accumulated state. Carries the messages plus the engine-side
/// `cancelled` flag (which is *not* on the wire).
pub struct FileRecord {
    pub file: String,
    pub messages: Vec<Message>,
    pub cancelled: bool,
}

/// Outcome of one engine invocation: count of *files* that had any failure
/// or error (matches `--max-fail` semantics), the merged results, and wall
/// time. Plain mode threads this through the rerun loop.
pub type RunOutcome = (u32, Vec<FileRecord>, Duration);

/// Per-file tally for the plain reporter. Wraps the shared
/// `protocol::FileTally` with the `cancelled` flag.
#[derive(Default, Clone)]
#[allow(dead_code)]
pub struct FileTally {
    pub passed: u32,
    pub failed: u32,
    pub errored: u32,
    pub warned: u32,
    pub skipped: u32,
    pub xfailed: u32,
    pub bad_file: bool,
    pub cancelled: bool,
}

impl From<&proto::FileTally> for FileTally {
    fn from(t: &proto::FileTally) -> Self {
        FileTally {
            passed: t.counts.pass,
            failed: t.counts.fail,
            errored: t.counts.error,
            warned: t.counts.warn,
            skipped: t.counts.skip,
            xfailed: t.counts.xfail,
            bad_file: t.bad,
            cancelled: matches!(t.status, proto::FileStatus::Cancelled),
        }
    }
}

/// Aggregate run statistics, fed to `print_summary` / `gha_print_summary`.
pub struct RunStats {
    pub passed: u32,
    pub failed: u32,
    pub errored: u32,
    pub warned: u32,
    pub skipped: u32,
    pub n_files: u32,
    pub elapsed: Duration,
    pub slowest: Option<(String, u64)>,
}

// ---------------------------------------------------------------------------
// RunAccumulator
// ---------------------------------------------------------------------------

#[derive(Default)]
pub struct RunAccumulator {
    pub totals: FileTally,
    /// Number of *files* with at least one failure or error. Distinct from
    /// `totals.failed`/`totals.errored`, which count individual expectations.
    pub failed_files: u32,
    pub file_durations: Vec<(String, u64)>,
    /// Per-file tallies buffered for end-of-run rendering.
    pub file_tallies: Vec<(String, FileTally, u64)>,
    pub failed_details: Vec<Finding>,
    pub warn_details: Vec<Finding>,
    pub all_results: Vec<FileRecord>,
}

impl RunAccumulator {
    pub fn push(
        &mut self,
        file_name: String,
        messages: Vec<Message>,
        cancelled: bool,
    ) {
        let (t, file_ms) = tally_messages(
            &messages,
            &file_name,
            cancelled,
            &mut self.failed_details,
            &mut self.warn_details,
        );
        self.totals.passed += t.passed;
        self.totals.failed += t.failed;
        self.totals.errored += t.errored;
        self.totals.warned += t.warned;
        self.totals.skipped += t.skipped;
        self.totals.xfailed += t.xfailed;
        if t.bad_file {
            self.failed_files += 1;
        }

        self.file_durations.push((file_name.clone(), file_ms));
        self.file_tallies.push((file_name.clone(), t, file_ms));
        self.all_results.push(FileRecord {
            file: file_name,
            messages,
            cancelled,
        });
    }

    /// Re-build an accumulator from a previously-produced result set. Used
    /// after the rerun loop merges in retried files.
    pub fn from_results(all_results: &[FileRecord]) -> Self {
        let mut acc = RunAccumulator::default();
        for rec in all_results {
            let (t, file_ms) = tally_messages(
                &rec.messages,
                &rec.file,
                rec.cancelled,
                &mut acc.failed_details,
                &mut acc.warn_details,
            );
            acc.totals.passed += t.passed;
            acc.totals.failed += t.failed;
            acc.totals.errored += t.errored;
            acc.totals.warned += t.warned;
            acc.totals.skipped += t.skipped;
            acc.totals.xfailed += t.xfailed;
            if t.bad_file {
                acc.failed_files += 1;
            }
            acc.file_durations.push((rec.file.clone(), file_ms));
            acc.file_tallies.push((rec.file.clone(), t, file_ms));
        }
        acc
    }

    pub fn into_stats(self, elapsed: Duration) -> RunStats {
        let n_files = self.file_durations.len() as u32;
        RunStats {
            passed: self.totals.passed,
            failed: self.totals.failed,
            errored: self.totals.errored,
            warned: self.totals.warned,
            skipped: self.totals.skipped,
            n_files,
            elapsed,
            slowest: self.file_durations.into_iter().max_by_key(|(_, ms)| *ms),
        }
    }
}

// ---------------------------------------------------------------------------
// Shared helpers
// ---------------------------------------------------------------------------

/// Walk one file's messages, tally outcomes and collect findings via
/// shared core functions.
pub fn tally_messages(
    messages: &[Message],
    file_name: &str,
    cancelled: bool,
    failed_details: &mut Vec<Finding>,
    warn_details: &mut Vec<Finding>,
) -> (FileTally, u64) {
    let core_tally = proto::tally_messages(messages, cancelled);
    let (failures, warnings) = proto::collect_findings(messages, file_name, std::path::Path::new(""));
    failed_details.extend(failures);
    warn_details.extend(warnings);

    let t = FileTally::from(&core_tally);
    (t, core_tally.duration_ms)
}

/// Names of files (basename) that contain at least one failure or error
/// event. Cancelled files are *not* counted as failed.
pub fn collect_failed_files(all_results: &[FileRecord]) -> HashSet<String> {
    use proto::Outcome;
    let mut out = HashSet::new();
    for rec in all_results {
        if rec.cancelled {
            continue;
        }
        let bad = rec.messages.iter().any(|m| matches!(
            m,
            Message::Event(e) if matches!(e.outcome, Outcome::Fail | Outcome::Error)
        ));
        if bad {
            out.insert(rec.file.clone());
        }
    }
    out
}

/// Replace any entries in `all_results` whose file name appears in
/// `replacements`.
pub fn replace_results(
    all_results: &mut Vec<FileRecord>,
    replacements: Vec<FileRecord>,
) {
    let index: std::collections::HashMap<&str, usize> = all_results
        .iter()
        .enumerate()
        .map(|(i, r)| (r.file.as_str(), i))
        .collect();
    let plan: Vec<Option<usize>> = replacements
        .iter()
        .map(|r| index.get(r.file.as_str()).copied())
        .collect();
    drop(index);
    for (rec, slot) in replacements.into_iter().zip(plan) {
        match slot {
            Some(i) => all_results[i] = rec,
            None => all_results.push(rec),
        }
    }
}

/// Merge runtime dependency observations into an in-memory dep map and,
/// opportunistically, persist them into the SQLite `dependencies` table.
/// The in-memory map is the source of truth for the rest of this run's
/// watch-mode decisions; the DB is updated so the *next* invocation sees
/// the same edges without a full rebuild.
pub fn merge_deps_from_results(
    dep_map: &mut std::collections::HashMap<String, Vec<String>>,
    results: &[FileRecord],
    root: &std::path::Path,
) {
    for rec in results {
        for msg in &rec.messages {
            if let Message::Deps(d) = msg {
                merge_deps_inmem(dep_map, &d.file, &d.sources);
                let _ = scrutin_core::storage::sqlite::with_open(root, |c| {
                    scrutin_core::storage::sqlite::merge_deps_for_test(c, &d.file, &d.sources)
                });
            }
        }
    }
}

/// In-memory mirror of the old `json_cache::merge_deps`. The test file
/// claims every source in `sources`; any previous edge for a source *not*
/// in `sources` has `test_file` removed.
fn merge_deps_inmem(
    map: &mut std::collections::HashMap<String, Vec<String>>,
    test_file: &str,
    sources: &[String],
) {
    let source_set: std::collections::HashSet<&str> =
        sources.iter().map(|s| s.as_str()).collect();
    for (src, tests) in map.iter_mut() {
        if !source_set.contains(src.as_str()) {
            tests.retain(|t| t != test_file);
        }
    }
    for src in sources {
        let entry = map.entry(src.clone()).or_default();
        if !entry.contains(&test_file.to_string()) {
            entry.push(test_file.to_string());
            entry.sort();
        }
    }
}

pub fn rebuild_depmap_in_background(pkg: &scrutin_core::project::package::Package) {
    use super::style;
    eprintln!("{}", style::dim("Rebuilding dependency map in background..."));
    let map = scrutin_core::analysis::deps::build_unified_dep_map(pkg);
    if map.is_empty() {
        return;
    }
    let _ = scrutin_core::storage::sqlite::with_open(&pkg.root, |c| {
        scrutin_core::storage::sqlite::replace_dep_map(c, &map)
    });
    let _ = scrutin_core::analysis::hashing::snapshot_hashes(pkg);
    eprintln!(
        "{}",
        style::dim(format!("Dependency map updated: {} source files.", map.len()))
    );
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    fn msg(line: &str) -> Message {
        serde_json::from_str(line).expect("parse test message")
    }

    fn ev(json: &str) -> Message {
        msg(json)
    }

    // ----- tally_messages -----

    #[test]
    fn tally_six_outcome_taxonomy() {
        let msgs = vec![
            ev(r#"{"type":"event","file":"f","outcome":"pass","subject":{"kind":"function","name":"a"}}"#),
            ev(r#"{"type":"event","file":"f","outcome":"fail","subject":{"kind":"function","name":"b"},"message":"oops","line":7}"#),
            ev(r#"{"type":"event","file":"f","outcome":"warn","subject":{"kind":"function","name":"c"},"message":"deprecated","line":3}"#),
            ev(r#"{"type":"event","file":"f","outcome":"error","subject":{"kind":"function","name":"<err>"},"message":"crashed","line":1}"#),
            ev(r#"{"type":"event","file":"f","outcome":"skip","subject":{"kind":"function","name":"d"}}"#),
            ev(r#"{"type":"event","file":"f","outcome":"xfail","subject":{"kind":"function","name":"e"}}"#),
            ev(r#"{"type":"summary","file":"f","duration_ms":42,"counts":{"pass":99,"fail":99,"error":99,"skip":99,"xfail":99,"warn":99}}"#),
            msg(r#"{"type":"done"}"#),
        ];
        let mut failed = Vec::new();
        let mut warned = Vec::new();
        let (t, ms) = tally_messages(&msgs, "f", false, &mut failed, &mut warned);
        assert_eq!(t.passed, 1);
        assert_eq!(t.failed, 1);
        assert_eq!(t.errored, 1);
        assert_eq!(t.warned, 1);
        assert_eq!(t.skipped, 1);
        assert_eq!(t.xfailed, 1);
        assert!(t.bad_file, "any fail or error must mark file bad");
        assert!(
            !t.cancelled,
            "cancelled flag must come from the engine, not from messages"
        );
        assert_eq!(ms, 42);
        assert_eq!(failed.len(), 2, "fail event + error event");
        assert_eq!(warned.len(), 1);
        assert!(failed.iter().any(|f| f.test == "<err>" && f.line == Some(1)));
    }

    #[test]
    fn tally_xfail_and_warn_do_not_set_bad_file() {
        let msgs = vec![
            ev(r#"{"type":"event","file":"f","outcome":"xfail","subject":{"kind":"function","name":"a"}}"#),
            ev(r#"{"type":"event","file":"f","outcome":"warn","subject":{"kind":"function","name":"b"}}"#),
        ];
        let (t, _) = tally_messages(&msgs, "f", false, &mut Vec::new(), &mut Vec::new());
        assert_eq!(t.xfailed, 1);
        assert_eq!(t.warned, 1);
        assert!(!t.bad_file);
    }

    #[test]
    fn tally_cancelled_flag_passes_through() {
        let msgs: Vec<Message> = vec![];
        let (t, _) = tally_messages(&msgs, "f", true, &mut Vec::new(), &mut Vec::new());
        assert!(t.cancelled);
        assert_eq!(t.skipped, 0);
        assert!(!t.bad_file);
    }

    #[test]
    fn tally_events_authoritative_over_summary_counts() {
        let msgs = vec![
            ev(r#"{"type":"event","file":"f","outcome":"pass","subject":{"kind":"function","name":"a"}}"#),
            ev(r#"{"type":"summary","file":"f","duration_ms":42,"counts":{"pass":5}}"#),
        ];
        let (t, ms) = tally_messages(&msgs, "f", false, &mut Vec::new(), &mut Vec::new());
        assert_eq!(t.passed, 1, "events authoritative, not summary counts");
        assert_eq!(ms, 42, "summary still provides wall time");
    }

    // ----- replace_results / accumulator parity -----

    fn pass_event(file: &str, test: &str) -> Vec<Message> {
        vec![msg(&format!(
            r#"{{"type":"event","file":"{file}","outcome":"pass","subject":{{"kind":"function","name":"{test}"}}}}"#
        ))]
    }
    fn fail_event(file: &str, test: &str) -> Vec<Message> {
        vec![msg(&format!(
            r#"{{"type":"event","file":"{file}","outcome":"fail","subject":{{"kind":"function","name":"{test}"}}}}"#
        ))]
    }
    fn rec(file: &str, msgs: Vec<Message>) -> FileRecord {
        FileRecord {
            file: file.into(),
            messages: msgs,
            cancelled: false,
        }
    }

    #[test]
    fn replace_results_in_place() {
        let mut all = vec![
            rec("a.R", fail_event("a.R", "t1")),
            rec("b.R", pass_event("b.R", "t1")),
        ];
        replace_results(&mut all, vec![rec("a.R", pass_event("a.R", "t1"))]);
        assert_eq!(all.len(), 2, "no append on replace");
        let acc = RunAccumulator::from_results(&all);
        assert_eq!(acc.failed_files, 0);
        assert_eq!(acc.totals.passed, 2);
    }

    #[test]
    fn replace_results_appends_unknown() {
        let mut all = vec![rec("a.R", pass_event("a.R", "t"))];
        replace_results(&mut all, vec![rec("b.R", fail_event("b.R", "t"))]);
        assert_eq!(all.len(), 2);
    }

    #[test]
    fn from_results_matches_live_push() {
        let files = vec![
            rec("a.R", pass_event("a.R", "t1")),
            rec("b.R", fail_event("b.R", "t1")),
        ];
        let acc = RunAccumulator::from_results(&files);
        assert_eq!(acc.totals.passed, 1);
        assert_eq!(acc.totals.failed, 1);
        assert_eq!(acc.failed_files, 1);
    }
}
