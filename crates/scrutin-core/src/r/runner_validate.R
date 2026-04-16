# scrutin validate runner
#
# Defines the validate-specific run function (sources a file, finds
# validation objects, calls summary(), emits NDJSON) and starts the main
# loop. The shared R runner infrastructure (including package loading +
# trace setup) is prepended at compile time from runner_r.R.

.scrutin_env$run_test <- function(path) {
  `%||%` <- .scrutin_env$`%||%`
  file <- basename(path)
  t0 <- proc.time()["elapsed"]
  counts <- list(pass = 0L, fail = 0L, error = 0L,
                 skip = 0L, xfail = 0L, warn = 0L)

  # Check that validate is available.
  if (!requireNamespace("validate", quietly = TRUE)) {
    counts$error <- counts$error + 1L
    .scrutin_env$emit(.scrutin_env$event(
      file = file,
      outcome = "error",
      subject_kind = "engine",
      subject_name = "<validate>",
      message = "validate package is not installed"
    ))
    elapsed <- as.integer((proc.time()["elapsed"] - t0) * 1000)
    .scrutin_env$emit_summary(file, counts, elapsed)
    return(invisible(NULL))
  }

  emit_validation <- function(cf_obj, parent_name) {
    s <- summary(cf_obj)
    if (nrow(s) == 0L) {
      counts$pass <<- counts$pass + 1L
      .scrutin_env$emit(.scrutin_env$event(
        file = file,
        outcome = "pass",
        subject_kind = "rule",
        subject_name = "<empty>",
        subject_parent = parent_name
      ))
      return(invisible(NULL))
    }

    errs <- validate::errors(cf_obj)
    warns <- validate::warnings(cf_obj)

    for (i in seq_len(nrow(s))) {
      rule_name <- as.character(s$name[i])
      items     <- as.integer(s$items[i])
      passes    <- as.integer(s$passes[i])
      fails     <- as.integer(s$fails[i])
      n_na      <- as.integer(s$nNA[i])
      has_error <- isTRUE(s$error[i])
      has_warn  <- isTRUE(s$warning[i])
      expr_text <- as.character(s$expression[i])

      # Outcome mapping per spec.
      outcome <- if (has_error) {
        "error"
      } else if (fails > 0L) {
        "fail"
      } else if (has_warn) {
        "warn"
      } else {
        "pass"
      }
      counts[[outcome]] <<- counts[[outcome]] + 1L

      # Message.
      msg <- NULL
      if (has_error) {
        msg <- errs[[rule_name]] %||% "rule evaluation error"
        if (is.character(msg) && length(msg) > 1L) {
          msg <- paste(msg, collapse = "\n")
        }
      } else if (has_warn && fails == 0L) {
        msg <- warns[[rule_name]] %||% "rule evaluation warning"
        if (is.character(msg) && length(msg) > 1L) {
          msg <- paste(msg, collapse = "\n")
        }
      } else if (fails > 0L) {
        msg <- expr_text
      }

      # Append NA note if relevant.
      if (!is.na(n_na) && n_na > 0L && !is.null(msg)) {
        msg <- paste0(msg, " (", n_na, " NA rows inconclusive)")
      } else if (!is.na(n_na) && n_na > 0L) {
        msg <- paste0(n_na, " NA rows inconclusive")
      }

      # Metrics (always emitted for validate).
      metrics <- list()
      if (!is.na(items))  metrics$total    <- as.numeric(items)
      if (!is.na(fails))  metrics$failed   <- as.numeric(fails)
      if (!is.na(items) && !is.na(fails) && items > 0L) {
        metrics$fraction <- as.numeric(fails) / as.numeric(items)
      }
      if (!is.na(n_na) && n_na > 0L) metrics$na <- as.numeric(n_na)
      if (length(metrics) == 0L) metrics <- NULL

      .scrutin_env$emit(.scrutin_env$event(
        file = file,
        outcome = outcome,
        subject_kind = "rule",
        subject_name = rule_name,
        subject_parent = parent_name,
        message = msg,
        metrics = metrics
      ))
    }
  }

  scratch <- new.env(parent = globalenv())
  tryCatch({
    sys.source(path, envir = scratch)

    nms <- ls(scratch, all.names = FALSE)
    validations <- Filter(
      function(n) inherits(get(n, envir = scratch), "validation"),
      nms
    )

    if (length(validations) == 0L) {
      counts$error <- counts$error + 1L
      .scrutin_env$emit(.scrutin_env$event(
        file = file,
        outcome = "error",
        subject_kind = "engine",
        subject_name = "<no_validation>",
        message = "no confrontation result (validation object) found in file"
      ))
    } else {
      for (nm in validations) {
        cf <- get(nm, envir = scratch)
        tryCatch(
          emit_validation(cf, nm),
          error = function(e) {
            counts$error <<- counts$error + 1L
            .scrutin_env$emit(.scrutin_env$event(
              file = file,
              outcome = "error",
              subject_kind = "engine",
              subject_name = nm,
              message = paste0(
                "failed to read validation object: ",
                conditionMessage(e)
              )
            ))
          }
        )
      }
    }

    elapsed <- as.integer((proc.time()["elapsed"] - t0) * 1000)
    .scrutin_env$emit_summary(file, counts, elapsed)
  }, error = function(e) {
    elapsed <- as.integer((proc.time()["elapsed"] - t0) * 1000)
    counts$error <- counts$error + 1L
    .scrutin_env$emit(.scrutin_env$event(
      file = file,
      outcome = "error",
      subject_kind = "function",
      subject_name = "<file>",
      message = conditionMessage(e),
      line = .scrutin_env$error_line(e),
      duration_ms = elapsed
    ))
    .scrutin_env$emit_summary(file, counts, elapsed)
  })
}

.scrutin_env$main()
