//! Shared server state. Every axum handler gets a cheap `Clone` of this.
//!
//! Ownership model:
//!   - `pkg` / `config` are read-only after startup.
//!   - `files` is a concurrent map of FileId → WireFile, rewritten by the
//!     forwarder task as run events stream in.
//!   - `current_run` holds the active `RunHandle` and summary. The
//!     forwarder task is the only writer; handlers read only.
//!   - `events_tx` is a broadcast channel fed by the forwarder; every SSE
//!     handler subscribes its own receiver.
//!   - `event_buffer` stores the last N events so reconnecting clients
//!     can replay via `Last-Event-ID`.

use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::Arc;
use std::time::Duration;

use anyhow::Result;
use scrutin_core::analysis::deps::{TestAction, resolve_tests};
use scrutin_core::engine::run_events::{self, FileResult, RunEvent};
use scrutin_core::engine::watcher::{FileWatcher, unique_paths};
use scrutin_core::project::package::Package;
use tokio::sync::{RwLock, broadcast};

use crate::wire::{
    FileId, RunId, WireCounts, WireEvent, WireFile, WireRunSummary, WireStatus,
    tally_file_messages,
};

/// Reverse dep map: source-file path (project-relative) → test-file
/// basenames that depend on it.
pub type DepMap = HashMap<String, Vec<String>>;

/// Capacity of the broadcast channel. Slow consumers lose the oldest
/// events; they recover via `Last-Event-ID` replay (bounded separately).
const BROADCAST_CAPACITY: usize = 1024;
/// Bounded replay buffer per spec §3.3.
const REPLAY_BUFFER_SIZE: usize = 1024;

#[derive(Clone)]
pub struct AppState {
    pub pkg: Arc<Package>,
    pub n_workers: usize,
    pub timeout_file_ms: u64,
    pub timeout_run_ms: u64,
    pub fork_workers: bool,
    pub watch: Arc<RwLock<bool>>,
    pub files: Arc<RwLock<HashMap<FileId, WireFile>>>,
    pub current_run: Arc<RwLock<Option<ActiveRun>>>,
    pub summary: Arc<RwLock<WireRunSummary>>,
    pub events_tx: broadcast::Sender<SeqEvent>,
    pub replay_buffer: Arc<RwLock<std::collections::VecDeque<SeqEvent>>>,
    pub seq: Arc<std::sync::atomic::AtomicU64>,
    pub dep_map: Arc<RwLock<Option<DepMap>>>,
    pub initial_files: Arc<Vec<PathBuf>>,
    /// Optional editor command from `[web].editor` in .scrutin/config.toml. When
    /// set, wins over `$VISUAL` / `$EDITOR` for the "open in editor"
    /// action. Whitespace-split into argv tokens so wrappers like
    /// `"code --wait"` work.
    pub editor: Option<String>,
}

/// A broadcast event plus its monotonic sequence id, so SSE clients can
/// resume via `Last-Event-ID`.
#[derive(Clone, Debug)]
pub struct SeqEvent {
    pub id: u64,
    pub event: WireEvent,
}

pub struct ActiveRun {
    pub run_id: RunId,
    pub handle: run_events::RunHandle,
}

impl AppState {
    pub fn new(
        pkg: Arc<Package>,
        initial_files: Vec<PathBuf>,
        n_workers: usize,
        watch: bool,
        timeout_file_ms: u64,
        timeout_run_ms: u64,
        fork_workers: bool,
        editor: Option<String>,
    ) -> Self {
        let this = Self::new_inner(pkg, initial_files, n_workers, watch, timeout_file_ms, timeout_run_ms, fork_workers, editor);
        // Spawn a background heartbeat that publishes the current busy
        // count + in_progress flag every second. Frontend reads this to
        // drive the "N/total workers" indicator between events.
        // Heartbeat cadence: 250ms during a run (feels live as workers
        // start/finish in bursts); 2s when idle (keeps SSE alive without
        // flooding the channel when there's nothing happening).
        let h = this.clone();
        tokio::spawn(async move {
            loop {
                let (busy, in_progress) = {
                    let s = h.summary.read().await;
                    (s.busy, s.in_progress)
                };
                h.publish(WireEvent::Heartbeat {
                    ts: chrono::Utc::now().to_rfc3339(),
                    busy,
                    in_progress,
                })
                .await;
                let delay = if in_progress { 250 } else { 2000 };
                tokio::time::sleep(tokio::time::Duration::from_millis(delay)).await;
            }
        });
        this
    }

