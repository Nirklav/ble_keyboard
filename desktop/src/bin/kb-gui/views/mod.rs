//! UI parts, each drawing a piece of [`App`](crate::app::App).

pub mod action_editor;
pub mod combos;
pub mod layout;
pub mod overview;
pub mod start;

use eframe::egui;
use protocol::config::{BT_PROFILES, JoystickButton, KeyAction, Layout};

use crate::app::{App, Screen};
use crate::labels;
use crate::theme;

/// Size of a key button on the grid.
const KEY_SIZE: egui::Vec2 = egui::vec2(76.0, 46.0);

/// Size of a big navigation button (layouts, combos).
pub const NAV_SIZE: egui::Vec2 = egui::vec2(180.0, 56.0);

/// Big navigation button of `size`: a title and a smaller second line, left aligned.
pub fn nav_button(ui: &mut egui::Ui, size: egui::Vec2, title: &str, subtitle: &str) -> egui::Response {
    let (rect, response) = ui.allocate_exact_size(size, egui::Sense::click());
    if ui.is_rect_visible(rect) {
        let visuals = ui.style().interact(&response);
        let painter = ui.painter();
        painter.rect(rect, visuals.corner_radius, visuals.weak_bg_fill, visuals.bg_stroke, egui::StrokeKind::Inside);
        let left = rect.left() + theme::GAP_M;
        let (title_y, align) = if subtitle.is_empty() {
            (rect.center().y, egui::Align2::LEFT_CENTER)
        } else {
            (rect.center().y - 1.0, egui::Align2::LEFT_BOTTOM)
        };
        let title_font = egui::FontId::proportional(16.0);
        painter.text(egui::pos2(left, title_y), align, title, title_font, visuals.text_color());
        if !subtitle.is_empty() {
            painter.text(
                egui::pos2(left, rect.center().y + 3.0),
                egui::Align2::LEFT_TOP,
                subtitle,
                egui::FontId::proportional(12.0),
                ui.visuals().weak_text_color(),
            );
        }
    }
    response
}

/// Title of the current screen with the "Back" button.
pub fn header(app: &mut App, ui: &mut egui::Ui) {
    let config = &app.doc.config;
    let title = match app.screen {
        Screen::Start | Screen::Overview => "Keyboard".to_string(),
        Screen::Layout(index) => format!("Layout {index}: {}", config.layouts[index].name),
        Screen::Combos => "Combos".to_string(),
        Screen::Combo(None) => "Built-in factory reset".to_string(),
        Screen::Combo(Some(index)) => format!("Combo {}", index + 1),
    };
    ui.horizontal(|ui| {
        let back = match app.screen {
            Screen::Overview if app.device.is_connected() => "‹ Disconnect",
            _ => "‹ Back",
        };
        let idle = !app.device.is_busy();
        if ui.add_enabled(idle || app.screen != Screen::Overview, egui::Button::new(back)).clicked() {
            match app.screen {
                Screen::Overview if app.device.is_connected() => app.device.send(crate::device::Command::Disconnect),
                screen => app.go(screen.parent()),
            }
        }
        ui.add_space(theme::GAP_XS);
        ui.label(theme::screen_title(title));
        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| app.header_actions(ui));
    });
}

/// How a key is drawn on the grid besides its action.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum KeyMark {
    None,
    /// Edited in the key editor.
    Selected,
    /// Part of something, e.g. of a combo.
    Marked,
}

/// Labels longer than this go on two lines, split at the first space or after the first `/`,
/// e.g. `Move` / `0,-10`, `Ctrl+Shift/` / `A`.
const KEY_LINE_CHARS: usize = 8;

