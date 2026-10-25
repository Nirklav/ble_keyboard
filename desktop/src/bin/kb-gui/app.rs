//! Application state and the top-level frame: menu, screens, dialogs.

use std::path::{Path, PathBuf};
use std::time::Duration;

use desktop::EXAMPLE_CONFIG;
use desktop::config_file;
use desktop::transport::Options;
use eframe::egui;
use protocol::ble::Info;
use protocol::config::{Config, JoystickButton};

use crate::device::{Command, DeviceWorker, Event};
use crate::editing;
use crate::views;

/// Default keyboard name filter, same as the CLI one.
const DEFAULT_NAME: &str = "Nirklav Keyboard";
/// Timeout of discovery and of every request.
const TIMEOUT: Duration = Duration::from_secs(10);
/// Smallest window that fits the default 10x5 key grid and the combo editor.
pub const MIN_SIZE: egui::Vec2 = egui::vec2(780.0, 540.0);

/// Edited config and its file.
pub struct Document {
    pub config: Config,
    /// File the config was opened from / saved to.
    pub path: Option<PathBuf>,
    /// Config as last opened / saved / pulled from / pushed to the keyboard.
    saved: Option<Config>,
}

impl Document {
    fn opened(config: Config, path: Option<PathBuf>) -> Document {
        Document { saved: Some(config.clone()), config, path }
    }

    fn example() -> Document {
        let config = config_file::parse(EXAMPLE_CONFIG).expect("the example config is valid");
        Document::opened(config, None)
    }

    pub fn is_dirty(&self) -> bool {
        self.saved.as_ref() != Some(&self.config)
    }

    fn title(&self) -> String {
        let name = match &self.path {
            Some(path) => path.file_name().unwrap_or(path.as_os_str()).to_string_lossy().into_owned(),
            None => "untitled".to_string(),
        };
        let dirty = if self.is_dirty() { " *" } else { "" };
        format!("{name}{dirty} - Keyboard config")
    }
}

/// Screen of the wizard-like navigation.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Screen {
    /// Nothing but the connect button.
    Start,
    /// Keyboard info, list of layouts, link to the combos.
    Overview,
    /// Key grid of a layout.
    Layout(usize),
    /// List of combos.
    Combos,
    /// One combo, `None` for the built-in recovery combo.
    Combo(Option<usize>),
}

impl Screen {
    /// Screen the "Back" button leads to.
    pub fn parent(self) -> Screen {
        match self {
            Screen::Start | Screen::Overview => Screen::Start,
            Screen::Layout(_) | Screen::Combos => Screen::Overview,
            Screen::Combo(_) => Screen::Combos,
        }
    }
}

/// What the key action editor window edits.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum EditTarget {
    LayoutKey { layout: usize, x: usize, y: usize },
    JoystickButton { layout: usize, button: JoystickButton },
    ComboKey { combo: usize, key: usize },
}

pub struct KeyEditor {
    pub target: EditTarget,
    /// Key list filter.
    pub search: String,
}

/// Action that drops the current document, waiting for "discard changes?".
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Discarding {
    NewFromExample,
    Open,
    Pull,
    Quit,
}

pub enum Dialog {
    Discard(Discarding),
    ConfirmReset,
    Message { title: String, text: String },
}

/// Keyboard connection state as seen by the UI.
pub struct DeviceState {
    /// `None` if the Bluetooth worker couldn't start.
    pub worker: Option<DeviceWorker>,
    /// Keyboard name filter.
    pub name: String,
    /// Info of the last connected keyboard.
    pub info: Option<Info>,
    pub connected: bool,
    /// Status text of the running command, `None` if idle.
    pub busy: Option<String>,
    /// Result of the last command, `true` for errors.
    pub status: Option<(bool, String)>,
}

impl DeviceState {
    pub fn send(&mut self, command: Command) {
        let Some(worker) = &self.worker else { return };
        let options = Options { name: self.name.trim().to_string(), timeout: TIMEOUT, mtu: None };
        self.busy = Some("Starting...".to_string());
        worker.send(options, command);
    }

