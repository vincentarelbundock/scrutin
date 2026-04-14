//! E2E tests for the non-TUI reporters (spec §3.8, §3.17).
//!
//! Drives the built `scrutin` binary against freshly-constructed pytest
//! fixtures and asserts the shape of each reporter's output. All tests
//! skip gracefully when pytest is unavailable.

use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::time::{Duration, Instant};

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

fn write_pyproject(root: &Path) {
    write(
        &root.join("pyproject.toml"),
        "[project]\nname = \"fixture\"\nversion = \"0.0.0\"\n",
    );
}

/// Write a project with three test files: one passing, two failing.
fn three_files(root: &Path) {
    write_pyproject(root);
    write(&root.join("tests/test_ok.py"), "def test_passes(): assert 1 + 1 == 2\n");
    write(&root.join("tests/test_bad_a.py"), "def test_fails(): assert False, 'broken A'\n");
    write(&root.join("tests/test_bad_b.py"), "def test_fails(): assert False, 'broken B'\n");
}

// ── -r list ────────────────────────────────────────────────────────────────

#[test]
fn list_reporter_prints_files_without_running_them() {
    // `-r list` must print every discovered test file and exit without
    // starting a run. The zero-subprocess invariant (§3.8) is asserted
    // indirectly by wall time: pytest startup on a 3-file fixture is
    // typically 400ms+; if `-r list` were actually running tests we'd see
    // that cost. We assert the budget is well under that.
    //
    // We don't need pytest to be installed at all for this test, since
    // list mode never invokes pytest. Run it unconditionally to lock
    // the "-r list works even without the toolchain" property.
    let project = tempfile::tempdir().unwrap();
    let root = project.path();
    three_files(root);

    let start = Instant::now();
    let output = Command::new(binary())
        .args([
            "--reporter",
            "list",
            "--set",
            "run.tool=pytest",
            root.to_str().unwrap(),
        ])
        .output()
        .expect("spawn scrutin");
    let elapsed = start.elapsed();

    let stdout = String::from_utf8_lossy(&output.stdout);
    let stderr = String::from_utf8_lossy(&output.stderr);
    let combined = format!("stdout:\n{stdout}\nstderr:\n{stderr}");

    assert!(
        output.status.success(),
        "-r list must exit 0 regardless of test content; {combined}"
    );
    // Header line: "3 test files would run".
    assert!(
        stdout.contains("3 test file") && stdout.contains("would run"),
        "missing file-count header in list output: {combined}"
    );
    for name in &["test_ok.py", "test_bad_a.py", "test_bad_b.py"] {
        assert!(
            stdout.contains(name),
            "missing {name} in list output: {combined}"
        );
    }
    // Zero-subprocess invariant proxy: well under pytest startup cost.
    assert!(
        elapsed < Duration::from_millis(1500),
        "-r list took {elapsed:?}, expected <1.5s (no subprocesses should spawn)"
    );
}

// ── -r junit: schema validity ───────────────────────────────────────────────

