# scrutin Internals

## Architecture

```
┌─────────────────────────────────────────────────────────────────┐
│                    scrutin (Rust binary)                         │
│                                                                 │
│  ┌──────────┐   ┌──────────────┐   ┌──────────────────────┐    │
│  │  Watcher │──▶│  Dep Tracker │──▶│  Run Engine           │    │
│  │ (notify) │   │  + Filter    │   │  (run_events.rs)      │    │
│  └──────────┘   └──────────────┘   └──────────┬───────────┘    │
│                                                │                │
│                          ┌─────────────────────┴──────┐         │
│                          │  Per-Suite Process Pools    │         │
│                          │  ┌────────┐  ┌────────┐    │         │
│                          │  │testthat│  │ pytest │ …  │         │
│                          │  └────────┘  └────────┘    │         │
│                          └──────────────────┬─────────┘         │
│                                             │                   │
│  ┌──────────────────────────────────────────▼──────────────┐    │
│  │              Frontend (TUI / Web / Plain)                │    │
│  └─────────────────────────────────────────────────────────┘    │
│                                                                 │
│  ┌──────────────────────────────────────────────────┐           │
│  │              Plugin Registry                     │           │
│  │  R:      testthat · tinytest · pointblank ·      │           │
│  │          validate · jarl                         │           │
│  │  Python: pytest · great_expectations · ruff      │           │
│  └──────────────────────────────────────────────────┘           │
└─────────────────────────────────────────────────────────────────┘
         │ spawn subprocesses via plugin
         ▼
┌─────────────────────────┐
│  Subprocess pool        │
│  (warm sessions)        │
│                         │
│  Embedded runner script │
│  - loads project        │
│  - runs tests           │
│  - emits NDJSON         │
└─────────────────────────┘
```

**Crate structure**: Cargo workspace with four crates:

- **`scrutin-core`** : engine, plugin trait, dep-map, protocol, DB, JUnit.
- **`scrutin-tui`** : ratatui terminal frontend.
- **`scrutin-web`** : axum browser dashboard (SSE-based, vanilla HTML/JS/CSS).
- **`scrutin-bin`** : the `scrutin` binary. CLI, config layering, reporter dispatch (tui/plain/web/list/junit), plain-mode renderers.

The Rust binary handles everything outside the language runtime: watching, scheduling, process management, UI, and result aggregation. Language subprocesses handle only test execution, driven by embedded runner scripts.

---

## Process Pool

### Concurrency Model

tokio async runtime. Subprocess stdio, the worker pool, the watcher, and event loops are all async. Each `ProcessPool` is a `VecDeque<RProcess>` queue gated by a `Semaphore` : acquiring a permit guarantees a free worker to pop. Any-free-worker assignment eliminates the imbalance of the older round-robin design (two slow files assigned to the same worker no longer serialize).

Multi-suite projects get one pool per tool, all created and joined inside `run_events::start_run`. Pools share a single `CancelHandle` so `cancel_all()` propagates across suites, and a single `BusyCounter` so the frontend reads total busy workers, not per-suite.

### Worker Lifecycle

Two pool implementations live side by side in `engine/pool.rs`:

- **`ProcessPool`** (default everywhere, also the Windows-only option): runs tests directly in the worker, killing and respawning the subprocess after each file. Safe even when test code forks or uses threaded numerical libraries.
- **`ForkPool`** (opt-in Unix only, `fork_workers: true`): one long-lived parent process per suite, forks a child per test file for full copy-on-write isolation. Faster startup, but unsafe when the test or any package it loads itself forks (`parallel::mclapply` in R, `multiprocessing` with `fork` start method in Python, BLAS/OpenMP-threaded code, etc.): forking an already-multithreaded parent can deadlock or crash the child, which is why it is no longer the default.

**ForkPool flow** (`engine/pool.rs::ForkPool::run_tests`):