    pub fn is_busy(&self) -> bool {
        self.busy.is_some()
    }

    pub fn set_status(&mut self, error: bool, text: impl Into<String>) {
        self.status = Some((error, text.into()));
    }
}

pub struct App {
    pub doc: Document,
    pub device: DeviceState,
    pub screen: Screen,
    /// Layout shown on the combo grid.
    pub grid_layout: usize,
    pub key_editor: Option<KeyEditor>,
    /// Combo key waiting for a grid click: `(combo, key)`.
    pub picking: Option<(usize, usize)>,
    pub dialog: Option<Dialog>,
    /// Config being written to the keyboard, becomes the saved one once written.
    pushing: Option<Config>,
    /// Quit was confirmed, don't ask again.
    quitting: bool,
    /// Last title sent to the window.
    title: String,
    /// Last window size checked against [`MIN_SIZE`].
    size: egui::Vec2,
}

impl App {
    /// `offline` starts in the editor instead of the connect screen.
    pub fn new(ctx: &egui::Context, offline: bool) -> App {
        let mut device = DeviceState {
            worker: None,
            name: DEFAULT_NAME.to_string(),
            info: None,
            connected: false,
            busy: None,
            status: None,
        };
        match DeviceWorker::spawn(ctx.clone()) {
            Ok(worker) => device.worker = Some(worker),
            Err(error) => device.set_status(true, format!("{error:#}")),
        }

        App {
            doc: Document::example(),
            device,
            screen: if offline { Screen::Overview } else { Screen::Start },
            grid_layout: 0,
            key_editor: None,
            picking: None,
            dialog: None,
            pushing: None,
            quitting: false,
            title: String::new(),
            size: egui::Vec2::ZERO,
        }
    }

    /// Validation result of the edited config for the connected (or the default) keyboard.
    pub fn validation(&self) -> Result<(), String> {
        let info = if self.device.connected { self.device.info.as_ref() } else { None };
        editing::check(&self.doc.config, info)
    }

    /// Also goes back to the overview.
    fn set_document(&mut self, doc: Document) {
        self.doc = doc;
        self.pushing = None;
        self.grid_layout = 0;
        self.go(Screen::Overview);
    }

    /// Closes the editors of the old screen.
    pub fn go(&mut self, screen: Screen) {
        if screen != self.screen {
            self.screen = screen;
            self.key_editor = None;
            self.picking = None;
        }
    }

    /// Writes the edited config to the keyboard.
    pub fn push(&mut self) {
        self.pushing = Some(self.doc.config.clone());
        self.device.send(Command::Push(Box::new(self.doc.config.clone())));
    }

    /// Runs `action` now, or asks first if there are unsaved changes.
    pub fn discard_then(&mut self, ctx: &egui::Context, action: Discarding) {
        if self.doc.is_dirty() {
            self.dialog = Some(Dialog::Discard(action));
        } else {
            self.run_discarding(ctx, action);
        }
    }

    fn run_discarding(&mut self, ctx: &egui::Context, action: Discarding) {
        match action {
            Discarding::NewFromExample => self.set_document(Document::example()),
            Discarding::Open => self.open(),
            Discarding::Pull => self.device.send(Command::Pull),
            Discarding::Quit => {
                self.quitting = true;
                ctx.send_viewport_cmd(egui::ViewportCommand::Close);
            }
        }
    }

    fn open(&mut self) {
        let Some(path) = file_dialog(self.doc.path.as_deref()).pick_file() else {
            return;
        };
        match config_file::load(&path) {
            Ok(config) => self.set_document(Document::opened(config, Some(path))),
            Err(error) => self.message("Can't open the config", format!("{error:#}")),
        }
    }

    pub fn save(&mut self) {
        match self.doc.path.clone() {
            Some(path) => self.save_to(path),
            None => self.save_as(),
        }
    }

