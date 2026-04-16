# Installation

!!! warning "Alpha software"
    *Scrutin* is alpha software under active development. Expect bugs, breaking changes, and rough edges. Please report issues at [github.com/vincentarelbundock/scrutin](https://github.com/vincentarelbundock/scrutin).

## The scrutin binary

Prebuilt binaries are published with each [release](https://github.com/vincentarelbundock/scrutin/releases).

=== "macOS / Linux"
    ```bash
    curl --proto '=https' --tlsv1.2 -LsSf \
      https://github.com/vincentarelbundock/scrutin/releases/latest/download/scrutin-installer.sh | sh
    ```

=== "Windows"
    ```powershell
    powershell -ExecutionPolicy Bypass -c \
      "irm https://github.com/vincentarelbundock/scrutin/releases/latest/download/scrutin-installer.ps1 | iex"
    ```

=== "From source"
    ```bash
    cargo install scrutin
    ```

Once the binary is on `PATH`, run `scrutin --version` to confirm the install.

## External tools

*Scrutin* orchestrates third-party tools but does not ship them. You only need to install the ones your project uses; each tool's docs page covers what *Scrutin* does with it.

### Test frameworks

Installed as importable packages in the environment *Scrutin* uses for that suite (the active R library, or the suite's resolved Python virtualenv).

| Tool | Language | Install | Docs |
|------|----------|---------|------|
| [testthat](https://testthat.r-lib.org/) | R | `install.packages("testthat")` | [testthat page](tools/testthat.md) |
| [tinytest](https://github.com/markvanderloo/tinytest) | R | `install.packages("tinytest")` | [tinytest page](tools/tinytest.md) |
| [pytest](https://docs.pytest.org/) | Python | `uv add --dev pytest` or `pip install pytest` | [pytest page](tools/pytest.md) |

### Data validation

Same story: importable packages loaded by the runner subprocess.

| Tool | Language | Install | Docs |
|------|----------|---------|------|
| [pointblank](https://rstudio.github.io/pointblank/) | R | `install.packages("pointblank")` | [pointblank page](tools/pointblank.md) |
| [validate](https://github.com/data-cleaning/validate) | R | `install.packages("validate")` | [validate page](tools/validate.md) |
| [Great Expectations](https://greatexpectations.io/) | Python | `uv add great_expectations` | [Great Expectations page](tools/great-expectations.md) |

### Linters and spell checkers

Standalone binaries. Put them on `PATH`.

| Tool | Purpose | Install | Docs |
|------|---------|---------|------|
| [jarl](https://jarl.etiennebacher.com/) | R linter | `install.packages("jarl")` then `jarl::jarl_install()` | [jarl page](tools/jarl.md) |
| [ruff](https://docs.astral.sh/ruff/) | Python linter / formatter | `uv tool install ruff` or `pipx install ruff` | [ruff page](tools/ruff.md) |
| [skyspell](https://codeberg.org/your-tools/skyspell) | Dictionary-based spell checker | See note below | [skyspell page](tools/skyspell.md) |
| [typos](https://github.com/crate-ci/typos) | Curated-misspelling spell checker | `cargo install typos-cli` | [typos page](tools/typos.md) |

!!! info "skyspell needs enchant-2"
    skyspell links against the Enchant spell-checking library, so `cargo install skyspell` fails with a `pkg-config` error unless the development headers are on the system first.

    === "Debian / Ubuntu"
        ```bash
        sudo apt install libenchant-2-dev pkg-config
        cargo install skyspell
        ```

    === "Fedora / RHEL"
        ```bash
        sudo dnf install enchant2-devel pkgconf-pkg-config
        cargo install skyspell
        ```

    === "macOS"
        ```bash
        brew install enchant pkg-config
        cargo install skyspell
        ```

    === "Arch"
        ```bash
        sudo pacman -S enchant pkgconf
        cargo install skyspell
        ```

Test frameworks and data-validation tools auto-detect from marker files (`DESCRIPTION`, `pyproject.toml`, `tests/testthat/`, ...) and activate automatically. Linters and spell checkers are opt-in: add a one-line `[[suite]]` entry in `.scrutin/config.toml`, covered in [Getting Started](getting-started.md).

At startup *Scrutin* checks that every required binary is reachable and refuses to run a suite whose binary is missing, with a pointer to the tool's homepage. Set `[preflight] command_tools = false` to bypass the check.

## Editor extensions (optional)

If you want *Scrutin* inside your editor, install the relevant extension:

- [VS Code](frontends/vscode.md): Marketplace or Open VSX
- [Positron](frontends/positron.md): same `.vsix` as VS Code
- [RStudio](frontends/rstudio.md): `R CMD INSTALL` the addin

The extensions shell out to the `scrutin` binary, so they pick up whatever you installed above.
