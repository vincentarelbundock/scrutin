#' Add two numbers.
#' @export
add <- function(x, y) x + y

#' Subtract two numbers.
#'
#' Intentionally buggy: returns x + y instead of x - y, so test files
#' that check subtraction will fail.
#' @export
subtract <- function(x, y) x + y

#' Divide two numbers.
#'
#' Emits an R warning when dividing by zero (instead of returning Inf
#' silently). Used by the runner to demonstrate warning capture.
#' @export
divide <- function(x, y) {
  if (y == 0) warning("division by zero")
  x / y
}
