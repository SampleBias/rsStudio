# Phase 02 — Script authoring and files

**Status:** the script editor, files browser, execution routing, and basic light/dark themes are implemented. End-to-end GUI and real-R validation remain outstanding; see the root README for current limitations.

**Objective:** edit and execute R scripts in the persistent session. **Depends on:** phase 01 session and lifecycle gate.

## Work packages

1. Add a multi-buffer text editor for UTF-8 `.R` files: syntax highlighting, line numbers, undo/redo, selection, find, configurable font, and dirty indicators. Select a maintained highlighting strategy and check its license; define behavior for malformed encoding and large files.
2. Implement new/open/save/save-as and project-directory browsing. Surface read/write errors, detect external changes before overwriting, and prompt before discarding unsaved work. Confirm destructive file operations. Keep project navigation separate from the process working directory and show both.
3. Execute the selected text or current line through the existing request queue. Explicitly define line advancement and multiline continuation; do not silently run a whole file when there is no selection. Console and editor executions operate on the same R environment.
4. Introduce the [semantic theme system](../ux-and-themes.md), original dark and light themes, focus navigation, and persisted user preferences. Follow Omarchy may be integrated here or before the phase 04 release gate.

## Demo and exit criteria

- Create a script, save, close, and reopen it without content loss. Simulate permission errors and external modification; no silent overwrite occurs.
- Run a selection that creates `x`, evaluate `x` in the console, then run another selection using it. Errors and multiline code retain understandable locations and state.
- Verify highlighting for strings, comments, numbers, and R operators; verify keyboard navigation and light/dark theme readability. The application remains responsive during execution.
- Automated tests cover file state and execution routing; manually verify editing, unsaved prompts, keyboard actions, and high-DPI layout on target Linux.
