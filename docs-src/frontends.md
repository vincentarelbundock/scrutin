# Frontends

Frontends are the interactive ways you watch a test run unfold: a terminal UI, a browser dashboard, and the editor integrations that embed that dashboard inside VS Code, Positron, and RStudio. All of them consume the same event stream from the engine and stream results in live, so you can browse and drill into failures while tests are still running.

Pick one with `-r` / `--reporter`:

```bash
scrutin                          # TUI (default when the terminal is a tty)
scrutin -r web                   # browser dashboard
scrutin -r web:0.0.0.0:3000      # web on a custom address
```

When no reporter is given, scrutin defaults to `tui` on a tty and `plain` otherwise. The VS Code, Positron, and RStudio integrations are thin wrappers: each one spawns `scrutin -r web` in the background and embeds the resulting page inside the editor.

For non-interactive outputs (plain, JUnit, GitHub Actions, list), see [Reporters](reporters.md).

## Terminal UI

The default frontend. A two-pane, vim-style interface built with ratatui. It launches automatically when your terminal supports it.

The left pane shows your test files. The right pane previews results for the highlighted file. Press `j`/`k` to navigate, `Enter` to drill into a file's test results, and `Esc` to go back.

In detail mode, the left pane shows individual tests within the file, and the right pane shows the failure message and source context for the highlighted test. Press `Enter` on a failing test to see the full error with source code from both the test file and the source function it exercises.

Results stream in live. Press `?` for a help overlay, `/` to filter, `s` to open the sort palette. Watch mode is on by default; disable it with `--set watch.enabled=false`. See [Keybindings](keybindings.md) for the full reference.

## Web dashboard

A browser-based dashboard with live updates. The frontend is embedded in the binary: no Node.js or build step required.

```bash
scrutin -r web                   # binds to 127.0.0.1:7878
scrutin -r web:0.0.0.0:3000      # custom address
```

The dashboard uses server-sent events to stream results as they arrive. It binds to localhost only by default. If the port is busy, scrutin tries the next one automatically.

## VS Code

A TypeScript extension that embeds the scrutin web dashboard in an editor panel and surfaces live pass/fail/error counts in the status bar via SSE.

### Installation

```bash
make vscode     # build + install into VS Code
```

The `scrutin` binary must be on `$PATH`, or set `scrutin.binaryPath` in settings.

The extension activates automatically when it detects `.scrutin/config.toml`, `DESCRIPTION`, or `pyproject.toml` in the workspace.

### Commands

The extension only exposes lifecycle commands. Run / rerun-failing / cancel / toggle-watch are reachable as chip buttons and keyboard shortcuts inside the webview.

| Command | Description |
|---------|-------------|
| `scrutin.start` | Start the scrutin server |
| `scrutin.stop` | Stop the server |
| `scrutin.showPanel` | Show/focus the dashboard panel |

### Settings

| Setting | Default | Description |
|---------|---------|-------------|
| `scrutin.binaryPath` | `""` | Absolute path to the scrutin binary. Leave empty to find it on `$PATH`. |
| `scrutin.autoStart` | `false` | Start the server automatically when the extension activates |

Watch mode and every other scrutin knob are controlled by `.scrutin/config.toml` (per-project) or `~/.config/scrutin/config.toml` (user-level). The extension doesn't override them.

## Positron

The same extension as [VS Code](#vs-code); commands and settings listed above apply unchanged. Install with:

```bash
make positron
```

## RStudio

An RStudio add-in that launches the dashboard in the Viewer pane. File navigation uses a FIFO-based bridge that calls `rstudioapi::navigateToFile()`.

### Installation

Install directly from GitHub:

```r
remotes::install_github("vincentarelbundock/scrutin/editors/rstudio")
```

Or build from a local checkout:

```bash
make rstudio    # R CMD INSTALL editors/rstudio
```

Requires: `jsonlite`, `later`, `processx`, `rstudioapi`.

Set the binary path with `options(scrutin.binary = "/path/to/scrutin")`, or ensure `scrutin` is on `$PATH`.

### Usage

```r
scrutin_start()           # start server + show dashboard
scrutin_start(watch = FALSE)
scrutin_stop()            # stop server + cleanup
scrutin_status()          # check if running, show URL + PID
scrutin_show()            # re-show the dashboard in Viewer
```

These are also available from the **Addins** menu in RStudio.
