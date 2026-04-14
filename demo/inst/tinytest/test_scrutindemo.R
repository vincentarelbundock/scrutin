## Sanity
expect_equal(add(2, 3), 5)
expect_equal(shout("hi"), "HI")

## Failure: subtract is buggy
expect_equal(subtract(10, 4), 6)
