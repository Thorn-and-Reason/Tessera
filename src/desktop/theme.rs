use eframe::egui::{self, Color32, FontData, FontDefinitions, FontFamily, FontId, TextStyle};
pub const NAVY: Color32 = Color32::from_rgb(38, 52, 73);
pub const PAPER: Color32 = Color32::from_rgb(242, 235, 220);
pub const SELECTED_LIGHT: Color32 = Color32::from_rgb(238, 230, 214);
pub const GOLD: Color32 = Color32::from_rgb(197, 155, 67);
pub const GOLD_INK: Color32 = Color32::from_rgb(134, 99, 31);
pub(crate) fn accent(ui: &egui::Ui) -> Color32 {
    if ui.visuals().dark_mode {
        GOLD
    } else {
        GOLD_INK
    }
}
pub fn install(ctx: &egui::Context) {
    let mut fonts = FontDefinitions::default();
    for (name, bytes) in [
        (
            "Noto Sans",
            include_bytes!("../../assets/fonts/NotoSans-Regular.ttf").as_slice(),
        ),
        (
            "Noto Sans Semibold",
            include_bytes!("../../assets/fonts/NotoSans-SemiBold.ttf").as_slice(),
        ),
        (
            "Noto Sans Mono",
            include_bytes!("../../assets/fonts/NotoSansMono-Regular.ttf").as_slice(),
        ),
        (
            "Libron",
            include_bytes!("../../assets/fonts/Libron-Regular.ttf").as_slice(),
        ),
    ] {
        fonts
            .font_data
            .insert(name.into(), FontData::from_static(bytes).into());
    }
    fonts
        .families
        .get_mut(&FontFamily::Proportional)
        .unwrap()
        .insert(0, "Noto Sans".into());
    fonts
        .families
        .get_mut(&FontFamily::Monospace)
        .unwrap()
        .insert(0, "Noto Sans Mono".into());
    fonts.families.insert(
        FontFamily::Name("semibold".into()),
        vec!["Noto Sans Semibold".into()],
    );
    fonts.families.insert(
        FontFamily::Name("brand-serif".into()),
        vec!["Libron".into(), "Noto Sans".into()],
    );
    ctx.set_fonts(fonts);
    ctx.data_mut(|d| d.insert_temp(egui::Id::new("tessera-brand-fonts"), true));
    for theme in [egui::Theme::Light, egui::Theme::Dark] {
        let mut style = (*ctx.style_of(theme)).clone();
        let dark = theme == egui::Theme::Dark;
        style.visuals = if dark {
            egui::Visuals::dark()
        } else {
            egui::Visuals::light()
        };
        let ink = if dark {
            Color32::from_rgb(242, 235, 220)
        } else {
            NAVY
        };
        let base = if dark {
            Color32::from_rgb(27, 37, 52)
        } else {
            PAPER
        };
        let border = Color32::from_rgb(125, 130, 127);
        let selected = if dark {
            Color32::from_rgb(62, 74, 91)
        } else {
            SELECTED_LIGHT
        };
        // Surfaces step away from the paper base: chrome (navigation, status,
        // project bar) sinks slightly; buttons and cards lift slightly.
        let chrome = if dark {
            Color32::from_rgb(33, 45, 62)
        } else {
            Color32::from_rgb(232, 223, 205)
        };
        let card = if dark {
            Color32::from_rgb(42, 56, 76)
        } else {
            Color32::from_rgb(249, 244, 232)
        };
        style.visuals.override_text_color = Some(ink);
        style.visuals.weak_text_color = Some(if dark {
            Color32::from_rgb(208, 199, 180)
        } else {
            Color32::from_rgb(78, 90, 102)
        });
        style.visuals.panel_fill = base;
        style.visuals.window_fill = base;
        style.visuals.extreme_bg_color = card;
        style.visuals.faint_bg_color = chrome;
        style.visuals.selection.bg_fill = selected;
        style.visuals.selection.stroke = egui::Stroke::new(1.5, ink);
        style.visuals.error_fg_color = if dark {
            Color32::from_rgb(255, 162, 151)
        } else {
            Color32::from_rgb(153, 38, 33)
        };
        style.visuals.warn_fg_color = if dark {
            GOLD
        } else {
            Color32::from_rgb(114, 77, 0)
        };
        style.visuals.widgets.noninteractive.bg_stroke = egui::Stroke::new(1., border);
        style.visuals.widgets.inactive.bg_fill = card;
        style.visuals.widgets.inactive.weak_bg_fill = card;
        style.visuals.widgets.inactive.bg_stroke = egui::Stroke::new(1., border);
        style.visuals.widgets.hovered.bg_fill = selected;
        style.visuals.widgets.hovered.weak_bg_fill = selected;
        style.visuals.widgets.hovered.bg_stroke = egui::Stroke::new(1.5, ink);
        style.visuals.widgets.active.bg_fill = selected;
        style.visuals.widgets.active.bg_stroke = egui::Stroke::new(2., ink);
        for widget in [
            &mut style.visuals.widgets.inactive,
            &mut style.visuals.widgets.hovered,
            &mut style.visuals.widgets.active,
        ] {
            widget.fg_stroke = egui::Stroke::new(1.5, ink);
            widget.corner_radius = egui::CornerRadius::same(6);
        }
        style.spacing.item_spacing = egui::vec2(10., 10.);
        style.spacing.button_padding = egui::vec2(12., 7.);
        style.spacing.interact_size.y = 30.;
        style
            .text_styles
            .insert(TextStyle::Body, FontId::proportional(14.));
        style
            .text_styles
            .insert(TextStyle::Button, FontId::proportional(14.));
        style.text_styles.insert(
            TextStyle::Heading,
            FontId::new(23., FontFamily::Name("brand-serif".into())),
        );
        style
            .text_styles
            .insert(TextStyle::Small, FontId::proportional(12.));
        style
            .text_styles
            .insert(TextStyle::Monospace, FontId::monospace(13.));
        ctx.set_style_of(theme, style);
    }
    ctx.set_theme(egui::ThemePreference::System);
}
pub(crate) fn text(ui: &egui::Ui) -> Color32 {
    ui.visuals().text_color()
}
pub(crate) fn surface(ui: &egui::Ui) -> Color32 {
    ui.visuals().faint_bg_color
}
pub(crate) fn line(ui: &egui::Ui) -> Color32 {
    ui.visuals().widgets.noninteractive.bg_stroke.color
}
pub(crate) fn muted(ui: &egui::Ui) -> Color32 {
    ui.visuals().weak_text_color()
}
pub(crate) fn selection(ui: &egui::Ui) -> Color32 {
    ui.visuals().selection.bg_fill
}
pub(crate) fn inspector_visible(ctx: &egui::Context) -> bool {
    ctx.content_rect().width() >= 1000.
}
pub(crate) fn inspector_width(ctx: &egui::Context) -> f32 {
    (ctx.content_rect().width() * 0.26).clamp(260., 340.)
}
/// Brand fonts are bound only by [`install`]; bare test contexts fall back to
/// the heading family so uninstalled font families never panic.
fn section_family(ui: &egui::Ui) -> FontFamily {
    let installed = ui
        .ctx()
        .data(|d| d.get_temp::<bool>(egui::Id::new("tessera-brand-fonts")) == Some(true));
    if installed {
        FontFamily::Name("semibold".into())
    } else {
        TextStyle::Heading.resolve(ui.style()).family
    }
}
pub(crate) fn section(ui: &mut egui::Ui, label: &str) -> egui::Response {
    ui.label(
        egui::RichText::new(label)
            .font(FontId::new(15.5, section_family(ui)))
            .color(text(ui)),
    )
}
pub(crate) fn wordmark(ui: &mut egui::Ui, label: &str) -> egui::Response {
    let family = TextStyle::Heading.resolve(ui.style()).family;
    ui.label(
        egui::RichText::new(label)
            .font(FontId::new(17., family))
            .color(muted(ui)),
    )
}
pub(crate) fn primary<'a>(ui: &egui::Ui, label: &'a str) -> egui::Button<'a> {
    // The decisive action must stay visible on both bases: navy on paper,
    // brand gold on navy. Both pairings clear 4.5:1 (see brand_theme tests).
    if ui.visuals().dark_mode {
        egui::Button::new(egui::RichText::new(label).color(NAVY).strong())
            .fill(GOLD)
            .stroke(egui::Stroke::new(1., GOLD))
    } else {
        egui::Button::new(egui::RichText::new(label).color(PAPER).strong())
            .fill(NAVY)
            .stroke(egui::Stroke::new(1., NAVY))
    }
}
pub(crate) fn secondary<'a>(_ui: &egui::Ui, label: &'a str) -> egui::Button<'a> {
    egui::Button::new(egui::RichText::new(label))
}
/// Borderless until hover; for toggles and trailing utilities that should not
/// compete with the task's next action.
pub(crate) fn quiet<'a>(_ui: &egui::Ui, label: &'a str) -> egui::Button<'a> {
    egui::Button::new(egui::RichText::new(label))
        .fill(Color32::TRANSPARENT)
        .stroke(egui::Stroke::NONE)
        .frame_when_inactive(false)
}
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Kind {
    Primary,
    Secondary,
    Quiet,
}
impl Kind {
    pub(crate) fn build<'a>(self, ui: &egui::Ui, label: &'a str) -> egui::Button<'a> {
        match self {
            Self::Primary => primary(ui, label),
            Self::Secondary => secondary(ui, label),
            Self::Quiet => quiet(ui, label),
        }
    }
}

