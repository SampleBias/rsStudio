mod editor;
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
            Ok(Box::new(App::new(prefs)))
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

struct App {
    session: Option<Session>,
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
    find_text: String,
    files_tab: bool,
    submitted_from_source: bool,
    state: State,
    executable: String,
    directory: String,
    runtime: String,
    output: String,
    input: String,
    submitted: String,
    next_id: u64,
    pending_id: Option<u64>,
    confirm_restart: bool,
}
impl App {
    fn new(prefs: settings::Settings) -> Self {
        let mut app = Self {
            session: None,
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
            find_text: String::new(),
            files_tab: false,
            submitted_from_source: false,
            state: State::Stopped,
            executable: String::new(),
            directory: std::env::current_dir()
                .unwrap_or_else(|_| PathBuf::from("."))
                .display()
                .to_string(),
            runtime: String::new(),
            output: String::new(),
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
                    self.submitted.clear();
                    self.state = State::Ready;
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
                    self.submitted.clear();
                    self.append(&format!("Error: {message}\n"));
                    self.state = State::Ready;
                }
                Event::Failed(message) => {
                    if self.pending_id.take().is_some()
                        && !self.submitted_from_source
                        && self.input.is_empty()
                    {
                        self.input = std::mem::take(&mut self.submitted);
                    }
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
        let previous_settings = (self.dark, self.font_size);
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
                ui.heading("rsStudio");
                ui.separator();
                let (name, color) = match self.state {
                    State::Starting => ("Starting", if self.dark { Color32::YELLOW } else { Color32::from_rgb(120, 80, 0) }),
                    State::Ready => ("Ready", if self.dark { Color32::LIGHT_GREEN } else { Color32::from_rgb(0, 105, 45) }),
                    State::Busy => ("Running", if self.dark { Color32::YELLOW } else { Color32::from_rgb(120, 80, 0) }),
                    State::Failed => ("Failed", if self.dark { Color32::LIGHT_RED } else { Color32::from_rgb(165, 30, 30) }),
                    State::Stopped => ("Stopped", if self.dark { Color32::GRAY } else { Color32::DARK_GRAY }),
                };
                ui.label(RichText::new(format!("● {name}")).color(color));
                if !self.runtime.is_empty() { ui.label(&self.runtime); }
                if ui.button("Restart R…").clicked() { self.confirm_restart = true; }
                if ui.button("Clear output").clicked() { self.output.clear(); }
                if ui.button(if self.dark { "Light theme" } else { "Dark theme" }).clicked() {
                    self.dark = !self.dark;
                    settings::Settings { dark: self.dark, font_size: self.font_size }.apply(ctx);
                }
            });
            ui.horizontal(|ui| {
                ui.label("R executable:");
                ui.add(egui::TextEdit::singleline(&mut self.executable).hint_text("R on PATH").desired_width(180.0));
                ui.label("Working directory:");
                ui.add(egui::TextEdit::singleline(&mut self.directory).desired_width(260.0));
                if ui.button("Apply & restart…").clicked() { self.confirm_restart = true; }
            });
            ui.small("R runs with --vanilla: startup profiles and saved workspaces are not loaded. Changes to the R path or directory apply on restart.");
        });
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
                    if ui.selectable_label(!self.files_tab, "Plots").clicked() {
                        self.files_tab = false;
                    }
                    if ui.selectable_label(self.files_tab, "Files").clicked() {
                        self.files_tab = true;
                    }
                    ui.weak("Data");
                });
                ui.separator();
                if self.files_tab {
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
                } else {
                    ui.add_space(8.0);
                    ui.label("Plots will appear here in a later phase.");
                }
            });

        egui::TopBottomPanel::bottom("console")
            .resizable(true)
            .default_height(300.0)
            .min_height(170.0)
            .show(ctx, |ui| {
                // Reserve the bottom of the console for its input, leaving the
                // remaining area for scrollable output.
                egui::TopBottomPanel::bottom("console_input").show_inside(ui, |ui| {
                    ui.horizontal(|ui| {
                        ui.strong("R input");
                        if ui
                            .add_enabled(
                                self.state == State::Ready,
                                egui::Button::new("Run  Ctrl+Enter"),
                            )
                            .clicked()
                        {
                            self.submit();
                        }
                    });
                    let input = ui.add_enabled(
                        self.state == State::Ready,
                        egui::TextEdit::multiline(&mut self.input)
                            .font(egui::TextStyle::Monospace)
                            .desired_rows(2)
                            .desired_width(f32::INFINITY),
                    );
                    if input.has_focus()
                        && ctx.input(|i| i.modifiers.ctrl && i.key_pressed(egui::Key::Enter))
                    {
                        self.submit();
                    }
                });

                ui.heading("Console");
                ui.weak("Submission console: interactive terminal input is not supported.");
                ui.separator();
                egui::ScrollArea::vertical()
                    .stick_to_bottom(true)
                    .show(ui, |ui| {
                        ui.monospace(&self.output);
                    });
            });

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
        if previous_settings != (self.dark, self.font_size) {
            let prefs = settings::Settings {
                dark: self.dark,
                font_size: self.font_size,
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
