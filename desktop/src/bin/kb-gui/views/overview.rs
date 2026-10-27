//! Overview: keyboard info, layouts, the way to the combos and the settings.

use eframe::egui;
use protocol::ble;
use protocol::config::{CONFIG_VERSION, MAX_LAYOUTS, Settings};

use crate::app::{App, Screen};
use crate::editing;
use crate::theme;
use crate::views::{NAV_SIZE, nav_button};

pub fn show(app: &mut App, ui: &mut egui::Ui) {
    keyboard(app, ui);

    theme::section_heading(ui, "Layouts");
    ui.add_space(theme::GAP_XS);
    layouts(app, ui);
    ui.add_space(theme::GAP_XL);

    theme::section_heading(ui, "Combos");
    ui.add_space(theme::GAP_XS);
    let count = app.doc.config.combos.len();
    let subtitle = format!("{count} configured + factory reset");
    if nav_button(ui, NAV_SIZE, "Special combos", &subtitle).clicked() {
        app.go(Screen::Combos);
    }
    ui.add_space(theme::GAP_XL);

    theme::section_heading(ui, "Settings");
    ui.add_space(theme::GAP_XS);
    settings(app, ui);
}

/// Timing of the mod-tap and layer-tap keys.
fn settings(app: &mut App, ui: &mut egui::Ui) {
    let settings = &mut app.doc.config.settings;
    egui::Grid::new("settings").num_columns(3).spacing([theme::GAP_M, theme::GAP_S]).show(ui, |ui| {
        ui.label("Tapping term");
        let term = egui::DragValue::new(&mut settings.tapping_term_ms).range(Settings::TAPPING_TERM_MS).suffix(" ms");
        ui.add(term);
        ui.weak("A mod-tap or layer-tap key held longer is a hold.");
        ui.end_row();

        ui.label("Prior idle");
        let idle = egui::DragValue::new(&mut settings.require_prior_idle_ms)
            .range(Settings::REQUIRE_PRIOR_IDLE_MS)
            .suffix(" ms");
        ui.add(idle);
        ui.weak("Pressed this soon after a typed key, it's a tap at once; 0 turns this off.");
        ui.end_row();
    });
    if *settings != Settings::default() {
        ui.add_space(theme::GAP_XS);
        let defaults = Settings::default();
        let hover = format!("{} ms and {} ms", defaults.tapping_term_ms, defaults.require_prior_idle_ms);
        if ui.button("Reset to defaults").on_hover_text(hover).clicked() {
            *settings = defaults;
        }
    }
}

/// Info of the connected keyboard in one line, nothing offline.
fn keyboard(app: &App, ui: &mut egui::Ui) {
    let device = &app.device;
    let (Some(info), true) = (&device.info, device.connected) else {
        return;
    };
    // One weak line; a version other than the app one is highlighted, it may break reading or writing.
    let warn = ui.visuals().warn_fg_color;
    let version = |name: &str, keyboard: u16, app: u16| {
        let text = format!("{name} {keyboard}");
        if keyboard == app {
            egui::RichText::new(text).color(ui.visuals().weak_text_color())
        } else {
            egui::RichText::new(format!("{text} (app: {app})")).color(warn)
        }
    };
    let parts = [
        egui::RichText::new(format!("Firmware {}", info.firmware_version)).color(ui.visuals().weak_text_color()),
        version("Protocol", info.protocol_version, ble::PROTOCOL_VERSION),
        version("Config format", info.config_version, CONFIG_VERSION),
        egui::RichText::new(format!(
            "{}×{} keys, up to {} layouts and {} combos",
            info.width, info.height, info.max_layouts, info.max_combos
        ))
        .color(ui.visuals().weak_text_color()),
    ];
    ui.horizontal_wrapped(|ui| {
        for (index, part) in parts.into_iter().enumerate() {
            if index > 0 {
                ui.weak("·");
            }
            ui.label(part);
        }
    });
    ui.add_space(theme::GAP_L);
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
