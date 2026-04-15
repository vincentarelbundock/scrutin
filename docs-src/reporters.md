# Reporters

Reporters are one-shot outputs meant for CI and scripting. They run through your test files once, emit a structured result, and exit: no watch mode, no interactive drill-in. Pick one with `-r` / `--reporter`:

```bash
scrutin -r plain                 # compact text summary
scrutin -r junit:report.xml      # JUnit XML + plain text
scrutin -r github                # GitHub Actions annotations + step summary
scrutin -r list                  # list matching files, no execution
```

Exit code is 0 when every file passes and 1 when any file fails, so reporters slot cleanly into shell pipelines and CI gates.

For live, interactive views of a run (terminal UI, browser dashboard, editor panels), see [Frontends](frontends.md).

## Plain

A compact text summary, suitable for CI and scripting.

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

## JUnit XML

Writes a JUnit XML report alongside plain text output. Useful for CI platforms that parse JUnit results.

```bash
scrutin -r junit:report.xml
```

The report includes run metadata in `<properties>` and marks flaky tests.

## GitHub Actions

Purpose-built for GitHub Actions CI runs. Streams `::group::` / `::endgroup::` markers per file so the job log has collapsible sections, emits `::error` and `::warning` workflow commands so failures and lint warnings appear as inline annotations on the pull request, and writes a Markdown summary (a pass/fail/error table plus the full failure messages) to `$GITHUB_STEP_SUMMARY` so it renders on the job summary page.

```yaml
- name: Run tests
  run: scrutin -r github
```

Falls back gracefully on non-GitHub runners: the annotations are just echoed into stdout, and the summary write is skipped when the env var is missing.

## List

Lists the test files that would run without actually running them. Useful for verifying filter patterns.

```bash
scrutin -r list
```
