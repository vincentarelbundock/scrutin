//! Plain-mode reporter: text output to stderr, watch loop, reruns, JUnit
//! sidecar, and DB persistence.

use std::collections::HashSet;
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

use anyhow::Result;

use super::super::style;

use scrutin_core::engine::protocol::{Finding, Message, file_display_name};
use scrutin_core::filter::apply_include_exclude;
use scrutin_core::metadata::RunMetadata;
use scrutin_core::project::config::Config;
use scrutin_core::project::package::Package;
use scrutin_core::report as junit;
use scrutin_core::storage::sqlite;

use super::{
    RunAccumulator, RunOutcome, RunStats,
    collect_failed_files, merge_deps_from_results,
    replace_results,
};

// ---------------------------------------------------------------------------
// Entry point
// ---------------------------------------------------------------------------

#[allow(clippy::too_many_arguments)]
pub async fn run(
    pkg: &Package,
    test_files: &[PathBuf],
    includes: &[String],
    excludes: &[String],
    n_workers: usize,
    watch: bool,
    mut dep_map: Option<std::collections::HashMap<String, Vec<String>>>,
    cfg: &Config,
    junit_path: Option<&Path>,
    depmap_stale: bool,
    is_full_suite: bool,
    run_metadata: &RunMetadata,
) -> Result<i32> {
    // Initial run (always happens).
    let mut last_exit = run_once(
        pkg, test_files, n_workers, cfg, junit_path, depmap_stale,
        is_full_suite, run_metadata, &mut dep_map,
    )
    .await?;

    if !watch {
        return Ok(last_exit);
    }

    // Watch loop: re-run affected tests on every file change.
    use scrutin_core::analysis::deps::{TestAction, resolve_tests};
    use scrutin_core::engine::watcher::{FileWatcher, unique_paths};

    let mut watcher = FileWatcher::new(pkg, cfg.watch.debounce_ms)?;
    let mut rx = watcher.rx.take().expect("watcher rx");

    eprintln!();
    eprintln!("{}", style::dim("watching for changes... (ctrl-c to exit)"));

    while let Some(events) = rx.recv().await {
        let paths = unique_paths(&events);
        if paths.is_empty() {
            continue;
        }

        let mut tests_to_run = Vec::new();
        let mut run_full = false;
        for changed in &paths {
            match resolve_tests(changed, pkg, dep_map.as_ref()) {
                TestAction::Run(files) => tests_to_run.extend(files),
                TestAction::FullSuite => {
                    run_full = true;
                    break;
                }
            }
        }

        let files_to_run = if run_full {
            let mut all = pkg.test_files().unwrap_or_default();
            apply_include_exclude(&mut all, includes, excludes);
            all
        } else {
            tests_to_run.sort();
            tests_to_run.dedup();
            tests_to_run
        };

        if !files_to_run.is_empty() {
            eprintln!();
            last_exit = run_once(
                pkg, &files_to_run, n_workers, cfg, junit_path,
                depmap_stale,
                run_full && includes.is_empty() && excludes.is_empty(),
                run_metadata,
                &mut dep_map,
            )
            .await?;
            eprintln!();
            eprintln!("{}", style::dim("watching for changes... (ctrl-c to exit)"));
        }
    }

    Ok(last_exit)
}

// ---------------------------------------------------------------------------
// Single run (initial attempt + reruns + persistence)
// ---------------------------------------------------------------------------