    pub fn save_as(&mut self) {
        let dialog = file_dialog(self.doc.path.as_deref()).set_file_name(
            self.doc.path.as_ref().and_then(|p| p.file_name()).map_or("config.toml".into(), |n| n.to_string_lossy()),
        );
        if let Some(path) = dialog.save_file() {
            self.save_to(path);
        }
    }

    fn save_to(&mut self, path: PathBuf) {
        match config_file::save(&path, &self.doc.config) {
            Ok(()) => {
                self.doc.saved = Some(self.doc.config.clone());
                self.doc.path = Some(path);
            }
            Err(error) => self.message("Can't save the config", format!("{error:#}")),
        }
    }

    pub fn message(&mut self, title: impl Into<String>, text: impl Into<String>) {
        self.dialog = Some(Dialog::Message { title: title.into(), text: text.into() });
    }

    fn handle_device_events(&mut self) {
        let Some(worker) = &self.device.worker else {
            return;
        };
        let events: Vec<Event> = worker.poll().collect();
        for event in events {
            if event.is_final() {
                self.device.busy = None;
            }
            match event {
                Event::Progress(text) => self.device.busy = Some(text),
                Event::Connected(info) => {
                    self.device.info = Some(info);
                    self.device.connected = true;
                }
                Event::Done(text) => self.device.set_status(false, text),
                Event::Disconnected => {
                    self.device.connected = false;
                    self.device.status = None;
                    self.go(Screen::Start);
                }
                Event::Pulled(config) => {
                    self.device.set_status(
                        false,
                        format!("Read {} layouts, {} combos", config.layouts.len(), config.combos.len()),
                    );
                    self.set_document(Document::opened(*config, None));
                }
                Event::Pushed { bytes } => {
                    if let Some(config) = self.pushing.take() {
                        self.doc.saved = Some(config);
                    }
                    self.device.set_status(false, format!("Config applied ({bytes} bytes)"));
                }
                Event::Failed(error) => {
                    self.pushing = None;
                    // A lost connection leads back to the start screen, like a disconnect.
                    if std::mem::take(&mut self.device.connected) {
                        self.go(Screen::Start);
                    }
                    // The start screen shows the error itself.
                    if self.screen != Screen::Start {
                        self.message("Keyboard error", error.clone());
                    }
                    self.device.set_status(true, error);
                }
            }
        }
    }

    /// Keeps the screen and the editor state pointing to existing layouts / combos after edits.
    fn fix_selection(&mut self) {
        let config = &self.doc.config;
        match self.screen {
            Screen::Layout(index) if index >= config.layouts.len() => self.go(Screen::Overview),
            Screen::Combo(Some(index)) if index >= config.combos.len() => self.go(Screen::Combos),
            _ => {}
        }
        let config = &self.doc.config;
        self.grid_layout = self.grid_layout.min(config.layouts.len().saturating_sub(1));
        let picking_valid = self.picking.is_none_or(|(c, k)| {
            self.screen == Screen::Combo(Some(c)) && config.combos.get(c).is_some_and(|combo| k < combo.keys.len())
        });
        if !picking_valid {
            self.picking = None;
        }
        let editor_valid = self.key_editor.as_ref().is_none_or(|editor| match editor.target {
            EditTarget::LayoutKey { layout, x, y } => config.layouts.get(layout).and_then(|l| l.get(x, y)).is_some(),
            EditTarget::JoystickButton { layout, .. } => layout < config.layouts.len(),
            EditTarget::ComboKey { combo, key } => config.combos.get(combo).is_some_and(|c| key < c.keys.len()),
        });
        if !editor_valid {
            self.key_editor = None;
        }
    }

    fn handle_shortcuts(&mut self, ctx: &egui::Context) {
        let save = egui::KeyboardShortcut::new(egui::Modifiers::COMMAND, egui::Key::S);
        let open = egui::KeyboardShortcut::new(egui::Modifiers::COMMAND, egui::Key::O);
        if self.dialog.is_some() {
            return;
        }
        if ctx.input_mut(|i| i.consume_shortcut(&save)) {
            self.save();
        }
        if ctx.input_mut(|i| i.consume_shortcut(&open)) {
            self.discard_then(ctx, Discarding::Open);
        }
    }

