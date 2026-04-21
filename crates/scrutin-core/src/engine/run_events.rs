//! Run lifecycle event seam.
//!
//! Boundary between the test-run engine (`pool`/`runner`) and any consumer
//! (TUI today, JSON event stream / web view tomorrow). Consumers call
//! [`start_run`], get back a [`RunHandle`] plus a receiver of [`RunEvent`]s,
//! and never need to touch `ProcessPool` directly.
//!
//! Multi-suite handling lives here. `start_run` partitions the input file
//! list by which `TestSuite` owns each file, spawns one `ProcessPool` per
//! suite that has any work to do, and multiplexes their results into a
//! single event channel. Every spawned pool shares one [`CancelHandle`] so
//! `cancel.cancel_all()` propagates across all of them.

use std::path::PathBuf;
use std::sync::Arc;
use std::time::Duration;

use anyhow::Result;
use tokio::sync::mpsc;

use crate::engine::command_pool::CommandPool;
use crate::engine::pool::{BusyCounter, CancelHandle, ForkPool, ProcessPool};
use crate::engine::protocol::{Event, Message};
use crate::logbuf::LogBuffer;
use crate::project::package::{Package, TestSuite};
use crate::r::LoadStrategy;

pub struct FileResult {
    pub file: PathBuf,
    pub messages: Vec<Message>,
    /// True iff the engine killed this file's worker mid-run (TUI cancel
    /// keys, `--max-fail` tripping). The wire protocol has no `cancelled`
    /// message: cancellation is engine-side only, attached here so
    /// consumers can distinguish "intentionally cut short" from "errored".
    pub cancelled: bool,
}

impl FileResult {
    /// Extract the runtime dependency observation from this file's messages,
    /// if present. Returns `(test_basename, [source_rel_paths])`.
    pub fn deps(&self) -> Option<(&str, &[String])> {
        self.messages.iter().find_map(|m| match m {
            Message::Deps(d) => Some((d.file.as_str(), d.sources.as_slice())),
            _ => None,
        })
    }
}

pub enum RunEvent {
    FileStarted(PathBuf),
    FileFinished(FileResult),
    Complete,
}

/// Internal message type sent from pool tasks to the run-events forwarder.
/// Separates the "this file started" signal (needed for live status) from
/// the "this file finished" payload so consumers can show a spinner for
/// in-progress files without conflating them with pending ones.
pub(crate) enum PoolMsg {
    FileStarted(PathBuf),
    FileFinished(FileResult),
}

#[derive(Clone)]
pub struct RunHandle {
    pub busy: BusyCounter,
    pub cancel: CancelHandle,
}

/// A pool that can run tests: either a long-lived worker pool or a
/// lightweight command-mode pool. Abstraction kept minimal (enum dispatch)
/// since only `run_tests` is needed.
enum Pool {
    Fork(Arc<ForkPool>),
    Worker(Arc<ProcessPool>),
    Command(Arc<CommandPool>),
}

impl Pool {
    async fn run_tests(&self, files: &[PathBuf], tx: mpsc::UnboundedSender<PoolMsg>) {
        match self {
            Pool::Fork(p) => p.run_tests(files, tx).await,
            Pool::Worker(p) => p.run_tests(files, tx).await,
            Pool::Command(p) => p.run_tests(files, tx).await,
        }
    }
}

