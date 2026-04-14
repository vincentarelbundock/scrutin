//! End-to-end test for the rerun loop and metadata capture features.
//!
//! Runs the buggy R fixture with `--set run.reruns=1
//! --set metadata.enabled=true --target junit:<tmp>` and asserts that:
//!
//! 1. The rerun loop actually re-executes the failing files (visible as a
//!    "rerun 1/1" line on stderr).
//! 2. The flaky-file marker propagates: a file that fails on attempt 1
//!    and *also* fails on attempt 2 must NOT be marked flaky (the test
//!    fixture is deterministically buggy, so we expect zero flaky markers
//!    in the JUnit output).
//! 3. The `<properties>` block is emitted with at least the
//!    `scrutin.version`, `os`, and `git.sha` properties.
//! 4. User-supplied labels via `--set extras.KEY=VALUE` survive
//!    the round-trip into JUnit XML.
//!
//! Skipped automatically when Rscript isn't available.

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
    PathBuf::from(env!("CARGO_BIN_EXE_scrutin"))
}

#[test]
fn reruns_and_metadata_round_trip() {
    if !rscript_available() {
        eprintln!("skipping: Rscript not on PATH");
        return;
    }

    let fixture = repo_root().join("demo");
    assert!(fixture.join("DESCRIPTION").exists(), "fixture missing");

    // Per-process tmp file so parallel test runs don't collide.
    let tmp_dir = std::env::temp_dir().join(format!("scrutin-it-{}", std::process::id()));
    let _ = std::fs::create_dir_all(&tmp_dir);
    let junit_path = tmp_dir.join("report.xml");
    let _ = std::fs::remove_file(&junit_path);

    let output = Command::new(binary())
        .args([
            "--reporter",
            &format!("junit:{}", junit_path.display()),
            "--set",
            "run.tool=testthat",
            "--set",
            "run.reruns=1",
            "--set",
            "metadata.enabled=true",
            "--set",
            "extras.build=4521",
            "--set",
            "extras.deploy=staging",
            "demo",
        ])
        .current_dir(repo_root())
        .output()
        .expect("failed to spawn scrutin binary");

    let stderr = String::from_utf8_lossy(&output.stderr);
    let stdout = String::from_utf8_lossy(&output.stdout);

    // 1. Rerun loop fired.
    assert!(
        stderr.contains("rerun 1/1"),
        "expected rerun loop to fire, stderr was:\n{stderr}\nstdout:\n{stdout}"
    );

    // JUnit file exists.
    assert!(
        junit_path.exists(),
        "JUnit report not written to {}",
        junit_path.display()
    );
    let xml = std::fs::read_to_string(&junit_path).expect("read junit xml");

    // 2. The fixture's failing files fail deterministically on every
    //    attempt, so they must NOT be marked flaky.
    assert!(
        !xml.contains("scrutin.flaky"),
        "deterministic failures should not be marked flaky:\n{xml}"
    );

    // 3. Provenance properties present.
    for needle in &[
        "<property name=\"scrutin.version\"",
        "<property name=\"os\"",
        "<property name=\"git.sha\"",
        "<property name=\"tool\" value=\"testthat\"",
    ] {
        assert!(
            xml.contains(needle),
            "missing provenance property {needle:?}:\n{xml}"
        );
    }

    // 4. User labels survive the round-trip.
    assert!(
        xml.contains("<property name=\"build\" value=\"4521\""),
        "missing user label `build`:\n{xml}"
    );
    assert!(
        xml.contains("<property name=\"deploy\" value=\"staging\""),
        "missing user label `deploy`:\n{xml}"
    );

    let _ = std::fs::remove_dir_all(&tmp_dir);
}
