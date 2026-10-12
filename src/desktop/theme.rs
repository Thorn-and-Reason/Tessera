use eframe::egui::{self, Color32, FontData, FontDefinitions, FontFamily, FontId, TextStyle};
pub const NAVY: Color32 = Color32::from_rgb(38, 52, 73);
pub const SELECTED_LIGHT: Color32 = Color32::from_rgb(228, 237, 247);
pub const GOLD: Color32 = Color32::from_rgb(197, 155, 67);
pub(crate) fn accent(ui: &egui::Ui) -> Color32 {
    if ui.visuals().dark_mode {
        Color32::from_rgb(158, 190, 224)
    } else {
        Color32::from_rgb(54, 94, 135)
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
    #[cfg(target_os = "macos")]
    {
        // Use the installed Mac font without distributing it; bundled fonts remain fallbacks.
        static SYSTEM_FONT: std::sync::OnceLock<Option<std::sync::Arc<FontData>>> =
            std::sync::OnceLock::new();
        if let Some(font) = SYSTEM_FONT.get_or_init(|| {
            std::fs::read("/System/Library/Fonts/SFNS.ttf")
                .ok()
                .map(|bytes| FontData::from_owned(bytes).into())
        }) {
            fonts
                .font_data
                .insert("system-ui".into(), std::sync::Arc::clone(font));
            fonts
                .families
                .get_mut(&FontFamily::Proportional)
                .unwrap()
                .insert(0, "system-ui".into());
        }
    }
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
            Color32::from_rgb(232, 235, 241)
        } else {
            Color32::from_rgb(39, 45, 55)
        };
        let base = if dark {
            Color32::from_rgb(24, 27, 32)
        } else {
            Color32::WHITE
        };
        let border = if dark {
            Color32::from_rgb(67, 74, 85)
        } else {
            Color32::from_rgb(213, 218, 225)
        };
        let selected = if dark {
            Color32::from_rgb(48, 65, 85)
        } else {
            SELECTED_LIGHT
        };
        let chrome = if dark {
            Color32::from_rgb(32, 36, 43)
        } else {
            Color32::from_rgb(245, 246, 248)
        };
        let card = if dark {
            Color32::from_rgb(40, 45, 54)
        } else {
            Color32::from_rgb(250, 251, 252)
        };
        style.visuals.override_text_color = Some(ink);
        style.visuals.weak_text_color = Some(if dark {
            Color32::from_rgb(179, 187, 199)
        } else {
            Color32::from_rgb(94, 103, 117)
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
        style.visuals.widgets.inactive.bg_stroke = egui::Stroke::new(
            1.,
            if dark {
                Color32::from_rgb(126, 137, 153)
            } else {
                Color32::from_rgb(133, 143, 158)
            },
        );
        style.visuals.widgets.hovered.bg_fill = selected;
        style.visuals.widgets.hovered.weak_bg_fill = selected;
        style.visuals.widgets.hovered.bg_stroke = egui::Stroke::new(1., ink);
        style.visuals.widgets.active.bg_fill = selected;
        style.visuals.widgets.active.bg_stroke = egui::Stroke::new(1., ink);
        for widget in [
            &mut style.visuals.widgets.inactive,
            &mut style.visuals.widgets.hovered,
            &mut style.visuals.widgets.active,
        ] {
            widget.fg_stroke = egui::Stroke::new(1.5, ink);
            widget.corner_radius = egui::CornerRadius::same(4);
        }
        style.spacing.item_spacing = egui::vec2(8., 6.);
        style.spacing.button_padding = egui::vec2(8., 4.);
        style.spacing.interact_size.y = 26.;
        style
            .text_styles
            .insert(TextStyle::Body, FontId::proportional(13.));
        style
            .text_styles
            .insert(TextStyle::Button, FontId::proportional(13.));
        style.text_styles.insert(
            TextStyle::Heading,
            FontId::new(18., FontFamily::Name("semibold".into())),
        );
        style
            .text_styles
            .insert(TextStyle::Small, FontId::proportional(11.5));
        style
            .text_styles
            .insert(TextStyle::Monospace, FontId::monospace(12.));
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
            .font(FontId::new(13., section_family(ui)))
            .color(text(ui)),
    )
}
pub(crate) fn wordmark(ui: &mut egui::Ui, label: &str) -> egui::Response {
    let family = if ui
        .ctx()
        .data(|d| d.get_temp::<bool>(egui::Id::new("tessera-brand-fonts")) == Some(true))
    {
        FontFamily::Name("brand-serif".into())
    } else {
        FontFamily::Proportional
    };
    ui.label(
        egui::RichText::new(label)
            .font(FontId::new(17., family))
            .color(muted(ui)),
    )
}
pub(crate) fn primary<'a>(ui: &egui::Ui, label: &'a str) -> egui::Button<'a> {
    let fill = if ui.visuals().dark_mode {
        Color32::from_rgb(158, 190, 224)
    } else {
        NAVY
    };
    let ink = if ui.visuals().dark_mode {
        Color32::from_rgb(24, 27, 32)
    } else {
        Color32::WHITE
    };
    egui::Button::new(egui::RichText::new(label).color(ink).strong())
        .fill(fill)
        .stroke(egui::Stroke::new(1., fill))
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
/// Selectable buttons keep border geometry constant in idle, hover and focus states.
pub(crate) fn selectable_label<'a>(
    ui: &mut egui::Ui,
    selected: bool,
    text: impl egui::IntoAtoms<'a>,
) -> egui::Response {
    let response = ui.add(egui::Button::selectable(selected, text).stroke(egui::Stroke::NONE));
    focus_ring(ui, &response);
    response
}
pub(crate) fn selectable_value<'a, Value: PartialEq>(
    ui: &mut egui::Ui,
    current: &mut Value,
    choice: Value,
    text: impl egui::IntoAtoms<'a>,
) -> egui::Response {
    let mut response = selectable_label(ui, *current == choice, text);
    if response.clicked() && *current != choice {
        *current = choice;
        response.mark_changed();
    }
    response
}
fn focus_ring(ui: &egui::Ui, response: &egui::Response) {
    if response.has_focus() {
        ui.painter().rect_stroke(
            response.rect.shrink(1.),
            4.,
            egui::Stroke::new(1.5, accent(ui)),
            egui::StrokeKind::Inside,
        );
    }
}
/// Desktop source-list row: standard button keyboard handling, left-aligned text.
pub(crate) fn list_row(
    ui: &mut egui::Ui,
    selected: bool,
    label: &str,
    subtitle: Option<&str>,
) -> egui::Response {
    let height = if subtitle.is_some() { 40. } else { 28. };
    let response = ui.add_sized(
        [ui.available_width(), height],
        egui::Button::new("")
            .selected(selected)
            .stroke(egui::Stroke::NONE)
            .frame_when_inactive(selected),
    );
    let width = (response.rect.width() - 16.).max(0.);
    let label_text = caption(label);
    let galley = egui::WidgetText::from(egui::RichText::new(&label_text).color(text(ui)))
        .into_galley(
            ui,
            Some(egui::TextWrapMode::Truncate),
            width,
            TextStyle::Body,
        );
    let y = if subtitle.is_some() {
        response.rect.top() + 3.
    } else {
        response.rect.center().y - galley.size().y / 2.
    };
    ui.painter()
        .galley(egui::pos2(response.rect.left() + 8., y), galley, text(ui));
    let accessible_label = if let Some(subtitle) = subtitle {
        let subtitle = caption(subtitle);
        let galley =
            egui::WidgetText::from(egui::RichText::new(&subtitle).small().color(muted(ui)))
                .into_galley(
                    ui,
                    Some(egui::TextWrapMode::Truncate),
                    width,
                    TextStyle::Small,
                );
        ui.painter().galley(
            egui::pos2(
                response.rect.left() + 8.,
                response.rect.bottom() - galley.size().y - 3.,
            ),
            galley,
            muted(ui),
        );
        format!("{label_text} · {subtitle}")
    } else {
        label_text
    };
    response.widget_info(|| {
        egui::WidgetInfo::selected(
            egui::WidgetType::SelectableLabel,
            ui.is_enabled(),
            selected,
            &accessible_label,
        )
    });
    focus_ring(ui, &response);
    response.on_hover_text(accessible_label)
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
    fn widget_frame(
        ctx: &egui::Context,
        choice: &mut u8,
        events: Vec<egui::Event>,
    ) -> (Vec<egui::Rect>, egui::Id, bool, bool) {
        let mut rects = Vec::new();
        let mut id = egui::Id::NULL;
        let mut changed = false;
        let mut output = ctx.run_ui(
            egui::RawInput {
                screen_rect: Some(egui::Rect::from_min_size(
                    egui::Pos2::ZERO,
                    egui::vec2(1000., 300.),
                )),
                events,
                ..Default::default()
            },
            |ui| {
                ui.horizontal(|ui| {
                    rects.push(ui.add(primary(ui, "Primary")).rect);
                    rects.push(ui.add(secondary(ui, "Secondary")).rect);
                    rects.push(ui.add(quiet(ui, "Quiet")).rect);
                    rects.push(selectable_value(ui, choice, 0, "First").rect);
                    let response = selectable_value(ui, choice, 1, "Second");
                    rects.push(response.rect);
                    id = response.id;
                    changed = response.changed();
                    rects.push(ui.small_button("Small").rect);
                    rects.push(ui.button("Neighbour").rect);
                });
            },
        );
        let focus = output.shapes.iter().any(|shape| matches!(&shape.shape, egui::Shape::Rect(r) if r.stroke.width == 1.5 && r.rect == rects[4].shrink(1.)));
        output.textures_delta.clear();
        (rects, id, changed, focus)
    }
    #[test]
    fn all_button_styles_keep_geometry_on_hover_press_and_selection_with_visible_focus() {
        use egui::{Event, Modifiers, PointerButton, Pos2};
        for appearance in [egui::Theme::Light, egui::Theme::Dark] {
            let ctx = egui::Context::default();
            install(&ctx);
            ctx.set_theme(appearance);
            let mut choice = 0;
            let off = Pos2::new(990., 290.);
            let mut baseline = Vec::new();
            for _ in 0..3 {
                baseline = widget_frame(&ctx, &mut choice, vec![Event::PointerMoved(off)]).0;
            }
            for rect in &baseline {
                for _ in 0..3 {
                    assert_eq!(
                        widget_frame(&ctx, &mut choice, vec![Event::PointerMoved(rect.center())]).0,
                        baseline
                    );
                }
            }
            let point = baseline[4].center();
            for _ in 0..3 {
                widget_frame(&ctx, &mut choice, vec![Event::PointerMoved(point)]);
            }
            let (rects, _, _, _) = widget_frame(
                &ctx,
                &mut choice,
                vec![Event::PointerButton {
                    pos: point,
                    button: PointerButton::Primary,
                    pressed: true,
                    modifiers: Modifiers::NONE,
                }],
            );
            assert_eq!(rects, baseline);
            let (rects, id, changed, _) = widget_frame(
                &ctx,
                &mut choice,
                vec![Event::PointerButton {
                    pos: point,
                    button: PointerButton::Primary,
                    pressed: false,
                    modifiers: Modifiers::NONE,
                }],
            );
            assert_eq!(rects, baseline);
            assert!(changed);
            assert_eq!(choice, 1);
            ctx.memory_mut(|memory| memory.request_focus(id));
            for _ in 0..3 {
                let (rects, _, _, focus) =
                    widget_frame(&ctx, &mut choice, vec![Event::PointerMoved(off)]);
                assert_eq!(rects, baseline);
                assert!(focus, "selected choices must retain a visible focus ring");
            }
        }
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
