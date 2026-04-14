## divide(x, 0) emits an R warning. scrutin's tinytest runner captures
## these via withCallingHandlers and reports them in the "warned" bucket.
result <- divide(7, 0)
expect_true(is.infinite(result))

result2 <- divide(-3, 0)
expect_true(is.infinite(result2))
