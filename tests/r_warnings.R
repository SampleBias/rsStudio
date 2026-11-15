# Run without sockets so warning semantics are checked even in restricted CI.
local({
  source("r/warnings.R", local = TRUE)
  capture_messages <- function(code) {
    path <- tempfile()
    con <- file(path, "wt")
    sink(con, type = "message")
    on.exit({ sink(type = "message"); close(con); unlink(path) })
    force(code)
    flush(con)
    readLines(path, warn = FALSE)
  }
  options(warn = 0)
  lines <- capture_messages(with_live_warnings({
    warning("live default warning", call. = FALSE)
    cat("EXPRESSION STILL RUNNING\n", file = stderr())
  }))
  stopifnot(grepl("live default warning", lines[1L]),
            identical(lines[2L], "EXPRESSION STILL RUNNING"), getOption("warn") == 0)

  options(warn = 1)
  lines <- capture_messages(with_live_warnings(warning("native immediate", call. = FALSE)))
  stopifnot(sum(grepl("native immediate", lines)) == 1L, getOption("warn") == 1)
  options(warn = -1)
  lines <- capture_messages(with_live_warnings(warning("suppressed")))
  stopifnot(length(lines) == 0L, getOption("warn") == -1)
  options(warn = 2)
  error <- tryCatch(with_live_warnings(warning("escalated", call. = FALSE)), error = identity)
  stopifnot(inherits(error, "error"), grepl("escalated", conditionMessage(error)), getOption("warn") == 2)

  options(warn = 0)
  lines <- capture_messages(with_live_warnings(withCallingHandlers(
    warning("muffled"), warning = function(w) invokeRestart("muffleWarning"))))
  stopifnot(length(lines) == 0L)
  options(warning.expression = quote(cat("CUSTOM WARNING\n", file = stderr())))
  lines <- capture_messages(with_live_warnings(warning("custom handler")))
  stopifnot(identical(lines, "CUSTOM WARNING"))
  options(warning.expression = NULL)
  invisible(parse("r/worker.R"))
  cat("R warning regression checks passed\n")
})
