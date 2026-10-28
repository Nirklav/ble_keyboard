//! Application state and the top-level frame: menu, screens, dialogs.

use std::path::PathBuf;
use std::time::Duration;

use desktop::EXAMPLE_CONFIG;
use desktop::config_file;
use desktop::link::Options;
use desktop::ops;
use eframe::egui;
use protocol::hid::Info;
use protocol::config::{Config, JoystickButton};

use crate::device::{Command, DeviceWorker, Event};
use crate::files::{Files, Opened};
use crate::theme;
use crate::views;

/// Timeout of every request.
const TIMEOUT: Duration = Duration::from_secs(10);
/// Smallest window that fits the default 10x5 key grid and the combo editor.
pub const MIN_SIZE: egui::Vec2 = egui::vec2(780.0, 540.0);

/// Edited config and its file.
pub struct Document {
    pub config: Config,
    /// File the config was opened from / saved to; in the browser only its name.
    pub path: Option<PathBuf>,
    /// Config as last opened / saved / pulled from / pushed to the keyboard.
    saved: Config,
}

impl Document {
    fn opened(config: Config, path: Option<PathBuf>) -> Document {
        Document { saved: config.clone(), config, path }
    }

    fn example() -> Document {
        let config = config_file::parse(EXAMPLE_CONFIG).expect("the example config is valid");
        Document::opened(config, None)
    }

    pub fn is_dirty(&self) -> bool {
        self.saved != self.config
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
    /// Action of a chord combo, [`ComboAction::Action`](protocol::config::ComboAction::Action).
    ComboAction { combo: usize },
}

pub struct KeyEditor {
    pub target: EditTarget,
    /// Key list filter.
    pub search: String,
    /// The key list should scroll to the current key, once.
    pub scroll_to_current: bool,
}

impl KeyEditor {
    /// Editor of `target`, keeps the search of the `previous` editor.
    pub fn open(target: EditTarget, previous: Option<KeyEditor>) -> KeyEditor {
        let search = previous.map(|e| e.search).unwrap_or_default();
        KeyEditor { target, search, scroll_to_current: true }
    }
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
    /// `None` if the keyboard worker couldn't start.
    pub worker: Option<DeviceWorker>,
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
        let options = Options { timeout: TIMEOUT };
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
    files: Files,
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
    /// `offline` starts in the editor instead of the connect screen, `fake_keyboard` talks to
    /// a fake keyboard instead of Bluetooth.
    pub fn new(ctx: &egui::Context, offline: bool, fake_keyboard: bool) -> App {
        theme::apply(ctx);
        let mut device = DeviceState {
            worker: None,
            info: None,
            connected: false,
            busy: None,
            status: None,
        };
        match DeviceWorker::spawn(ctx.clone(), fake_keyboard) {
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
            files: Files::new(),
            pushing: None,
            quitting: false,
            title: String::new(),
            size: egui::Vec2::ZERO,
        }
    }

    /// Validation result of the edited config for the connected (or the default) keyboard.
    pub fn validation(&self) -> Result<(), String> {
        let info = if self.device.connected { self.device.info.as_ref() } else { None };
        ops::check(&self.doc.config, info)
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
            Discarding::Open => self.files.open(ctx, self.doc.path.as_deref()),
            Discarding::Pull => self.device.send(Command::Pull),
            Discarding::Quit => {
                self.quitting = true;
                ctx.send_viewport_cmd(egui::ViewportCommand::Close);
            }
        }
    }

    fn handle_opened(&mut self, opened: Opened) {
        match opened {
            Ok((config, path)) => self.set_document(Document::opened(config, Some(path))),
            Err(error) => self.message("Can't open the config", error),
        }
    }

    pub fn save(&mut self) {
        self.save_with(false);
    }

    pub fn save_as(&mut self) {
        self.save_with(true);
    }

