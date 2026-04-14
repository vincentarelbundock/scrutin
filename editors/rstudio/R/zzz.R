.onUnload <- function(libpath) {
  if (!is.null(.state$process) && .state$process$is_alive()) {
    .state$process$kill()
  }
  stop_editor_bridge()
}
