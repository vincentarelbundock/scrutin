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

If your project uses both R and Python, all suites run concurrently in a single invocation.

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

Most tools scrutin wraps ship as their own binaries. Scrutin detects them at startup and refuses to run a suite whose binary is missing (turn this off with `[preflight] command_tools = false` if you know better).

| Tool | Kind | Install |
| ---- | ---- | ------- |
| [testthat](tools/testthat.md), [tinytest](tools/tinytest.md), [pointblank](tools/pointblank.md), [validate](tools/validate.md) | R packages | `install.packages("<name>")` in R |
| [jarl](tools/jarl.md) | Rust binary | `cargo install jarl` |
| [pytest](tools/pytest.md) | Python package | `pip install pytest` (or `uv add --dev pytest`) |
| [Great Expectations](tools/great-expectations.md) | Python package | `pip install great_expectations` |
| [ruff](tools/ruff.md) | Rust binary | `pip install ruff`, `brew install ruff`, or `cargo install ruff` |
| [skyspell](tools/skyspell.md) | Rust binary | `cargo install skyspell`; macOS additionally needs `brew install enchant hunspell` |

Worker-mode tools (testthat, tinytest, pytest, ...) are imported by scrutin's runner subprocess and must be resolvable in the worker's language environment. Command-mode tools (jarl, ruff, skyspell) just need to be on `PATH`.

## Supported tools

Scrutin supports the following tools. Each tool page includes a minimal example with directory structure and configuration. All matching tools activate automatically. See [Project Discovery](project-discovery.md) for the detection rules.

| Language | Unit tests | Data validation | Linter | Spell check |
| -------- | ---------- | --------------- | ------ | ----------- |
| R        | [testthat](tools/testthat.md), [tinytest](tools/tinytest.md) | [pointblank](tools/pointblank.md), [validate](tools/validate.md) | [jarl](tools/jarl.md) | [skyspell](tools/skyspell.md) |
| Python   | [pytest](tools/pytest.md) | [Great Expectations](tools/great-expectations.md) | [ruff](tools/ruff.md) | [skyspell](tools/skyspell.md) |

To restrict to one tool: `--set run.tool=testthat`.

## Quick setup

Generate a config file and `.scrutin/` directory:

```bash
scrutin init
```

This creates a `.scrutin/config.toml` with sensible defaults. You can customize workers, timeouts, filters, and more: see the [configuration reference](reference/configuration.md).
