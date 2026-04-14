# Keybindings

The TUI and web share a single keybinding table, defined in `scrutin_core::keymap::DEFAULT_KEYMAP`. Press `?` in the TUI (or click the help button in the web) to see the same table rendered live.

## Navigation

| Key | Action |
|-----|--------|
| `j` / `k` or `Down` / `Up` | Move cursor down / up |
| `g` / `G` or `Home` / `End` | Top / bottom |
| `PageDown` / `PageUp` or `Ctrl-f` / `Ctrl-b` | Page down / up |
| `J` / `K` | Scroll the source pane |
| `Enter` / `l` / `Right` | Drill in |
| `Esc` / `h` / `q` / `Left` | Pop one level |

## Run control

| Key | Action |
|-----|--------|
| `r` | At Files: open run palette. At Detail/Failure: re-run current file |
| `R` | At Detail/Failure: open run palette |
| `x` | Cancel current file |
| `X` | Cancel entire run |

## Filtering and display

| Key | Action |
|-----|--------|
| `/` | Filter palette |
| `s` | Sort palette (sequential, status, name, suite, time) |
| `t` / `T` | Cycle status filter |
| `p` / `P` | Cycle suite filter |
| `Space` | Toggle selection on current file |
| `v` | Visual select mode |
| `-` | Toggle vertical / horizontal split |
| `(` / `)` | Shrink / grow list pane |

## Other

| Key | Action |
|-----|--------|
| `a` | Plugin action palette (jarl fix, ruff fix, ...) |
| `e` | Open test file in `$EDITOR` |
| `E` | Open source file in `$EDITOR` |
| `y` | Yank failure message to clipboard |
| `L` | Log overlay |
| `?` | Help overlay |

## Custom keybindings

Override any binding in `scrutin.toml`. Each `[keymap.<mode>]` table fully replaces the defaults for that mode (replace, not overlay): deleting a line unbinds the key, deleting the whole subtable restores the built-in defaults.

```toml
[keymap.normal]
j     = "cursor_down"
k     = "cursor_up"
Enter = "enter"
```

Mode names: `normal`, `detail`, `failure`, `help`, `log`. Action names are enumerated in `scrutin_core::keymap::all_action_names()` and the full default map is written out by `scrutin init` so you can edit in place.
