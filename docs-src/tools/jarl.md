# jarl

An R linter that scrutin can drive as a plugin. Unlike test tools, jarl checks code style rather than correctness. It maps lint diagnostics to `warn` events, so lint issues appear alongside test results in the TUI and web dashboard. jarl is opt-in: enable it with an explicit `[[suite]] tool = "jarl"` entry in `.scrutin/config.toml`, or pass files on the command line with `-t jarl` in [file mode](../project-discovery.md#file-mode).

## Directory structure

```
mypackage/
├── .scrutin/
│   └── config.toml    # [[suite]] tool = "jarl"
├── DESCRIPTION
├── jarl.toml          # optional: jarl's own config (rules, per-check knobs)
└── R/
    ├── math.R
    └── strings.R
```

## Minimal example

**.scrutin/config.toml**

```toml
[[suite]]
tool = "jarl"
```

**jarl.toml** (optional)

`jarl.toml` is read by jarl itself (not by scrutin) if you want to tune rules. Omit it to use jarl's built-in defaults.

**R/math.R**

```r
# jarl flags T/F instead of TRUE/FALSE
is_positive <- function(x) {
  if (x > 0) T else F
}
```

## Running

```bash
scrutin mypackage              # TUI
scrutin -r plain mypackage     # text output
```

jarl runs as its own suite alongside any other suites you've declared; suites run one at a time, but within the jarl suite every matched file is linted in parallel.

## Plugin actions

Enter the Detail view for a jarl warning to see a numbered chip row of fix actions. Press the digit to invoke:

| Key | Action |
|-----|--------|
| `1` | Jarl: fix (this file) |
| `2` | Jarl: fix (this file, unsafe) |
| `3` | Jarl: fix all (suite) |
| `4` | Jarl: fix all (suite, unsafe) |

All four invoke `jarl` once with every matching file (after include / exclude filters) as trailing arguments. After a fix, the affected files are re-linted automatically.

## Configuration

The minimal suite entry is just `tool = "jarl"`. Override defaults on the same block:

```toml
[[suite]]
tool = "jarl"
# default `run` lints R/**/*.R; `watch` defaults to `run` (linters re-check what they operate on).
# Override to lint a different tree:
# run = ["scripts/**/*.R", "inst/examples/**/*.R"]
```

jarl has no separate source/watch list (it lints files directly and does not track dependencies between them).
