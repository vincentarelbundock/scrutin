# Editor Integrations

Both integrations work the same way under the hood: they spawn `scrutin -r web` as a background process and display the browser dashboard inside the editor. File clicks in the dashboard navigate to the source in your IDE.

## VS Code / Positron

The VS Code extension embeds the scrutin web dashboard in an editor panel with real-time status bar updates via SSE.

### Installation

```bash
make vscode     # build + install into VS Code
make positron   # build + install into Positron
```

The `scrutin` binary must be on `$PATH`, or set `scrutin.binaryPath` in VS Code settings.

The extension activates automatically when it detects `scrutin.toml`, `DESCRIPTION`, or `pyproject.toml` in the workspace.

### Commands

| Command | Description |
|---------|-------------|
| `scrutin.start` | Start the scrutin server |
| `scrutin.stop` | Stop the server |
| `scrutin.showPanel` | Show/focus the dashboard panel |
| `scrutin.runAll` | Trigger a full test run |
| `scrutin.rerunFailing` | Re-run only failing tests |
| `scrutin.cancel` | Cancel the current run |
| `scrutin.toggleWatch` | Toggle watch mode |

### Settings

| Setting | Default | Description |
|---------|---------|-------------|
| `scrutin.binaryPath` | `""` | Absolute path to the scrutin binary. Leave empty to find it on `$PATH`. |
| `scrutin.watchOnStart` | `true` | Enable watch mode on startup |
| `scrutin.autoStart` | `false` | Start the server automatically when the extension activates |

The status bar shows live pass/fail/error counts during a run.

## RStudio

An RStudio add-in that launches the dashboard in the Viewer pane. File navigation uses a FIFO-based bridge that calls `rstudioapi::navigateToFile()`.

### Installation

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
