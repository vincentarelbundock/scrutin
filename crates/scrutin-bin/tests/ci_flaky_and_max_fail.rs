//! E2E tests for rerun/flaky behavior and max_fail semantics (spec §3.9).
//!
//! Drives the built `scrutin` binary against freshly-constructed pytest
//! fixtures in tempdirs. The flaky fixture uses a counter file outside
//! the project root to flip pass/fail per attempt; max_fail fixtures use
//! deterministic failures.
//!
//! All tests skip gracefully when pytest is unavailable.

use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::time::{SystemTime, UNIX_EPOCH};

fn pytest_available() -> bool {
    Command::new("python3")
        .args(["-m", "pytest", "--version"])
        .output()
        .map(|o| o.status.success())
        .unwrap_or(false)
}

fn binary() -> PathBuf {
    PathBuf::from(env!("CARGO_BIN_EXE_scrutin"))
}

fn write(path: &Path, contents: &str) {
    fs::create_dir_all(path.parent().unwrap()).unwrap();
    fs::write(path, contents).unwrap();
}

/// Allocate a unique path outside the project root for cross-attempt
/// state. Nanosecond epoch + pid keeps concurrent test runs isolated.
fn unique_state_file(tag: &str) -> PathBuf {
    let ns = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_nanos();
    std::env::temp_dir().join(format!("scrutin-{tag}-{}-{}", std::process::id(), ns))
}

/// Write a pyproject.toml marker file so the pytest plugin detects
/// the project. The [project] table is the minimum pytest needs.
fn write_pyproject(root: &Path) {
    write(
        &root.join("pyproject.toml"),
        "[project]\nname = \"fixture\"\nversion = \"0.0.0\"\n",
    );
}

// ── Flaky detection: fails on attempt 1, passes on attempt 2 ────────────────

#[test]
fn flaky_file_passes_on_rerun_is_marked_flaky() {
    // A test that fails on its first invocation and passes on any
    // subsequent invocation. With --set run.reruns=2, scrutin must:
    //   1. Run the file (fails).
    //   2. Rerun the failing file once (passes).
    //   3. Mark the file flaky (passed after failing).
    //   4. Exit with 0 (no remaining failures).
    if !pytest_available() {
        eprintln!("skipping: pytest not available");
        return;
    }
    let project = tempfile::tempdir().unwrap();
    let root = project.path();
    let counter = unique_state_file("flaky");
    // Clean slate in case a previous crashed run left it behind.
    let _ = fs::remove_file(&counter);

    write_pyproject(root);
    write(
        &root.join("tests/test_flaky.py"),
        &format!(
            r#"from pathlib import Path

COUNTER = Path({counter:?})

def test_flaky():
    n = int(COUNTER.read_text()) if COUNTER.exists() else 0
    n += 1
    COUNTER.write_text(str(n))
    assert n >= 2, f"fail on attempt {{n}} (pass on any attempt >= 2)"
"#,
            counter = counter.to_string_lossy(),
        ),
    );

    let output = Command::new(binary())
        .args([
            "--reporter",
            "plain",
            "--set",
            "run.tool=pytest",
            "--set",
            "run.reruns=2",
            root.to_str().unwrap(),
        ])
        .output()
        .expect("spawn scrutin");

    let stderr = String::from_utf8_lossy(&output.stderr);
    let stdout = String::from_utf8_lossy(&output.stdout);
    let combined = format!("stdout:\n{stdout}\nstderr:\n{stderr}");

    let _ = fs::remove_file(&counter);

    assert!(
        stderr.contains("rerun 1/"),
        "rerun loop did not fire; {combined}"
    );
    assert!(
        output.status.success(),
        "exit must be 0 when failing file passes on rerun; {combined}"
    );
    // Plain reporter labels flaky files explicitly.
    assert!(
        combined.contains("flaky"),
        "flaky marker missing from output; {combined}"
    );
}

