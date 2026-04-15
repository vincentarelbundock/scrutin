use std::collections::{HashSet, VecDeque};
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::Mutex as StdMutex;
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::time::Duration;

use anyhow::Result;
use tokio::io::{AsyncBufReadExt, AsyncWriteExt};
use tokio::net::TcpListener;
use tokio::sync::{Notify, Semaphore, mpsc};
use tokio::task::JoinSet;

use crate::engine::protocol::{Event, Message};
use crate::engine::run_events::FileResult;
use crate::engine::runner::RProcess;
use crate::logbuf::LogBuffer;
use crate::project::package::{Package, TestSuite};

/// Floor for `default_workers()`: tests benefit from at least one paired
/// worker (read+write side of a fixture, etc.) even on single-core boxes.
const MIN_DEFAULT_WORKERS: usize = 2;

/// Ceiling for `default_workers()`. Past 8, R/Python workers start to fight
/// for cache and the per-worker memory cost dominates. Users who really
/// want more can set `[run] workers` in .scrutin/config.toml.
const MAX_DEFAULT_WORKERS: usize = 8;

/// Shared cancellation state for an in-flight run. The pool checks the
/// global flag and the per-file set between worker steps; the TUI sets
/// these from the key handler when the user presses `x` / `X`.
#[derive(Default)]
struct CancelState {
    all: AtomicBool,
    files: StdMutex<HashSet<PathBuf>>,
    notify: Notify,
}

#[derive(Clone, Default)]
pub struct CancelHandle {
    inner: Arc<CancelState>,
}

impl CancelHandle {
    pub fn cancel_all(&self) {
        self.inner.all.store(true, Ordering::Relaxed);
        self.inner.notify.notify_waiters();
    }

    pub fn cancel_file(&self, path: &Path) {
        if let Ok(mut set) = self.inner.files.lock() {
            set.insert(path.to_path_buf());
        }
        self.inner.notify.notify_waiters();
    }

    pub fn is_all_cancelled(&self) -> bool {
        self.inner.all.load(Ordering::Relaxed)
    }

    pub fn is_file_cancelled(&self, path: &Path) -> bool {
        self.is_all_cancelled()
            || self
                .inner
                .files
                .lock()
                .map(|s| s.contains(path))
                .unwrap_or(false)
    }

    /// Resolves when any cancellation event is signalled.
    async fn cancelled(&self) {
        self.inner.notify.notified().await
    }
}

/// Async process pool.
///
/// A queue of warm workers (`workers: VecDeque<RProcess>`) protected by a
/// short-held std mutex, with a `Semaphore` signalling availability. Tasks
/// `acquire` a permit (awaits if every worker is busy), then pop the next
/// free worker from the queue, run the test, and push the worker back.
///
/// This replaces the previous round-robin assignment + per-worker mutex
/// scheme. The old design pre-bound files to specific workers, so two slow
/// files assigned to the same worker would serialize on its mutex even when
/// other workers were idle. With the queue, *any* free worker handles the
/// next file, eliminating the imbalance.
pub struct ProcessPool {
    workers: Arc<StdMutex<VecDeque<RProcess>>>,
    /// Permit count == number of workers currently in the queue. Acquiring
    /// a permit guarantees the queue has at least one worker to pop.
    available: Arc<Semaphore>,
    pkg: Arc<Package>,
    suite: Arc<TestSuite>,
    timeout: Duration,
    busy: BusyCounter,
    log: Option<LogBuffer>,
    cancel: CancelHandle,
    /// When true (default), workers stay alive and fork() per test file for
    /// COW-based isolation. When false, workers are killed and respawned
    /// after every file.
    fork_workers: bool,
    /// Set the first time any worker reports a `<worker_startup>` error.
    /// Once poisoned, no further respawns are attempted and every subsequent
    /// file synthesizes an Error message instead.
    poisoned: Arc<AtomicBool>,
    poison_msg: Arc<StdMutex<String>>,
}