    fn handle_close_request(&mut self, ctx: &egui::Context) {
        if ctx.input(|i| i.viewport().close_requested()) && !self.quitting && self.doc.is_dirty() {
            ctx.send_viewport_cmd(egui::ViewportCommand::CancelClose);
            self.dialog = Some(Dialog::Discard(Discarding::Quit));
        }
    }

    /// Grows the window back to [`MIN_SIZE`]: WSLg ignores the min size hint.
    fn enforce_min_size(&mut self, ctx: &egui::Context) {
        let Some(size) = ctx.input(|i| i.viewport().inner_rect.map(|r| r.size())) else { return };
        // Once per resize, in case the window manager refuses.
        if size == self.size {
            return;
        }
        self.size = size;
        if size.x < MIN_SIZE.x || size.y < MIN_SIZE.y {
            ctx.send_viewport_cmd(egui::ViewportCommand::InnerSize(size.max(MIN_SIZE)));
        }
    }

    fn update_title(&mut self, ctx: &egui::Context) {
        let title = self.doc.title();
        if title != self.title {
            ctx.send_viewport_cmd(egui::ViewportCommand::Title(title.clone()));
            self.title = title;
        }
    }

    fn menu(&mut self, ui: &mut egui::Ui) {
        egui::MenuBar::new().ui(ui, |ui| {
            ui.menu_button("File", |ui| {
                if ui.button("New from example").clicked() {
                    self.discard_then(ui.ctx(), Discarding::NewFromExample);
                }
                if ui.button("Open...").clicked() {
                    self.discard_then(ui.ctx(), Discarding::Open);
                }
                if ui.button("Save").clicked() {
                    self.save();
                }
                if ui.button("Save as...").clicked() {
                    self.save_as();
                }
                ui.separator();
                if ui.button("Quit").clicked() {
                    self.discard_then(ui.ctx(), Discarding::Quit);
                }
            });
            ui.separator();
            let path = match &self.doc.path {
                Some(path) => path.display().to_string(),
                None => "untitled".to_string(),
            };
            ui.label(path);
            if self.doc.is_dirty() {
                ui.colored_label(ui.visuals().warn_fg_color, "unsaved changes");
            }
        });
    }

    /// "Write to keyboard" and the connection state, right to left.
    pub fn push_button(&mut self, ui: &mut egui::Ui) {
        let idle = !self.device.is_busy() && self.device.worker.is_some();
        let valid = self.validation().is_ok();
        let push = ui
            .add_enabled(idle && valid, egui::Button::new("Write to keyboard").min_size(egui::vec2(0.0, 28.0)))
            .on_hover_text("Write the edited config to the keyboard (connects first if needed)")
            .on_disabled_hover_text("Fix the config errors first (see the status bar)");
        if push.clicked() {
            self.push();
        }
        if self.device.connected {
            ui.colored_label(egui::Color32::from_rgb(0, 160, 0), "connected");
        } else {
            ui.weak("not connected");
        }
    }

