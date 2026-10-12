//! Sparse lists keep keyboard paging in the existing egui scroll state.
use super::{procedure_forms::record, DesktopState};
use eframe::egui;
use std::hash::Hash;

#[derive(Default)]
pub(super) struct PageControls(Vec<(String, egui::Response)>);
impl PageControls {
    pub(super) fn append(&mut self, other: Self) {
        self.0.extend(other.0);
    }
    pub(super) fn record(self, state: &mut DesktopState) {
        for (label, response) in self.0 {
            record(state, &label, &response);
        }
    }
}

pub(super) fn show_rows(
    ui: &mut egui::Ui,
    id: impl Hash + std::fmt::Debug,
    title: &str,
    row_height: f32,
    rows: usize,
    bounds: (f32, f32),
    add: impl FnOnce(&mut egui::Ui, std::ops::Range<usize>),
) -> PageControls {
    if rows == 0 {
        return PageControls::default();
    }
    let pending_id = ui.make_persistent_id((&id, "keyboard-page-offset"));
    let pending = ui
        .ctx()
        .data_mut(|data| data.remove_temp::<f32>(pending_id));
    let mut area = egui::ScrollArea::vertical()
        .id_salt(&id)
        .max_height(bounds.0)
        .min_scrolled_height(bounds.1.max(row_height + ui.spacing().item_spacing.y));
    if let Some(offset) = pending {
        area = area.vertical_scroll_offset(offset);
    }
    let output = area.show_viewport(ui, |ui, viewport| {
        visible_rows(ui, viewport, row_height, rows, add);
    });
    page_controls(
        ui,
        id,
        title,
        Paging {
            offset: output.state.offset.y,
            height: output.inner_rect.height(),
            content: output.content_size.y,
            row_height,
            pending_id,
        },
    )
}
fn visible_rows(
    ui: &mut egui::Ui,
    viewport: egui::Rect,
    row_height: f32,
    rows: usize,
    add: impl FnOnce(&mut egui::Ui, std::ops::Range<usize>),
) {
    let spacing = ui.spacing().item_spacing.y;
    let stride = row_height + spacing;
    ui.set_height((stride * rows as f32 - spacing).max(0.));
    let mut first = (viewport.min.y / stride).floor() as usize;
    let mut end = (viewport.max.y / stride).ceil() as usize;
    // Same shrink backfill as egui show_rows, without its hidden overscan row.
    if end > rows {
        let span = end.saturating_sub(first);
        end = rows;
        first = rows.saturating_sub(span);
    }
    let top = ui.max_rect().top();
    let rect = egui::Rect::from_x_y_ranges(
        ui.max_rect().x_range(),
        top + first as f32 * stride..=top + end as f32 * stride,
    );
    ui.scope_builder(egui::UiBuilder::new().max_rect(rect), |ui| {
        ui.skip_ahead_auto_ids(first);
        add(ui, first..end);
    });
}

struct Paging {
    offset: f32,
    height: f32,
    content: f32,
    row_height: f32,
    pending_id: egui::Id,
}
fn page_controls(
    ui: &mut egui::Ui,
    id: impl Hash + std::fmt::Debug,
    title: &str,
    paging: Paging,
) -> PageControls {
    let max_offset = (paging.content - paging.height).max(0.);
    let mut controls = PageControls::default();
    if max_offset > 1. {
        let row_stride = paging.row_height + ui.spacing().item_spacing.y;
        let page = (paging.height / row_stride).floor().max(1.) * row_stride;
        ui.push_id((&id, "keyboard-paging"), |ui| {
            ui.horizontal(|ui| {
                for (direction, enabled, target) in [
                    (
                        "Previous",
                        paging.offset > 1.,
                        (paging.offset - page).max(0.),
                    ),
                    (
                        "Next",
                        paging.offset < max_offset - 1.,
                        (paging.offset + page).min(max_offset),
                    ),
                ] {
                    let (label, response) = page_button(ui, direction, title, enabled);
                    if response.clicked() {
                        ui.ctx()
                            .data_mut(|data| data.insert_temp(paging.pending_id, target));
                    }
                    controls.0.push((label, response));
                }
            });
        });
    }
    controls
}

fn page_button(
    ui: &mut egui::Ui,
    direction: &str,
    title: &str,
    enabled: bool,
) -> (String, egui::Response) {
    let label = format!("{direction} {title} page");
    let response = ui.add_enabled(
        enabled,
        egui::Button::new(format!("{direction} page")).small(),
    );
    response.widget_info(|| egui::WidgetInfo::labeled(egui::WidgetType::Button, enabled, &label));
    (label, response)
}

#[cfg(test)]
mod tests;