    fn new_inner(
        pkg: Arc<Package>,
        initial_files: Vec<PathBuf>,
        n_workers: usize,
        watch: bool,
        timeout_file_ms: u64,
        timeout_run_ms: u64,
        fork_workers: bool,
        editor: Option<String>,
    ) -> Self {
        let root = pkg.root.clone();
        let mut files: HashMap<FileId, WireFile> = HashMap::new();
        for abs in &initial_files {
            let suite_name = pkg
                .suite_for(abs)
                .map(|s| s.plugin.name().to_string())
                .unwrap_or_default();
            let wf = WireFile::new(abs, &root, suite_name);
            files.insert(wf.id, wf);
        }
        let (tx, _) = broadcast::channel::<SeqEvent>(BROADCAST_CAPACITY);
        Self {
            pkg,
            n_workers,
            timeout_file_ms,
            timeout_run_ms,
            fork_workers,
            watch: Arc::new(RwLock::new(watch)),
            files: Arc::new(RwLock::new(files)),
            current_run: Arc::new(RwLock::new(None)),
            summary: Arc::new(RwLock::new(WireRunSummary::default())),
            events_tx: tx,
            replay_buffer: Arc::new(RwLock::new(std::collections::VecDeque::with_capacity(
                REPLAY_BUFFER_SIZE,
            ))),
            seq: Arc::new(std::sync::atomic::AtomicU64::new(0)),
            dep_map: Arc::new(RwLock::new(None)),
            initial_files: Arc::new(initial_files),
            editor,
        }
    }

    /// Publish a wire event: assigns a sequence id, pushes into the replay
    /// buffer, and broadcasts. Called by the forwarder task.
    pub async fn publish(&self, event: WireEvent) {
        let id = self.seq.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
        let seq_ev = SeqEvent { id, event };
        {
            let mut buf = self.replay_buffer.write().await;
            if buf.len() == REPLAY_BUFFER_SIZE {
                buf.pop_front();
            }
            buf.push_back(seq_ev.clone());
        }
        // Ignore send errors (no subscribers is fine).
        let _ = self.events_tx.send(seq_ev);
    }

    /// Cancel the active run (if any) via its shared CancelHandle.
    pub async fn cancel_all(&self) {
        let cur = self.current_run.read().await;
        if let Some(ref active) = *cur {
            active.handle.cancel.cancel_all();
        }
    }

