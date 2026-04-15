# tinytest

A lightweight, zero-dependency testing framework for R packages. *Scrutin* auto-detects tinytest when a `DESCRIPTION` file and an `inst/tinytest/` directory are both present.

## Directory structure

```
mypackage/
├── DESCRIPTION
├── NAMESPACE
├── R/
│   └── math.R
└── inst/
    └── tinytest/
        └── test_math.R
```

## Minimal example

**R/math.R**

```r
#' @export
add <- function(x, y) x + y
```

**inst/tinytest/test_math.R**

```r
expect_equal(add(2, 3), 5)
expect_equal(add(-1, 1), 0)
```

tinytest tests are plain R scripts with `expect_*` calls at the top level (no `test_that()` wrapper).

## Running

```bash
scrutin mypackage              # TUI
scrutin -r plain mypackage     # text output
```

## Configuration

No configuration is required. To override defaults in `.scrutin/config.toml`:

```toml
[[suite]]
tool = "tinytest"
# defaults pick up inst/tinytest/**/test-*.R and watch R/**/*.R
```

## Custom runner

`scrutin init` writes the default runner to `.scrutin/tinytest/runner.R`. Point to your edited copy either globally or on the specific `[[suite]]`:

```toml
[tinytest]
runner = ".scrutin/tinytest/runner.R"
```

or

```toml
[[suite]]
tool   = "tinytest"
runner = ".scrutin/tinytest/runner.R"
```