// ── Deterministic fail: all reruns fail too, not marked flaky ───────────────

#[test]
fn deterministic_failure_is_not_marked_flaky() {
    // A test that fails on every invocation must not be marked flaky
    // just because we retried. This is already covered for R in
    // ci_reruns_and_metadata.rs; here we lock the same invariant for
    // pytest on a pure-Python fixture.
    if !pytest_available() {
        return;
    }
    let project = tempfile::tempdir().unwrap();
    let root = project.path();
    write_pyproject(root);
    write(
        &root.join("tests/test_always_fails.py"),
        "def test_always(): assert False, \"deterministic\"\n",
    );

    let output = Command::new(binary())
        .args([
            "--reporter",
            "plain",
            "--set",
            "run.tool=pytest",
            "--set",
            "run.reruns=1",
            root.to_str().unwrap(),
        ])
        .output()
        .expect("spawn scrutin");

    let stderr = String::from_utf8_lossy(&output.stderr);
    let stdout = String::from_utf8_lossy(&output.stdout);
    let combined = format!("stdout:\n{stdout}\nstderr:\n{stderr}");

    assert!(
        !output.status.success(),
        "exit must be non-zero when all attempts fail; {combined}"
    );
    // "X file passed on rerun (flaky):" is the plain reporter's flaky
    // banner. It must not fire for a deterministic failure.
    assert!(
        !combined.contains("passed on rerun (flaky)"),
        "deterministic failure should not trip the flaky banner; {combined}"
    );
}

// ── max_fail is file-level, not expectation-level ───────────────────────────

#[test]
fn max_fail_counts_files_not_expectations() {
    // A single test file with 10 failing assertions against --max-fail=1
    // must not abort mid-file: max_fail budgets are spent per *file*, so
    // the whole file's assertions are expected to run and be reported.
    // The run still exits non-zero because the file is "bad".
    if !pytest_available() {
        return;
    }
    let project = tempfile::tempdir().unwrap();
    let root = project.path();
    write_pyproject(root);

    // One file, 10 distinct failing assertions.
    let mut body = String::new();
    for i in 0..10 {
        body.push_str(&format!(
            "def test_assert_{i}(): assert False, \"fail number {i}\"\n",
        ));
    }
    write(&root.join("tests/test_many_failures.py"), &body);

    let output = Command::new(binary())
        .args([
            "--reporter",
            "plain",
            "--set",
            "run.tool=pytest",
            "--set",
            "run.max_fail=1",
            root.to_str().unwrap(),
        ])
        .output()
        .expect("spawn scrutin");

    let stderr = String::from_utf8_lossy(&output.stderr);
    let stdout = String::from_utf8_lossy(&output.stdout);
    let combined = format!("stdout:\n{stdout}\nstderr:\n{stderr}");

    assert!(
        !output.status.success(),
        "exit must be non-zero on failing file; {combined}"
    );
    // The file completed its run; all 10 failures should be counted.
    // The summary line includes "N failed, M passed, ...". Parsing it
    // loosely: look for "10 failed" to confirm expectation-level budget
    // isn't consumed.
    assert!(
        combined.contains("10 failed") || combined.contains("failed, 0 passed")
            || combined.contains("10 tests failed"),
        "expected all 10 assertions to be reported as failed; \
         max_fail=1 must budget per file not per expectation; {combined}"
    );
}

