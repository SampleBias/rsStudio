# The worker stays inside one top-level expression. Forward default (deferred)
# warnings now, while leaving R's handler active to retain its native history.
# Explicit warn=1/2/-1 and custom warning.expression handlers remain R's job.
with_live_warnings <- function(code) {
  withCallingHandlers(code, warning = function(condition) {
    if (isTRUE(getOption("warn") == 0) && is.null(getOption("warning.expression"))) {
      call <- conditionCall(condition)
      context <- if (is.null(call)) "" else paste0(" in ", paste(deparse(call), collapse = " "))
      cat("Warning", context, ": ", conditionMessage(condition), "\n", sep = "", file = stderr())
      flush(stderr())
    }
  })
}
