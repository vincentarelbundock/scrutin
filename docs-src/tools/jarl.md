# jarl

An R linter that scrutin can drive as a plugin. Unlike test tools, jarl checks code style rather than correctness. It maps lint diagnostics to `warn` events, so lint issues appear alongside test results in the TUI and web dashboard. Scrutin auto-detects jarl when a `jarl.toml` file, a `DESCRIPTION` file, and an `R/` directory are all present.

## Directory structure

```
mypackage/
├── DESCRIPTION
├── jarl.toml
└── R/
    ├── math.R
    └── strings.R
```

## Minimal example

**jarl.toml**

An empty file is enough to opt in. jarl uses its built-in rule set by default.

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

jarl runs concurrently alongside any test tools in the same project.

## Plugin actions

Enter the Detail view for a jarl warning to see a numbered chip row of fix actions. Press the digit to invoke:

| Key | Action |
|-----|--------|
| `1` | Jarl: fix (this file) |
| `2` | Jarl: fix (this file, unsafe) |
| `3` | Jarl: fix all (suite) |
| `4` | Jarl: fix all (suite, unsafe) |

Both invoke `jarl` once with every matching file (after include / exclude filters) as trailing arguments. After a fix, the affected files are re-linted automatically.

## Configuration

No configuration is required beyond the `jarl.toml` marker file. To override defaults in `.scrutin/config.toml`:

```toml
[[suite]]
tool = "jarl"
# default `run` lints R/**/*.R; `watch` defaults to `run` (linters re-check what they operate on).
# Override to lint a different tree:
# run = ["scripts/**/*.R", "inst/examples/**/*.R"]
```

jarl has no separate source/watch list (it lints files directly and does not track dependencies between them).
