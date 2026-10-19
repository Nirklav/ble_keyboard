//! Combos: the list of special key combinations and the editor of one, keys picked on the grid
//! or matched by action.

use eframe::egui;
use protocol::config::{
    Combo, ComboAction, ComboKeyIdent, Config, KeyAction, MAX_COMBO_KEYS, MAX_COMBOS, recovery_combo,
};

use crate::app::{App, EditTarget, KeyEditor, Screen};
use crate::editing;
use crate::labels;
use crate::views::{key_grid, layout_select, nav_button};

/// List of the combos: the built-in recovery one, then the configured ones.
pub fn list(app: &mut App, ui: &mut egui::Ui) {
    let config = &app.doc.config;
    let layouts = editing::layout_names(config);
    let mut open = None;

    ui.strong("Built-in");
    ui.weak("Part of the firmware, always active, can't be changed or removed.");
    ui.add_space(4.0);
    let recovery = recovery_combo();
    let title = labels::combo_action_label(&recovery.action, &layouts);
    if nav_button(ui, LIST_BUTTON, &title, &summary(config, &recovery)).clicked() {
        open = Some(None);
    }
    ui.add_space(12.0);

    ui.strong(format!("Configured ({} of {MAX_COMBOS})", config.combos.len()));
    ui.add_space(4.0);
    for (index, combo) in config.combos.iter().enumerate() {
        let title = format!("{}. {}", index + 1, labels::combo_action_label(&combo.action, &layouts));
        if nav_button(ui, LIST_BUTTON, &title, &summary(config, combo)).clicked() {
            open = Some(Some(index));
        }
    }
    if let Some(index) = open {
        app.go(Screen::Combo(index));
    }

    ui.add_space(4.0);
    let add = ui
        .add_enabled(app.doc.config.combos.len() < MAX_COMBOS, egui::Button::new("+ Add combo"))
        .on_disabled_hover_text(format!("At most {MAX_COMBOS} combos"));
    if add.clicked() {
        match editing::add_combo(&mut app.doc.config) {
            Ok(index) => {
                app.go(Screen::Combo(Some(index)));
                app.picking = Some((index, 0));
            }
            Err(error) => app.message("Can't add a combo", error),
        }
    }
}

/// Size of a combo button in the list.
const LIST_BUTTON: egui::Vec2 = egui::vec2(420.0, 48.0);

/// Combo `index`, `None` for the built-in recovery one (read only).
pub fn show(app: &mut App, ui: &mut egui::Ui, index: Option<usize>) {
    let recovery = recovery_combo();
    grid(app, ui, &recovery);
    ui.add_space(8.0);
    ui.separator();

    let Some(index) = index else {
        ui.weak("Built into the firmware: always active, can't be changed or removed.");
        ui.add_space(4.0);
        egui::Grid::new("recovery").num_columns(2).spacing([12.0, 4.0]).show(ui, |ui| {
            ui.label("Action:");
            ui.label(labels::combo_action_label(&recovery.action, &[]));
            ui.end_row();
            for (number, key) in recovery.keys.iter().enumerate() {
                ui.label(format!("Key {}:", number + 1));
                ui.monospace(labels::combo_key_label(key, app.doc.config.layouts.get(app.grid_layout)));
                ui.end_row();
            }
        });
        return;
    };
    if editor(app, ui, index) {
        app.doc.config.combos.remove(index);
        app.go(Screen::Combos);
    }
}

/// Key grid of the selected layout: shows the keys of the combo, sets the picked key.
fn grid(app: &mut App, ui: &mut egui::Ui, recovery: &Combo) {
    let layouts = editing::layout_names(&app.doc.config);
    ui.horizontal(|ui| {
        ui.label("Layout shown:");
        let mut layout = app.grid_layout as u8;
        layout_select(ui, "combo_grid_layout", &mut layout, &layouts);
        app.grid_layout = layout as usize;

        match app.picking {
            Some((_, key)) => {
                ui.colored_label(ui.visuals().warn_fg_color, format!("Click the key for key {}", key + 1));
                if ui.button("Cancel").clicked() {
                    app.picking = None;
                }
            }
            None => {
                ui.weak("Combo keys are highlighted.");
            }
        }
    });

    let config = &app.doc.config;
    let combo = match app.screen {
        Screen::Combo(Some(index)) => &config.combos[index],
        _ => recovery,
    };
    let clicked = key_grid(ui, &config.layouts[app.grid_layout], |x, y| editing::combo_uses_coord(combo, x, y));

    if let (Some((x, y)), Some((combo, key))) = (clicked, app.picking) {
        app.doc.config.combos[combo].keys[key].ident = ComboKeyIdent::Coord { x: x as u8, y: y as u8 };
        app.picking = None;
    }
}

