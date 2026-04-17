//! TUI entry point and orchestration.
//!
//! Submodules:
//!   - state: AppState + types
//!   - view:  draw_* rendering
//!   - input: key/mouse handling

mod input;
mod keymap;
mod state;
mod view;

pub use keymap::default_keymap_for_init;
pub use state::RunGroup;

use std::io;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use anyhow::Result;
use crossterm::event::{self, DisableMouseCapture, EnableMouseCapture, Event};
use crossterm::terminal::{
    EnterAlternateScreen, LeaveAlternateScreen, disable_raw_mode, enable_raw_mode,
};
use ratatui::Terminal;
use ratatui::backend::CrosstermBackend;

use scrutin_core::analysis::deps::{TestAction, resolve_tests};
use scrutin_core::analysis::hashing;
use scrutin_core::engine::run_events::{self, RunEvent};
use scrutin_core::engine::watcher::{FileWatcher, unique_paths};
use scrutin_core::logbuf::LogBuffer;
use scrutin_core::project::package::Package;
use scrutin_core::storage::sqlite;

use input::{handle_key, handle_mouse};
use state::*;
use view::draw;

#[allow(clippy::too_many_arguments)]
pub async fn run_tui(
    pkg: &Package,
    test_files: &[PathBuf],
    filters: &[String],
    excludes: &[String],
    n_workers: usize,
    watch: bool,
    mut dep_map: Option<std::collections::HashMap<String, Vec<String>>>,
    log: LogBuffer,
    run_groups: Vec<RunGroup>,
    active_group: Option<String>,
    rerun_max: u32,
    rerun_delay_ms: u64,
    watch_debounce_ms: u64,
    timeout_file_ms: u64,
    timeout_run_ms: u64,
    fork_workers: bool,
    keymap_config: &std::collections::HashMap<String, std::collections::HashMap<String, String>>,
    agent: scrutin_core::project::config::AgentConfig,
) -> Result<()> {
    // The unified dep map covers every active suite (R via runtime
    // instrumentation, pytest via import scan). R deps are updated
    // incrementally on each test run; Python deps are rebuilt when stale.
    enable_raw_mode()?;
    let mut stdout = io::stderr();
    crossterm::execute!(stdout, EnterAlternateScreen, EnableMouseCapture)?;
    let backend = CrosstermBackend::new(stdout);
    let mut terminal = Terminal::new(backend)?;

    // Created before AppState so it can be stashed in state for use by
    // suspend_tui (vim-launch path).
    let poll_paused = Arc::new(AtomicBool::new(false));

    let state = Arc::new(Mutex::new(AppState::new(
        pkg,
        test_files,
        n_workers,
        &dep_map,
        log.clone(),
        rerun_max,
        rerun_delay_ms,
        timeout_file_ms,
        timeout_run_ms,
        fork_workers,
        keymap_config,
        agent,
    )));
    {
        let mut st = state.lock().unwrap();
        if let Some(name) = active_group.as_deref() {
            st.filter.group = run_groups.iter().position(|g| g.name == name);
        }
        st.run_groups = run_groups;
        st.poll_paused = Some(poll_paused.clone());
    }
    let (event_tx, mut event_rx) = tokio::sync::mpsc::unbounded_channel::<TuiEvent>();

    // Key events: crossterm has no native async, so poll on a blocking task.
    // The `poll_paused` flag lets us suspend stdin polling while a child
    // process (e.g. $EDITOR / vim) takes over the terminal — otherwise this
    // background poller steals every keystroke from vim.
    let key_tx = event_tx.clone();
    let pp = poll_paused.clone();
    tokio::task::spawn_blocking(move || {
        loop {
            if pp.load(Ordering::Relaxed) {
                std::thread::sleep(Duration::from_millis(50));
                if key_tx.is_closed() {
                    break;
                }
                continue;
            }
            if event::poll(Duration::from_millis(100)).unwrap_or(false) {
                match event::read() {
                    Ok(Event::Key(key)) => {
                        if key_tx.send(TuiEvent::Key(key)).is_err() {
                            break;
                        }
                    }
                    Ok(Event::Mouse(m)) => {
                        if key_tx.send(TuiEvent::Mouse(m)).is_err() {
                            break;
                        }
                    }
                    Ok(Event::Resize(_, _)) => {
                        if key_tx.send(TuiEvent::Tick).is_err() {
                            break;
                        }
                    }
                    _ => {}
                }
            }
            if key_tx.is_closed() {
                break;
            }
        }
    });

    // Tick task
    let tick_tx = event_tx.clone();
    tokio::spawn(async move {
        let mut interval = tokio::time::interval(Duration::from_millis(200));
        interval.tick().await; // skip immediate first tick
        loop {
            interval.tick().await;
            if tick_tx.send(TuiEvent::Tick).is_err() {
                break;
            }
        }
    });

    // Watch task. FileWatcher must outlive the forwarder task — if it's
    // dropped, the underlying debouncer is dropped and the channel closes.
    let _watcher_guard = if watch {
        let mut watcher = FileWatcher::new(pkg, watch_debounce_ms)?;
        let mut rx = watcher.rx.take().expect("watcher rx");
        let watch_tx = event_tx.clone();
        tokio::spawn(async move {
            while let Some(events) = rx.recv().await {
                let paths = unique_paths(&events);
                if !paths.is_empty()
                    && watch_tx.send(TuiEvent::WatchEvent(paths)).is_err() {
                        break;
                    }
            }
        });
        state.lock().unwrap().display.watch_active = true;
        Some(watcher)
    } else {
        None
    };

    // Eagerly build the dep map in the background if we don't have one yet,
    // so it's ready for the first watch event without waiting for a full run.
    if dep_map.is_none() {
        let pkg_clone = pkg.clone();
        let pkg_root = pkg.root.clone();
        let tx = event_tx.clone();
        state.lock().unwrap().run.depmap_rebuilding = true;
        tokio::task::spawn_blocking(move || {
            let map = scrutin_core::analysis::deps::build_unified_dep_map(&pkg_clone);
            if !map.is_empty() {
                let _ = sqlite::with_open(&pkg_root, |c| sqlite::replace_dep_map(c, &map));
                let _ = hashing::snapshot_hashes(&pkg_clone);
                let _ = tx.send(TuiEvent::DepMapRebuilt(map));
            } else {
                // No suites contribute — clear the rebuilding flag via an
                // empty map event so the UI doesn't show "rebuilding" forever.
                let _ = tx.send(TuiEvent::DepMapRebuilt(std::collections::HashMap::new()));
            }
        });
    }

    // Do not auto-run on startup — wait for user input (press `r` to run).

    let filters = filters.to_vec();
    let excludes = excludes.to_vec();

    loop {
        // Draw
        {
            let mut st = state.lock().unwrap();
            terminal.draw(|f| draw(f, &mut st))?;
        }

        // Handle event
        let evt = match event_rx.recv().await {
            Some(e) => e,
            None => break,
        };
        match evt {
            TuiEvent::Run(RunEvent::FileFinished(result)) => {
                // Merge runtime dep observations before consuming the result.
                if let Some((test_file, sources)) = result.deps() {
                    let map = dep_map.get_or_insert_with(std::collections::HashMap::new);
                    merge_deps_inmem(map, test_file, sources);
                    let owned_sources: Vec<String> = sources.to_vec();
                    let test_file_owned = test_file.to_string();
                    let pkg_root = pkg.root.clone();
                    tokio::task::spawn_blocking(move || {
                        let _ = sqlite::with_open(&pkg_root, |c| {
                            sqlite::merge_deps_for_test(c, &test_file_owned, &owned_sources)
                        });
                    });
                    let mut st = state.lock().unwrap();
                    st.dep_map = Some(map.clone());
                    st.reverse_dep_map = build_reverse_dep_map(&dep_map);
                }
                state.lock().unwrap().apply_result(&result);
            }
            TuiEvent::Run(RunEvent::Complete) => {
                // Decide whether the rerun loop should fire another
                // attempt before we mark the run finished. We do this
                // *before* `finish_run` because the rerun path needs to
                // keep `running = true` to suppress watch retriggers and
                // status-bar transitions.
                let rerun_decision: Option<(Vec<PathBuf>, u32)> = {
                    let mut st = state.lock().unwrap();
                    // Mark any files that started failing and now pass
                    // on a later attempt as flaky. This is the only place
                    // we can detect the transition.
                    if st.run.current_attempt > 0 {
                        for entry in &mut st.files {
                            if entry.attempt > 0
                                && matches!(entry.status, FileStatus::Passed { .. })
                            {
                                entry.flaky = true;
                            }
                        }
                    }
                    if st.rerun_max > 0 && st.run.current_attempt < st.rerun_max {
                        let failed: Vec<PathBuf> = st
                            .files
                            .iter()
                            .filter(|e| matches!(e.status, FileStatus::Failed { .. }))
                            .map(|e| e.path.clone())
                            .collect();
                        if failed.is_empty() {
                            None
                        } else {
                            Some((failed, st.run.current_attempt + 1))
                        }
                    } else {
                        None
                    }
                };

                if let Some((failed_files, next_attempt)) = rerun_decision {
                    // Schedule another attempt. The delay runs on a
                    // detached task so we don't block the TUI loop.
                    let delay_ms = state.lock().unwrap().rerun_delay_ms;
                    let pkg_owned = pkg.clone();
                    let state_clone = state.clone();
                    let tx_clone = event_tx.clone();
                    tokio::spawn(async move {
                        if delay_ms > 0 {
                            tokio::time::sleep(Duration::from_millis(delay_ms)).await;
                        }
                        let _ = start_test_run_inner(
                            &pkg_owned,
                            &failed_files,
                            &state_clone,
                            &tx_clone,
                            false,
                            next_attempt,
                        );
                    });
                    // Skip the dep-map rebuild on intermediate attempts —
                    // a rerun isn't a "full suite" run from the dep-map's
                    // perspective.
                    continue;
                }

                {
                    let mut st = state.lock().unwrap();
                    st.finish_run();
                }

                // Persist the dep map updated by runtime instrumentation.
                // Incremental merges happen per-FileFinished above; this
                // final replace_dep_map keeps the DB authoritative against
                // the in-memory map in case any edges were pruned here.
                if let Some(ref map) = dep_map {
                    let _ = sqlite::with_open(&pkg.root, |c| sqlite::replace_dep_map(c, map));
                }

                // Rebuild the Python side of the dep map if stale (R deps
                // are now updated incrementally via instrumentation).
                let already_rebuilding = state.lock().unwrap().run.depmap_rebuilding;
                if !already_rebuilding {
                    let pkg_clone = pkg.clone();
                    let pkg_root = pkg.root.clone();
                    let has_python = pkg.test_suites.iter().any(|s| s.plugin.name() == "pytest");
                    let stale = has_python
                        && hashing::is_dep_map_stale(&pkg_clone).unwrap_or(true);
                    if stale {
                        state.lock().unwrap().run.depmap_rebuilding = true;
                        let tx = event_tx.clone();
                        tokio::task::spawn_blocking(move || {
                            let map = scrutin_core::analysis::deps::build_unified_dep_map(
                                &pkg_clone,
                            );
                            if !map.is_empty() {
                                let _ = sqlite::with_open(&pkg_root, |c| {
                                    sqlite::replace_dep_map(c, &map)
                                });
                                let _ = hashing::snapshot_hashes(&pkg_clone);
                            }
                            let _ = tx.send(TuiEvent::DepMapRebuilt(map));
                        });
                    }
                }
            }
            TuiEvent::DepMapRebuilt(map) => {
                {
                    let mut st = state.lock().unwrap();
                    st.reverse_dep_map = build_reverse_dep_map(&Some(map.clone()));
                    st.dep_map = Some(map.clone());
                    st.run.depmap_rebuilding = false;
                }
                dep_map = Some(map);
            }
            TuiEvent::WatchEvent(changed_paths) => {
                let st = state.lock().unwrap();
                if st.run.running || st.display.watch_paused {
                    continue;
                }
                drop(st);

                let mut tests_to_run = Vec::new();
                let mut run_full = false;
                for changed in &changed_paths {
                    match resolve_tests(changed, pkg, dep_map.as_ref()) {
                        TestAction::Run(files) => tests_to_run.extend(files),
                        TestAction::FullSuite => {
                            run_full = true;
                            break;
                        }
                    }
                }
                tests_to_run.sort();
                tests_to_run.dedup();

                let files_to_run = if run_full {
                    let mut files = pkg.test_files().unwrap_or_default();
                    apply_glob_filters(&mut files, &filters, &excludes);
                    files
                } else {
                    apply_glob_filters(&mut tests_to_run, &filters, &excludes);
                    tests_to_run
                };

                if !files_to_run.is_empty() {
                    // Watch-triggered runs are only considered a "full suite" when
                    // resolve_tests fell back to FullSuite AND no CLI filters are active.
                    let is_full = run_full && filters.is_empty() && excludes.is_empty();
                    start_test_run(pkg, &files_to_run, &state, &event_tx, is_full)?;
                }
            }
            TuiEvent::Key(key) => {
                let should_quit = handle_key(
                    key,
                    &state,
                    pkg,
                    test_files,
                    &event_tx,
                    &mut terminal,
                    filters.is_empty() && excludes.is_empty(),
                )?;
                if should_quit {
                    break;
                }
            }
            TuiEvent::Mouse(m) => {
                handle_mouse(m, &state);
            }
            TuiEvent::Tick => {}
        }
    }

    disable_raw_mode()?;
    crossterm::execute!(
        terminal.backend_mut(),
        DisableMouseCapture,
        LeaveAlternateScreen
    )?;
    terminal.show_cursor()?;

    let st = state.lock().unwrap();
    if st.run.run_totals.bad() {
        std::process::exit(1);
    }

    Ok(())
}

