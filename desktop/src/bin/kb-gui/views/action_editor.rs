//! Key action editor: a window editing one layout key, joystick button or combo key matched by action.

use eframe::egui;
use protocol::config::{ComboKeyIdent, KeyAction};
use protocol::keys::{Key, KeyMod, MediaKey};

use crate::app::{App, EditTarget};
use crate::editing::{self, ActionKind};
use crate::labels;
use crate::views::layout_select;

/// Shows the editor window if [`App::key_editor`] is set.
pub fn window(app: &mut App, ctx: &egui::Context) {
    let Some(editor) = &mut app.key_editor else {
        return;
    };
    let layouts = editing::layout_names(&app.doc.config);
    let config = &mut app.doc.config;

    // `fn_error`: why Fn can't be here.
    let (title, action, fn_error) = match editor.target {
        EditTarget::LayoutKey { layout, x, y } => {
            let title = format!("Key ({x}, {y}) of {}", config.layouts[layout].name);
            let fn_error = editing::other_fn_key(&config.layouts[layout], x, y)
                .map(|(fx, fy)| format!("This layout already has an Fn key at ({fx}, {fy}), only one is allowed."));
            (title, &mut config.layouts[layout].rows[y][x], fn_error)
        }
        EditTarget::JoystickButton { layout, button } => {
            let title = format!("Joystick {} button of {}", button.name().to_lowercase(), config.layouts[layout].name);
            let fn_error = Some("Fn can't be on a joystick button.".to_string());
            (title, config.layouts[layout].joystick.get_mut(button), fn_error)
        }
        EditTarget::ComboKey { combo, key } => {
            let title = format!("Combo {combo}, key {key}");
            let ident = &mut config.combos[combo].keys[key].ident;
            if !matches!(ident, ComboKeyIdent::Action(_)) {
                *ident = ComboKeyIdent::Action(KeyAction::None);
            }
            let ComboKeyIdent::Action(action) = ident else { unreachable!() };
            (title, action, None)
        }
    };

    let mut open = true;
    let mut done = false;
    egui::Window::new(title)
        .id(egui::Id::unique("key_editor"))
        .open(&mut open)
        .resizable(false)
        .collapsible(false)
        .default_pos(ctx.content_rect().center() - egui::vec2(160.0, 200.0))
        .show(ctx, |ui| {
            if matches!(editor.target, EditTarget::ComboKey { .. }) {
                ui.weak(
                    "The combo key matches any key or joystick button registered as this action in the current layout.",
                );
                ui.add_space(4.0);
            }
            let allow_fn = !matches!(editor.target, EditTarget::JoystickButton { .. });
            action_editor(ui, action, &layouts, &mut editor.search, allow_fn);
            if let (Some(error), KeyAction::Fn { .. }) = (&fn_error, &action) {
                ui.add_space(4.0);
                ui.colored_label(ui.visuals().warn_fg_color, error);
            }
            ui.separator();
            if ui.button("Done").clicked() {
                done = true;
            }
        });
    if !open || done {
        app.key_editor = None;
    }
}

/// Edits `action`: kind selector and the fields of that kind, `allow_fn` offers [`KeyAction::Fn`].
pub fn action_editor(ui: &mut egui::Ui, action: &mut KeyAction, layouts: &[String], search: &mut String, allow_fn: bool) {
    let kind = ActionKind::of(action);
    ui.horizontal(|ui| {
        ui.label("Action:");
        egui::ComboBox::from_id_salt("action_kind").selected_text(kind.name()).show_ui(ui, |ui| {
            for other in ActionKind::ALL.into_iter().filter(|&kind| allow_fn || kind != ActionKind::Fn) {
                if ui.selectable_label(other == kind, other.name()).clicked() && other != kind {
                    *action = other.convert(action);
                }
            }
        });
    });
    ui.add_space(4.0);

    match action {
        KeyAction::Key(key) => key_picker(ui, key, search),
        KeyAction::KeyWithMod(key, mods) => {
            mods_editor(ui, mods);
            ui.add_space(4.0);
            key_picker(ui, key, search);
        }
        KeyAction::Mod(mods) => mods_editor(ui, mods),
        KeyAction::Media(media) => {
            for key in MediaKey::ALL {
                ui.radio_value(media, key, key.name());
            }
        }
        KeyAction::MouseKey(buttons) => {
            ui.horizontal(|ui| {
                for (button, name, _) in labels::MOUSE_BUTTONS {
                    let mut on = buttons.contains(button);
                    if ui.checkbox(&mut on, name).changed() {
                        buttons.set(button, on);
                    }
                }
            });
        }
        KeyAction::MouseTravel { dx, dy } => {
            ui.horizontal(|ui| {
                ui.add(egui::DragValue::new(dx).prefix("dx: ").range(i8::MIN..=i8::MAX));
                ui.add(egui::DragValue::new(dy).prefix("dy: ").range(i8::MIN..=i8::MAX));
            });
            ui.weak("Cursor move per report while held; negative dy is up.");
        }
        KeyAction::MouseScroll { dy } => {
            ui.add(egui::DragValue::new(dy).prefix("dy: ").range(i8::MIN..=i8::MAX));
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

/// Searchable list of all keys.
fn key_picker(ui: &mut egui::Ui, key: &mut Key, search: &mut String) {
    ui.horizontal(|ui| {
        ui.label("Key:");
        ui.strong(format!("{} ({})", labels::key_label(*key), labels::key_name(*key)));
    });
    ui.add(egui::TextEdit::singleline(search).hint_text("Search keys").desired_width(240.0));
    let filter = search.trim().to_lowercase();
    egui::ScrollArea::vertical().max_height(220.0).auto_shrink([false, true]).show(ui, |ui| {
        for candidate in labels::KEYS {
            let name = labels::key_name(candidate);
            let label = labels::key_label(candidate);
            if !filter.is_empty() && !name.to_lowercase().contains(&filter) && !label.to_lowercase().contains(&filter) {
                continue;
            }
            let text = if label == name { name } else { format!("{label}  ({name})") };
            let response = ui.selectable_value(key, candidate, text);
            if response.clicked() {
                response.scroll_to_me(None);
            }
        }
    });
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
