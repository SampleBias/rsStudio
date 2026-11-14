# rsStudio phase 1 worker. Only base R is required.
# stdout/stderr are user output, never control messages.
local({
port <- as.integer(Sys.getenv("RSSTUDIO_PORT"))
secret <- Sys.getenv("RSSTUDIO_SECRET")
Sys.unsetenv(c("RSSTUDIO_PORT", "RSSTUDIO_SECRET"))
con <- socketConnection(host = "127.0.0.1", port = port, open = "r+b", blocking = TRUE, timeout = 86400)
max_frame <- 1024L * 1024L
read_exact <- function(n) {
  result <- raw()
  while (length(result) < n) {
    part <- readBin(con, "raw", n = n - length(result))
    if (length(part) == 0L) stop("control connection closed")
    result <- c(result, part)
  }
  result
}
read_message <- function() {
  header <- read_exact(4L)
  n <- readBin(header, integer(), n = 1L, size = 4L, endian = "big", signed = TRUE)
  if (is.na(n) || n < 1L || n > max_frame) stop("invalid frame length")
  text <- rawToChar(read_exact(n))
  Encoding(text) <- "UTF-8"
  tabs <- gregexpr("\t", text, fixed = TRUE)[[1L]]
  if (length(tabs) < 2L || tabs[1L] < 1L) stop("invalid message")
  list(kind = substr(text, 1L, tabs[1L] - 1L),
       id = substr(text, tabs[1L] + 1L, tabs[2L] - 1L),
       payload = substring(text, tabs[2L] + 1L))
}
send_message <- function(kind, id, payload = "") {
  bytes <- charToRaw(enc2utf8(paste0(kind, "\t", id, "\t", payload)))
  if (length(bytes) > max_frame) stop("response exceeds frame limit")
  writeBin(as.integer(length(bytes)), con, size = 4L, endian = "big")
  writeBin(bytes, con)
  flush(con)
}
send_message("HELLO", "0", paste0("1:", secret))
rm(secret)
repeat {
  request <- tryCatch(read_message(), error = function(e) NULL)
  if (is.null(request) || identical(request$kind, "SHUTDOWN")) break
  if (!identical(request$kind, "EVAL")) {
    send_message("ERROR", request$id, "Unknown request")
    next
  }
  parsed <- tryCatch(parse(text = request$payload, keep.source = FALSE), error = identity)
  if (inherits(parsed, "error")) {
    # R's parser does not expose an incomplete-input class. LC_MESSAGES=C is set
    # by the supervisor; keep this behavior covered by real-R integration tests.
    message <- conditionMessage(parsed)
    kind <- if (grepl("unexpected end of input", message, fixed = TRUE)) "INCOMPLETE" else "ERROR"
    send_message(kind, request$id, message)
    next
  }
  err <- tryCatch({
    for (expression in parsed) {
      result <- withVisible(eval(expression, envir = .GlobalEnv))
      if (result$visible) print(result$value)
      flush(stdout())
      flush(stderr())
    }
    NULL
  }, error = identity)
  if (inherits(err, "error")) send_message("ERROR", request$id, conditionMessage(err))
  else send_message("DONE", request$id)
}
close(con)

})