/// Bridge a [`run_events::start_run`] to the TUI event channel.
///
/// This is the *only* place the TUI talks to the run engine. It mutates
/// `AppState` for the bookkeeping no other consumer would need (resetting
/// per-file status, stashing the busy/cancel handles), then forwards every
/// `RunEvent` straight into `TuiEvent::Run` so the main loop can pattern
/// match on the same events any future frontend would.
fn start_test_run(
    pkg: &Package,
    files: &[PathBuf],
    state: &Arc<Mutex<AppState>>,
    event_tx: &tokio::sync::mpsc::UnboundedSender<TuiEvent>,
    is_full_suite: bool,
) -> Result<()> {
    start_test_run_inner(pkg, files, state, event_tx, is_full_suite, 0)
}

/// Inner form that lets the rerun loop pass an explicit `attempt` counter.
/// External callers (key handlers, watch loop, palette dispatch) all use
/// `start_test_run` which forwards `attempt = 0` (fresh run).
fn start_test_run_inner(
    pkg: &Package,
    files: &[PathBuf],
    state: &Arc<Mutex<AppState>>,
    event_tx: &tokio::sync::mpsc::UnboundedSender<TuiEvent>,
    is_full_suite: bool,
    attempt: u32,
) -> Result<()> {
    let (n_workers, log, timeout_file_ms, timeout_run_ms, fork_workers) = {
        let mut st = state.lock().unwrap();
        st.reset_for_run(files, is_full_suite, attempt);
        (st.n_workers, st.log.clone(), st.timeout_file_ms, st.timeout_run_ms, st.fork_workers)
    };

    let timeout = Duration::from_millis(timeout_file_ms);
    let timeout_run = if timeout_run_ms > 0 {
        Some(Duration::from_millis(timeout_run_ms))
    } else {
        None
    };
    let pkg_owned = pkg.clone();
    let files_owned = files.to_vec();
    let event_tx_owned = event_tx.clone();
    let state_owned = state.clone();

    // Pool startup is async, so the whole run lives on a spawned task — that
    // also keeps `start_test_run` sync, which `handle_key` relies on (it
    // holds a `std::sync::MutexGuard` across the call, so an async fn would
    // be `!Send`).
    tokio::spawn(async move {
        let (handle, mut rx) =
            match run_events::start_run(&pkg_owned, files_owned, n_workers, timeout, timeout_run, fork_workers, Some(log))
                .await
            {
                Ok(pair) => pair,
                Err(_) => {
                    let _ = event_tx_owned.send(TuiEvent::Run(RunEvent::Complete));
                    return;
                }
            };
        {
            let mut st = state_owned.lock().unwrap();
            st.run.busy_counter = Some(handle.busy);
            st.run.cancel = Some(handle.cancel);
        }
        while let Some(ev) = rx.recv().await {
            if event_tx_owned.send(TuiEvent::Run(ev)).is_err() {
                break;
            }
        }
    });

    Ok(())
}

