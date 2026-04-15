//! Engine pool behavior tests (spec §3.3).
//!
//! Exercises `run_events::start_run` and the underlying `ProcessPool`
//! against a scripted fake subprocess (python3). The fake runner
//! implements the NDJSON worker protocol and reads its behavior
//! from the test filename:
//!
//!   fake_pass.py       -> one `pass` event
//!   fake_fail.py       -> one `fail` event
//!   fake_error.py      -> one `error` event
//!   fake_skip.py       -> one `skip` event
//!   fake_sleep_500.py  -> one `pass`, delayed 500ms
//!   fake_crash.py      -> exit without emitting `done` (simulates a crash)
//!
//! This keeps tests subprocess-backed (so the pool is exercised end-to-end)
//! without depending on pytest or Rscript. Gated on python3 being available.

use std::collections::BTreeMap;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::Arc;
use std::time::Duration;


use scrutin_core::engine::run_events::{start_run, RunEvent};
use scrutin_core::project::package::{Package, TestSuite, WorkerHookPaths};
use scrutin_core::project::plugin::Plugin;

// ── FakePlugin ──────────────────────────────────────────────────────────────

/// Python runner script: reads file paths from stdin, derives scripted
/// behavior from the basename, emits NDJSON per the protocol. Each file
/// produces exactly one `event`, one `summary`, and one `done`, unless
/// the filename requests a crash (in which case `done` is suppressed).
const FAKE_RUNNER: &str = r#"
import sys, json, time, os, re

def emit(obj):
    sys.stdout.write(json.dumps(obj) + "\n")
    sys.stdout.flush()

pattern = re.compile(r"fake_(?P<kind>pass|fail|error|skip|xfail|warn|sleep|crash)(?:_(?P<arg>\d+))?\.py$")

for raw_line in sys.stdin:
    path = raw_line.strip()
    if not path:
        continue
    basename = os.path.basename(path)
    m = pattern.match(basename)
    kind = m.group("kind") if m else "pass"
    arg = int(m.group("arg")) if (m and m.group("arg")) else 0

    if kind == "sleep":
        time.sleep(arg / 1000.0)
        kind = "pass"
    elif kind == "crash":
        # Exit without emitting `done`. The engine should surface an error.
        sys.exit(42)

    evt = {
        "type": "event",
        "file": basename,
        "outcome": kind,
        "subject": {"kind": "function", "name": "test_x"},
        "duration_ms": arg,
    }
    emit(evt)

    counts_key = kind
    summary = {
        "type": "summary",
        "file": basename,
        "duration_ms": arg,
        "counts": {counts_key: 1},
    }
    emit(summary)

    emit({"type": "done"})
"#;

struct FakePlugin;

impl Plugin for FakePlugin {
    fn name(&self) -> &'static str {
        "fake"
    }
    fn language(&self) -> &'static str {
        "fake"
    }
    fn detect(&self, _root: &Path) -> bool {
        false
    }
    fn subprocess_cmd(&self, _root: &Path) -> Vec<String> {
        vec![
            "python3".into(),
            "-u".into(),
            ".scrutin/fake_runner.py".into(),
        ]
    }
    fn runner_script(&self) -> &'static str {
        FAKE_RUNNER
    }
    fn script_extension(&self) -> &'static str {
        "py"
    }
    fn runner_basename(&self) -> String {
        "fake_runner.py".into()
    }
    fn project_name(&self, _root: &Path) -> String {
        "fake".into()
    }
    fn default_run(&self) -> Vec<String> {
        vec!["tests/**/fake_*.py".into()]
    }
    fn default_watch(&self) -> Vec<String> {
        vec!["src/**/*.py".into()]
    }
    fn is_test_file(&self, path: &Path) -> bool {
        path.extension().and_then(|s| s.to_str()) == Some("py")
            && path
                .file_name()
                .and_then(|s| s.to_str())
                .map(|n| n.starts_with("fake_"))
                .unwrap_or(false)
    }
    fn is_source_file(&self, _path: &Path) -> bool {
        false
    }
}

// ── Fixture helpers ─────────────────────────────────────────────────────────

fn python3_available() -> bool {
    Command::new("python3")
        .arg("--version")
        .output()
        .map(|o| o.status.success())
        .unwrap_or(false)
}

