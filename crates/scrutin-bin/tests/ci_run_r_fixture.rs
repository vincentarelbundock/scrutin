//! End-to-end smoke test: run the built `scrutin` binary against the R demo
//! fixture in `--ci` mode and check it produces the expected pass/fail counts.
//!
//! Skipped automatically if `Rscript` is not on PATH (so contributors without
//! R installed can still run `cargo test`).

use std::path::PathBuf;
use std::process::Command;

fn rscript_available() -> bool {
    Command::new("Rscript")
        .arg("--version")
        .output()
        .map(|o| o.status.success())
        .unwrap_or(false)
}

fn repo_root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("..").join("..").canonicalize().expect("workspace root")
}

fn binary() -> PathBuf {
    // Set by cargo when running integration tests for a binary crate.
    PathBuf::from(env!("CARGO_BIN_EXE_scrutin"))
}

#[test]
fn ci_run_against_scrutindemo_testthat() {
    if !rscript_available() {
        eprintln!("skipping: Rscript not on PATH");
        return;
    }

    let fixture = repo_root().join("demo");
    assert!(fixture.join("DESCRIPTION").exists(), "fixture missing");

    // Pass a *relative* path to exercise the canonicalization in main.rs:
    // R's pkgload::load_all otherwise resolves it from R's own cwd and fails.
    let output = Command::new(binary())
        .args([
            "--reporter",
            "plain",
            "--set",
            "run.tool=testthat",
            "demo",
        ])
        .current_dir(repo_root())
        .output()
        .expect("failed to spawn scrutin binary");

    let stdout = String::from_utf8_lossy(&output.stdout);
    let stderr = String::from_utf8_lossy(&output.stderr);
    let combined = format!("{stdout}\n{stderr}");

    // The fixture is intentionally buggy: it should report at least one
    // failure and a non-zero exit code in --ci mode.
    assert!(
        !output.status.success(),
        "expected non-zero exit on buggy fixture\n--- stdout ---\n{stdout}\n--- stderr ---\n{stderr}"
    );

    // Look for the summary line "N passed, N failed, ..." printed by --ci.
    assert!(
        combined.contains("passed,") && combined.contains("failed,"),
        "missing summary line in output:\n{combined}"
    );

    // We expect both test files to have been discovered.
    assert!(
        combined.contains("test-math.R"),
        "test-math.R not mentioned in output:\n{combined}"
    );
    assert!(
        combined.contains("test-strings.R"),
        "test-strings.R not mentioned in output:\n{combined}"
    );
}
