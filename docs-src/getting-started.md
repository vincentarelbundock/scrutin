# Getting Started

!!! warning "Alpha software"
    scrutin is alpha software under active development. Expect bugs, breaking changes, and rough edges. Please report issues at [github.com/vincentarelbundock/scrutin](https://github.com/vincentarelbundock/scrutin).

Point scrutin at your project and it figures out the rest. It detects which test tools are present, discovers test files, and runs them in parallel.

```bash
scrutin                          # auto-detect, launch TUI (watch on by default)
scrutin -r plain                 # one-shot run, text output
scrutin -r web                   # browser dashboard
scrutin --set watch.enabled=false  # TUI, one-shot
```

If your project uses both R and Python, scrutin runs every active tool in a single invocation. Within each tool, files run in parallel; across tools, they run one after another so the interpreter only has to warm up once per tool. See [Parallelism](parallelism.md) for the tradeoffs and the opt-in fork mode that removes the warm-up cost.

## Installation

Prebuilt binaries are published with each [release](https://github.com/vincentarelbundock/scrutin/releases).

Install the latest release via shell script (macOS, Linux):

```bash
curl --proto '=https' --tlsv1.2 -LsSf https://github.com/vincentarelbundock/scrutin/releases/latest/download/scrutin-installer.sh | sh
```

Install the latest release via PowerShell (Windows):

```powershell
powershell -ExecutionPolicy Bypass -c "irm https://github.com/vincentarelbundock/scrutin/releases/latest/download/scrutin-installer.ps1 | iex"
```

## External tool dependencies

scrutin orchestrates third-party tools but does not ship them. Follow each tool's own install instructions on its project page:

| Tool | Kind | Homepage |
| ---- | ---- | -------- |
| testthat | R package | <https://testthat.r-lib.org/> |
| tinytest | R package | <https://cran.r-project.org/package=tinytest> |
| pointblank | R package | <https://rstudio.github.io/pointblank/> |
| validate | R package | <https://cran.r-project.org/package=validate> |
| jarl | Rust binary | <https://github.com/vincentarelbundock/jarl> |
| pytest | Python package | <https://docs.pytest.org/> |
| Great Expectations | Python package | <https://greatexpectations.io/> |
| ruff | Rust binary | <https://docs.astral.sh/ruff/> |
| skyspell | Rust binary | <https://codeberg.org/your-tools/skyspell> |
| typos | Rust binary | <https://github.com/crate-ci/typos> |

Test and data-validation tools (testthat, tinytest, pointblank, validate, pytest, Great Expectations) run inside an R or Python interpreter, so they must be installed as importable packages in the language environment scrutin uses for that suite (the active R library, or the suite's resolved Python virtualenv). Linters and spell checkers (jarl, ruff, skyspell, typos) are standalone binaries: just put them on `PATH`.

scrutin checks for the required binaries at startup and refuses to run a suite whose binary is missing, with a pointer to the tool's homepage. Turn that preflight off with `[preflight] command_tools = false` if you have a reason to bypass it.

## Supported tools

Each tool page includes a minimal example with directory structure and configuration. Test and data-validation tools activate automatically when their marker files are present; linters and spell checkers are opt-in via an explicit `[[suite]]` entry. See [Projects and Files](project-discovery.md) for the detection rules.

| Tool | Language | Category | Auto-detect |
| ---- | -------- | -------- | :---------: |
| [testthat](tools/testthat.md) | R | Unit tests | yes |
| [tinytest](tools/tinytest.md) | R | Unit tests | yes |
| [pointblank](tools/pointblank.md) | R | Data validation | yes |
| [validate](tools/validate.md) | R | Data validation | yes |
| [pytest](tools/pytest.md) | Python | Unit tests | yes |
| [Great Expectations](tools/great-expectations.md) | Python | Data validation | yes |
| [jarl](tools/jarl.md) | R | Linter | opt-in |
| [ruff](tools/ruff.md) | Python | Linter | opt-in |
| [skyspell](tools/skyspell.md) | Prose | Spell check | opt-in |
| [typos](tools/typos.md) | Any | Spell check | opt-in |

To restrict to one tool: `scrutin --tool testthat` (short form `-t`).

## Quick setup

Generate a config file and `.scrutin/` directory:

```bash
scrutin init
```

This creates a `.scrutin/config.toml` with sensible defaults. You can customize workers, timeouts, filters, and more: see the [configuration reference](reference/configuration.md).