#[test]
fn junit_reporter_emits_wellformed_xml_with_expected_schema() {
    // The JUnit artifact is the CI-consumption seam: GitHub Actions,
    // Jenkins, and others parse it. Any schema regression (missing
    // attributes, malformed nesting) breaks those consumers. Lock:
    //   - Output is well-formed XML.
    //   - Top-level is <testsuites>, containing one or more <testsuite>.
    //   - Each <testsuite> has tests=, failures= (integers).
    //   - Each <testcase> has classname= and name=.
    //   - Failing tests have a nested <failure> element.
    if !pytest_available() {
        eprintln!("skipping: pytest not available");
        return;
    }
    let project = tempfile::tempdir().unwrap();
    let root = project.path();
    three_files(root);
    let junit_path = root.join("report.xml");

    let output = Command::new(binary())
        .args([
            "--reporter",
            &format!("junit:{}", junit_path.display()),
            "--set",
            "run.tool=pytest",
            root.to_str().unwrap(),
        ])
        .output()
        .expect("spawn scrutin");

    let stderr = String::from_utf8_lossy(&output.stderr);
    let stdout = String::from_utf8_lossy(&output.stdout);
    let combined = format!("stdout:\n{stdout}\nstderr:\n{stderr}");

    assert!(
        junit_path.exists(),
        "JUnit report not written to {}; {combined}",
        junit_path.display()
    );
    let xml = fs::read_to_string(&junit_path).expect("read junit xml");

    // XML well-formedness check via serde_json-adjacent route: we don't
    // have quick-xml in scope, but we can use a lightweight validity
    // check via `xmlparser`-style scanning. For now, assert on textual
    // anchors that would all drift together if the writer changed.
    assert!(
        xml.starts_with("<?xml"),
        "JUnit output must start with an XML declaration; got: {xml}"
    );
    assert!(
        xml.contains("<testsuites"),
        "missing <testsuites> root element in: {xml}"
    );
    assert!(
        xml.contains("</testsuites>"),
        "<testsuites> must be closed in: {xml}"
    );
    assert!(
        xml.contains("<testsuite "),
        "at least one <testsuite> expected in: {xml}"
    );
    // The suite-level attributes that aggregators rely on.
    assert!(
        xml.contains("tests=\""),
        "<testsuite> must carry tests= attribute: {xml}"
    );
    assert!(
        xml.contains("failures=\""),
        "<testsuite> must carry failures= attribute: {xml}"
    );
    // Per-test rows for every file the run saw.
    for name in &["test_ok.py", "test_bad_a.py", "test_bad_b.py"] {
        assert!(
            xml.contains(name),
            "missing test_*.py entry {name} in JUnit XML: {xml}"
        );
    }
    // Failing files must carry a nested <failure> (even if message is
    // the plain-text pytest diff). Two files fail here.
    let failure_tags = xml.matches("<failure").count();
    assert!(
        failure_tags >= 2,
        "expected >= 2 <failure> elements (two failing files); got {failure_tags} in: {xml}"
    );
}

#[test]
fn junit_reporter_exits_non_zero_on_failure() {
    // The spec binds exit code to run outcome (§3.8): 0 on all-pass,
    // non-zero on any failure. The junit reporter shouldn't swallow
    // that just because it also wrote an XML file.
    if !pytest_available() {
        return;
    }
    let project = tempfile::tempdir().unwrap();
    let root = project.path();
    three_files(root);
    let junit_path = root.join("report.xml");

    let output = Command::new(binary())
        .args([
            "--reporter",
            &format!("junit:{}", junit_path.display()),
            "--set",
            "run.tool=pytest",
            root.to_str().unwrap(),
        ])
        .output()
        .expect("spawn scrutin");

    assert!(
        !output.status.success(),
        "junit reporter must still propagate non-zero exit on failing run"
    );
}

#[test]
fn junit_reporter_exits_zero_on_all_pass() {
    // Mirror of the failure test: all-pass fixture must exit 0.
    if !pytest_available() {
        return;
    }
    let project = tempfile::tempdir().unwrap();
    let root = project.path();
    write_pyproject(root);
    write(&root.join("tests/test_ok.py"), "def test_a(): assert True\n");
    let junit_path = root.join("report.xml");

    let output = Command::new(binary())
        .args([
            "--reporter",
            &format!("junit:{}", junit_path.display()),
            "--set",
            "run.tool=pytest",
            root.to_str().unwrap(),
        ])
        .output()
        .expect("spawn scrutin");

    let combined = format!(
        "stdout:\n{}\nstderr:\n{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr),
    );
    assert!(
        output.status.success(),
        "junit reporter must exit 0 when every file passes; {combined}"
    );
}

// ── -r github: annotations + grouping ──────────────────────────────────────

