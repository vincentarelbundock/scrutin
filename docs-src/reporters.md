# Reporters

Scrutin supports several output modes, selected with `-r` / `--reporter`. It defaults to `tui` when stderr is a tty, and `plain` otherwise.

Available reporters: `tui`, `plain`, `github`, `web[:ADDR]`, `junit:PATH`, `list`.

## Plain

A compact text summary, suitable for CI and scripting. Exit code is 0 if all pass, 1 if any fail.

```bash
scrutin -r plain
```

```
● test-model.R          4 passed              87ms
✗ test-plots.R          2 passed  1 failed    43ms

── Failures ──

  FAIL  make_plot handles empty data
    `result` is NULL, not an S3 object with class "ggplot"
    At: tests/testthat/test-plots.R:23

7 passed  1 failed  0 skipped  ∷  143ms
```

Plain mode is one-shot. Use the TUI (the default) for live watch-mode feedback at the terminal.

## Web dashboard

A browser-based dashboard with live updates. The frontend is embedded in the binary: no Node.js or build step required.

```bash
scrutin -r web                   # binds to 127.0.0.1:7878
scrutin -r web:0.0.0.0:3000     # custom address
```

The dashboard uses server-sent events to stream results as they arrive. It binds to localhost only by default.

The same dashboard is available inside [VS Code, Positron, and RStudio](editors.md) through editor extensions.

## GitHub Actions

Emits `::group::` / `::endgroup::` blocks per file, `::error` and `::warning` annotations that surface inline on pull requests, and a Markdown summary written to `$GITHUB_STEP_SUMMARY`.

```bash
scrutin -r github
```

Single-shot: no watch loop, no reruns.

## JUnit XML

Writes a JUnit XML report alongside plain text output. Useful for CI platforms that parse JUnit results.

```bash
scrutin -r junit:report.xml
```

The report includes run metadata in `<properties>` and marks flaky tests.

## List

Lists the test files that would run without actually running them. Useful for verifying filter patterns.

```bash
scrutin -r list
```

## Flaky test detection

Set `run.reruns` to re-execute failing files:

```bash
scrutin --set run.reruns=2
```

A file that fails and then passes on rerun is marked **flaky**. Flaky results appear in the plain-mode summary, JUnit XML (`scrutin.flaky="true"`), and `scrutin stats` output.

## Run metadata

Scrutin records provenance for every run: version, OS, hostname, git SHA, branch, dirty state, and CI provider. This is written to JUnit XML and the local DuckDB history database.

Add custom labels with `--set metadata.extra.key=value`. Disable provenance capture with `[metadata] enabled = false` in `scrutin.toml`.
