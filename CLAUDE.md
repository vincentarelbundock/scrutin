# CLAUDE.md

This file provides guidance to Claude Code (claude.ai/code) when working with code in this repository.

## Writing style

Never use em dashes or en dashes in code, documentation, comments, or commit messages. Use colons, commas, parentheses, or separate sentences instead.

## Project Overview

scrutin is a fast, watch-mode test runner with Rust orchestration. It watches a project's source and test directories, detects which test files are affected by a change, and re-runs only those tests in isolated subprocesses with a live ratatui terminal UI. It started as an R-only runner (testthat + tinytest) but now also supports Python (pytest), and **multiple tools/languages can coexist in the same project root** : a single invocation will detect every tool whose marker files are present and run them concurrently via per-suite worker pools.

**Status**: implemented, unreleased. Cargo workspace with four crates (`scrutin-core`, `scrutin-tui`, `scrutin-web`, `scrutin-bin`); ~7k lines of Rust plus an R companion (`crates/scrutin-core/src/r/runner.R`) and a pytest companion (`crates/scrutin-core/src/python/pytest/runner.py`). The web frontend lives in `scrutin-web` as a vanilla HTML+JS+CSS bundle embedded into the binary via rust-embed.

## Common commands

- `cargo build` : build everything
- `cargo test --workspace` : full test suite
- `cargo test -p scrutin-core -- <test_name>` : run a single test in a specific crate
- `cargo run -- -r plain demo` : smoke test against the combined fixture (testthat + tinytest + pytest in one project)
- `cargo run -- demo` : interactive TUI mode (default when stderr is a tty)
- `cargo run -- -w demo` : TUI + watch mode
- `cargo run -- -r plain -w demo` : plain + watch mode (re-runs affected tests on file change)
- `cargo run -- -r github demo` : GitHub Actions reporter (annotations, log groups, step summary)
- `cargo run -- -r junit:report.xml demo` : JUnit XML reporter (plain output + XML file)
- `cargo run -- -r list demo` : list test files that would run across **every active suite** after filters/excludes; spawns no subprocesses
- `cargo run -- -r plain --set run.max_fail=1 demo` : stop after the first failing **file** (cancels all suites)
- `cargo run -- --set run.workers=8 --set run.shuffle=true demo` : generic config override (`-s` short form)
- `cargo run -- -r plain --set run.reruns=2 demo` : re-execute failing files up to 2 extra times; passes on rerun are marked flaky
- `cargo run -- -r junit:r.xml --set metadata.extra.build=4521 demo` : add a build label to the JUnit `<properties>` block + DB
- `cargo run -- -r web demo` : browser dashboard (default `127.0.0.1:7878`)
- `cargo run -- init demo` / `cargo run -- stats demo` : verb subcommands
- `make revert` : restore demo lint files (`demo/R/lint.R`, `demo/src/scrutindemo_py/lint.py`) to their unfixed state
- `make install` : `cargo install` from the bin crate
- `make docs-serve` : generate CLI reference + serve docs site via zensical
- `make vscode` / `make positron` / `make rstudio` : build and install editor extensions

## Rust edition

The workspace uses **edition 2024**. This means `gen` is a reserved keyword, `unsafe` on extern blocks is required, and the MSRV is Rust 1.85+.

## Architecture

Cargo workspace with four crates:

- **`crates/scrutin-core`** : language-agnostic engine + per-language slices. The library all frontends depend on. Owns project discovery, the run engine, dep-map analysis, JUnit/DB persistence, and the embedded R/pytest companion scripts.
- **`crates/scrutin-tui`** : ratatui frontend. Depends on `scrutin-core`.
- **`crates/scrutin-web`** : axum-based browser dashboard. Depends only on `scrutin-core` (never on `scrutin-tui`). Bundles a vanilla HTML/CSS/JS frontend via rust-embed; no node at runtime. See `docs-src/web-spec.md` for the full design.
- **`crates/scrutin-bin`** : the `scrutin` binary. Owns argv parsing, config layering, plain-mode rendering, and the rerun loop. Depends on `scrutin-core`, `scrutin-tui`, and `scrutin-web`.

### `scrutin-core/src/` layout

