//! End-to-end smoke test: run the built `scrutin` binary against the pytest
//! demo fixture in plain-reporter mode and check it produces the expected
//! pass/fail counts.
//!
//! Skipped automatically if `pytest` (via `python3 -m pytest --version`) is
//! not available, so contributors without a Python toolchain installed can
//! still run `cargo test`.

use std::path::PathBuf;
use std::process::Command;

fn pytest_available() -> bool {
    Command::new("python3")
        .args(["-m", "pytest", "--version"])
        .output()
        .map(|o| o.status.success())
        .unwrap_or(false)
}

fn repo_root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("..").join("..").canonicalize().expect("workspace root")
}

fn binary() -> PathBuf {
    PathBuf::from(env!("CARGO_BIN_EXE_scrutin"))
}

#[test]
fn ci_run_against_scrutindemo_pytest() {
    if !pytest_available() {
        eprintln!("skipping: pytest not available via `python3 -m pytest`");
        return;
    }

    let fixture = repo_root().join("demo");
    assert!(
        fixture.join("pyproject.toml").exists(),
        "fixture missing pyproject.toml"
    );

    let output = Command::new(binary())
        .args([
            "--reporter",
            "plain",
            "--set",
            "run.tool=pytest",
            "demo",
        ])
        .current_dir(repo_root())
        .output()
        .expect("failed to spawn scrutin binary");

    let stdout = String::from_utf8_lossy(&output.stdout);
    let stderr = String::from_utf8_lossy(&output.stderr);
    let combined = format!("{stdout}\n{stderr}");

    // The fixture is intentionally buggy (mirrors the R fixture's shape):
    // it should report at least one failure and a non-zero exit code.
    assert!(
        !output.status.success(),
        "expected non-zero exit on buggy fixture\n--- stdout ---\n{stdout}\n--- stderr ---\n{stderr}"
    );

    // Plain-mode summary line.
    assert!(
        combined.contains("passed,") && combined.contains("failed,"),
        "missing summary line in output:\n{combined}"
    );

    // Both test files should have been discovered.
    assert!(
        combined.contains("test_math.py"),
        "test_math.py not mentioned in output:\n{combined}"
    );
    assert!(
        combined.contains("test_strings.py"),
        "test_strings.py not mentioned in output:\n{combined}"
    );
}
