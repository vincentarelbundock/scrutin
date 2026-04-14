# Testing Spec

This document specifies what the scrutin test suite should cover, what
behaviors are locked in (i.e., tests exist precisely to prevent their
regression), and what is deliberately out of scope.

It is both a design doc and a rollout plan: sections 0 and 1 set policy,
section 2 lists the shared harnesses that must exist before most of the
behavior-locking tests can be written cleanly, and sections 3 onward
enumerate the surfaces of the system with the specific behaviors to lock
and the test shape required to lock them.

Companion docs: `protocol.md` pins the NDJSON wire format and outcome
taxonomy referenced throughout; `internals.md` describes the architecture
the tests exercise.

## 0. Guiding principles

- **Test contracts, not implementation.** The six-outcome taxonomy, the
  NDJSON protocol shape, the config precedence order, and the dep-map
  inputs-to-outputs are contracts. The internal types that implement
  them are not. Tests that break on refactor without catching a real
  regression are worse than nothing.
- **One honest E2E per tool beats ten mocked ones.** scrutin's
  value is correctness across real R/Python subprocesses; mocking the
  subprocess erases the thing being tested. The `ci_run_*_fixture`
  pattern (see `crates/scrutin-bin/tests/`) is the right shape: add
  more of them, don't replace them with unit tests.
- **Locked behaviors are documented twice.** Once in this spec (what)
  and once in a test named after the behavior (proof). If a test has
  no corresponding spec line, either the spec is incomplete or the test
  is testing implementation.
- **Skip gracefully, don't mock.** If `Rscript` or `pytest` is
  unavailable, skip the test; don't fake the subprocess. CI provides
  the real coverage; local dev on a stripped machine isn't blocked.
- **Snapshot with care.** Golden-file snapshots are valuable for wire
  formats (NDJSON, JUnit, CTRF, wire JSON) and TUI buffers, not for
  free-form human-readable output that will churn.

## 1. Test pyramid

| Layer                                 | What lives here                                                                                                               | Runtime budget  | Target count |
| ------------------------------------- | ----------------------------------------------------------------------------------------------------------------------------- | --------------- | ------------ |
| **Unit**                              | Pure functions: glob matching, config merging, outcome rank, protocol encode/decode, filter, git SHA parse, hashing           | under 5ms each  | ~250         |
| **Module-integration** (no subprocess) | Engine pool with fake runner, dep-map against an in-memory package, watcher against tempdir, DuckDB schema round-trip        | under 500ms each | ~60         |
| **E2E fixture**                       | scrutin binary against `demo/` or a purpose-built fixture, real `Rscript`/`pytest` subprocess                                 | 1 to 10s each   | ~30          |
| **Snapshot**                          | TUI view trees, JUnit XML, CTRF JSON, web wire types, compared byte-for-byte against a golden file                            | under 50ms each | ~40          |

## 2. Harnesses to build first (blocking)

Without these, most behavior-locking tests cannot be written cleanly.
They are listed in dependency order.

1. **`FakePlugin` + `fake_runner` binary.** A plugin that declares a
   synthetic "language" and a tiny runner binary that emits scripted
   NDJSON on stdin. Lets engine, pool, and run_events tests run
   without R or Python. Lives in `crates/scrutin-core/src/testing/`.
2. **`temp_package!` macro.** Builds an on-disk package with a given
   layout (DESCRIPTION + R/ + tests/, or pyproject.toml + src/ +
   tests/) in a tempdir, returns a `Package`. Eliminates per-test
   boilerplate.
3. **NDJSON golden harness.** `assert_ndjson_matches("fixture/name.ndjson", stream)`
   that pretty-prints diffs. Used for runner-companion output locking.
4. **`RunAccumulator` builder.** A typed way to construct a known-state
   accumulator for reporter tests without driving a full run.
5. **TUI test harness.** A headless `AppState` builder, key-event
   feeder, and buffer renderer, so `press_keys("tjjs").frame()`
   produces a `ratatui::buffer::Buffer` for snapshot comparison.
   Without this the TUI stays untested.

