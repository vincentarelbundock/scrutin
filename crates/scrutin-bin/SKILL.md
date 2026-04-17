---
name: scrutin
description: Run unit tests, linters, and data validators in R and Python projects using the scrutin test runner. Use when the user asks to run tests, check code quality, rerun failing tests, watch files for changes, or when `.scrutin/config.toml` or a `scrutin` binary is present. Covers testthat, tinytest, pytest, pointblank, validate, Great Expectations, jarl, ruff, skyspell, and typos.
---

# scrutin

scrutin is a watch-mode test runner for R and Python projects. It auto-detects every tool whose marker files are present (testthat, tinytest, pytest, pointblank, validate, Great Expectations), plus opt-in linters and spell-checkers declared in `.scrutin/config.toml` (jarl, ruff, skyspell, typos). A single invocation runs every active suite concurrently.

## When to use this skill

Use it when the user asks to:

- Run tests or lint a project
- Rerun only failing or flaky tests
- List which test files would run
- Produce a JUnit XML report or GitHub Actions annotations
- Watch files and re-run affected tests on each save
- Interpret a scrutin failure or configure `.scrutin/config.toml`

Detection: the project has a `.scrutin/` directory, `scrutin` is on `PATH`, or the codebase has testthat/tinytest/pytest marker files (`tests/testthat/`, `inst/tinytest/`, `tests/test_*.py`).

## Running scrutin

Always use the **plain reporter** (`-r plain`) when driving scrutin from an agent. It is deterministic, no-color-by-default, writes machine-friendly progress to stderr and a summary at the end, and exits non-zero on any failure.

```bash
scrutin -r plain                  # run every active suite once
scrutin -r plain path/to/project  # run in a specific directory
scrutin -r list                   # list files that would run, no subprocesses
scrutin -r junit:report.xml       # plain output + JUnit XML sidecar
scrutin -r github                 # GitHub Actions annotations + step summary
```

Do **not** launch `scrutin` with no `-r` flag from an agent. The default is the interactive ratatui TUI when stderr is a tty, which is unusable without a human at the keyboard.

## Useful flags

Config overrides go through `--set` (short: `-s`). Repeatable, TOML-parsed values:

```bash
scrutin -r plain --set run.max_fail=1        # stop after first failing file
scrutin -r plain --set run.reruns=2          # rerun failing files up to 2 extra times
scrutin -r plain --set run.workers=8         # pool size
scrutin -r plain --set run.tool=pytest       # restrict to one tool
scrutin -r plain --set filter.include='["test_math*"]'
scrutin -r plain --set filter.exclude='["test_slow*"]'
```

Named filter groups come from `[filter.groups.<name>]` in `.scrutin/config.toml`. Activate one with `-s filter.group=NAME`:

```bash
scrutin -r plain -s filter.group=fast
```

(In the TUI and web, `f` / `F` cycle the active group at runtime.)

## Interpreting output

Exit code is the source of truth: **0 = all files passed, non-zero = some file failed**. Do not parse counts from the text summary; trust the exit code.

The plain reporter prints one line per file, then a "Failures" block with the failure messages and a final counts line: `N passed  M failed  K skipped  ∷  Wms`. Failure messages include the test name and source location (`At: tests/testthat/test-plots.R:23`). That location is what you open to investigate.

For machine-readable structured output, prefer `-r junit:report.xml` over screen-scraping plain text.

## Configuration

scrutin reads `.scrutin/config.toml` (project root, ancestor-walked) and then `~/.config/scrutin/config.toml`. There are **no config environment variables** on purpose: `.scrutin/config.toml` is the only persistent source of truth.

To scaffold a config file and runner scripts for a new project:

```bash
scrutin init
```

Linters and spell-checkers are opt-in. Add them via `[[suite]]` blocks in `.scrutin/config.toml`:

```toml
[[suite]]
tool = "ruff"

[[suite]]
tool = "skyspell"
```

Auto-detected suites (testthat, tinytest, pytest, pointblank, validate, Great Expectations) do not need a `[[suite]]` entry unless the user wants to pin them.

## Common workflows

**"Run the tests"** → `scrutin -r plain`. Report the exit code and the failure block verbatim.

**"Why is this test failing?"** → Run `scrutin -r plain --set filter.include='["<file-stem>*"]'` to narrow to one file, read the failure trace, open the test file and the source file the trace points at.

**"Rerun only the failing tests"** → `scrutin -r plain --set run.reruns=2`. Files that pass on a rerun are marked flaky. For rerunning *only* previously-failed files in a new invocation, use `scrutin stats` to find them.

**"List what would run"** → `scrutin -r list`. No subprocesses spawned; honors filters.

**"CI setup"** → `scrutin -r github` on GitHub Actions (emits annotations and a step summary). `scrutin -r junit:report.xml` elsewhere.

