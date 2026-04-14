# Watch Mode

In watch mode, scrutin monitors your project and re-runs affected tests every time you save a file. It's on by default in the TUI and web: just run `scrutin` and start editing. Set `watch.enabled = false` in `.scrutin/config.toml` (or pass `--set watch.enabled=false`) to opt out. The plain, GitHub, JUnit, and list reporters are always one-shot regardless.

## What happens on save

1. The file watcher detects a change (with 50ms debounce to coalesce rapid saves)
2. The dependency tracker determines which test files are affected
3. If a source file changed, workers reload the package before running
4. If only a test file changed, workers run it immediately without reloading
5. Results stream into the frontend as they arrive

## Dependency mapping

Scrutin doesn't re-run your entire test suite on every change. It tracks which test files depend on which source files.

**R**: Runtime instrumentation. On the first watch run (or when source / test files have changed since the last build), scrutin instruments every function in the package namespace, runs each test file, and records which functions were called. The map is cached as JSON under `.scrutin/` and invalidated when file contents change. Multi-suite aware: editing `R/math.R` correctly invalidates test files under both `tests/testthat/` and `inst/tinytest/`.

**Python**: Static import analysis. Source and test files are scanned by a line-based Rust parser (`scrutin_core::python::imports`) for `import` and `from ... import` statements, building an inverted index from source modules to test files. Python dep analysis is rebuilt each invocation.

**Fallback**: When no mapping exists for a changed file, all test files re-run.

## Ignored files

The built-in walker also skips common build / VCS noise regardless of config (`.git/`, `target/`, `node_modules/`, `__pycache__/`, `.pytest_cache/`, `.Rproj.user/`, etc.). The `[watch] ignore` list adds user-supplied glob patterns on top, matched against paths relative to the project root. The default is `[".git", "*.Rhistory"]`.

Add your own patterns:

```toml
[watch]
debounce_ms = 50
ignore = [".git", "*.Rhistory", "renv/"]
```

## Worker pool

Scrutin keeps warm subprocesses with your project pre-loaded, so re-runs start instantly. The default pool size is `min(available_parallelism, 8)` with a minimum of 2. Each tool gets its own pool.

On Linux/macOS (`[run] fork_workers = true`, the default), each suite has one long-lived parent that `fork()`s a copy-on-write child per test file, giving both fast startup and full process isolation. With `fork_workers = false` (or on Windows, which is auto-forced off), workers are killed and respawned after every file. Crashed workers are replaced automatically.
