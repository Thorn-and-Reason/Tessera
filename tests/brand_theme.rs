#![cfg(feature = "desktop")]
use eframe::egui::{self, Color32, FontFamily, TextStyle};
use tessera::desktop::configure_context;

fn luminance(color: Color32) -> f64 {
    let linear = |v: u8| {
        let v = f64::from(v) / 255.;
        if v <= 0.04045 {
            v / 12.92
        } else {
            ((v + 0.055) / 1.055).powf(2.4)
        }
    };
    0.2126 * linear(color.r()) + 0.7152 * linear(color.g()) + 0.0722 * linear(color.b())
}
fn contrast(a: Color32, b: Color32) -> f64 {
    let (a, b) = (luminance(a), luminance(b));
    (a.max(b) + 0.05) / (a.min(b) + 0.05)
}
#[test]
fn desktop_theme_has_compact_controls_sans_headings_and_legible_surfaces() {
    let ctx = egui::Context::default();
    configure_context(&ctx);
    for theme in [egui::Theme::Light, egui::Theme::Dark] {
        let style = ctx.style_of(theme);
        let visual = &style.visuals;
        assert!(contrast(visual.text_color(), visual.panel_fill) >= 4.5);
        assert!(contrast(visual.weak_text_color(), visual.faint_bg_color) >= 4.5);
        assert!(contrast(visual.selection.stroke.color, visual.selection.bg_fill) >= 4.5);
        assert_eq!(
            style.text_styles[&TextStyle::Heading].family,
            FontFamily::Name("semibold".into())
        );
        assert!(style.spacing.interact_size.y <= 28.);
        assert!(style.text_styles[&TextStyle::Heading].size <= 18.);
        assert!(style.spacing.button_padding.y <= 4.);
    }
    assert_eq!(
        ctx.style_of(egui::Theme::Light).visuals.panel_fill,
        Color32::WHITE
    );
    assert_eq!(
        ctx.style_of(egui::Theme::Light).visuals.text_color(),
        Color32::from_rgb(39, 45, 55)
    );
}
