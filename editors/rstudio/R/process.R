.state <- new.env(parent = emptyenv())
.state$process <- NULL
.state$url <- NULL

#' Start the scrutin web dashboard
#'
#' Spawns `scrutin -r web` as a background process and opens the dashboard
#' in the RStudio Viewer pane.
#'
#' @param project Path to the project root. Defaults to the active RStudio
#'   project or the working directory.
#' @param watch Logical; enable file-watch mode (`-w`).
#' @param port Port number. If `NULL`, a free port is chosen automatically.
#' @return The dashboard URL (invisibly).
#' @export
scrutin_start <- function(project = NULL, watch = TRUE, port = NULL) {
  if (!is.null(.state$process) && .state$process$is_alive()) {
    message("scrutin is already running at ", .state$url)
    scrutin_show()
    return(invisible(.state$url))
  }

  project <- project %||%
    tryCatch(rstudioapi::getActiveProject(), error = function(e) NULL) %||%
    getwd()
  port <- port %||% find_free_port()
  bin <- find_scrutin_binary()

  args <- c("-r", paste0("web:127.0.0.1:", port), "--no-open")
  if (watch) args <- c(args, "-w")
  args <- c(args, project)

  env <- editor_env()

  .state$process <- processx::process$new(
    bin, args,
    stdout = "|", stderr = "|",
    env = c("current", env),
    cleanup = TRUE, cleanup_tree = TRUE
  )

  url <- wait_for_server(.state$process, timeout = 10)
  .state$url <- url

  scrutin_show()
  invisible(url)
}

#' Stop the running scrutin process
#' @export
scrutin_stop <- function() {
  if (is.null(.state$process)) {
    message("scrutin is not running.")
    return(invisible())
  }
  if (.state$process$is_alive()) {
    .state$process$kill()
  }
  stop_editor_bridge()
  .state$process <- NULL
  .state$url <- NULL
  message("scrutin stopped.")
  invisible()
}

#' Report whether scrutin is running
#' @return A list with components `running`, `url`, and `pid` (invisibly).
#' @export
scrutin_status <- function() {
  running <- !is.null(.state$process) && .state$process$is_alive()
  info <- list(
    running = running,
    url     = if (running) .state$url else NA_character_,
    pid     = if (running) .state$process$get_pid() else NA_integer_
  )
  if (running) {
    message("scrutin is running at ", info$url, " (pid ", info$pid, ")")
  } else {
    message("scrutin is not running.")
  }
  invisible(info)
}

# -- helpers ----------------------------------------------------------

editor_env <- function() {
  # Positron has a CLI that works like `code --goto`, already supported
  # by the Rust side. For RStudio proper, use the file-based bridge.
  if (nzchar(Sys.getenv("POSITRON"))) {
    return(c(EDITOR = "positron"))
  }
  # RStudio: start the editor bridge and point EDITOR at our helper script.
  fifo <- start_editor_bridge()
  helper <- system.file("bin", "rstudio-open", package = "scrutin", mustWork = TRUE)
  c(EDITOR = helper, SCRUTIN_EDITOR_FIFO = fifo)
}

find_scrutin_binary <- function() {
  bin <- getOption("scrutin.binary", default = Sys.which("scrutin"))
  if (!nzchar(bin)) {
    stop(
      "Cannot find the 'scrutin' binary. ",
      "Install it or set options(scrutin.binary = '/path/to/scrutin').",
      call. = FALSE
    )
  }
  bin
}

find_free_port <- function() {
  for (i in seq_len(20L)) {
    port <- sample(49152L:65535L, 1L)
    tryCatch({
      srv <- serverSocket(port)
      close(srv)
      return(port)
    }, error = function(e) NULL)
  }
  stop("Could not find a free port after 20 attempts.", call. = FALSE)
}

wait_for_server <- function(proc, timeout = 10) {
  deadline <- Sys.time() + timeout
  while (Sys.time() < deadline) {
    if (!proc$is_alive()) {
      err <- proc$read_all_error_lines()
      stop("scrutin exited before the server started:\n", paste(err, collapse = "\n"), call. = FALSE)
    }
    lines <- proc$read_error_lines()
    for (line in lines) {
      m <- regmatches(line, regexpr("http://[^ ]+", line))
      if (length(m) == 1L) return(m)
    }
    Sys.sleep(0.1)
  }
  proc$kill()
  stop("Timed out waiting for the scrutin web server to start.", call. = FALSE)
}
