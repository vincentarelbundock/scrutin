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

## Supported tools

Scrutin supports [testthat](tools/testthat.md), [tinytest](tools/tinytest.md), [pointblank](tools/pointblank.md), [validate](tools/validate.md), and [jarl](tools/jarl.md) for R, plus [pytest](tools/pytest.md) and [Great Expectations](tools/great-expectations.md) for Python. Each tool page includes a minimal example with directory structure and configuration. All matching tools activate automatically. See [Project Discovery](project-discovery.md) for the detection rules.

To restrict to one tool: `--set run.tool=testthat`.

## Quick setup

Generate a config file and `.scrutin/` directory:

```bash
scrutin init
```

This creates a `scrutin.toml` with sensible defaults. You can customize workers, timeouts, filters, and more: see the [configuration reference](reference/configuration.md).