/// Editor of combo `index`; returns `true` if it should be removed.
fn editor(app: &mut App, ui: &mut egui::Ui, index: usize) -> bool {
    let layouts = editing::layout_names(&app.doc.config);
    let config = &app.doc.config;
    let texts: Vec<String> = config.combos[index]
        .keys
        .iter()
        .map(|k| labels::combo_ident_label(k, config.layouts.get(app.grid_layout)))
        .collect();
    let mut remove = false;
    let combo = &mut app.doc.config.combos[index];
    ui.horizontal(|ui| {
        ui.label("Action:");
        action_select(ui, index, &mut combo.action, &layouts);
    });

    ui.add_space(4.0);
    let mut remove_key = None;
    let keys = combo.keys.len();
    egui::Grid::new("combo_keys").num_columns(6).spacing([12.0, 4.0]).show(ui, |ui| {
        for key in 0..keys {
            ui.label(format!("Key {}:", key + 1));
            let combo_key = &mut combo.keys[key];
            let by_coord = matches!(combo_key.ident, ComboKeyIdent::Coord { .. });
            ui.horizontal(|ui| {
                if ui.selectable_label(by_coord, "Position").clicked() && !by_coord {
                    combo_key.ident = ComboKeyIdent::Coord { x: 0, y: 0 };
                }
                if ui.selectable_label(!by_coord, "Action").clicked() && by_coord {
                    combo_key.ident = ComboKeyIdent::Action(KeyAction::None);
                }
            });

            ui.monospace(texts.get(key).map_or("", String::as_str));
            match combo_key.ident {
                ComboKeyIdent::Coord { .. } => {
                    let picking = app.picking == Some((index, key));
                    if ui.selectable_label(picking, "Pick on grid").clicked() {
                        app.picking = if picking { None } else { Some((index, key)) };
                    }
                }
                ComboKeyIdent::Action(_) => {
                    if ui.button("Edit...").clicked() {
                        let search = app.key_editor.take().map(|e| e.search).unwrap_or_default();
                        app.key_editor = Some(KeyEditor { target: EditTarget::ComboKey { combo: index, key }, search });
                    }
                }
            }

            let count = egui::DragValue::new(&mut combo_key.count)
                .range(0..=u8::MAX)
                .custom_formatter(|n, _| if n == 0.0 { "hold".to_string() } else { format!("x{n}") })
                .custom_parser(|text| match text.trim() {
                    "hold" => Some(0.0),
                    text => text.trim_start_matches('x').parse().ok(),
                });
            ui.add(count).on_hover_text("Number of presses (press + release); 0 means the key must be held");

            if ui.add_enabled(keys > 1, egui::Button::new("x")).on_hover_text("Remove the key").clicked() {
                remove_key = Some(key);
            }
            ui.end_row();
        }
    });
    if let Some(key) = remove_key {
        combo.keys.remove(key);
        app.picking = None;
        if matches!(app.key_editor.as_ref().map(|e| e.target), Some(EditTarget::ComboKey { combo, .. }) if combo == index)
        {
            app.key_editor = None;
        }
    }

    let add = ui
        .add_enabled(combo.keys.len() < MAX_COMBO_KEYS, egui::Button::new("+ Add key"))
        .on_disabled_hover_text(format!("At most {MAX_COMBO_KEYS} keys per combo"));
    if add.clicked()
        && let Ok(key) = editing::add_combo_key(combo)
    {
        app.picking = Some((index, key));
    }
    ui.add_space(8.0);
    if ui.button("Remove the combo").clicked() {
        remove = true;
    }
    remove
}

/// Combo box of the combo action, with the layout for `SetLayout`.
fn action_select(ui: &mut egui::Ui, index: usize, action: &mut ComboAction, layouts: &[String]) {
    let kinds = [
        (ComboAction::SwitchLayout, "Switch layout"),
        (ComboAction::SetLayout(0), "Set layout"),
        (ComboAction::Unpair, "Unpair"),
        (ComboAction::ResetConfig, "Reset config"),
    ];
    let same_kind = |a: &ComboAction, b: &ComboAction| std::mem::discriminant(a) == std::mem::discriminant(b);
    let current = kinds.iter().find(|(kind, _)| same_kind(kind, action)).map_or("?", |(_, name)| *name);

    egui::ComboBox::from_id_salt(("combo_action", index)).selected_text(current).show_ui(ui, |ui| {
        for (kind, name) in kinds {
            if ui.selectable_label(same_kind(&kind, action), name).clicked() && !same_kind(&kind, action) {
                *action = kind;
            }
        }
    });
    if let ComboAction::SetLayout(layout) = action {
        layout_select(ui, ("combo_layout", index), layout, layouts);
    }
    ui.weak(labels::combo_action_label(action, layouts));
}

/// Keys of `combo`, e.g. `Fn 1/2 (2, 4) hold + Esc (0, 0) x5`, keys named after the first layout.
fn summary(config: &Config, combo: &Combo) -> String {
    let keys: Vec<String> = combo.keys.iter().map(|key| labels::combo_key_label(key, config.layouts.first())).collect();
    keys.join(" + ")
}
