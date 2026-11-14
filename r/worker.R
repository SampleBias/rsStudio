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
# Use R's configured user library when available; otherwise use a versioned
# rsStudio user library. Add an existing directory before any package queries.
default_user_library <- function() {
  configured <- Sys.getenv("R_LIBS_USER", unset = "")
  candidates <- strsplit(configured, .Platform$path.sep, fixed = TRUE)[[1L]]
  if (length(candidates) && nzchar(candidates[1L])) return(path.expand(candidates[1L]))
  minor <- strsplit(R.version$minor, "\\.")[[1L]][1L]
  file.path(path.expand("~"), "R", "rsstudio-library", paste0(R.version$major, ".", minor))
}
user_lib <- default_user_library()
if (dir.exists(user_lib)) .libPaths(c(user_lib, .libPaths()))
valid_package <- function(name) {
  length(name) == 1L && nchar(name, type = "bytes") <= 100L &&
    grepl("^[A-Za-z][A-Za-z0-9.]*$", name) && !grepl("\\.$", name)
}
package_status <- function(names) {
  if (!length(names) || length(names) > 200L || anyDuplicated(names) ||
      !all(vapply(names, valid_package, logical(1L)))) stop("Invalid package names")
  rows <- vapply(names, function(name) {
    location <- find.package(name, quiet = TRUE)
    if (!length(location)) return(paste(name, "-", "0", sep = "\t"))
    version <- tryCatch(as.character(utils::packageVersion(name, lib.loc = dirname(location[1L]))),
                        error = function(e) "unknown")
    if (grepl("[\t\r\n]", version) || nchar(version, type = "bytes") > 100L) version <- "unknown"
    attached <- as.integer(paste0("package:", name) %in% search())
    paste(name, version, attached, sep = "\t")
  }, character(1L))
  paste(rows, collapse = "\n")
}
install_one <- function(name, repo) {
  if (!valid_package(name) || !repo %in% c("CRAN", "BIOC")) stop("Invalid package request")
  if (length(find.package(name, quiet = TRUE))) {
    cat(name, "is already installed. Refresh the package list to load it.\n")
    return(invisible(NULL))
  }
  if (!dir.exists(user_lib) && !dir.create(user_lib, recursive = TRUE, showWarnings = FALSE))
    stop("Could not create the R user library: ", user_lib)
  if (file.access(user_lib, 2L) != 0L) stop("R user library is not writable: ", user_lib)
  .libPaths(c(user_lib, .libPaths()))
  cat("Installing", name, "into", user_lib, "\n")
  cran <- "https://cloud.r-project.org"
  if (identical(repo, "BIOC")) {
    if (!requireNamespace("BiocManager", quietly = TRUE)) {
      utils::install.packages("BiocManager", lib = user_lib, repos = cran, dependencies = NA)
      if (!requireNamespace("BiocManager", quietly = TRUE)) stop("BiocManager installation failed; see console output")
    }
    old_repos <- getOption("repos")
    on.exit(options(repos = old_repos), add = TRUE)
    options(repos = c(CRAN = cran))
    BiocManager::install(name, lib = user_lib, ask = FALSE, update = FALSE, dependencies = NA)
  } else {
    utils::install.packages(name, lib = user_lib, repos = cran, dependencies = NA)
  }
  if (!length(find.package(name, lib.loc = user_lib, quiet = TRUE)))
    stop("Installation did not complete; see console output for details")
  tryCatch(utils::packageVersion(name, lib.loc = user_lib),
           error = function(e) stop("Installed package is invalid; see console output: ", conditionMessage(e)))
  cat("Installed", name, "successfully.\n")
}
send_message("HELLO", "0", paste0("1:", secret))
rm(secret)
repeat {
  request <- tryCatch(read_message(), error = function(e) NULL)
  if (is.null(request) || identical(request$kind, "SHUTDOWN")) break
  if (request$kind %in% c("PACKAGES", "INSTALL", "LOAD")) {
    result <- tryCatch({
      if (identical(request$kind, "PACKAGES")) {
        names <- strsplit(request$payload, "\n", fixed = TRUE)[[1L]]
        send_message("STATUS", request$id, package_status(names))
      } else if (identical(request$kind, "INSTALL")) {
        fields <- strsplit(request$payload, "\t", fixed = TRUE)[[1L]]
        if (length(fields) != 2L) stop("Invalid package request")
        install_one(fields[2L], fields[1L])
        send_message("DONE", request$id)
      } else {
        if (!valid_package(request$payload)) stop("Invalid package name")
        base::library(package = request$payload, character.only = TRUE, quietly = TRUE)
        cat("Loaded", request$payload, "into the current R session.\n")
        send_message("DONE", request$id)
      }
      flush(stdout()); flush(stderr())
      NULL
    }, error = identity)
    if (inherits(result, "error")) send_message("ERROR", request$id, conditionMessage(result))
    next
  }
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
