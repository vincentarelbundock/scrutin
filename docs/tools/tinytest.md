# tinytest

A lightweight, zero-dependency testing framework for R packages. *Scrutin* auto-detects tinytest when a `DESCRIPTION` file and an `inst/tinytest/` directory are both present.

## Installing tinytest

[tinytest](https://github.com/markvanderloo/tinytest) is not shipped with *Scrutin*. Install it from CRAN:

```r
install.packages("tinytest")
```

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

`scrutin init` writes the default runner to `.scrutin/runners/tinytest.R`. Edit that file in place: the engine picks it up automatically whenever it exists. To point at a different path, set `runner` on an explicit suite:

```toml
[[suite]]
tool   = "tinytest"
runner = "shared/tinytest-runner.R"
```
