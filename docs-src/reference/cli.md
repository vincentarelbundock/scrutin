# Command-Line

This document contains the help content for the `scrutin` command-line program.

**Command Overview:**

* [`scrutin`↴](#scrutin)
* [`scrutin run`↴](#scrutin-run)
* [`scrutin init`↴](#scrutin-init)
* [`scrutin stats`↴](#scrutin-stats)

## `scrutin`

Fast watch-mode test runner

**Usage:** `scrutin [OPTIONS] [PATH]
       scrutin <COMMAND>`

###### **Subcommands:**

* `run` : Run tests (default)
* `init` : Initialize .scrutin/config.toml and .scrutin/ in the current package
* `stats` : Show flaky tests and slowness statistics from the local history DB

###### **Arguments:**

* `<PATH>` : Path to the project (default: current directory)

  Default value: `.`

###### **Options:**

* `-r`, `--reporter <NAME[:ARG]>` : Output reporter. Values: `tui`, `plain`, `github`, `web[:ADDR]`, `list`, `junit:PATH`. Defaults to `tui` when stderr is a tty, else `plain`
* `-s`, `--set <KEY=VALUE>` : Override a .scrutin/config.toml field. Repeatable. Dotted keys walk into nested tables (e.g. `run.workers=8`, `filter.include=["test_math*"]`, `watch.enabled=true`). RHS is parsed as a TOML expression, falling back to a bare string for unquoted values



## `scrutin run`

Run tests (default)

**Usage:** `scrutin run [OPTIONS] [PATH]`

###### **Arguments:**

* `<PATH>` : Path to the project (default: current directory)

  Default value: `.`

###### **Options:**

* `-r`, `--reporter <NAME[:ARG]>` : Output reporter. Values: `tui`, `plain`, `github`, `web[:ADDR]`, `list`, `junit:PATH`. Defaults to `tui` when stderr is a tty, else `plain`
* `-s`, `--set <KEY=VALUE>` : Override a .scrutin/config.toml field. Repeatable. Dotted keys walk into nested tables (e.g. `run.workers=8`, `filter.include=["test_math*"]`, `watch.enabled=true`). RHS is parsed as a TOML expression, falling back to a bare string for unquoted values



## `scrutin init`

Initialize .scrutin/config.toml and .scrutin/ in the current package

**Usage:** `scrutin init [PATH]`

###### **Arguments:**

* `<PATH>` : Path to the project (default: current directory)

  Default value: `.`



## `scrutin stats`

Show flaky tests and slowness statistics from the local history DB

**Usage:** `scrutin stats [PATH]`

###### **Arguments:**

* `<PATH>` : Path to the project (default: current directory)

  Default value: `.`



<hr/>

<small><i>
    This document was generated automatically by
    <a href="https://crates.io/crates/clap-markdown"><code>clap-markdown</code></a>.
</i></small>
