# ruff

A fast Python linter written in Rust. Like [jarl](jarl.md) for R, ruff is not a test framework: lint diagnostics map to `warn` events, clean files produce a synthetic `pass`, and lint issues appear alongside test results in the TUI and web dashboard. ruff is opt-in: enable it with an explicit `[[suite]] tool = "ruff"` entry in `.scrutin/config.toml`, or pass files on the command line with `-t ruff` in [file mode](../project-discovery.md#file-mode).

## Installing ruff

ruff is not shipped with *Scrutin*. Follow the upstream install instructions at [docs.astral.sh/ruff](https://docs.astral.sh/ruff/).

## Directory structure

```
myproject/
├── .scrutin/
│   └── config.toml     # [[suite]] tool = "ruff"
├── pyproject.toml      # optional: ruff's own config under [tool.ruff]
└── src/
    └── myproject/
        ├── __init__.py
        └── utils.py
```

## Minimal example

**.scrutin/config.toml**

```toml
[[suite]]
tool = "ruff"
```

**pyproject.toml** (optional)

`pyproject.toml`'s `[tool.ruff]` section, or a standalone `ruff.toml` / `.ruff.toml` at the suite root, is read by ruff itself (not by *Scrutin*) to tune rules:

```toml
[tool.ruff]
line-length = 88
```

Omit it to use ruff's built-in defaults.

**src/myproject/utils.py**

```python
import os, sys  # E401: multiple imports on one line

def greet(name):
    x = 42  # F841: unused variable
    return f"hello, {name}"
```

## Running

```bash
scrutin myproject              # TUI
scrutin -r plain myproject     # text output
```

ruff runs as its own suite alongside any other suites you've declared; suites run one at a time, but within the ruff suite every matched file is linted in parallel. It uses command mode (calling `ruff check --output-format json` directly), so no Python subprocess is needed.

## Plugin actions

In the Detail view, ruff warnings show a numbered chip row of fix actions. Press the digit to invoke:

| Key | Action |
|-----|--------|
| `1` | Ruff: fix (this file) |
| `2` | Ruff: fix (this file, unsafe) |
| `3` | Ruff: fix all (suite) |
| `4` | Ruff: fix all (suite, unsafe) |

All four invoke `ruff check --fix` once with every matching file (after include / exclude filters) as trailing arguments. After a fix, the affected files are re-linted automatically.

## Configuration

The minimal suite entry is just `tool = "ruff"`. ruff's own configuration (`ruff.toml`, `.ruff.toml`, or `[tool.ruff]` in `pyproject.toml`) controls which rules are enabled, excluded paths, and other linter settings; *Scrutin* doesn't interpret it.

To override the default suite in `.scrutin/config.toml`:

```toml
[[suite]]
tool = "ruff"
# default `run` lints **/*.py under the suite root; `watch` defaults to `run`.
# Override to scope:
# run = ["src/**/*.py", "tests/**/*.py"]
```

ruff runs with `cwd = suite.root` so it picks up the local `pyproject.toml` / `ruff.toml`. Each file is checked independently, no dependency tracking.