```
lib.rs
├── r/                          ← all R-related code
│   ├── mod.rs                  registry (plugins()), shared helpers (parse_pkg_name, env, ...)
│   ├── parse.rs                tree-sitter R parser (defs, identifiers)
│   ├── depmap.rs               source→tests dep map (multi-suite-aware)
│   ├── testthat/{mod,plugin,runner}.rs/.R
│   ├── tinytest/{mod,plugin,runner}.rs/.R
│   ├── pointblank/{mod,plugin,runner}.rs/.R  data-validation plugin
│   ├── validate/{mod,plugin,runner}.rs/.R   data-validation plugin (validate pkg)
│   └── jarl/{mod,plugin}.rs                  R linter plugin (opt-in via jarl.toml)
├── python/                     ← all Python-related code
│   ├── mod.rs                  registry
│   ├── imports.rs              import-graph dep map
│   ├── pytest/
│   │   ├── mod.rs
│   │   ├── plugin.rs
│   │   └── runner.py           embedded pytest companion script
│   ├── ruff/{mod,plugin}.rs    Python linter plugin (command mode, like jarl)
│   └── great_expectations/{mod,plugin}.rs  data-validation plugin
├── analysis/                   ← cross-language utilities only
│   ├── walk.rs                 shared filesystem walker + ignore list
│   ├── deps.rs                 resolve_tests (cross-language test resolver)
│   └── hashing.rs              multi-suite content fingerprints
├── project/
│   ├── package.rs              Package + TestSuite (multi-suite data model)
│   ├── config.rs               .scrutin/config.toml parsing + --set overrides
│   └── plugin.rs               Plugin trait + PluginAction + all_plugins() registry
├── engine/
│   ├── run_events.rs           the run-engine seam (RunEvent, RunHandle, start_run)
│   ├── pool.rs                 per-suite async worker pool
│   ├── runner.rs               single-subprocess management
│   ├── protocol.rs             NDJSON wire types
│   └── watcher.rs              notify-based file watcher
├── report/junit.rs             JUnit XML writer
├── storage/sqlite.rs           embedded SQLite (rusqlite, bundled):
│                               runs + results + extras + dependencies + hashes
└── filter.rs, git.rs, hooks.rs, logbuf.rs, metadata.rs
```

**Adding a new language** = drop a sibling directory next to `r/` and `python/`, register the plugins in `project/plugin.rs::all_plugins()`. No edits anywhere else.

**Adding a new tool to an existing language** = drop a sibling directory next to `r/testthat/` and `r/tinytest/`, register in that language's `mod.rs::plugins()`. The shared helpers in `r/mod.rs` cover the boilerplate; new tool files are typically ~60 lines of `Plugin` trait impl. Non-test tools (linters, validators) follow the same pattern: jarl maps lint diagnostics to `warn` events, pointblank maps validation steps to `pass`/`fail`.

**Plugin actions** = plugins define actions via `Plugin::actions() -> Vec<PluginAction>`. Each action has a name, label, command (file paths appended), `rerun: bool`, and `scope: ActionScope` (File or All). In the TUI and web, pressing `a` opens an action palette listing all available actions for the selected file's suite. `ActionScope::File` runs the command on the currently selected file; `ActionScope::All` runs the command on every file in the suite (after include/exclude filters) in a single invocation. After execution, the affected files are optionally re-run. Example: jarl and ruff both expose "fix" / "fix (unsafe)" (file-scope) and "fix all" / "fix all (unsafe)" (all-suite) so users can either touch just the current file or sweep the whole suite.

### Key modules

