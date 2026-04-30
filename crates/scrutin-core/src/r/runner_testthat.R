# scrutin testthat runner
#
# Defines the testthat-specific run function (using a custom R6 Reporter)
# and starts the main loop. The shared R runner infrastructure (NDJSON
# encoder, emit helpers, srcref lookup, worker hooks, stdin loop, package
# loading, trace setup) is prepended at compile time from runner_r.R.

# --- testthat reporter that emits NDJSON events ---

.scrutin_env$ScrutinReporter <- R6::R6Class("ScrutinReporter",
  inherit = testthat::Reporter,
  public = list(
    current_file = NULL,
    start_time = NULL,
    counts = NULL,
    events = NULL,

    initialize = function(...) {
      super$initialize(...)
      self$counts <- list(pass = 0L, fail = 0L, error = 0L,
                          skip = 0L, xfail = 0L, warn = 0L)
      self$events <- list()
    },

    set_file = function(file) {
      self$current_file <- basename(file)
    },

    add_result = function(context, test, result) {
      ms <- if (is.null(self$start_time)) 0L else
        as.integer(difftime(Sys.time(), self$start_time, units = "secs") * 1000)
      line <- .scrutin_env$expectation_line(result, self$current_file)

      outcome <- if (inherits(result, "expectation_skip")) {
        "skip"
      } else if (inherits(result, "expectation_warning")) {
        "warn"
      } else if (inherits(result, "expectation_error")) {
        "error"
      } else if (inherits(result, "expectation_success")) {
        "pass"
      } else {
        "fail"
      }
      self$counts[[outcome]] <- self$counts[[outcome]] + 1L

      msg <- if (outcome == "pass") NULL else result$message
      self$events[[length(self$events) + 1L]] <- .scrutin_env$event(
        file = self$current_file,
        outcome = outcome,
        subject_kind = "function",
        subject_name = if (is.null(test) || !nzchar(test)) "<file>" else test,
        message = msg,
        line = line,
        duration_ms = ms
      )
    },

    start_test = function(context, test) {
      self$start_time <- Sys.time()
    },

    flush = function() {
      if (length(self$events) == 0L) return(invisible())
      for (event in self$events) {
        .scrutin_env$emit(event)
      }
      self$events <- list()
      invisible()
    },

    end_reporter = function() {}
  )
)

.scrutin_env$run_test <- function(path) {
  file <- basename(path)
  t0 <- proc.time()["elapsed"]
  reporter <- .scrutin_env$ScrutinReporter$new()
  reporter$set_file(path)
  tryCatch({
    testthat::test_file(path, reporter = reporter)
    elapsed <- as.integer((proc.time()["elapsed"] - t0) * 1000)
    if (sum(unlist(reporter$counts)) == 0L) {
      reporter$counts[["skip"]] <- 1L
      reporter$events[[length(reporter$events) + 1L]] <- .scrutin_env$event(
        file = file,
        outcome = "skip",
        subject_kind = "function",
        subject_name = "<file>",
        message = NULL,
        line = NULL,
        duration_ms = elapsed
      )
    }
    reporter$flush()
    .scrutin_env$emit_summary(file, reporter$counts, elapsed)
  }, error = function(e) {
    elapsed <- as.integer((proc.time()["elapsed"] - t0) * 1000)
    reporter$flush()
    .scrutin_env$emit(.scrutin_env$event(
      file = file,
      outcome = "error",
      subject_kind = "function",
      subject_name = "<file>",
      message = conditionMessage(e),
      line = .scrutin_env$error_line(e),
      duration_ms = elapsed
    ))
    .scrutin_env$emit_summary(file, list(error = 1L), elapsed)
  })
}

.scrutin_env$main()