/// Execute one plain-mode run: initial attempt, reruns, JUnit writing, DB
/// persistence.
#[allow(clippy::too_many_arguments)]
async fn run_once(
    pkg: &Package,
    test_files: &[PathBuf],
    n_workers: usize,
    cfg: &Config,
    junit_path: Option<&Path>,
    depmap_stale: bool,
    is_full_suite: bool,
    run_metadata: &RunMetadata,
    dep_map: &mut Option<std::collections::HashMap<String, Vec<String>>>,
) -> Result<i32> {
    let timeout = Duration::from_millis(cfg.run.timeout_file_ms);
    let timeout_run = if cfg.run.timeout_run_ms > 0 {
        Some(Duration::from_millis(cfg.run.timeout_run_ms))
    } else {
        None
    };
    let max_fail = cfg.run.max_fail;

    let initial_quiet = cfg.run.reruns > 0;
    let (mut exit, mut all_results, mut elapsed) =
        run_via_engine(pkg, test_files, n_workers, timeout, timeout_run, cfg.run.fork_workers, max_fail, initial_quiet).await?;

    // Rerun loop: any file that failed/errored on attempt N is re-submitted
    // on attempt N+1, up to `cfg.run.reruns` extra attempts.
    let mut flaky_files: HashSet<String> = HashSet::new();
    let mut last_attempt_was_rerun = false;
    if cfg.run.reruns > 0 {
        for attempt in 1..=cfg.run.reruns {
            let failed_now = collect_failed_files(&all_results);
            if failed_now.is_empty() {
                break;
            }
            if cfg.run.reruns_delay > 0 {
                tokio::time::sleep(Duration::from_millis(cfg.run.reruns_delay)).await;
            }
            eprintln!(
                "{}",
                style::yellow(format!(
                    "rerun {}/{}: re-running {} failed file{}",
                    attempt,
                    cfg.run.reruns,
                    failed_now.len(),
                    if failed_now.len() == 1 { "" } else { "s" }
                ))
            );
            let to_run: Vec<PathBuf> = test_files
                .iter()
                .filter(|p| failed_now.contains(&file_display_name(p)))
                .cloned()
                .collect();
            let is_last_potential = attempt == cfg.run.reruns;
            let (_rexit, rresults, relapsed) = run_via_engine(
                pkg,
                &to_run,
                n_workers,
                timeout,
                timeout_run,
                cfg.run.fork_workers,
                max_fail,
                !is_last_potential,
            )
            .await?;
            elapsed += relapsed;
            last_attempt_was_rerun = true;
            let new_failed = collect_failed_files(&rresults);
            for rec in &rresults {
                if !new_failed.contains(&rec.file) {
                    flaky_files.insert(rec.file.clone());
                }
            }
            replace_results(&mut all_results, rresults);
            exit = RunAccumulator::from_results(&all_results).failed_files;
        }
    }
    if cfg.run.reruns > 0 && !last_attempt_was_rerun {
        let stats = RunAccumulator::from_results(&all_results).into_stats(elapsed);
        print_summary(&stats, true);
    }

    if !flaky_files.is_empty() {
        eprintln!(
            "{}",
            style::yellow(format!(
                "{} file{} passed on rerun (flaky): {}",
                flaky_files.len(),
                if flaky_files.len() == 1 { "" } else { "s" },
                {
                    let mut v: Vec<&String> = flaky_files.iter().collect();
                    v.sort();
                    v.iter().map(|s| s.as_str()).collect::<Vec<_>>().join(", ")
                }
            ))
        );
    }

    if let Some(junit_path) = junit_path {
        let md_for_junit = if run_metadata.is_empty() { None } else { Some(run_metadata) };
        let suites_for_junit: Vec<(String, Vec<Message>)> = all_results
            .iter()
            .map(|r| (r.file.clone(), r.messages.to_vec()))
            .collect();
        if let Err(e) = junit::write_report(
            junit_path,
            &suites_for_junit,
            elapsed.as_secs_f64(),
            &flaky_files,
            md_for_junit,
        ) {
            eprintln!("Warning: failed to write JUnit XML: {}", e);
        } else {
            eprintln!(
                "{}",
                style::dim(format!("JUnit report written to {}", junit_path.display()))
            );
        }
    }

    // Record run history via SQLite (best-effort).
    super::persist_run(pkg, &all_results, &flaky_files, run_metadata);
    print_flaky_warnings(&pkg.root);

    // Merge runtime dep observations from instrumentation. The SQLite write
    // happens inside merge_deps_from_results; the in-memory map is kept in
    // sync so this run's remaining watch-mode decisions see the new edges.
    {
        let map = dep_map.get_or_insert_with(std::collections::HashMap::new);
        merge_deps_from_results(map, &all_results, &pkg.root);
    }

    // Rebuild Python side of the dep map if stale (R deps are updated
    // incrementally via instrumentation above).
    super::maybe_rebuild_depmap(pkg, is_full_suite, depmap_stale);

    Ok(if exit > 0 { 1 } else { 0 })
}

// ---------------------------------------------------------------------------
// Engine loop
// ---------------------------------------------------------------------------

