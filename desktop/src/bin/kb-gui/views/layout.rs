//! Layout editor: name, order and the key grid.

use eframe::egui;
use protocol::config::{MAX_LAYOUT_NAME_LEN, MAX_LAYOUTS};

use crate::app::{App, EditTarget, KeyEditor, Screen};
use crate::editing;
use crate::views::{joystick_row, key_grid};

pub fn show(app: &mut App, ui: &mut egui::Ui, index: usize) {
    toolbar(app, ui, index);
    // The toolbar may have moved or removed the layout.
    let Screen::Layout(index) = app.screen else {
        return;
    };
    ui.add_space(8.0);

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
    let mut clicked = key_grid(ui, layout, |x, y| selected == Some((x, y)))
        .map(|(x, y)| EditTarget::LayoutKey { layout: index, x, y });
    ui.add_space(8.0);
    if let Some(button) = joystick_row(ui, layout, selected_button) {
        clicked = Some(EditTarget::JoystickButton { layout: index, button });
    }
    if let Some(target) = clicked {
        let search = app.key_editor.take().map(|e| e.search).unwrap_or_default();
        app.key_editor = Some(KeyEditor { target, search });
    }
    ui.add_space(4.0);
    ui.weak(
        "Click a key to edit it. Fn keys switch layouts: the first number while held, the second after a double tap. \
        Joystick buttons do what the layout active at the press says.",
    );
}

fn toolbar(app: &mut App, ui: &mut egui::Ui, index: usize) {
    let count = app.doc.config.layouts.len();
    ui.horizontal(|ui| {
        ui.label("Name:");
        let name = &mut app.doc.config.layouts[index].name;
        ui.add(egui::TextEdit::singleline(name).char_limit(MAX_LAYOUT_NAME_LEN).desired_width(140.0))
            .on_hover_text(format!("Shown on the keyboard display, at most {MAX_LAYOUT_NAME_LEN} ASCII characters"));
        ui.weak(format!("{}/{MAX_LAYOUT_NAME_LEN}", name.chars().count()));

        ui.separator();
        if ui.add_enabled(index > 0, egui::Button::new("Move left")).clicked() {
            editing::swap_layouts(&mut app.doc.config, index, index - 1);
            app.go(Screen::Layout(index - 1));
        }
        if ui.add_enabled(index + 1 < count, egui::Button::new("Move right")).clicked() {
            editing::swap_layouts(&mut app.doc.config, index, index + 1);
            app.go(Screen::Layout(index + 1));
        }
        if index == 0 {
            ui.weak("(active after boot)");
        }

        ui.separator();
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
        if ui.add_enabled(count > 1, egui::Button::new("Remove")).clicked() {
            match editing::remove_layout(&mut app.doc.config, index) {
                Ok(()) => app.go(Screen::Overview),
                Err(error) => app.message("Can't remove the layout", error),
            }
        }
    });
}
