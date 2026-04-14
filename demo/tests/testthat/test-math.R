test_that("add works", {
  expect_equal(add(2, 3), 5)
  expect_equal(add(-1, 1), 0)
})

test_that("subtract works (this will fail because subtract is buggy)", {
  expect_equal(subtract(5, 3), 2)
})

test_that("divide warns on zero", {
  expect_warning(divide(1, 0), "division by zero")
})

test_that("divide produces a stray warning the runner should surface", {
  result <- divide(10, 0)
  expect_true(is.infinite(result))
})

test_that("slow integration test", {
  skip("integration server not configured")
  expect_equal(add(1, 1), 2)
})