## 3. Surfaces and behaviors to lock

### 3.1 NDJSON protocol (`engine/protocol.rs`)

**Lock:** the wire shape as documented in `protocol.md`. Three
top-level variants (`event`, `summary`, `done`); event carries
`Outcome` from the six-bucket taxonomy; `summary.duration_ms` is
authoritative for wall time; events are authoritative for counts.

**Tests:**

- Each of the six outcomes round-trips (string to enum to string, unchanged).
- `rawStatus` and unknown fields are preserved in `extra` (forward compat).
- jsonlite edge case: `NULL` serialized as `{}` deserializes to `None`.
- Malformed line does not panic; logged and skipped.
- Two events for the same file: counts accumulate; summary replaces timing.

### 3.2 Runner companions (`r/*/runner.*.R`, `python/*/runner.py`)

**Lock:** each companion emits exactly the taxonomy expected. This is
the seam where the R/Python/tool idiom is translated to scrutin's
vocabulary; drift here is the single most common source of bugs.

**Per-tool E2E** (one fixture file per outcome bucket, already
present in `demo/`):

- `pass` case: `event(pass)` + `summary(0 failed)` + `done`.
- `fail` case: `event(fail)` with a non-empty `message`.
- `error` case (syntax error, missing import): `event(error)`.
- `skip` case: `event(skip)`.
- `xfail` case (pytest `@pytest.mark.xfail`, manual for R): `event(xfail)`.
- `warn` case (jarl, ruff): `event(warn)`.
- Crashed runner (kill the subprocess mid-file): `error` for the file, not a hang.

**Snapshot:** full NDJSON for a known `demo/` subtree, byte-compared
against `tests/goldens/<tool>-<case>.ndjson`.

### 3.3 Engine: pool, runner, run_events

**Lock:** per-suite warm worker pool; cancel propagates; any-free-worker
assignment; startup-hook failure poisons the pool.

**Tests (use `FakePlugin`):**

- N files, W workers, W < N: all N complete; no more than W concurrent.
- Cancel during run: remaining files never start; running files are
  SIGKILLed within a bounded timeout.
- Per-file timeout: file exceeds timeout produces `error` event, pool reclaimed.
- Startup hook exits non-zero: pool reports poisoned, no files run, run completes.
- Multi-suite fan-out: 2 suites with 3 files each, different worker
  counts, run concurrently; events for both interleave on the single
  `mpsc` receiver.
- `CancelHandle` shared across suites: cancel on one propagates.

### 3.4 Dep-map (`r/depmap.rs`, `python/imports.rs`)

**Lock:** editing a source file invalidates the test files that
reference its symbols, across all suites of that language.

**Tests:**

- R: `R/math.R` defines `add()`; `tests/testthat/test-math.R` calls
  `add()`. Edit `R/math.R` produces set including `test-math.R`.
- R multi-suite: same, plus `inst/tinytest/test-math.R` calls `add()`.
  Edit `R/math.R` includes **both** test files.
- R: edit a test file invalidates only itself.
- R: edit an unreferenced file (`R/unused.R`) invalidates nothing.
- R: circular source reference does not infinite-loop.
- Python: `from pkg.math import add` in `tests/test_math.py`. Edit
  `src/pkg/math.py` includes `test_math.py`.
- Python: relative imports (`from .math import add`) resolve correctly.
- Python: `import pkg; pkg.math.add(...)` (attribute access): document
  behavior explicitly. Current design: invalidates all tests importing
  `pkg`. This is a design question worth locking with a test.
- Stale dep-map: file hash changed since cache triggers rebuild.

### 3.5 Watcher (`engine/watcher.rs`)

**Lock:** file-system events produce the expected rerun set within a
debounce window.

**Tests:**