/// Drive a run through `run_events::start_run` and accumulate results.
async fn run_via_engine(
    pkg: &Package,
    test_files: &[PathBuf],
    n_workers: usize,
    timeout: Duration,
    timeout_run: Option<Duration>,
    fork_workers: bool,
    max_fail: u32,
    quiet_summary: bool,
) -> Result<RunOutcome> {
    use scrutin_core::engine::run_events::{self, RunEvent};

    let n_files = test_files.len();
    let t0 = Instant::now();

    let (handle, mut rx) =
        run_events::start_run(pkg, test_files.to_vec(), n_workers, timeout, timeout_run, fork_workers, None).await?;
    let cancel = handle.cancel.clone();

    let mut acc = RunAccumulator::default();
    let mut tripped = false;
    while let Some(ev) = rx.recv().await {
        match ev {
            RunEvent::FileFinished(result) => {
                let file_name = file_display_name(&result.file);
                if tripped {
                    // Post-trip arrivals are work that was already in flight
                    // when max_fail fired and couldn't be preempted before
                    // emitting results. Record the file as cancelled with
                    // no events so it doesn't inflate the failure count or
                    // the bad-file tally; the run's failure totals reflect
                    // the "stop after K bad files" intent.
                    acc.push(file_name, vec![Message::Done], true);
                    continue;
                }
                acc.push(file_name, result.messages, result.cancelled);
                if max_fail > 0 && acc.failed_files >= max_fail {
                    tripped = true;
                    eprintln!(
                        "{}",
                        style::dim(format!(
                            "max-fail reached ({}), cancelling in-flight workers.",
                            max_fail
                        ))
                    );
                    cancel.cancel_all();
                }
            }
            RunEvent::FileStarted(_) => {}
            RunEvent::Complete => break,
        }
    }

    Ok(finalize(acc, n_files, t0.elapsed(), quiet_summary))
}

// ---------------------------------------------------------------------------
// Rendering
// ---------------------------------------------------------------------------

/// Finalize a run: sort findings, print per-file results and summary,
/// return the `RunOutcome` tuple.
fn finalize(
    mut acc: RunAccumulator,
    n_files: usize,
    elapsed: Duration,
    quiet_summary: bool,
) -> RunOutcome {
    let failed_files = acc.failed_files;
    let all_results = std::mem::take(&mut acc.all_results);
    if !quiet_summary {
        // Print buffered per-file results sorted by name for stable output.
        acc.failed_details.sort_by(|a, b| a.file.cmp(&b.file));
        acc.warn_details.sort_by(|a, b| a.file.cmp(&b.file));
        // Plain mode: merge failures and warnings into one list
        // sorted by file so all findings for a file are grouped.
        let mut combined: Vec<(&Finding, FindingKind)> = Vec::new();
        for f in &acc.failed_details {
            combined.push((f, FindingKind::Failure));
        }
        for f in &acc.warn_details {
            combined.push((f, FindingKind::Warning));
        }
        combined.sort_by(|a, b| a.0.file.cmp(&b.0.file));
        if !combined.is_empty() {
            eprintln!();
            let mut prev_file: Option<&str> = None;
            for (d, kind) in &combined {
                if prev_file.is_some() && prev_file != Some(&d.file) {
                    eprintln!();
                }
                prev_file = Some(&d.file);
                let location = if let Some(line) = d.line {
                    format!("{}:{}", d.file, line)
                } else {
                    d.file.clone()
                };
                eprintln!("{} {} > {}", kind.label(), location, d.test);
                if !d.message.is_empty() {
                    for line in d.message.lines() {
                        eprintln!("  {}", line);
                    }
                }
            }
        }
    }
    let mut stats = acc.into_stats(elapsed);
    stats.n_files = n_files as u32;
    if !quiet_summary {
        print_summary(&stats, true);
    }

    (failed_files, all_results, elapsed)
}

#[derive(Clone, Copy)]
enum FindingKind {
    Failure,
    Warning,
}

impl FindingKind {
    fn label(self) -> &'static str {
        match self {
            FindingKind::Failure => "FAIL",
            FindingKind::Warning => "WARN",
        }
    }
}

