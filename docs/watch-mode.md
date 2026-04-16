# Watch

In watch mode, *Scrutin* monitors your project and re-runs affected tests every time you save a file. It's on by default in the TUI and web: just run `scrutin` and start editing. Set `watch.enabled = false` in `.scrutin/config.toml` (or pass `--set watch.enabled=false`) to opt out. The plain, GitHub, JUnit, and list reporters are always one-shot regardless.

## What happens on save

1. The file watcher detects a change (with 50ms debounce to coalesce rapid saves)
2. The dependency tracker determines which test files are affected
3. If a source file changed, workers reload the package before running
4. If only a test file changed, workers run it immediately without reloading
5. Results stream into the frontend as they arrive

## Dependency mapping

*Scrutin* doesn't re-run your entire test suite on every change. It tracks which test files depend on which source files.

**R**: Runtime instrumentation. On the first watch run (or when source / test files have changed since the last build), *Scrutin* instruments every function in the package namespace, runs each test file, and records which functions were called. The map is persisted in SQLite (`.scrutin/state.db`, tables `dependencies` and `hashes`) and invalidated when file contents change. Multi-suite aware: editing `R/math.R` correctly invalidates test files under both `tests/testthat/` and `inst/tinytest/`.

**Python**: Static import analysis. *Scrutin* scans every `.py` file for `import` and `from ... import` statements and inverts the graph into a "source module → test files" index. Transitive: if `test_x.py` imports `helpers.py` which imports `core.py`, editing `core.py` triggers `test_x.py`. Dynamic imports (`importlib.import_module`) are missed; unresolved edits fall back to the filename heuristic or a full suite re-run. The Python index is rebuilt from scratch on every invocation (cheap enough that caching isn't worth the staleness risk).

**Fallback**: When no mapping exists for a changed file, all test files re-run.

## Ignored files

The built-in walker also skips common build / VCS noise regardless of config (`.git/`, `target/`, `node_modules/`, `__pycache__/`, `.pytest_cache/`, `.Rproj.user/`, etc.). The `[watch] ignore` list adds user-supplied glob patterns on top, matched against paths relative to the project root. The default is `[".git", "*.Rhistory"]`.

Add your own patterns:

```toml
[watch]
debounce_ms = 50
ignore = [".git", "*.Rhistory", "renv/"]
```

See [Parallelism](parallelism.md) for how *Scrutin* schedules workers across suites, and the safe-spawn vs. fast-fork tradeoff.
