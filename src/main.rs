mod agent;
mod clipboard_image;
mod completion;
mod console;
mod cran;
mod editor;
mod file_worker;
mod packages;
mod plots;
mod protocol;
mod session;
mod settings;
mod syntax;

use editor::Editor;
use eframe::egui::{self, Color32, RichText};
use session::{Event, Session};
use std::path::PathBuf;
use std::time::{Duration, Instant};

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
    ImportData,
    ExportPlot,
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

fn import_command(path: &std::path::Path, name: &str) -> Result<String, String> {
    if name.trim().is_empty() || name.len() > 100 || name.contains('\0') {
        return Err("Enter an object name between 1 and 100 bytes.".into());
    }
    let reader = match path
        .extension()
        .and_then(|s| s.to_str())
        .map(str::to_ascii_lowercase)
        .as_deref()
    {
        Some("csv") => "read.csv",
        Some("tsv") => "read.delim",
        _ => return Err("Choose a CSV or TSV file.".into()),
    };
    let path = path.to_str().ok_or("The file path must be UTF-8.")?;
    if path.contains('\0') {
        return Err("Invalid file path.".into());
    }
    let path = serde_json::to_string(path).map_err(|e| e.to_string())?;
    let name = serde_json::to_string(name).map_err(|e| e.to_string())?;
    Ok(format!(
        "if (exists({name}, envir = .GlobalEnv, inherits = FALSE)) stop(\"Object already exists; choose another name\") else assign({name}, utils::{reader}({path}, check.names = FALSE), envir = .GlobalEnv)"
    ))
}

fn inject_completion(ctx: &egui::Context, id: egui::Id, query: &completion::Query, text: String) {
    let mut state = egui::TextEdit::load_state(ctx, id).unwrap_or_default();
    state
        .cursor
        .set_char_range(Some(egui::text::CCursorRange::two(
            egui::text::CCursor::new(query.start),
            egui::text::CCursor::new(query.end),
        )));
    state.store(ctx, id);
    ctx.input_mut(|input| input.events.push(egui::Event::Paste(text)));
    ctx.memory_mut(|memory| memory.request_focus(id));
}

fn completion_keys(
    ctx: &egui::Context,
    completion: &mut completion::Completion,
    can_wait: bool,
) -> Option<(completion::Query, String)> {
    if ctx.input(|input| input.modifiers != egui::Modifiers::NONE) {
        return None;
    }
    let key = |key| ctx.input_mut(|input| input.consume_key(egui::Modifiers::NONE, key));
    if completion.visible {
        if key(egui::Key::Escape) {
            completion.dismiss();
        } else if key(egui::Key::ArrowDown) {
            completion.move_selection(1);
        } else if key(egui::Key::ArrowUp) {
            completion.move_selection(-1);
        } else if key(egui::Key::Tab) || key(egui::Key::Enter) {
            return completion.insertion();
        }
    } else if completion.manual_pending() && key(egui::Key::Escape) {
        completion.dismiss();
    } else if !completion.results.is_empty() && key(egui::Key::Tab) {
        completion.open();
        if completion.results.len() == 1 {
            return completion.insertion();
        }
    } else if can_wait && completion.awaiting_lookup() && key(egui::Key::Tab) {
        completion.request_now(Instant::now());
    }
    None
}

/// A character and Tab can arrive in the same UI frame. Let the editor apply
/// the character first, then complete against the updated cursor and prefix.
fn coalesced_completion_tab(ctx: &egui::Context) -> bool {
    ctx.input(|input| {
        if input.modifiers != egui::Modifiers::NONE {
            return false;
        }
        let mut typed = false;
        let mut tab = false;
        for event in &input.events {
            match event {
                egui::Event::Text(text) | egui::Event::Ime(egui::ImeEvent::Commit(text)) => {
                    if tab
                        || text.is_empty()
                        || !text
                            .chars()
                            .all(|c| c.is_alphanumeric() || c == '_' || c == '.')
                    {
                        return false;
                    }
                    typed = true;
                }
                egui::Event::Key {
                    key: egui::Key::Tab,
                    pressed: true,
                    modifiers,
                    ..
                } if *modifiers == egui::Modifiers::NONE => tab = true,
                egui::Event::Ime(egui::ImeEvent::Preedit(_))
                | egui::Event::Cut
                | egui::Event::Paste(_) => return false,
                egui::Event::Key {
                    key:
                        egui::Key::Backspace
                        | egui::Key::Delete
                        | egui::Key::ArrowLeft
                        | egui::Key::ArrowRight
                        | egui::Key::Home
                        | egui::Key::End
                        | egui::Key::PageUp
                        | egui::Key::PageDown,
                    pressed: true,
                    ..
                } => {
                    return false;
                }
                _ => {}
            }
        }
        typed && tab
    })
}