    fn status_bar(&self, ui: &mut egui::Ui, validation: &Result<(), String>) {
        ui.horizontal(|ui| {
            if let Some(busy) = &self.device.busy {
                ui.spinner();
                ui.label(busy);
            } else if let Some((error, text)) = &self.device.status {
                if *error {
                    ui.colored_label(ui.visuals().error_fg_color, text);
                } else {
                    ui.label(text);
                }
            }
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| match validation {
                Ok(()) => ui.colored_label(egui::Color32::from_rgb(0, 160, 0), "Config is valid"),
                Err(error) => ui.colored_label(ui.visuals().error_fg_color, format!("Invalid: {error}")),
            });
        });
    }

    fn dialogs(&mut self, ctx: &egui::Context) {
        let Some(dialog) = &self.dialog else { return };
        let mut close = false;
        let mut confirmed = false;
        let response = egui::Modal::new(egui::Id::unique("dialog")).show(ctx, |ui| {
            ui.set_max_width(420.0);
            let (title, text, confirm) = match dialog {
                Dialog::Discard(_) => {
                    ("Unsaved changes", "The config has unsaved changes. Discard them?", Some("Discard"))
                }
                Dialog::ConfirmReset => (
                    "Reset the keyboard",
                    "Restore the factory config on the keyboard? The current keyboard config is lost.",
                    Some("Reset"),
                ),
                Dialog::Message { title, text } => (title.as_str(), text.as_str(), None),
            };
            ui.heading(title);
            ui.add_space(4.0);
            ui.label(text);
            ui.add_space(8.0);
            ui.horizontal(|ui| match confirm {
                Some(confirm) => {
                    if ui.button(confirm).clicked() {
                        confirmed = true;
                    }
                    if ui.button("Cancel").clicked() {
                        close = true;
                    }
                }
                None => {
                    if ui.button("OK").clicked() {
                        close = true;
                    }
                }
            });
        });
        if response.should_close() {
            close = true;
        }

        if confirmed {
            match self.dialog.take() {
                Some(Dialog::Discard(action)) => self.run_discarding(ctx, action),
                Some(Dialog::ConfirmReset) => self.device.send(Command::Reset),
                _ => {}
            }
        } else if close {
            self.dialog = None;
        }
    }
}

impl eframe::App for App {
    fn on_exit(&mut self, _gl: Option<&eframe::glow::Context>) {
        // Ends the session, so the keyboard may deep sleep again.
        self.device.worker = None;
    }

    fn ui(&mut self, ui: &mut egui::Ui, _frame: &mut eframe::Frame) {
        let ctx = ui.ctx().clone();
        self.handle_device_events();
        self.handle_close_request(&ctx);
        self.handle_shortcuts(&ctx);
        // The config may have been replaced or edited outside the views.
        self.fix_selection();

        let validation = self.validation();
        if self.screen != Screen::Start {
            egui::Panel::top("menu").show(ui, |ui| self.menu(ui));
            egui::Panel::bottom("status").show(ui, |ui| self.status_bar(ui, &validation));
        }
        egui::CentralPanel::default_margins().show(ui, |ui| match self.screen {
            Screen::Start => views::start::show(self, ui),
            screen => {
                views::header(self, ui);
                egui::ScrollArea::both().auto_shrink(false).show(ui, |ui| match screen {
                    Screen::Start => {}
                    Screen::Overview => views::overview::show(self, ui),
                    Screen::Layout(index) => views::layout::show(self, ui, index),
                    Screen::Combos => views::combos::list(self, ui),
                    Screen::Combo(index) => views::combos::show(self, ui, index),
                });
            }
        });

        self.fix_selection();
        views::action_editor::window(self, &ctx);
        self.fix_selection();
        self.dialogs(&ctx);
        self.update_title(&ctx);
        self.enforce_min_size(&ctx);
    }
}

fn file_dialog(current: Option<&Path>) -> rfd::FileDialog {
    let dialog = rfd::FileDialog::new().add_filter("TOML config", &["toml"]).add_filter("All files", &["*"]);
    match current.and_then(Path::parent) {
        Some(dir) if !dir.as_os_str().is_empty() => dialog.set_directory(dir),
        _ => dialog,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn document_dirty_state() {
        let mut doc = Document::example();
        assert!(!doc.is_dirty());
        assert_eq!(doc.title(), "untitled - Keyboard config");

        doc.config.layouts[0].name = "OTHER".to_string();
        assert!(doc.is_dirty());
        assert_eq!(doc.title(), "untitled * - Keyboard config");


        let opened = Document::opened(doc.config, Some(PathBuf::from("dir/my.toml")));
        assert_eq!(opened.title(), "my.toml - Keyboard config");
    }
}