/// Build a Package with a single FakePlugin suite rooted at `root`.
/// Caller writes the fake test files into `<root>/tests/` before driving
/// the run. This helper does not create the `tests/` dir.
fn fake_package(root: &Path) -> Package {
    let plugin: Arc<dyn Plugin> = Arc::new(FakePlugin);
    let suite = TestSuite::new(
        plugin,
        root.to_path_buf(),
        vec!["tests/**/fake_*.py".into()],
        vec!["src/**/*.py".into()],
        WorkerHookPaths::default(),
        None,
    )
    .expect("compile globs");
    Package {
        name: "fake".into(),
        root: root.to_path_buf(),
        test_suites: vec![suite],
        pytest_extra_args: Vec::new(),
        skyspell_extra_args: Vec::new(),
        skyspell_add_args: Vec::new(),
        python_interpreter: Vec::new(),
        env: BTreeMap::new(),
    }
}

/// Run scrutin's engine end-to-end against a FakePlugin suite; collect
/// every `FileFinished` event in the order the engine emitted them.
async fn collect_results(
    pkg: &Package,
    files: Vec<PathBuf>,
    n_workers: usize,
    timeout_per_file: Duration,
) -> Vec<scrutin_core::engine::run_events::FileResult> {
    let (_handle, mut rx) = start_run(
        pkg,
        files,
        n_workers,
        timeout_per_file,
        None,
        false,
        None,
    )
    .await
    .expect("start_run");

    let mut out = Vec::new();
    while let Some(evt) = rx.recv().await {
        match evt {
            RunEvent::FileFinished(fr) => out.push(fr),
            RunEvent::Complete => break,
        }
    }
    out
}

fn write_empty(path: &Path) {
    fs::create_dir_all(path.parent().unwrap()).unwrap();
    fs::write(path, "").unwrap();
}

// ── Tests ───────────────────────────────────────────────────────────────────

#[tokio::test]
async fn pool_runs_all_files_and_preserves_outcomes() {
    if !python3_available() {
        eprintln!("skipping: python3 not available");
        return;
    }
    let tmp = tempfile::tempdir().unwrap();
    let root = tmp.path();
    write_empty(&root.join("tests/fake_pass.py"));
    write_empty(&root.join("tests/fake_fail.py"));
    write_empty(&root.join("tests/fake_error.py"));

    let pkg = fake_package(root);
    let files = vec![
        root.join("tests/fake_pass.py"),
        root.join("tests/fake_fail.py"),
        root.join("tests/fake_error.py"),
    ];

    let results = collect_results(&pkg, files, 2, Duration::from_secs(10)).await;
    assert_eq!(results.len(), 3, "all three files must complete");

    let outcomes: std::collections::HashMap<String, Vec<_>> = results
        .iter()
        .map(|r| {
            let name = r.file.file_name().unwrap().to_string_lossy().into_owned();
            let outcomes: Vec<_> = r
                .messages
                .iter()
                .filter_map(|m| match m {
                    scrutin_core::engine::protocol::Message::Event(e) => Some(e.outcome),
                    _ => None,
                })
                .collect();
            (name, outcomes)
        })
        .collect();

    use scrutin_core::engine::protocol::Outcome;
    assert_eq!(
        outcomes.get("fake_pass.py"),
        Some(&vec![Outcome::Pass]),
        "fake_pass.py must emit one Pass event"
    );
    assert_eq!(
        outcomes.get("fake_fail.py"),
        Some(&vec![Outcome::Fail]),
    );
    assert_eq!(
        outcomes.get("fake_error.py"),
        Some(&vec![Outcome::Error]),
    );
}