    /// Spawn a new run. Cancels any existing run first. The run's
    /// `RunEvent` stream is consumed by a forwarder task that updates
    /// `files`/`summary` and broadcasts to SSE subscribers.
    pub async fn spawn_run(&self, files: Vec<PathBuf>) -> Result<RunId> {
        // Cancel any existing run.
        {
            let cur = self.current_run.read().await;
            if let Some(ref active) = *cur {
                active.handle.cancel.cancel_all();
            }
        }

        let run_id = RunId::new();
        let started_at = chrono::Utc::now().to_rfc3339();

        // Compute the file_ids that this run will touch, in stable order.
        let root = self.pkg.root.clone();
        let touched_ids: Vec<FileId> = files
            .iter()
            .map(|p| {
                let rel = p.strip_prefix(&root).unwrap_or(p).to_path_buf();
                FileId::of(&rel)
            })
            .collect();

        // Reset per-file state for touched files to Pending.
        {
            let mut fmap = self.files.write().await;
            for id in &touched_ids {
                if let Some(f) = fmap.get_mut(id) {
                    f.status = WireStatus::Pending;
                    f.counts = WireCounts::default();
                    f.messages.clear();
                    f.bad = false;
                }
            }
        }

        // Reset summary for the new run.
        {
            let mut s = self.summary.write().await;
            *s = WireRunSummary {
                run_id: Some(run_id),
                started_at: Some(started_at.clone()),
                finished_at: None,
                in_progress: true,
                totals: WireCounts::default(),
                bad_files: Vec::new(),
                busy: 0,
            };
        }

        // Start the engine.
        let timeout = Duration::from_millis(self.timeout_file_ms);
        let timeout_run = if self.timeout_run_ms > 0 {
            Some(Duration::from_millis(self.timeout_run_ms))
        } else {
            None
        };
        let (handle, mut rx) =
            run_events::start_run(&self.pkg, files, self.n_workers, timeout, timeout_run, self.fork_workers, None).await?;

        {
            let mut cur = self.current_run.write().await;
            *cur = Some(ActiveRun {
                run_id,
                handle: handle.clone(),
            });
        }

        self.publish(WireEvent::RunStarted {
            run_id,
            started_at,
            files: touched_ids,
        })
        .await;

        // Forwarder task. Drains the run engine's events, updates
        // accumulator, and broadcasts WireEvents. A sibling ticker task
        // polls the BusyCounter every 200ms so the web UI's "busy
        // workers" indicator updates between file-finished events.
        let state = self.clone();
        let handle_for_ticker = handle.clone();
        let state_for_ticker = state.clone();
        let ticker_stop = Arc::new(tokio::sync::Notify::new());
        let ticker_stop_cloned = ticker_stop.clone();
        tokio::spawn(async move {
            let mut interval =
                tokio::time::interval(tokio::time::Duration::from_millis(200));
            loop {
                tokio::select! {
                    _ = interval.tick() => {
                        let busy = handle_for_ticker.busy.get() as u32;
                        let mut s = state_for_ticker.summary.write().await;
                        s.busy = busy;
                    }
                    _ = ticker_stop_cloned.notified() => break,
                }
            }
        });
        tokio::spawn(async move {
            while let Some(ev) = rx.recv().await {
                match ev {
                    RunEvent::FileFinished(result) => {
                        // Merge runtime dep observations into the dep map.
                        if let Some((test_file, sources)) = result.deps() {
                            let mut dm = state.dep_map.write().await;
                            let map = dm.get_or_insert_with(HashMap::new);
                            merge_deps_inmem(map, test_file, sources);
                            let owned_sources: Vec<String> = sources.to_vec();
                            let owned_test = test_file.to_string();
                            let pkg_root = state.pkg.root.clone();
                            tokio::task::spawn_blocking(move || {
                                let _ = scrutin_core::storage::sqlite::with_open(
                                    &pkg_root,
                                    |c| {
                                        scrutin_core::storage::sqlite::merge_deps_for_test(
                                            c,
                                            &owned_test,
                                            &owned_sources,
                                        )
                                    },
                                );
                            });
                        }
                        let wf = apply_file_result(&state, run_id, &result).await;
                        // Update busy count immediately (the ticker will
                        // refresh it between events, but this keeps the
                        // count accurate on the FileFinished itself).
                        {
                            let mut s = state.summary.write().await;
                            s.busy = handle.busy.get() as u32;
                        }
                        state
                            .publish(WireEvent::FileFinished {
                                run_id,
                                file: wf,
                            })
                            .await;
                    }
                    RunEvent::Complete => break,
                }
            }
            // Stop the ticker.
            ticker_stop.notify_one();

            // Persist the dep map updated by runtime instrumentation.
            // Incremental merges fire per-FileFinished above; this final
            // replace is the DB equivalent of the old "write the whole map
            // at end-of-run" flush and keeps the DB authoritative against
            // the in-memory map.
            {
                let dm = state.dep_map.read().await;
                if let Some(ref map) = *dm {
                    let pkg_root = state.pkg.root.clone();
                    let map = map.clone();
                    tokio::task::spawn_blocking(move || {
                        let _ = scrutin_core::storage::sqlite::with_open(&pkg_root, |c| {
                            scrutin_core::storage::sqlite::replace_dep_map(c, &map)
                        });
                    });
                }
            }

            // Finalize.
            let finished_at = chrono::Utc::now().to_rfc3339();
            let (totals, bad_files) = {
                let summary = state.summary.read().await;
                (summary.totals, summary.bad_files.clone())
            };
            {
                let mut s = state.summary.write().await;
                s.finished_at = Some(finished_at.clone());
                s.in_progress = false;
                s.busy = 0;
            }
            {
                let mut cur = state.current_run.write().await;
                *cur = None;
            }
            state
                .publish(WireEvent::RunComplete {
                    run_id,
                    finished_at,
                    totals,
                    bad_files,
                })
                .await;
        });

        Ok(run_id)
    }