pub fn print_summary(s: &RunStats, ci: bool) {
    let bad = s.failed + s.errored;
    let executed = s.passed + s.failed + s.errored;
    let pass_rate = if executed > 0 {
        (s.passed as f64 / executed as f64) * 100.0
    } else {
        0.0
    };
    let avg_file_ms = if s.n_files > 0 {
        s.elapsed.as_millis() as f64 / s.n_files as f64
    } else {
        0.0
    };

    eprintln!();
    if ci {
        eprintln!(
            "{} passed, {} failed, {} errored, {} warned, {} skipped ({:.2}s)",
            s.passed,
            s.failed,
            s.errored,
            s.warned,
            s.skipped,
            s.elapsed.as_secs_f64()
        );
        eprintln!(
            "pass rate {:.1}%  ·  {} files  ·  avg {:.0}ms/file{}",
            pass_rate,
            s.n_files,
            avg_file_ms,
            match &s.slowest {
                Some((f, ms)) => format!("  ·  slowest {} ({}ms)", f, ms),
                None => String::new(),
            }
        );
    } else {
        let summary = format!(
            "{} passed  {} failed  {} errored  {} warned  {} skipped  {:.2}s",
            s.passed,
            s.failed,
            s.errored,
            s.warned,
            s.skipped,
            s.elapsed.as_secs_f64()
        );
        if bad > 0 {
            eprintln!("{}", style::red_bold(&summary));
        } else if s.warned > 0 {
            eprintln!("{}", style::yellow_bold(&summary));
        } else {
            eprintln!("{}", style::green_bold(&summary));
        }
        let line = format!(
            "pass rate {:.1}%  ·  {} files  ·  avg {:.0}ms/file{}",
            pass_rate,
            s.n_files,
            avg_file_ms,
            match &s.slowest {
                Some((f, ms)) => format!("  ·  slowest {} ({}ms)", f, ms),
                None => String::new(),
            }
        );
        eprintln!("{}", style::dim(line));
    }
}

fn print_flaky_warnings(root: &Path) {
    let Ok(flaky) = sqlite::with_open(root, |c| sqlite::flaky_tests(c)) else {
        return;
    };
    if !flaky.is_empty() {
        eprintln!();
        eprintln!(
            "{} {} flaky test{} detected (run `scrutin stats` for details)",
            style::yellow("⚠"),
            flaky.len(),
            if flaky.len() == 1 { "" } else { "s" }
        );
    }
}

/// Translate this run's [`FileRecord`]s into SQLite `ResultRow`s, keyed
/// by tool. The `retries` column reflects whether the file was re-executed:
/// any file present in `flaky_files` succeeded on a rerun within this run
/// (so it had at least one retry).
///
/// `tool_version` and `app_version` are resolved once per tool at call time
/// (one subprocess per active plugin, not one per row) and looked up by
/// tool name when stamping each row.
pub(crate) fn build_result_rows(
    pkg: &Package,
    all_results: &[super::FileRecord],
    flaky_files: &HashSet<String>,
) -> Vec<sqlite::ResultRow> {
    // `r.file` is a basename (see `file_display_name`). To recover the
    // owning suite we look up the full path via `pkg.test_files()`, which
    // returns the canonical absolute paths of every test file, then
    // `suite_for` can apply the `run`-glob-aware predicate. This avoids
    // false positives when two plugins accept the same basename pattern
    // (e.g. testthat owns `tests/testthat/test-*.R` while tinytest owns
    // `inst/tinytest/test_*.R`, but the extension alone is ambiguous).
    let files_by_name: std::collections::HashMap<String, std::path::PathBuf> = pkg
        .test_files()
        .unwrap_or_default()
        .into_iter()
        .filter_map(|p| {
            p.file_name()
                .and_then(|n| n.to_str())
                .map(|s| (s.to_string(), p.clone()))
        })
        .collect();

    // Resolve tool / app versions once per active suite. Each plugin is
    // queried at most once; misses (tool not installed, parse failures)
    // become `None` and the DB row gets a NULL.
    let mut tool_versions: std::collections::HashMap<String, Option<String>> =
        std::collections::HashMap::new();
    let mut app_versions: std::collections::HashMap<String, Option<String>> =
        std::collections::HashMap::new();
    for suite in &pkg.test_suites {
        let name = suite.plugin.name().to_string();
        tool_versions
            .entry(name.clone())
            .or_insert_with(|| suite.plugin.tool_version(&suite.root));
        app_versions
            .entry(name)
            .or_insert_with(|| suite.plugin.project_version(&suite.root));
    }

    all_results
        .iter()
        .map(|r| {
            let tool = files_by_name
                .get(&r.file)
                .and_then(|p| pkg.suite_for(p))
                .map(|s| s.plugin.name().to_string())
                .unwrap_or_default();
            let retries = if flaky_files.contains(&r.file) { 1 } else { 0 };
            let tool_version = tool_versions.get(&tool).cloned().unwrap_or(None);
            let app_version = app_versions.get(&tool).cloned().unwrap_or(None);
            sqlite::ResultRow {
                file: r.file.clone(),
                tool,
                tool_version,
                app_name: Some(pkg.name.clone()),
                app_version,
                messages: r.messages.clone(),
                retries,
            }
        })
        .collect()
}
