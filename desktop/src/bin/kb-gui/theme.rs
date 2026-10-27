//! Look of the app: spacing, text sizes and colors, and the styled widgets built from them.
//! Views take every gap and color from here instead of their own numbers.

use eframe::egui::{self, Color32, CornerRadius, FontFamily, FontId, RichText, Stroke, TextStyle};

/// Spacing scale.
pub const GAP_XS: f32 = 4.0;
pub const GAP_S: f32 = 8.0;
pub const GAP_M: f32 = 12.0;
pub const GAP_L: f32 = 16.0;
pub const GAP_XL: f32 = 24.0;

/// Corner radius of buttons, fields and cards.
pub const RADIUS: u8 = 6;

/// Widest the content gets: the default 10x5 key grid with the key editor beside it.
/// A wider window (a maximized one, a browser tab) gets empty sides instead of the content at its edges.
pub const MAX_WIDTH: f32 = 1200.0;

/// Main action and selection.
pub const ACCENT: Color32 = Color32::from_rgb(47, 111, 214);
/// Outline of a selected key.
pub const ACCENT_STROKE: Color32 = Color32::from_rgb(120, 170, 255);
/// Text on [`ACCENT`].
pub const ON_ACCENT: Color32 = Color32::WHITE;
/// Fn keys: they switch layouts.
pub const FN_FILL: Color32 = Color32::from_rgb(76, 58, 118);
/// Keys marked on the grid, e.g. the keys of a combo.
pub const MARK_FILL: Color32 = Color32::from_rgb(104, 76, 24);
pub const MARK_STROKE: Color32 = Color32::from_rgb(222, 166, 64);
pub const SUCCESS: Color32 = Color32::from_rgb(92, 190, 120);
pub const WARNING: Color32 = Color32::from_rgb(230, 180, 80);
pub const DANGER: Color32 = Color32::from_rgb(235, 100, 90);

/// Body text; the egui default is too dim on the dark background.
const TEXT: Color32 = Color32::from_gray(200);
/// Hints and secondary lines, still 5.5:1 on the panel.
const WEAK_TEXT: Color32 = Color32::from_gray(140);
const STRONG_TEXT: Color32 = Color32::from_gray(240);

/// Title of a section inside a screen.
fn section_style() -> TextStyle {
    TextStyle::Name("section".into())
}

/// Sets the dark theme and the app style on `ctx`.
pub fn apply(ctx: &egui::Context) {
    ctx.set_theme(egui::Theme::Dark);
    ctx.style_mut_of(egui::Theme::Dark, |style| {
        use FontFamily::Proportional;
        style.text_styles = [
            (TextStyle::Heading, FontId::new(20.0, Proportional)),
            (section_style(), FontId::new(16.0, Proportional)),
            (TextStyle::Body, FontId::new(14.0, Proportional)),
            (TextStyle::Button, FontId::new(14.0, Proportional)),
            (TextStyle::Small, FontId::new(12.0, Proportional)),
            (TextStyle::Monospace, FontId::new(13.0, FontFamily::Monospace)),
        ]
        .into();

        let spacing = &mut style.spacing;
        spacing.item_spacing = egui::vec2(GAP_S, 6.0);
        spacing.button_padding = egui::vec2(10.0, 4.0);
        spacing.interact_size.y = 26.0;
        spacing.window_margin = egui::Margin::same(GAP_L as i8);

        let visuals = &mut style.visuals;
        visuals.selection.bg_fill = ACCENT;
        visuals.selection.stroke = Stroke::new(1.0, ON_ACCENT);
        visuals.weak_text_color = Some(WEAK_TEXT);
        visuals.warn_fg_color = WARNING;
        visuals.error_fg_color = DANGER;
        visuals.window_corner_radius = CornerRadius::same(10);
        visuals.menu_corner_radius = CornerRadius::same(RADIUS);
        let widgets = &mut visuals.widgets;
        widgets.noninteractive.fg_stroke.color = TEXT;
        widgets.inactive.fg_stroke.color = TEXT;
        widgets.hovered.fg_stroke.color = STRONG_TEXT;
        widgets.active.fg_stroke.color = STRONG_TEXT;
        let all = [&mut widgets.noninteractive, &mut widgets.inactive, &mut widgets.hovered, &mut widgets.active];
        for widget in all.into_iter().chain([&mut widgets.open]) {
            widget.corner_radius = CornerRadius::same(RADIUS);
        }
    });
}

