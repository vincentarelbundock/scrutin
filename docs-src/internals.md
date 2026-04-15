# How it works

Scrutin is a single Rust binary that orchestrates test execution across R and Python. It handles file watching, dependency resolution, process management, and result presentation. The actual test execution happens in language-specific subprocesses driven by embedded runner scripts.

``` mermaid
flowchart TD
    W[File watcher] --> D[Dependency tracker] --> E[Run engine]

    subgraph P [Tool plugins]
        direction LR
        P1[testthat] ~~~ P2[pytest] ~~~ P3[ruff] ~~~ P4[...]
    end

    E --> P
    subgraph UI [Frontends]
        direction LR
        U1[TUI] ~~~ U2[Web] ~~~ U3[IDE] ~~~ U4[Plain] ~~~ U5[GitHub] ~~~ U6[JUnit]
    end

    P --> UI

    style P fill:transparent,stroke:#888
    style UI fill:transparent,stroke:#888
```

## File watcher

Watch mode is on by default in the TUI and web. scrutin monitors your project's source and test directories for changes using OS-level file notifications. When a file changes, the watcher passes it to the dependency tracker to figure out which tests are affected, then hands the result to the run engine.

When watch mode is off (any plain/github/junit/list reporter, or `--set watch.enabled=false`), runs are single-shot and this step is skipped entirely.

## Dependency tracker

When a file changes, the dependency tracker determines which test files need to re-run. It uses different strategies depending on the language.

**Filename matching** is always active and instant. Scrutin maps source file names to likely test files using naming conventions:

```
R/foo.R         -->  tests/testthat/test-foo.R
src/foo.py      -->  tests/test_foo.py
```

This covers most cases but can miss indirect dependencies.

**R runtime tracing** provides higher accuracy. During every test run, scrutin's R runner calls `trace()` on each function in the package namespace so that each source file whose functions get invoked is recorded. When the test finishes, the runner emits the list of touched source files back to the engine, which accumulates a `source → [tests]` map across runs. Because this is observed rather than inferred, only real call paths create edges: dynamic dispatch, method lookup, and helpers called through wrappers are all captured. The map is cached locally and refined every time a test executes. It is multi-suite aware: editing `R/math.R` invalidates test files under both `tests/testthat/` and `inst/tinytest/`.

**Python static import analysis** parses all `.py` files to extract `import` and `from ... import` statements, building a full import graph across both test and source files. Resolution is **transitive**: if `test_x.py` imports `helpers.py` which imports `core.py`, editing `core.py` will trigger `test_x.py`. The parser handles absolute imports, relative imports, submodule probing, multi-line parenthesized imports, and backslash continuation lines. Circular imports (allowed by Python) are handled safely. Dynamic imports (`importlib.import_module`) are missed. Unresolved changes fall back to the filename heuristic or a full suite re-run.

If no mapping is found for a changed file, all test files re-run.

## Run engine

The run engine is the core of scrutin. It takes a list of test files, partitions them by tool, and runs them through the appropriate plugin. Each tool gets its own pool of worker subprocesses running concurrently. A project with testthat + pytest gets two pools at once. All pools share a single cancel signal, so cancelling a run stops everything.

The engine communicates with worker subprocesses via NDJSON (newline-delimited JSON). File paths go to the worker on stdin; results come back from the forked child on a loopback TCP socket in fork mode, or on stdout in the non-fork / Windows fallback. Four message types flow back:

- **event**: one per test or validation step, carrying the outcome, test name, optional error message, line number, and duration.
- **summary**: one per file, carrying the authoritative wall-clock time.
- **deps**: one per file (R only), listing the source files whose functions were touched during the test.
- **done**: signals the worker is ready for the next file.

Every tool emits the same message format, so the engine and all frontends are tool-agnostic.

### Worker lifecycle

Workers are long-lived processes that keep your project loaded in memory. When a test file needs to run, scrutin sends the file path to an idle worker over stdin.