/// Button of a key showing its `action`; layer keys are tinted, empty keys only outlined,
/// transparent ones also weak, `mark` overrides all.
fn key_button(ui: &mut egui::Ui, action: &KeyAction, mark: KeyMark, hover: String) -> egui::Response {
    let label = labels::action_label(action);
    let colors = match mark {
        KeyMark::Selected => Some((theme::ACCENT, theme::ACCENT_STROKE)),
        KeyMark::Marked => Some((theme::MARK_FILL, theme::MARK_STROKE)),
        KeyMark::None if action.layers().next().is_some() => Some((theme::FN_FILL, theme::FN_FILL)),
        KeyMark::None if matches!(action, KeyAction::None | KeyAction::Transparent) => {
            Some((egui::Color32::TRANSPARENT, ui.visuals().widgets.inactive.bg_fill))
        }
        KeyMark::None => None,
    };
    // Narrow padding: the key labels are short, but the keys are packed.
    ui.spacing_mut().button_padding.x = theme::GAP_XS;
    let button = match split_label(&label) {
        Some((first, rest)) if label.chars().count() > KEY_LINE_CHARS && !rest.is_empty() => {
            egui::Button::new(format!("{first}\n{rest}")).wrap_mode(egui::TextWrapMode::Extend)
        }
        _ if *action == KeyAction::Transparent && mark == KeyMark::None => {
            egui::Button::new(egui::RichText::new(label).color(ui.visuals().weak_text_color())).truncate()
        }
        _ => egui::Button::new(label).truncate(),
    };
    let mut button = button.min_size(KEY_SIZE);
    if let Some((fill, stroke)) = colors {
        button = button.fill(fill).stroke(egui::Stroke::new(1.0, stroke));
    }
    ui.add_sized(KEY_SIZE, button).on_hover_text(hover)
}

/// Where a long key label breaks: at the first space, or after the first `/`.
fn split_label(label: &str) -> Option<(&str, &str)> {
    label.split_once(' ').or_else(|| label.find('/').map(|i| label.split_at(i + 1)))
}

/// Spacing between the keys on the grid.
const KEY_GAP: f32 = 6.0;

/// Draws the joystick buttons of `layout` in a row, `selected` is marked.
/// Returns the clicked button.
pub fn joystick_row(ui: &mut egui::Ui, layout: &Layout, selected: Option<JoystickButton>) -> Option<JoystickButton> {
    let mut clicked = None;
    egui::Grid::new("joystick").spacing([KEY_GAP, theme::GAP_XS]).show(ui, |ui| {
        for button in JoystickButton::ALL {
            ui.weak(button.name());
        }
        ui.end_row();
        for button in JoystickButton::ALL {
            let action = layout.joystick.get(button);
            let hover = format!("{} button: {}", button.name(), labels::action_description(action));
            let mark = if selected == Some(button) { KeyMark::Selected } else { KeyMark::None };
            if key_button(ui, action, mark, hover).clicked() {
                clicked = Some(button);
            }
        }
        ui.end_row();
    });
    clicked
}

/// Draws `layout` as a grid of key buttons, `mark(x, y)` says how to draw each key.
/// Returns the clicked key `(x, y)`.
pub fn key_grid(ui: &mut egui::Ui, layout: &Layout, mark: impl Fn(usize, usize) -> KeyMark) -> Option<(usize, usize)> {
    let mut clicked = None;
    egui::Grid::new("key_grid").spacing([KEY_GAP, KEY_GAP]).show(ui, |ui| {
        for (y, row) in layout.rows.iter().enumerate() {
            for (x, action) in row.iter().enumerate() {
                let hover = format!("({x}, {y}): {}", labels::action_description(action));
                if key_button(ui, action, mark(x, y), hover).clicked() {
                    clicked = Some((x, y));
                }
            }
            ui.end_row();
        }
    });
    clicked
}

/// Combo box choosing a BLE profile index, shown as `BT 1`..
pub fn bt_profile_select(ui: &mut egui::Ui, id_salt: impl std::hash::Hash + std::fmt::Debug, value: &mut u8) {
    egui::ComboBox::from_id_salt(id_salt).selected_text(labels::bt_profile_name(*value)).show_ui(ui, |ui| {
        for index in 0..BT_PROFILES as u8 {
            ui.selectable_value(value, index, labels::bt_profile_name(index));
        }
    });
}

/// Combo box choosing a layout index among `layouts` names.
pub fn layout_select(
    ui: &mut egui::Ui,
    id_salt: impl std::hash::Hash + std::fmt::Debug,
    value: &mut u8,
    layouts: &[String],
) {
    egui::ComboBox::from_id_salt(id_salt).selected_text(labels::layout_choice(*value as usize, layouts)).show_ui(
        ui,
        |ui| {
            for index in 0..layouts.len() {
                ui.selectable_value(value, index as u8, labels::layout_choice(index, layouts));
            }
        },
    );
}