    /// `ask` for the file even if the config has one.
    fn save_with(&mut self, ask: bool) {
        match self.files.save(self.doc.path.as_deref(), &self.doc.config, ask) {
            Ok(Some(path)) => {
                self.doc.saved = self.doc.config.clone();
                self.doc.path = Some(path);
            }
            Ok(None) => {}
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
                        self.doc.saved = config;
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
            EditTarget::ComboAction { combo } => config.combos.get(combo).is_some_and(|c| c.is_chord()),
        });
        if !editor_valid {
            self.key_editor = None;
        }
    }

    fn handle_shortcuts(&mut self, ctx: &egui::Context) {
        let save = egui::KeyboardShortcut::new(egui::Modifiers::COMMAND, egui::Key::S);
        let save_as = egui::KeyboardShortcut::new(egui::Modifiers::COMMAND | egui::Modifiers::SHIFT, egui::Key::S);
        let open = egui::KeyboardShortcut::new(egui::Modifiers::COMMAND, egui::Key::O);
        if self.dialog.is_some() {
            return;
        }
        // Before Save: its shortcut matches with Shift too.
        if ctx.input_mut(|i| i.consume_shortcut(&save_as)) {
            self.save_as();
        }
        if ctx.input_mut(|i| i.consume_shortcut(&save)) {
            self.save();
        }
        if ctx.input_mut(|i| i.consume_shortcut(&open)) {
            self.discard_then(ctx, Discarding::Open);
        }
    }

    fn handle_close_request(&mut self, ctx: &egui::Context) {
        // The browser has no close request: the page asks before unloading, see main.rs.
        #[cfg(target_arch = "wasm32")]
        crate::UNSAVED.set(self.doc.is_dirty());
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

    /// Header buttons, right to left: the keyboard group (Write, Read, Reset and the connection state;
    /// Connect offline), then the file group.
    pub fn header_actions(&mut self, ui: &mut egui::Ui) {
        let idle = !self.device.is_busy() && self.device.worker.is_some();
        let valid = self.validation().is_ok();
        // Offline the only keyboard action is connecting, without reading: the edited config is written after that.
        let size = egui::vec2(0.0, 30.0);
        if self.device.connected {
            let push = ui
                .add_enabled(idle && valid, theme::primary_button("Write").min_size(size))
                .on_hover_text("Write the edited config to the keyboard")
                .on_disabled_hover_text("Fix the config errors first (see the status bar)");
            if push.clicked() {
                self.push();
            }
            let read = ui
                .add_enabled(idle, egui::Button::new("Read"))
                .on_hover_text("Read the config from the keyboard, replacing the edited one");
            if read.clicked() {
                self.discard_then(ui.ctx(), Discarding::Pull);
            }
            let reset = ui
                .add_enabled(idle, theme::danger_button("Reset"))
                .on_hover_text("Restore the factory config on the keyboard (asks first)");
            if reset.clicked() {
                self.dialog = Some(Dialog::ConfirmReset);
            }
            ui.add_space(theme::GAP_XS);
            theme::status(ui, theme::SUCCESS, "Connected");
        } else {
            let connect = ui
                .add_enabled(idle, theme::primary_button("Connect").min_size(size))
                .on_hover_text("Connect to the keyboard, the edited config stays as it is");
            if connect.clicked() {
                self.device.send(Command::Connect);
            }
            ui.add_space(theme::GAP_XS);
            theme::status(ui, ui.visuals().weak_text_color(), "Offline")
                .on_hover_text("Edits stay in the app until written to the keyboard");
        }

        ui.add_space(theme::GAP_S);
        ui.separator();
        ui.add_space(theme::GAP_S);
        if ui.button("Save as...").on_hover_text("Save the config to another file (Ctrl+Shift+S)").clicked() {
            self.save_as();
        }
        if ui.button("Save").on_hover_text("Save the config to its file (Ctrl+S)").clicked() {
            self.save();
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
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                match validation {
                    Ok(()) => ui.colored_label(theme::SUCCESS, "Config is valid"),
                    Err(error) => ui.colored_label(ui.visuals().error_fg_color, format!("Invalid: {error}")),
                };
                ui.add_space(theme::GAP_L);
                if self.doc.is_dirty() {
                    ui.colored_label(ui.visuals().warn_fg_color, "unsaved changes");
                }
                match &self.doc.path {
                    Some(path) => ui.weak(path.display().to_string()),
                    None => ui.weak("not saved to a file"),
                };
            });
        });
    }

    /// `screen` in the [`theme::column`], with the key editor on its right when `editing`.
    /// The screen keeps its left edge when the editor opens; the empty sides of a wide window still scroll it.
    fn editor_screen(&mut self, ui: &mut egui::Ui, screen: Screen, editing: bool) {
        let rect = ui.available_rect_before_wrap();
        let column = theme::column(rect);
        let mut scroll = rect;
        if editing {
            // Before the screen, so it shows the key just clicked on the grid on the next frame.
            let x = column.right() - views::action_editor::WIDTH;
            let editor = egui::Rect::from_x_y_ranges(x..=column.right(), rect.y_range());
            ui.scope_builder(egui::UiBuilder::new().max_rect(editor), |ui| {
                let frame = egui::Frame::group(ui.style()).inner_margin(theme::GAP_L);
                frame.show(ui, |ui| {
                    ui.set_min_size(ui.available_size());
                    views::action_editor::panel(self, ui);
                });
            });
            self.fix_selection();
            scroll.set_right(x - theme::GAP_L);
        }
        let margin = column.left() - rect.left();
        let width = scroll.right().min(column.right()) - column.left();
        ui.scope_builder(egui::UiBuilder::new().max_rect(scroll), |ui| {
            egui::ScrollArea::both().auto_shrink(false).show(ui, |ui| {
                let content = ui.max_rect();
                let content = egui::Rect::from_x_y_ranges(
                    content.left() + margin..=content.left() + margin + width,
                    content.y_range(),
                );
                ui.scope_builder(egui::UiBuilder::new().max_rect(content), |ui| match screen {
                    Screen::Start => {}
                    Screen::Overview => views::overview::show(self, ui),
                    Screen::Layout(index) => views::layout::show(self, ui, index),
                    Screen::Combos => views::combos::list(self, ui),
                    Screen::Combo(index) => views::combos::show(self, ui, index),
                });
            });
        });
    }

    fn dialogs(&mut self, ctx: &egui::Context) {
        let Some(dialog) = &self.dialog else { return };
        let mut close = false;
        let mut confirmed = false;
        let frame = egui::Frame::popup(&ctx.global_style()).inner_margin(theme::GAP_L);
        let response = egui::Modal::new(egui::Id::unique("dialog")).frame(frame).show(ctx, |ui| {
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
            ui.label(theme::screen_title(title));
            ui.add_space(theme::GAP_S);
            ui.label(text);
            ui.add_space(theme::GAP_L);
            ui.horizontal(|ui| match confirm {
                Some(confirm) => {
                    if ui.add(theme::danger_button(confirm)).clicked() {
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
        if let Some(opened) = self.files.poll() {
            self.handle_opened(opened);
        }
        self.handle_close_request(&ctx);
        self.handle_shortcuts(&ctx);
        // The config may have been replaced or edited outside the views.
        self.fix_selection();

        let validation = self.validation();
        if self.screen != Screen::Start {
            // Above the editor panel: the header buttons need the whole window width.
            let margin = egui::Margin::symmetric(8, theme::GAP_S as i8);
            let frame = egui::Frame::side_top_panel(ui.style()).inner_margin(margin);
            egui::Panel::top("header").frame(frame).show(ui, |ui| theme::centered(ui, |ui| views::header(self, ui)));
            egui::Panel::bottom("status").show(ui, |ui| theme::centered(ui, |ui| self.status_bar(ui, &validation)));
        }
        let editing = self.key_editor.as_ref().map(|e| e.target);
        egui::CentralPanel::default_margins().show(ui, |ui| match self.screen {
            Screen::Start => views::start::show(self, ui),
            screen => self.editor_screen(ui, screen, editing.is_some()),
        });

        self.fix_selection();
        // The editor panel is already drawn: show the key just clicked on the central panel.
        if self.key_editor.as_ref().map(|e| e.target) != editing {
            ctx.request_repaint();
        }
        self.dialogs(&ctx);
        self.update_title(&ctx);
        self.enforce_min_size(&ctx);
    }
}
