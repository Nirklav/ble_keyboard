//! Key action editor: a side panel editing one layout key, joystick button, combo key matched by action
//! or chord action.

use eframe::egui;
use protocol::config::{ComboAction, ComboKeyIdent, KeyAction};
use protocol::keys::{Key, KeyMod, MediaKey};
use protocol::motion::{Velocity, TRAVEL_UNIT_MS};

use crate::app::{App, EditTarget};
use crate::editing::{self, ActionKind};
use crate::labels;
use crate::theme;
use crate::views::{bt_profile_select, layout_select};

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
        EditTarget::ComboAction { combo } => {
            let place = format!("Combo {}, chord action", combo + 1);
            // Not a chord any more: the editor is closed at the end of the frame, the action is kept.
            let ComboAction::Action(action) = &mut config.combos[combo].action else {
                return;
            };
            (place, action, None)
        }
    };

    let mut close = ui.input(|i| i.key_pressed(egui::Key::Escape));
    ui.horizontal(|ui| {
        let title = match labels::action_label(action) {
            label if label.is_empty() => match editor.target {
                EditTarget::ComboKey { .. } | EditTarget::ComboAction { .. } => "No action".to_string(),
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

    match editor.target {
        EditTarget::ComboKey { .. } => {
            ui.weak("The combo key matches any key or joystick button registered as this action in the current layout.");
            ui.add_space(theme::GAP_S);
        }
        EditTarget::ComboAction { .. } => {
            ui.weak("The chord keys pressed together do this action while held, instead of their own ones.");
            ui.add_space(theme::GAP_S);
        }
        _ => {}
    }
    if let (Some(error), KeyAction::Fn { .. }) = (&fn_error, &action) {
        ui.colored_label(ui.visuals().warn_fg_color, error);
        ui.add_space(theme::GAP_S);
    }
    // A combo key is matched against the action after the fall-through, so it's never `Transparent`.
    let hidden: &[ActionKind] = match editor.target {
        EditTarget::JoystickButton { .. } => &[ActionKind::Fn],
        EditTarget::ComboKey { .. } => &[ActionKind::Transparent],
        // See `ValidationError::ChordActionKind`.
        EditTarget::ComboAction { .. } => {
            &[ActionKind::Fn, ActionKind::Transparent, ActionKind::ModTap, ActionKind::LayerTap]
        }
        _ => &[],
    };
    action_editor(ui, action, &layouts, &mut editor.search, &mut editor.scroll_to_current, hidden);
    if close {
        app.key_editor = None;
    }
}

/// Edits `action`: kind selector and the fields of that kind, the `hidden` kinds aren't offered.
/// `scroll_to_current` scrolls the key list to the current key and is reset.
fn action_editor(
    ui: &mut egui::Ui,
    action: &mut KeyAction,
    layouts: &[String],
    search: &mut String,
    scroll_to_current: &mut bool,
    hidden: &[ActionKind],
) {
    let kind = ActionKind::of(action);
    ui.horizontal(|ui| {
        ui.label("Action:");
        // All the kinds without scrolling if the window is tall enough, otherwise down to its bottom
        // (the selector and a margin above it); an unlimited popup would be cut off by the window.
        // At least a few rows (22 + 2 spacing, see below), so it isn't empty near the bottom of the window.
        let below = ui.ctx().content_rect().bottom() - ui.cursor().top() - 3.0 * ui.spacing().interact_size.y;
        let below = below.max(5.0 * 24.0);
        let combo = egui::ComboBox::from_id_salt("action_kind").selected_text(kind.name()).height(below);
        combo.show_ui(ui, |ui| {
            // Compact rows like the key list, so all the kinds fit under the selector.
            ui.spacing_mut().item_spacing.y = 2.0;
            ui.spacing_mut().interact_size.y = 22.0;
            for other in ActionKind::ALL.into_iter().filter(|kind| !hidden.contains(kind)) {
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
            ui.label("Modifiers:");
            mods_editor(ui, mods);
            ui.add_space(theme::GAP_S);
            key_picker(ui, key, search, scroll_to_current);
        }
        KeyAction::Mod(mods) => {
            ui.label("Modifiers:");
            mods_editor(ui, mods);
        }
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
            let speed = Velocity::of(&KeyAction::MouseTravel { dx: *dx, dy: *dy });
            ui.weak(format!(
                "Cursor speed while held: pixels per {TRAVEL_UNIT_MS} ms, ({}, {}) px/s; negative dy is up.",
                speed.x, speed.y
            ));
        }
        KeyAction::MouseScroll { dy } => {
            ui.horizontal(|ui| {
                ui.label("Up (dy):");
                ui.add(egui::DragValue::new(dy).range(i8::MIN..=i8::MAX));
            });
            ui.weak("A press scrolls one step, held it scrolls this many steps per second; positive is up.");
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
        KeyAction::Transparent => {
            ui.weak("Does what this key does in the next active layer below; nothing in the base layer.");
        }
        KeyAction::Momentary(layer) => {
            layer_row(ui, "Layout:", "momentary_layer", layer, layouts);
            ui.weak("The layout is on top of the active ones while the key is held.");
        }
        KeyAction::Toggle(layer) => {
            layer_row(ui, "Layout:", "toggle_layer", layer, layouts);
            ui.weak("A press puts the layout on top of the active ones, the next press turns it off.");
        }
        KeyAction::To(layer) => {
            layer_row(ui, "Layout:", "to_layer", layer, layouts);
            ui.weak("A press makes the layout the base one and turns the other layers off.");
        }
        KeyAction::ModTap { mods, tap } => {
            ui.weak(TAP_HOLD_HINT);
            ui.add_space(theme::GAP_S);
            ui.label("Hold:");
            mods_editor(ui, mods);
            if mods.is_empty() {
                // The config with it is rejected, see `ActionError::ModTapWithoutMods`.
                ui.colored_label(
                    ui.visuals().warn_fg_color,
                    "Pick at least one modifier: without one the hold does nothing and the taps are delayed.",
                );
            }
            ui.add_space(theme::GAP_S);
            ui.label("Tap:");
            key_picker(ui, tap, search, scroll_to_current);
        }
        KeyAction::LayerTap { layer, tap } => {
            ui.weak(TAP_HOLD_HINT);
            ui.add_space(theme::GAP_S);
            layer_row(ui, "Hold:", "layer_tap_layer", layer, layouts);
            ui.add_space(theme::GAP_S);
            ui.label("Tap:");
            key_picker(ui, tap, search, scroll_to_current);
        }
        KeyAction::BtSelect(profile) => {
            ui.horizontal(|ui| {
                ui.label("Profile:");
                bt_profile_select(ui, "bt_select_profile", profile);
            });
            ui.weak(BT_HINT);
        }
        KeyAction::BtNext => {
            ui.weak("A press switches to the next BLE profile, after the last one to the first.");
            ui.add_space(theme::GAP_S);
            ui.weak(BT_HINT);
        }
        KeyAction::BtPrev => {
            ui.weak("A press switches to the previous BLE profile, before the first one to the last.");
            ui.add_space(theme::GAP_S);
            ui.weak(BT_HINT);
        }
        KeyAction::BtClear => {
            ui.weak(
                "A press forgets the host of the current BLE profile, the profile waits for a new host to pair. \
                The old host has to remove the keyboard before it can pair again.",
            );
        }
    }
}

/// How the BLE profiles work, see `keyboard/src/bt/profiles.rs`.
const BT_HINT: &str = "Each profile remembers one host. Switching drops the current host and connects to the \
    host of the profile; a profile without a host waits for a new one to pair. A host bound to one profile can't \
    pair with another one until its profile is cleared. The display shows e.g. BT 2 or BT 2 pair.";

/// How a tap-hold key decides, see [`protocol::keymap`].
const TAP_HOLD_HINT: &str = "Hold: held past the tapping term, or another key pressed and released meanwhile. \
    The timing is in the overview settings.";

/// `label` and a layout selector in one row.
fn layer_row(ui: &mut egui::Ui, label: &str, id_salt: &str, layer: &mut u8, layouts: &[String]) {
    ui.horizontal(|ui| {
        ui.label(label);
        layout_select(ui, id_salt, layer, layouts);
    });
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

/// Modifier checkboxes.
fn mods_editor(ui: &mut egui::Ui, mods: &mut KeyMod) {
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
