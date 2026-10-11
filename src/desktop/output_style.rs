//! Styling applies only to captured Results; the workspace theme stays unchanged.
use eframe::egui::{self, Color32, FontId, TextStyle};

pub(super) const BLUE: Color32 = Color32::from_rgb(242, 235, 220);
pub(super) fn blue(ui: &egui::Ui) -> Color32 {
    if ui.visuals().dark_mode {
        Color32::from_rgb(62, 74, 91)
    } else {
        BLUE
    }
}

pub(super) fn apply(ui: &mut egui::Ui) {
    let heading = TextStyle::Heading.resolve(ui.style());
    let style = ui.style_mut();
    style
        .text_styles
        .insert(TextStyle::Heading, FontId::new(19., heading.family));
}
