//! First screen: the keyboard name and the connect button.

use eframe::egui;

use crate::app::{App, Discarding};

/// Size of the connect button, the options are as wide.
const CONNECT_SIZE: egui::Vec2 = egui::vec2(220.0, 48.0);

pub fn show(app: &mut App, ui: &mut egui::Ui) {
    ui.vertical_centered(|ui| {
        ui.add_space((ui.available_height() * 0.3 - 60.0).max(0.0));
        ui.label(egui::RichText::new("Keyboard config").size(24.0).strong());
        ui.add_space(16.0);

        let idle = !app.device.is_busy() && app.device.worker.is_some();
        let connect = egui::Button::new(egui::RichText::new("Connect").size(20.0)).min_size(CONNECT_SIZE);
        let connect = ui.add_enabled(idle, connect).on_hover_text("Connect to the keyboard and read its config");
        if connect.clicked() {
            app.discard_then(ui.ctx(), Discarding::Pull);
        }
        ui.add_space(8.0);

        let device = &app.device;
        if let Some(busy) = &device.busy {
            ui.horizontal(|ui| {
                ui.spinner();
                ui.label(busy);
            });
        } else if device.worker.is_none() {
            ui.colored_label(ui.visuals().error_fg_color, "Bluetooth worker is not running");
        } else if let Some((true, error)) = &device.status {
            ui.colored_label(ui.visuals().error_fg_color, error);
        }
        ui.add_space(16.0);

        // A fixed width column in the middle, so the options don't jump to the left edge.
        let width = CONNECT_SIZE.x;
        let margin = ((ui.available_width() - width) / 2.0).max(0.0);
        ui.horizontal(|ui| {
            ui.add_space(margin);
            ui.vertical(|ui| {
                ui.set_width(width);
                options(app, ui, idle);
            });
        });
    });
}

fn options(app: &mut App, ui: &mut egui::Ui, idle: bool) {
    egui::CollapsingHeader::new("Options").show_unindented(ui, |ui| {
        ui.label("Keyboard name:");
        ui.add_enabled(idle, egui::TextEdit::singleline(&mut app.device.name).desired_width(f32::INFINITY))
            .on_hover_text("Case-insensitive part of the keyboard name, used if it isn't found by its config service");
    });
}
