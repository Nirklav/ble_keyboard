//! Key action editor: a side panel editing one layout key, joystick button or combo key matched by action.

use eframe::egui;
use protocol::config::{ComboKeyIdent, KeyAction};
use protocol::keys::{Key, KeyMod, MediaKey};

use crate::app::{App, EditTarget};
use crate::editing::{self, ActionKind};
use crate::labels;
use crate::theme;
use crate::views::layout_select;

/// Width of the editor panel.
pub const WIDTH: f32 = 300.0;

/// Draws the editor of [`App::key_editor`], which must be set.
pub fn panel(app: &mut App, ui: &mut egui::Ui) {
    let Some(editor) = &mut app.key_editor else {
        return;
    };
    let layouts = editing::layout_names(&app.doc.config);
    let config = &mut app.doc.config;

    // `fn_error`: why Fn can't be here.
    let (place, action, fn_error) = match editor.target {
        EditTarget::LayoutKey { layout, x, y } => {
            let place = format!("Layout {}, key ({x}, {y})", config.layouts[layout].name);
            let fn_error = editing::other_fn_key(&config.layouts[layout], x, y)
                .map(|(fx, fy)| format!("This layout already has an Fn key at ({fx}, {fy}), only one is allowed."));
            (place, &mut config.layouts[layout].rows[y][x], fn_error)
        }
        EditTarget::JoystickButton { layout, button } => {
            let place =
                format!("Layout {}, joystick {} button", config.layouts[layout].name, button.name().to_lowercase());
            let fn_error = Some("Fn can't be on a joystick button.".to_string());
            (place, config.layouts[layout].joystick.get_mut(button), fn_error)
        }
        EditTarget::ComboKey { combo, key } => {
            let place = format!("Combo {}, key {}", combo + 1, key + 1);
            let ident = &mut config.combos[combo].keys[key].ident;
            if !matches!(ident, ComboKeyIdent::Action(_)) {
                *ident = ComboKeyIdent::Action(KeyAction::None);
            }
            let ComboKeyIdent::Action(action) = ident else { unreachable!() };
            (place, action, None)
        }
    };

    let mut close = ui.input(|i| i.key_pressed(egui::Key::Escape));
    ui.horizontal(|ui| {
        let title = match labels::action_label(action) {
            label if label.is_empty() => match editor.target {
                EditTarget::ComboKey { .. } => "No action".to_string(),
                _ => "Empty key".to_string(),
            },
            label => label,
        };
        theme::section_heading(ui, title);
        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
            if ui.add(egui::Button::new("×").frame_when_inactive(false)).on_hover_text("Close (Esc)").clicked() {
                close = true;
            }
        });
    });
    ui.weak(place);
    ui.add_space(theme::GAP_XS);
    ui.separator();
    ui.add_space(theme::GAP_XS);

    if matches!(editor.target, EditTarget::ComboKey { .. }) {
        ui.weak("The combo key matches any key or joystick button registered as this action in the current layout.");
        ui.add_space(theme::GAP_S);
    }
    if let (Some(error), KeyAction::Fn { .. }) = (&fn_error, &action) {
        ui.colored_label(ui.visuals().warn_fg_color, error);
        ui.add_space(theme::GAP_S);
    }
    let allow_fn = !matches!(editor.target, EditTarget::JoystickButton { .. });
    action_editor(ui, action, &layouts, &mut editor.search, &mut editor.scroll_to_current, allow_fn);
    if close {
        app.key_editor = None;
    }
}