/// Live counter of currently-busy workers. Cloneable so multiple pools can
/// share one counter — `run_events::start_run` constructs one counter for
/// the whole run and hands it to every per-suite pool, so the value the
/// frontend reads is the *total* number of in-flight workers across all
/// suites, not just the first one.
#[derive(Clone, Default)]
pub struct BusyCounter(Arc<AtomicUsize>);

impl BusyCounter {
    pub fn get(&self) -> usize {
        self.0.load(Ordering::Relaxed)
    }
    pub fn inc(&self) {
        self.0.fetch_add(1, Ordering::Relaxed);
    }
    pub fn dec(&self) {
        self.0.fetch_sub(1, Ordering::Relaxed);
    }
}

impl ProcessPool {
    pub async fn with_timeout_and_log(
        pkg: &Package,
        suite: &TestSuite,
        n_workers: usize,
        timeout: Duration,
        fork_workers: bool,
        log: Option<LogBuffer>,
        shared_cancel: Option<CancelHandle>,
        shared_busy: Option<BusyCounter>,
    ) -> Result<Self> {
        let pkg = Arc::new(pkg.clone());
        let suite = Arc::new(suite.clone());
        let mut queue: VecDeque<RProcess> = VecDeque::with_capacity(n_workers);
        for _ in 0..n_workers {
            let proc =
                RProcess::spawn_with_timeout_and_log(&pkg, &suite, timeout, fork_workers, log.clone()).await?;
            queue.push_back(proc);
        }

        Ok(ProcessPool {
            workers: Arc::new(StdMutex::new(queue)),
            available: Arc::new(Semaphore::new(n_workers)),
            pkg,
            suite,
            timeout,
            fork_workers,
            busy: shared_busy.unwrap_or_default(),
            log,
            cancel: shared_cancel.unwrap_or_default(),
            poisoned: Arc::new(AtomicBool::new(false)),
            poison_msg: Arc::new(StdMutex::new(String::new())),
        })
    }

    pub fn busy_counter(&self) -> BusyCounter {
        self.busy.clone()
    }

