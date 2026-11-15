# rsStudio User Guide

rsStudio is a Linux-first, Rust-native IDE for R. Rust and egui/eframe provide the interface; an independently installed R runtime handles statistical computation. The working name is pending trademark clearance. This project is independently developed and is not affiliated with Posit or RStudio.

## Prototype

The prototype contains a desktop window and an R session supervisor. Its resizable workspace places Source and Console on the left, with Environment and Output on the right. The Source pane provides multiple UTF-8 script buffers, basic R syntax highlighting, line numbers, find, and file open/save. The Files tab browses a project directory separately from R’s working directory. The Packages tab installs and loads R packages; data viewing remains planned. The [development plan](README.md), [phase 1 checklist](phases/01-foundation.md), and [control protocol](protocol.md) describe the intended behavior and remaining gates.

### Plots

Run plotting code such as `plot(mtcars$mpg, mtcars$wt)` in the R console or Source; the result appears in the **Plots** tab. The arrows navigate the last 30 plot pages, **Zoom** opens a larger view, **Export PNG…** saves the selected plot to a new `.png` file, and **Remove** or **Clear** removes images from the pane. The export dialog will not overwrite an existing file. History is cleared when R is restarted.

The pane uses a 960 × 720 PNG graphics device and scales images to fit the available space. It supports standard R graphics and grid-based plots, including ggplot2. Commands such as `points()` and `lines()` update the current page. Explicitly opened devices such as `png()` or `pdf()` keep their own output instead of appearing in the pane. Plotting requires an R build with PNG support and Cairo or a working graphical display. Interactive graphics and vector/PDF export are not yet supported.

### Prerequisites and running

Install a Rust toolchain with Cargo, an independently installed R interpreter, and the development libraries required by eframe on your Linux distribution. Rust 1.99.1 is available in the current development environment; minimum supported Rust and R versions are not yet established. Network access to crates.io is required for the initial dependency download.

```sh
R --version
cargo run
```

The app searches `PATH` for `R`. To use a different installation, enter its executable path in the window and select **Apply & restart**. Set the R working directory the same way. If R is missing, install it separately and retry. The UI starts R with `--vanilla`, which disables user and site startup profiles and workspace restoration; this may affect project tools such as `renv`.

Type or paste a command in the **R input** area at the bottom of the console and select **Run** or press **Ctrl+Enter**. Drag the divider above the Console to resize it; the position is saved for the next launch. Drag the divider above R input to resize the input area. New output scrolls into view automatically, and **Latest output** returns to the end after scrolling up. In Source, select code or place the cursor on a line and use **Run selection / line** or **Ctrl+Enter**; the cursor stays in place. Select a complete multiline expression before running it. Source and console commands share one R workspace until restart. An incomplete expression remains in the input for continuation. Restart discards the session state and stops a running calculation. This initial submission console is not a terminal: `readline()`, menus, debugging, packages needing an interactive R session, and standard input are not supported. The app does not sandbox R code.

Default R warnings are forwarded to the console during evaluation. Explicit `options(warn = -1)`, `warn = 1`, `warn = 2`, and custom warning handlers keep their normal behavior. Console output is retained up to 1 MiB, with an explicit truncation marker when older output is removed. Restarting or closing the IDE terminates the owned R process group on Linux, including child processes still in that group.

### Working with scripts

Use **New**, **Open**, **Save**, and **Save as** in Source; open and save dialogs accept a path. **Ctrl+N**, **Ctrl+O**, **Ctrl+S**, and **Ctrl+Shift+S** provide shortcuts. An asterisk marks unsaved changes, and closing a dirty tab or window asks before discarding them. The editor accepts UTF-8 scripts up to 2 MiB; other encodings and larger files are rejected. Save As refuses to overwrite an existing path. If an open file changes externally, save is refused and you can choose a new path to preserve both versions. As with many local editors, an external write racing the final replacement cannot be ruled out.