    /// Start watching the filesystem. Creates a `FileWatcher` and spawns a
    /// task that resolves changed files → affected tests → `spawn_run`.
    /// Returns the guard (must be kept alive to keep the debouncer running).
    pub fn start_watcher(&self, debounce_ms: u64) -> Result<FileWatcher> {
        let mut watcher = FileWatcher::new(&self.pkg, debounce_ms)?;
        let mut rx = watcher.rx.take().expect("watcher rx");
        let state = self.clone();
        tokio::spawn(async move {
            while let Some(events) = rx.recv().await {
                let paths = unique_paths(&events);
                if paths.is_empty() {
                    continue;
                }
                // Don't trigger while a run is in progress.
                {
                    let cur = state.current_run.read().await;
                    if cur.is_some() {
                        continue;
                    }
                }
                let watching = *state.watch.read().await;
                if !watching {
                    continue;
                }

                let dep_map = state.dep_map.read().await;
                let mut tests_to_run = Vec::new();
                let mut run_full = false;
                for changed in &paths {
                    match resolve_tests(changed, &state.pkg, dep_map.as_ref()) {
                        TestAction::Run(files) => tests_to_run.extend(files),
                        TestAction::FullSuite => {
                            run_full = true;
                            break;
                        }
                    }
                }
                drop(dep_map);

                let files_to_run = if run_full {
                    state.initial_files.to_vec()
                } else {
                    tests_to_run.sort();
                    tests_to_run.dedup();
                    tests_to_run
                };

                if files_to_run.is_empty() {
                    continue;
                }

                // Publish a WatcherTriggered event before starting the run.
                let changed_strs: Vec<String> = paths
                    .iter()
                    .filter_map(|p| {
                        p.strip_prefix(&state.pkg.root)
                            .unwrap_or(p)
                            .to_str()
                            .map(String::from)
                    })
                    .collect();
                let will_rerun: Vec<FileId> = files_to_run
                    .iter()
                    .map(|p| {
                        let rel = p.strip_prefix(&state.pkg.root).unwrap_or(p).to_path_buf();
                        FileId::of(&rel)
                    })
                    .collect();
                state
                    .publish(WireEvent::WatcherTriggered {
                        changed_files: changed_strs,
                        will_rerun,
                    })
                    .await;

                let _ = state.spawn_run(files_to_run).await;
            }
        });
        Ok(watcher)
    }

    /// Build the dep map eagerly in the background so it's ready for the
    /// first watch event.
    pub fn start_dep_map_build(&self) {
        let state = self.clone();
        tokio::task::spawn_blocking(move || {
            let map = scrutin_core::analysis::deps::build_unified_dep_map(&state.pkg);
            if !map.is_empty() {
                let _ = scrutin_core::storage::sqlite::with_open(&state.pkg.root, |c| {
                    scrutin_core::storage::sqlite::replace_dep_map(c, &map)
                });
                let _ = scrutin_core::analysis::hashing::snapshot_hashes(&state.pkg);
            }
            let rt = tokio::runtime::Handle::current();
            rt.block_on(async {
                let mut dm = state.dep_map.write().await;
                *dm = Some(map);
            });
        });
    }
}

/// Translate a single FileResult into its WireFile, update the per-file
/// entry in `state.files`, and fold the deltas into the run totals.
async fn apply_file_result(state: &AppState, run_id: RunId, result: &FileResult) -> WireFile {
    let rel = result
        .file
        .strip_prefix(&state.pkg.root)
        .unwrap_or(&result.file)
        .to_path_buf();
    let file_id = FileId::of(&rel);
    let (counts, messages, file_ms, status, bad) = tally_file_messages(&result.messages, result.cancelled);

    let mut fmap = state.files.write().await;
    // Build a fresh WireFile if we've never seen this path (can happen
    // for files discovered mid-run, e.g. the runner creates a file).
    let entry = fmap.entry(file_id).or_insert_with(|| {
        let suite_name = state
            .pkg
            .suite_for(&result.file)
            .map(|s| s.plugin.name().to_string())
            .unwrap_or_default();
        WireFile::new(&result.file, &state.pkg.root, suite_name)
    });
    entry.status = status;
    entry.last_duration_ms = Some(file_ms);
    entry.last_run_id = Some(run_id);
    entry.counts = counts;
    entry.messages = messages;
    entry.bad = bad;
    let cloned = entry.clone();
    drop(fmap);

    // Fold deltas into the summary totals (one per run — the summary's
    // `totals` mirrors the file counts for files that finished this run).
    let mut summary = state.summary.write().await;
    summary.totals.merge(&counts);
    if bad && !summary.bad_files.contains(&file_id) {
        summary.bad_files.push(file_id);
    }

    cloned
}

/// In-memory merge of runtime-observed deps. See the TUI's analog. The
/// SQLite side is handled by the caller on a separate blocking task so
/// the async hot path stays DB-free.
fn merge_deps_inmem(
    map: &mut HashMap<String, Vec<String>>,
    test_file: &str,
    sources: &[String],
) {
    let source_set: std::collections::HashSet<&str> =
        sources.iter().map(|s| s.as_str()).collect();
    for (src, tests) in map.iter_mut() {
        if !source_set.contains(src.as_str()) {
            tests.retain(|t| t != test_file);
        }
    }
    for src in sources {
        let entry = map.entry(src.clone()).or_default();
        if !entry.contains(&test_file.to_string()) {
            entry.push(test_file.to_string());
            entry.sort();
        }
    }
}

/// Convenience: list test files in stable order for use in `GET /api/files`.
pub fn sorted_files(map: &HashMap<FileId, WireFile>) -> Vec<WireFile> {
    let mut v: Vec<WireFile> = map.values().cloned().collect();
    v.sort_by(|a, b| a.path.cmp(&b.path));
    v
}


