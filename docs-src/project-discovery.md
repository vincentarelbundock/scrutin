# Project Discovery

Scrutin needs to know two things: which tools to run, and where the test files live. There are two ways to tell it.

**Tool**: a test runner or other utility that scrutin knows how to drive (testthat, pytest, jarl, etc.). Each tool is compiled into the binary as a plugin.

**Suite**: a configured instance of a tool, pointing at specific directories in your project. A project can have multiple suites, even multiple suites using the same tool with different directories.

## Explicit suites

Declare `[[suite]]` entries in `.scrutin/config.toml` to control exactly what runs. When at least one `[[suite]]` is present, auto-detection is skipped entirely.

```toml
[[suite]]
tool        = "testthat"
test_dirs   = ["tests/testthat"]
source_dirs = ["R"]

[[suite]]
tool        = "pytest"
test_dirs   = ["tests"]
source_dirs = ["src"]
```

Each suite requires two fields:

| Field | Type | Required | Description |
|-------|------|----------|-------------|
| `tool` | string | yes | Tool name (see table below). |
| `test_dirs` | array of strings | yes | Directories containing test files, relative to the project root. |
| `source_dirs` | array of strings | no | Source directories for watch-mode dependency tracking. |
| `runner` | string | no | Path to a custom runner script (replaces the built-in default). |

A suite can list multiple test directories. This is useful for linters that scan several locations:

```toml
[[suite]]
tool      = "jarl"
test_dirs = ["R", "scripts", "inst/examples"]
```

## Auto-detection

When no `[[suite]]` entries exist, scrutin scans the project root for marker files and activates every matching tool:

| Tool | Detected when |
|-----------|---------------|
| testthat | `DESCRIPTION` + `tests/testthat/` directory |
| tinytest | `DESCRIPTION` + `inst/tinytest/` directory |
| pointblank | `DESCRIPTION` + `tests/pointblank/` directory |
| validate | `DESCRIPTION` + `tests/validate/` directory |
| jarl | `jarl.toml` + `DESCRIPTION` + `R/` directory |
| pytest | `pyproject.toml` (or `setup.py`/`setup.cfg`) + `tests/` or `test/` directory, or `test_*.py` files at the root |
| ruff | `ruff.toml`, `.ruff.toml`, or `[tool.ruff]` in `pyproject.toml` |
| Great Expectations | `tests/great_expectations/` directory |

All matching tools activate. A project with both `tests/testthat/` and `inst/tinytest/` gets both suites.

To restrict auto-detection to a single tool:

```bash
scrutin --set run.tool=testthat
```

## Project root

The CLI path argument is the project root. If you don't pass one, scrutin uses the current directory.

```bash
scrutin                     # root is .
scrutin path/to/myproject   # root is path/to/myproject
```

All paths in `.scrutin/config.toml` (test_dirs, source_dirs, runner, hooks) are relative to this root.

Scrutin does not walk up to find a parent project. If you run `scrutin` from inside `tests/testthat/`, it looks for tools in `tests/testthat/`, not in the parent R package. Run it from the project root or pass the path explicitly.

## Source and test directories

In auto-detection mode, each tool has default directories:

### Test directories

| Tool | Defaults |
|-----------|----------|
| testthat | `tests/testthat` |
| tinytest | `inst/tinytest` |
| pointblank | `tests/pointblank` |
| validate | `tests/validate` |
| jarl | `R` |
| ruff | `.` (project root; ruff's own config handles exclusions) |
| pytest | `tests`, `test` |
| Great Expectations | `tests/great_expectations` |

Pytest also discovers `test_*.py` files at the project root, regardless of which test directory is active.

### Source directories

| Tool | Defaults |
|-----------|----------|
| testthat, tinytest, pointblank, validate | `R` |
| jarl, ruff | (none) |
| pytest, Great Expectations | `src`, `lib` |

Source directories are used for dependency tracking in watch mode and for the file watcher.

## Config file

Scrutin looks for `.scrutin/config.toml` in the project root directory. If none is found, it falls back to `~/.config/scrutin/config.toml`. See the [configuration reference](reference/configuration.md) for the full schema.

## Virtual environment detection (Python)

For Python projects, scrutin resolves the interpreter in this order:

1. `[python].interpreter` in `.scrutin/config.toml`
2. `[python].venv` in `.scrutin/config.toml` (path to a virtualenv directory)
3. `$VIRTUAL_ENV` environment variable
4. `.venv/` or `venv/` in the project root
5. `$CONDA_PREFIX` environment variable
6. `python3` on `$PATH` (or `python` on Windows)

Override in config:

```toml
[python]
venv = "my_env"           # relative to project root, or absolute
interpreter = "python3.12" # skip venv detection entirely
```

## Troubleshooting

**"No test tools detected"**: scrutin found no marker files in the directory you pointed it at. Either run from the project root, pass the root path explicitly, or add `[[suite]]` entries to `.scrutin/config.toml`.

**Wrong tool detected**: use `--set run.tool=pytest` to restrict auto-detection, or switch to explicit `[[suite]]` entries.

**Tests not found**: run `scrutin -r list` to see which test files scrutin discovers. If the list is empty, check that your test directory matches the defaults above, or declare the directory explicitly with `[[suite]]`.

**Non-standard layout**: declare your suites explicitly. Explicit `[[suite]]` entries bypass all auto-detection and let you point scrutin at any directory structure.
