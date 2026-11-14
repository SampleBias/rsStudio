# Phase 01 — GUI and persistent R session

**Status:** prototype implementation started; the GUI, live R worker, and phase gate are not yet verified. See the root README for the environment limitations.

**Objective:** demonstrate the first milestone: a responsive Rust desktop window connected to one persistent R process. **Prerequisites:** Rust toolchain and a locally installed R; document the versions actually tested.

## Work packages

1. Create a minimal Cargo application with egui/eframe, version-controlled formatting/lint configuration, and a Linux development README. Choose a renderer after testing the target compositor. Show a console input, output area, session status, restart, and an R path selection/error state.
2. Implement R discovery, version validation, project working-directory selection, and a child supervisor as specified in [architecture](../architecture.md#runtime-discovery-and-startup). Launch without a shell; no R install or download is performed by the app.
3. Prototype the base-R helper and dedicated framed control channel. Record the concrete protocol and prove its handshake, limits, source encoding, completion and error semantics, and concurrency behavior before depending on it. Preserve a single `.GlobalEnv` between submissions.
4. Connect asynchronous output and events to the UI. Support visible expression results, errors, incomplete multiline input, a busy indicator, and subsequent commands after recoverable errors. Keep the UI responsive while R runs. Clearly label the initial submission-console limitations.
5. Add process-exit detection, stop/restart with state-loss notice, graceful close with bounded cleanup, and a new session generation on restart. Verify signal-based interrupt before exposing it as working; otherwise use stop/restart.
6. Add tests and a short local setup/troubleshooting guide. Establish the minimum supported R and Rust versions from actual testing.

## Demo and exit criteria

- Launch the app on supported Linux with R available; display the detected executable and version. Without R, show an actionable error and allow retry.
- Submit `x <- 41`, then `x + 1` in a separate submission; display `42` from the same process. A multiline function definition works, and an error is followed by a successful command.
- Output containing `>` or protocol-like text cannot complete or corrupt another request; stdout/stderr remain visible under sustained output. UI remains responsive during `Sys.sleep(5)`.
- Restart clears `x`, stale output cannot update the new session, and quitting leaves no managed R child behind. Test an unexpected child exit and startup failure.
- Run the applicable checks in [quality](../quality-and-delivery.md#verification-strategy), including a real-R integration job. Document limitations of `readline()`, `interactive()`, interactive packages, and interruption.

**Out of scope:** full editor, plotting, data viewer, automatic R installation, and claims of full terminal-compatible R behavior.
