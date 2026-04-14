#' Intentionally messy code to trigger jarl lint warnings.

# Unnecessary nested if
check_sign <- function(x) {
  if (x > 0) {
    if (x > 10) {
      "big positive"
    } else {
      "small positive"
    }
  }
}

# seq_len instead of 1:n
count_up <- function(n) {
  out <- c()
  for (i in 1:n) {
    out <- c(out, i)
  }
  out
}

# T/F instead of TRUE/FALSE
flag_positive <- function(x) {
  if (x > 0) T else F
}

# Unneeded return in if/else
classify <- function(x) {
  if (x > 0) {
    return("positive")
  } else {
    return("negative")
  }
}

# == NA instead of is.na
#
# This one needs two fix passes. Pass 1: equals_na rewrites `x == NA` to
# `is.na(x)`, producing `any(is.na(x))`. Pass 2: any_is_na rewrites that
# to `anyNA(x)`. The second violation only exists after the first fix, so
# jarl cannot resolve both in a single invocation.
has_missing <- function(x) {
  any(x == NA)
}

# Unnecessary paste0 with sep
join_words <- function(a, b) {
  paste(a, b, sep = "")
}

# ifelse with TRUE/FALSE
is_even <- function(x) {
  ifelse(x %% 2 == 0, TRUE, FALSE)
}

# Growing a vector in a loop
slow_squares <- function(n) {
  result <- c()
  for (i in 1:n) {
    result <- c(result, i^2)
  }
  result
}
