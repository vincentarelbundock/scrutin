# Frontends

A **reporter** controls how scrutin displays results. Select one with `-r` / `--reporter`. When no reporter is specified, scrutin defaults to `tui` if your terminal supports it, and `plain` otherwise.

```bash
scrutin                          # TUI (default)
scrutin -r plain                 # text summary
scrutin -r web                   # browser dashboard
scrutin -r junit:report.xml      # JUnit XML + plain text
scrutin -r list                  # list matching files, no execution
```

## Terminal UI

The default frontend. A two-pane, vim-style interface built with ratatui. It launches automatically when your terminal supports it.

The left pane shows your test files. The right pane previews results for the highlighted file. Press `j`/`k` to navigate, `Enter` to drill into a file's test results, and `Esc` to go back.

In detail mode, the left pane shows individual tests within the file, and the right pane shows the failure message and source context for the highlighted test. Press `Enter` on a failing test to see the full error with source code from both the test file and the source function it exercises.

Results stream in live, so you can browse and drill into failures while other tests are still running. Press `?` for a help overlay, `/` to filter, `s` to open the sort palette. Watch mode is on by default; disable it with `--set watch.enabled=false`. See [Keybindings](keybindings.md) for the full reference.

## Web dashboard

A browser-based dashboard with live updates. The frontend is embedded in the binary: no Node.js or build step required.

```bash
scrutin -r web                   # binds to 127.0.0.1:7878
scrutin -r web:0.0.0.0:3000     # custom address
```

The dashboard uses server-sent events to stream results as they arrive. It binds to localhost only by default. If the port is busy, scrutin tries the next one automatically.

The same dashboard is available inside [VS Code, Positron, and RStudio](editors.md) through editor extensions.

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

Plain mode is always one-shot. For a live feedback loop at the terminal, use the TUI (which has watch on by default).

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
