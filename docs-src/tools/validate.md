# validate

A rule-based data validation framework for R. Test files are `.R` scripts that `confront()` data against a `validator()` and leave `validation` objects in the environment. The runner emits one event per validation rule with per-row metrics. Scrutin auto-detects validate when a `DESCRIPTION` file and a `tests/validate/` directory are both present.

## Directory structure

```
mypackage/
├── DESCRIPTION
├── R/
│   └── helpers.R
└── tests/
    └── validate/
        └── test_cars.R
```

## Minimal example

**tests/validate/test_cars.R**

```r
library(validate)

v <- validator(
  speed_pos   = speed > 0,
  dist_pos    = dist > 0,
  speed_limit = speed < 25
)
result <- confront(cars, v)
```

Every top-level `validation` object left in the environment after sourcing is picked up by the runner. Each rule becomes a separate event.

## Running

```bash
scrutin mypackage              # TUI
scrutin -r plain mypackage     # text output
```

## Configuration

No configuration is required. To override defaults in `.scrutin/config.toml`:

```toml
[[suite]]
tool        = "validate"
test_dirs = ["tests/validate"]
source_dirs = ["R"]
```