    pub async fn run_tests(&self, test_files: &[PathBuf], tx: mpsc::UnboundedSender<FileResult>) {
        let mut set: JoinSet<()> = JoinSet::new();

        // Each spawned task acquires a semaphore permit (which awaits if
        // every worker is currently busy), pops *any* free worker from the
        // shared queue, runs the test, and pushes the worker back. The pop
        // is guaranteed to succeed because permits and queue size are kept
        // in lockstep.
        for test_file in test_files {
            let workers = self.workers.clone();
            let available = self.available.clone();
            let busy = self.busy.clone();
            let pkg = self.pkg.clone();
            let suite = self.suite.clone();
            let timeout = self.timeout;
            let log = self.log.clone();
            let tx = tx.clone();
            let file = test_file.clone();
            let cancel = self.cancel.clone();
            let fork_workers = self.fork_workers;
            let poisoned = self.poisoned.clone();
            let poison_msg = self.poison_msg.clone();

            set.spawn(async move {
                let permit = match available.acquire_owned().await {
                    Ok(p) => p,
                    Err(_) => return,
                };

                // Pool-wide poison: a worker_startup hook has failed. Don't
                // waste a worker; synthesize an error, return the permit
                // (drop) without ever popping from the queue.
                if poisoned.load(Ordering::Relaxed) {
                    let msg = poison_msg
                        .lock()
                        .map(|s| s.clone())
                        .unwrap_or_else(|_| "worker startup hook failed".to_string());
                    let _ = tx.send(startup_error_result(&file, &msg));
                    drop(permit);
                    return;
                }

                // Pre-flight: if cancelled before we even popped a worker,
                // synthesize a Cancelled result and skip.
                if cancel.is_file_cancelled(&file) {
                    let _ = tx.send(cancelled_result(&file));
                    drop(permit);
                    return;
                }

                // Pop the next free worker. Holding a permit guarantees
                // there is one — the std mutex is briefly held just for the
                // VecDeque op, never across an await.
                let mut worker = match workers.lock().unwrap().pop_front() {
                    Some(w) => w,
                    None => {
                        // Should be unreachable: a permit implies a worker.
                        // Bail defensively rather than unwrap-panic.
                        let _ = tx.send(FileResult {
                            file: file.clone(),
                            messages: vec![Message::Event(Event::engine_error(
                                file.file_name()
                                    .unwrap_or_default()
                                    .to_string_lossy()
                                    .to_string(),
                                "internal: worker queue empty under permit",
                            ))],
                            cancelled: false,
                        });
                        drop(permit);
                        return;
                    }
                };
                busy.inc();

                // Re-check after acquiring the worker — the user may have
                // cancelled while we were waiting for the queue.
                if cancel.is_file_cancelled(&file) {
                    busy.dec();
                    workers.lock().unwrap().push_back(worker);
                    drop(permit);
                    let _ = tx.send(cancelled_result(&file));
                    return;
                }

                // Race the test run against cancellation. On cancel, the
                // run_fut is dropped (cancel-safe at await points) and we
                // kill + respawn the worker so the next file gets a clean
                // child.
                let outcome = {
                    let run_fut = worker.run_test(&file);
                    tokio::select! {
                        biased;
                        _ = wait_for_cancel(&cancel, &file) => Outcome::Cancelled,
                        r = run_fut => Outcome::Finished(r),
                    }
                };

                let messages = match outcome {
                    Outcome::Cancelled => {
                        worker.kill().await;
                        if let Ok(new_proc) =
                            RProcess::spawn_with_timeout_and_log(&pkg, &suite, timeout, fork_workers, log.clone())
                                .await
                        {
                            worker = new_proc;
                        }
                        busy.dec();
                        workers.lock().unwrap().push_back(worker);
                        drop(permit);
                        let _ = tx.send(cancelled_result(&file));
                        return;
                    }
                    Outcome::Finished(Ok(msgs)) => msgs,
                    Outcome::Finished(Err(e)) => {
                        let msg = e.to_string();
                        // Worker-startup hook failure: poison the pool
                        // and abort. Do NOT respawn — the new worker
                        // would hit the same broken hook.
                        if let Some(rest) = msg.strip_prefix("WORKER_STARTUP_FAILED: ") {
                            poisoned.store(true, Ordering::Relaxed);
                            if let Ok(mut slot) = poison_msg.lock()
                                && slot.is_empty()
                            {
                                *slot = rest.to_string();
                            }
                            cancel.cancel_all();
                            busy.dec();
                            workers.lock().unwrap().push_back(worker);
                            drop(permit);
                            let _ = tx.send(startup_error_result(&file, rest));
                            return;
                        }
                        // Worker is likely dead — try to respawn it.
                        if let Ok(new_proc) =
                            RProcess::spawn_with_timeout_and_log(&pkg, &suite, timeout, fork_workers, log.clone())
                                .await
                        {
                            worker = new_proc;
                        }
                        vec![Message::Event(Event::engine_error(
                            file.file_name()
                                .unwrap_or_default()
                                .to_string_lossy()
                                .to_string(),
                            msg,
                        ))]
                    }
                };

                // Kill and respawn the worker when fork mode is off.
                // With fork mode, the worker stays alive and fork()s per
                // file internally, so isolation is handled in R/Python.
                if !fork_workers {
                    worker.kill().await;
                    if let Ok(new_proc) =
                        RProcess::spawn_with_timeout_and_log(&pkg, &suite, timeout, fork_workers, log.clone())
                            .await
                    {
                        worker = new_proc;
                    }
                }

                busy.dec();
                workers.lock().unwrap().push_back(worker);
                drop(permit);
                let _ = tx.send(FileResult {
                    file,
                    messages,
                    cancelled: false,
                });
            });
        }

        // Drain task results so we own all panics. A panicking worker task
        // would otherwise be silently swallowed by the JoinSet.
        while let Some(joined) = set.join_next().await {
            if let Err(e) = joined
                && e.is_panic()
                && let Some(ref lb) = self.log
            {
                lb.push(
                    self.suite.plugin.name(),
                    &format!("worker task panicked: {e}\n"),
                );
            }
        }
    }

