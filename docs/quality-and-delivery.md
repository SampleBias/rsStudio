# Quality, security, and delivery

## Verification strategy

Once a Cargo project exists, CI should run `cargo fmt --all -- --check`, `cargo clippy --workspace --all-targets -- -D warnings`, and `cargo test --workspace`. Add a separate integration job that installs a documented supported R version and fails if R is absent; local tests may explicitly skip R-dependent cases with a reason. Pin or otherwise control CI toolchain versions. No coverage percentage is mandated before the team establishes a meaningful baseline.

Test the protocol with a fake child for malformed frames, partial reads, floods, timeouts, unexpected exit, and stale responses. Test real R for persistent state, errors and recovery, multiline submissions, output on both streams, Unicode, long calculations, restart, and shutdown. Use isolated temporary project directories and controlled startup environment. Exercise the GUI manually on Linux, including Wayland/Hyprland and X11 or XWayland where supported, keyboard navigation, DPI, theme changes, file errors, and an absent R installation. Do not depend exclusively on snapshot tests for interactive behavior.

## Runtime and data safety

R executes arbitrary code with the user's permissions; clearly communicate this when opening or running untrusted projects. Never treat the R subprocess, project directory, or helper as a security sandbox. Avoid automatically running project code; document startup profile behavior. Use argument arrays and typed protocol fields, validate frame lengths and paths, create private temporary directories, avoid following unexpected symlinks for destructive operations, and remove temporary plots on normal shutdown. Confirm file deletion and preserve unsaved buffers; write saves safely and handle external modifications and failures. Avoid collecting code, data, environment variables, or control secrets in logs.

## Packaging and licenses

For MVP, document installation of a supported system R separately; detect it at runtime. Do not silently download or bundle R. Review transitive Rust dependencies, fonts, icons, theme assets, and R helper dependencies before shipping. R is GPL-licensed; subprocess use does not by itself settle every distribution or licensing question. Obtain a qualified review before bundling, linking, or redistributing R, and include required notices and source offers as applicable. Original project work is MIT-licensed; track the working name and trademark review separately. No trademark clearance is assumed by this plan.

## Release gate

A release candidate must pass the [MVP acceptance scenario](roadmap.md#mvp-acceptance-scenario), automated CI, supported-platform smoke tests, and dependency/license review. Publish installation, R discovery, troubleshooting, known console/graphics limitations, update and uninstall instructions, and a reproducible build description. Record supported versions and unresolved defects. Choose concrete startup/memory targets and hardware after measuring a prototype; document methodology and block release on agreed regressions rather than inventing unmeasured promises.
