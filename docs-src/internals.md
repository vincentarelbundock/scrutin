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
    subgraph UI [Outputs]
        direction LR
        U1[TUI] ~~~ U2[Web] ~~~ U3[Editors] ~~~ U4[Plain] ~~~ U5[GitHub] ~~~ U6[JUnit]
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

The run engine is the core of scrutin. It takes a list of test files, partitions them by tool, and runs them through the appropriate plugin. Each tool gets its own pool of worker subprocesses, but the pools run **sequentially**: one suite's workers come up, process every file assigned to that suite (in parallel up to the pool size), shut down, and the next suite's pool takes its place. Running the pools concurrently would mean paying the interpreter warm-up cost (`pkgload::load_all()`, `import mypkg`, ...) multiple times in parallel, which adds up fast. All pools share a single cancel signal, so cancelling a run stops everything.

The engine communicates with worker subprocesses via NDJSON (newline-delimited JSON). File paths go to the worker on stdin; results come back from the forked child on a loopback TCP socket in fork mode, or on stdout in the non-fork / Windows fallback. Four message types flow back:

- **event**: one per test or validation step, carrying the outcome, test name, optional error message, line number, and duration.
- **summary**: one per file, carrying the authoritative wall-clock time.
- **deps**: one per file (R only), listing the source files whose functions were touched during the test.
- **done**: signals the worker is ready for the next file.

Every tool emits the same message format, so the engine and all frontends are tool-agnostic.

### Worker lifecycle

Workers are subprocesses that load the language interpreter, run one or more test files, and emit results back to the engine. Startup runs an embedded runner script that loads your project (`pkgload::load_all()` for R, `import` for Python) and waits for file paths on stdin.

- **Isolation**: by default every test file runs in a fresh subprocess (slow but bulletproof); an opt-in fork mode keeps one parent loaded and forks a copy-on-write child per file (fast but risky around code that itself forks). See [Parallelism](parallelism.md) for the tradeoffs.
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

Each supported tool is compiled into the binary as a plugin. A plugin knows how to detect the right kind of project, discover files the tool should operate on, launch the tool, and map its output into scrutin's common event format. Multiple plugins can be active in the same run; see [Projects and Files](project-discovery.md) for the full list and for which tools auto-detect versus opt in.

Plugins come in two flavors:

- **Interpreter-integrated** (testthat, tinytest, pointblank, validate, pytest, Great Expectations). The test code runs inside a long-lived R or Python subprocess with your package imported, so you get the fastest iteration but the tool and your package must be installed in that interpreter.
- **Standalone CLI** (jarl, ruff, skyspell, typos). scrutin shells out to the tool's binary per file and parses its output. No interpreter coupling; just put the binary on `PATH`.

Plugins can expose per-file actions (e.g. jarl / ruff / typos "fix" variants). In the Detail view they render as a numbered chip row under the warning; pressing `1`-`N` invokes the Nth action. Spell-check plugins attach per-warning suggestions to the same chip row, with `0` reserved for "add to dictionary" (skyspell only).

## Frontends and reporters

The run engine streams results to whichever output is active, picked with `-r` / `--reporter`. [Frontends](frontends.md) are interactive (TUI, web dashboard, VS Code / Positron / RStudio embeds); [Reporters](reporters.md) are one-shot outputs for CI and scripting (plain text, JUnit XML, GitHub Actions annotations, list). All of them consume the same event stream from the engine, so adding a new output is a matter of writing one more consumer; none of the tool plugins or the engine have to change.

## The `.scrutin/` directory

Created automatically in your project root. This directory should be gitignored (`scrutin init` adds it for you).

- **`state.db`**: embedded SQLite database (via `rusqlite` with the `bundled` feature, so no external binary is required). Holds run history and run metadata (git SHA, branch, hostname, CI provider, CI build identifiers, custom labels), the source-to-test dependency map, and xxhash64 file fingerprints used to invalidate that map. Powers `scrutin stats` for flaky-test detection and slow-test ranking. See the [History](history.md) page for the full schema. The file is the single source of truth for local caches: delete it for a clean slate.
- **`runner_*.R` / `runner_*.py`**: embedded runner scripts, written at subprocess startup.
