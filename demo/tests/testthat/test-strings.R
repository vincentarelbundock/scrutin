test_that("shout uppercases", {
  expect_equal(shout("hello"), "HELLO")
})

test_that("fetch_remote errors out", {
  fetch_remote("https://example.invalid")
})

test_that("environment-gated test", {
  skip_if_not(Sys.getenv("RUN_SLOW_TESTS") == "1", "set RUN_SLOW_TESTS=1 to enable")
  expect_true(TRUE)
})
