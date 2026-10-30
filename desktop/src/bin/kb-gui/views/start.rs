//! First screen: the connect button, the ways to edit without the keyboard.

use eframe::egui;

use desktop::{compat, firmware};

use crate::app::{App, Dialog, Discarding, Keyboard};
use crate::theme;

/// Width of the column in the middle: the connect button and the other buttons.
const WIDTH: f32 = 260.0;
/// Height of the connect button.
const CONNECT_HEIGHT: f32 = 48.0;

pub fn show(app: &mut App, ui: &mut egui::Ui) {
    ui.vertical_centered(|ui| {
        ui.add_space((ui.available_height() * 0.3 - 80.0).max(0.0));
        ui.label(egui::RichText::new("Keyboard config").size(28.0).strong());
        ui.add_space(theme::GAP_XS);
        ui.weak("Connect to the keyboard (paired with this computer) to read its config.");
        ui.add_space(theme::GAP_XL);

        let idle = !app.device.is_busy() && app.device.worker.is_some();
        let connect = theme::primary_button("Connect").min_size(egui::vec2(WIDTH, CONNECT_HEIGHT));
        let connect = ui.add_enabled(idle, connect).on_hover_text("Connect to the keyboard and read its config");
        if connect.clicked() {
            app.discard_then(ui.ctx(), Discarding::Pull);
        }
        ui.add_space(theme::GAP_S);

        let device = &app.device;
        if let Some(busy) = &device.busy {
            theme::busy_centered(ui, busy, WIDTH);
        } else if device.worker.is_none() {
            ui.colored_label(ui.visuals().error_fg_color, "Keyboard worker is not running");
        } else if let Some((error, text)) = &device.status {
            // Not an error here: the result of an update from this screen (the others are cleared on the way here).
            let color = if *error { ui.visuals().error_fg_color } else { ui.visuals().text_color() };
            ui.colored_label(color, text);
        }
        if !device.is_busy() {
            update(app, ui, idle);
        }
        ui.add_space(theme::GAP_L);

        // A fixed width column in the middle, so its rows don't jump to the left edge.
        let margin = ((ui.available_width() - WIDTH) / 2.0).max(0.0);
        ui.horizontal(|ui| {
            ui.add_space(margin);
            ui.vertical(|ui| {
                ui.set_width(WIDTH);
                offline(app, ui, idle);
            });
        });
    });
}

/// Update to the bundled firmware for a keyboard whose firmware the app can't edit.
fn update(app: &mut App, ui: &mut egui::Ui, idle: bool) {
    let (Keyboard::Incompatible(versions), Some(bundled)) = (&app.device.keyboard, firmware::bundled_firmware()) else {
        return;
    };
    ui.add_space(theme::GAP_S);
    let compatible = compat::check_update(versions, bundled);
    let button = ui.add_enabled(idle && compatible.is_ok(), egui::Button::new(format!("Update firmware to {}", bundled.version)));
    let button = match &compatible {
        Ok(()) => button.on_hover_text("Install the firmware bundled with the app (asks first)"),
        Err(error) => button.on_disabled_hover_text(format!("{error:#}")),
    };
    if button.clicked() {
        app.dialog = Some(Dialog::ConfirmUpdate(bundled.version.clone()));
    }
}

/// Ways to edit a config without the keyboard.
fn offline(app: &mut App, ui: &mut egui::Ui, idle: bool) {
    ui.horizontal(|ui| {
        let size = egui::vec2((WIDTH - ui.spacing().item_spacing.x) / 2.0, 0.0);
        let open = ui.add_enabled(idle, egui::Button::new("Open file...").min_size(size));
        if open.on_hover_text("Edit a config file, write it to the keyboard later").clicked() {
            app.discard_then(ui.ctx(), Discarding::Open);
        }
        let create = ui.add_enabled(idle, egui::Button::new("Create new").min_size(size));
        if create.on_hover_text("Start from the example config, write it to the keyboard later").clicked() {
            app.discard_then(ui.ctx(), Discarding::NewFromExample);
        }
    });
}