/// Middle part of `rect`, at most [`MAX_WIDTH`] wide: the column every panel lines its content up to.
pub fn column(rect: egui::Rect) -> egui::Rect {
    let width = rect.width().min(MAX_WIDTH);
    let left = rect.center().x - width / 2.0;
    egui::Rect::from_x_y_ranges(left..=left + width, rect.y_range())
}

/// Runs `add` in the [`column`] of the space left in `ui`.
pub fn centered<R>(ui: &mut egui::Ui, add: impl FnOnce(&mut egui::Ui) -> R) -> R {
    let rect = column(ui.available_rect_before_wrap());
    ui.scope_builder(egui::UiBuilder::new().max_rect(rect).layout(*ui.layout()), add).inner
}

/// The main action of a screen.
pub fn primary_button(text: &str) -> egui::Button<'static> {
    egui::Button::new(RichText::new(text).color(ON_ACCENT)).fill(ACCENT)
}

/// An action that loses data: removes, resets.
pub fn danger_button(text: &str) -> egui::Button<'static> {
    egui::Button::new(RichText::new(text).color(DANGER))
}

/// Title of the current screen.
pub fn screen_title(text: impl Into<String>) -> RichText {
    RichText::new(text).heading().color(STRONG_TEXT)
}

/// Title of a section inside a screen.
pub fn section_heading(ui: &mut egui::Ui, text: impl Into<String>) {
    ui.label(RichText::new(text).text_style(section_style()).color(STRONG_TEXT));
}

/// Colored dot and a label, e.g. the connection state. One widget, so it reads the same in a
/// right-to-left layout; the dot is painted, the default fonts have no circle glyph.
pub fn status(ui: &mut egui::Ui, color: Color32, text: &str) -> egui::Response {
    const DOT: f32 = 8.0;
    let galley = egui::WidgetText::from(text).into_galley(ui, None, f32::INFINITY, TextStyle::Body);
    let size = egui::vec2(DOT + GAP_S + galley.size().x, galley.size().y);
    let (rect, response) = ui.allocate_exact_size(size, egui::Sense::hover());
    let painter = ui.painter();
    painter.circle_filled(egui::pos2(rect.left() + DOT / 2.0, rect.center().y), DOT / 2.0, color);
    painter.galley(egui::pos2(rect.left() + DOT + GAP_S, rect.top()), galley, ui.visuals().text_color());
    response
}

/// Small colored square and a weak label explaining that color.
pub fn legend(ui: &mut egui::Ui, fill: Color32, stroke: Color32, text: &str) {
    ui.horizontal(|ui| {
        ui.spacing_mut().item_spacing.x = GAP_S;
        let (rect, _) = ui.allocate_exact_size(egui::vec2(14.0, 14.0), egui::Sense::hover());
        ui.painter().rect(rect, CornerRadius::same(3), fill, Stroke::new(1.0, stroke), egui::StrokeKind::Inside);
        ui.weak(text);
    });
}

/// Segmented toggle: `options` side by side, the `selected` one filled. Returns the clicked other one.
pub fn segmented(ui: &mut egui::Ui, selected: usize, options: &[&str]) -> Option<usize> {
    let mut clicked = None;
    ui.horizontal(|ui| {
        ui.spacing_mut().item_spacing.x = 0.0;
        let last = options.len().saturating_sub(1);
        for (index, text) in options.iter().enumerate() {
            let (left, right) = (if index == 0 { RADIUS } else { 0 }, if index == last { RADIUS } else { 0 });
            let corners = CornerRadius { nw: left, sw: left, ne: right, se: right };
            let button = if index == selected {
                egui::Button::new(RichText::new(*text).color(ON_ACCENT)).fill(ACCENT)
            } else {
                egui::Button::new(*text)
            };
            if ui.add(button.corner_radius(corners)).clicked() && index != selected {
                clicked = Some(index);
            }
        }
    });
    clicked
}