pub fn mark(ctx: &egui::Context) -> Result<egui::TextureHandle, image::ImageError> {
    let image =
        image::load_from_memory(include_bytes!("../../assets/tessera-icon.png"))?.into_rgba8();
    Ok(ctx.load_texture(
        "Tessera tiled monogram",
        egui::ColorImage::from_rgba_unmultiplied(
            [image.width() as usize, image.height() as usize],
            image.as_raw(),
        ),
        egui::TextureOptions::LINEAR,
    ))
}

/// Bound presentation before egui converts labels into owned text. Authoritative strings
/// remain untouched. Returning fmt::Error stops Debug/container formatting at the cap.
pub(crate) fn caption(value: impl std::fmt::Display) -> String {
    caption_limit(value, 256)
}
pub(crate) fn caption_limit(value: impl std::fmt::Display, limit: usize) -> String {
    use std::fmt::Write;
    struct Caption {
        text: String,
        limit: usize,
    }
    impl Write for Caption {
        fn write_str(&mut self, text: &str) -> std::fmt::Result {
            let room = self.limit - 3 - self.text.len();
            if text.len() <= room {
                self.text.push_str(text);
                return Ok(());
            }
            let mut end = room;
            while !text.is_char_boundary(end) {
                end -= 1;
            }
            self.text.push_str(&text[..end]);
            self.text.push('…');
            Err(std::fmt::Error)
        }
    }
    let limit = limit.clamp(3, 256);
    let mut result = Caption {
        text: String::with_capacity(limit),
        limit,
    };
    let _ = write!(&mut result, "{value}");
    result.text
}

