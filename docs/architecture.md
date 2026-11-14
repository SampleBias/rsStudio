# Architecture and R runtime

## Boundaries

The GUI owns presentation, editor buffers, settings, and dispatch. A session supervisor owns one R child process, its lifecycle, control connection, and separate stdout/stderr readers. All long-running work stays off the egui frame thread; bounded channels deliver events and request repaints. R owns computation and the global workspace. The UI never evaluates R or links to R internals. Proposed modules: `app`, `ui`, `editor`, `session`, `protocol`, `files`, and `theme`; split crates only when boundaries justify it.

## Runtime discovery and startup

1. Use an explicitly configured R executable if set; otherwise search `PATH` for `R`. Store an executable path, not a shell command; launch with argument APIs, never shell interpolation.
2. Validate by running `R --version` with a timeout, capture the reported version, and report missing, inaccessible, or incompatible installations with a way to select a path and retry. Choose a minimum supported version during phase 01 and test it in CI.
3. Start an independently installed R process in the selected project directory. For the initial worker use `R --vanilla --quiet --no-echo -f <worker.R>` (or an equivalent verified invocation); launch a bundled, project-owned base-R helper script, not a separate R process for each command. No R packages should be required for the first milestone.
4. `--vanilla` suppresses user/site profiles and environment files and disables workspace restoration and saving; it can affect project setup such as `renv`. State this clearly in the UI. Make profile-enabled behavior a later explicit, tested option. `--vanilla` is not a sandbox. See [R invocation](https://cran.r-project.org/doc/FAQ/R-intro.html) and [R startup](https://www.stat.ethz.ch/R-manual/R-devel/library/base/html/Startup.html).

## Control protocol and execution

The prototype must prove the protocol before other features depend on it. The Rust supervisor listens on an ephemeral loopback TCP port, passes its address and a fresh random per-launch secret to the child, and accepts one authenticated R connection with a startup deadline. Use a base-R-compatible, versioned, length-prefixed UTF-8 protocol with bounded frames and request IDs; specify byte order, maximum frame size, message types, and encoding in an implementation RFC before coding. Treat the secret as a local access control aid, not a sandbox; never log it. If a connection fails, close the listener, terminate and reap the child, and report a recoverable error. Evaluate a Unix-domain socket if the chosen supported R versions expose a reliable base-R implementation.

The phase 1 protocol supports handshake, evaluate, and shutdown; future inspection messages require a protocol revision. Responses cover completion, errors, and incomplete input. See [protocol version 1](protocol.md). One request executes at a time; requests have IDs, and each restart replaces the event channel so replies from an older process cannot update the current UI state. The worker parses each complete submission, evaluates expressions in `.GlobalEnv`, prints visible results, reports errors, and continues after recoverable R errors. Parse incomplete input separately from syntax errors: preserve the pending buffer and request continuation; never execute partial input. Escape and frame source as data, never splice arbitrary source or object names into helper code.

Drain stdout and stderr concurrently as byte streams, with incremental UTF-8 decoding and replacement for invalid sequences; cap buffers and surface truncation. Streams may interleave and cannot be reliably attributed to a particular expression or ordered relative to control messages without additional synchronization. Output text, including prompt-looking strings, is never a control message. A user can alter R connections or terminate the process, so detect broken pipes, control-channel loss, and EOF; fail outstanding requests and expose restart. Frame limits and queue limits must produce visible errors rather than deadlock.

## Lifecycle and console compatibility boundary

States: `NotConfigured → Starting → Ready → Busy → Ready`; failures lead to `Failed` or `Exited`, and restart creates a new generation. Show state and disable conflicting commands. On close, request graceful shutdown with a deadline, then terminate and reap; never silently save `.RData`. On Linux prototype interrupt using a signal to the R child and prove it returns to a usable worker loop; otherwise expose stop-and-restart with an explicit state-loss warning. Test child crashes and descendant-process cleanup; do not claim interruption works until verified.

The initial helper is a **submission console**, not a terminal: it preserves objects across requests and provides normal visible results, errors, multiline input, and output, but R started with `-f` is non-interactive. `interactive()`, `readline()`, menus, `browser()`, packages requiring a TTY, and commands reading stdin are not guaranteed. Prevent indefinite hangs with a visible stop/restart control; communicate these limits. A timeout must not silently kill legitimate long-running calculations. Before promising full interactive-console support, prototype an interactive R/PTY or other independently implemented transport, with separate structured control and robust input routing; compare it against the worker against the phase 04 compatibility tests. Switching transports is an explicit decision gate, not a hidden assumption.

## Feature integration

- **Editor:** send text as a normal evaluate request; source and console share one workspace and ordered queue.
- **Environment/data:** after completed user execution, request summaries by object name, on demand. Metadata and row pages must be bounded; avoid evaluating arbitrary object expressions. Document that active bindings and class methods may execute user R code. Represent unsupported types or failures explicitly, without crashing or eagerly copying whole objects.
- **Plots:** prototype a managed graphics device that captures successive pages to private temporary files and reports completed images; integrate with default-device behavior only after verifying base graphics, `ggplot2` when installed, explicit `print`, user-created devices, errors, resizing, and cleanup. Do not promise universal graphics capture. Never pollute project directories.

R and user code have the user's OS permissions. The subprocess is for isolation and responsiveness, not security. See [quality and delivery](quality-and-delivery.md) for threat and licensing boundaries.