pub(crate) use scrutin_core::analysis::deps::build_reverse_dep_map;

/// Try to find the source file most relevant to a failing test file
fn find_source_for_test(
    test_name: &str,
    pkg_root: &Path,
    reverse_map: &std::collections::HashMap<String, Vec<String>>,
) -> Option<PathBuf> {
    let sources = reverse_map.get(test_name)?;
    // Prefer the source file whose stem matches the test stem
    // e.g., test-foo.R → R/foo.R
    let test_stem = test_name
        .strip_prefix("test-")
        .or_else(|| test_name.strip_prefix("test_"))
        .and_then(|s| s.strip_suffix(".R").or_else(|| s.strip_suffix(".r")))
        .unwrap_or("");

    let best = sources
        .iter()
        .find(|s| {
            let src_stem = Path::new(s)
                .file_stem()
                .and_then(|f| f.to_str())
                .unwrap_or("");
            src_stem.eq_ignore_ascii_case(test_stem)
        })
        .or_else(|| sources.first())?;

    let path = pkg_root.join(best);
    if path.exists() { Some(path) } else { None }
}

fn apply_glob_filters(files: &mut Vec<PathBuf>, filters: &[String], excludes: &[String]) {
    scrutin_core::filter::apply_include_exclude(files, filters, excludes);
}

/// In-memory merge of runtime-observed dependencies. The test file claims
/// every source in `sources`; previous edges for sources *not* in `sources`
/// have `test_file` removed. Mirrors the old `json_cache::merge_deps`; the
/// SQLite side is handled separately so the in-memory map stays usable by
/// the TUI without a DB round trip on every FileFinished event.
fn merge_deps_inmem(
    map: &mut std::collections::HashMap<String, Vec<String>>,
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