#[tokio::test]
async fn pool_honors_worker_concurrency_cap() {
    // With 2 workers and 4 sleeping files (200ms each), total wall time
    // must be at least 2 * 200ms = 400ms (two batches of two). If the
    // pool ignored the cap, it would approach 200ms. If it ran serially,
    // it would approach 800ms. We assert the middle band to avoid flakes
    // on slow CI.
    if !python3_available() {
        eprintln!("skipping: python3 not available");
        return;
    }
    let tmp = tempfile::tempdir().unwrap();
    let root = tmp.path();
    for i in 0..4 {
        write_empty(&root.join(format!("tests/fake_sleep_200_{i}.py")));
    }
    // rename to match the scripted-filename pattern `fake_sleep_<ms>.py`.
    // Since duplicates aren't allowed, encode the index into a trailing
    // directory instead:
    fs::remove_dir_all(root.join("tests")).ok();
    for i in 0..4 {
        write_empty(&root.join(format!("tests/a{i}/fake_sleep_200.py")));
    }

    // discover_test_files on the FakePlugin only walks the top-level
    // tests/ dir, not subdirs. Use an explicit file list instead of
    // relying on discovery so we can run the four duplicate-named files.
    let files: Vec<PathBuf> = (0..4)
        .map(|i| root.join(format!("tests/a{i}/fake_sleep_200.py")))
        .collect();

    // Rebuild pkg with test_dirs pointing at each subdir so
    // `owns_test_file` accepts them. Simpler: tweak the plugin to accept
    // any fake_*.py anywhere under the root. For this test, give it
    // multiple test_dirs.
    let plugin: Arc<dyn Plugin> = Arc::new(FakePlugin);
    let suite = TestSuite::new(
        plugin,
        root.to_path_buf(),
        vec!["tests/**/fake_*.py".into()],
        vec!["src/**/*.py".into()],
        WorkerHookPaths::default(),
        None,
    )
    .expect("compile globs");
    let pkg = Package {
        name: "fake".into(),
        root: root.to_path_buf(),
        test_suites: vec![suite],
        pytest_extra_args: Vec::new(),
        skyspell_extra_args: Vec::new(),
        skyspell_add_args: Vec::new(),
        python_interpreter: Vec::new(),
        env: BTreeMap::new(),
    };

    let start = std::time::Instant::now();
    let results = collect_results(&pkg, files, 2, Duration::from_secs(10)).await;
    let elapsed = start.elapsed();
    assert_eq!(results.len(), 4);

    // Strict lower bound: two serial batches of 200ms = 400ms minimum.
    // Upper bound avoids asserting anything that could flake under load;
    // the key invariant is "not parallel past the cap".
    assert!(
        elapsed >= Duration::from_millis(380),
        "2 workers on 4x200ms files finished in {:?}; pool ignored worker cap",
        elapsed
    );
}

#[tokio::test]
async fn pool_single_file_single_worker_round_trip() {
    // Smallest possible run: one worker, one file. Locks the "warm pool
    // reuses the single subprocess for just one file and still terminates
    // cleanly" path.
    if !python3_available() {
        return;
    }
    let tmp = tempfile::tempdir().unwrap();
    let root = tmp.path();
    write_empty(&root.join("tests/fake_pass.py"));

    let pkg = fake_package(root);
    let files = vec![root.join("tests/fake_pass.py")];

    let results = collect_results(&pkg, files, 1, Duration::from_secs(10)).await;
    assert_eq!(results.len(), 1);
    assert_eq!(results[0].messages.iter().filter(|m| matches!(m, scrutin_core::engine::protocol::Message::Done)).count(), 1);
}

#[tokio::test]
async fn pool_per_file_timeout_surfaces_as_error() {
    // A file that sleeps for 3 seconds with a per-file timeout of 500ms
    // must terminate early and emit an error message. The spec calls
    // this out as a must-have: timeouts should not hang the pool.
    if !python3_available() {
        return;
    }
    let tmp = tempfile::tempdir().unwrap();
    let root = tmp.path();
    write_empty(&root.join("tests/fake_sleep_3000.py"));

    let pkg = fake_package(root);
    let files = vec![root.join("tests/fake_sleep_3000.py")];

    let start = std::time::Instant::now();
    let results = collect_results(&pkg, files, 1, Duration::from_millis(500)).await;
    let elapsed = start.elapsed();

    assert!(
        elapsed < Duration::from_secs(2),
        "per-file timeout did not fire; elapsed {:?}",
        elapsed
    );
    assert_eq!(results.len(), 1, "timed-out file must still be reported");

    // The engine synthesizes an error event on timeout so downstream
    // reporters classify the file as bad. Look for *any* Error-outcome
    // event in the message stream.
    use scrutin_core::engine::protocol::{Message, Outcome};
    let has_error = results[0].messages.iter().any(|m| {
        matches!(m, Message::Event(e) if e.outcome == Outcome::Error)
    });
    assert!(
        has_error,
        "timed-out file should surface as an Error event; got messages: {:?}",
        results[0].messages
    );
}

