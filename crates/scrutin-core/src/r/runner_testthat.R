# scrutin testthat runner
#
# Sources the shared R runner infrastructure, defines the testthat-specific
# run function (using a custom R6 Reporter), and starts the main loop.
# Edit this file to customize package loading or test execution.

# Locate this script's directory (works under both Rscript and source()).
.scrutin_script_dir <- local({
  # Rscript sets --file=<path> in commandArgs().
  args <- commandArgs(trailingOnly = FALSE)
  m <- grep("^--file=", args, value = TRUE)
  if (length(m) > 0) return(dirname(sub("^--file=", "", m[1])))
  # Fallback for source(): sys.frame has $ofile.
  for (i in rev(seq_len(sys.nframe()))) {
    f <- sys.frame(i)$ofile
    if (!is.null(f)) return(dirname(f))
  }
  "."
})
source(file.path(.scrutin_script_dir, "runner_r.R"), local = FALSE)

# Load the package under test and set up runtime dependency tracing.
.scrutin_env$load_package()
.scrutin_env$setup_tracing()

# --- testthat reporter that emits NDJSON events ---

.scrutin_env$ScrutinReporter <- R6::R6Class("ScrutinReporter",
  inherit = testthat::Reporter,
  public = list(
    current_file = NULL,
    start_time = NULL,
    counts = NULL,

    initialize = function(...) {
      super$initialize(...)
      self$counts <- list(pass = 0L, fail = 0L, error = 0L,
                          skip = 0L, xfail = 0L, warn = 0L)
    },

    set_file = function(file) {
      self$current_file <- basename(file)
    },

    add_result = function(context, test, result) {
      ms <- as.integer(difftime(Sys.time(), self$start_time, units = "secs") * 1000)
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
      .scrutin_env$emit(.scrutin_env$event(
        file = self$current_file,
        outcome = outcome,
        subject_kind = "function",
        subject_name = test,
        message = msg,
        line = line,
        duration_ms = ms
      ))
    },

    start_test = function(context, test) {
      self$start_time <- Sys.time()
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
    .scrutin_env$emit_summary(file, reporter$counts, elapsed)
  }, error = function(e) {
    elapsed <- as.integer((proc.time()["elapsed"] - t0) * 1000)
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
