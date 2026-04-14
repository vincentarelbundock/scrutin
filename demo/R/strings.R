#' Shout a message in upper case.
#' @export
shout <- function(x) toupper(x)

#' Pretend to call an external service.
#'
#' Always throws — used to demonstrate the "errored" outcome bucket.
#' @export
fetch_remote <- function(url) {
  stop("network unavailable: cannot reach ", url)
}
