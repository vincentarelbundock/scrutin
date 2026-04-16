# Parallelism

*Scrutin* runs test files concurrently across a pool of worker subprocesses. The default pool size is `min(available_parallelism, 8)` with a minimum of 2. Each tool gets its own pool, but the pools run **sequentially**: one suite's workers start, chew through all its files in parallel, and shut down before the next suite's pool spins up. Running pools concurrently would mean paying the interpreter warm-up cost (`pkgload::load_all()`, `import mypkg`, ...) `workers × suites` times instead of `workers` times. Within any one suite every file still runs in parallel up to the pool size.

## Spawn

By default (`[run] fork_workers = false`), every test file runs in a fresh subprocess. The worker loads your project, runs one file, exits, and is replaced by a new one for the next file. This is the safest choice: process isolation is absolute, and nothing a test does (leaked threads, monkey-patched globals, loaded C libraries) can affect the next file.

The tradeoff is that you pay the project-load cost (`pkgload::load_all()`, `import mypkg`, etc.) on every file. For small projects this is invisible; for large ones it adds up.

## Fork

On Linux and macOS, set `[run] fork_workers = true` to trade some safety for speed. Each suite keeps one long-lived parent process with your project pre-loaded, and `fork()`s a copy-on-write child for each test file. The project loads once per worker, each child runs in an isolated COW clone, and exits. Re-runs on save are near-instant because no reload happens.

Fork mode is dangerous when the code under test forks on its own: R's `parallel::mclapply` / `mcparallel`, Python's `multiprocessing` with the default `fork` start method, and any BLAS/OpenMP-threaded numerical library can deadlock or crash the child when forked from an already-multithreaded parent. Leave `fork_workers = false` unless you are confident none of your tests (or their dependencies) do that. Fork mode is auto-forced off on Windows, where `fork()` is unavailable.

## Hooks

Two scopes of lifecycle hook attach to the pool model above. Both are configured under `[hooks]` in `.scrutin/config.toml` (see the [configuration reference](reference/configuration.md) for the full TOML shape).

**Process hooks** wrap the entire invocation. `[hooks] startup` runs once from the Rust binary before any pool starts; a non-zero exit aborts the run. `[hooks] teardown` runs once after the last result drains; failures log a warning but never mask the test exit code. Use these for work that must happen exactly once per invocation regardless of how many suites are active: starting a test database, seeding fixtures, flushing a coverage file.

**Worker hooks** (`[hooks.<language>]` / `[hooks.<language>.<tool>]`) fire inside each warm worker subprocess on boot and shutdown, so they pay the per-worker cost, not per-file. Under spawn mode that is every test file; under fork mode it is once per pool worker. Use these for in-process setup that must live alongside the tests: registering a testthat reporter, monkey-patching a module, priming an in-memory cache. Tool-level entries override language-level entries on a per-field basis, so `[hooks.python]` can define `worker_startup` for every Python tool while `[hooks.python.pytest]` overrides just `worker_teardown`.

Worker hooks only apply to warm-runner pools (testthat, tinytest, pointblank, validate, pytest, Great Expectations). Command-mode plugins (jarl, ruff, skyspell, typos) shell out to an external binary once per file and have no worker lifecycle; worker-hook entries for those tools are parsed but silently ignored. Put any setup they need into process-level `[hooks] startup` instead.
