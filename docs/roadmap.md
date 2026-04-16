# Roadmap

Planned features that are not yet implemented.

## CTRF reporter

Emit [Common Test Report Format](https://ctrf.io) output via a new `-r ctrf:PATH` reporter, giving the R and Python ecosystems a producer for a schema that so far has traction only in JS/TS tooling.

## History dashboard

Turn `scrutin stats` into a history view over the SQLite database: pass/fail/flaky rates, average and p95 durations, retry-prone files, and a "what changed since last run" diff against a baseline run. The data is already captured on every run, so this is a query and presentation layer, not new instrumentation.

## Test coverage helpers

Dispatch to [covr](https://covr.r-lib.org) for R and [coverage.py](https://coverage.readthedocs.io) for pytest, aggregate the results across suites, and emit a unified summary with `term`, `html`, or `lcov` output configurable under `[coverage]`.

## Doc tests

Treat runnable examples in documentation as first-class test files: roxygen `@examples` blocks on the R side, [doctest](https://docs.python.org/3/library/doctest.html) (or `pytest --doctest-modules`) on the Python side. Each doc-test source becomes a synthetic file in the run with the same six-outcome taxonomy as regular tests.

## Expand internal test suite

Broaden *Scrutin*'s own `cargo test` coverage: more end-to-end runs against the `demo/` fixture across every plugin and every outcome in the six-value taxonomy, plus targeted tests for the engine seams (multi-suite fan-out, cancellation, rerun, watch-mode dep-map invalidation).

## More tool plugins

Extend the plugin registry to cover more of the R and Python validation ecosystem. Each tool drops into its language directory (`r/<tool>/` or `python/<tool>/`) with a `plugin.rs` and, where applicable, a runner companion, following the pattern already used by pointblank, validate, and great_expectations. Candidates: [pandera](https://pandera.readthedocs.io) and [pydantic](https://docs.pydantic.dev) on the Python side; further R data-validation packages as they appear.
