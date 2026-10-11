//! Concrete native chart primitives shared by captured result renderers.
use super::{procedure_forms::record, theme, DesktopState};
use crate::{charts::ChartScale, error::AppResult};
use eframe::egui::{self, Align2, FontId, Pos2, Rect, Sense, Stroke};

pub(super) fn paint_text(ui: &egui::Ui, pos: Pos2, align: Align2, text: impl ToString) {
    ui.painter().text(
        pos,
        align,
        text.to_string(),
        FontId::proportional(11.),
        theme::text(ui),
    );
}
pub(super) fn canvas(
    ui: &mut egui::Ui,
    state: &mut DesktopState,
    title: &str,
    left: f32,
    right: f32,
) -> Rect {
    canvas_height(ui, state, title, left, right, 232.)
}
pub(super) fn canvas_height(
    ui: &mut egui::Ui,
    state: &mut DesktopState,
    title: &str,
    left: f32,
    right: f32,
    height: f32,
) -> Rect {
    ui.strong(theme::caption(title));
    let (rect, response) = ui.allocate_exact_size(
        egui::vec2(
            ui.available_width().clamp(left + right + 120., 800.),
            height,
        ),
        Sense::click(),
    );
    response.widget_info(|| {
        egui::WidgetInfo::labeled(
            egui::WidgetType::Other,
            true,
            theme::caption(format_args!("{title} · Exact values in the table")),
        )
    });
    record(state, "Chart", &response);
    ui.painter().rect_filled(rect, 0., theme::surface(ui));
    Rect::from_min_max(
        rect.min + egui::vec2(left, 12.),
        rect.max - egui::vec2(right, 54.),
    )
}
pub(super) fn x(rect: Rect, scale: &ChartScale, value: f64) -> AppResult<f32> {
    Ok(rect.left() + scale.position(value)? as f32 * rect.width())
}
pub(super) fn y(rect: Rect, scale: &ChartScale, value: f64) -> AppResult<f32> {
    Ok(rect.bottom() - scale.position(value)? as f32 * rect.height())
}
fn axis_name(ui: &egui::Ui, text: String, origin: Pos2, width: f32, rows: usize, center: bool) {
    let mut job =
        egui::text::LayoutJob::simple(text, FontId::proportional(11.), theme::text(ui), width);
    job.wrap.max_rows = rows;
    job.wrap.break_anywhere = true;
    let galley = ui.painter().layout_job(job);
    let offset = if center {
        (width - galley.size().x) * 0.5
    } else {
        0.
    };
    ui.painter()
        .galley(origin + egui::vec2(offset, 0.), galley, theme::text(ui));
}
pub(super) fn axes(
    ui: &mut egui::Ui,
    rect: Rect,
    xscale: &ChartScale,
    yscale: Option<&ChartScale>,
    xname: &str,
    yname: &str,
) -> AppResult<()> {
    let stroke = Stroke::new(1., theme::text(ui));
    ui.painter()
        .line_segment([rect.left_bottom(), rect.right_bottom()], stroke);
    let name_left = rect.left().max(ui.clip_rect().left());
    let name_right = rect.right().min(ui.clip_rect().right());
    let name_width = (name_right - name_left).max(1.);
    axis_name(
        ui,
        theme::caption_limit(xname, 64),
        Pos2::new(name_left, rect.bottom() + 24.),
        name_width,
        2,
        true,
    );
    for tick in xscale.axis_ticks() {
        let (v, p) = tick?;
        let pos = Pos2::new(rect.left() + p as f32 * rect.width(), rect.bottom());
        if p > 0. && p < 1. {
            ui.painter().line_segment(
                [Pos2::new(pos.x, rect.top()), pos],
                Stroke::new(1., theme::line(ui)),
            );
        }
        ui.painter()
            .line_segment([pos, pos + egui::vec2(0., 5.)], stroke);
        paint_text(
            ui,
            pos + egui::vec2(0., 16.),
            Align2::CENTER_CENTER,
            xscale.axis_label(v),
        );
    }
    if let Some(scale) = yscale {
        ui.painter()
            .line_segment([rect.left_bottom(), rect.left_top()], stroke);
        axis_name(
            ui,
            theme::caption_limit(yname, 32),
            Pos2::new(name_left, rect.top() - 12.),
            name_width,
            1,
            false,
        );
        for tick in scale.axis_ticks() {
            let (v, p) = tick?;
            let pos = Pos2::new(rect.left(), rect.bottom() - p as f32 * rect.height());
            if p > 0. && p < 1. {
                ui.painter().line_segment(
                    [pos, Pos2::new(rect.right(), pos.y)],
                    Stroke::new(1., theme::line(ui)),
                );
            }
            ui.painter()
                .line_segment([pos, pos - egui::vec2(5., 0.)], stroke);
            paint_text(
                ui,
                pos - egui::vec2(8., 0.),
                Align2::RIGHT_CENTER,
                scale.axis_label(v),
            );
        }
    }
    Ok(())
}
pub(super) fn unavailable(ui: &mut egui::Ui, result: AppResult<()>) {
    if result.is_err() {
        ui.label("Drawing unavailable · exact values remain in the table");
    }
}
pub(super) fn hover(ui: &egui::Ui, point: Pos2, text: impl FnOnce() -> String) {
    if ui.rect_contains_pointer(Rect::from_center_size(point, egui::vec2(12., 12.))) {
        egui::Tooltip::always_open(
            ui.ctx().clone(),
            ui.layer_id(),
            egui::Id::new("chart-point"),
            egui::PopupAnchor::Pointer,
        )
        .show(|ui| {
            ui.label(text());
        });
    }
}