struct App {
    session: Option<Session>,
    assistant: agent::Agent,
    plots: plots::Plots,
    paste_receiver: Option<std::sync::mpsc::Receiver<Result<Option<Vec<u8>>, String>>>,
    environment: Vec<serde_json::Value>,
    selected_environment: std::collections::BTreeSet<String>,
    refresh_environment: bool,
    pending_environment: bool,
    environment_notice: String,
    import_name: String,
    show_assistant: bool,
    crab_icon: egui::TextureHandle,
    editor: Editor,
    files: file_worker::Worker,
    file_busy: bool,
    preferences_changed: Option<Instant>,
    preferences_pending: bool,
    deferred_exit: bool,
    source_cache: syntax::SourceCache,
    completion: completion::Completion,
    pending_completion: Option<(u64, completion::Key)>,
    completion_insert: Option<(completion::Query, String)>,
    output_pending: String,
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
    package_info: cran::Lookup,
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
            plots: plots::Plots::default(),
            paste_receiver: None,
            environment: Vec::new(),
            selected_environment: std::collections::BTreeSet::new(),
            refresh_environment: true,
            pending_environment: false,
            environment_notice: String::new(),
            import_name: "data".into(),
            show_assistant: true,
            crab_icon,
            editor: Editor::new(),
            files: file_worker::Worker::new(),
            file_busy: false,
            preferences_changed: None,
            preferences_pending: false,
            deferred_exit: false,
            source_cache: syntax::SourceCache::default(),
            completion: completion::Completion::default(),
            pending_completion: None,
            completion_insert: None,
            output_pending: String::new(),
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
            package_info: cran::Lookup::default(),
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
        self.plots.clear();
        self.pending_id = None;
        self.pending_completion = None;
        self.completion.clear();
        self.completion_insert = None;
        self.pending_package = None;
        self.pending_environment = false;
        self.refresh_environment = true;
        self.environment.clear();
        self.selected_environment.clear();
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
        self.output_pending.push_str(text);
        if !text.is_empty() {
            self.follow_console = true;
        }
    }
    fn flush_output(&mut self) {
        console::append_batch(&mut self.output, &self.output_pending);
        self.output_pending.clear();
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
        if let PackageOperation::Install(name) = &operation
            && self.package_info.get(name).is_some()
        {
            // Retry a failed manual-package lookup when installation is retried;
            // successful and in-flight lookups are deduplicated by the cache.
            self.package_info.request(name);
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
    fn add_package(&mut self) {
        let name = self.package_name.trim().to_owned();
        if !packages::valid_name(&name) {
            self.package_notice =
                "Enter a valid R package name (letters, numbers, and dots).".into();
        } else if self.packages.iter().any(|p| p.name == name) {
            self.package_filter = name;
            self.package_notice = "Package is already in the list.".into();
        } else if self.packages.len() >= 200 {
            self.package_notice = "The package list is full.".into();
        } else {
            self.packages.push(packages::Package {
                name: name.clone(),
                description: if self.package_bioc {
                    "Bioconductor package"
                } else {
                    "CRAN package"
                },
                repository: if self.package_bioc {
                    packages::Repository::Bioconductor
                } else {
                    packages::Repository::Cran
                },
                is_default: false,
            });
            if !self.package_bioc {
                self.package_info.request(&name);
            }
            self.package_filter = name;
            self.package_name.clear();
            self.refresh_packages = true;
        }
    }
    fn can_remove_package(&self, name: &str) -> bool {
        self.packages
            .iter()
            .any(|p| p.name == name && !p.is_default)
            && match &self.pending_package {
                // A status reply must still match the package list it queried.
                Some(PackageOperation::Query) => false,
                Some(PackageOperation::Install(active) | PackageOperation::Load(active)) => {
                    active != name
                }
                None => true,
            }
    }
    fn remove_package(&mut self, name: &str) {
        if !self.can_remove_package(name) {
            return;
        }
        self.packages.retain(|p| p.name != name);
        self.package_status.remove(name);
        self.package_info.forget(name);
        if self.package_filter.trim() == name {
            self.package_filter.clear();
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
                self.add_package();
            }
        });
        ui.separator();
        let filter = self.package_filter.to_lowercase();
        let entries = self.packages.clone();
        let mut action = None;
        let mut retry = None;
        let mut remove = None;
        egui::ScrollArea::vertical()
            .id_salt("package_list")
            .auto_shrink([false, true])
            .show(ui, |ui| {
                // Floating scrollbars expand over the content. Reserve their
                // full width and a gap so the remove button stays reachable.
                let scroll = ui.spacing().scroll;
                let gutter = ui.spacing().item_spacing.x
                    + if scroll.floating { scroll.bar_width + scroll.bar_outer_margin } else { 0.0 };
                ui.set_max_width((ui.available_width() - gutter).max(0.0));
                for package in entries.iter().filter(|p| {
                    p.name.to_lowercase().contains(&filter)
                        || p.description.to_lowercase().contains(&filter)
                        || matches!(self.package_info.get(&p.name), Some(cran::State::Ready(info))
                            if info.title.to_lowercase().contains(&filter)
                                || info.description.to_lowercase().contains(&filter))
                }) {
                    ui.push_id(&package.name, |ui| {
                        // Reserve the right edge first so descriptions can wrap
                        // without displacing the remove control.
                        ui.with_layout(egui::Layout::right_to_left(egui::Align::Min), |ui| {
                            if !package.is_default
                                && ui.add_enabled(
                                    self.can_remove_package(&package.name),
                                    egui::Button::new(RichText::new("×").size(14.0).weak())
                                        .small().frame(false),
                                )
                                .on_hover_text("Remove from list (keeps the installed package)")
                                .on_disabled_hover_text("Wait for the package operation to finish.")
                                .clicked()
                            {
                                remove = Some(package.name.clone());
                            }
                            ui.allocate_ui_with_layout(
                                egui::vec2(ui.available_width(), 0.0),
                                egui::Layout::left_to_right(egui::Align::Center).with_main_wrap(true),
                                |ui| {
                                    ui.strong(&package.name);
                                    match self.package_info.get(&package.name) {
                                        Some(cran::State::Ready(info)) => {
                                            ui.weak(&info.title).on_hover_text(&info.description);
                                            ui.hyperlink_to("CRAN", cran::package_url(&package.name));
                                        }
                                        Some(cran::State::Loading) => {
                                            ui.spinner();
                                            ui.weak("Looking up CRAN…");
                                        }
                                        Some(state @ (cran::State::Missing | cran::State::Failed(_))) => {
                                            let (label, detail) = match state {
                                                cran::State::Missing => ("Not found on CRAN", "Check the package name; it may be archived or hosted in another repository."),
                                                cran::State::Failed(error) => ("CRAN information unavailable", error.as_str()),
                                                _ => unreachable!(),
                                            };
                                            ui.weak(label).on_hover_text(detail);
                                            if ui.small_button("Retry lookup").clicked() {
                                                retry = Some(package.name.clone());
                                            }
                                        }
                                        None => { ui.weak(package.description); }
                                    }
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
                                },
                            );
                        });
                    });
                    ui.separator();
                }
            });
        if let Some(operation) = action {
            self.package_request(operation);
        }
        if let Some(name) = retry {
            self.package_info.request(&name);
        }
        if let Some(name) = remove {
            self.remove_package(&name);
        }
    }
    fn request_completion(&mut self, key: completion::Key) {
        if self.state != State::Ready || self.pending_completion.is_some() {
            return;
        }
        let id = self.next_id;
        self.next_id = self.next_id.wrapping_add(1).max(1);
        if let Some(session) = &self.session {
            match session.complete(id, key.payload()) {
                Ok(()) => {
                    self.pending_completion = Some((id, key));
                }
                Err(_) => self.completion.receive(
                    key,
                    completion::Results {
                        names: Vec::new(),
                        more: false,
                    },
                ),
            }
        }
    }
    fn request_environment(&mut self) {
        if self.state != State::Ready {
            return;
        }
        let id = self.next_id;
        self.next_id = self.next_id.wrapping_add(1).max(1);
        if let Some(session) = &self.session {
            match session.environment(id) {
                Ok(()) => {
                    self.pending_id = Some(id);
                    self.pending_environment = true;
                    self.refresh_environment = false;
                    self.state = State::Busy;
                }
                Err(error) => {
                    self.environment_notice = error;
                    self.refresh_environment = false;
                }
            }
        }
    }
    fn paste_image(&mut self) {
        if self.paste_receiver.is_some() || self.assistant.busy() {
            return;
        }
        let (sender, receiver) = std::sync::mpsc::channel();
        std::thread::spawn(move || {
            let _ = sender.send(clipboard_image::read());
        });
        self.paste_receiver = Some(receiver);
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
            PathAction::ExportPlot => self.project.join("plot.png").display().to_string(),
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
        let task = match action {
            PathAction::Open => file_worker::Task::Open(path),
            PathAction::SaveAs => file_worker::Task::Save(self.editor.current().clone(), path),
            PathAction::Project => file_worker::Task::List(path),
            PathAction::ImportData => {
                match import_command(&path, &self.import_name) {
                    Ok(command) => {
                        if self.submit_text(command, false) {
                            self.path_action = None;
                        } else {
                            self.message = Some("R is not ready to import data.".into());
                        }
                    }
                    Err(error) => self.message = Some(error),
                }
                return;
            }
            PathAction::ExportPlot => match self.plots.export_snapshot(&path) {
                Ok(png) => file_worker::Task::ExportPlot(path, png),
                Err(error) => {
                    self.message = Some(error);
                    return;
                }
            },
        };
        if self.queue_file(task) {
            self.path_action = None;
        }
    }
    fn queue_file(&mut self, task: file_worker::Task) -> bool {
        if self.file_busy {
            self.message = Some("A file operation is still in progress.".into());
            return false;
        }
        match self.files.submit(task) {
            Ok(()) => {
                self.file_busy = true;
                true
            }
            Err(error) => {
                self.message = Some(error);
                false
            }
        }
    }
    fn refresh_project(&mut self) {
        self.queue_file(file_worker::Task::List(self.project.clone()));
    }
    fn save_current(&mut self) {
        if let Some(path) = self.editor.current().path.clone() {
            self.queue_file(file_worker::Task::Save(self.editor.current().clone(), path));
        } else {
            self.ask_path(PathAction::SaveAs);
        }
    }
    fn poll_files(&mut self) {
        for event in self.files.poll() {
            let result = match event {
                file_worker::Event::Open(result) => {
                    self.file_busy = false;
                    result.map(|buffer| self.editor.accept_open(buffer))
                }
                file_worker::Event::Save(result) => {
                    self.file_busy = false;
                    result.map(|buffer| self.editor.accept_save(buffer))
                }
                file_worker::Event::List(result) => {
                    self.file_busy = false;
                    match result {
                        Ok((path, entries)) => {
                            self.project = path;
                            self.project_entries = entries;
                            self.project_error = None;
                        }
                        Err(error) => self.project_error = Some(error.to_string()),
                    }
                    Ok(())
                }
                file_worker::Event::ExportPlot(result) => {
                    self.file_busy = false;
                    result
                }
                file_worker::Event::Preferences(result) => {
                    self.preferences_pending = false;
                    result.map_err(|e| {
                        std::io::Error::other(format!("Could not save preferences: {e}"))
                    })
                }
            };
            if let Err(error) = result {
                self.message = Some(error.to_string());
            }
        }
    }
    fn queue_preferences(&mut self) {
        if self.preferences_pending {
            return;
        }
        let prefs = settings::Settings {
            dark: self.dark,
            font_size: self.font_size,
            console_fraction: self.console_fraction,
        };
        match self.files.submit(file_worker::Task::Preferences(prefs)) {
            Ok(()) => {
                self.preferences_pending = true;
                self.preferences_changed = None;
            }
            Err(error) => {
                self.message = Some(error);
                self.preferences_changed = None;
            }
        }
    }
    fn poll(&mut self, ctx: &egui::Context) {
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
                    self.refresh_environment = true;
                }
                Event::Output(text) => self.append(&text),
                Event::Plot { page, png } => match self.plots.receive(ctx, page, png) {
                    Ok(()) => self.output_tab = OutputTab::Plots,
                    Err(error) => self.append(&format!("[Plot error] {error}\n")),
                },
                Event::PlotError(error) => self.append(&format!("[Plot error] {error}\n")),
                Event::Done(id) if self.pending_id == Some(id) => {
                    self.pending_id = None;
                    self.completion.invalidate();
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
                    self.refresh_environment = true;
                    self.state = State::Ready;
                }
                Event::Environment(id, payload)
                    if self.pending_id == Some(id) && self.pending_environment =>
                {
                    self.pending_id = None;
                    self.pending_environment = false;
                    self.state = State::Ready;
                    match serde_json::from_str::<Vec<serde_json::Value>>(&payload) {
                        Ok(objects)
                            if objects.len() <= 20
                                && objects
                                    .iter()
                                    .all(|v| v.get("name").and_then(|s| s.as_str()).is_some()) =>
                        {
                            self.environment = objects;
                            self.selected_environment.retain(|name| {
                                self.environment
                                    .iter()
                                    .any(|v| v["name"].as_str() == Some(name.as_str()))
                            });
                            self.environment_notice.clear();
                        }
                        _ => {
                            self.environment.clear();
                            self.selected_environment.clear();
                            self.environment_notice = "Could not read the R environment.".into();
                        }
                    }
                }
                Event::Completions(id, payload)
                    if self
                        .pending_completion
                        .as_ref()
                        .is_some_and(|(pending, _)| *pending == id) =>
                {
                    if let Some((_, key)) = self.pending_completion.take() {
                        let parsed = serde_json::from_str::<serde_json::Value>(&payload)
                            .ok()
                            .and_then(|v| {
                                let names = v.get("names")?.as_array()?;
                                if names.len() > 200 {
                                    return None;
                                }
                                let names = names
                                    .iter()
                                    .map(|n| n.as_str().map(str::to_owned))
                                    .collect::<Option<Vec<_>>>()?;
                                Some(completion::Results {
                                    names,
                                    more: v.get("more")?.as_bool()?,
                                })
                            })
                            .unwrap_or(completion::Results {
                                names: Vec::new(),
                                more: false,
                            });
                        self.completion.receive(key, parsed);
                    }
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
                Event::Error(id, _)
                    if self
                        .pending_completion
                        .as_ref()
                        .is_some_and(|(pending, _)| *pending == id) =>
                {
                    if let Some((_, key)) = self.pending_completion.take() {
                        self.completion.receive(
                            key,
                            completion::Results {
                                names: Vec::new(),
                                more: false,
                            },
                        );
                    }
                }
                Event::Error(id, message) if self.pending_id == Some(id) => {
                    self.pending_id = None;
                    if self.pending_environment {
                        self.pending_environment = false;
                        self.environment.clear();
                        self.selected_environment.clear();
                        self.environment_notice =
                            format!("Could not inspect the R environment: {message}");
                        self.state = State::Ready;
                        continue;
                    }
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
                    self.refresh_environment = true;
                    self.completion.invalidate();
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
                    self.pending_completion = None;
                    self.completion.clear();
                    self.pending_environment = false;
                    self.environment.clear();
                    self.selected_environment.clear();
                    self.package_status.clear();
                    self.package_notice = format!("R session failed: {message}");
                    self.append(&format!("\n[Session error] {message}\n"));
                    self.state = State::Failed;
                }
                Event::Exited if self.state != State::Failed => {
                    self.state = State::Stopped;
                    self.environment.clear();
                    self.selected_environment.clear();
                    self.pending_completion = None;
                    self.completion.clear();
                }
                _ => {}
            }
        }
    }
}
impl eframe::App for App {
    fn update(&mut self, ctx: &egui::Context, _frame: &mut eframe::Frame) {
        self.poll(ctx);
        self.poll_files();
        self.package_info.poll();
        if self.refresh_environment && self.state == State::Ready {
            self.request_environment();
        }
        if self.output_tab == OutputTab::Packages
            && self.refresh_packages
            && self.state == State::Ready
        {
            self.package_request(PackageOperation::Query);
        }
        if self.state == State::Ready
            && !self.refresh_environment
            && let Some(key) = self.completion.need_request(Instant::now())
        {
            self.request_completion(key);
        }
        self.assistant.poll();
        let paste_result = self
            .paste_receiver
            .as_ref()
            .and_then(|rx| match rx.try_recv() {
                Ok(result) => Some(result),
                Err(std::sync::mpsc::TryRecvError::Empty) => None,
                Err(std::sync::mpsc::TryRecvError::Disconnected) => {
                    Some(Err("Clipboard image read failed.".into()))
                }
            });
        if let Some(result) = paste_result {
            self.paste_receiver = None;
            match result {
                Ok(Some(image)) => self.assistant.image = Some(image),
                Ok(None) => {}
                Err(error) => self.assistant.error = Some(error),
            }
        }
        // Keep an in-flight request visible even if visibility was changed elsewhere.
        if self.assistant.busy() {
            self.show_assistant = true;
        }
        let previous_settings = (self.dark, self.font_size, self.console_fraction);
        let close_requested = ctx.input(|i| i.viewport().close_requested());
        if close_requested || self.deferred_exit {
            if self.editor.any_dirty() && !self.allow_exit {
                ctx.send_viewport_cmd(egui::ViewportCommand::CancelClose);
                self.confirm_exit = true;
                self.deferred_exit = false;
            } else {
                if self.preferences_changed.is_some() {
                    self.queue_preferences();
                }
                if self.file_busy || self.preferences_pending || self.preferences_changed.is_some()
                {
                    ctx.send_viewport_cmd(egui::ViewportCommand::CancelClose);
                    self.deferred_exit = true;
                } else if !matches!(self.state, State::Stopped | State::Failed) {
                    // Keep pumping UI events while the supervisor stops/reaps R.
                    // Drop still provides immediate cancellation for a restart.
                    if let Some(session) = &self.session {
                        session.stop();
                    }
                    ctx.send_viewport_cmd(egui::ViewportCommand::CancelClose);
                    self.deferred_exit = true;
                } else if self.deferred_exit {
                    self.deferred_exit = false;
                    ctx.send_viewport_cmd(egui::ViewportCommand::Close);
                }
            }
        }
        self.flush_output();
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
                if self.file_busy {
                    ui.spinner();
                    ui.weak("File operation in progress…");
                }
                if ui.button("Clear output").clicked() {
                    self.output.clear();
                    self.output_pending.clear();
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
                    ui.small("Select Environment objects to share their summaries with the agent. Paste an image into the prompt to attach it.");
                    ui.separator();
                    egui::TopBottomPanel::bottom("assistant_request").show_inside(ui, |ui| {
                        ui.label("Ask for R code");
                        let prompt_response = ui.add_enabled(!self.assistant.busy(), egui::TextEdit::multiline(&mut self.assistant.prompt)
                            .desired_rows(3).desired_width(f32::INFINITY)
                            .hint_text("Describe your analysis or paste an image…"));
                        if prompt_response.has_focus() && ui.input_mut(|input| input.consume_key(egui::Modifiers::COMMAND, egui::Key::V)) {
                            self.paste_image();
                        }
                        if self.assistant.image.is_some() {
                            ui.horizontal(|ui| {
                                ui.label("Image attached to next request");
                                if ui.add_enabled(!self.assistant.busy(), egui::Button::new("Remove")).clicked() { self.assistant.image = None; }
                            });
                        }
                        ui.horizontal(|ui| {
                            if ui.add_enabled(!self.assistant.busy() && self.paste_receiver.is_none() && (self.selected_environment.is_empty() || (self.state == State::Ready && !self.refresh_environment)), egui::Button::new("Generate")).clicked() {
                                let source = self.assistant.include_source.then(|| self.editor.current().text.clone());
                                let objects = self.environment.iter().filter(|v| v["name"].as_str().is_some_and(|name| self.selected_environment.contains(name))).cloned().collect();
                                self.assistant.send(source.as_deref(), objects);
                            }
                            if self.assistant.busy() { ui.spinner(); ui.label("Generating…"); }
                            if ui.add_enabled(!self.assistant.busy(), egui::Button::new("Clear chat")).clicked() {
                                self.assistant.conversation.clear(); self.assistant.answer = None; self.assistant.error = None;
                            }
                        });
                    });
                    let mut destination: Option<(bool, String)> = None;
                    for event in &self.assistant.tool_log { ui.weak(event); }
                    if let Some(answer) = &self.assistant.answer {
                        if answer.truncated { ui.colored_label(Color32::LIGHT_RED, "Response was truncated; request a shorter answer."); }
                        else if answer.code.is_empty() { ui.weak("No complete R code block found; ask for code in an R block."); }
                    }
                    if let Some(error) = &self.assistant.error { ui.colored_label(Color32::LIGHT_RED, error); }
                    egui::ScrollArea::vertical().id_salt("assistant_output").stick_to_bottom(true).show(ui, |ui| {
                        for (turn_index, turn) in self.assistant.conversation.iter().enumerate() {
                            ui.strong(if turn.role == "user" { "You" } else { "Assistant" });
                            if turn.role == "user" { ui.label(&turn.content); }
                            else {
                                for (part_index, (is_code, content)) in agent::display_parts(&turn.content).into_iter().enumerate() {
                                    if !is_code { ui.label(content); continue; }
                                    ui.push_id((turn_index, part_index), |ui| {
                                        egui::Frame::group(ui.style()).show(ui, |ui| {
                                            ui.horizontal_wrapped(|ui| {
                                                ui.strong("R");
                                                let enabled = !turn.truncated && !content.trim().is_empty();
                                                if ui.add_enabled(enabled, egui::Button::new("Copy")).clicked() { ui.ctx().copy_text(content.clone()); }
                                                if ui.add_enabled(enabled, egui::Button::new("Append to Source")).clicked() { destination = Some((false, content.clone())); }
                                                if ui.add_enabled(enabled, egui::Button::new("Append to Console")).clicked() { destination = Some((true, content.clone())); }
                                            });
                                            egui::ScrollArea::both().max_height(140.0).id_salt("code").show(ui, |ui| {
                                                let job = syntax::highlight(&content, self.dark, (self.font_size - 1.0).max(10.0));
                                                ui.label(job);
                                            });
                                        });
                                    });
                                }
                            }
                            ui.separator();
                        }
                    });
                    if let Some((console, code)) = destination {
                        if console {
                            if !self.input.is_empty() && !self.input.ends_with('\n') { self.input.push('\n'); }
                            self.input.push_str(&code);
                        } else {
                            let text = &mut self.editor.current_mut().text;
                            if !text.is_empty() && !text.ends_with('\n') { text.push('\n'); }
                            text.push_str(&code);
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
                        ui.horizontal(|ui| {
                            ui.heading("Environment");
                            if ui.add_enabled(self.state == State::Ready, egui::Button::new("Import CSV/TSV…")).clicked() { self.ask_path(PathAction::ImportData); }
                            if ui.add_enabled(self.state == State::Ready, egui::Button::new("Refresh")).clicked() { self.refresh_environment = true; }
                        });
                        ui.separator();
                        ui.small("Select objects to let the assistant inspect their names, types and sample values.");
                        if !self.environment_notice.is_empty() { ui.colored_label(Color32::LIGHT_RED, &self.environment_notice); }
                        if self.environment.is_empty() { ui.weak("No objects in the R environment."); }
                        egui::ScrollArea::vertical().id_salt("environment_objects").show(ui, |ui| {
                            for object in &self.environment {
                                let name = object["name"].as_str().unwrap_or_default();
                                let class = object["class"].as_str().unwrap_or_default();
                                let count = object["length"].as_u64().unwrap_or_default();
                                let mut selected = self.selected_environment.contains(name);
                                if ui.checkbox(&mut selected, format!("{name}  ·  {class}  ·  {count} rows/values")).changed() {
                                    if selected { self.selected_environment.insert(name.to_owned()); } else { self.selected_environment.remove(name); }
                                }
                            }
                        });
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
                            self.queue_file(file_worker::Task::List(parent.to_owned()));
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
                                        self.queue_file(file_worker::Task::List(path.clone()));
                                    }
                                } else if ui.button(name).clicked() {
                                    self.queue_file(file_worker::Task::Open(path.clone()));
                                }
                            }
                            if entries.len() > 300 {
                                ui.weak("Showing the first 300 entries.");
                            }
                        });
                } else if self.output_tab == OutputTab::Packages {
                    self.packages_ui(ui);
                } else {
                    if self.plots.show(ui) {
                        self.ask_path(PathAction::ExportPlot);
                    }
                }
            });
        self.plots.show_zoom(ctx);

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
                if ui
                    .add_enabled(!self.file_busy, egui::Button::new("Close"))
                    .clicked()
                {
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
            let cache = &mut self.source_cache;
            let can_wait_for_completion = self.state == State::Ready
                || self.pending_completion.is_some()
                || self.pending_environment;
            let buffer = self.editor.current_mut();
            let id = egui::Id::new(("source_editor", buffer.id));
            let completion = &mut self.completion;
            let pending_insert = &mut self.completion_insert;
            let menu_rect = ctx.memory(|memory| memory.area_rect(id.with("completion_menu")));
            let menu_interaction = completion.visible
                && ctx.input(|input| {
                    (input.pointer.any_down() || input.pointer.any_released())
                        && input
                            .pointer
                            .hover_pos()
                            .is_some_and(|pos| menu_rect.is_some_and(|rect| rect.contains(pos)))
                });
            let before = completion.analyze(&buffer.text, buffer.id, buffer.cursor);
            let focused = ctx.memory(|memory| memory.has_focus(id));
            let defer_tab = focused
                && completion.visible
                && before.as_ref() == completion.query.as_ref()
                && coalesced_completion_tab(ctx)
                && ctx.input_mut(|input| input.consume_key(egui::Modifiers::NONE, egui::Key::Tab));
            let input_changes_context = ctx.input(|input| {
                input.events.iter().any(|event| match event {
                    egui::Event::Text(_)
                    | egui::Event::Paste(_)
                    | egui::Event::Cut
                    | egui::Event::Ime(egui::ImeEvent::Preedit(_) | egui::ImeEvent::Commit(_)) => {
                        true
                    }
                    egui::Event::Key {
                        key, pressed: true, ..
                    } => matches!(
                        key,
                        egui::Key::Backspace
                            | egui::Key::Delete
                            | egui::Key::ArrowLeft
                            | egui::Key::ArrowRight
                            | egui::Key::Home
                            | egui::Key::End
                            | egui::Key::PageUp
                            | egui::Key::PageDown
                    ),
                    _ => false,
                })
            });
            let matches = focused
                && !input_changes_context
                && before.as_ref() == completion.query.as_ref()
                && before.is_some();
            // Let any text or IME update finish before applying a mouse or
            // deferred selection; the next frame rechecks the cursor context.
            let mut insert = if input_changes_context {
                None
            } else {
                pending_insert.take()
            };
            if matches
                && let Some(selected) = completion_keys(ctx, completion, can_wait_for_completion)
            {
                insert = Some(selected);
            }
            if matches && insert.is_none() {
                insert = completion.manual_result();
            }
            let mut inserted = false;
            if let Some((query, text)) = insert
                && before.as_ref() == Some(&query)
            {
                let mut replacement = query;
                if text == "\t" {
                    replacement.start = replacement.cursor;
                    replacement.end = replacement.cursor;
                }
                inject_completion(ctx, id, &replacement, text);
                inserted = true;
            }
            let mut run = false;
            egui::ScrollArea::both()
                .id_salt("source_scroll")
                .show(ui, |ui| {
                    ui.horizontal_top(|ui| {
                        ui.label(
                            RichText::new(cache.line_numbers(&buffer.text))
                                .font(egui::FontId::monospace(size))
                                .weak(),
                        );
                        let mut layouter =
                            |ui: &egui::Ui, text: &dyn egui::TextBuffer, _wrap_width: f32| {
                                cache.layout(ui, text.as_str(), dark, size)
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
                        if menu_interaction && !response.response.has_focus() {
                            ctx.memory_mut(|memory| memory.request_focus(id));
                        }
                        let has_focus = response.response.has_focus() || menu_interaction;
                        let active = has_focus
                            .then(|| completion.analyze(&buffer.text, buffer.id, buffer.cursor))
                            .flatten();
                        completion.observe(active, Instant::now());
                        if defer_tab && let Some(query) = completion.query.clone() {
                            if !completion.results.is_empty() {
                                *pending_insert = completion.insertion();
                            } else if can_wait_for_completion {
                                completion.request_now(Instant::now());
                            } else {
                                *pending_insert = Some((query, "\t".into()));
                            }
                        }
                        if inserted {
                            completion.dismiss();
                        }
                        if completion.visible && has_focus {
                            let range = response.cursor_range.unwrap_or_else(|| {
                                egui::text::CCursorRange::one(egui::text::CCursor::new(
                                    buffer.cursor.0,
                                ))
                            });
                            let caret = response
                                .galley
                                .pos_from_cursor(range.primary)
                                .translate(response.galley_pos.to_vec2());
                            let screen = ctx.content_rect();
                            let x = caret
                                .left()
                                .clamp(screen.left(), (screen.right() - 230.0).max(screen.left()));
                            let y = if caret.bottom() + 180.0 < screen.bottom() {
                                caret.bottom()
                            } else {
                                (caret.top() - 180.0).max(screen.top())
                            };
                            egui::Area::new(id.with("completion_menu"))
                                .order(egui::Order::Foreground)
                                .fixed_pos(egui::pos2(x, y))
                                .show(ctx, |ui| {
                                    egui::Frame::popup(ui.style()).show(ui, |ui| {
                                        ui.set_min_width(220.0);
                                        egui::ScrollArea::vertical()
                                            .max_height(180.0)
                                            .id_salt("items")
                                            .show(ui, |ui| {
                                                for (index, name) in
                                                    completion.results.iter().enumerate()
                                                {
                                                    let selected = index == completion.selected;
                                                    let response =
                                                        ui.selectable_label(selected, name);
                                                    if selected {
                                                        response.scroll_to_me(Some(
                                                            egui::Align::Center,
                                                        ));
                                                    }
                                                    if response.clicked() {
                                                        completion.selected = index;
                                                        *pending_insert = completion.insertion();
                                                        ctx.memory_mut(|memory| {
                                                            memory.request_focus(id)
                                                        });
                                                    }
                                                }
                                                if completion.results.len() == 100 {
                                                    ui.weak("Type more to narrow the list.");
                                                }
                                            });
                                    });
                                });
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
                PathAction::ImportData => "Import CSV or TSV into Environment",
                PathAction::ExportPlot => "Export plot as PNG",
            };
            egui::Window::new(title)
                .collapsible(false)
                .resizable(false)
                .show(ctx, |ui| {
                    ui.label(
                        "Enter an absolute path, or a path relative to the project directory.",
                    );
                    ui.add(egui::TextEdit::singleline(&mut self.path_input).desired_width(400.0));
                    if action == PathAction::ImportData {
                        ui.label("R object name:");
                        ui.add(
                            egui::TextEdit::singleline(&mut self.import_name).desired_width(250.0),
                        );
                    }
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
            self.preferences_changed = Some(Instant::now());
        }
        if self
            .preferences_changed
            .is_some_and(|changed| changed.elapsed() >= Duration::from_millis(500))
        {
            self.queue_preferences();
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

#[cfg(test)]
mod assistant_import_tests {
    use super::*;
    #[test]
    fn import_escapes_user_paths_and_refuses_to_replace_objects() {
        let command =
            import_command(std::path::Path::new("/tmp/a\";bad().csv"), "x\";bad()").unwrap();
        assert!(command.contains("exists("));
        assert!(command.contains("a\\\";bad().csv"));
        assert!(command.contains("x\\\";bad()"));
        assert!(import_command(std::path::Path::new("/tmp/a.rds"), "x").is_err());
        assert!(import_command(std::path::Path::new("/tmp/a.csv"), "").is_err());
    }

    #[test]
    fn generated_import_loads_csv_without_overwriting() {
        if std::process::Command::new("Rscript")
            .arg("--version")
            .output()
            .is_err()
        {
            return;
        }
        let path =
            std::env::temp_dir().join(format!("rsstudio-import-{}-\".csv", std::process::id()));
        std::fs::write(&path, "value\n1\n2\n").unwrap();
        let command = import_command(&path, "my data").unwrap();
        let script = format!(
            "{command}; stopifnot(nrow(get('my data')) == 2); tryCatch({{{command}; stop('overwrite allowed')}}, error=function(e) stopifnot(grepl('Object already exists', conditionMessage(e))))"
        );
        let output = std::process::Command::new("Rscript")
            .args(["--vanilla", "-e", &script])
            .output()
            .unwrap();
        std::fs::remove_file(path).unwrap();
        assert!(
            output.status.success(),
            "{}",
            String::from_utf8_lossy(&output.stderr)
        );
    }

    #[test]
    fn chat_accepts_image_shortcut_without_breaking_text_paste() {
        let ctx = egui::Context::default();
        let mut text = String::new();
        let _ = ctx.run(egui::RawInput::default(), |ctx| {
            egui::CentralPanel::default().show(ctx, |ui| {
                ui.add(egui::TextEdit::multiline(&mut text).id_source("chat_test"))
                    .request_focus();
            });
        });
        let modifiers = egui::Modifiers {
            ctrl: true,
            command: true,
            ..Default::default()
        };
        let mut image_paste = false;
        let _ = ctx.run(
            egui::RawInput {
                modifiers,
                events: vec![egui::Event::Key {
                    key: egui::Key::V,
                    physical_key: Some(egui::Key::V),
                    pressed: true,
                    repeat: false,
                    modifiers,
                }],
                ..Default::default()
            },
            |ctx| {
                egui::CentralPanel::default().show(ctx, |ui| {
                    let response =
                        ui.add(egui::TextEdit::multiline(&mut text).id_source("chat_test"));
                    image_paste = response.has_focus()
                        && ui.input_mut(|i| i.consume_key(egui::Modifiers::COMMAND, egui::Key::V));
                });
            },
        );
        assert!(image_paste);
        let _ = ctx.run(
            egui::RawInput {
                events: vec![egui::Event::Paste("hello".into())],
                ..Default::default()
            },
            |ctx| {
                egui::CentralPanel::default().show(ctx, |ui| {
                    ui.add(egui::TextEdit::multiline(&mut text).id_source("chat_test"));
                });
            },
        );
        assert_eq!(text, "hello");
    }

    #[test]
    fn source_completion_replaces_token_and_can_be_undone() {
        let ctx = egui::Context::default();
        let id = egui::Id::new("completion-editor-test");
        let mut text = "π <- airquality$Tempx".to_owned();
        let caret = text.chars().count() - 1;
        let query = completion::query(&text, 1, (caret, caret)).unwrap();
        let _ = ctx.run(
            egui::RawInput {
                time: Some(1.0),
                ..Default::default()
            },
            |ctx| {
                egui::CentralPanel::default().show(ctx, |ui| {
                    ui.add(egui::TextEdit::multiline(&mut text).id(id).code_editor())
                        .request_focus();
                });
            },
        );
        let _ = ctx.run(
            egui::RawInput {
                time: Some(2.0),
                ..Default::default()
            },
            |ctx| {
                inject_completion(ctx, id, &query, "Temp".into());
                egui::CentralPanel::default().show(ctx, |ui| {
                    ui.add(egui::TextEdit::multiline(&mut text).id(id).code_editor());
                });
            },
        );
        assert_eq!(text, "π <- airquality$Temp");
        let modifiers = egui::Modifiers {
            ctrl: true,
            command: true,
            ..Default::default()
        };
        let _ = ctx.run(
            egui::RawInput {
                time: Some(3.0),
                modifiers,
                events: vec![egui::Event::Key {
                    key: egui::Key::Z,
                    physical_key: Some(egui::Key::Z),
                    pressed: true,
                    repeat: false,
                    modifiers,
                }],
                ..Default::default()
            },
            |ctx| {
                egui::CentralPanel::default().show(ctx, |ui| {
                    ui.add(egui::TextEdit::multiline(&mut text).id(id).code_editor());
                });
            },
        );
        assert_eq!(text, "π <- airquality$Tempx");
    }

    #[test]
    fn completion_keyboard_selects_results_and_leaves_indentation_alone() {
        let ctx = egui::Context::default();
        let mut completion = completion::Completion::default();
        let query = completion::query("airquality$", 1, (11, 11)).unwrap();
        completion.observe(Some(query.clone()), Instant::now());
        completion.receive(
            query.key.clone(),
            completion::Results {
                names: vec!["Temp".into(), "Wind".into()],
                more: false,
            },
        );
        let event = |key, modifiers| egui::Event::Key {
            key,
            physical_key: Some(key),
            pressed: true,
            repeat: false,
            modifiers,
        };
        let _ = ctx.run(
            egui::RawInput {
                events: vec![event(egui::Key::ArrowDown, egui::Modifiers::NONE)],
                ..Default::default()
            },
            |ctx| {
                assert!(completion_keys(ctx, &mut completion, true).is_none());
            },
        );
        let mut selected = None;
        let _ = ctx.run(
            egui::RawInput {
                events: vec![event(egui::Key::Tab, egui::Modifiers::NONE)],
                ..Default::default()
            },
            |ctx| {
                selected = completion_keys(ctx, &mut completion, true);
                assert!(!ctx.input(|i| i.key_pressed(egui::Key::Tab)));
            },
        );
        assert_eq!(selected.unwrap().1, "Wind");
        completion.receive(
            query.key,
            completion::Results {
                names: Vec::new(),
                more: false,
            },
        );
        let _ = ctx.run(
            egui::RawInput {
                events: vec![event(egui::Key::Tab, egui::Modifiers::NONE)],
                ..Default::default()
            },
            |ctx| {
                assert!(completion_keys(ctx, &mut completion, true).is_none());
                assert!(ctx.input(|i| i.key_pressed(egui::Key::Tab)));
            },
        );
        let query = completion::query("airquality$Te", 1, (13, 13)).unwrap();
        completion.observe(Some(query.clone()), Instant::now());
        completion.request_now(Instant::now());
        completion.receive(
            query.key,
            completion::Results {
                names: vec!["Temp".into()],
                more: false,
            },
        );
        let _ = ctx.run(
            egui::RawInput {
                events: vec![event(egui::Key::Escape, egui::Modifiers::NONE)],
                ..Default::default()
            },
            |ctx| {
                assert!(completion_keys(ctx, &mut completion, true).is_none());
                assert!(completion.manual_result().is_none());
            },
        );
    }

    #[test]
    fn source_dropdown_keyboard_completes_in_the_full_app() {
        let ctx = egui::Context::default();
        let mut app = App::new(settings::Settings::default(), &ctx);
        app.session.take();
        app.state = State::Stopped;
        app.refresh_environment = false;
        app.show_assistant = false;
        let buffer = app.editor.current_mut();
        buffer.text = "airquality$".into();
        let caret = buffer.text.chars().count();
        buffer.cursor = (caret, caret);
        let id = egui::Id::new(("source_editor", buffer.id));
        let mut edit_state = egui::TextEdit::load_state(&ctx, id).unwrap_or_default();
        edit_state
            .cursor
            .set_char_range(Some(egui::text::CCursorRange::one(
                egui::text::CCursor::new(caret),
            )));
        edit_state.store(&ctx, id);
        ctx.memory_mut(|memory| memory.request_focus(id));
        let mut frame = eframe::Frame::_new_kittest();
        let input = |events| egui::RawInput {
            screen_rect: Some(egui::Rect::from_min_size(
                egui::Pos2::ZERO,
                egui::vec2(1280.0, 800.0),
            )),
            events,
            ..Default::default()
        };
        let _ = ctx.run(input(vec![]), |ctx| {
            eframe::App::update(&mut app, ctx, &mut frame)
        });
        let query = app
            .completion
            .query
            .clone()
            .expect("source editor has focus");
        app.completion.receive(
            query.key,
            completion::Results {
                names: vec!["Temp".into(), "Wind".into()],
                more: false,
            },
        );
        let event = |key| egui::Event::Key {
            key,
            physical_key: Some(key),
            pressed: true,
            repeat: false,
            modifiers: egui::Modifiers::NONE,
        };
        let _ = ctx.run(
            input(vec![
                egui::Event::Ime(egui::ImeEvent::Disabled),
                event(egui::Key::ArrowDown),
            ]),
            |ctx| eframe::App::update(&mut app, ctx, &mut frame),
        );
        assert_eq!(
            app.completion.selected, 1,
            "arrow should select the next suggestion"
        );
        let _ = ctx.run(
            input(vec![
                egui::Event::Ime(egui::ImeEvent::Enabled),
                event(egui::Key::Tab),
            ]),
            |ctx| eframe::App::update(&mut app, ctx, &mut frame),
        );
        assert_eq!(app.editor.current().text, "airquality$Wind");
    }

    #[test]
    fn source_dropdown_completes_after_typing_and_tab_in_one_frame() {
        let ctx = egui::Context::default();
        let mut app = App::new(settings::Settings::default(), &ctx);
        app.session.take();
        app.state = State::Stopped;
        app.refresh_environment = false;
        app.show_assistant = false;
        let buffer = app.editor.current_mut();
        buffer.text = "airquality$".into();
        let caret = buffer.text.chars().count();
        buffer.cursor = (caret, caret);
        let id = egui::Id::new(("source_editor", buffer.id));
        let mut edit_state = egui::TextEdit::load_state(&ctx, id).unwrap_or_default();
        edit_state
            .cursor
            .set_char_range(Some(egui::text::CCursorRange::one(
                egui::text::CCursor::new(caret),
            )));
        edit_state.store(&ctx, id);
        ctx.memory_mut(|memory| memory.request_focus(id));
        let mut frame = eframe::Frame::_new_kittest();
        let input = |events| egui::RawInput {
            screen_rect: Some(egui::Rect::from_min_size(
                egui::Pos2::ZERO,
                egui::vec2(1280.0, 800.0),
            )),
            events,
            ..Default::default()
        };
        let _ = ctx.run(input(vec![]), |ctx| {
            eframe::App::update(&mut app, ctx, &mut frame)
        });
        let query = app
            .completion
            .query
            .clone()
            .expect("source editor has focus");
        app.completion.receive(
            query.key,
            completion::Results {
                names: vec!["Temp".into(), "Wind".into()],
                more: false,
            },
        );
        let event = |key| egui::Event::Key {
            key,
            physical_key: Some(key),
            pressed: true,
            repeat: false,
            modifiers: egui::Modifiers::NONE,
        };
        let _ = ctx.run(
            input(vec![egui::Event::Text("T".into()), event(egui::Key::Tab)]),
            |ctx| eframe::App::update(&mut app, ctx, &mut frame),
        );
        let _ = ctx.run(input(vec![]), |ctx| {
            eframe::App::update(&mut app, ctx, &mut frame)
        });
        assert_eq!(app.editor.current().text, "airquality$Temp");
    }

    #[test]
    fn source_dropdown_mouse_click_selects_a_suggestion() {
        let ctx = egui::Context::default();
        let mut app = App::new(settings::Settings::default(), &ctx);
        app.session.take();
        app.state = State::Stopped;
        app.refresh_environment = false;
        app.show_assistant = false;
        let buffer = app.editor.current_mut();
        buffer.text = "airquality$".into();
        let caret = buffer.text.chars().count();
        buffer.cursor = (caret, caret);
        let id = egui::Id::new(("source_editor", buffer.id));
        let mut state = egui::TextEdit::load_state(&ctx, id).unwrap_or_default();
        state
            .cursor
            .set_char_range(Some(egui::text::CCursorRange::one(
                egui::text::CCursor::new(caret),
            )));
        state.store(&ctx, id);
        ctx.memory_mut(|m| m.request_focus(id));
        let mut frame = eframe::Frame::_new_kittest();
        let input = |events| egui::RawInput {
            screen_rect: Some(egui::Rect::from_min_size(
                egui::Pos2::ZERO,
                egui::vec2(1280.0, 800.0),
            )),
            events,
            ..Default::default()
        };
        let _ = ctx.run(input(vec![]), |ctx| {
            eframe::App::update(&mut app, ctx, &mut frame)
        });
        let query = app.completion.query.clone().unwrap();
        app.completion.receive(
            query.key,
            completion::Results {
                names: vec!["Temp".into(), "Wind".into()],
                more: false,
            },
        );
        let _ = ctx.run(input(vec![]), |ctx| {
            eframe::App::update(&mut app, ctx, &mut frame)
        });
        let rect = ctx
            .memory(|m| m.area_rect(id.with("completion_menu")))
            .expect("dropdown visible");
        let position = rect.min + egui::vec2(30.0, 15.0);
        let button = |pressed| egui::Event::PointerButton {
            pos: position,
            button: egui::PointerButton::Primary,
            pressed,
            modifiers: egui::Modifiers::NONE,
        };
        let _ = ctx.run(input(vec![egui::Event::PointerMoved(position)]), |ctx| {
            eframe::App::update(&mut app, ctx, &mut frame)
        });
        let _ = ctx.run(input(vec![button(true)]), |ctx| {
            eframe::App::update(&mut app, ctx, &mut frame)
        });
        let _ = ctx.run(input(vec![button(false)]), |ctx| {
            eframe::App::update(&mut app, ctx, &mut frame)
        });
        let _ = ctx.run(
            input(vec![egui::Event::Ime(
                egui::ImeEvent::Commit(String::new()),
            )]),
            |ctx| eframe::App::update(&mut app, ctx, &mut frame),
        );
        let _ = ctx.run(input(vec![]), |ctx| {
            eframe::App::update(&mut app, ctx, &mut frame)
        });
        assert_eq!(app.editor.current().text, "airquality$Temp");
    }

    #[test]
    fn completion_keeps_ready_status_and_preserves_queued_running_status() {
        if std::net::TcpListener::bind("127.0.0.1:0").is_err() {
            assert!(
                std::env::var_os("RSSTUDIO_REQUIRE_R").is_none(),
                "loopback is required for this test"
            );
            return;
        }
        if std::process::Command::new("R")
            .arg("--version")
            .output()
            .is_err()
        {
            assert!(
                std::env::var_os("RSSTUDIO_REQUIRE_R").is_none(),
                "R is required for this test"
            );
            return;
        }
        let ctx = egui::Context::default();
        let mut app = App::new(settings::Settings::default(), &ctx);
        let deadline = Instant::now() + Duration::from_secs(10);
        while app.state == State::Starting {
            app.poll(&ctx);
            assert!(Instant::now() < deadline, "R startup timed out");
            std::thread::sleep(Duration::from_millis(10));
        }
        assert!(app.state == State::Ready);
        for prefix in ["a", "ai", "a"] {
            app.request_completion(completion::Key {
                kind: completion::Kind::Symbol,
                prefix: prefix.into(),
            });
            assert!(app.pending_completion.is_some());
            while app.pending_completion.is_some() {
                assert!(
                    app.state == State::Ready,
                    "editing lookups must remain Ready"
                );
                app.poll(&ctx);
                assert!(
                    app.state == State::Ready,
                    "lookup completion must remain Ready"
                );
                assert!(Instant::now() < deadline, "editing lookup timed out");
                std::thread::sleep(Duration::from_millis(10));
            }
        }
        let key = completion::Key {
            kind: completion::Kind::Symbol,
            prefix: "mea".into(),
        };
        app.request_completion(key);
        assert!(app.state == State::Ready, "a lookup must not show Running");
        assert!(app.pending_completion.is_some());
        assert!(app.pending_id.is_none());
        assert!(app.submit_text("Sys.sleep(0.5); x <- 41".into(), false));
        assert!(app.state == State::Busy);
        while app.pending_completion.is_some() {
            app.poll(&ctx);
            assert!(Instant::now() < deadline, "completion timed out");
            std::thread::sleep(Duration::from_millis(10));
        }
        assert!(
            app.state == State::Busy,
            "completion must not mark an active command Ready"
        );
        while app.pending_id.is_some() {
            app.poll(&ctx);
            assert!(Instant::now() < deadline, "queued command timed out");
            std::thread::sleep(Duration::from_millis(10));
        }
        assert!(app.state == State::Ready);
        assert!(app.submit_text("stopifnot(x == 41)".into(), false));
        while app.pending_id.is_some() {
            app.poll(&ctx);
            assert!(Instant::now() < deadline, "verification timed out");
            std::thread::sleep(Duration::from_millis(10));
        }
        assert!(app.state == State::Ready);
        assert!(
            !app.output_pending.contains("Error:"),
            "{}",
            app.output_pending
        );
    }
}

#[cfg(test)]
mod package_metadata_tests {
    use super::*;

    fn app(ctx: &egui::Context) -> App {
        let mut app = App::new(settings::Settings::default(), ctx);
        app.session.take();
        app.state = State::Stopped;
        app.refresh_environment = false;
        app.package_info = cran::Lookup::with_fetch(|_| {
            cran::State::Ready(cran::Metadata {
                title: "Neural tensor models".into(),
                description: "Tensor utilities with automatic differentiation.".into(),
            })
        });
        app
    }

    #[test]
    fn manually_added_cran_package_displays_metadata_and_searches_its_description() {
        let ctx = egui::Context::default();
        let mut app = app(&ctx);
        app.package_name = " torch ".into();
        app.add_package();
        assert_eq!(app.packages.last().unwrap().name, "torch");
        assert!(matches!(
            app.package_info.get("torch"),
            Some(cran::State::Loading)
        ));
        let deadline = Instant::now() + Duration::from_secs(2);
        while !matches!(app.package_info.get("torch"), Some(cran::State::Ready(_))) {
            app.package_info.poll();
            assert!(
                Instant::now() < deadline,
                "Metadata lookup did not complete"
            );
            std::thread::sleep(Duration::from_millis(2));
        }
        app.package_filter = "automatic".into();
        app.package_status.insert(
            "torch".into(),
            packages::Status {
                version: None,
                loaded: false,
            },
        );
        let output = ctx.run(egui::RawInput::default(), |ctx| {
            egui::CentralPanel::default().show(ctx, |ui| app.packages_ui(ui));
        });
        fn labels(shape: &egui::Shape, text: &mut String) {
            match shape {
                egui::Shape::Text(shape) => {
                    text.push_str(&shape.galley.job.text);
                    text.push('\n');
                }
                egui::Shape::Vec(shapes) => {
                    for shape in shapes {
                        labels(shape, text);
                    }
                }
                _ => {}
            }
        }
        let mut text = String::new();
        for shape in &output.shapes {
            labels(&shape.shape, &mut text);
        }
        assert!(text.contains("torch"), "{text}");
        assert!(text.contains("Neural tensor models"), "{text}");
        assert!(text.contains("Not installed"), "{text}");
        assert!(!text.contains("Custom package"));
        assert!(app.pending_id.is_none());
        assert!(app.input.is_empty());
        assert!(matches!(app.state, State::Stopped));
    }

    #[test]
    fn bioconductor_and_invalid_names_do_not_start_cran_lookups() {
        let ctx = egui::Context::default();
        let mut app = app(&ctx);
        app.package_bioc = true;
        app.package_name = "edgeR".into();
        app.add_package();
        let package = app.packages.last().unwrap();
        assert!(package.repository == packages::Repository::Bioconductor);
        assert_eq!(package.description, "Bioconductor package");
        assert!(app.package_info.get("edgeR").is_none());
        let count = app.packages.len();
        app.package_bioc = false;
        app.package_name = "../../invalid".into();
        app.add_package();
        assert_eq!(app.packages.len(), count);
        assert!(app.package_info.get("../../invalid").is_none());
    }

    fn package_frame(
        app: &mut App,
        ctx: &egui::Context,
        width: f32,
        events: Vec<egui::Event>,
    ) -> Vec<egui::Rect> {
        package_frame_at_height(app, ctx, width, 600.0, events)
    }

    fn package_frame_at_height(
        app: &mut App,
        ctx: &egui::Context,
        width: f32,
        height: f32,
        events: Vec<egui::Event>,
    ) -> Vec<egui::Rect> {
        let output = ctx.run(
            egui::RawInput {
                screen_rect: Some(egui::Rect::from_min_size(
                    egui::Pos2::ZERO,
                    egui::vec2(width, height),
                )),
                events,
                ..Default::default()
            },
            |ctx| {
                egui::CentralPanel::default().show(ctx, |ui| app.packages_ui(ui));
            },
        );
        fn collect(shape: &egui::Shape, rects: &mut Vec<egui::Rect>) {
            match shape {
                egui::Shape::Text(text) if text.galley.job.text == "×" => {
                    rects.push(text.galley.rect.translate(text.pos.to_vec2()))
                }
                egui::Shape::Vec(shapes) => {
                    for shape in shapes {
                        collect(shape, rects);
                    }
                }
                _ => {}
            }
        }
        let mut rects = Vec::new();
        for shape in output.shapes {
            collect(&shape.shape, &mut rects);
        }
        rects
    }

    #[test]
    fn remove_button_is_small_at_the_right_only_for_added_packages_and_removes_on_click() {
        let ctx = egui::Context::default();
        let mut app = app(&ctx);
        app.package_name = "torch".into();
        app.add_package();
        app.package_bioc = true;
        app.package_name = "edgeR".into();
        app.add_package();
        for width in [320.0, 800.0] {
            for name in ["torch", "edgeR"] {
                app.package_filter = name.into();
                let buttons = package_frame(&mut app, &ctx, width, vec![]);
                assert_eq!(buttons.len(), 1, "Missing remove control for {name}");
                let button = buttons[0];
                assert!(
                    button.center().x > width - 35.0 && button.right() <= width,
                    "Remove control left the right edge at width {width}: {button:?}"
                );
                assert!(button.width() < 20.0 && button.height() < 20.0);
            }
            for name in ["dplyr", "DESeq2"] {
                app.package_filter = name.into();
                assert!(
                    package_frame(&mut app, &ctx, width, vec![]).is_empty(),
                    "Default package {name} had a remove control"
                );
            }
        }
        app.package_filter = "torch".into();
        app.package_status.insert(
            "torch".into(),
            packages::Status {
                version: Some("1.0".into()),
                loaded: true,
            },
        );
        let button = package_frame(&mut app, &ctx, 320.0, vec![])[0];
        let click = |pressed| {
            vec![
                egui::Event::PointerMoved(button.center()),
                egui::Event::PointerButton {
                    pos: button.center(),
                    button: egui::PointerButton::Primary,
                    pressed,
                    modifiers: egui::Modifiers::NONE,
                },
            ]
        };
        package_frame(&mut app, &ctx, 320.0, click(true));
        package_frame(&mut app, &ctx, 320.0, click(false));
        assert!(!app.packages.iter().any(|p| p.name == "torch"));
        assert!(!app.package_status.contains_key("torch"));
        assert!(app.package_info.get("torch").is_none());
        assert!(app.package_filter.is_empty());
        assert!(app.pending_package.is_none() && app.pending_id.is_none());
        assert!(app.input.is_empty() && app.state == State::Stopped);
        assert!(app.packages.iter().any(|p| p.name == "edgeR"));
        assert!(app.packages.iter().filter(|p| p.is_default).count() == packages::catalog().len());
    }

    #[test]
    fn removal_preserves_defaults_and_in_flight_package_operations() {
        let ctx = egui::Context::default();
        let mut app = app(&ctx);
        app.package_name = "torch".into();
        app.add_package();
        app.package_bioc = true;
        app.package_name = "edgeR".into();
        app.add_package();
        app.remove_package("dplyr");
        assert!(app.packages.iter().any(|p| p.name == "dplyr"));
        app.pending_id = Some(10);
        app.pending_package = Some(PackageOperation::Query);
        app.remove_package("torch");
        assert!(app.packages.iter().any(|p| p.name == "torch"));
        app.pending_package = Some(PackageOperation::Install("torch".into()));
        app.remove_package("torch");
        assert!(app.packages.iter().any(|p| p.name == "torch"));
        app.remove_package("edgeR");
        assert!(!app.packages.iter().any(|p| p.name == "edgeR"));
        assert!(
            matches!(&app.pending_package, Some(PackageOperation::Install(name)) if name == "torch")
        );
        assert_eq!(app.pending_id, Some(10));
        app.pending_package = Some(PackageOperation::Load("torch".into()));
        app.remove_package("torch");
        assert!(app.packages.iter().any(|p| p.name == "torch"));
        app.pending_id = None;
        app.pending_package = None;
        app.remove_package("torch");
        assert!(!app.packages.iter().any(|p| p.name == "torch"));
    }

    #[test]
    fn remove_button_stays_clear_of_the_expanded_scrollbar_and_remains_clickable() {
        for width in [220.0, 320.0] {
            let ctx = egui::Context::default();
            ctx.style_mut(|style| style.animation_time = 0.0);
            let mut app = app(&ctx);
            app.package_name = "torch".into();
            app.add_package();
            let last = app.packages.len() - 1;
            app.packages.swap(0, last);
            app.package_filter.clear();
            // The unfiltered list overflows this short pane, unlike the single-row
            // views in the original remove-button test.
            package_frame_at_height(&mut app, &ctx, width, 240.0, vec![]);
            let button = package_frame_at_height(&mut app, &ctx, width, 240.0, vec![])[0];
            let scrollbar_left = width - 8.0 - ctx.style().spacing.scroll.bar_width;
            assert!(
                button.right() <= scrollbar_left - 4.0,
                "Remove button overlaps the scrollbar at width {width}: {button:?}"
            );
            package_frame_at_height(
                &mut app,
                &ctx,
                width,
                240.0,
                vec![egui::Event::PointerMoved(egui::pos2(
                    width - 10.0,
                    button.center().y,
                ))],
            );
            for pressed in [true, false] {
                package_frame_at_height(
                    &mut app,
                    &ctx,
                    width,
                    240.0,
                    vec![
                        egui::Event::PointerMoved(button.center()),
                        egui::Event::PointerButton {
                            pos: button.center(),
                            button: egui::PointerButton::Primary,
                            pressed,
                            modifiers: egui::Modifiers::NONE,
                        },
                    ],
                );
            }
            assert!(
                !app.packages.iter().any(|p| p.name == "torch"),
                "Scrollbar intercepted the remove click at width {width}"
            );
        }
    }
}
