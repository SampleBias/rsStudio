# rsStudio

rsStudio is a Linux-first, Rust-native IDE for R. Rust and egui/eframe provide the interface; an independently installed R runtime handles statistical computation. The working name is pending trademark clearance. This project is independently developed and is not affiliated with Posit or RStudio.

## Prototype

The prototype contains a desktop window and an R session supervisor. Its resizable workspace places Source and Console on the left, with Environment and Output on the right. The Source pane provides multiple UTF-8 script buffers, basic R syntax highlighting, line numbers, find, and file open/save. The Files tab browses a project directory separately from R’s working directory. Environment, data, and plots remain planned. The [development plan](docs/README.md), [phase 1 checklist](docs/phases/01-foundation.md), and [control protocol](docs/protocol.md) describe the intended behavior and remaining gates.

### Prerequisites and running

Install a Rust toolchain with Cargo, an independently installed R interpreter, and the development libraries required by eframe on your Linux distribution. Rust 1.99.1 is available in the current development environment; minimum supported Rust and R versions are not yet established. Network access to crates.io is required for the initial dependency download.

```sh
R --version
cargo run
```

The app searches `PATH` for `R`. To use a different installation, enter its executable path in the window and select **Apply & restart**. Set the R working directory the same way. If R is missing, install it separately and retry. The UI starts R with `--vanilla`, which disables user and site startup profiles and workspace restoration; this may affect project tools such as `renv`.

Enter a command in the console and select **Run** or press **Ctrl+Enter**. In Source, select code or place the cursor on a line and use **Run selection / line** or **Ctrl+Enter**; the cursor stays in place. Select a complete multiline expression before running it. Source and console commands share one R workspace until restart. An incomplete expression remains in the input for continuation. Restart discards the session state and stops a running calculation. This initial submission console is not a terminal: `readline()`, menus, debugging, packages needing an interactive R session, and standard input are not supported. The app does not sandbox R code.

### Working with scripts

Use **New**, **Open**, **Save**, and **Save as** in Source; open and save dialogs accept a path. **Ctrl+N**, **Ctrl+O**, **Ctrl+S**, and **Ctrl+Shift+S** provide shortcuts. An asterisk marks unsaved changes, and closing a dirty tab or window asks before discarding them. The editor accepts UTF-8 scripts up to 2 MiB; other encodings and larger files are rejected. Save As refuses to overwrite an existing path. If an open file changes externally, save is refused and you can choose a new path to preserve both versions. As with many local editors, an external write racing the final replacement cannot be ruled out.

The editor uses a small built-in lexer for common R syntax; it is not a complete R parser. Font size and light/dark theme are saved under `$XDG_CONFIG_HOME/rsstudio/settings`, or `~/.config/rsstudio/settings` when `XDG_CONFIG_HOME` is unset. Automatic Omarchy theme following is planned for a later phase.

### Development checks

```sh
cargo fmt --all -- --check
cargo clippy --workspace --all-targets -- -D warnings
cargo test --workspace
RSSTUDIO_REQUIRE_R=1 cargo test --test session_integration
```

The last command requires R and loopback TCP access; it fails when either is unavailable. Without that variable, tests that require unavailable R or loopback access print an explicit skip reason. Linux CI installs R and requires these tests. The supervisor binds only to the local loopback interface and starts R without a shell.

## License

Original project files are available under the [MIT License](LICENSE). R and third-party dependencies have their own licenses.