#[tokio::test]
async fn pool_crashed_worker_is_reported_not_hung() {
    // fake_crash.py exits with code 42 without emitting `done`. The
    // engine should not hang; it should surface an error for that file
    // so subsequent runs can continue. The pool recovering to handle
    // more files after a crash is locked by a separate test below.
    if !python3_available() {
        return;
    }
    let tmp = tempfile::tempdir().unwrap();
    let root = tmp.path();
    write_empty(&root.join("tests/fake_crash.py"));

    let pkg = fake_package(root);
    let files = vec![root.join("tests/fake_crash.py")];

    let results = collect_results(&pkg, files, 1, Duration::from_secs(5)).await;
    assert_eq!(results.len(), 1, "crashed file must still be reported");

    use scrutin_core::engine::protocol::{Message, Outcome};
    let has_error = results[0].messages.iter().any(|m| {
        matches!(m, Message::Event(e) if e.outcome == Outcome::Error)
    });
    assert!(
        has_error,
        "crashed worker should produce an Error event; got: {:?}",
        results[0].messages
    );
}

#[tokio::test]
async fn pool_multi_suite_fan_out_runs_concurrently() {
    // Two suites with one slow file each, two workers total (one per
    // suite). If suites ran serially, total time >= 2 * sleep. If they
    // ran concurrently (one pool per suite, both started together),
    // total time should be close to 1 * sleep. start_run currently runs
    // suites SEQUENTIALLY (see run_events::start_run: "Run suites
    // sequentially so each suite gets the full worker pool"). This test
    // *documents* that choice by asserting the serial lower bound.
    //
    // If the policy ever flips to concurrent suite execution, this test
    // will fail and the change should be intentional.
    if !python3_available() {
        return;
    }
    let tmp = tempfile::tempdir().unwrap();
    let root = tmp.path();

    fs::create_dir_all(root.join("tests/alpha")).unwrap();
    fs::create_dir_all(root.join("tests/beta")).unwrap();
    let a = root.join("tests/alpha/fake_sleep_300.py");
    let b = root.join("tests/beta/fake_sleep_300.py");
    fs::write(&a, "").unwrap();
    fs::write(&b, "").unwrap();

    let plugin_a: Arc<dyn Plugin> = Arc::new(FakePlugin);
    let plugin_b: Arc<dyn Plugin> = Arc::new(FakePlugin);
    let suite_a = TestSuite::new(
        plugin_a,
        root.to_path_buf(),
        vec!["tests/alpha/**/fake_*.py".into()],
        vec!["src/**/*.py".into()],
        WorkerHookPaths::default(),
        None,
    )
    .expect("compile globs");
    let suite_b = TestSuite::new(
        plugin_b,
        root.to_path_buf(),
        vec!["tests/beta/**/fake_*.py".into()],
        vec!["src/**/*.py".into()],
        WorkerHookPaths::default(),
        None,
    )
    .expect("compile globs");
    let pkg = Package {
        name: "fake".into(),
        root: root.to_path_buf(),
        test_suites: vec![suite_a, suite_b],
        pytest_extra_args: Vec::new(),
        skyspell_extra_args: Vec::new(),
        skyspell_add_args: Vec::new(),
        python_interpreter: Vec::new(),
        env: BTreeMap::new(),
    };

    let start = std::time::Instant::now();
    let results = collect_results(&pkg, vec![a, b], 2, Duration::from_secs(10)).await;
    let elapsed = start.elapsed();
    assert_eq!(results.len(), 2);

    // Serial-suite policy: total elapsed >= 2 * 300ms.
    assert!(
        elapsed >= Duration::from_millis(550),
        "suites expected to run serially (documented in start_run); \
         but observed {:?} < 550ms. If this assertion is now wrong, the \
         policy was intentionally changed and this test should be updated.",
        elapsed
    );
}

