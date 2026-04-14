# testthat

The standard testing framework for R packages. Scrutin auto-detects testthat when a `DESCRIPTION` file and a `tests/testthat/` directory are both present.

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
tool        = "testthat"
test_dirs = ["tests/testthat"]
source_dirs = ["R"]
```

## Working directory

Scrutin runs workers from the **project root**, not from `tests/testthat/`. Use `testthat::test_path()` to build paths to fixture files:

```r
test_that("reads fixture data", {
  d <- read.csv(test_path("fixtures", "data.csv"))
  expect_equal(nrow(d), 10)
})
```

Bare relative paths like `"fixtures/data.csv"` will not resolve correctly.

## Package loading

Workers call `pkgload::load_all()` to load the package. When a source file changes in watch mode, the engine reloads the package automatically before running affected tests. If you add or change `@export` or `@importFrom` tags, run `devtools::document()` separately: scrutin does not invoke roxygen.

## Custom runner

`scrutin init` writes the default runner to `.scrutin/testthat/runner.R`. To use an edited copy, point to it either globally (applies to every testthat suite) or on the specific `[[suite]]`:

```toml
[testthat]
runner = ".scrutin/testthat/runner.R"
```

or

```toml
[[suite]]
tool        = "testthat"
test_dirs = ["tests/testthat"]
runner    = ".scrutin/testthat/runner.R"
```
