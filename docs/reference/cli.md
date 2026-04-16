# Command-Line

This document contains the help content for the `scrutin` command-line program.

**Command Overview:**

* [`scrutin`↴](#scrutin)
* [`scrutin run`↴](#scrutin-run)
* [`scrutin init`↴](#scrutin-init)
* [`scrutin init skill`↴](#scrutin-init-skill)
* [`scrutin stats`↴](#scrutin-stats)

## `scrutin`

Fast watch-mode test runner

**Usage:** `scrutin [OPTIONS] [PATHS]...
       scrutin <COMMAND>`

###### **Subcommands:**

* `run` : Run tests (default)
* `init` : Initialize scaffolding. Default: `.scrutin/config.toml` and runner scripts in the current package. `init skill` installs the Agent Skill for Claude Code / Codex instead
* `stats` : Show flaky tests and slowness statistics from the local history DB

###### **Arguments:**

* `<PATHS>` : Path(s) to the project or to individual files. A single directory is the project root (default: `.`). One or more file paths activates file-mode: scrutin runs the tool named by `--tool` on just those files, with no project context. Mixing files and directories is an error

###### **Options:**

* `-t`, `--tool <NAME>` : Tool to run in file-mode. Sugar for `--set run.tool=<name>`. Required when `paths` contains files instead of a directory. Must name a command-mode plugin (skyspell, jarl, ruff); worker-mode plugins (pytest, testthat, ...) need a project root
* `-r`, `--reporter <NAME[:ARG]>` : Output reporter. Values: `tui`, `plain`, `github`, `web[:ADDR]`, `list`, `junit:PATH`. Defaults to `tui` when stderr is a tty, else `plain`. File-mode defaults to `plain` regardless
* `-s`, `--set <KEY=VALUE>` : Override a .scrutin/config.toml field. Repeatable. Dotted keys walk into nested tables (e.g. `run.workers=8`, `filter.include=["test_math*"]`, `watch.enabled=true`, `filter.group=fast`). RHS is parsed as a TOML expression, falling back to a bare string for unquoted values



## `scrutin run`

Run tests (default)

**Usage:** `scrutin run [OPTIONS] [PATHS]...`

###### **Arguments:**

* `<PATHS>` : Path(s) to the project or to individual files. A single directory is the project root (default: `.`). One or more file paths activates file-mode: scrutin runs the tool named by `--tool` on just those files, with no project context. Mixing files and directories is an error

###### **Options:**

* `-t`, `--tool <NAME>` : Tool to run in file-mode. Sugar for `--set run.tool=<name>`. Required when `paths` contains files instead of a directory. Must name a command-mode plugin (skyspell, jarl, ruff); worker-mode plugins (pytest, testthat, ...) need a project root
* `-r`, `--reporter <NAME[:ARG]>` : Output reporter. Values: `tui`, `plain`, `github`, `web[:ADDR]`, `list`, `junit:PATH`. Defaults to `tui` when stderr is a tty, else `plain`. File-mode defaults to `plain` regardless
* `-s`, `--set <KEY=VALUE>` : Override a .scrutin/config.toml field. Repeatable. Dotted keys walk into nested tables (e.g. `run.workers=8`, `filter.include=["test_math*"]`, `watch.enabled=true`, `filter.group=fast`). RHS is parsed as a TOML expression, falling back to a bare string for unquoted values



## `scrutin init`

Initialize scaffolding. Default: `.scrutin/config.toml` and runner scripts in the current package. `init skill` installs the Agent Skill for Claude Code / Codex instead

**Usage:** `scrutin init [PATH]
       init <COMMAND>`

###### **Subcommands:**

* `skill` : Install the scrutin Agent Skill for Claude Code, Codex, or any other agent that loads `~/.claude/skills/<name>/SKILL.md`

###### **Arguments:**

* `<PATH>` : Path to the project (default: current directory). Used when no subcommand is given

  Default value: `.`



## `scrutin init skill`

Install the scrutin Agent Skill for Claude Code, Codex, or any other agent that loads `~/.claude/skills/<name>/SKILL.md`.

Default destination: `~/.claude/skills/scrutin/`. Pass a directory to override, or `-` to write the skill to stdout instead of a file.

**Usage:** `scrutin init skill [OPTIONS] [PATH]`

###### **Arguments:**

* `<PATH>` : Destination directory, or `-` for stdout

###### **Options:**

* `--force` : Overwrite an existing `SKILL.md` at the destination



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
