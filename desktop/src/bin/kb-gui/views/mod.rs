//! UI parts, each drawing a piece of [`App`](crate::app::App).

pub mod action_editor;
pub mod combos;
pub mod layout;
pub mod overview;
pub mod start;

use eframe::egui;
use protocol::config::{KeyAction, Layout};

use crate::app::{App, Screen};
use crate::labels;

/// Size of a key button on the grid.
const KEY_SIZE: egui::Vec2 = egui::vec2(72.0, 44.0);

/// Size of a big navigation button (layouts, combos).
pub const NAV_SIZE: egui::Vec2 = egui::vec2(180.0, 56.0);

/// Big navigation button of `size`: a title and a smaller second line, left aligned.
pub fn nav_button(ui: &mut egui::Ui, size: egui::Vec2, title: &str, subtitle: &str) -> egui::Response {
    let (rect, response) = ui.allocate_exact_size(size, egui::Sense::click());
    if ui.is_rect_visible(rect) {
        let visuals = ui.style().interact(&response);
        let painter = ui.painter();
        painter.rect(rect, visuals.corner_radius, visuals.weak_bg_fill, visuals.bg_stroke, egui::StrokeKind::Inside);
        let left = rect.left() + 12.0;
        let (title_y, align) = if subtitle.is_empty() {
            (rect.center().y, egui::Align2::LEFT_CENTER)
        } else {
            (rect.center().y - 1.0, egui::Align2::LEFT_BOTTOM)
        };
        painter.text(egui::pos2(left, title_y), align, title, egui::FontId::proportional(16.0), visuals.text_color());
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
            Screen::Overview if app.device.connected => "< Disconnect",
            _ => "< Back",
        };
        let idle = !app.device.is_busy();
        if ui.add_enabled(idle || app.screen != Screen::Overview, egui::Button::new(back)).clicked() {
            match app.screen {
                Screen::Overview if app.device.connected => app.device.send(crate::device::Command::Disconnect),
                screen => app.go(screen.parent()),
            }
        }
        ui.heading(title);
        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| app.push_button(ui));
    });
    ui.separator();
}

/// Draws `layout` as a grid of key buttons, `highlight(x, y)` marks keys as selected.
/// Returns the clicked key `(x, y)`.
pub fn key_grid(
    ui: &mut egui::Ui,
    layout: &Layout,
    highlight: impl Fn(usize, usize) -> bool,
) -> Option<(usize, usize)> {
    let mut clicked = None;
    egui::Grid::new("key_grid").spacing([4.0, 4.0]).show(ui, |ui| {
        for (y, row) in layout.rows.iter().enumerate() {
            for (x, action) in row.iter().enumerate() {
                let mut button = egui::Button::new(labels::action_label(action))
                    .min_size(KEY_SIZE)
                    .selected(highlight(x, y))
                    .truncate();
                if matches!(action, KeyAction::Fn { .. }) && !highlight(x, y) {
                    button =
                        button.fill(ui.visuals().widgets.inactive.bg_fill.lerp_to_gamma(egui::Color32::BLUE, 0.25));
                }
                let response = ui
                    .add_sized(KEY_SIZE, button)
                    .on_hover_text(format!("({x}, {y}): {}", labels::action_description(action)));
                if response.clicked() {
                    clicked = Some((x, y));
                }
            }
            ui.end_row();
        }
    });
    clicked
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