pub async fn start_run(
    pkg: &Package,
    files: Vec<PathBuf>,
    n_workers: usize,
    timeout: Duration,
    timeout_run: Option<Duration>,
    fork_workers: bool,
    log: Option<LogBuffer>,
) -> Result<(RunHandle, mpsc::UnboundedReceiver<RunEvent>)> {
    // Partition files by the suite that owns each one. Orphan files (no
    // suite claims them) are surfaced as warnings rather than dropped
    // silently: discovery-driven callers can't trip this, but a future
    // LSP/web frontend that takes user-clicked paths could.
    let mut buckets: Vec<Vec<PathBuf>> = (0..pkg.test_suites.len()).map(|_| Vec::new()).collect();
    let mut orphans: Vec<PathBuf> = Vec::new();
    for f in files {
        match pkg.test_suites.iter().position(|s| s.owns_test_file(&f)) {
            Some(idx) => buckets[idx].push(f),
            None => orphans.push(f),
        }
    }
    if !orphans.is_empty() {
        if let Some(ref lb) = log {
            for o in &orphans {
                lb.push(
                    "engine",
                    &format!("warning: no suite owns {}; skipping\n", o.display()),
                );
            }
        } else {
            for o in &orphans {
                eprintln!("warning: no suite owns {}; skipping", o.display());
            }
        }
    }

    // Local mutable copy of the package's suites so we can inject per-run
    // env vars (e.g. `R_LIBS_USER` from a pre-pool `R CMD INSTALL`). The
    // pool constructors borrow these by reference, and the kept tempdirs
    // are moved into the spawned task below so they outlive the run.
    let mut suites: Vec<TestSuite> = pkg.test_suites.clone();
    let mut install_tempdirs: Vec<tempfile::TempDir> = Vec::new();
    // Synthesized FileResults for install failures, drained at the start
    // of the spawned task so frontends see them before any worker output.
    let mut preflight_failures: Vec<FileResult> = Vec::new();

    for (idx, files_for_suite) in buckets.iter().enumerate() {
        if files_for_suite.is_empty() {
            continue;
        }
        let suite = &suites[idx];
        if suite.plugin.language() != "r" || suite.r_load != LoadStrategy::Install {
            continue;
        }
        match install_r_package_to_tempdir(suite, log.as_ref()) {
            Ok(td) => {
                let lib_path = td.path().to_string_lossy().into_owned();
                suites[idx]
                    .extra_env
                    .push(("R_LIBS_USER".into(), lib_path));
                install_tempdirs.push(td);
            }
            Err(err) => {
                let msg = format!(
                    "R CMD INSTALL failed for suite '{}': {}",
                    suite.plugin.name(),
                    err
                );
                if let Some(ref lb) = log {
                    lb.push("engine", &format!("{}\n", msg));
                }
                for f in files_for_suite {
                    let basename = f
                        .file_name()
                        .and_then(|n| n.to_str())
                        .unwrap_or("")
                        .to_string();
                    preflight_failures.push(FileResult {
                        file: f.clone(),
                        messages: vec![Message::Event(Event::engine_error(
                            basename,
                            msg.clone(),
                        ))],
                        cancelled: false,
                    });
                }
            }
        }
    }

    // Drop bucket entries whose install failed, so we don't spin up a pool
    // that would just fail per-file all over again.
    let install_failed: std::collections::HashSet<usize> = preflight_failures
        .iter()
        .filter_map(|fr| {
            suites
                .iter()
                .position(|s| s.owns_test_file(&fr.file))
        })
        .collect();
    let buckets: Vec<Vec<PathBuf>> = buckets
        .into_iter()
        .enumerate()
        .map(|(idx, b)| if install_failed.contains(&idx) { Vec::new() } else { b })
        .collect();

    // Spawn one pool per non-empty suite. Every pool shares one
    // `CancelHandle` (so a single `cancel_all()` fans out) and one
    // `BusyCounter` (so the frontend's "running N workers" indicator
    // reflects the *total* in-flight count across every suite, not just
    // the first suite to spawn).
    let shared_cancel = CancelHandle::default();
    let shared_busy = BusyCounter::default();
    let mut pools: Vec<(Pool, Vec<PathBuf>)> = Vec::new();
    for (idx, files_for_suite) in buckets.into_iter().enumerate() {
        if files_for_suite.is_empty() {
            continue;
        }
        let suite = &suites[idx];

        // Command-mode plugins (jarl, ruff) get a lightweight pool that
        // spawns one short-lived command per file. Worker-mode plugins
        // (testthat, pytest, ...) get the full ProcessPool with warm
        // long-lived subprocesses.
        let pool = if suite.plugin.command_spec(&suite.root, pkg).is_some() {
            Pool::Command(Arc::new(CommandPool::new(
                pkg,
                suite,
                n_workers,
                timeout,
                log.clone(),
                shared_cancel.clone(),
                shared_busy.clone(),
            )))
        } else if fork_workers && !cfg!(target_os = "windows") {
            Pool::Fork(Arc::new(
                ForkPool::new(
                    pkg,
                    suite,
                    n_workers,
                    timeout,
                    log.clone(),
                    Some(shared_cancel.clone()),
                    Some(shared_busy.clone()),
                )
                .await?,
            ))
        } else {
            Pool::Worker(Arc::new(
                ProcessPool::with_timeout_and_log(
                    pkg,
                    suite,
                    n_workers,
                    timeout,
                    false,
                    log.clone(),
                    Some(shared_cancel.clone()),
                    Some(shared_busy.clone()),
                )
                .await?,
            ))
        };
        pools.push((pool, files_for_suite));
    }

    let handle = RunHandle {
        busy: shared_busy,
        cancel: shared_cancel,
    };

    let (event_tx, event_rx) = mpsc::unbounded_channel::<RunEvent>();

    let cancel_for_task = handle.cancel.clone();
    // Move tempdirs into the task so they're dropped (and the temp R
    // libraries deleted) only after the run completes.
    let _kept_tempdirs = install_tempdirs;
    tokio::spawn(async move {
        // Keep tempdirs alive for the run's duration.
        let _kept_tempdirs = _kept_tempdirs;
        let (result_tx, mut result_rx) = mpsc::unbounded_channel::<PoolMsg>();

        // Surface install failures up front, before any worker output, so
        // the frontend shows them as ordinary file results.
        for fr in preflight_failures {
            let _ = result_tx.send(PoolMsg::FileFinished(fr));
        }

        // Run suites sequentially so each suite gets the full worker pool.
        // This avoids spawning N workers x M suites simultaneously (each
        // paying the load_all/warm-up cost). Instead, one suite's workers
        // start, run all its files, and shut down before the next suite
        // begins. Command-mode pools (jarl, ruff) are lightweight, so the
        // savings come from worker-mode suites (testthat, pytest, ...).
        //
        // Each suite still uses all N workers internally for parallelism.
        // The result_tx/result_rx channel stays open across suites so the
        // forwarding loop below sees a continuous stream of FileResults.
        let result_tx_for_suites = result_tx.clone();
        let cancel_for_suites = cancel_for_task.clone();
        let suite_runner = tokio::spawn(async move {
            for (pool, files_for_suite) in pools {
                if cancel_for_suites.is_all_cancelled() {
                    break;
                }
                let tx = result_tx_for_suites.clone();
                pool.run_tests(&files_for_suite, tx).await;
            }
        });
        // Drop the original sender so the channel closes once the sequential
        // suite runner finishes.
        drop(result_tx);

        // Whole-run timeout: if configured, fire cancel_all() when the
        // budget expires. The per-file forwarding loop below races against
        // this deadline. When no run timeout is set, the deadline future
        // pends forever so the select! always takes the recv branch.
        let deadline = async {
            match timeout_run {
                Some(d) => tokio::time::sleep(d).await,
                None => std::future::pending().await,
            }
        };
        tokio::pin!(deadline);

        loop {
            tokio::select! {
                msg = result_rx.recv() => {
                    match msg {
                        Some(PoolMsg::FileStarted(path)) => {
                            if event_tx.send(RunEvent::FileStarted(path)).is_err() {
                                cancel_for_task.cancel_all();
                                break;
                            }
                        }
                        Some(PoolMsg::FileFinished(r)) => {
                            if event_tx.send(RunEvent::FileFinished(r)).is_err() {
                                // Consumer dropped the receiver: cancel
                                // everything in flight so workers stop
                                // spawning new subprocesses.
                                cancel_for_task.cancel_all();
                                break;
                            }
                        }
                        None => break, // all pools finished
                    }
                }
                _ = &mut deadline => {
                    cancel_for_task.cancel_all();
                    break;
                }
            }
        }

        let _ = suite_runner.await;
        let _ = event_tx.send(RunEvent::Complete);
    });

    Ok((handle, event_rx))
}

