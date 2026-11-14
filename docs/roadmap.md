# Roadmap and delivery gates

## Product boundary

Build a lightweight desktop IDE for local R analysis: scripts, an R console, data-frame inspection, plots, variables, and files. Support an independently installed R runtime first. Linux is the only MVP target; Windows and macOS are later ports. Offline use is the default. No cloud account is required.

## Sequence

| Phase | Outcome | Depends on | Gate |
| --- | --- | --- | --- |
| 01 — Foundation | GUI and persistent R session | Rust toolchain and local R | Assign a variable in one command and retrieve it in another; restart cleanly. |
| 02 — Authoring | Useful editing and project files | 01 | Edit, save, reopen, and run a selection in the same session. |
| 03 — Inspect and plot | Data, environment, and plots | 01 and 02 | Explore a data frame and display a plot produced by that session. |
| 04 — MVP release | Stable, installable Linux build | 01–03 | End-to-end release checklist and documented installation pass. |
| 05 — Later platforms | Windows/macOS support | 04 | Platform-specific discovery, process handling, packaging, and validation pass. |

Parallel work can begin on themes and editor prototypes during phase 01, but integration must respect the runtime interface and gate dependencies. Avoid building the full IDE shell before the first end-to-end demo works.

## MVP acceptance scenario

On a clean supported Linux machine, install rsStudio and R using documented steps. Open a project directory, create and save an R script, run a selection to create a data frame, inspect its values in a paged viewer, and render a plot. Run another console command to verify the same session state. Browse and reopen a file, switch between light and dark themes, restart R, and verify the old variable is gone. Failure to find R must produce an actionable setup screen rather than a crash.

## Explicitly deferred

Remote R, multiple simultaneous sessions, notebooks/Quarto, debugging, language-server intelligence, package installation UI, version-control integration, database connections, and R runtime auto-download are outside the MVP. A minimal editor is not expected to match every mature IDE feature. Full terminal-style input and advanced R interaction require the feasibility decision in [architecture](architecture.md#console-compatibility-boundary).

## Decisions to close

- Apply the MIT License to original project code and clear the working name before release.
- Pick supported Linux distributions, minimum R/Rust versions, and eframe renderer after testing real hardware; record the resulting matrix.
- Approve the console interaction scope after the phase 01 prototype; do not claim full R console compatibility until it passes the documented interaction tests.
- Review dependency licenses and any R redistribution obligations before selecting a distribution model.
