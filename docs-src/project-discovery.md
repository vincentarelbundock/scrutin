# Project Discovery

Scrutin needs to know three things: which tools to run, where each tool should run from, and which files it should operate on. There are two ways to tell it.

**Tool**: a test runner or other utility that scrutin knows how to drive (testthat, pytest, jarl, etc.). Each tool is compiled into the binary as a plugin.

**Suite**: a configured instance of a tool, with its own working directory and file patterns. A project can have multiple suites, even multiple suites using the same tool.

## Project root vs suite root

Two distinct concepts, used throughout the docs:

- **Project root**: where `.scrutin/config.toml` lives. Anchors shared state: `.scrutin/state.db`, runner scripts under `.scrutin/`, startup/teardown hooks, `.gitignore` entries, git metadata. This is the address you pass on the command line.
- **Suite root**: the directory one suite's tool runs from. Every suite has one. In a single-package project (or under auto-detection) it equals the project root; in a monorepo with an R package at `r/` and a Python package at `python/`, each suite points at its own subtree.

Setting a suite's `root` is what makes `pkgload::load_all()` find the right `DESCRIPTION`, pytest find the right `pyproject.toml` and `.venv`, and ruff/jarl find the right config file: the engine spawns the tool with `cwd = suite.root` and sets `SCRUTIN_PKG_DIR = suite.root`.

## Explicit suites

Declare `[[suite]]` entries in `.scrutin/config.toml` to control exactly what runs. When at least one `[[suite]]` is present, auto-detection is skipped entirely.

```toml
[[suite]]
tool = "testthat"

[[suite]]
tool = "pytest"
```

Each suite takes the following fields:

| Field | Type | Required | Description |
|-------|------|----------|-------------|
| `tool` | string | yes | Tool name (see table below). |
| `root` | string | no | Suite root, relative to the project root (or absolute). Default: `.` (the project root). |
| `run` | array of glob patterns | no | Files the tool operates on (tests to execute, files to lint). Relative to `root`. Default: plugin-provided globs. |
| `watch` | array of glob patterns | no | Files watched to trigger reruns. Default: plugin-provided (or same as `run` for linters). |
| `runner` | string | no | Path to a custom runner script (replaces the built-in default), relative to the project root. |

Unknown keys are a hard error, so typos surface immediately.

### Monorepo layout

When sibling packages live in different subdirectories, give each suite its own `root`:

```toml
[[suite]]
tool = "testthat"
root = "r"

[[suite]]
tool = "tinytest"
root = "r"

[[suite]]
tool = "pytest"
root = "python"

[[suite]]
tool = "ruff"
root = "python"
```

Each suite's subprocess runs with `cwd = <root>`. Relative file I/O inside tests (`read.csv("inst/extdata/foo.csv")`, `open("tests/fixtures/x.txt")`) resolves against that subtree.

### Custom globs

`run` and `watch` accept glob patterns in the `globset` dialect (`*`, `**`, `?`, `[abc]`, `{a,b}`). Literal file paths are valid globs (they match exactly themselves).

```toml
[[suite]]
tool  = "pytest"
root  = "python"
run   = ["tests/**/test_*.py"]
watch = ["marginaleffects/**/*.py"]
```

Relative patterns are anchored under `root`; absolute patterns are used verbatim. There is no directory-recursion shortcut: `run = ["tests"]` matches the literal path `tests`, not files under it. Write `tests/**/*.py` for recursion.

### Scattered files

Because `run` is a glob list, files can live in unrelated directories:

```toml
[[suite]]
tool = "testthat"
root = "/home/me/sketch"
run  = ["foo_test.R", "../misc/bar_test.R"]
```

No shared-ancestor requirement: routing is per-glob match.

## Auto-detection

When no `[[suite]]` entries exist, scrutin scans the project root for marker files and activates every matching tool. Every auto-detected suite gets `root = pkg.root` (the project root).

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

Auto-detection only scans the project root. Monorepos with packages in subdirectories must use explicit `[[suite]]` declarations (subdirectory names are arbitrary, so there is no safe heuristic).

To restrict auto-detection to a single tool:

```bash
scrutin --set run.tool=testthat
```

## Project root on the CLI

The CLI path argument is the project root. If you don't pass one, scrutin uses the current directory.

```bash
scrutin                     # project root is .
scrutin path/to/myproject   # project root is path/to/myproject
```

Scrutin does not walk up to find a parent project. If you run `scrutin` from inside `tests/testthat/`, it looks for tools in `tests/testthat/`. Run it from the project root or pass the path explicitly.

## Default globs

In auto-detection mode (and when `run`/`watch` are omitted in an explicit `[[suite]]`), each plugin provides its own default patterns.

### `run` (inputs)

| Tool | Defaults |
|-----------|----------|
| testthat | `tests/testthat/**/test-*.R`, `test_*.R`, `test-*.r` |
| tinytest | `inst/tinytest/**/test-*.R`, `test_*.R`, `test-*.r` |
| pointblank | `tests/pointblank/**/test-*.R`, `test_*.R`, `test-*.r` |
| validate | `tests/validate/**/test-*.R`, `test_*.R`, `test-*.r` |
| jarl | `R/**/*.R`, `R/**/*.r` |
| pytest | `tests/**/test_*.py`, `tests/**/*_test.py`, `test/**/test_*.py`, `test/**/*_test.py`, `test_*.py`, `*_test.py` |
| ruff | `**/*.py` |
| Great Expectations | `tests/great_expectations/**/test_*.py`, `tests/great_expectations/**/*_test.py` |

### `watch` (dep-map triggers)

| Tool | Defaults |
|-----------|----------|
| testthat, tinytest, pointblank, validate | `R/**/*.R`, `R/**/*.r` |
| pytest, Great Expectations | `src/**/*.py`, `lib/**/*.py`, `**/*.py` |
| jarl, ruff | (empty: falls back to `run`) |

Watch globs are consulted by the file watcher and the dep-map staleness check. Editing a file matching a `watch` pattern triggers reruns of tests that depend on it (for runners) or re-checking that file (for linters).

## Config file

Scrutin looks for `.scrutin/config.toml` in the project root directory. If none is found, it falls back to `~/.config/scrutin/config.toml`. See the [configuration reference](reference/configuration.md) for the full schema.

## Virtual environment detection (Python)

For Python projects, scrutin resolves the interpreter per suite. Resolution uses the suite's root as the anchor, not the project root:

1. `[python].interpreter` in `.scrutin/config.toml` (project-wide override)
2. `[python].venv` in `.scrutin/config.toml` (path to a virtualenv directory, relative to project root or absolute)
3. `$VIRTUAL_ENV` environment variable
4. `.venv/` or `venv/` under the **suite root**
5. `$CONDA_PREFIX` environment variable
6. `python3` on `$PATH` (or `python` on Windows)

In a monorepo with `[[suite]] tool = "pytest" root = "python"`, step 4 finds `/repo/python/.venv/bin/python` — the venv that belongs to that package. If you have a single shared venv for the whole repo, set `[python].venv` explicitly.

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

**Monorepo (R + Python in sibling subdirs)**: explicit `[[suite]]` entries with `root` per suite. See the monorepo section above.

**Tests can't find their fixtures**: subprocess CWD is the suite's root, not the project root. A test doing `read.csv("data/foo.csv")` looks for `<suite.root>/data/foo.csv`. Either set `root` correctly or use absolute paths in the test.
