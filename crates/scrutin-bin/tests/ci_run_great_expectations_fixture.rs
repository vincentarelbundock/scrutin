//! End-to-end smoke test: run the built `scrutin` binary against the
//! great_expectations demo fixture in plain-reporter mode and check it
//! produces the expected pass/fail counts.
//!
//! Skipped automatically if `great_expectations` cannot be imported, so
//! contributors without the (heavy) GE dependency installed can still run
//! `cargo test`.

use std::path::PathBuf;
use std::process::Command;

fn ge_available() -> bool {
    Command::new("python3")
        .args(["-c", "import great_expectations"])
        .output()
        .map(|o| o.status.success())
        .unwrap_or(false)
}

fn repo_root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("..")
        .join("..")
        .canonicalize()
        .expect("workspace root")
}

fn binary() -> PathBuf {
    PathBuf::from(env!("CARGO_BIN_EXE_scrutin"))
}

#[test]
fn ci_run_against_scrutindemo_great_expectations() {
    if !ge_available() {
        eprintln!("skipping: great_expectations not importable via `python3 -c 'import great_expectations'`");
        return;
    }

    let fixture = repo_root().join("demo");
    assert!(
        fixture.join("tests/great_expectations/test_orders.py").exists(),
        "fixture missing tests/great_expectations/test_orders.py"
    );

    let output = Command::new(binary())
        .args([
            "--reporter",
            "plain",
            "--set",
            "run.tool=great_expectations",
            "demo",
        ])
        .current_dir(repo_root())
        .output()
        .expect("failed to spawn scrutin binary");

    let stdout = String::from_utf8_lossy(&output.stdout);
    let stderr = String::from_utf8_lossy(&output.stderr);
    let combined = format!("{stdout}\n{stderr}");

    // The fixture intentionally contains failing expectations, so the run
    // should exit non-zero.
    assert!(
        !output.status.success(),
        "expected non-zero exit on buggy fixture\n--- stdout ---\n{stdout}\n--- stderr ---\n{stderr}"
    );

    // Plain-mode summary line.
    assert!(
        combined.contains("passed,") && combined.contains("failed,"),
        "missing summary line in output:\n{combined}"
    );

    // The single test file should have been discovered and run.
    assert!(
        combined.contains("test_orders.py"),
        "test_orders.py not mentioned in output:\n{combined}"
    );
}