1. Pool startup: bind a random loopback TCP port (`TcpListener::bind("127.0.0.1:0")`), then spawn one parent via `RProcess::spawn_fork_parent` with `SCRUTIN_TCP_PORT` set in its environment.
2. Parent startup: interpreter loads the runner script, loads the project once (`pkgload::load_all()` for R, `import` for Python), then loops reading file paths on stdin.
3. Per file: Rust acquires a semaphore permit, locks the parent, writes the file path to its stdin, and `accept()`s the next TCP connection under the same lock (serializing the fork+connect handshake so children can't be mismatched).
4. Parent forks a child (`parallel::mcparallel()` in R, `os.fork()` in Python). Child connects back to the TCP port, redirects its `emit_raw()` / `emit()` to write NDJSON on the socket, runs the test, and exits. Parent keeps running and reaps children non-blockingly.
5. Rust reads NDJSON from the socket until EOF (socket close signals completion), releases the permit, and moves on. The NDJSON-reading phase runs outside the handshake lock, so multiple children execute concurrently.

**ProcessPool flow**: a `VecDeque<RProcess>` gated by a `Semaphore`. Each worker reads a path on stdin, runs the test in-process, emits NDJSON to stdout, and is killed + respawned after the file completes.

**Pool sizing**: default `min(available_parallelism, 8)` clamped to minimum 2. Configurable via `[run] workers` in `.scrutin/config.toml`. In fork mode, "workers" controls the semaphore permits (concurrent children), not parent processes: a suite has exactly one parent.

**Pre-warming**: the parent (fork mode) or every worker (process mode) starts before any file is dispatched.

**Shutdown**: parents exit cleanly when stdin closes or on a `!shutdown` line; teardown hooks fire before `quit()`.

**Failure handling**: if a child fails to connect back or exits without sending `done`, the missing result is synthesized as an engine Error. If a worker crashes outright (EOF before any reply), the error is recorded and, in process mode, the worker is respawned.

**Pool poisoning**: if a `worker_startup` hook fails, the pool is poisoned : no further respawns are attempted and every subsequent file synthesizes an Error. The `CancelHandle` is triggered to stop the entire run.

### Worker command protocol

The Rust binary writes file paths to the parent's stdin, one per line. Only two sentinels exist:

```
/path/to/test-foo.R   ← run this test file (fork a child in fork mode, run in-process otherwise)
!shutdown             ← run worker_teardown hook and exit 0
```

There is no `!reload` command: fork mode gets fresh state for free because every child is a copy-on-write clone; process mode gets it via kill + respawn. The watcher's source-vs-test distinction no longer changes the command sent to workers.

### Timeouts

Disabled by default. When `timeout_file_ms` is set to a positive value and a worker exceeds it:

1. Worker is killed
2. Timeout error result recorded
3. Worker is replaced

### Signal Handling

On SIGINT/SIGTERM:

1. Stop accepting new runs
2. Kill all child subprocesses (tokio `kill_on_drop`)
3. Restore terminal state
4. Exit

**Stderr handling**: each subprocess's stderr is drained by a tokio task into a bounded ring buffer (8KB cap). Lines are forwarded to the shared `LogBuffer` (visible in the TUI Log pane). Plugin-specific `is_noise_line` filters out R startup chatter, Python warning preludes, etc.

---

## Dependency Tracker

Given a changed file path, returns the set of test files to re-run. Two-tier strategy:

### Tier 1 : Filename Heuristic

Always runs, zero cost. Maps source file stems to test file candidates using the plugin's patterns:

```
R:     R/foo.R         →  tests/testthat/test-foo.R, test_foo.R
Python: src/foo.py      →  tests/test_foo.py, foo_test.py
```

Case-insensitive. Used as the immediate response on any file change before consulting cached maps.

### Tier 2 : Runtime Instrumentation (R only)

Built incrementally from the test runs themselves. The R runner (`r/runner_r.R`) calls `trace()` on every function in the package namespace before running a test file; each tracer records the source file whose function was hit. After the test completes, the runner emits a `deps` NDJSON message listing the source files touched, and the engine merges those edges into a persistent `source_file → [test_files]` map.

Multi-suite aware: each R suite (testthat *and* tinytest) emits its own `deps` messages, so editing `R/math.R` correctly invalidates test files under both `tests/testthat/` and `inst/tinytest/`.

There is no standalone dep-map build step and no `scrutin-dependency-map.R` script. A previous tree-sitter-based static analyzer (`r/parse.rs`) has been removed in favor of this runtime approach.

**Storage**: persisted via `storage/json_cache.rs` under `.scrutin/`. Loaded into an in-memory `HashMap<PathBuf, Vec<PathBuf>>` on startup and updated after every test run that reports new edges.

**Cache invalidation**: files are hashed with xxhash64. On change, only the changed file is re-hashed; hash mismatch marks its entries as stale so the next run repopulates them.

### Static Import Analysis (Python)

Python source and test files are scanned by a line-based Rust parser (`scrutin_core::python::imports::scan_imports_str`). It extracts `import pkg`, `from pkg import func`, and `from pkg.module import func` (handling multi-line parenthesized and backslash-continued imports). Imports are resolved against local `.py` files and inverted into a `module_path → [test_files]` index with transitive closure across the import graph. Circular imports are safe; dynamic imports (`importlib.import_module`) are not tracked.

The Python dep map is rebuilt from scratch on every invocation (not cached).

### Fallback

If no mapping is found from any tier, re-run all test files.

---

## NDJSON Protocol

The contract between runner scripts and the Rust binary. All plugins
emit the same schema : the engine and all frontends are plugin-agnostic.

> **Authoritative spec: [NDJSON Protocol](protocol.md).**
> The taxonomy, message shapes, consumer policies, and per-library
> mappings live there. This section is a quick reference.

### Transport

In fork mode (Unix default), NDJSON flows from the forked child to Rust on a TCP socket the child opens back to the parent's listener (`127.0.0.1:$SCRUTIN_TCP_PORT`); EOF on the socket signals end of run. In non-fork / Windows mode the same NDJSON is written to the worker's stdout. Either way, the consuming code is the same `Message` deserializer.

### Message types

Four top-level variants discriminated on `"type"` (`event`, `summary`, `deps`, `done`):

```json
{"type":"event","file":"test-model.R","outcome":"pass",
 "subject":{"kind":"function","name":"fit_model works"},
 "duration_ms":14}

{"type":"event","file":"test-model.R","outcome":"fail",
 "subject":{"kind":"function","name":"handles NA"},
 "message":"expected TRUE got FALSE","line":23,"duration_ms":3}

{"type":"event","file":"test_users.py","outcome":"fail",
 "subject":{"kind":"check","name":"not_null","parent":"users.email"},
 "metrics":{"total":1000000,"failed":7,"fraction":7e-6},
 "failures":[{"row":42,"value":null}],
 "message":"7 nulls"}

{"type":"summary","file":"test-model.R","duration_ms":87,
 "counts":{"pass":4,"fail":1}}

{"type":"deps","file":"test-model.R","sources":["R/model.R","R/utils.R"]}

{"type":"done"}
```

### Outcome taxonomy (six values)

| Outcome   | Meaning                                                       |
| --------- | ------------------------------------------------------------- |
| `pass`    | Assertion held / validation step passed its threshold         |
| `fail`    | Assertion broken / threshold violated                         |
| `error`   | Could not evaluate (exception, missing column, broken setup) |
| `skip`    | Intentionally not run                                         |
| `xfail`   | Failed but predicted; does *not* count as a regression        |
| `warn`    | Soft failure: surfaced but does *not* break the build         |

### Counting policy

Events are authoritative for **per-test counts**. The summary's
`counts` block is a debugging aid : consumers ignore it and tally
events directly. The summary contributes only `duration_ms` (worker
wall time, more accurate than the sum of per-event ms).

The `bad_file = failed > 0 || errored > 0` rule is what makes `xfail`
and `warn` not break CI gates. It is enforced once, in
`tally_messages`, and inherited by every consumer.

### Cancellation

Not on the wire. The engine attaches a `cancelled: bool` flag to
`FileResult` when it kills a worker mid-file (TUI cancel keys,
`--max-fail` tripping). Workers never need to know they were
cancelled; they just get killed.

NDJSON is constructed directly in runner scripts (hand-rolled `cat()`
in R, `json.dumps()` in Python) : no serialization library
dependencies.

---

## Plugin System

Each language+tool combination is a plugin: a Rust trait implementation plus an embedded runner script. Plugins are compiled into the binary: no external plugin mechanism.

### The `Plugin` Trait

```rust
pub trait Plugin: Send + Sync {
    // Identity + detection
    fn name(&self) -> &'static str;
    fn language(&self) -> &'static str;
    fn detect(&self, root: &Path) -> bool;

    // Worker-mode runtime
    fn subprocess_cmd(&self, root: &Path) -> Vec<String>;
    fn runner_script(&self) -> &'static str;
    fn script_extension(&self) -> &'static str;
    fn runner_basename(&self) -> String;              // default: runner.<ext>
    fn env_vars(&self, root: &Path) -> Vec<(String, String)>;
    fn is_noise_line(&self, line: &str) -> bool;

    // File discovery + classification
    fn project_name(&self, root: &Path) -> String;
    fn source_dirs(&self) -> Vec<&'static str>;
    fn test_dirs(&self) -> Vec<&'static str>;
    fn discover_test_files(&self, root: &Path, test_dir: &Path) -> Result<Vec<PathBuf>>;
    fn is_test_file(&self, path: &Path) -> bool;
    fn is_source_file(&self, path: &Path) -> bool;
    fn test_file_candidates(&self, source_stem: &str) -> Vec<String>;

    // UI hints
    fn supported_outcomes(&self) -> &'static [Outcome];
    fn subject_label(&self) -> &'static str;          // "test" / "step" / "check" / ...
    fn actions(&self) -> Vec<PluginAction>;           // TUI/web action palette entries

    // Command-mode (opt-in, used by jarl + ruff)
    fn command_spec(&self, root: &Path) -> Option<CommandSpec>;
    fn parse_command_output(
        &self,
        file: &str, stdout: &str, stderr: &str,
        exit_code: Option<i32>, duration_ms: u64,
    ) -> Vec<Message>;
}
```

Worker-mode plugins (testthat, tinytest, pointblank, validate, pytest, great_expectations) leave `command_spec` at its default `None` and drive an NDJSON subprocess. Command-mode plugins (jarl, ruff) return `Some(CommandSpec { argv })` from `command_spec` and parse the tool's stdout directly in Rust : no worker, no runner script, no NDJSON.

### Embedded Script Delivery

Runner scripts are compiled into the binary via `include_str!()` and written to `.scrutin/` at subprocess startup:

```rust
let script = plugin.runner_script();
let runner_path = pkg.root.join(".scrutin").join(plugin.runner_basename());
std::fs::write(&runner_path, script)?;
```

The file lives for the lifetime of the subprocess. No installation step, no language-side package management.

### Runner structure

```
crates/scrutin-core/src/
  r/
    mod.rs                # data-driven RPlugin struct + plugins() registry
    depmap.rs
    runner_r.R            # shared R companion (traces, env, emit helpers)
    runner_testthat.R
    runner_tinytest.R
    runner_pointblank.R
    runner_validate.R
    jarl/                 # command-mode R linter (own plugin.rs)
      mod.rs
      plugin.rs
  python/
    mod.rs                # plugins() registry
    imports.rs            # line-based import scanner
    pytest/
      mod.rs
      plugin.rs
      runner.py
    great_expectations/
      mod.rs
      plugin.rs
      runner.py
    ruff/                 # command-mode Python linter (no runner script)
      mod.rs
      plugin.rs
```

The four R worker-mode tools (testthat, tinytest, pointblank, validate) share a data-driven `RPlugin` struct in `r/mod.rs`: each entry pairs a tool-specific runner (`runner_<name>.R`) with the shared `runner_r.R` companion. `scrutin init` scaffolds them into `.scrutin/<tool>/runner.<ext>` so users can edit them without forking the binary.

### Adding a new language

Drop a sibling directory next to `r/` and `python/`, register in `project/plugin.rs::all_plugins()`. No edits anywhere else.

### Adding a new tool to an existing language

For an R **worker-mode** tool, drop a `runner_<name>.R` next to the existing ones and add an `RPlugin { ... }` entry in `r/mod.rs::plugins()`. For a structurally different R plugin (e.g. a command-mode linter), drop a sibling directory next to `r/jarl/` and register in `r/mod.rs::plugins()`. Python plugins follow the same pattern: sibling directory next to `python/pytest/`, registered in `python/mod.rs::plugins()`.

---

## Persistent Storage

Two stores, both under `.scrutin/` and gitignored.

**DuckDB run history** (`.scrutin/state.db`) via the `duckdb` CLI (`storage/duckdb_cli.rs`):

- **Run history**: every test event from every run, with outcome, duration, git SHA, and optional metrics.
- **Run metadata**: key-value provenance and user labels per run.

Writes are best-effort: if the `duckdb` binary is not on PATH, history is silently skipped. Schema migration policy: drop and recreate on version mismatch (acceptable for pre-release; users lose history on schema bumps).

**JSON dep-map cache** (`storage/json_cache.rs`):

- **`depmap.json`**: `source_file → [test_files]` edges accumulated from runtime `deps` messages.
- **`hashes.json`**: per-file xxhash64 fingerprints for staleness detection.

Both files are rewritten atomically (write to `.tmp`, rename) after every run.

Key queries powered by the DB:

- `scrutin stats`: flaky test detection (alternating pass/fail in recent history), slow test ranking.
- `--failed-first`: last-run failures from `test_runs`.

Dep-map load/store goes through `json_cache::{load_dep_map, store_dep_map}`, not the DB.