/// Edits `action`: kind selector and the fields of that kind, `allow_fn` offers [`KeyAction::Fn`].
/// `scroll_to_current` scrolls the key list to the current key and is reset.
fn action_editor(
    ui: &mut egui::Ui,
    action: &mut KeyAction,
    layouts: &[String],
    search: &mut String,
    scroll_to_current: &mut bool,
    allow_fn: bool,
) {
    let kind = ActionKind::of(action);
    ui.horizontal(|ui| {
        ui.label("Action:");
        // Tall enough for all the kinds, no scrolling.
        let combo = egui::ComboBox::from_id_salt("action_kind").selected_text(kind.name()).height(f32::INFINITY);
        combo.show_ui(ui, |ui| {
            for other in ActionKind::ALL.into_iter().filter(|&kind| allow_fn || kind != ActionKind::Fn) {
                if ui.selectable_label(other == kind, other.name()).clicked() && other != kind {
                    *action = other.convert(action);
                }
            }
        });
    });
    ui.add_space(theme::GAP_S);

    match action {
        KeyAction::Key(key) => key_picker(ui, key, search, scroll_to_current),
        KeyAction::KeyWithMod(key, mods) => {
            mods_editor(ui, mods);
            ui.add_space(theme::GAP_S);
            key_picker(ui, key, search, scroll_to_current);
        }
        KeyAction::Mod(mods) => mods_editor(ui, mods),
        KeyAction::Media(media) => {
            for key in MediaKey::ALL {
                ui.radio_value(media, key, key.name());
            }
        }
        KeyAction::MouseKey(buttons) => {
            ui.horizontal_wrapped(|ui| {
                for (button, name, _) in labels::MOUSE_BUTTONS {
                    let mut on = buttons.contains(button);
                    if ui.checkbox(&mut on, name).changed() {
                        buttons.set(button, on);
                    }
                }
            });
        }
        KeyAction::MouseTravel { dx, dy } => {
            egui::Grid::new("mouse_travel").num_columns(2).show(ui, |ui| {
                ui.label("Right (dx):");
                ui.add(egui::DragValue::new(dx).range(i8::MIN..=i8::MAX));
                ui.end_row();
                ui.label("Down (dy):");
                ui.add(egui::DragValue::new(dy).range(i8::MIN..=i8::MAX));
                ui.end_row();
            });
            ui.weak("Cursor move per report while held; negative dy is up.");
        }
        KeyAction::MouseScroll { dy } => {
            ui.horizontal(|ui| {
                ui.label("Up (dy):");
                ui.add(egui::DragValue::new(dy).range(i8::MIN..=i8::MAX));
            });
            ui.weak("Wheel scroll per report while held; positive is up.");
        }
        KeyAction::Fn { temporary, permanent } => {
            egui::Grid::new("fn_layouts").num_columns(2).show(ui, |ui| {
                ui.label("While held:");
                layout_select(ui, "fn_temporary", temporary, layouts);
                ui.end_row();
                ui.label("After a double tap:");
                layout_select(ui, "fn_permanent", permanent, layouts);
                ui.end_row();
            });
        }
        KeyAction::None => {
            ui.weak("The key does nothing.");
        }
    }
}

/// Searchable list of all keys in groups, takes the rest of the panel height.
fn key_picker(ui: &mut egui::Ui, key: &mut Key, search: &mut String, scroll_to_current: &mut bool) {
    ui.add(egui::TextEdit::singleline(search).hint_text("Search keys").desired_width(f32::INFINITY));
    ui.add_space(theme::GAP_XS);
    let filter = search.trim().to_lowercase();
    egui::ScrollArea::vertical().auto_shrink([false, false]).show(ui, |ui| {
        ui.spacing_mut().item_spacing.y = 2.0;
        ui.spacing_mut().interact_size.y = 22.0;
        for (group, ranges) in labels::KEY_GROUPS {
            let matching: Vec<(Key, String)> = ranges
                .iter()
                .flat_map(|range| &labels::KEYS[range.clone()])
                .filter_map(|&candidate| {
                    let name = labels::key_name(candidate);
                    let label = labels::key_label(candidate);
                    let matches = filter.is_empty()
                        || name.to_lowercase().contains(&filter)
                        || label.to_lowercase().contains(&filter);
                    let text = if label == name { name } else { format!("{label}  ({name})") };
                    matches.then_some((candidate, text))
                })
                .collect();
            if matching.is_empty() {
                continue;
            }
            if !group.is_empty() {
                ui.add_space(theme::GAP_S);
                ui.weak(group);
            }
            for (candidate, text) in matching {
                let response = ui.selectable_value(key, candidate, text);
                if response.clicked() {
                    response.scroll_to_me(None);
                } else if *scroll_to_current && *key == candidate {
                    response.scroll_to_me(Some(egui::Align::Center));
                }
            }
        }
    });
    *scroll_to_current = false;
}

fn mods_editor(ui: &mut egui::Ui, mods: &mut KeyMod) {
    ui.label("Modifiers:");
    egui::Grid::new("mods").show(ui, |ui| {
        for (i, (modifier, name)) in labels::MODIFIERS.iter().enumerate() {
            let mut on = mods.contains(*modifier);
            if ui.checkbox(&mut on, *name).changed() {
                mods.set(*modifier, on);
            }
            if i % 4 == 3 {
                ui.end_row();
            }
        }
    });
}