/// Install an R package source tree into a fresh temp library via
/// `R CMD INSTALL`. Returns the kept `TempDir` (caller is responsible for
/// keeping it alive — when dropped, the lib is wiped).
///
/// Synchronous on purpose: this is a one-shot pre-pool step that gates the
/// entire run. Workers can't usefully start until it finishes.
fn install_r_package_to_tempdir(
    suite: &TestSuite,
    log: Option<&LogBuffer>,
) -> Result<tempfile::TempDir> {
    let td = tempfile::Builder::new()
        .prefix("scrutin-r-lib-")
        .tempdir()
        .map_err(|e| anyhow::anyhow!("could not create temp R library dir: {e}"))?;
    let lib = td.path().to_string_lossy().into_owned();
    let root = suite.root.to_string_lossy().into_owned();

    if let Some(lb) = log {
        lb.push(
            "engine",
            &format!("R CMD INSTALL --library={} {}\n", lib, root),
        );
    }

    let output = std::process::Command::new("R")
        .arg("CMD")
        .arg("INSTALL")
        .arg("--no-multiarch")
        .arg("--no-test-load")
        .arg(format!("--library={}", lib))
        .arg(&root)
        .output()
        .map_err(|e| anyhow::anyhow!("could not spawn R CMD INSTALL: {e}"))?;

    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        let stdout = String::from_utf8_lossy(&output.stdout);
        // Last few lines of stderr is usually the actionable bit.
        let tail: String = stderr
            .lines()
            .rev()
            .take(20)
            .collect::<Vec<_>>()
            .into_iter()
            .rev()
            .collect::<Vec<_>>()
            .join("\n");
        let combined = if tail.is_empty() { stdout.into_owned() } else { tail };
        anyhow::bail!("exit {}\n{}", output.status, combined);
    }

    Ok(td)
}
