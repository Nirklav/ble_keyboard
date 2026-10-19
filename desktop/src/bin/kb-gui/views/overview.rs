//! Overview: keyboard info and actions, layouts and the way to the combos.

use eframe::egui;
use protocol::ble;
use protocol::config::{CONFIG_VERSION, MAX_LAYOUTS};

use crate::app::{App, Dialog, Discarding, Screen};
use crate::editing;
use crate::views::{NAV_SIZE, nav_button};

pub fn show(app: &mut App, ui: &mut egui::Ui) {
    keyboard(app, ui);
    ui.add_space(16.0);

    ui.heading("Layouts");
    ui.add_space(4.0);
    layouts(app, ui);
    ui.add_space(16.0);

    ui.heading("Combos");
    ui.add_space(4.0);
    let count = app.doc.config.combos.len();
    let subtitle = format!("{count} configured + factory reset");
    if nav_button(ui, NAV_SIZE, "Special combos", &subtitle).clicked() {
        app.go(Screen::Combos);
    }
}

/// Info of the connected keyboard and the config actions.
fn keyboard(app: &mut App, ui: &mut egui::Ui) {
    let idle = !app.device.is_busy() && app.device.worker.is_some();
    let device = &app.device;
    match (&device.info, device.connected) {
        (Some(info), true) => {
            let warn = ui.visuals().warn_fg_color;
            egui::Grid::new("device_info").num_columns(2).striped(true).show(ui, |ui| {
                ui.label("Firmware");
                ui.label(&info.firmware_version);
                ui.end_row();
                ui.label("Protocol");
                if info.protocol_version == ble::PROTOCOL_VERSION {
                    ui.label(info.protocol_version.to_string());
                } else {
                    ui.colored_label(warn, format!("{} (app: {})", info.protocol_version, ble::PROTOCOL_VERSION));
                }
                ui.end_row();
                ui.label("Config format");
                if info.config_version == CONFIG_VERSION {
                    ui.label(info.config_version.to_string());
                } else {
                    ui.colored_label(warn, format!("{} (app: {CONFIG_VERSION})", info.config_version));
                }
                ui.end_row();
                ui.label("Keys");
                ui.label(format!("{}x{}", info.width, info.height));
                ui.end_row();
                ui.label("Max layouts / combos");
                ui.label(format!("{} / {}", info.max_layouts, info.max_combos));
                ui.end_row();
            });
        }
        _ => {
            ui.weak("Not connected, the config is edited offline.");
        }
    }
    ui.add_space(4.0);

    ui.horizontal(|ui| {
        let pull = if app.device.connected { "Read again" } else { "Connect and read" };
        let pull = ui
            .add_enabled(idle, egui::Button::new(pull))
            .on_hover_text("Read the config from the keyboard, replacing the edited one");
        if pull.clicked() {
            app.discard_then(ui.ctx(), Discarding::Pull);
        }
        let reset = ui
            .add_enabled(idle, egui::Button::new("Factory reset..."))
            .on_hover_text("Restore the factory config on the keyboard");
        if reset.clicked() {
            app.dialog = Some(Dialog::ConfirmReset);
        }
    });
}

/// Layout buttons and "Add".
fn layouts(app: &mut App, ui: &mut egui::Ui) {
    ui.horizontal_wrapped(|ui| {
        let mut open = None;
        for (index, layout) in app.doc.config.layouts.iter().enumerate() {
            let subtitle =
                if index == 0 { "Layout 0, active after boot".to_string() } else { format!("Layout {index}") };
            if nav_button(ui, NAV_SIZE, &layout.name, &subtitle).clicked() {
                open = Some(index);
            }
        }
        if let Some(index) = open {
            app.go(Screen::Layout(index));
        }

        let can_add = app.doc.config.layouts.len() < MAX_LAYOUTS;
        let add = ui
            .add_enabled(can_add, egui::Button::new("+ Add").min_size(egui::vec2(64.0, NAV_SIZE.y)))
            .on_hover_text("New empty layout")
            .on_disabled_hover_text(format!("At most {MAX_LAYOUTS} layouts"));
        if add.clicked() {
            let config = &mut app.doc.config;
            let layout = editing::empty_layout(config, editing::unique_layout_name(config));
            match editing::add_layout(config, layout) {
                Ok(index) => app.go(Screen::Layout(index)),
                Err(error) => app.message("Can't add a layout", error),
            }
        }
    });
}
