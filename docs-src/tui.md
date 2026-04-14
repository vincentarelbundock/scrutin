# Terminal UI

The default frontend. Scrutin launches the TUI automatically when stderr is a tty. It uses vim-style modal navigation: if you know `j` / `k` / `Enter` / `Esc`, you already know how to use it.

## Navigating

The screen is split into two panes. At the **Files** level (the default), the left pane shows your test files and the right pane previews the results for the highlighted file. Press `j` / `k` to move through the list, and `Enter` (or `l`) to drill in.

At the **Detail** level, the left pane shows individual tests within the file and the right pane shows the failure message and source context for the highlighted test. Press `Enter` on a failing test to drill into the **Failure** level with full error and source.

Press `Esc` (or `h` / `q`) to pop back one level. `?` opens a help overlay with the full, live-generated keybinding reference.

## Levels and overlays

The TUI separates drill **level** (Files / Detail / Failure) from transient **overlays** (Help, Log, palettes). Overlays sit on top of a level without changing it; `Esc` dismisses the overlay first, then pops the level.

| | Entry | Exit | Purpose |
|--|--|--|--|
| Files level | default | `q` | Navigate file list, trigger runs |
| Detail level | `Enter` on a file | `Esc` / `h` / `q` | Test results within a file |
| Failure level | `Enter` on a failed test | `Esc` / `h` / `q` | Full failure message + source |
| Filter palette | `/` | `Esc` / `Enter` | Live filter by basename |
| Run palette | `r` at Files level, `R` at Detail/Failure | `Esc` / `Enter` | Run all / failing / selection |
| Sort palette | `s` | `Esc` / `Enter` | Pick sort mode |
| Action palette | `a` | `Esc` / `Enter` | Plugin actions (jarl fix, ruff fix, ...) |
| Help overlay | `?` | `Esc` / `q` / `?` | Keybinding reference |
| Log overlay | `L` | `Esc` / `q` | Subprocess stderr and internal messages |

## Running tests

At the Files level, `r` opens the run palette (run all, run failing, run selection, run current). In Detail / Failure, `r` immediately re-runs the current file and `R` opens the run palette. `x` cancels the current file; `X` cancels the entire run.

During a run, results stream in live: you can browse files and drill into failures while other tests are still running.

## Filtering and sorting

`/` opens the filter palette. The file list narrows as you type. `Enter` confirms, `Esc` cancels.

`s` opens the sort palette. Modes: **sequential** (emission order), **status**, **name**, **suite**, **time**. The selection persists across runs. `t` / `T` cycles the status filter (all → failures → errors → passes → ...); `p` / `P` cycles the suite filter when a project has more than one tool.

## Keybindings

These are the defaults, compiled into `scrutin_core::keymap::DEFAULT_KEYMAP` and shared across frontends. `?` in the TUI renders the same table live.

### Navigation

| Key | Action |
|-----|--------|
| `j` / `k` or `Down` / `Up` | Move cursor down / up |
| `g` / `G` or `Home` / `End` | Jump to top / bottom |
| `PageDown` / `PageUp` or `Ctrl-f` / `Ctrl-b` | Page down / up |
| `J` / `K` | Scroll source pane down / up |
| `Enter` / `l` / `Right` | Drill in |
| `Esc` / `h` / `q` / `Left` | Back / pop level |

### Run control

| Key | Action |
|-----|--------|
| `r` | At Files: open run palette. At Detail/Failure: re-run current file |
| `R` | At Detail/Failure: open run palette |
| `x` | Cancel current file |
| `X` | Cancel entire run |

### Filtering and display

| Key | Action |
|-----|--------|
| `/` | Filter palette |
| `s` | Sort palette |
| `t` / `T` | Cycle status filter forward / back |
| `p` / `P` | Cycle suite filter forward / back |
| `Space` | Toggle selection on current file |
| `v` | Visual select mode |
| `-` | Toggle vertical / horizontal split |
| `(` / `)` | Shrink / grow the list pane |

### Other

| Key | Action |
|-----|--------|
| `a` | Plugin action palette (jarl fix, ruff fix, ...) |
| `e` | Open test file in `$EDITOR` |
| `E` | Open source file in `$EDITOR` |
| `y` | Yank failure message to clipboard |
| `L` | Log overlay |
| `?` | Help overlay |

## Custom keybindings

You can override any keybinding in `.scrutin/config.toml`. Each `[keymap.<mode>]` table fully **replaces** the defaults for that mode : deleting a line unbinds the key, deleting the whole subtable restores the built-in defaults. `scrutin init` writes the defaults out in full so you can edit in place.

```toml
[keymap.normal]
j     = "cursor_down"
k     = "cursor_up"
Enter = "enter"
```

Mode names: `normal`, `detail`, `failure`, `help`, `log`. Action names are enumerated in `scrutin_core::keymap::all_action_names()`.