- Edit `R/math.R` produces one batch containing that path (debounced).
- Rapid-fire edits (10 saves in 50ms): one batch, not ten.
- Editor atomic-save (write-tmp plus rename): one event, not two.
- Deleted file: removal event propagates; dep-map invalidates downstream.
- File outside watched dirs: no event.

### 3.6 Config (`project/config.rs`)

**Lock:** precedence is `defaults -> scrutin.toml -> --set -> CLI flags`,
walked from project root upward, fallback
`~/.config/scrutin/scrutin.toml`, no env vars.

**Tests** (9 already exist; add):

- `--set foo.bar=1 --set foo.bar=2`: last wins.
- `--set run.workers=abc`: typed error, not silent fallback.
- scrutin.toml in parent dir is found; in sibling dir is not.
- No scrutin.toml anywhere and no `~/.config/scrutin/scrutin.toml`: defaults used.
- Env vars `SCRUTIN_*` are ignored (assert this explicitly: it is a
  deliberate design choice, not an oversight).

### 3.7 CLI (`scrutin-bin/src/cli/mod.rs`)

**Lock:** argv to `Cli` struct parse is stable; default subcommand is
`run`; one reporter per invocation.

**Tests** (12 already exist; add):

- `scrutin demo`: default `run` with default reporter.
- `scrutin -r junit:out.xml -r plain demo`: error, one reporter per invocation.
- `scrutin init demo` and `scrutin stats demo`: routed to correct subcommand.
- `scrutin -r web demo`: binds loopback by default.
- `scrutin -r web:0.0.0.0:7878 demo`: if loopback-only is a security
  posture, non-loopback bind should error; lock that here.

### 3.8 Reporters

**plain** (E2E against `demo/`):

- Summary line format stable (snapshot).
- Exit code 0 on all-pass, 1 on any failure, 2 on internal error.

**github:**

- Emits `::group::` and `::endgroup::` per file (snapshot).
- `::error file=...,line=...` for each failed file.
- Writes to `$GITHUB_STEP_SUMMARY` when set.

**junit** (schema lock):

- Output is parseable by a standard JUnit XML parser (round-trip with
  `quick-xml` or a dedicated crate).
- `<testsuite name>`, `<testcase classname>`, `<failure>`, `<skipped>`
  tags present with expected attributes.
- User-supplied metadata via `--set metadata.extra.*` lands in `<properties>`.

**list:**

- Prints exactly the files that would run, across all active suites,
  after filter and exclude.
- Spawns zero subprocesses (assert with a fake plugin that panics if invoked).

### 3.9 Rerun, flaky, max_fail

**Lock:** `reruns=N` retries failing files up to N times; a pass on
rerun marks the file `flaky=true`; `max_fail=K` stops after K **bad
files** (not K expectations), and cancels all suites.

**Tests:**

- File fails once, passes on rerun: final outcome pass, `flaky=true`, `retries=1`.
- File fails all N+1 attempts: final outcome fail, `flaky=false`, `retries=N`.
- `max_fail=1` with 3 failing files across 2 suites: exactly 1 bad
  file in output, both pools cancelled.
- `max_fail` counts a file with 10 failed expectations as 1, not 10.

### 3.10 DuckDB history (`storage/db.rs`)

**Lock:** schema is stable or versioned; a run inserts the expected
rows; reads are deterministic.

**Tests:**

- Fresh DB: schema created; `scrutin stats` on empty DB does not panic.
- Run N files: N rows in results table with expected columns (aligned
  to CTRF names per TODO).
- Schema change: migration runs; old data readable.
- Concurrent runs do not corrupt the DB (write lock).

### 3.11 Filter, include, exclude

**Lock:** glob patterns in config and `-t TAG` (future) filter the
resolved file set consistently across reporters.

**Tests** (glob_match tests exist; add):

- `include = ["tests/fast/*.R"]` plus `exclude = ["tests/fast/skip-*.R"]`: exact set.
- Filter applied identically to `-r list` and actual runs.
- Empty filter equals no filter (runs everything).

### 3.12 Plugin actions

