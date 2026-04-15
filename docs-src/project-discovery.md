# Projects and Files

scrutin runs in one of two modes:

1. **File mode**: point a single tool at one or more files. No project, no config, no auto-detection.
2. **Project mode** (the default): run every applicable tool against a full package, loading your code into R or Python so tests can exercise it.

File mode is covered first because it's the simplest.

## File mode

File mode checks individual files with a single tool. No config, no package, no auto-detection.

```bash
scrutin README.md NEWS.md --tool skyspell
scrutin src/foo.py --tool ruff
scrutin . --tool typos
```

Available tools:

| Tool | Language | What it does |
|------|----------|--------------|
| jarl | R | R linter |
| ruff | Python | Fast Python linter / formatter |
| skyspell | Prose | Dictionary-based spell checker |
| typos | Any | Curated-misspelling spell checker (language-agnostic) |

These tools invoke a CLI once per file and don't need a surrounding project. Test and data-validation tools (testthat, tinytest, pointblank, validate, pytest, Great Expectations) do need a project: they load your package into a long-lived R or Python interpreter so the tests can exercise it, and they're refused in file mode with a pointer to run from the project root instead.

File mode differs from project mode:

- You must pass `--tool <name>`; there's nothing to auto-detect.
- The project-local `.scrutin/config.toml` isn't loaded (there's no project). A user-level config (see [Config file lookup](#config-file-lookup)) still applies, so your global preferences carry over; use `--set` overrides for one-off tweaks on top.
- Run state (history DB, dep-map cache) lives in a scratch directory, so nothing lands next to your files.
- Watch mode still applies if your terminal supports the TUI: edits to the listed files trigger reruns.

## Project mode

scrutin runs from the project directory and activates every applicable tool against the whole package. Works for **testthat**, **tinytest**, **pointblank**, **validate** (R), and **pytest**, **Great Expectations** (Python) with zero configuration; **jarl**, **ruff**, **skyspell**, **typos** join in when you opt them in explicitly (see [Example projects](#example-projects) below).

### Concepts

**Project root**: the directory you point scrutin at on the command line. This is where `.scrutin/config.toml` lives (if any), and where the run's shared state is anchored: `.scrutin/state.db`, runner scripts, hooks, and ignore patterns.

**Tool**: a test runner or quality checker scrutin knows how to drive (testthat, pytest, jarl, ruff, skyspell, ...). Support for every supported tool is built into the scrutin binary; there are no separate scrutin plugins to install. You still install the underlying tools (R, Python, pytest, ruff, ...) through their usual channels.

**Suite**: one configured instance of a tool. A suite owns a working directory and a set of file patterns. A project can run many suites in one invocation, even several using the same tool on different subtrees; suites run one after another so each gets the full worker pool, and files within a suite run in parallel.

**Suite root**: the working directory a suite's subprocess runs from. Subprocesses are spawned with `cwd = suite.root` and `SCRUTIN_PKG_DIR = suite.root`, so `pkgload::load_all()`, `pytest`, `ruff`, and your tests' relative `read.csv(...)` / `open(...)` calls all resolve against that subtree. In a single-package project, suite root equals project root. In a monorepo, each suite points at its own package.

### Pointing at a project

```bash
scrutin                   # project root is the current directory
scrutin path/to/project   # project root is the given path
```

scrutin does **not** walk upward to find a parent project. Running it from inside `tests/testthat/` looks for tools in `tests/testthat/`. Run from the project root or pass the path explicitly.

### Auto-detection

With no `[[suite]]` entries in your config, scrutin scans the project root for marker files and activates every matching test or data-validation tool. A project with both `tests/testthat/` and `inst/tinytest/` gets both suites; adding `pyproject.toml` with a `tests/` directory adds pytest alongside them. Every auto-detected suite gets `root` equal to the project root.

| Tool | Language | Detected when |
|------|----------|---------------|
| testthat | R | `DESCRIPTION` + `tests/testthat/` |
| tinytest | R | `DESCRIPTION` + `inst/tinytest/` |
| pointblank | R | `DESCRIPTION` + `tests/pointblank/` |
| validate | R | `DESCRIPTION` + `tests/validate/` |
| pytest | Python | `pyproject.toml` (or `setup.py` / `setup.cfg`) + `tests/` or `test/`, or `test_*.py` at the root |
| Great Expectations | Python | `tests/great_expectations/` |

Linters and spell checkers (**jarl**, **ruff**, **skyspell**, **typos**) never auto-detect, even when their config files are present. They're orthogonal to testing: your editor probably already runs ruff on save, and spell checking is rarely something you want gating a CI test run. Enable them with an explicit `[[suite]]` entry when you want scrutin to orchestrate them alongside your tests.

Auto-detection only scans the project root: subdirectory names are arbitrary, so scrutin does not guess. To narrow to a single tool, use `--tool` (short form `-t`):

```bash
scrutin --tool pytest
```

`--tool` is sugar for `--set run.tool=<name>`, which applies to both auto-detected and explicitly-declared suites.

### Suite config

When auto-detection doesn't fit (monorepo, non-standard test layout, scattered files, opt-in linters / spell checkers), declare `[[suite]]` entries in `.scrutin/config.toml`. As soon as one `[[suite]]` is present, auto-detection is skipped entirely.

A suite with every field set:

```toml
[[suite]]
tool   = "pytest"                     # which plugin to use
root   = "backend/"                   # suite working directory (relative to project root)
run    = ["tests/**/test_*.py"]       # files the tool operates on (relative to root)
watch  = ["marginaleffects/**/*.py"]  # files whose edits trigger reruns of dependent tests
runner = ".scrutin/pytest/my.py"      # custom runner script (relative to project root)
```

Only `tool` is required; every other field falls back to a plugin-specific default. The table below documents the fields; the subsection after it covers the relative / absolute path rules the example skirted.

| Field | Type | Required | Description |
|-------|------|----------|-------------|
| `tool` | string | yes | Tool name from the detection table above. |
| `root` | string | no | Suite root. Default: `.`. |
| `run` | glob list | no | Files the tool operates on. Default: plugin-provided (see [Configuration](reference/configuration.md)). |
| `watch` | glob list | no | Files that trigger reruns. Default: plugin-provided (or same as `run` for linters). |
| `runner` | string | no | Path to a custom runner script that replaces the built-in default. |

Unknown keys are a hard error, so typos surface immediately.

### Paths: relative vs absolute

Every path-valued field (`root`, `run`, `watch`, `runner`) accepts both forms:

- **Relative** paths are resolved against the **project root** for `root` and `runner`, and against the **suite root** for `run` and `watch` globs. This is the common case and keeps configs portable across checkouts.
- **Absolute** paths are used verbatim. Useful for sketch directories outside the repo, shared fixture trees, or a runner that lives in `~/.scrutin/`.

```toml
# Relative: project-root-anchored
[[suite]]
tool   = "pytest"
root   = "backend/"               # project-root + "backend/"
run    = ["tests/**/test_*.py"]   # suite-root + "tests/..."
runner = ".scrutin/pytest/my.py"  # project-root + ".scrutin/..."

# Absolute: taken verbatim
[[suite]]
tool = "testthat"
root = "/home/me/scratch/sketch"
run  = ["/srv/shared-fixtures/foo_test.R"]
```

Globs under a relative `root` are still anchored under that root, not under the project root: a `run = ["tests/**/*.py"]` in a suite with `root = "backend/"` matches `<project_root>/backend/tests/**/*.py`. There's no directory-recursion shortcut: `run = ["tests"]` matches the literal path `tests`, not files under it. Write `tests/**/*.py` for recursion.

### Example projects

#### Tests and quality checks

A single R package where tests auto-detect fine, but you also want jarl linting and skyspell on the docs. As soon as you declare *any* `[[suite]]`, auto-detection is off, so testthat has to be re-declared explicitly even though it would have been free:

```toml
[[suite]]
tool = "testthat"     # would auto-detect; re-declared because any [[suite]] disables auto-detection

[[suite]]
tool = "jarl"         # opt-in, never auto-detects

[[suite]]
tool = "skyspell"
run  = ["README.md", "NEWS.md", "man/**/*.Rd"]
```

All three suites run with `cwd` at the project root. testthat uses its plugin-default `run` globs; skyspell's are overridden to cover the package's prose files.

#### Multi-lingual and multi-tool

R package in `r_dir/`, Python package in `python_dir/`, plus a project-wide skyspell suite spell-checking the docs:

```toml
[[suite]]
tool = "testthat"
root = "r_dir"

[[suite]]
tool = "tinytest"
root = "r_dir"

[[suite]]
tool = "pytest"
root = "python_dir"

[[suite]]
tool = "ruff"
root = "python_dir"

[[suite]]
tool = "skyspell"
run  = ["README.md", "NEWS.md", "docs/**/*.md"]
```

Each subprocess runs with `cwd` set to its suite's `root`: the testthat and tinytest workers run inside `r_dir/`, the pytest and ruff workers run inside `python_dir/`, the skyspell worker runs at the project root (no `root` declared, so `root = "."`). Relative file I/O inside tests (`read.csv("data/foo.csv")`, `open("tests/fixtures/x.txt")`) resolves against the suite's subtree, not against the project root.

If each package has its own virtualenv, step 4 of [Python virtual environments](#python-virtual-environments) picks up `python_dir/.venv/` for the pytest suite automatically.

#### Non-standard layout

Tests live outside the conventional `tests/` directory, with custom dep-map globs so editing a source file triggers the right test files:

```toml
[[suite]]
tool  = "pytest"
root  = "backend/"
run   = ["spec/**/check_*.py", "integration/**/it_*.py"]
watch = ["marginaleffects/**/*.py", "spec/**/*.py"]

[env]
PYTHONPATH = "backend/src"
```

`run` selects both the regular spec files and a separate integration-test tree; `watch` lists the source files whose edits should trigger reruns. The `[env]` block is not suite-specific (it's global) but shows up here because it's often what a non-standard layout needs to make imports resolve.

#### Scattered files

A bag of ad-hoc R test scripts that don't live in a package. The `root` is absolute because this directory isn't inside any project you've checked out; `run` mixes a local file with one reached via `..`:

```toml
[[suite]]
tool = "testthat"
root = "/home/me/sketch"
run  = ["foo_test.R", "../misc/bar_test.R"]
```

No shared-ancestor requirement: routing is per-glob match, so a suite can pull files from anywhere on disk.

#### Custom runner script

Swap the default testthat runner for one that `library()`s your installed package instead of `pkgload::load_all()` on source:

```toml
[[suite]]
tool   = "testthat"
runner = ".scrutin/testthat/installed.R"
```

`scrutin init` writes the default runner out to `.scrutin/testthat/runner.R` so you can copy, edit, and point at the edited version. Same pattern works for pytest (`runner = ".scrutin/pytest/my_runner.py"`) and every other worker-mode tool.

### Python virtual environments

For Python suites, scrutin resolves the interpreter anchored at the **suite root**, not the project root. Resolution order:

1. `[python].interpreter` in `.scrutin/config.toml` (project-wide override)
2. `[python].venv` (path to a virtualenv, relative to project root or absolute)
3. `$VIRTUAL_ENV`
4. `.venv/` or `venv/` under the suite root
5. `$CONDA_PREFIX`
6. `python3` on `$PATH` (or `python` on Windows)

In a monorepo, step 4 picks up each package's own `.venv`. For a single shared venv, set `[python].venv` explicitly:

```toml
[python]
venv = "my_env"             # relative to project root, or absolute
interpreter = "python3.12"  # skip venv detection entirely
```

### Config file lookup

scrutin looks for `.scrutin/config.toml` in the project root. If none is found, it falls back to a user-level config in the platform-standard config directory:

| Platform | Path |
|----------|------|
| Linux | `~/.config/scrutin/config.toml` (or `$XDG_CONFIG_HOME/scrutin/config.toml`) |
| macOS | `~/Library/Application Support/scrutin/config.toml` |
| Windows | `%APPDATA%\scrutin\config.toml` |

The same fallback applies in file mode, so your global preferences (language for skyspell, extra args for a linter, etc.) carry over even without a project. See the [configuration reference](reference/configuration.md) for the full schema, including per-tool defaults and options not covered here.
