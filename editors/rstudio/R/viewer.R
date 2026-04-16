#' Show the scrutin dashboard in the Viewer pane
#' @export
scrutin_show <- function() {
  if (is.null(.state$url)) {
    stop("scrutin is not running. Call scrutin_start() first.", call. = FALSE)
  }
  url <- paste0(.state$url, "/?theme=", ide_theme())
  if (rstudioapi::isAvailable()) {
    rstudioapi::viewer(url)
  } else {
    utils::browseURL(url)
  }
  invisible()
}

ide_theme <- function() {
  tryCatch({
    info <- rstudioapi::getThemeInfo()
    if (isTRUE(info$dark)) "dark" else "light"
  }, error = function(e) "dark")
}
