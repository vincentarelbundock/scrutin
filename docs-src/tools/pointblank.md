# pointblank

A data validation framework for R. Test files are `.R` scripts that produce interrogated `ptblank_agent` objects. The runner emits one event per validation step. *Scrutin* auto-detects pointblank when a `DESCRIPTION` file and a `tests/pointblank/` directory are both present.

## Directory structure

```
mypackage/
├── DESCRIPTION
├── R/
│   └── helpers.R
└── tests/
    └── pointblank/
        └── test_users.R
```

## Minimal example

**tests/pointblank/test_users.R**

```r
users <- data.frame(
  id    = 1:5,
  email = c("a@x.com", "b@x.com", "c@x.com", "d@x.com", "e@x.com"),
  age   = c(22, 31, 45, 19, 28)
)

agent <- pointblank::create_agent(tbl = users, tbl_name = "users") |>
  pointblank::col_vals_not_null(columns = "id") |>
  pointblank::col_vals_not_null(columns = "email") |>
  pointblank::col_vals_between(columns = "age", left = 18, right = 65) |>
  pointblank::interrogate()
```

Every top-level `ptblank_agent` object left in the environment after sourcing is picked up by the runner. You can have multiple agents per file.

## Running

```bash
scrutin mypackage              # TUI
scrutin -r plain mypackage     # text output
```

## Configuration

No configuration is required. To override defaults in `.scrutin/config.toml`:

```toml
[[suite]]
tool = "pointblank"
# defaults pick up tests/pointblank/**/test-*.R and watch R/**/*.R
```