- **`scrutin-bin/src/cli/mod.rs`** : `Cli` struct (clap `Subcommand`-based: default `run`, plus `init` / `stats`), `RunArgs`, `ReporterSpec` + `resolve_reporter`, top-level orchestration and subcommand dispatch, init scaffolding. One reporter per invocation via `--reporter` (`-r`): `tui`, `plain`, `github`, `web[:ADDR]`, `list`, `junit:PATH`. Watch mode (`-w`) applies to TUI and web. Config layering: defaults, `.scrutin/config.toml`, `--set`, surviving CLI flags.
- **`scrutin-bin/src/cli/reporter/mod.rs`** : shared reporter types (`FileRecord`, `RunAccumulator`, `FileTally`, `RunStats`) and helpers (`tally_messages`, `collect_failed_files`, `replace_results`). All non-TUI, non-web reporters depend on this.
- **`scrutin-bin/src/cli/reporter/plain.rs`** : plain-mode reporter. Watch loop, rerun logic, JUnit sidecar output, DB persistence, text rendering. Runs always go through `run_via_engine`, which calls `run_events::start_run`, so the same multi-suite seam serves all frontends.
- **`scrutin-bin/src/cli/reporter/github.rs`** : GitHub Actions reporter (`-r github`). Streams `::group::`/`::endgroup::` per file, emits `::error`/`::warning` annotations for inline PR feedback, writes a markdown summary to `$GITHUB_STEP_SUMMARY`. Single-shot (no watch, no reruns). Adding a new reporter = one new file here + enum variant + match arm in `cli/mod.rs`.
- **`scrutin-core::engine::run_events`** : the run-engine seam. Defines `RunEvent`, `RunHandle`, `FileResult`, and `start_run()`, the **only** entry point any frontend should use. **Owns multi-suite fan-out**: partitions the input file list by `pkg.suite_for(file)`, spawns one `ProcessPool` per non-empty suite (sharing one `CancelHandle` so `cancel_all()` propagates), and multiplexes their `FileResult`s into a single `mpsc::UnboundedReceiver<RunEvent>`. Consumers downstream don't see suites at all.
- **`scrutin-core::engine::pool`** : async worker pool for *one* suite: a `VecDeque<RProcess>` queue gated by a `Semaphore`, any-free-worker assignment, per-file timeouts, cancellation, pool poisoning on startup-hook failure. A pool always corresponds to a single `TestSuite`; multi-tool projects get multiple pools, all created and joined inside `run_events::start_run`.
- **`scrutin-core::engine::runner`** : spawns and manages a single subprocess via `tokio::process`; takes `(pkg, suite)` so it knows which plugin's `subprocess_cmd` / `runner_basename` / `worker_hooks` to use.
- **`scrutin-core::engine::protocol`** : NDJSON message types per `docs/specs/reporting.md`. Three top-level variants: `event` (carries an `Outcome` from the six-value taxonomy + `Subject` + optional `Metrics`/`failures`), `summary` (per-file authoritative `duration_ms`), `done`. Schema is mirrored independently in `r/runner.R` and `python/pytest/runner.py` : change all three together. The taxonomy and consumer policies (events authoritative for counts, summary authoritative for wall time, `bad_file = failed > 0 || errored > 0`) are pinned in the spec doc.
- **`scrutin-core::project::package`** : `Package` carries `test_suites: Vec<TestSuite>`; one suite per detected tool, each with its own `plugin` / `root` / `run` (glob patterns for input files) / `watch` (glob patterns for dep-map triggers) / `worker_hooks`. `tool_names()` returns a `+`-joined label (e.g. `tinytest+testthat+pytest`); `suite_for(path)` is the routing primitive (matches `run` globs + plugin predicate). Each suite's `root` drives both the subprocess CWD (`cmd.current_dir`) and the `SCRUTIN_PKG_DIR` env var, so `pkgload::load_all()` / `pytest` / venv / `ruff.toml` discovery find the right subtree in a monorepo. **Never reach into a single plugin** : iterate `test_suites` or call `is_any_test_file` / `is_any_source_file`.

