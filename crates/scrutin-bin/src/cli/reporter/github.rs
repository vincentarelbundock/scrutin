//! GitHub Actions reporter: streams `::group::`/`::endgroup::` per file,
//! emits `::error`/`::warning` annotations, and writes a markdown summary
//! to `$GITHUB_STEP_SUMMARY`.

use std::collections::HashSet;
use std::path::PathBuf;
use std::time::{Duration, Instant};

use anyhow::Result;

use scrutin_core::engine::protocol::{self as proto, Message, Outcome, file_display_name};
use scrutin_core::metadata::RunMetadata;
use scrutin_core::project::config::Config;
use scrutin_core::project::package::Package;
use scrutin_core::storage::sqlite;

use super::{FileRecord, RunStats, merge_deps_from_results, rebuild_depmap_in_background};

// ---------------------------------------------------------------------------
// Entry point
// ---------------------------------------------------------------------------

#[allow(clippy::too_many_arguments)]
pub async fn run(
    pkg: &Package,
    test_files: &[PathBuf],
    n_workers: usize,
    cfg: &Config,
    depmap_stale: bool,
    is_full_suite: bool,
    run_metadata: &RunMetadata,
) -> Result<i32> {
    use scrutin_core::engine::run_events::{self, RunEvent};

    let timeout = Duration::from_millis(cfg.run.timeout_file_ms);
    let timeout_run = if cfg.run.timeout_run_ms > 0 {
        Some(Duration::from_millis(cfg.run.timeout_run_ms))
    } else {
        None
    };
    let max_fail = cfg.run.max_fail;
    let n_files = test_files.len();
    let t0 = Instant::now();

    let (handle, mut rx) =
        run_events::start_run(pkg, test_files.to_vec(), n_workers, timeout, timeout_run, cfg.run.fork_workers, None)
            .await?;
    let cancel = handle.cancel.clone();

    let mut all_results: Vec<FileRecord> = Vec::new();
    let mut totals = proto::Counts::default();
    let mut failed_files: u32 = 0;
    let mut tripped = false;
    let mut file_durations: Vec<(String, u64)> = Vec::new();

    while let Some(ev) = rx.recv().await {
        match ev {
            RunEvent::FileFinished(result) => {
                let file_name = file_display_name(&result.file);
                let rel_path = result
                    .file
                    .strip_prefix(&pkg.root)
                    .unwrap_or(&result.file)
                    .display()
                    .to_string();
                let tally = proto::tally_messages(&result.messages, result.cancelled);

                render_file(&file_name, &rel_path, &result.messages, &tally);

                totals.merge(&tally.counts);
                file_durations.push((file_name.clone(), tally.duration_ms));
                if tally.bad {
                    failed_files += 1;
                }

                all_results.push(FileRecord {
                    file: file_name,
                    messages: result.messages,
                    cancelled: result.cancelled,
                });

                if !tripped && max_fail > 0 && failed_files >= max_fail {
                    tripped = true;
                    eprintln!(
                        "max-fail reached ({}), cancelling in-flight workers.",
                        max_fail,
                    );
                    cancel.cancel_all();
                }
            }
            RunEvent::Complete => break,
        }
    }

    let elapsed = t0.elapsed();
    let slowest = file_durations.into_iter().max_by_key(|(_, ms)| *ms);
    let stats = RunStats {
        passed: totals.pass,
        failed: totals.fail,
        errored: totals.error,
        warned: totals.warn,
        skipped: totals.skip,
        n_files: n_files as u32,
        elapsed,
        slowest,
    };
    print_summary(&stats);

    // DB persistence (best-effort).
    {
        let run_id = uuid::Uuid::new_v4().to_string();
        let timestamp = chrono::Utc::now().to_rfc3339();
        let rows = super::plain::build_result_rows(pkg, &all_results, &HashSet::new());
        let _ = sqlite::with_open(&pkg.root, |c| {
            sqlite::record_run(c, &run_id, &timestamp, &run_metadata.provenance, &rows)?;
            sqlite::record_extras(c, &run_id, &run_metadata.labels)
        });
    }

    // Merge runtime dep observations from instrumentation.
    {
        let mut map = sqlite::with_open(&pkg.root, |c| Ok(sqlite::load_dep_map(c)))
            .unwrap_or_default();
        merge_deps_from_results(&mut map, &all_results, &pkg.root);
    }

    // Rebuild Python side of the dep map if stale.
    if is_full_suite && depmap_stale
        && pkg.test_suites.iter().any(|s| s.plugin.name() == "pytest")
    {
        rebuild_depmap_in_background(pkg);
    }

    Ok(if failed_files > 0 { 1 } else { 0 })
}