    pub fn default_workers() -> usize {
        // Physical cores, not logical: R/Python tests are CPU-bound on real
        // cores; SMT/hyperthreads add little and fight for cache. The upper
        // cap limits per-worker memory and DB-lock contention; users can
        // override via `[run] workers` in .scrutin/config.toml.
        std::thread::available_parallelism()
            .map(usize::from)
            .unwrap_or(MIN_DEFAULT_WORKERS)
            .clamp(MIN_DEFAULT_WORKERS, MAX_DEFAULT_WORKERS)
    }
}

enum Outcome {
    Finished(Result<Vec<Message>>),
    Cancelled,
}

/// Wakes whenever a cancellation event is signalled and re-checks whether
/// *this* file (or all files) should be cancelled. Returns when so.
async fn wait_for_cancel(cancel: &CancelHandle, file: &Path) {
    loop {
        if cancel.is_file_cancelled(file) {
            return;
        }
        cancel.cancelled().await;
    }
}

fn startup_error_result(file: &Path, msg: &str) -> FileResult {
    let name = file
        .file_name()
        .unwrap_or_default()
        .to_string_lossy()
        .to_string();
    FileResult {
        file: file.to_path_buf(),
        messages: vec![
            Message::Event(Event::engine_error(
                name,
                format!("worker_startup hook failed: {msg}"),
            )),
            Message::Done,
        ],
        cancelled: false,
    }
}

fn cancelled_result(file: &Path) -> FileResult {
    FileResult {
        file: file.to_path_buf(),
        messages: vec![Message::Done],
        cancelled: true,
    }
}

// ── ForkPool ───────────────────────────────────────────────────────────────
//
// Single warm parent process per suite. The parent loads the project once,
// then forks a child for each test file. Each child connects to Rust via
// TCP to deliver NDJSON results. TCP stream close = file done.
//
// Parallelism is controlled by a semaphore (N permits). The parent forks
// immediately on receiving a path (doesn't wait), so up to N children can
// run concurrently.

pub struct ForkPool {
    /// The warm parent process. We write file paths to its stdin.
    parent: Arc<tokio::sync::Mutex<RProcess>>,
    /// TCP listener that children connect to after fork.
    listener: Arc<TcpListener>,
    /// Controls max concurrent children (same as ProcessPool's semaphore).
    available: Arc<Semaphore>,
    suite: Arc<TestSuite>,
    timeout: Duration,
    busy: BusyCounter,
    log: Option<LogBuffer>,
    cancel: CancelHandle,
}

impl ForkPool {
    pub async fn new(
        pkg: &Package,
        suite: &TestSuite,
        n_workers: usize,
        timeout: Duration,
        log: Option<LogBuffer>,
        shared_cancel: Option<CancelHandle>,
        shared_busy: Option<BusyCounter>,
    ) -> Result<Self> {
        // Bind a TCP listener on a random loopback port.
        let listener = TcpListener::bind("127.0.0.1:0").await?;
        let port = listener.local_addr()?.port();

        // Spawn 1 parent process with SCRUTIN_TCP_PORT set.
        let parent = RProcess::spawn_fork_parent(
            pkg, suite, timeout, port, log.clone(),
        ).await?;

        Ok(ForkPool {
            parent: Arc::new(tokio::sync::Mutex::new(parent)),
            listener: Arc::new(listener),
            available: Arc::new(Semaphore::new(n_workers)),
            suite: Arc::new(suite.clone()),
            timeout,
            busy: shared_busy.unwrap_or_default(),
            log,
            cancel: shared_cancel.unwrap_or_default(),
        })
    }

    pub fn busy_counter(&self) -> BusyCounter {
        self.busy.clone()
    }

