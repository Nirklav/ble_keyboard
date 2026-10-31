//! Overview: keyboard info, layouts, the way to the combos and the settings.

use std::ops::RangeInclusive;

use eframe::egui;
use protocol::hid;
use protocol::config::{CONFIG_VERSION, MAX_LAYOUTS, Settings};

use desktop::compat;
use desktop::firmware::{self, Firmware};

use crate::app::{App, Dialog, Keyboard, Screen};
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
    if nav_button(ui, NAV_SIZE, "Combos and chords", &subtitle).clicked() {
        app.go(Screen::Combos);
    }
    ui.add_space(theme::GAP_XL);

    theme::section_heading(ui, "Settings");
    ui.add_space(theme::GAP_XS);
    settings(app, ui);
}

/// Timing of the keys, sleep, the display and the joystick cursor.
fn settings(app: &mut App, ui: &mut egui::Ui) {
    let settings = &mut app.doc.config.settings;
    egui::Grid::new("settings").num_columns(3).spacing([theme::GAP_M, theme::GAP_S]).show(ui, |ui| {
        group(ui, "Keys");
        let hint = "A mod-tap or layer-tap key held longer is a hold.";
        number(ui, "Tapping term", &mut settings.tapping_term_ms, Settings::TAPPING_TERM_MS, " ms", hint);
        let hint = "Pressed this soon after a typed key, it's a tap at once; 0 turns this off.";
        number(ui, "Prior idle", &mut settings.require_prior_idle_ms, Settings::REQUIRE_PRIOR_IDLE_MS, " ms", hint);
        let hint = "The keys of a chord must all be pressed within this time.";
        number(ui, "Combo term", &mut settings.combo_term_ms, Settings::COMBO_TERM_MS, " ms", hint);
        let hint = "A key change is ignored this long after the previous one: contact bounce.";
        number(ui, "Debounce", &mut settings.debounce_ms, Settings::DEBOUNCE_MS, " ms", hint);

        group(ui, "Sleep");
        let hint = "Deep sleep after this long without activity while a host is connected.";
        number(ui, "Connected", &mut settings.sleep_connected_s, Settings::SLEEP_S, " s", hint);
        let hint = "The same while no host is connected.";
        number(ui, "Disconnected", &mut settings.sleep_disconnected_s, Settings::SLEEP_S, " s", hint);

        group(ui, "Display");
        let hint = "The status goes off after this long without key presses: an OLED burns in.";
        number(ui, "Off after", &mut settings.display_off_s, Settings::DISPLAY_OFF_S, " s", hint);

        group(ui, "Joystick");
        let hint = "Cursor speed at full deflection.";
        number(ui, "Speed", &mut settings.joystick_speed, Settings::JOYSTICK_SPEED, " px/s", hint);
        let hint = "Time to reach the speed of the deflection, 0 is at once.";
        number(ui, "Ramp", &mut settings.joystick_ramp_ms, Settings::JOYSTICK_RAMP_MS, " ms", hint);
        ui.label("Invert");
        ui.horizontal(|ui| {
            ui.checkbox(&mut settings.joystick_invert_x, "X");
            ui.checkbox(&mut settings.joystick_invert_y, "Y");
        });
        ui.weak("Moves the cursor the other way along the axis.");
        ui.end_row();
    });
    if *settings != Settings::default() {
        ui.add_space(theme::GAP_XS);
        if ui.button("Reset to defaults").on_hover_text("Every setting to its default value").clicked() {
            *settings = Settings::default();
        }
    }
}

/// Group heading row of the settings grid.
fn group(ui: &mut egui::Ui, name: &str) {
    ui.strong(name);
    ui.end_row();
}

/// Settings grid row of a number in `range`.
fn number(ui: &mut egui::Ui, name: &str, value: &mut u16, range: RangeInclusive<u16>, suffix: &str, hint: &str) {
    ui.label(name);
    ui.add(egui::DragValue::new(value).range(range).suffix(suffix));
    ui.weak(hint);
    ui.end_row();
}

/// Info of the connected keyboard in one line, nothing offline.
fn keyboard(app: &mut App, ui: &mut egui::Ui) {
    let Keyboard::Connected { versions, info } = &app.device.keyboard else {
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
    // The bundled firmware, whether the keyboard runs that very build, whether it can be installed.
    let bundled = firmware::bundled_firmware()
        .map(|bundled| (bundled, bundled.runs_on(versions), compat::check_update(versions, bundled)));
    let firmware = format!("Firmware {}", versions.firmware_version);
    let firmware = match bundled {
        Some((bundled, false, _)) if bundled.version == versions.firmware_version => {
            egui::RichText::new(format!("{firmware} (app: another build)")).color(warn)
        }
        Some((bundled, false, _)) => egui::RichText::new(format!("{firmware} (app: {})", bundled.version)).color(warn),
        _ => egui::RichText::new(firmware).color(ui.visuals().weak_text_color()),
    };
    let parts = [
        firmware,
        version("Protocol", info.protocol_version, hid::PROTOCOL_VERSION),
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
        if let Some((bundled, runs, compatible)) = bundled {
            ui.add_space(theme::GAP_XS);
            update_button(app, ui, bundled, runs, compatible);
        }
    });
    ui.add_space(theme::GAP_L);
}

/// Installs the firmware bundled with the app, asks first; `compatible` is [`compat::check_update`].
fn update_button(app: &mut App, ui: &mut egui::Ui, bundled: &Firmware, runs: bool, compatible: anyhow::Result<()>) {
    let version = &bundled.version;
    let (label, hint) = if runs {
        ("Reinstall", format!("Install firmware {version} bundled with the app again, the keyboard runs it already"))
    } else {
        ("Update", format!("Install firmware {version} bundled with the app"))
    };
    let disabled = match compatible {
        Err(error) => Some(format!("{error:#}")),
        Ok(()) if app.device.is_busy() => Some("Wait for the keyboard command to finish".to_string()),
        Ok(()) => None,
    };
    let button = ui
        .add_enabled(disabled.is_none(), egui::Button::new(label).small())
        .on_hover_text(hint)
        .on_disabled_hover_text(disabled.unwrap_or_default());
    if button.clicked() {
        app.dialog = Some(Dialog::ConfirmUpdate(version.clone()));
    }
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
