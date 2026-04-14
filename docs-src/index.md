---
title: scrutin
hide:
  - navigation
  - toc
  - title
---

<div class="hero" markdown>

<svg xmlns="http://www.w3.org/2000/svg" width="285" height="87" viewBox="0 0 285.02344 87" role="img" class="hero-logo">
  <title>Scrutin</title>
  <rect x="1" y="1" width="58" height="72" rx="5" ry="5" fill="none" stroke="currentColor" stroke-width="2"/>
  <rect x="11" y="15" width="11" height="11" rx="2" ry="2" fill="none" stroke="currentColor" stroke-width="1.8"/>
  <line x1="29" y1="20.5" x2="51" y2="20.5" stroke="currentColor" stroke-width="1.5" opacity="0.45"/>
  <rect x="11" y="33" width="11" height="11" rx="2" ry="2" fill="currentColor"/>
  <polyline class="hero-check" points="16.5,45.5 20,50 27,42.5" fill="none" stroke-width="1.6" stroke-linecap="round" stroke-linejoin="round" transform="translate(-3,-7)"/>
  <line x1="29" y1="38.5" x2="51" y2="38.5" stroke="currentColor" stroke-width="1.5" opacity="0.45"/>
  <rect x="11" y="51" width="11" height="11" rx="2" ry="2" fill="none" stroke="currentColor" stroke-width="1.8"/>
  <line x1="29" y1="56.5" x2="49" y2="56.5" stroke="currentColor" stroke-width="1.5" opacity="0.45"/>
  <circle cx="55" cy="65" r="16" fill="none" stroke="currentColor" stroke-width="2.5"/>
  <line x1="67" y1="77" x2="79" y2="85" stroke="currentColor" stroke-width="4" stroke-linecap="round"/>
  <text x="93" y="54.2" font-family="Menlo, ui-monospace, SFMono-Regular, monospace" font-size="48" font-weight="500" fill="currentColor" letter-spacing="-1">Scrutin</text>
</svg>

## Quality dashboard, file watcher, and parallel runner.

Scrutin discovers your R and Python unit tests, linters, and data validators automatically. It watches files for any edit you make, and uses dependency mapping to re-run only the checks that relate to your changes. The results are streamed live to a terminal UI, web browser dashboard, or into your editor.

[Get Started](getting-started.md){ .md-button .md-button--primary }

</div>

---

<div class="feature-section" markdown>

## See everything in one place

Every test, lint, and data validation result in a single dashboard. Filter by status, sort by name, time, or suite, and drill into failures to see expected vs. actual values with the failing line highlighted. Press `e` to jump straight into your editor.

</div>

<div class="screenshot-row" markdown>

![Terminal UI](assets/screenshot_tui_normal_mode.png){ .screenshot }

![Failure detail](assets/screenshot_tui_error_mode.png){ .screenshot }

</div>

---

<div class="feature-section" markdown>

## What it runs

</div>

<div class="grid cards" markdown>

-   :material-test-tube: **Unit tests**

    ---

    Run your test suites in isolated workers with live result streaming.
    Supports **pytest**, **testthat**, and **tinytest**.

-   :material-magnify-scan: **Code quality**

    ---

    Lint checks run through the same pipeline as tests, with diagnostics
    mapped to warnings and fix actions exposed as keyboard shortcuts.
    Supports **jarl** (R linter) and **ruff** (Python linter).

-   :material-database-check: **Data validation**

    ---

    Data quality checks run alongside code quality checks with the same
    outcome taxonomy (pass/fail/warn/skip/error) and rerun logic.
    Supports **pointblank** (R), **validate** (R), and **Great Expectations** (Python).

</div>

---

<div class="feature-section" markdown>

## Fast and focused

**Re-run only what changed.** Scrutin watches your project for file changes and uses dependency mapping to figure out which checks are affected. Edit a source file, and only the tests that depend on it re-run.

**Parallel execution.** Test files run concurrently across isolated workers. One crash never takes down the rest. Failing files are automatically retried to catch flaky tests.

**R and Python side by side.** Multiple tools can coexist in the same project. Scrutin detects each one automatically, runs them concurrently, and merges results into a single view.

</div>

---

<div class="feature-section" markdown>

## Editor integrations

</div>

<div class="screenshot-row three" markdown>

<div class="screenshot-card" markdown>
[![VS Code](assets/screenshot_editor_vscode.png){ .screenshot }](editors.md)

**VS Code**
{ .screenshot-label }
</div>

<div class="screenshot-card" markdown>
[![Positron](assets/screenshot_editor_positron.png){ .screenshot }](editors.md)

**Positron**
{ .screenshot-label }
</div>

<div class="screenshot-card" markdown>
[![RStudio](assets/screenshot_editor_rstudio.png){ .screenshot }](editors.md)

**RStudio**
{ .screenshot-label }
</div>

</div>

---

<div class="feature-section" markdown>

## Ship it!

</div>

<div class="grid cards" markdown>

-   :material-package-variant-closed: **Easy to install**

    ---

    Install a single binary. Works on macOS, Linux, and Windows.
    Just make sure R or Python are available on your system.

-   :material-file-document-outline: **Continuous integration**

    ---

    JUnit XML output for CI platforms. Exit code 0 or 1 for scripts.
    GitHub Actions annotations for inline comments on pull requests.

-   :material-history: **Run history**

    ---

    Every run is saved to a local DuckDB database. Track flaky tests,
    spot regressions, and compare run times across commits.

</div>