- **Project root vs suite root**: `Package.root` is the project root (where `.scrutin/config.toml` lives; anchors `state.db`, runner scripts, hooks, git metadata). `TestSuite.root` is the suite root (per-suite working directory). In single-package projects and auto-detection, they're equal. In monorepos (`[[suite]] root = "r"`, `[[suite]] root = "python"`), each suite points at its own subtree.
- **`scrutin-core::analysis::hashing`** : multi-suite content fingerprints via `hash_package_files(pkg)`. Walks every active suite's source + test dirs through the shared `analysis::walk` helper. `is_dep_map_stale(pkg, db)` and `snapshot_hashes(pkg, db)` both take `&Package`.
- **`scrutin-core::r::depmap`** : multi-suite-aware. `build_dep_map(pkg)` iterates every R suite (testthat *and* tinytest), so editing `R/math.R` correctly invalidates test files under both `tests/testthat/` and `inst/tinytest/`.
- **`scrutin-tui/src/lib.rs`** : `run_tui` event loop, `start_test_run` (the TUI's bridge into `run_events::start_run`).
- **`scrutin-tui/src/{state,keymap,input}.rs`** + **`scrutin-tui/src/view/`** : modal TUI. `AppState` is decomposed into named sub-structs by concern: `nav` (mode_stack, cursors, scrolls, viewport heights), `filter` (text/status/suite/outcomes), `display` (sort, watch, layout pct), `multi` (multi-selection), `run` (running, totals, busy/cancel handles). Adding a field has an obvious home; field accesses self-document. The view tree is split into `view/{mod,layout,icons,source,sort,overlays,file_list,counts,hints,breadcrumb,log,normal,detail,failure}.rs` with `mod.rs` owning top-level dispatch.
- **TUI mode taxonomy** : `Mode` historically conflated drill *level* (Normal/Detail/Failure) and *overlay* (Help/Log/ActionOutput/Palette). `Level` and `Overlay` enums separate these so dispatch sites can target the axis they care about: `state.level()` returns the topmost non-overlay frame; `state.overlay_kind()` returns `Some(Overlay::*)` if an overlay sits on top. New code prefers these typed accessors over `state.mode()` matches. `Mode` is kept as the stack-frame type for backwards compatibility.
- **TUI cursor dispatch** : `AppState::move_cursor(mode, delta)` is the single seam for per-mode cursor movement (`isize::MIN/MAX` for top/bottom). Each mode targets its own cursor (`file_cursor` / `test_cursor` / `failure_cursor` / `log_scroll` / `overlay.scroll`); adding a new cursor target is one method change, not 6 dispatch arms.
- **Overlays** share a single `OverlayState` struct (scroll, view_height, optional cursor) and `draw_text_overlay` renderer. Two flavors: text overlays (Help, ActionOutput) are scroll-only; menu overlays (Run, Sort, Action palettes) have a cursor. `PaletteKind::Action` is the fix/action menu opened by `a`.

Communication protocol: NDJSON over the worker's stdout, one message per line.

Config precedence: defaults → `.scrutin/config.toml` (ancestor-walked from project root, fallback `~/.config/scrutin/config.toml`) → `--set` overrides → CLI flags. **scrutin intentionally has no config env vars** : `.scrutin/config.toml` is the only persistent source of truth.

## Fixtures

- `demo/` : single project root containing **all tools side-by-side**: an R package (DESCRIPTION + R/) with testthat tests under `tests/testthat/` and tinytest tests under `inst/tinytest/`, plus a Python package (`pyproject.toml` + `src/scrutindemo_py/`) with pytest tests under `tests/test_*.py`. Every test file is intentionally engineered to exercise one of the six-outcome taxonomy buckets (pass/fail/error/skip/xfail/warn : see `docs-src/specs/reporting.md`). `demo/jarl.toml` opts in to the jarl suite; ruff activates via `pyproject.toml`. This is the canonical multi-suite smoke test.
- `demo/R/lint.R` and `demo/src/scrutindemo_py/lint.py` : intentionally messy files with lint violations for testing jarl/ruff fix actions. These are tracked in git so `make revert` (or `git checkout -- demo/R/lint.R demo/src/scrutindemo_py/lint.py`) restores them after fix actions modify them. Never commit the fixed versions.

### `scrutin-web/src/` layout

```
lib.rs                          ← pub run_web(addr, pkg, ...) entry point
├── server.rs                   axum Router + loopback middleware
├── state.rs                    AppState (shared), spawn_run, forwarder task
├── wire.rs                     WireFile/WireMessage/WireEvent + core→wire translation
├── assets.rs                   rust-embed macro over frontend/
└── routes/
    ├── mod.rs                  api_router() + static_router()
    ├── snapshot.rs             GET /api/{snapshot,files,file/{id},suites,config}
    ├── events.rs               GET /api/events (SSE with broadcast fan-out)
    ├── control.rs              POST /api/{run,rerun,rerun-failing,cancel,watch,open-editor,plugin-action}
    └── static_files.rs         SPA fallback over embedded dist/
```

Frontend lives in `crates/scrutin-web/frontend/` as three static files
(`index.html`, `app.js`, `style.css`). No build step, no node at runtime;
rust-embed bakes them into the binary. Launched with `scrutin -r web[:ADDR]`
(default `127.0.0.1:7878`). The server binds loopback-only and every
route is additionally wrapped in a `require_loopback` middleware. See
`docs-src/web-spec.md` for the full API surface and design rationale.

### Editor extensions

`editors/` contains IDE integrations: a **VS Code / Positron** extension (`editors/vscode/`, TypeScript) and an **RStudio** addin (`editors/rstudio/`, R package). Built via `make vscode` / `make positron` / `make rstudio`.

### Doc generation

`cargo run --features generate-docs -- generate-docs target/docs` produces CLI reference, man pages, and shell completions. The `generate-docs` feature flag gates this codepath. `make docs` runs this + `zensical build` for the full doc site.

## Key Design Decisions

- **Cargo workspace, four crates.** `scrutin-core` is the library both `scrutin-tui` and `scrutin-web` depend on. `scrutin-bin` is the `scrutin` binary, depending on core + tui + web. The split exists so language-agnostic engine code stays library-shaped and multiple frontends can be added without cross-cutting edits.
- **Per-language top-level modules** (`r/`, `python/`). Anything language-specific : plugins, parsers, dep-map builders, runner companions : lives under one tree per language. `analysis/` is reserved for genuinely cross-language utilities (`walk`, `deps`, `hashing`). Adding a new language = one new top-level dir + one line in `project/plugin::all_plugins()`.
- **tokio for subprocess/IO/event-loop.** Subprocess stdio, the worker pool, the watcher, and the TUI event loop are all async.
- **TUI keeps `start_test_run` sync** even though it spawns async work. This is deliberate: `handle_key` holds a `std::sync::MutexGuard` across calls, and making it `async` would force the future to be `!Send`. `start_test_run` instead spins a `tokio::spawn` internally and returns immediately.
- **Run-engine seam in `run_events.rs`** so the TUI is not the only possible consumer. `RunEvent::{FileFinished, Complete}` is the contract any future frontend (web view, JSON event stream, LSP) plugs into.
- **Responsive TUI layout collapses, never hides.** `tui/view.rs::split_panes` decides list+main pane sizing from terminal width, focus, and per-mode `screen_mode`. Below the minimums it collapses to a single (focused) pane instead of disappearing. All breakpoints (`MIN_LIST_COLS`, `MIN_MAIN_COLS`, `FILE_DETAIL_MIN_COLS`, `HINTS_BAR_MIN_ROWS`, `COUNTS_BAR_MIN_ROWS`, `MIN_TERMINAL_COLS/ROWS`) are `pub(super) const` in `tui/state.rs` : tune in one place. Resize never mutates focus/`screen_mode`; layout is recomputed each frame.
- Crossterm has no native async, so key polling runs on `tokio::task::spawn_blocking` and emits via an mpsc channel.
- Plugin trait so new languages don't require touching the engine. Plugins live under their language's top-level dir.
- **Multi-suite via per-pool fan-out, not per-file routing in one pool.** A `ProcessPool` is bound to one suite for the lifetime of a run; mixing testthat + pytest means *two pools running concurrently*, each with its own warm worker subprocesses. The fan-out happens once, in `run_events::start_run`, so neither the pool nor the consumer (TUI / plain mode) needs to know how many suites exist. Adding a third tool to a project is a config-free change : `detect_plugins` finds it, `start_run` spawns another pool.
- **`max_fail` is file-level, not expectation-level.** A single crashing file counts as one bad file regardless of how many expectations it took down with it. Documented in `.scrutin/config.toml` and enforced via `RunAccumulator::failed_files`.
- **Single tally walker.** `tally_messages` is the only place plain mode classifies a `Message`; both the live `RunAccumulator::push` path and the post-rerun `RunAccumulator::from_results` recomputation route through it so they cannot drift.
- SQLite (embedded via rusqlite, bundled) for history and local caches. One `.scrutin/state.db` holds runs, results, user extras, the dep map, and file-hash fingerprints.
- jsonlite-tolerant deserializers in `protocol.rs` because R serializes `NULL` as `{}`.
- **Plugin escape hatches over scrutin-side abstractions.** When users need an obscure tool knob (e.g. `--tb=long` for pytest), prefer a verbatim `extra_args`-style passthrough in the relevant `[<plugin>]` config section over growing an scrutin-level config field. The test for "should scrutin grow first-class config for X?" is *does X make sense for the R plugins too?* `max_fail` and `failed_first` pass that test; `traceback` and `verbosity` don't.
- **Custom runner scripts via config.** Each tool section (`[testthat]`, `[tinytest]`, `[pytest]`) accepts a `runner` field pointing to a local script. When set, the engine reads that file instead of the built-in default. `scrutin init` writes the default runners to `.scrutin/<tool>/runner.<ext>` so users can edit them (e.g. swap `pkgload::load_all()` for `library()`, add project-specific setup). The config line is commented out by default; the built-in is used unless explicitly overridden.
- **Modal TUI as orchestrator.** The TUI is structured around modes that own their own keymap tables and layout preferences. The mode chip in the hints bar advertises which mode is active; help and hints are auto-generated from binding tables; Esc uniformly pops a mode stack frame. New modes are a `Mode` variant + a `&[Binding]` slice + (sometimes) an extras handler : not a new dispatch branch in `handle_key`. Plugin actions open via `a` as a `PaletteKind::Action` menu; the action output appears in a scrollable `Mode::ActionOutput` overlay. Both use the shared `OverlayState`.
- **Sort modes match across TUI and web.** Five sort modes (sequential, status, name, suite, time) are available in both frontends. `s`/`S` cycles sort mode; `t`/`T` cycles the status filter. The web exposes the same modes via a dropdown and keyboard shortcuts. Status rank order is sourced from `scrutin_core::engine::protocol::Outcome::rank()` (TUI reads directly; web receives `outcome_order` in `/api/snapshot`).
