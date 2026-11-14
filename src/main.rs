mod agent;
mod console;
mod editor;
mod packages;
mod protocol;
mod session;
mod settings;
mod syntax;

use editor::Editor;
use eframe::egui::{self, Color32, RichText};
use session::{Event, Session};
use std::fs;
use std::path::PathBuf;
use std::time::Duration;

fn main() -> eframe::Result {
    eframe::run_native(
        "rsStudio",
        eframe::NativeOptions {
            viewport: egui::ViewportBuilder::default()
                .with_inner_size([1200.0, 800.0])
                .with_min_inner_size([720.0, 480.0]),
            ..Default::default()
        },
        Box::new(|cc| {
            let prefs = settings::Settings::load();
            prefs.apply(&cc.egui_ctx);
            Ok(Box::new(App::new(prefs, &cc.egui_ctx)))
        }),
    )
}

#[derive(Clone, Copy, PartialEq)]
enum State {
    Starting,
    Ready,
    Busy,
    Failed,
    Stopped,
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum PathAction {
    Open,
    SaveAs,
    Project,
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum OutputTab {
    Plots,
    Files,
    Packages,
}

enum PackageOperation {
    Query,
    Install(String),
    Load(String),
}

struct App {
    session: Option<Session>,
    assistant: agent::Agent,
    show_assistant: bool,
    crab_icon: egui::TextureHandle,
    editor: Editor,
    project: PathBuf,
    project_entries: Vec<(PathBuf, bool)>,
    project_error: Option<String>,
    path_action: Option<PathAction>,
    path_input: String,
    message: Option<String>,
    close_buffer: Option<u64>,
    confirm_exit: bool,
    allow_exit: bool,
    dark: bool,
    font_size: f32,
    console_fraction: f32,
    find_text: String,
    output_tab: OutputTab,
    packages: Vec<packages::Package>,
    package_status: std::collections::BTreeMap<String, packages::Status>,
    package_filter: String,
    package_name: String,
    package_bioc: bool,
    package_notice: String,
    pending_package: Option<PackageOperation>,
    refresh_packages: bool,
    submitted_from_source: bool,
    state: State,
    executable: String,
    directory: String,
    runtime: String,
    output: String,
    follow_console: bool,
    input: String,
    submitted: String,
    next_id: u64,
    pending_id: Option<u64>,
    confirm_restart: bool,
}
impl App {
    fn new(prefs: settings::Settings, ctx: &egui::Context) -> Self {
        let image = image::load_from_memory_with_format(
            include_bytes!("../assets/crab.png"),
            image::ImageFormat::Png,
        )
        .expect("bundled crab icon is a valid PNG")
        .into_rgba8();
        let color_image = egui::ColorImage::from_rgba_unmultiplied(
            [image.width() as usize, image.height() as usize],
            image.as_raw(),
        );
        let crab_icon = ctx.load_texture("crab_icon", color_image, egui::TextureOptions::LINEAR);
        let mut app = Self {
            session: None,
            assistant: agent::Agent::default(),
            show_assistant: true,
            crab_icon,
            editor: Editor::new(),
            project: std::env::current_dir().unwrap_or_else(|_| PathBuf::from(".")),
            project_entries: Vec::new(),
            project_error: None,
            path_action: None,
            path_input: String::new(),
            message: None,
            close_buffer: None,
            confirm_exit: false,
            allow_exit: false,
            dark: prefs.dark,
            font_size: prefs.font_size,
            console_fraction: prefs.console_fraction,
            find_text: String::new(),
            output_tab: OutputTab::Plots,
            packages: packages::catalog(),
            package_status: std::collections::BTreeMap::new(),
            package_filter: String::new(),
            package_name: String::new(),
            package_bioc: false,
            package_notice: String::new(),
            pending_package: None,
            refresh_packages: true,
            submitted_from_source: false,
            state: State::Stopped,
            executable: String::new(),
            directory: std::env::current_dir()
                .unwrap_or_else(|_| PathBuf::from("."))
                .display()
                .to_string(),
            runtime: String::new(),
            output: String::new(),
            follow_console: true,
            input: String::new(),
            submitted: String::new(),
            next_id: 1,
            pending_id: None,
            confirm_restart: false,
        };
        app.refresh_project();
        app.start();
        app
    }
    fn start(&mut self) {
        // Dropping the old receiver makes late events from that session unreachable.
        self.session.take();
        self.pending_id = None;
        self.pending_package = None;
        self.package_status.clear();
        self.package_notice.clear();
        self.refresh_packages = true;
        self.submitted.clear();
        self.runtime.clear();
        self.state = State::Starting;
        self.append("\n[Starting a new R session; previous R state is discarded.]\n");
        self.session = Some(Session::start(
            self.executable.clone(),
            PathBuf::from(&self.directory),
        ));
    }
    fn append(&mut self, text: &str) {
        const LIMIT: usize = 1024 * 1024;
        self.output.push_str(text);
        if !text.is_empty() {
            self.follow_console = true;
        }
        if self.output.len() > LIMIT {
            let mut cut = self.output.len() - LIMIT;
            while !self.output.is_char_boundary(cut) {
                cut += 1;
            }
            self.output.drain(..cut);
            self.output.insert_str(0, "[Earlier output truncated]\n");
        }
    }
    fn submit(&mut self) {
        let text = self.input.clone();
        if self.submit_text(text, false) {
            self.input.clear();
        }
    }
    fn submit_source(&mut self) {
        let text = self.editor.current().selected_or_line().to_owned();
        self.submit_text(text, true);
    }
    fn submit_text(&mut self, text: String, from_source: bool) -> bool {
        if self.state != State::Ready || text.trim().is_empty() {
            return false;
        }
        let id = self.next_id;
        self.next_id = self.next_id.wrapping_add(1).max(1);
        if let Some(session) = &self.session {
            match session.evaluate(id, text.clone()) {
                Ok(()) => {
                    self.append(&format!("\n> {text}\n"));
                    self.submitted = text;
                    self.submitted_from_source = from_source;
                    self.pending_id = Some(id);
                    self.state = State::Busy;
                    return true;
                }
                Err(error) => self.append(&format!("\n[Cannot run] {error}\n")),
            }
        }
        false
    }
    fn package_request(&mut self, operation: PackageOperation) {
        if self.state != State::Ready {
            return;
        }
        let id = self.next_id;
        self.next_id = self.next_id.wrapping_add(1).max(1);
        let Some(session) = &self.session else {
            return;
        };
        let result = match &operation {
            PackageOperation::Query => {
                let names: Vec<_> = self.packages.iter().map(|p| p.name.clone()).collect();
                session.package_status(id, &names)
            }
            PackageOperation::Install(name) => {
                let bioc = self
                    .packages
                    .iter()
                    .find(|p| p.name == *name)
                    .is_some_and(|p| p.repository == packages::Repository::Bioconductor);
                session.install_package(id, name, bioc)
            }
            PackageOperation::Load(name) => session.load_package(id, name),
        };
        match result {
            Ok(()) => {
                self.package_notice = match &operation {
                    PackageOperation::Query => "Checking packages…".into(),
                    PackageOperation::Install(name) => {
                        format!("Installing {name}… See Console for progress.")
                    }
                    PackageOperation::Load(name) => format!("Loading {name}…"),
                };
                if !matches!(operation, PackageOperation::Query) {
                    self.append(&format!("\n[Packages] {}\n", self.package_notice));
                }
                self.pending_id = Some(id);
                self.pending_package = Some(operation);
                self.refresh_packages = false;
                self.state = State::Busy;
            }
            Err(error) => {
                self.package_notice = error;
                self.refresh_packages = false;
            }
        }
    }
    fn packages_ui(&mut self, ui: &mut egui::Ui) {
        ui.horizontal(|ui| {
            ui.strong("R packages");
            if ui
                .add_enabled(self.state == State::Ready, egui::Button::new("Refresh"))
                .clicked()
            {
                self.package_request(PackageOperation::Query);
            }
        });
        ui.small(
            "Install from CRAN or Bioconductor into your R user library; load into this session.",
        );
        if self.state == State::Stopped || self.state == State::Failed {
            ui.weak("Start R to check or install packages.");
        }
        if !self.package_notice.is_empty() {
            ui.label(&self.package_notice);
        }
        ui.horizontal(|ui| {
            ui.label("Search:");
            ui.add(
                egui::TextEdit::singleline(&mut self.package_filter)
                    .hint_text("Filter packages")
                    .desired_width(130.0),
            );
        });
        ui.horizontal_wrapped(|ui| {
            ui.add(
                egui::TextEdit::singleline(&mut self.package_name)
                    .hint_text("Package name")
                    .desired_width(120.0),
            );
            ui.checkbox(&mut self.package_bioc, "Bioconductor");
            if ui
                .add_enabled(self.state == State::Ready, egui::Button::new("Add"))
                .clicked()
            {
                let name = self.package_name.trim();
                if !packages::valid_name(name) {
                    self.package_notice =
                        "Enter a valid R package name (letters, numbers, and dots).".into();
                } else if self.packages.iter().any(|p| p.name == name) {
                    self.package_filter = name.to_owned();
                    self.package_notice = "Package is already in the list.".into();
                } else if self.packages.len() >= 200 {
                    self.package_notice = "The package list is full.".into();
                } else {
                    self.packages.push(packages::Package {
                        name: name.to_owned(),
                        description: "Custom package",
                        repository: if self.package_bioc {
                            packages::Repository::Bioconductor
                        } else {
                            packages::Repository::Cran
                        },
                    });
                    self.package_filter = name.to_owned();
                    self.package_name.clear();
                    self.refresh_packages = true;
                }
            }
        });
        ui.separator();
        let filter = self.package_filter.to_lowercase();
        let entries = self.packages.clone();
        let mut action = None;
        egui::ScrollArea::vertical()
            .id_salt("package_list")
            .show(ui, |ui| {
                for package in entries.iter().filter(|p| {
                    p.name.to_lowercase().contains(&filter)
                        || p.description.to_lowercase().contains(&filter)
                }) {
                    ui.horizontal_wrapped(|ui| {
                        ui.strong(&package.name);
                        ui.weak(package.description);
                        if package.repository == packages::Repository::Bioconductor {
                            ui.small("Bioconductor");
                        }
                        match self.package_status.get(&package.name) {
                            Some(status) if status.loaded => {
                                ui.label(format!(
                                    "Loaded {}",
                                    status.version.as_deref().unwrap_or("")
                                ));
                            }
                            Some(status) if status.version.is_some() => {
                                ui.label(format!(
                                    "Installed {}",
                                    status.version.as_deref().unwrap_or("")
                                ));
                                if ui
                                    .add_enabled(
                                        self.state == State::Ready,
                                        egui::Button::new("Load"),
                                    )
                                    .clicked()
                                {
                                    action = Some(PackageOperation::Load(package.name.clone()));
                                }
                            }
                            Some(_) if package.repository == packages::Repository::Bundled => {
                                ui.weak("Bundled with R; unavailable in this installation");
                            }
                            Some(_) => {
                                ui.weak("Not installed");
                                if ui
                                    .add_enabled(
                                        self.state == State::Ready,
                                        egui::Button::new("Install"),
                                    )
                                    .clicked()
                                {
                                    action = Some(PackageOperation::Install(package.name.clone()));
                                }
                            }
                            None => {
                                ui.weak("Status unknown");
                            }
                        }
                    });
                    ui.separator();
                }
            });
        if let Some(operation) = action {
            self.package_request(operation);
        }
    }
    fn ask_path(&mut self, action: PathAction) {
        self.path_input = match action {
            PathAction::SaveAs => self
                .editor
                .current()
                .path
                .as_deref()
                .unwrap_or(&self.project)
                .display()
                .to_string(),
            _ => self.project.display().to_string(),
        };
        self.path_action = Some(action);
    }
    fn apply_path(&mut self, action: PathAction) {
        let path = PathBuf::from(self.path_input.trim());
        let path = if path.is_absolute() {
            path
        } else {
            self.project.join(path)
        };
        let result = match action {
            PathAction::Open => self.editor.open(path),
            PathAction::SaveAs => self.editor.current_mut().save(&path),
            PathAction::Project => std::fs::canonicalize(path).and_then(|p| {
                if p.is_dir() {
                    self.project = p;
                    self.refresh_project();
                    Ok(())
                } else {
                    Err(std::io::Error::new(
                        std::io::ErrorKind::InvalidInput,
                        "Not a directory",
                    ))
                }
            }),
        };
        match result {
            Ok(()) => self.path_action = None,
            Err(error) => self.message = Some(error.to_string()),
        }
    }
    fn refresh_project(&mut self) {
        match fs::read_dir(&self.project) {
            Ok(entries) => {
                self.project_error = None;
                self.project_entries = entries
                    .take(10_000)
                    .filter_map(Result::ok)
                    .filter_map(|entry| {
                        let kind = entry.file_type().ok()?;
                        if kind.is_file() || kind.is_dir() {
                            Some((entry.path(), kind.is_dir()))
                        } else {
                            None
                        }
                    })
                    .collect();
                self.project_entries
                    .sort_by(|a, b| a.0.file_name().cmp(&b.0.file_name()));
            }
            Err(error) => {
                self.project_entries.clear();
                self.project_error = Some(error.to_string());
            }
        }
    }
    fn save_current(&mut self) {
        if let Some(path) = self.editor.current().path.clone() {
            if let Err(error) = self.editor.current_mut().save(&path) {
                self.message = Some(error.to_string());
            }
        } else {
            self.ask_path(PathAction::SaveAs);
        }
    }
    fn poll(&mut self) {
        let mut events = Vec::new();
        if let Some(session) = &self.session {
            events.extend(session.poll());
        }
        for event in events {
            match event {
                Event::Starting => self.state = State::Starting,
                Event::Ready {
                    executable,
                    version,
                } => {
                    self.runtime = format!("{version} ({executable})");
                    self.state = State::Ready;
                }
                Event::Output(text) => self.append(&text),
                Event::Done(id) if self.pending_id == Some(id) => {
                    self.pending_id = None;
                    if let Some(operation) = self.pending_package.take() {
                        self.package_notice = match operation {
                            PackageOperation::Install(name) => {
                                format!("Installation finished: {name}. Refreshing status.")
                            }
                            PackageOperation::Load(name) => {
                                format!("Loaded {name}. Refreshing status.")
                            }
                            PackageOperation::Query => "Package check finished.".into(),
                        };
                        self.append(&format!("[Packages] {}\n", self.package_notice));
                    } else {
                        self.submitted.clear();
                    }
                    self.refresh_packages = true;
                    self.state = State::Ready;
                }
                Event::PackageStatus(id, payload)
                    if self.pending_id == Some(id)
                        && matches!(self.pending_package, Some(PackageOperation::Query)) =>
                {
                    self.pending_id = None;
                    self.pending_package = None;
                    self.state = State::Ready;
                    match packages::parse_status(&payload, &self.packages) {
                        Ok(status) => {
                            self.package_status = status;
                            self.package_notice = "Package status updated.".into();
                        }
                        Err(error) => {
                            self.package_status.clear();
                            self.package_notice = error;
                        }
                    }
                }
                Event::Incomplete(id, _message) if self.pending_id == Some(id) => {
                    self.pending_id = None;
                    if self.submitted_from_source {
                        self.append("[Incomplete expression: select the complete expression in Source and run again.]\n");
                    } else {
                        self.input = format!("{}\n", self.submitted);
                        self.append("[Incomplete expression: continue editing and run again.]\n");
                    }
                    self.submitted.clear();
                    self.state = State::Ready;
                }
                Event::Error(id, message) if self.pending_id == Some(id) => {
                    self.pending_id = None;
                    if self.pending_package.take().is_some() {
                        self.package_notice = format!("Package operation failed: {message}");
                        self.package_status.clear();
                        self.refresh_packages = false;
                    } else {
                        self.submitted.clear();
                        self.refresh_packages = true;
                    }
                    self.append(&format!("Error: {message}\n"));
                    self.state = State::Ready;
                }
                Event::Failed(message) => {
                    if self.pending_id.take().is_some()
                        && self.pending_package.is_none()
                        && !self.submitted_from_source
                        && self.input.is_empty()
                    {
                        self.input = std::mem::take(&mut self.submitted);
                    }
                    self.pending_package = None;
                    self.package_status.clear();
                    self.package_notice = format!("R session failed: {message}");
                    self.append(&format!("\n[Session error] {message}\n"));
                    self.state = State::Failed;
                }
                Event::Exited if self.state != State::Failed => {
                    self.state = State::Stopped;
                }
                _ => {}
            }
        }
    }
}
impl eframe::App for App {
    fn update(&mut self, ctx: &egui::Context, _frame: &mut eframe::Frame) {
        self.poll();
        if self.output_tab == OutputTab::Packages
            && self.refresh_packages
            && self.state == State::Ready
        {
            self.package_request(PackageOperation::Query);
        }
        self.assistant.poll();
        // Keep an in-flight request visible even if visibility was changed elsewhere.
        if self.assistant.busy() {
            self.show_assistant = true;
        }
        let previous_settings = (self.dark, self.font_size, self.console_fraction);
        if ctx.input(|i| i.viewport().close_requested())
            && self.editor.any_dirty()
            && !self.allow_exit
        {
            ctx.send_viewport_cmd(egui::ViewportCommand::CancelClose);
            self.confirm_exit = true;
        }
        if self.path_action.is_none()
            && self.message.is_none()
            && self.close_buffer.is_none()
            && !self.confirm_exit
            && !self.confirm_restart
        {
            if ctx.input(|i| i.modifiers.command && i.key_pressed(egui::Key::S)) {
                if ctx.input(|i| i.modifiers.shift) {
                    self.ask_path(PathAction::SaveAs);
                } else {
                    self.save_current();
                }
            } else if ctx.input(|i| i.modifiers.command && i.key_pressed(egui::Key::O)) {
                self.ask_path(PathAction::Open);
            } else if ctx.input(|i| i.modifiers.command && i.key_pressed(egui::Key::N)) {
                self.editor.new_buffer();
            }
        }
        ctx.request_repaint_after(Duration::from_millis(50));
        egui::TopBottomPanel::top("toolbar").show(ctx, |ui| {
            ui.horizontal_wrapped(|ui| {
                ui.add(
                    egui::Image::new((self.crab_icon.id(), egui::vec2(24.0, 24.0)))
                        .alt_text("crab"),
                );
                ui.heading("rsStudio");
                ui.separator();
                let (name, color) = match self.state {
                    State::Starting => (
                        "Starting",
                        if self.dark {
                            Color32::YELLOW
                        } else {
                            Color32::from_rgb(120, 80, 0)
                        },
                    ),
                    State::Ready => (
                        "Ready",
                        if self.dark {
                            Color32::LIGHT_GREEN
                        } else {
                            Color32::from_rgb(0, 105, 45)
                        },
                    ),
                    State::Busy => (
                        "Running",
                        if self.dark {
                            Color32::YELLOW
                        } else {
                            Color32::from_rgb(120, 80, 0)
                        },
                    ),
                    State::Failed => (
                        "Failed",
                        if self.dark {
                            Color32::LIGHT_RED
                        } else {
                            Color32::from_rgb(165, 30, 30)
                        },
                    ),
                    State::Stopped => (
                        "Stopped",
                        if self.dark {
                            Color32::GRAY
                        } else {
                            Color32::DARK_GRAY
                        },
                    ),
                };
                ui.label(RichText::new(format!("● {name}")).color(color));
                if !self.runtime.is_empty() {
                    ui.label(&self.runtime);
                }
                if ui.button("Restart R…").clicked() {
                    self.confirm_restart = true;
                }
                if ui.button("Clear output").clicked() {
                    self.output.clear();
                }
                if ui
                    .button(if self.dark {
                        "Light theme"
                    } else {
                        "Dark theme"
                    })
                    .clicked()
                {
                    self.dark = !self.dark;
                    settings::Settings {
                        dark: self.dark,
                        font_size: self.font_size,
                        console_fraction: self.console_fraction,
                    }
                    .apply(ctx);
                }
            });
            ui.horizontal(|ui| {
                ui.label("R executable:");
                ui.add(
                    egui::TextEdit::singleline(&mut self.executable)
                        .hint_text("R on PATH")
                        .desired_width(180.0),
                );
                ui.label("Working directory:");
                ui.add(egui::TextEdit::singleline(&mut self.directory).desired_width(260.0));
                if ui.button("Apply & restart…").clicked() {
                    self.confirm_restart = true;
                }
            });
        });
        // Keep the tile visible even when the assistant panel is collapsed.
        egui::SidePanel::right("assistant_tile")
            .resizable(false)
            .exact_width(50.0)
            .show(ctx, |ui| {
                ui.vertical_centered(|ui| {
                    let label = if self.show_assistant {
                        "Hide R-gentic"
                    } else {
                        "Show R-gentic"
                    };
                    let tile = ui.add_enabled(
                        !self.assistant.busy(),
                        egui::Button::selectable(
                            self.show_assistant,
                            RichText::new(if self.show_assistant { "›" } else { "‹" }).size(24.0),
                        )
                        .min_size(egui::vec2(32.0, 38.0)),
                    );
                    if tile
                        .on_hover_text(label)
                        .on_disabled_hover_text(
                            "The assistant stays open while generating a response.",
                        )
                        .clicked()
                    {
                        self.show_assistant = !self.show_assistant;
                    }
                    if self.assistant.busy() {
                        ui.spinner();
                    }
                });
            });

        // The assistant sits to the left of its tile, with the environment and
        // output panes to its left.
        if self.show_assistant {
            egui::SidePanel::right("coding_assistant")
                .resizable(true).default_width(340.0).min_width(250.0)
                .show(ctx, |ui| {
                    ui.heading("R-gentic Assist");
                    ui.weak("OpenRouter · R code suggestions");
                    ui.separator();
                    ui.label("API key (kept in memory for this session)");
                    ui.horizontal(|ui| {
                        ui.add(egui::TextEdit::singleline(&mut self.assistant.key)
                            .password(true).hint_text("OpenRouter API key").desired_width(175.0));
                        if ui.button("Clear key").clicked() { self.assistant.key.clear(); }
                    });
                    ui.horizontal(|ui| {
                        ui.label("Model:");
                        ui.add(egui::TextEdit::singleline(&mut self.assistant.model).desired_width(190.0));
                    });
                    ui.small("Requests are sent to OpenRouter and its model provider; model usage may incur charges.");
                    ui.checkbox(&mut self.assistant.include_source, "Include current Source text with requests");
                    ui.separator();
                    egui::TopBottomPanel::bottom("assistant_request").show_inside(ui, |ui| {
                        ui.label("Ask for R code");
                        ui.add_enabled(!self.assistant.busy(), egui::TextEdit::multiline(&mut self.assistant.prompt)
                            .desired_rows(3).desired_width(f32::INFINITY)
                            .hint_text("Describe the analysis or R code you need…"));
                        ui.horizontal(|ui| {
                            if ui.add_enabled(!self.assistant.busy(), egui::Button::new("Generate")).clicked() {
                                let source = self.assistant.include_source.then(|| self.editor.current().text.clone());
                                self.assistant.send(source.as_deref());
                            }
                            if self.assistant.busy() { ui.spinner(); ui.label("Generating…"); }
                            if ui.add_enabled(!self.assistant.busy(), egui::Button::new("Clear chat")).clicked() {
                                self.assistant.conversation.clear(); self.assistant.answer = None; self.assistant.error = None;
                            }
                        });
                        ui.small("Review code before running. R is not sandboxed.");
                    });
                    let mut destination = None;
                    if let Some(answer) = &self.assistant.answer {
                        ui.horizontal_wrapped(|ui| {
                            if ui.add_enabled(!answer.code.is_empty(), egui::Button::new("Append R to active Source")).clicked() { destination = Some(false); }
                            if ui.add_enabled(!answer.code.is_empty(), egui::Button::new("Append R to Console")).clicked() { destination = Some(true); }
                        });
                        if answer.truncated { ui.colored_label(Color32::LIGHT_RED, "Response was truncated; request a shorter answer."); }
                        else if answer.code.is_empty() { ui.weak("No complete R code block found; ask for code in an R block."); }
                    }
                    if let Some(error) = &self.assistant.error { ui.colored_label(Color32::LIGHT_RED, error); }
                    egui::ScrollArea::vertical().id_salt("assistant_output").stick_to_bottom(true).show(ui, |ui| {
                        for turn in &self.assistant.conversation {
                            ui.strong(if turn.role == "user" { "You" } else { "Assistant" });
                            // Source is only sent after explicit opt-in; avoid duplicating its full text in the chat view.
                            let content = if turn.role == "user" { turn.content.split("\n\nUser-provided R source (untrusted context;").next().unwrap_or(&turn.content) } else { &turn.content };
                            ui.label(content);
                            ui.separator();
                        }
                    });
                    if let Some(console) = destination && let Some(answer) = &self.assistant.answer {
                        if console {
                            if !self.input.is_empty() && !self.input.ends_with('\n') { self.input.push('\n'); }
                            self.input.push_str(&answer.code);
                        } else {
                            let text = &mut self.editor.current_mut().text;
                            if !text.is_empty() && !text.ends_with('\n') { text.push('\n'); }
                            text.push_str(&answer.code);
                        }
                    }
                });
        }
        // The side panel is created first so both right-hand panes extend from
        // the toolbar to the bottom, independently of the left-hand split.
        egui::SidePanel::right("workspace_right")
            .resizable(true)
            .default_width(430.0)
            .min_width(220.0)
            .show(ctx, |ui| {
                egui::TopBottomPanel::top("environment")
                    .resizable(true)
                    .default_height(280.0)
                    .min_height(100.0)
                    .show_inside(ui, |ui| {
                        ui.heading("Environment");
                        ui.separator();
                        ui.add_space(8.0);
                        ui.label("The session's variables will appear here.");
                        ui.weak("Environment inspection is planned for phase 3.");
                    });

                ui.horizontal(|ui| {
                    if ui
                        .selectable_label(self.output_tab == OutputTab::Plots, "Plots")
                        .clicked()
                    {
                        self.output_tab = OutputTab::Plots;
                    }
                    if ui
                        .selectable_label(self.output_tab == OutputTab::Files, "Files")
                        .clicked()
                    {
                        self.output_tab = OutputTab::Files;
                    }
                    if ui
                        .selectable_label(self.output_tab == OutputTab::Packages, "Packages")
                        .clicked()
                    {
                        self.output_tab = OutputTab::Packages;
                    }
                    ui.weak("Data");
                });
                ui.separator();
                if self.output_tab == OutputTab::Files {
                    ui.horizontal(|ui| {
                        ui.strong("Project files");
                        if ui.button("Choose…").clicked() {
                            self.ask_path(PathAction::Project);
                        }
                        if ui.button("Up").clicked()
                            && let Some(parent) = self.project.parent()
                        {
                            self.project = parent.to_owned();
                            self.refresh_project();
                        }
                        if ui.button("Refresh").clicked() {
                            self.refresh_project();
                        }
                    });
                    ui.small(self.project.display().to_string());
                    if let Some(error) = &self.project_error {
                        ui.label(format!("Cannot read directory: {error}"));
                    }
                    let entries = self.project_entries.clone();
                    egui::ScrollArea::vertical()
                        .id_salt("project_files")
                        .show(ui, |ui| {
                            for (path, is_dir) in entries.iter().take(300) {
                                let Some(name) = path.file_name() else {
                                    continue;
                                };
                                let name = name.to_string_lossy();
                                if *is_dir {
                                    if ui.button(format!("📁 {name}/")).clicked() {
                                        self.project = path.clone();
                                        self.refresh_project();
                                    }
                                } else if ui.button(name).clicked()
                                    && let Err(error) = self.editor.open(path.clone())
                                {
                                    self.message = Some(error.to_string());
                                }
                            }
                            if entries.len() > 300 {
                                ui.weak("Showing the first 300 entries.");
                            }
                        });
                } else if self.output_tab == OutputTab::Packages {
                    self.packages_ui(ui);
                } else {
                    ui.add_space(8.0);
                    ui.label("Plots will appear here in a later phase.");
                }
            });

        let workspace_height = ctx.available_rect().height();
        let console_panel = egui::TopBottomPanel::bottom("console")
            .resizable(true)
            .default_height(workspace_height * self.console_fraction)
            .min_height(220.0)
            .max_height((workspace_height - 140.0).max(220.0))
            .show(ctx, |ui| {
                let view = console::show(
                    ui,
                    &self.output,
                    &mut self.input,
                    self.state == State::Ready,
                    std::mem::take(&mut self.follow_console),
                );
                if view.run {
                    self.submit();
                }
            });
        if workspace_height > 0.0
            && ctx
                .read_response(egui::Id::new("console").with("__resize"))
                .is_some_and(|response| response.drag_stopped())
        {
            self.console_fraction =
                (console_panel.response.rect.height() / workspace_height).clamp(0.01, 0.99);
        }

        egui::CentralPanel::default().show(ctx, |ui| {
            ui.horizontal_wrapped(|ui| {
                ui.heading("Source");
                if ui.button("New").clicked() {
                    self.editor.new_buffer();
                }
                if ui.button("Open…").clicked() {
                    self.ask_path(PathAction::Open);
                }
                if ui.button("Save").clicked() {
                    self.save_current();
                }
                if ui.button("Save as…").clicked() {
                    self.ask_path(PathAction::SaveAs);
                }
                if ui.button("Close").clicked() {
                    if self.editor.current().dirty() {
                        self.close_buffer = Some(self.editor.current().id);
                    } else {
                        self.editor.close(self.editor.active);
                    }
                }
                if ui
                    .add_enabled(
                        self.state == State::Ready,
                        egui::Button::new("Run selection / line  Ctrl+Enter"),
                    )
                    .clicked()
                {
                    self.submit_source();
                }
            });
            ui.horizontal_wrapped(|ui| {
                for (index, buffer) in self.editor.buffers.iter().enumerate() {
                    let label = format!(
                        "{}{}",
                        buffer.name(),
                        if buffer.dirty() { " *" } else { "" }
                    );
                    if ui
                        .selectable_label(index == self.editor.active, label)
                        .clicked()
                    {
                        self.editor.active = index;
                    }
                }
            });
            ui.horizontal(|ui| {
                ui.label("Find:");
                ui.add(egui::TextEdit::singleline(&mut self.find_text).desired_width(130.0));
                let find = ui.button("Next").clicked();
                ui.label("Font:");
                ui.add(egui::Slider::new(&mut self.font_size, 10.0..=24.0).show_value(true));
                if find && !self.find_text.is_empty() {
                    let buffer = self.editor.current_mut();
                    let start_char = buffer.cursor.0.max(buffer.cursor.1);
                    let start_byte = buffer
                        .text
                        .char_indices()
                        .nth(start_char)
                        .map_or(buffer.text.len(), |(i, _)| i);
                    if let Some(byte) = buffer.text[start_byte..]
                        .find(&self.find_text)
                        .map(|i| i + start_byte)
                        .or_else(|| buffer.text.find(&self.find_text))
                    {
                        let first = buffer.text[..byte].chars().count();
                        let last = first + self.find_text.chars().count();
                        buffer.cursor = (first, last);
                        let id = egui::Id::new(("source_editor", buffer.id));
                        let mut state = egui::TextEdit::load_state(ctx, id).unwrap_or_default();
                        state
                            .cursor
                            .set_char_range(Some(egui::text::CCursorRange::two(
                                egui::text::CCursor::new(first),
                                egui::text::CCursor::new(last),
                            )));
                        state.store(ctx, id);
                        ctx.memory_mut(|m| m.request_focus(id));
                    }
                }
            });
            ui.separator();
            let dark = self.dark;
            let size = self.font_size;
            let buffer = self.editor.current_mut();
            let id = egui::Id::new(("source_editor", buffer.id));
            let mut run = false;
            egui::ScrollArea::both()
                .id_salt("source_scroll")
                .show(ui, |ui| {
                    ui.horizontal_top(|ui| {
                        let lines = buffer.text.lines().count()
                            + usize::from(buffer.text.is_empty() || buffer.text.ends_with('\n'));
                        let numbers = (1..=lines).map(|n| format!("{n}\n")).collect::<String>();
                        ui.label(
                            RichText::new(numbers)
                                .font(egui::FontId::monospace(size))
                                .weak(),
                        );
                        let mut layouter =
                            |ui: &egui::Ui, text: &dyn egui::TextBuffer, _wrap_width: f32| {
                                let mut job = syntax::highlight(text.as_str(), dark, size);
                                job.wrap.max_width = f32::INFINITY;
                                ui.fonts_mut(|fonts| fonts.layout_job(job))
                            };
                        let response = egui::TextEdit::multiline(&mut buffer.text)
                            .id(id)
                            .font(egui::FontId::monospace(size))
                            .code_editor()
                            .frame(false)
                            .desired_width(ui.available_width().max(300.0))
                            .desired_rows(16)
                            .layouter(&mut layouter)
                            .show(ui);
                        if let Some(range) = response.cursor_range {
                            buffer.cursor = (range.primary.index, range.secondary.index);
                        }
                        run = response.response.has_focus()
                            && ctx.input(|i| i.modifiers.ctrl && i.key_pressed(egui::Key::Enter));
                    });
                });
            if run {
                self.submit_source();
            }
        });
        if let Some(action) = self.path_action {
            let title = match action {
                PathAction::Open => "Open file",
                PathAction::SaveAs => "Save script as",
                PathAction::Project => "Choose project directory",
            };
            egui::Window::new(title)
                .collapsible(false)
                .resizable(false)
                .show(ctx, |ui| {
                    ui.label(
                        "Enter an absolute path, or a path relative to the project directory.",
                    );
                    ui.add(egui::TextEdit::singleline(&mut self.path_input).desired_width(400.0));
                    ui.horizontal(|ui| {
                        if ui.button("OK").clicked() {
                            self.apply_path(action);
                        }
                        if ui.button("Cancel").clicked() {
                            self.path_action = None;
                        }
                    });
                });
        }
        if let Some(id) = self.close_buffer
            && let Some(index) = self
                .editor
                .buffers
                .iter()
                .position(|buffer| buffer.id == id)
        {
            egui::Window::new("Discard unsaved changes?")
                .collapsible(false)
                .show(ctx, |ui| {
                    ui.label(format!(
                        "{} has unsaved changes. Save it first to keep them.",
                        self.editor.buffers[index].name()
                    ));
                    ui.horizontal(|ui| {
                        if ui.button("Discard and close").clicked() {
                            self.editor.close(index);
                            self.close_buffer = None;
                        }
                        if ui.button("Cancel").clicked() {
                            self.close_buffer = None;
                        }
                    });
                });
        }
        if self.confirm_exit {
            egui::Window::new("Quit with unsaved changes?")
                .collapsible(false)
                .show(ctx, |ui| {
                    ui.label(
                        "One or more scripts have unsaved changes. Save them first to keep them.",
                    );
                    ui.horizontal(|ui| {
                        if ui.button("Discard and quit").clicked() {
                            self.confirm_exit = false;
                            self.allow_exit = true;
                            ctx.send_viewport_cmd(egui::ViewportCommand::Close);
                        }
                        if ui.button("Cancel").clicked() {
                            self.confirm_exit = false;
                        }
                    });
                });
        }
        if let Some(message) = self.message.clone() {
            egui::Window::new("File operation")
                .collapsible(false)
                .show(ctx, |ui| {
                    ui.label(message);
                    if ui.button("OK").clicked() {
                        self.message = None;
                    }
                });
        }
        if previous_settings != (self.dark, self.font_size, self.console_fraction) {
            let prefs = settings::Settings {
                dark: self.dark,
                font_size: self.font_size,
                console_fraction: self.console_fraction,
            };
            if let Err(error) = prefs.save() {
                self.message = Some(format!("Could not save preferences: {error}"));
            }
        }
        if self.confirm_restart {
            egui::Window::new("Restart R?").collapsible(false).resizable(false).show(ctx, |ui| {
                ui.label("Restarting stops the current calculation and discards all variables in the R session.");
                ui.horizontal(|ui| {
                    if ui.button("Restart and discard session").clicked() { self.confirm_restart = false; self.start(); }
                    if ui.button("Cancel").clicked() { self.confirm_restart = false; }
                });
            });
        }
    }
}
