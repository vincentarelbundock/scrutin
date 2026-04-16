# RStudio

An RStudio add-in that launches the dashboard in the Viewer pane. File navigation uses a FIFO-based bridge that calls `rstudioapi::navigateToFile()`.

![RStudio](../assets/screenshot_rstudio_normal.png){ .screenshot }

## Installation

Install directly from GitHub:

```r
remotes::install_github("vincentarelbundock/scrutin/editors/rstudio")
```

Or clone the [GitHub repository](https://github.com/vincentarelbundock/scrutin) and build from source:

```bash
make rstudio    # R CMD INSTALL editors/rstudio
```

Requires: `jsonlite`, `later`, `processx`, `rstudioapi`.

Unlike the VS Code and Positron extensions, the RStudio add-in does **not** bundle the `scrutin` binary. Install it separately (see the [install instructions](../getting-started.md#install)) and either ensure it's on `$PATH` or point at it explicitly with `options(scrutin.binary = "/path/to/scrutin")`.

## Usage

```r
scrutin_start()           # start server + show dashboard
scrutin_start(watch = FALSE)
scrutin_stop()            # stop server + cleanup
scrutin_status()          # check if running, show URL + PID
scrutin_show()            # re-show the dashboard in Viewer
scrutin_init()            # scaffold .scrutin/config.toml in the project
```

These are also available from the **Addins** menu in RStudio.
