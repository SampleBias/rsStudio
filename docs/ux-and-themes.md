# UX and themes

## Workspace

Design a recognizable workflow for R users without reproducing another product's branding or visual assets. Use a resizable workspace with an editor, console, object inspector, and a tabbed area for plots/files/data; make the console and editor immediately discoverable. Persist pane sizes and selected tabs, offer keyboard focus navigation, and keep essential actions available without a mouse. Show R path/version, working directory, session state, busy status, and recovery actions. Use clear empty, loading, error, and unsaved-change states.

The [Posit pane guide](https://docs.posit.co/ide/user/ide/guide/ui/ui-panes.html) is a workflow reference for source, console, environment, files, and plots. Layout details are design inputs, not source material or a requirement to clone the interface.

## Theme system

Ship at least one original dark and one original light palette; offer explicit selection and an optional **Follow Omarchy** mode. Apply semantic tokens consistently to window/panel backgrounds, text, muted text, borders, focus, selection, errors, warnings, syntax categories, and plot/data surfaces. Keep editor and console fonts configurable. The application remains usable outside Omarchy and never changes OS theme files.

Omarchy documents a `colors.toml` palette and an active staged theme under `~/.local/state/omarchy/current/theme`; see its [theming guide](https://github.com/omacom/omarchy/blob/quattro/docs/theming.md) and [theme manual](https://github.com/omacom/omarchy/blob/quattro/manual/43-making-your-own-theme.md). In Follow Omarchy mode, read the active `colors.toml` as untrusted data and map `mode`, `background`, `foreground`, `accent`, `selection`, `muted`, and applicable named colors to IDE tokens. No scripts, templates, arbitrary config, or external commands are executed. Support the documented current path as an integration point, not an immutable API; validate it against supported Omarchy releases. Derive absent optional roles from safe fallback tokens; if the file is absent or invalid, retain the last valid theme or use the built-in default and show a nonblocking status. Manual selection always overrides follow mode.

For live switching, watch the parent directory as well as the file, debounce updates, reopen after atomic directory replacement, and retry transient reads. Apply only fully parsed, validated palettes. Test absent, malformed, partial, light/dark, switched, and non-Omarchy cases. Do not assume third-party palette keys or redistribute their artwork without license review.

## Usability and accessibility

Aim for at least 4.5:1 contrast for normal text and 3:1 for meaningful UI indicators; validate each bundled palette and offer a readable fallback for imported palettes that fail. Do not use color as the only status signal. Include visible keyboard focus, scalable text, high-DPI testing, and keyboard operation for core flows. Evaluate egui's actual accessibility support on the target Linux desktop early; document gaps and avoid promising screen-reader support that has not been verified.