// ---------------------------------------------------------------------------
// Per-file rendering
// ---------------------------------------------------------------------------

/// Render one file's results as a GitHub Actions log group with annotations.
fn render_file(
    file_name: &str,
    rel_path: &str,
    messages: &[Message],
    tally: &proto::FileTally,
) {
    let status = if tally.status == proto::FileStatus::Cancelled {
        "CNCL"
    } else if tally.bad {
        "FAIL"
    } else if tally.warned {
        "WARN"
    } else {
        "OK"
    };

    let duration = if tally.duration_ms > 0 {
        format!(" ({}ms)", tally.duration_ms)
    } else {
        String::new()
    };
    eprintln!("::group::{} {}{}", status, file_name, duration);

    for ev in proto::process_events(messages) {
        match ev.outcome {
            Outcome::Fail | Outcome::Error => {
                let line_part = ev.line.map(|l| format!(",line={l}")).unwrap_or_default();
                let msg = escape(ev.message.lines().next().unwrap_or(""));
                eprintln!(
                    "::error file={rel_path}{line_part}::{} > {msg}",
                    ev.name,
                );
                // Print full message body inside the group for context.
                for line in ev.message.lines() {
                    eprintln!("  {line}");
                }
            }
            Outcome::Warn => {
                let line_part = ev.line.map(|l| format!(",line={l}")).unwrap_or_default();
                let msg = escape(ev.message.lines().next().unwrap_or(""));
                eprintln!(
                    "::warning file={rel_path}{line_part}::{} > {msg}",
                    ev.name,
                );
            }
            Outcome::Skip | Outcome::Xfail => {
                let label = match ev.outcome {
                    Outcome::Skip => "skip",
                    Outcome::Xfail => "xfail",
                    _ => unreachable!(),
                };
                eprintln!("  {label}  {}", ev.name);
            }
            Outcome::Pass => {
                eprintln!("  pass  {} ({}ms)", ev.name, ev.duration_ms);
            }
        }
    }

    eprintln!("::endgroup::");
}

// ---------------------------------------------------------------------------
// Summary
// ---------------------------------------------------------------------------

/// Print a text summary to stderr and write a markdown table to
/// `$GITHUB_STEP_SUMMARY` when the env var is set.
fn print_summary(s: &RunStats) {
    let bad = s.failed + s.errored;
    let executed = s.passed + s.failed + s.errored;
    let pass_rate = if executed > 0 {
        (s.passed as f64 / executed as f64) * 100.0
    } else {
        0.0
    };

    eprintln!();
    eprintln!(
        "{} passed, {} failed, {} errored, {} warned, {} skipped ({:.2}s, {:.1}% pass rate)",
        s.passed, s.failed, s.errored, s.warned, s.skipped,
        s.elapsed.as_secs_f64(), pass_rate,
    );

    // Write markdown job summary when running on GitHub Actions.
    if let Ok(summary_path) = std::env::var("GITHUB_STEP_SUMMARY") {
        use std::io::Write;
        if let Ok(mut f) = std::fs::OpenOptions::new()
            .create(true)
            .append(true)
            .open(&summary_path)
        {
            let icon = if bad > 0 { "&#x274C;" } else { "&#x2705;" };
            let _ = writeln!(f, "### {icon} scrutin results\n");
            let _ = writeln!(
                f,
                "| Passed | Failed | Errored | Warned | Skipped | Time |"
            );
            let _ = writeln!(
                f,
                "|--------|--------|---------|--------|---------|------|"
            );
            let _ = writeln!(
                f,
                "| {} | {} | {} | {} | {} | {:.2}s |",
                s.passed, s.failed, s.errored, s.warned, s.skipped,
                s.elapsed.as_secs_f64(),
            );
        }
    }
}

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

/// Escape special characters in GitHub Actions workflow command messages.
/// GHA requires `%`, `\n`, and `\r` to be percent-encoded.
pub fn escape(s: &str) -> String {
    s.replace('%', "%25")
        .replace('\n', "%0A")
        .replace('\r', "%0D")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn escape_percent_and_newlines() {
        assert_eq!(escape("100% done\nok\r"), "100%25 done%0Aok%0D");
    }

    #[test]
    fn escape_no_special_chars() {
        assert_eq!(escape("hello world"), "hello world");
    }
}
