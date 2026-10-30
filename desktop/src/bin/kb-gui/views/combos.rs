//! Combos: the list of key combinations (special ones and chords) and the editor of one, keys picked
//! on the grid or matched by action.

use eframe::egui;
use protocol::board::recovery_combo;
use protocol::config::{
    Combo, ComboAction, ComboKeyIdent, Config, KeyAction, MAX_COMBO_KEYS, MAX_COMBOS,
};

use crate::app::{App, EditTarget, KeyEditor, Screen};
use crate::editing;
use crate::labels;
use crate::theme;
use crate::views::{KeyMark, bt_profile_select, key_grid, layout_select};

/// List of the combos: the built-in recovery one, then the configured ones.
pub fn list(app: &mut App, ui: &mut egui::Ui) {
    let config = &app.doc.config;
    let layouts = editing::layout_names(config);
    let mut open = None;

    theme::section_heading(ui, "Built-in");
    ui.weak("Part of the firmware, always active, can't be changed or removed.");
    ui.add_space(theme::GAP_XS);
    let recovery = recovery_combo();
    let title = labels::combo_action_label(&recovery.action, &layouts);
    if combo_button(ui, &title, &chips(config, &recovery)).clicked() {
        open = Some(None);
    }
    ui.add_space(theme::GAP_L);

    theme::section_heading(ui, format!("Configured ({} of {MAX_COMBOS})", config.combos.len()));
    ui.add_space(theme::GAP_XS);
    for (index, combo) in config.combos.iter().enumerate() {
        let title = format!("{}. {}", index + 1, labels::combo_action_label(&combo.action, &layouts));
        if combo_button(ui, &title, &chips(config, combo)).clicked() {
            open = Some(Some(index));
        }
    }
    if let Some(index) = open {
        app.go(Screen::Combo(index));
    }

    ui.add_space(theme::GAP_XS);
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
const LIST_BUTTON: egui::Vec2 = egui::vec2(420.0, 60.0);
/// Height of a key chip on a combo button.
const CHIP_HEIGHT: f32 = 20.0;

/// Combo button in the list: the action and the keys as chips.
fn combo_button(ui: &mut egui::Ui, title: &str, keys: &[String]) -> egui::Response {
    let (rect, response) = ui.allocate_exact_size(LIST_BUTTON, egui::Sense::click());
    if !ui.is_rect_visible(rect) {
        return response;
    }
    let visuals = ui.style().interact(&response);
    let painter = ui.painter().with_clip_rect(rect);
    painter.rect(rect, visuals.corner_radius, visuals.weak_bg_fill, visuals.bg_stroke, egui::StrokeKind::Inside);
    let left = rect.left() + theme::GAP_M;
    let title_pos = egui::pos2(left, rect.top() + theme::GAP_S);
    painter.text(title_pos, egui::Align2::LEFT_TOP, title, egui::FontId::proportional(16.0), visuals.text_color());

    let font = egui::FontId::proportional(12.0);
    let weak = ui.visuals().weak_text_color();
    let center_y = rect.bottom() - theme::GAP_S - CHIP_HEIGHT / 2.0;
    let mut x = left;
    for (index, key) in keys.iter().enumerate() {
        if index > 0 {
            let pos = egui::pos2(x + theme::GAP_XS, center_y);
            let plus = painter.text(pos, egui::Align2::LEFT_CENTER, "+", font.clone(), weak);
            x = plus.right() + theme::GAP_XS;
        }
        let galley = painter.layout_no_wrap(key.clone(), font.clone(), visuals.text_color());
        let chip = egui::Rect::from_min_size(
            egui::pos2(x, center_y - CHIP_HEIGHT / 2.0),
            egui::vec2(galley.size().x + 2.0 * theme::GAP_S, CHIP_HEIGHT),
        );
        painter.rect_filled(chip, egui::CornerRadius::same(4), ui.visuals().panel_fill);
        painter.galley(egui::pos2(chip.left() + theme::GAP_S, center_y - galley.size().y / 2.0), galley, weak);
        x = chip.right();
    }
    response
}

/// Combo `index`, `None` for the built-in recovery one (read only).
pub fn show(app: &mut App, ui: &mut egui::Ui, index: Option<usize>) {
    let recovery = recovery_combo();
    grid(app, ui, &recovery);
    ui.add_space(theme::GAP_S);
    ui.separator();

    let Some(index) = index else {
        ui.weak("Built into the firmware: always active, can't be changed or removed.");
        ui.add_space(theme::GAP_XS);
        egui::Grid::new("recovery").num_columns(2).spacing([theme::GAP_M, theme::GAP_XS]).show(ui, |ui| {
            ui.label("Action:");
            ui.label(labels::combo_action_label(&recovery.action, &[]));
            ui.end_row();
            for (number, key) in recovery.keys.iter().enumerate() {
                ui.label(format!("Key {}:", number + 1));
                ui.label(labels::combo_key_label(key, app.doc.config.layouts.get(app.grid_layout)));
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
    let mark = |x, y| if editing::combo_uses_coord(combo, x, y) { KeyMark::Marked } else { KeyMark::None };
    let clicked = key_grid(ui, &config.layouts[app.grid_layout], mark);

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
    let was_chord = combo.is_chord();
    let mut edit_action = false;
    ui.horizontal(|ui| {
        ui.label("Action:");
        action_select(ui, index, &mut combo.action, &layouts);
        if combo.is_chord() && ui.button("Edit...").clicked() {
            edit_action = true;
        }
    });
    // A chord has 2+ held keys; a special combination needs presses.
    if combo.is_chord() && !was_chord {
        combo.keys.iter_mut().for_each(|key| key.count = 0);
        if combo.keys.len() < 2
            && let Ok(key) = editing::add_combo_key(combo)
        {
            app.picking = Some((index, key));
        }
        edit_action = true;
    } else if was_chord && !combo.is_chord() && let Some(last) = combo.keys.last_mut() {
        last.count = 1;
    }
    if edit_action {
        app.key_editor = Some(KeyEditor::open(EditTarget::ComboAction { combo: index }, app.key_editor.take()));
    }
    let chord = combo.is_chord();
    if chord {
        ui.weak("A chord: the keys pressed together, within the combo term of the settings, do the action instead.");
    }

    ui.add_space(theme::GAP_XS);
    let mut remove_key = None;
    let keys = combo.keys.len();
    egui::Grid::new("combo_keys").num_columns(6).spacing([theme::GAP_M, theme::GAP_XS]).show(ui, |ui| {
        for key in 0..keys {
            ui.label(format!("Key {}:", key + 1));
            let combo_key = &mut combo.keys[key];
            let by_coord = matches!(combo_key.ident, ComboKeyIdent::Coord { .. });
            // A position is picked on the grid right away; an action is edited in the editor panel,
            // which would turn the key back into an action if left open on a position.
            match theme::segmented(ui, if by_coord { 0 } else { 1 }, &["Position", "Action"]) {
                Some(0) => {
                    combo_key.ident = ComboKeyIdent::Coord { x: 0, y: 0 };
                    app.picking = Some((index, key));
                    let target = EditTarget::ComboKey { combo: index, key };
                    if app.key_editor.as_ref().is_some_and(|e| e.target == target) {
                        app.key_editor = None;
                    }
                }
                Some(_) => {
                    combo_key.ident = ComboKeyIdent::Action(KeyAction::None);
                    if app.picking == Some((index, key)) {
                        app.picking = None;
                    }
                }
                None => {}
            }

            ui.label(texts.get(key).map_or("", String::as_str));
            match combo_key.ident {
                ComboKeyIdent::Coord { .. } => {
                    let picking = app.picking == Some((index, key));
                    if ui.add(egui::Button::new("Pick on grid").selected(picking)).clicked() {
                        app.picking = if picking { None } else { Some((index, key)) };
                    }
                }
                ComboKeyIdent::Action(_) => {
                    if ui.button("Edit...").clicked() {
                        let target = EditTarget::ComboKey { combo: index, key };
                        app.key_editor = Some(KeyEditor::open(target, app.key_editor.take()));
                    }
                }
            }

            if chord {
                ui.weak("press");
            } else {
                let count = egui::DragValue::new(&mut combo_key.count)
                    .range(0..=u8::MAX)
                    .custom_formatter(|n, _| if n == 0.0 { "hold".to_string() } else { format!("×{n}") })
                    .custom_parser(|text| match text.trim() {
                        "hold" => Some(0.0),
                        text => text.trim_start_matches(['x', '×']).parse().ok(),
                    });
                ui.add(count).on_hover_text("Number of presses (press + release); 0 means the key must be held");
            }

            let min_keys = if chord { 2 } else { 1 };
            if ui.add_enabled(keys > min_keys, egui::Button::new("×")).on_hover_text("Remove the key").clicked() {
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
    ui.add_space(theme::GAP_L);
    if ui.add(theme::danger_button("Remove the combo")).clicked() {
        remove = true;
    }
    remove
}

/// Combo box of the combo action, with the layout for `SetLayout`; the action of a chord is edited
/// in the editor panel.
fn action_select(ui: &mut egui::Ui, index: usize, action: &mut ComboAction, layouts: &[String]) {
    let kinds = [
        (ComboAction::Action(KeyAction::None), "Chord: key action"),
        (ComboAction::SwitchLayout, "Switch layout"),
        (ComboAction::SetLayout(0), "Set layout"),
        (ComboAction::ResetConfig, "Reset config"),
        (ComboAction::BtSelect(0), "BLE profile: select"),
        (ComboAction::BtNext, "BLE profile: next"),
        (ComboAction::BtPrev, "BLE profile: previous"),
        (ComboAction::BtClear, "BLE profile: forget host"),
        (ComboAction::BtClearAll, "BLE profile: forget all hosts"),
        (ComboAction::Unpair, "Unpair (= forget host)"),
    ];
    let same_kind = |a: &ComboAction, b: &ComboAction| std::mem::discriminant(a) == std::mem::discriminant(b);
    let current = kinds.iter().find(|(kind, _)| same_kind(kind, action)).map_or("?", |(_, name)| *name);

    // All the kinds without scrolling if the window is tall enough, like the kind selector of the action editor.
    let below = ui.ctx().content_rect().bottom() - ui.cursor().top() - 3.0 * ui.spacing().interact_size.y;
    let combo = egui::ComboBox::from_id_salt(("combo_action", index)).selected_text(current).height(below.max(120.0));
    combo.show_ui(ui, |ui| {
        for (kind, name) in kinds {
            if ui.selectable_label(same_kind(&kind, action), name).clicked() && !same_kind(&kind, action) {
                *action = kind;
            }
        }
    });
    match action {
        ComboAction::SetLayout(layout) => layout_select(ui, ("combo_layout", index), layout, layouts),
        ComboAction::BtSelect(profile) => bt_profile_select(ui, ("combo_bt_profile", index), profile),
        _ => {}
    }
    ui.weak(labels::combo_action_label(action, layouts));
}

/// Chips of the keys of `combo`, keys named after the first layout; chord keys have no count.
fn chips(config: &Config, combo: &Combo) -> Vec<String> {
    let chip = if combo.is_chord() { labels::combo_chord_key_chip } else { labels::combo_key_chip };
    combo.keys.iter().map(|key| chip(key, config.layouts.first())).collect()
}
