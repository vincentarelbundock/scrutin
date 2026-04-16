# testthat

The standard testing framework for R packages. *Scrutin* auto-detects testthat when a `DESCRIPTION` file and a `tests/testthat/` directory are both present.

## Installing testthat

[testthat](https://testthat.r-lib.org/) is not shipped with *Scrutin*. Install it from CRAN:

```r
install.packages("testthat")
```

## Directory structure

```
mypackage/
├── DESCRIPTION
├── NAMESPACE
├── R/
│   └── math.R
└── tests/
    └── testthat/
        └── test-math.R
```

## Minimal example

**R/math.R**

```r
#' @export
add <- function(x, y) x + y
```

**tests/testthat/test-math.R**

```r
test_that("add works", {
  expect_equal(add(2, 3), 5)
  expect_equal(add(-1, 1), 0)
})
```

## Running

```bash
scrutin mypackage              # TUI
scrutin -r plain mypackage     # text output
```

## Configuration

No configuration is required. To override defaults in `.scrutin/config.toml`:

```toml
[[suite]]
tool = "testthat"
# defaults pick up tests/testthat/**/test-*.R and watch R/**/*.R

# Override explicitly if needed:
# run   = ["tests/testthat/**/test-*.R"]
# watch = ["R/**/*.R"]
```

## Working directory

*Scrutin* runs workers from the **suite root** (the directory containing `DESCRIPTION`, which in a single-package project equals the project root; in a monorepo use `[[suite]] root = "r"` or similar). The subprocess CWD is the suite root. `testthat::test_path()` is the portable way to build paths to fixture files:

```r
test_that("reads fixture data", {
  d <- read.csv(test_path("fixtures", "data.csv"))
  expect_equal(nrow(d), 10)
})
```

Bare relative paths like `"inst/extdata/data.csv"` resolve against the suite root, which makes them portable between *Scrutin* and interactive `devtools::load_all()` sessions.

## Package loading

Workers call `pkgload::load_all()` to load the package. When a source file changes in watch mode, the engine reloads the package automatically before running affected tests. If you add or change `@export` or `@importFrom` tags, run `devtools::document()` separately: *Scrutin* does not invoke roxygen.

## Custom runner

`scrutin init` writes the default runner to `.scrutin/runners/testthat.R`. Edit that file in place: the engine automatically prefers it over the embedded default whenever it exists, no config change needed. To point at a different path (e.g. a shared runner in a sibling directory), set `runner` on an explicit suite:

```toml
[[suite]]
tool   = "testthat"
runner = "shared/testthat-runner.R"
```
