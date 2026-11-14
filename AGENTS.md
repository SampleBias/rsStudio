# Repository Guidelines

## Project Structure & Module Organization

`src/` contains the Rust desktop application, `r/` the R session worker, `tests/` integration tests, `assets/` bundled artwork, `examples/` sample R scripts, and `docs/` project documentation. `vendor/` contains the patched egui-winit integration; see its patch notes before updating it. Avoid committing generated output, dependency caches, or local environment files.

## Build, Test, and Development Commands

Install Rust, Cargo, R, and the Linux libraries required by eframe. From the repository root, use `cargo run` to launch the app, `cargo fmt --all -- --check` to check formatting, `cargo clippy --workspace --all-targets -- -D warnings` for linting, and `cargo test --workspace` for tests. Run `RSSTUDIO_REQUIRE_R=1 cargo test --test session_integration` to require R and loopback access in integration tests. On Wayland, image paste also requires `wl-paste` from `wl-clipboard`.

## Coding Style & Naming Conventions

Follow the conventions of the language and framework selected for the project. Use consistent indentation and descriptive names; keep files and modules focused on a single responsibility. If a formatter or linter is added, commit its configuration and document how to run it. Apply it to changed files before submitting a contribution.

## Testing Guidelines

Add Rust tests alongside meaningful behavior changes, using the established unit and integration test conventions. The integration suite can skip tests when R or loopback access is unavailable unless `RSSTUDIO_REQUIRE_R=1` is set. No coverage target is configured.

## Commit & Pull Request Guidelines

No Git history is available here to establish a commit message convention. Write concise, imperative commit subjects that describe the change. For pull requests, explain the purpose and behavior of the change, link any relevant issue, and list the checks performed. Include screenshots when a change affects the user interface.

## Security & Configuration

Keep credentials and machine-specific settings out of version control. Document required configuration variables and provide a sanitized example when configuration is introduced.
