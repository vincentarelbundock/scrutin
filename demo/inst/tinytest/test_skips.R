if (Sys.getenv("RUN_SLOW_TESTS") != "1") {
  exit_file("set RUN_SLOW_TESTS=1 to enable")
}

expect_equal(add(100, 200), 300)