**Lock:** `ActionScope::File` runs the command on the selected file
only; `ActionScope::All` runs on all files in the suite after
filters; `rerun: true` re-runs affected files after.

**Tests:**

- File-scope action invoked with exactly the selected path.
- All-scope action invoked with the full filtered list.
- `rerun: true` triggers a new run covering the affected files.
- Action output captured and exposed via `Mode::ActionOutput`.

### 3.13 Hooks (`project/hooks.rs`)

**Lock:** pre-run and post-run hooks execute in project root; non-zero
exit from pre-run aborts the run; post-run always runs (even on cancel
or failure).

**Tests** (1 already exists; expand):

- Hook env includes expected vars (scrutin version, run id).
- Pre-run hook SIGKILL: run aborts with a clear error.

### 3.14 TUI state machine

**Lock:** mode stack invariants; `level()` and `overlay_kind()` return
the right frames; keybindings per table do what they claim.

**Tests (need the harness from section 2):**

- Mode stack: push/pop round-trip; Esc pops exactly one frame.
- Overlay on top of level: `level()` returns the level,
  `overlay_kind()` returns `Some(...)`.
- Palette dispatch: `a` opens Action palette; selecting an item runs
  the action and opens `ActionOutput`.
- Cursor dispatch: `move_cursor(mode, delta)` updates exactly the
  cursor for that mode.
- Layout collapse: terminal width below `MIN_LIST_COLS + MIN_MAIN_COLS`
  collapses to a single focused pane.

### 3.15 TUI snapshot rendering

**Lock:** the file list, counts bar, and hints bar render identically
for a given `AppState`.

**Tests (snapshot):**

- `demo/` after a full run, Normal mode: snapshot buffer.
- Detail mode on a failing file: snapshot buffer.
- Failure mode, cursor at second failure: snapshot buffer.
- Resize mid-render: no panic; layout recomputed.

### 3.16 Web wire format and routes

**Lock:** API surface in `docs-src/web-spec.md`; loopback-only
middleware rejects external requests; `outcome_order` in snapshot
matches `Outcome::rank()`.

**Tests** (7 wire tests exist; add):

- Each route returns the documented shape (golden JSON).
- Non-loopback request returns 403.
- SSE stream delivers events in the right order (smoke test with
  `reqwest` against a test server).
- SPA fallback serves `index.html` for unknown paths.

### 3.17 End-to-end CLI (`ci_run_*_fixture` style)

**Lock:** `scrutin -r plain demo` produces the expected aggregate
counts and exits correctly.

**Tests** (4 exist; add one per reporter against `demo/`):

- `-r github demo`: expected stdout plus step summary.
- `-r junit:x.xml demo`: valid XML with expected content.
- `-r list demo`: exact file list, no subprocess.
- `-r plain --set run.reruns=2 demo` with a flaky test: marked flaky.
- `-r plain --set run.max_fail=1 demo`: early exit.

## 4. Deliberately out of scope

- **Ratatui drawing correctness.** We test our state machine and our
  buffer output; we do not test ratatui itself.
- **R and Python language semantics.** We test that testthat results
  round-trip; we do not test testthat.
- **Cross-OS matrix.** Windows support is on the TODO; testing it is
  gated on that landing.
- **DuckDB version pinning.** We test our schema; we do not test duckdb.

## 5. Rollout order

1. Harnesses (section 2): `FakePlugin`, `temp_package!`, NDJSON
   golden, TUI harness. **Blocks everything else.**
2. Protocol and runner companion goldens (sections 3.1, 3.2).
   **Highest regression value.** Any taxonomy drift shows up immediately.
3. Dep-map (section 3.4) and rerun/flaky/max_fail (section 3.9).
   **Core value props.**
4. Engine pool, cancel, multi-suite (section 3.3).
5. Reporters (section 3.8) snapshots.
6. TUI state and snapshot (sections 3.14, 3.15).
7. Web routes (section 3.16).
8. Everything else (watcher, filter, hooks, DB, actions).
