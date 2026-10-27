//! Layout editor: name, order and the key grid.

use eframe::egui;
use protocol::config::{JoystickButton, KeyAction, MAX_LAYOUT_NAME_LEN, MAX_LAYOUTS};

use crate::app::{App, EditTarget, KeyEditor, Screen};
use crate::editing;
use crate::theme;
use crate::views::{KeyMark, joystick_row, key_grid};

pub fn show(app: &mut App, ui: &mut egui::Ui, index: usize) {
    toolbar(app, ui, index);
    // The toolbar may have moved or removed the layout.
    let Screen::Layout(index) = app.screen else {
        return;
    };
    ui.add_space(theme::GAP_M);

    let target = app.key_editor.as_ref().map(|e| e.target);
    let selected = match target {
        Some(EditTarget::LayoutKey { layout, x, y }) if layout == index => Some((x, y)),
        _ => None,
    };
    let selected_button = match target {
        Some(EditTarget::JoystickButton { layout, button }) if layout == index => Some(button),
        _ => None,
    };
    let layout = &app.doc.config.layouts[index];
    let mark = |x, y| if selected == Some((x, y)) { KeyMark::Selected } else { KeyMark::None };
    let mut clicked = key_grid(ui, layout, mark).map(|(x, y)| EditTarget::LayoutKey { layout: index, x, y });
    ui.add_space(theme::GAP_S);
    ui.horizontal(|ui| {
        ui.weak("Click a key to edit it.");
        ui.add_space(theme::GAP_L);
        let actions = || layout.rows.iter().flatten().chain(JoystickButton::ALL.map(|b| layout.joystick.get(b)));
        if actions().any(|action| matches!(action, KeyAction::Fn { .. })) {
            let text = "Layer keys; Fn: the first layout while held, the second after a double tap";
            theme::legend(ui, theme::FN_FILL, theme::FN_FILL, text);
        } else if actions().any(|action| action.layers().next().is_some()) {
            theme::legend(ui, theme::FN_FILL, theme::FN_FILL, "Layer keys");
        }
    });
    ui.add_space(theme::GAP_L);
    theme::section_heading(ui, "Joystick");
    if let Some(button) = joystick_row(ui, layout, selected_button) {
        clicked = Some(EditTarget::JoystickButton { layout: index, button });
    }
    if let Some(target) = clicked {
        app.key_editor = Some(KeyEditor::open(target, app.key_editor.take()));
    }
    ui.weak("The buttons do what the layout active at the press says.");
}

fn toolbar(app: &mut App, ui: &mut egui::Ui, index: usize) {
    let count = app.doc.config.layouts.len();
    ui.horizontal(|ui| {
        ui.label("Name");
        let name = &mut app.doc.config.layouts[index].name;
        ui.add(egui::TextEdit::singleline(name).char_limit(MAX_LAYOUT_NAME_LEN).desired_width(140.0))
            .on_hover_text(format!("Shown on the keyboard display, at most {MAX_LAYOUT_NAME_LEN} ASCII characters"));
        ui.weak(format!("{}/{MAX_LAYOUT_NAME_LEN}", name.chars().count()))
            .on_hover_text(format!("At most {MAX_LAYOUT_NAME_LEN} characters"));

        ui.add_space(theme::GAP_L);
        let left =
            ui.add_enabled(index > 0, egui::Button::new("‹")).on_hover_text("Move left, before the previous layout");
        if left.clicked() {
            editing::swap_layouts(&mut app.doc.config, index, index - 1);
            app.go(Screen::Layout(index - 1));
        }
        ui.label(format!("{} of {count}", index + 1));
        let right = ui
            .add_enabled(index + 1 < count, egui::Button::new("›"))
            .on_hover_text("Move right, after the next layout");
        if right.clicked() {
            editing::swap_layouts(&mut app.doc.config, index, index + 1);
            app.go(Screen::Layout(index + 1));
        }
        if index == 0 {
            ui.weak("active after boot");
        }

        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
            let remove = ui
                .add_enabled(count > 1, theme::danger_button("Remove"))
                .on_disabled_hover_text("The only layout can't be removed");
            if remove.clicked() {
                match editing::remove_layout(&mut app.doc.config, index) {
                    Ok(()) => app.go(Screen::Overview),
                    Err(error) => app.message("Can't remove the layout", error),
                }
            }
            let duplicate = ui
                .add_enabled(count < MAX_LAYOUTS, egui::Button::new("Duplicate"))
                .on_hover_text("Copy of this layout")
                .on_disabled_hover_text(format!("At most {MAX_LAYOUTS} layouts"));
            if duplicate.clicked() {
                match editing::duplicate_layout(&mut app.doc.config, index) {
                    Ok(index) => app.go(Screen::Layout(index)),
                    Err(error) => app.message("Can't duplicate the layout", error),
                }
            }
        });
    });
}
