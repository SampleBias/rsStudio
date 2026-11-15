# Exercise the actual generated worker with duplex raw streams instead of TCP.
# Session integration tests separately cover the real socket transport.
local({
  frame <- function(kind, id, payload = "") {
    bytes <- charToRaw(enc2utf8(paste(kind, id, payload, sep = "\t")))
    c(writeBin(as.integer(length(bytes)), raw(), size = 4L, endian = "big"), bytes)
  }
  input_bytes <- c(
    frame("EVAL", 1, "x <- 41"),
    frame("EVAL", 2, "sqrt(-1); x + 1"),
    frame("EVAL", 3, "stop('expected failure')"),
    frame("EVAL", 4, "x + 1"),
    frame("EVAL", 5, "function(x) {"),
    frame("EVAL", 6, "options(warn = 2); sqrt(-1)"),
    frame("EVAL", 7, "options(warn = -1); sqrt(-1)"),
    frame("PACKAGES", 8, "stats\nrsStudioDefinitelyMissingPackage"),
    frame("ENVIRONMENT", 9),
    frame("EVAL", 10, "plot(1:3)"),
    frame("EVAL", 11, "points(2, 2, col = 'red')"),
    frame("SHUTDOWN", 0)
  )
  input <- rawConnection(input_bytes, "rb")
  output <- rawConnection(raw(), "wb")
  on.exit(close(output))
  mock <- new.env(parent = globalenv())
  mock$socketConnection <- function(...) input
  mock$writeBin <- function(object, con, ...) {
    base::writeBin(object, if (identical(con, input)) output else con, ...)
  }
  plot_dir <- tempfile("rsstudio-worker-plots-")
  dir.create(plot_dir)
  on.exit(unlink(plot_dir, recursive = TRUE), add = TRUE)
  Sys.setenv(RSSTUDIO_PORT = "1", RSSTUDIO_SECRET = "test-secret", RSSTUDIO_PLOT_DIR = plot_dir)
  script <- commandArgs(trailingOnly = TRUE)[1L]
  printed <- capture.output(eval(parse(script), envir = mock))
  stopifnot(sum(grepl("42", printed, fixed = TRUE)) == 2L,
            identical(get("x", .GlobalEnv), 41),
            !exists("with_live_warnings", .GlobalEnv, inherits = FALSE))
  replies <- rawConnection(rawConnectionValue(output), "rb")
  on.exit(close(replies), add = TRUE)
  messages <- list()
  repeat {
    n <- readBin(replies, "integer", n = 1L, size = 4L, endian = "big")
    if (!length(n)) break
    messages[[length(messages) + 1L]] <- strsplit(rawToChar(readBin(replies, "raw", n = n)), "\t", fixed = TRUE)[[1L]]
  }
  kinds <- vapply(messages, `[`, "", 1L)
  stopifnot(identical(kinds,
                     c("HELLO", "DONE", "DONE", "ERROR", "DONE", "INCOMPLETE", "ERROR", "DONE", "STATUS", "ENVIRONMENT", "PLOT", "DONE", "PLOT", "DONE")),
            identical(messages[[1L]][3L], "1:test-secret"),
            grepl("expected failure", messages[[4L]][3L], fixed = TRUE),
            grepl("NaNs produced", messages[[7L]][3L], fixed = TRUE))
  environment <- paste(messages[[10L]][-(1:2)], collapse = "\t")
  stopifnot(grepl('"name":"x"', environment, fixed = TRUE))
  plots <- messages[kinds == "PLOT"]
  stopifnot(identical(plots[[1L]][3L], plots[[2L]][3L]))
  for (plot in plots) {
    path <- file.path(plot_dir, plot[4L])
    header <- readBin(path, "raw", n = 8L)
    stopifnot(identical(header, as.raw(c(137, 80, 78, 71, 13, 10, 26, 10))))
  }
  cat("Generated R worker regression checks passed\n")
})