#[test]
fn max_fail_one_halts_after_first_bad_file() {
    // Three failing files, concurrent workers (default). max_fail=1 must
    // yield exactly 1 failure in the summary; the other two files may
    // have been in flight when the trip fired, but their post-trip
    // results must be recorded as cancelled so they don't inflate the
    // failure count.
    //
    // This is the user-facing contract: `max_fail=1` means "stop at the
    // first failure", not "stop at the first failure plus whatever else
    // finished at the same time." Under concurrent workers, the plain
    // reporter discards (as cancelled) any post-trip arrivals.
    if !pytest_available() {
        return;
    }
    let project = tempfile::tempdir().unwrap();
    let root = project.path();
    write_pyproject(root);

    for i in 0..3 {
        write(
            &root.join(format!("tests/test_fail_{i}.py")),
            &format!("def test_x(): assert False, \"file {i}\"\n"),
        );
    }

    let output = Command::new(binary())
        .args([
            "--reporter",
            "plain",
            "--set",
            "run.tool=pytest",
            "--set",
            "run.max_fail=1",
            root.to_str().unwrap(),
        ])
        .output()
        .expect("spawn scrutin");

    let stderr = String::from_utf8_lossy(&output.stderr);
    let stdout = String::from_utf8_lossy(&output.stdout);
    let combined = format!("stdout:\n{stdout}\nstderr:\n{stderr}");

    assert!(
        !output.status.success(),
        "exit must be non-zero with max_fail tripped; {combined}"
    );
    assert!(
        stderr.contains("max-fail reached"),
        "the user-facing 'max-fail reached' message must appear; {combined}"
    );
    // The authoritative count is "N failed" in the summary line.
    let failed_count = parse_failed_count(&combined);
    assert_eq!(
        failed_count, 1,
        "max_fail=1 must yield exactly 1 failure in the summary (stronger \
         semantic: post-trip in-flight arrivals are recorded as cancelled, \
         not as failures); got: {combined}"
    );
}

/// Extract the final `N failed` count from the plain-mode summary line.
///
/// The summary line is the last one of shape `X passed, Y failed, ...`.
/// Preceding per-file `FAIL file:line > test` lines also contain the
/// word `failed` in some builds, so we scan for the summary-shaped line
/// specifically (contains both " passed," and " failed,").
fn parse_failed_count(output: &str) -> usize {
    let summary = output
        .lines()
        .rev()
        .find(|l| l.contains(" passed,") && l.contains(" failed,"))
        .unwrap_or("");
    // "X passed, Y failed, Z errored, ..."
    let after_passed = match summary.find(" passed,") {
        Some(i) => &summary[i + " passed,".len()..],
        None => return 0,
    };
    let failed_part = after_passed.trim_start();
    let num: String = failed_part
        .chars()
        .take_while(|c| c.is_ascii_digit())
        .collect();
    num.parse().unwrap_or(0)
}

#[test]
fn max_fail_zero_is_disabled_runs_all_files() {
    // `max_fail=0` is documented as "disabled" (no budget). All three
    // failing files should be run to completion rather than any being
    // cancelled. Locks the "0 means unlimited" semantics so someone
    // doesn't accidentally change the sentinel to "abort immediately".
    if !pytest_available() {
        return;
    }
    let project = tempfile::tempdir().unwrap();
    let root = project.path();
    write_pyproject(root);

    for i in 0..3 {
        write(
            &root.join(format!("tests/test_fail_{i}.py")),
            &format!("def test_x(): assert False, \"file {i}\"\n"),
        );
    }

    let output = Command::new(binary())
        .args([
            "--reporter",
            "plain",
            "--set",
            "run.tool=pytest",
            "--set",
            "run.max_fail=0",
            root.to_str().unwrap(),
        ])
        .output()
        .expect("spawn scrutin");

    let stderr = String::from_utf8_lossy(&output.stderr);
    let stdout = String::from_utf8_lossy(&output.stdout);
    let combined = format!("stdout:\n{stdout}\nstderr:\n{stderr}");

    // All three files should appear in the output.
    for i in 0..3 {
        let needle = format!("test_fail_{i}.py");
        assert!(
            combined.contains(&needle),
            "{needle} must appear in output with max_fail=0; {combined}"
        );
    }
    assert!(
        !combined.contains("max_fail") && !combined.contains("reached max"),
        "max_fail=0 must not trip a max-fail message; {combined}"
    );
}