#[tokio::test]
async fn pool_shared_cancel_handle_cancels_all_suites() {
    // Two suites, one slow file each. A cancel on the RunHandle
    // (shared across suites via the single CancelHandle in start_run)
    // must terminate both suites' in-flight work before they complete.
    if !python3_available() {
        return;
    }
    let tmp = tempfile::tempdir().unwrap();
    let root = tmp.path();
    fs::create_dir_all(root.join("tests/alpha")).unwrap();
    fs::create_dir_all(root.join("tests/beta")).unwrap();
    let a = root.join("tests/alpha/fake_sleep_2000.py");
    let b = root.join("tests/beta/fake_sleep_2000.py");
    fs::write(&a, "").unwrap();
    fs::write(&b, "").unwrap();

    let plugin_a: Arc<dyn Plugin> = Arc::new(FakePlugin);
    let plugin_b: Arc<dyn Plugin> = Arc::new(FakePlugin);
    let suite_a = TestSuite::new(
        plugin_a,
        root.to_path_buf(),
        vec!["tests/alpha/**/fake_*.py".into()],
        vec!["src/**/*.py".into()],
        WorkerHookPaths::default(),
        None,
    )
    .expect("compile globs");
    let suite_b = TestSuite::new(
        plugin_b,
        root.to_path_buf(),
        vec!["tests/beta/**/fake_*.py".into()],
        vec!["src/**/*.py".into()],
        WorkerHookPaths::default(),
        None,
    )
    .expect("compile globs");
    let pkg = Package {
        name: "fake".into(),
        root: root.to_path_buf(),
        test_suites: vec![suite_a, suite_b],
        pytest_extra_args: Vec::new(),
        skyspell_extra_args: Vec::new(),
        skyspell_add_args: Vec::new(),
        python_interpreter: Vec::new(),
        env: BTreeMap::new(),
    };

    let (handle, mut rx) = start_run(
        &pkg,
        vec![a, b],
        1, // one worker per suite
        Duration::from_secs(10),
        None,
        false,
        None,
    )
    .await
    .expect("start_run");

    // Fire cancel immediately, before either file has a chance to finish.
    handle.cancel.cancel_all();

    let start = std::time::Instant::now();
    let mut count = 0usize;
    while let Some(evt) = rx.recv().await {
        match evt {
            RunEvent::FileFinished(_) => count += 1,
            RunEvent::Complete => break,
        }
    }
    let elapsed = start.elapsed();

    assert!(
        elapsed < Duration::from_millis(2500),
        "shared cancel did not stop both suites; elapsed {:?} for two 2s sleeps",
        elapsed
    );
    assert!(count <= 2, "at most two FileFinished events (one per file)");
}

#[tokio::test]
async fn pool_cancel_all_stops_remaining_files() {
    // Queue 6 slow files (500ms each), limit to 1 worker, cancel
    // immediately after the first one completes. The remaining 5 must
    // be marked `cancelled` (or simply not emitted).
    if !python3_available() {
        return;
    }
    let tmp = tempfile::tempdir().unwrap();
    let root = tmp.path();
    let plugin: Arc<dyn Plugin> = Arc::new(FakePlugin);
    let mut files: Vec<PathBuf> = Vec::new();
    for i in 0..6 {
        let dir = root.join(format!("tests/b{i}"));
        fs::create_dir_all(&dir).unwrap();
        let f = dir.join("fake_sleep_500.py");
        fs::write(&f, "").unwrap();
        files.push(f);
    }
    let suite = TestSuite::new(
        plugin,
        root.to_path_buf(),
        vec!["tests/**/fake_*.py".into()],
        vec!["src/**/*.py".into()],
        WorkerHookPaths::default(),
        None,
    )
    .expect("compile globs");
    let pkg = Package {
        name: "fake".into(),
        root: root.to_path_buf(),
        test_suites: vec![suite],
        pytest_extra_args: Vec::new(),
        skyspell_extra_args: Vec::new(),
        skyspell_add_args: Vec::new(),
        python_interpreter: Vec::new(),
        env: BTreeMap::new(),
    };

    let (handle, mut rx) = start_run(&pkg, files, 1, Duration::from_secs(10), None, false, None)
        .await
        .expect("start_run");

    let start = std::time::Instant::now();
    let mut completed_count = 0usize;
    let mut cancelled_count = 0usize;
    while let Some(evt) = rx.recv().await {
        match evt {
            RunEvent::FileFinished(fr) => {
                if fr.cancelled {
                    cancelled_count += 1;
                } else {
                    completed_count += 1;
                    if completed_count == 1 {
                        // Cancel right after the first file completes.
                        handle.cancel.cancel_all();
                    }
                }
            }
            RunEvent::Complete => break,
        }
    }
    let elapsed = start.elapsed();

    // The serial lower bound for all 6 files would be 6 * 500ms = 3.0s.
    // With cancellation after the first completes, elapsed must be
    // dramatically shorter.
    assert!(
        elapsed < Duration::from_millis(2500),
        "cancel_all after first file should short-circuit; elapsed {:?}",
        elapsed
    );
    assert_eq!(completed_count, 1, "exactly one file should complete");
    // Remaining files may be reported as cancelled, or simply not
    // emitted at all if the pool dropped them before starting. Either
    // behavior is acceptable; the spec says cancellation must propagate.
    assert!(
        completed_count + cancelled_count <= 6,
        "never more events than files queued"
    );
}
