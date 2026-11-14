# Phase 04 — Linux MVP release

**Objective:** ship a usable, documented Linux MVP. **Depends on:** phases 01–03 and their gates.

## Work packages

1. Complete file browsing, data inspection, plot export, and theme settings; implement and validate optional Omarchy follow mode per [UX and themes](../ux-and-themes.md#theme-system). Persist settings using the desktop's configuration conventions with sensible defaults and no required Omarchy installation.
2. Close the console compatibility decision. Run the phase 01 submission-worker tests plus `interactive()`, `readline()`, a menu, `browser()`, long-running code, signals, and representative interactive packages. If these are unsupported, keep the limitation visible in product and release notes; if full console compatibility is required for product acceptance, replace or extend the transport and repeat the suite before release.
3. Define supported Linux distributions, graphics backends, R versions, and hardware test matrix based on results. Measure cold startup, UI responsiveness, and memory under agreed representative workloads; set and validate release thresholds.
4. Package the application without bundling R, provide install/uninstall and upgrade instructions, and verify R discovery on a clean machine. Include version, diagnostics, known limitations, and troubleshooting for missing R, permissions, themes, plots, and crashes.
5. Complete accessibility review, security and dependency review, license review and notices, working-name clearance, and release candidate sign-off. Record unresolved issues and owners.

## Demo and exit criteria

- Complete the [MVP acceptance scenario](../roadmap.md#mvp-acceptance-scenario) on each supported Linux configuration, including a clean install and missing-R recovery.
- Automated checks, real-R tests, theme and file error cases, platform smoke tests, and agreed performance thresholds pass. Validate an actual Wayland/Hyprland session and the other supported display paths.
- Verify imported theme replacement and fallback; inspect light and dark contrast and keyboard focus. Document any tested accessibility limitations.
- Confirm process cleanup and user-facing failure/recovery paths, then publish a release candidate only after license and trademark decisions are recorded.

**Release scope:** local R analysis and the six MVP features. No remote execution or multi-session promise.