pub(crate) fn compact_number(value: f64) -> String {
    if value != 0.0 && (value.abs() < 0.005 || value.abs() >= 1e9) {
        format!("{value:.2e}")
    } else {
        format!("{value:.2}")
    }
}

pub(crate) use crate::search::NameQuery;
#[cfg(test)]
mod tests {
    use super::*;
    fn contrast(a: Color32, b: Color32) -> f32 {
        let luminance = |colour: Color32| {
            let linear = |byte: u8| {
                let channel = f32::from(byte) / 255.;
                if channel <= 0.04045 {
                    channel / 12.92
                } else {
                    ((channel + 0.055) / 1.055).powf(2.4)
                }
            };
            0.2126 * linear(colour.r()) + 0.7152 * linear(colour.g()) + 0.0722 * linear(colour.b())
        };
        let (a, b) = (luminance(a), luminance(b));
        (a.max(b) + 0.05) / (a.min(b) + 0.05)
    }
    #[test]
    fn both_themes_keep_text_and_focus_legible() {
        let ctx = egui::Context::default();
        install(&ctx);
        for appearance in [egui::Theme::Light, egui::Theme::Dark] {
            let style = ctx.style_of(appearance);
            let visual = &style.visuals;
            let foreground = visual.override_text_color.unwrap();
            for background in [
                visual.panel_fill,
                visual.faint_bg_color,
                visual.selection.bg_fill,
            ] {
                assert!(contrast(foreground, background) >= 4.5);
                assert!(contrast(visual.weak_text_color(), background) >= 4.5);
                assert!(contrast(visual.selection.stroke.color, background) >= 3.);
            }
            for foreground in [visual.error_fg_color, visual.warn_fg_color] {
                for background in [visual.panel_fill, visual.faint_bg_color] {
                    assert!(contrast(foreground, background) >= 4.5);
                }
            }
            assert!(
                contrast(
                    visual.widgets.inactive.bg_stroke.color,
                    visual.widgets.inactive.bg_fill
                ) >= 3.
            );
        }
        assert!(contrast(NAVY, GOLD) >= 4.5);
    }
    #[test]
    fn captions_and_numbers_preserve_boundaries() {
        assert_eq!(caption("ŵŶ"), "ŵŶ");
        assert!(caption("ŵ".repeat(300)).len() <= 256);
        assert_eq!(compact_number(23.333333333333332), "23.33");
        assert_eq!(compact_number(4.844240566555988), "4.84");
        assert_eq!(compact_number(0.0), "0.00");
        assert_eq!(compact_number(1e-280), "1.00e-280");
        assert_eq!(compact_number(-1e280), "-1.00e280");
        assert!(NameQuery::new("gôr ŵ").unwrap().matches("Sgôr ŵ"));
        assert!(!NameQuery::new("unrelated").unwrap().matches("Sgôr ŵ"));
    }
}