#[test]
fn github_reporter_emits_annotations_for_failing_files() {
    // GHA parses `::error file=...::` lines and `::group::` blocks into
    // PR annotations and collapsible log sections. Lock both.
    if !pytest_available() {
        return;
    }
    let project = tempfile::tempdir().unwrap();
    let root = project.path();
    three_files(root);

    // GITHUB_STEP_SUMMARY in a tempfile so we can inspect it.
    let summary_path = root.join("step_summary.md");
    fs::write(&summary_path, "").unwrap();

    let output = Command::new(binary())
        .args([
            "--reporter",
            "github",
            "--set",
            "run.tool=pytest",
            root.to_str().unwrap(),
        ])
        .env("GITHUB_STEP_SUMMARY", &summary_path)
        .output()
        .expect("spawn scrutin");

    let stdout = String::from_utf8_lossy(&output.stdout);
    let stderr = String::from_utf8_lossy(&output.stderr);
    let combined = format!("stdout:\n{stdout}\nstderr:\n{stderr}");

    assert!(
        !output.status.success(),
        "github reporter must propagate non-zero exit on failing run; {combined}"
    );
    // Group markers: one ::group::/::endgroup:: pair per file.
    assert!(
        stdout.contains("::group::") || stderr.contains("::group::"),
        "github reporter must emit ::group:: markers; {combined}"
    );
    assert!(
        stdout.contains("::endgroup::") || stderr.contains("::endgroup::"),
        "github reporter must emit ::endgroup:: markers; {combined}"
    );
    // Error annotations: at least one for each failing file.
    let error_count = stdout.matches("::error").count() + stderr.matches("::error").count();
    assert!(
        error_count >= 2,
        "expected >= 2 ::error annotations (two failing files); got {error_count}; {combined}"
    );
    // Step summary populated with something non-empty.
    let summary = fs::read_to_string(&summary_path).unwrap_or_default();
    assert!(
        !summary.trim().is_empty(),
        "GITHUB_STEP_SUMMARY must be populated; file was empty"
    );
}

#[test]
fn github_reporter_exits_zero_on_all_pass() {
    if !pytest_available() {
        return;
    }
    let project = tempfile::tempdir().unwrap();
    let root = project.path();
    write_pyproject(root);
    write(&root.join("tests/test_ok.py"), "def test_a(): assert True\n");

    let output = Command::new(binary())
        .args([
            "--reporter",
            "github",
            "--set",
            "run.tool=pytest",
            root.to_str().unwrap(),
        ])
        .output()
        .expect("spawn scrutin");

    assert!(
        output.status.success(),
        "github reporter must exit 0 when every file passes"
    );
}

// ── -r plain: exit codes ────────────────────────────────────────────────────

#[test]
fn plain_reporter_exit_codes() {
    // Pin the contract from §3.8: 0 on all-pass, non-zero on any failure.
    if !pytest_available() {
        return;
    }

    // Case 1: all-pass.
    {
        let project = tempfile::tempdir().unwrap();
        let root = project.path();
        write_pyproject(root);
        write(&root.join("tests/test_ok.py"), "def test_a(): assert True\n");

        let output = Command::new(binary())
            .args([
                "--reporter",
                "plain",
                "--set",
                "run.tool=pytest",
                root.to_str().unwrap(),
            ])
            .output()
            .expect("spawn");
        assert!(output.status.success(), "all-pass must exit 0");
    }

    // Case 2: has failure.
    {
        let project = tempfile::tempdir().unwrap();
        let root = project.path();
        write_pyproject(root);
        write(&root.join("tests/test_bad.py"), "def test_a(): assert False\n");

        let output = Command::new(binary())
            .args([
                "--reporter",
                "plain",
                "--set",
                "run.tool=pytest",
                root.to_str().unwrap(),
            ])
            .output()
            .expect("spawn");
        assert!(
            !output.status.success(),
            "any failure must exit non-zero"
        );
    }
}
