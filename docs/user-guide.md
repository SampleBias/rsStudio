# rsStudio User Guide

rsStudio is a Linux-first, Rust-native IDE for R. Rust and egui/eframe provide the interface; an independently installed R runtime handles statistical computation. The working name is pending trademark clearance. This project is independently developed and is not affiliated with Posit or RStudio.

## Prototype

The prototype contains a desktop window and an R session supervisor. Its resizable workspace places Source and Console on the left, with Environment and Output on the right. The Source pane provides multiple UTF-8 script buffers, basic R syntax highlighting, line numbers, find, and file open/save. The Files tab browses a project directory separately from R’s working directory. The Packages tab installs and loads R packages; environment inspection, data viewing, and plots remain planned. The [development plan](README.md), [phase 1 checklist](phases/01-foundation.md), and [control protocol](protocol.md) describe the intended behavior and remaining gates.

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

### Installing and loading R packages

Open **Packages** in the lower-right workspace. The list includes common CRAN packages and Bioconductor packages such as DESeq2 and limma. Once R is ready, the list shows installed versions and whether each package is attached. Click **Install** to download a package and its required dependencies; progress and errors appear in the Console. Click **Load** to attach an installed package to the current R session. Loading is separate from installation. Use **Refresh** to recheck status.

To add another package, enter its name and select **Add**; choose **Bioconductor** first for a Bioconductor package. `stats` is bundled with R. CRAN packages use `https://cloud.r-project.org`; Bioconductor packages use `BiocManager`, which is installed from CRAN if needed. Installations use the first `R_LIBS_USER` directory, or an rsStudio user library if R does not define one. rsStudio creates the user library if necessary; it does not request administrator access. Large packages may take time and may need network access, compilers, or system libraries. Restarting R during installation can leave an incomplete install.

Installing a package makes it available to R, but does not yet provide rsStudio support for every package's documentation, code completion, graphics, or interactive viewer. See the official [R package installation documentation](https://stat.ethz.ch/R-manual/R-devel/library/utils/html/install.packages.html) and [Bioconductor installation guidance](https://www.bioconductor.org/install/).

### R coding assistant

Click the **R AI** tile on the right edge to show or hide the assistant panel. The panel stays open while a response is being generated and can be hidden once the request finishes. The **R assistant** panel connects to [OpenRouter's chat completions API](https://openrouter.ai/docs/api/api-reference/chat/send-chat-completion-request). Enter an OpenRouter API key and a model ID (the default is `openrouter/free`), then describe the R code you need. Requests are sent over HTTPS and may incur charges depending on the selected model and provider. The key is masked, kept in memory only, and cleared when the application exits; it is not saved with project preferences.

By default, requests contain the prompt and recent chat messages only. Check **Include current Source text with requests** to explicitly send the active script with that request. OpenRouter and its selected provider receive the submitted content, so avoid sharing sensitive information unless appropriate. The assistant has no filesystem, shell, or R-session tools. It does not execute code or modify files. Responses can contain errors or unsafe R code; review them before using **Append R to active Source** or **Append R to Console**, then run code yourself. R runs with your normal account permissions: the assistant's restricted capabilities do not sandbox R. Only complete R-labeled code blocks can be appended, and truncated responses cannot be appended.

### Development checks

```sh
cargo fmt --all -- --check
cargo clippy --workspace --all-targets -- -D warnings
cargo test --workspace
RSSTUDIO_REQUIRE_R=1 cargo test --test session_integration
```

The last command requires R and loopback TCP access; it fails when either is unavailable. Without that variable, tests that require unavailable R or loopback access print an explicit skip reason. Linux CI installs R and requires these tests. The supervisor binds only to the local loopback interface and starts R without a shell.

## License

Original project files are available under the [MIT License](../LICENSE). R and third-party dependencies have their own licenses.
