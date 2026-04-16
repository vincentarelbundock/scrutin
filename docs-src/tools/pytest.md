# pytest

The standard testing framework for Python. *Scrutin* auto-detects pytest when a `pyproject.toml` (or `setup.py`/`setup.cfg`) is present alongside a `tests/` or `test/` directory, or `test_*.py` files at the project root.

## Installing pytest

pytest is not shipped with *Scrutin*. Follow the upstream install instructions at [docs.pytest.org](https://docs.pytest.org/).

## Directory structure

```
myproject/
├── pyproject.toml
├── src/
│   └── myproject/
│       ├── __init__.py
│       └── math.py
└── tests/
    └── test_math.py
```

## Minimal example

**src/myproject/math.py**

```python
def add(x: int, y: int) -> int:
    return x + y
```

**tests/test_math.py**

```python
from myproject import add

def test_add():
    assert add(2, 3) == 5
    assert add(-1, 1) == 0
```

## Running

```bash
scrutin myproject              # TUI
scrutin -r plain myproject     # text output
```

## Configuration

No configuration is required. To override defaults in `.scrutin/config.toml`:

```toml
[[suite]]
tool = "pytest"
# defaults pick up tests/**/test_*.py and watch src/**/*.py + lib/**/*.py
```

## Virtual environment

*Scrutin* auto-detects your Python virtual environment per suite. Detection order:

1. `[python].interpreter` in `.scrutin/config.toml`
2. `[python].venv` in `.scrutin/config.toml`
3. `$VIRTUAL_ENV` environment variable
4. `.venv/` or `venv/` under the **suite root** (which equals the project root for single-package layouts; in a monorepo with `[[suite]] root = "python"`, this finds `python/.venv`)
5. `$CONDA_PREFIX` environment variable
6. `python3` on `$PATH` (or `python` on Windows)

Override in config:

```toml
[python]
venv = ".venv"
```

## Extra pytest flags

Pass arbitrary flags through to `pytest.main()` via `[pytest] extra_args`. Appended verbatim to every invocation, letting you reach for obscure pytest knobs without *Scrutin* growing a CLI option for each one:

```toml
[pytest]
extra_args = ["--tb=short", "-vv"]
```

## Custom runner

`scrutin init` writes the default runner to `.scrutin/runners/scrutin_pytest.py` (the `scrutin_` prefix avoids the runner shadowing `import pytest` from inside itself). Edit that file in place: the engine picks it up automatically whenever it exists. To point at a different path, set `runner` on an explicit suite:

```toml
[[suite]]
tool   = "pytest"
runner = "shared/pytest-runner.py"
```