GitHub Actions matrix workflow (cargo is pre-installed on every GitHub-hosted runner image, so `cargo install scrutin` works on all three OSes without a separate toolchain setup step):

```yaml
name: tests
on: [push, pull_request]
jobs:
  scrutin:
    strategy:
      fail-fast: false
      matrix:
        os: [ubuntu-latest, macos-latest, windows-latest]
    runs-on: ${{ matrix.os }}
    steps:
      - uses: actions/checkout@v4
      - uses: r-lib/actions/setup-r@v2          # drop if R-free
      - uses: actions/setup-python@v5           # drop if Python-free
        with:
          python-version: "3.12"
      - name: Install scrutin
        run: cargo install scrutin
      - name: Run tests
        run: scrutin -r github
```

`-r github` streams `::group::` / `::error` / `::warning` workflow commands (collapsible job-log sections plus inline PR annotations) and writes a Markdown summary to `$GITHUB_STEP_SUMMARY`. Add a separate `scrutin -r junit:report.xml` step if you also want a JUnit artifact for a test-results viewer. For larger matrices, cache Cargo state with `Swatinem/rust-cache` or install a prebuilt binary via `taiki-e/install-action` to skip the from-source compile.

### Bundled composite actions

The scrutin repository ships four composite actions under `.github/actions/` that you can reuse directly. Reference them as `vincentarelbundock/scrutin/.github/actions/<name>@<ref>`; pin `<ref>` to a release tag (e.g. `v0.0.7`) rather than `main`.

- **`install_scrutin`**: downloads the latest scrutin release binary via the official installer. Much faster than `cargo install scrutin` on every job, and works on Linux, macOS, and Windows runners.
- **`install_r`**: installs R plus the package's DESCRIPTION dependencies. On Linux it uses r-ci / r2u for binary apt-based package installs (fast cold-start); on macOS and Windows it falls back to `r-lib/actions`.
- **`install_python`**: installs `uv` and a pinned Python version. Optional `sync: "true"` input runs `uv sync` in a working directory so pytest sees the project.
- **`check_r`**: runs `R CMD check` with `--no-tests --as-cran`, then runs the package's tests via scrutin. The split is the point: `R CMD check` validates docs, examples, vignettes, and CRAN policy without re-running tests, and scrutin then runs them once with `-r github` so failures surface as PR annotations. Default `packages` input pulls in `tinytest` and `pkgload`; override via the `packages` input if the package needs more.

Prefer these over hand-rolled `cargo install` + `setup-r` + `setup-python` steps for R packages. A two-job workflow using them looks like:

```yaml
jobs:
  check-r:
    strategy: { fail-fast: false, matrix: { os: [ubuntu-latest, macos-latest, windows-latest] } }
    runs-on: ${{ matrix.os }}
    steps:
      - uses: actions/checkout@v4
      - uses: vincentarelbundock/scrutin/.github/actions/check_r@v0.0.7

  test-python:
    runs-on: ubuntu-latest
    steps:
      - uses: actions/checkout@v4
      - uses: vincentarelbundock/scrutin/.github/actions/install_python@v0.0.7
        with: { python-version: "3.12", sync: "true" }
      - uses: vincentarelbundock/scrutin/.github/actions/install_scrutin@v0.0.7
      - run: scrutin -r github --set run.tool=pytest
```

`check_r` also accepts `reporter:` (default `github`), `tool:` (restrict to one tool), `scrutin-args:` (extra args appended to the scrutin invocation), and `working-directory:` (package root; set this when the R package lives in a subdirectory of the repo rather than at the top level) inputs for customization.

**Watch mode** → Only meaningful for a human with the TUI or web dashboard open. Do not launch watch mode from an agent.

## Tool-specific knobs

Passthrough args live under `[<tool>]` tables in `.scrutin/config.toml`, e.g.:

```toml
[pytest]
extra_args = ["--tb=long", "-xvs"]

[skyspell]
extra_args = ["--lang", "en_US"]
```

Prefer these over growing scrutin-level flags. scrutin exposes a generic knob only when it makes sense across tools (`max_fail`, `failed_first`, `reruns`). Per-tool ergonomics belong in the tool's config section.

## Things to avoid

- Don't run scrutin with no reporter from an agent: you'll hang on the TUI.
- Don't parse counts from plain text. Trust the exit code.
- Don't suggest config env vars: scrutin has none. Use `--set` or `.scrutin/config.toml`.
- Don't enable watch mode in non-interactive invocations.
- Don't grow the CLI surface for one-off tool options; route them through `[<tool>].extra_args`.

## Further reading

- Docs site: <https://vincentarelbundock.github.io/scrutin>
- LLM-specific page: <https://vincentarelbundock.github.io/scrutin/llms/>
- Reporters: <https://vincentarelbundock.github.io/scrutin/reporters/>
- Configuration reference: <https://vincentarelbundock.github.io/scrutin/reference/configuration/>