- **Startup**: the worker starts the language interpreter, runs an embedded runner script, and loads the project (`pkgload::load_all()` for R, `import` for Python). It then waits for file paths on stdin.
- **Fork isolation** (opt-in): by default (`fork_workers: false`), workers are killed and respawned after every file. This is the safe choice but pays the project-load cost on every file. Set `fork_workers: true` to keep workers alive and `fork()` a copy-on-write child per file: the project loads once, each child runs the test in an isolated COW clone, and exits. Fork mode is Linux/macOS only and is automatically disabled on Windows.

  **Why fork mode is opt-in**: forking a process that is already multithreaded, or whose test code itself spawns forked workers, can deadlock or crash the child. Common offenders are R's `parallel::mclapply` / `parallel::mcparallel`, Python's `multiprocessing` with the default `fork` start method, and BLAS/OpenMP-backed numerical libraries that hold internal thread pools. POSIX is explicit that only async-signal-safe code is legal between `fork()` and `exec()` in a multithreaded process; most R/Python packages do not respect that, so the safe default is to spawn fresh.
- **Crashes**: if a worker crashes mid-test, the error is recorded and the worker is automatically replaced. One crash never takes down the rest of the run.
- **Timeouts**: per-file timeouts are disabled by default. Set `timeout_file_ms` in config to any positive value to kill and replace workers that don't return within that many milliseconds.

### Outcome taxonomy

Every test result is classified into one of six outcomes:

| Outcome | Meaning | Breaks the build? |
|---------|---------|-------------------|
| `pass` | Assertion held | No |
| `fail` | Assertion broken | **Yes** |
| `error` | Could not evaluate (exception, missing dependency) | **Yes** |
| `skip` | Intentionally not run | No |
| `xfail` | Failed but expected, not a regression | No |
| `warn` | Soft issue, surfaced but does not break the build | No |

A file is considered failing when `failed > 0 || errored > 0`. This determines the exit code and CI gate behavior. Expected failures (`xfail`) and warnings (`warn`) are visible but do not break anything.

## Tool plugins

Each tool is compiled into the binary as a plugin. Plugins define how to detect the project, spawn workers, discover test files, and map source files to test files. Multiple plugins can be active simultaneously in the same project.

| Plugin | Language | Type | Mode |
|--------|----------|------|------|
| testthat | R | Unit testing | Worker |
| tinytest | R | Unit testing | Worker |
| pointblank | R | Data validation | Worker |
| validate | R | Data validation | Worker |
| jarl | R | Linting | Command |
| pytest | Python | Unit testing | Worker |
| ruff | Python | Linting | Command |
| Great Expectations | Python | Data validation | Worker |

**Worker mode** plugins use long-lived subprocesses that communicate via NDJSON, as described above.

**Command mode** plugins (jarl, ruff) run an external CLI tool directly per file and parse the output in Rust. No persistent subprocess is needed. This is appropriate for tools that are fast enough to invoke per file.

Plugins can define custom actions (e.g., jarl and ruff expose "fix", "fix (unsafe)", "fix all", "fix all (unsafe)"). The Detail view renders these as a numbered chip row under the warning message in both TUI and web; pressing `1`-`N` invokes the Nth action directly. Spell-check plugins (skyspell) attach per-warning `corrections` with ranked suggestions; those render as the same chip row, with `0` reserved for "add to dictionary".

## Frontends

The run engine streams results to whichever frontend is active. One frontend per invocation, selected with `--reporter` (`-r`):

- **TUI** (default): interactive terminal UI with file list, test detail, filtering, sorting, and keyboard navigation.
- **Web**: browser dashboard served on localhost with live updates via server-sent events.
- **IDE**: VS Code, Positron, and RStudio extensions that embed the web frontend.
- **Plain**: streaming text output for terminals and CI logs.
- **GitHub**: GitHub Actions annotations, log groups, and step summary.
- **JUnit**: XML report file compatible with CI systems that consume JUnit format.

## The `.scrutin/` directory

Created automatically in your project root. This directory should be gitignored (`scrutin init` adds it for you).

- **`state.db`**: DuckDB database storing run history and run metadata (git SHA, branch, hostname, CI provider, custom labels). Written via the `duckdb` CLI on a best-effort basis (no CLI = no history). Powers `scrutin stats` for flaky test detection and slow test ranking. Schema migrations drop and recreate tables, so version bumps may reset history.
- **`depmap.json` / `hashes.json`**: JSON cache for the runtime-traced source-to-test map and the xxhash64 file fingerprints that invalidate it. Written atomically after every run.
- **`runner_*.R` / `runner_*.py`**: embedded runner scripts, written at subprocess startup.
