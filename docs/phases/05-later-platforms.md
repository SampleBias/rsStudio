# Phase 05 — Windows and macOS

**Objective:** extend the verified desktop experience after the Linux MVP. **Depends on:** phase 04 and platform-specific ownership and test infrastructure.

## Work packages

1. Define supported OS versions and minimum R versions. Research and implement native runtime discovery and user-selected paths; test multiple installations, spaces, Unicode, permissions, and invalid configurations. Do not assume Linux `PATH` or signal behavior applies.
2. Port the child supervisor and control channel; design platform-specific interruption, termination, process-tree cleanup, and graceful shutdown. Test R console compatibility on each OS before advertising it. Keep the same versioned, platform-neutral application/session API where practical.
3. Test native file dialogs, path encoding, configuration locations, display scaling, keyboard conventions, accessibility, graphics devices, and fonts. Omarchy integration remains optional and Linux-specific; bundled themes work everywhere.
4. Choose packaging and signing/notarization approaches appropriate to each platform. Document separate R installation, upgrades, permissions, and uninstall behavior. Revisit license obligations before any proposal to bundle R.
5. Add native CI jobs and real-hardware smoke tests for supported versions. Do not treat successful cross-compilation as evidence of a working desktop application.

## Exit criteria

On each supported OS, install the application and independently installed R, complete the [MVP acceptance scenario](../roadmap.md#mvp-acceptance-scenario), and pass the real-R integration and lifecycle tests. Publish the platform matrix, known limitations, installation and recovery instructions, and signing details. Release each platform independently when its own gate is met.
