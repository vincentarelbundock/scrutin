# Warning-only file: every test passes but emits a warning, so the file
# row should render with a yellow icon and the WARN status (no failures).

test_that("divide(x, 0) warns and returns Inf", {
  result <- divide(8, 0)
  expect_true(is.infinite(result))
})

test_that("divide(-x, 0) warns and returns -Inf", {
  result <- divide(-2, 0)
  expect_true(is.infinite(result))
})
