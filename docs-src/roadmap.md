# Roadmap

Planned features that are not yet implemented. For smaller in-progress items, see `TODO.md` in the repository.

## Server/IDE socket mode (headless NDJSON)

Run *Scrutin* as a long-lived background process that editor extensions connect to over a socket (Unix domain socket or loopback TCP), instead of re-spawning the binary for every action.

Motivation: editor extensions (VS Code, Positron, RStudio) currently shell out per run, losing warm R and Python worker subprocesses between invocations. A persistent socket keeps the pool hot and lets the editor subscribe to live events the same way the web SSE endpoint does, without HTTP overhead or browser-specific assumptions.

Shape:

- New reporter variant (e.g. `-r socket:/tmp/scrutin.sock`) that runs headless: no TUI, no web UI, no plain text output.
- The socket streams NDJSON events from `scrutin-core::engine::run_events` (FileFinished, Complete, cancellation, watch state).
- Clients send NDJSON commands back: run, rerun, rerun-failing, cancel, apply plugin action, toggle watch. Same verbs as the web's control routes.
- Wire format reuses `scrutin-web::wire` so editor clients and the browser dashboard stay schema-compatible.

## CTRF reporter

Emit [Common Test Report Format](https://ctrf.io) (CTRF) output via a new reporter variant (`-r ctrf:PATH`). CTRF is a JSON schema for test results with growing adoption in the JS/TS ecosystem (Playwright, Jest, Cypress reporters) but no existing pytest or R producer, so this is a producer-angle opportunity rather than an ingest one.

Shape:

- New file in `scrutin-bin/src/cli/reporter/`, enum variant in `cli/mod.rs`, one match arm. Same pattern as the JUnit and GitHub reporters.
- Six-outcome mapping uses CTRF's `rawStatus` field to preserve *Scrutin*'s finer taxonomy: `pass→passed`, `fail→failed+rawStatus=fail`, `error→failed+rawStatus=error`, `skip→skipped`, `xfail→skipped+rawStatus=xfail`, `warn→other+rawStatus=warn`.
- File-level granularity (one CTRF `test` per *Scrutin* file) for consistency with `max_fail`; per-expectation detail goes in `extra`.
- Multi-suite runs either emit one CTRF file per suite or list siblings in `extra`: decide at implementation time.
