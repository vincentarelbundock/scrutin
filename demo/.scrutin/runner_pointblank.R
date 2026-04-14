# scrutin pointblank runner
#
# Sources the shared R runner infrastructure, defines the pointblank-specific
# run function (walks interrogated agents), and starts the main loop.
# Edit this file to customize package loading or test execution.

# Locate this script's directory (works under both Rscript and source()).
.scrutin_script_dir <- local({
  args <- commandArgs(trailingOnly = FALSE)
  m <- grep("^--file=", args, value = TRUE)
  if (length(m) > 0) return(dirname(sub("^--file=", "", m[1])))
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

.scrutin_env$run_test <- function(path) {
  `%||%` <- .scrutin_env$`%||%`
  file <- basename(path)
  t0 <- proc.time()["elapsed"]
  counts <- list(pass = 0L, fail = 0L, error = 0L,
                 skip = 0L, xfail = 0L, warn = 0L)

  emit_step <- function(agent_obj, agent_label) {
    xl <- pointblank::get_agent_x_list(agent_obj)
    n_steps <- length(xl$i)
    if (n_steps == 0L) return(invisible(NULL))
    parent <- if (!is.null(agent_label) && nzchar(agent_label)) {
      agent_label
    } else if (length(xl$tbl_name) >= 1 && nzchar(xl$tbl_name[1])) {
      xl$tbl_name[1]
    } else {
      NULL
    }
    for (i in seq_len(n_steps)) {
      step_type <- as.character(xl$type[i] %||% "")
      cols <- xl$columns[[i]]
      cols_str <- if (is.null(cols)) "" else paste(as.character(cols), collapse = ",")
      step_name <- if (nzchar(cols_str)) {
        paste0(step_type, "(", cols_str, ")")
      } else {
        step_type
      }

      n        <- if (length(xl$n) >= i)        xl$n[i]        else NA
      n_failed <- if (length(xl$n_failed) >= i) xl$n_failed[i] else NA
      f_failed <- if (length(xl$f_failed) >= i) xl$f_failed[i] else NA
      eval_err <- isTRUE(xl$eval_error[i])
      flag_warn   <- isTRUE(xl$warn[i])
      flag_notify <- isTRUE(xl$notify[i])
      flag_stop   <- isTRUE(xl$stop[i])

      outcome <- if (eval_err) {
        "error"
      } else if (flag_stop) {
        "fail"
      } else if (flag_warn || flag_notify) {
        "warn"
      } else if (!is.na(n_failed) && n_failed == 0) {
        "pass"
      } else {
        "fail"
      }
      counts[[outcome]] <<- counts[[outcome]] + 1L

      metrics <- list()
      if (!is.na(n))        metrics$total    <- as.numeric(n)
      if (!is.na(n_failed)) metrics$failed   <- as.numeric(n_failed)
      if (!is.na(f_failed)) metrics$fraction <- as.numeric(f_failed)
      if (length(metrics) == 0) metrics <- NULL

      msg <- NULL
      if (eval_err) {
        cap <- xl$capture_stack[[i]]
        if (!is.null(cap)) {
          msg <- paste(as.character(cap), collapse = "\n")
        }
      }

      .scrutin_env$emit(.scrutin_env$event(
        file = file,
        outcome = outcome,
        subject_kind = "step",
        subject_name = step_name,
        subject_parent = parent,
        message = msg,
        metrics = metrics
      ))
    }
  }

  scratch <- new.env(parent = globalenv())
  tryCatch({
    sys.source(path, envir = scratch)

    nms <- ls(scratch, all.names = FALSE)
    agents <- Filter(function(n) inherits(get(n, envir = scratch), "ptblank_agent"), nms)

    if (length(agents) == 0) {
      counts$error <- counts$error + 1L
      .scrutin_env$emit(.scrutin_env$event(
        file = file,
        outcome = "error",
        subject_kind = "engine",
        subject_name = "<no_agent>",
        message = "no interrogated pointblank agent found in file"
      ))
    } else {
      for (nm in agents) {
        ag <- get(nm, envir = scratch)
        xl <- tryCatch(pointblank::get_agent_x_list(ag), error = function(e) NULL)
        if (is.null(xl) || length(xl$time_end) == 0) {
          counts$error <- counts$error + 1L
          .scrutin_env$emit(.scrutin_env$event(
            file = file,
            outcome = "error",
            subject_kind = "engine",
            subject_name = nm,
            message = "agent was not interrogated (call interrogate())"
          ))
          next
        }
        emit_step(ag, nm)
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