File opening, saving, and directory listing run on a bounded background worker. The toolbar shows progress while a file operation is pending; you can keep editing and using the console. A save writes the text as it was when you clicked Save; later edits remain marked as unsaved. Non-regular files such as FIFOs are rejected. Quitting waits for pending file writes and R shutdown while the window continues processing events.

The editor uses a small built-in lexer for common R syntax; it is not a complete R parser. Font size and light/dark theme are saved under `$XDG_CONFIG_HOME/rsstudio/settings`, or `~/.config/rsstudio/settings` when `XDG_CONFIG_HOME` is unset. Preference writes are debounced while you adjust controls and flushed before quitting. Automatic Omarchy theme following is planned for a later phase.

### Installing and loading R packages

Open **Packages** in the lower-right workspace. The default list includes BiocManager, BiocVersion, DESeq2, and limma alongside common CRAN packages. BiocVersion records the Bioconductor release and is normally installed automatically by BiocManager. Once R is ready, the list shows installed versions and whether each package is attached. Click **Install** to download a package and its required dependencies; progress and errors appear in the Console. Click **Load** to attach an installed package to the current R session. Loading is separate from installation. Use **Refresh** to recheck status.

To add another package, enter its name and select **Add**; choose **Bioconductor** first for a Bioconductor package. `stats` is bundled with R. CRAN packages use `https://cloud.r-project.org`; Bioconductor packages use `BiocManager`, which is installed from CRAN if needed. Installations use the first `R_LIBS_USER` directory, or an rsStudio user library if R does not define one. rsStudio creates the user library if necessary; it does not request administrator access. Large packages may take time and may need network access, compilers, or system libraries. Restarting R during installation can leave an incomplete install.

Installing a package makes it available to R, but does not yet provide rsStudio support for every package's documentation, code completion, graphics, or interactive viewer. See the official [R package installation documentation](https://stat.ethz.ch/R-manual/R-devel/library/utils/html/install.packages.html) and [Bioconductor installation guidance](https://www.bioconductor.org/install/).

### R-gentic Assist

Click the arrow on the right edge to show or hide R-gentic Assist. The panel stays open while a response is being generated and can be hidden once the request finishes. The **R-gentic Assist** panel connects to [OpenRouter's chat completions API](https://openrouter.ai/docs/api/api-reference/chat/send-chat-completion-request). Enter an OpenRouter API key, then describe the R code or analysis you need. The editable model ID defaults to `thinkingmachines/inkling-2-small`, a paid model; requests may incur charges. The key is masked, kept in memory only, and cleared when the application exits; it is not saved with project preferences.

By default, requests contain the prompt and recent chat messages only. Check **Include current Source text with requests** to send the active script. To load data, use **Import CSV/TSV…** in Environment, choose a file path and an R object name, or create objects in the R console. Import refuses to overwrite an existing object. Environment refreshes after R commands; use **Refresh** if necessary. Check the objects you want the assistant to inspect. If the model calls its inspection tool, the selected objects' names, classes, column types, sample missing-value counts, and up to five example values per column are sent to OpenRouter and its provider. Summaries cover up to 20 objects and 20 columns per object; missing counts cover the first 10,000 values, and examples are clipped to 60 characters. The tool does not send the full dataset or execute R code.

To attach an image, focus the chat input and paste it with Ctrl+V. The attached image is sent with the next request (up to 4 MiB); use **Remove** to discard it before sending. On Wayland, image paste uses `wl-paste` from `wl-clipboard` and expects a PNG clipboard image. The panel reports when the environment tool is used. Selected contents, prompts and conversation are sent to OpenRouter and its provider; choose a provider and privacy policy appropriate for your data.

The model can inspect only selected environment summaries through this tool; it cannot browse arbitrary files, access a shell, or run R. Complete R code blocks have **Copy**, **Append to Source**, and **Append to Console** controls. The append buttons do not run code. Truncated answers cannot be copied or appended using these controls. Review code before running it: R runs with your normal account permissions, and the assistant does not sandbox it.

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
