# Editor callback bridge: polls a temp file for open-file requests
# written by the inst/bin/rstudio-open helper script, then calls
# rstudioapi::navigateToFile() inside the running IDE session.

start_editor_bridge <- function() {
  fifo <- tempfile("scrutin-editor-")
  file.create(fifo)
  .state$editor_fifo <- fifo

  poll <- function() {
    if (is.null(.state$editor_fifo) || !file.exists(.state$editor_fifo)) return()
    lines <- readLines(.state$editor_fifo, warn = FALSE)
    if (length(lines) == 0L) return()
    # Truncate the file so requests aren't replayed.
    writeLines(character(0), .state$editor_fifo)
    for (line in lines) {
      req <- tryCatch(jsonlite::fromJSON(line), error = function(e) NULL)
      if (is.null(req) || is.null(req$file)) next
      nav_line <- if (!is.null(req$line) && !is.na(req$line)) req$line else -1L
      tryCatch(
        rstudioapi::navigateToFile(req$file, line = nav_line),
        error = function(e) message("scrutin: could not open file: ", e$message)
      )
    }
  }

  later::later(function() repeat_poll(poll), delay = 0.5)

  invisible(fifo)
}

repeat_poll <- function(poll_fn, interval = 0.3) {
  poll_fn()
  if (!is.null(.state$editor_fifo)) {
    later::later(function() repeat_poll(poll_fn, interval), delay = interval)
  }
}

stop_editor_bridge <- function() {
  if (!is.null(.state$editor_fifo)) {
    unlink(.state$editor_fifo)
    .state$editor_fifo <- NULL
  }
}