    pub async fn run_tests(&self, test_files: &[PathBuf], tx: mpsc::UnboundedSender<FileResult>) {
        let mut set: JoinSet<()> = JoinSet::new();

        for test_file in test_files {
            let parent = self.parent.clone();
            let listener = self.listener.clone();
            let available = self.available.clone();
            let busy = self.busy.clone();
            let timeout = self.timeout;
            let tx = tx.clone();
            let file = test_file.clone();
            let cancel = self.cancel.clone();

            set.spawn(async move {
                let permit = match available.acquire_owned().await {
                    Ok(p) => p,
                    Err(_) => return,
                };

                if cancel.is_file_cancelled(&file) {
                    drop(permit);
                    let _ = tx.send(cancelled_result(&file));
                    return;
                }

                busy.inc();

                // Write the file path to the parent's stdin AND accept the
                // child's TCP connection under the same lock. This serializes
                // the fork+connect handshake so that child connections cannot
                // be mixed up between tasks. Parallel execution still happens
                // during the NDJSON reading phase after accept returns.
                let stream = tokio::select! {
                    biased;
                    _ = wait_for_cancel(&cancel, &file) => Err(anyhow::anyhow!("cancelled")),
                    stream = async {
                        let mut p = parent.lock().await;
                        let path_str = format!("{}\n", file.to_string_lossy());
                        if let Some(stdin) = p.stdin_mut() {
                            let _ = stdin.write_all(path_str.as_bytes()).await;
                            let _ = stdin.flush().await;
                        }
                        match tokio::time::timeout(timeout, listener.accept()).await {
                            Ok(Ok((stream, _))) => Ok(stream),
                            Ok(Err(e)) => Err(anyhow::Error::from(e)),
                            Err(_) => Err(anyhow::anyhow!("timeout waiting for child TCP connection")),
                        }
                    } => stream,
                };

                // Read NDJSON from the TCP stream (lock is released, so
                // other tasks can fork+accept concurrently).
                let result = match stream {
                    Err(e) => Err(e),
                    Ok(stream) => {
                        let mut reader = tokio::io::BufReader::new(stream);
                        let mut messages = Vec::new();
                        let mut line = String::new();
                        loop {
                            line.clear();
                            match tokio::time::timeout(
                                timeout,
                                reader.read_line(&mut line),
                            ).await {
                                Ok(Ok(0)) => break, // EOF = child exited
                                Ok(Ok(_)) => {
                                    let trimmed = line.trim();
                                    if trimmed.is_empty() { continue; }
                                    match serde_json::from_str::<Message>(trimmed) {
                                        Ok(Message::Done) => {
                                            messages.push(Message::Done);
                                            break;
                                        }
                                        Ok(msg) => messages.push(msg),
                                        Err(_) => {} // skip malformed
                                    }
                                }
                                Ok(Err(_)) => break, // read error
                                Err(_) => break, // timeout
                            }
                        }
                        if !matches!(messages.last(), Some(Message::Done)) {
                            messages.push(Message::Done);
                        }
                        Ok(messages)
                    }
                };

                busy.dec();
                drop(permit);

                let messages = match result {
                    Ok(msgs) => msgs,
                    Err(e) => {
                        if cancel.is_file_cancelled(&file) {
                            let _ = tx.send(cancelled_result(&file));
                            return;
                        }
                        let name = file.file_name()
                            .unwrap_or_default()
                            .to_string_lossy()
                            .to_string();
                        vec![
                            Message::Event(Event::engine_error(name, e.to_string())),
                            Message::Done,
                        ]
                    }
                };

                let _ = tx.send(FileResult {
                    file,
                    messages,
                    cancelled: false,
                });
            });
        }

        while let Some(joined) = set.join_next().await {
            if let Err(e) = joined
                && e.is_panic()
                && let Some(ref lb) = self.log
            {
                lb.push(
                    self.suite.plugin.name(),
                    &format!("fork-pool task panicked: {e}\n"),
                );
            }
        }
    }
}
