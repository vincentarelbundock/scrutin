# ruff

A fast Python linter written in Rust. Like [jarl](jarl.md) for R, ruff is not a test framework: lint diagnostics map to `warn` events, clean files produce a synthetic `pass`, and lint issues appear alongside test results in the TUI and web dashboard. Scrutin auto-detects ruff when a ruff configuration marker is present (`ruff.toml`, `.ruff.toml`, or a `[tool.ruff]` section in `pyproject.toml`).

## Directory structure

```
myproject/
├── pyproject.toml
└── src/
    └── myproject/
        ├── __init__.py
        └── utils.py
```

## Minimal example

**pyproject.toml**

```toml
[project]
name = "myproject"
version = "0.1.0"

[tool.ruff]
line-length = 88
```

The `[tool.ruff]` section is enough to opt in. Alternatively, place a `ruff.toml` or `.ruff.toml` at the project root.

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

ruff runs concurrently alongside any test tools in the same project. It uses command mode (calling `ruff check --output-format json` directly), so no Python subprocess is needed.

## Plugin actions

Press `a` in the TUI (or the action button in the web) to open the action palette. ruff defines two entries, both of which run across the whole suite:

| Action | Description |
|--------|-------------|
| Ruff: fix all | Apply safe fixes to every file in the suite |
| Ruff: fix all (unsafe) | Apply safe + unsafe fixes to every file in the suite |

Both invoke `ruff check --fix` once with every matching file (after include / exclude filters) as trailing arguments. After a fix, the affected files are re-linted automatically.

## Configuration

No scrutin-specific configuration is required beyond the ruff config marker. ruff's own configuration (`ruff.toml`, `.ruff.toml`, or `[tool.ruff]` in `pyproject.toml`) controls which rules are enabled, excluded paths, and other linter settings.

To override the default suite in `.scrutin/config.toml`:

```toml
[[suite]]
tool        = "ruff"
test_dirs = ["."]
```

ruff lints from the project root by default. It has no source directories (each file is checked independently, no dependency tracking).
