//! First screen: the connect button, the ways to edit without the keyboard, the connection options.

use eframe::egui;

use crate::app::{App, Discarding};
use crate::theme;

/// Width of the column in the middle: the connect button, the other buttons and the options.
const WIDTH: f32 = 260.0;
/// Height of the connect button.
const CONNECT_HEIGHT: f32 = 48.0;

pub fn show(app: &mut App, ui: &mut egui::Ui) {
    ui.vertical_centered(|ui| {
        ui.add_space((ui.available_height() * 0.3 - 80.0).max(0.0));
        ui.label(egui::RichText::new("Keyboard config").size(28.0).strong());
        ui.add_space(theme::GAP_XS);
        ui.weak("Connect to the keyboard over Bluetooth to read its config.");
        ui.add_space(theme::GAP_XL);

        let idle = !app.device.is_busy() && app.device.worker.is_some();
        let connect = theme::primary_button("Connect").min_size(egui::vec2(WIDTH, CONNECT_HEIGHT));
        let connect = ui.add_enabled(idle, connect).on_hover_text("Connect to the keyboard and read its config");
        if connect.clicked() {
            app.discard_then(ui.ctx(), Discarding::Pull);
        }
        ui.add_space(theme::GAP_S);

        // Spinner above the text: a horizontal row wouldn't be centered.
        let device = &app.device;
        if let Some(busy) = &device.busy {
            ui.spinner();
            ui.label(busy);
        } else if device.worker.is_none() {
            ui.colored_label(ui.visuals().error_fg_color, "Bluetooth worker is not running");
        } else if let Some((true, error)) = &device.status {
            ui.colored_label(ui.visuals().error_fg_color, error);
        }
        ui.add_space(theme::GAP_L);

        // A fixed width column in the middle, so its rows don't jump to the left edge.
        let margin = ((ui.available_width() - WIDTH) / 2.0).max(0.0);
        ui.horizontal(|ui| {
            ui.add_space(margin);
            ui.vertical(|ui| {
                ui.set_width(WIDTH);
                offline(app, ui, idle);
                ui.add_space(theme::GAP_L);
                options(app, ui, idle);
            });
        });
    });
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

fn options(app: &mut App, ui: &mut egui::Ui, idle: bool) {
    egui::CollapsingHeader::new("Connection options").show_unindented(ui, |ui| {
        ui.label("Keyboard name:");
        ui.add_enabled(idle, egui::TextEdit::singleline(&mut app.device.name).desired_width(f32::INFINITY))
            .on_hover_text("Case-insensitive part of the keyboard name, used if it isn't found by its config service");
    });
}
