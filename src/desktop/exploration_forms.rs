//! New-family roles reuse existing pickers and the checked draft boundary.
use super::{draft::AnalysisDraft, procedure_forms::scalar, theme, DesktopState};
use crate::{analysis::HistogramBins, data::Dataset};
use eframe::egui;
pub(super) fn show(
    ui: &mut egui::Ui,
    state: &mut DesktopState,
    data: &Dataset,
    draft: &mut AnalysisDraft,
) {
    match draft {
        AnalysisDraft::Frequencies(r) => {
            theme::section(ui, "Variables");
            if r.columns.is_empty() {
                ui.small("Select at least one variable.");
            }
            let mut remove = None;
            egui::ScrollArea::vertical()
                .id_salt("frequency-roles")
                .max_height(180.)
                .show_rows(ui, 28., r.columns.len(), |ui, range| {
                    for i in range {
                        ui.horizontal(|ui| {
                            let name = data.column(r.columns[i]).map_or_else(
                                |_| format!("Unavailable variable {}", r.columns[i].0),
                                |c| theme::caption_limit(c.name(), 128),
                            );
                            ui.add_sized(
                                egui::vec2((ui.available_width() - 70.).max(0.), 28.),
                                egui::Label::new(name).truncate(),
                            );
                            let response = ui.small_button("Remove");
                            response.widget_info(|| {
                                egui::WidgetInfo::labeled(
                                    egui::WidgetType::Button,
                                    true,
                                    format!(
                                        "Remove frequency variable {}",
                                        data.column(r.columns[i]).map_or_else(
                                            |_| r.columns[i].0.to_string(),
                                            |c| theme::caption_limit(c.name(), 128)
                                        )
                                    ),
                                )
                            });
                            if response.clicked() {
                                remove = Some(i);
                            }
                        });
                    }
                });
            if let Some(i) = remove {
                r.columns.remove(i);
            }
            let heap = crate::analysis::AnalysisRequestRef::Frequencies(r)
                .heap_bytes()
                .unwrap_or(usize::MAX);
            super::procedure_forms::select_all_columns(
                ui,
                state,
                &mut r.columns,
                heap,
                "Select all",
                data.columns().iter().map(|c| c.id()),
            );
            let mut added = None;
            scalar(
                ui,
                state,
                data,
                "Add frequency variable",
                &mut added,
                false,
                None,
            );
            if let Some(id) = added {
                if !r.columns.contains(&id) {
                    let heap = crate::analysis::AnalysisRequestRef::Frequencies(r)
                        .heap_bytes()
                        .unwrap_or(usize::MAX);
                    super::procedure_forms::append_column(state, &mut r.columns, heap, id);
                }
            }
        }
        AnalysisDraft::Histogram { variable, .. } => {
            scalar(ui, state, data, "Variable", variable, true, None);
        }
        AnalysisDraft::Scatter { x, y, .. } => {
            scalar(ui, state, data, "X variable", x, true, *y);
            scalar(ui, state, data, "Y variable", y, true, *x);
        }
        AnalysisDraft::Boxplot { outcome, group, .. } => {
            scalar(ui, state, data, "Outcome", outcome, true, *group);
            scalar(
                ui,
                state,
                data,
                "Grouping variable (optional)",
                group,
                false,
                *outcome,
            );
        }
        _ => {}
    }
}
pub(super) fn bins(ui: &mut egui::Ui, state: &mut DesktopState, bins: &mut HistogramBins) {
    ui.label("Bins");
    let mut automatic = matches!(bins, HistogramBins::Automatic);
    let response = ui.checkbox(&mut automatic, "Automatic (Sturges)");
    super::procedure_forms::record(state, "Automatic (Sturges)", &response);
    if automatic {
        *bins = HistogramBins::Automatic;
    } else {
        let mut count = if let HistogramBins::Count(n) = bins {
            *n
        } else {
            10
        };
        let response = ui.add(
            egui::DragValue::new(&mut count)
                .range(1..=128)
                .clamp_existing_to_range(false),
        );
        response.widget_info(|| {
            egui::WidgetInfo::labeled(egui::WidgetType::DragValue, true, "Bin count")
        });
        super::procedure_forms::record(state, "Bin count", &response);
        *bins = HistogramBins::Count(count);
    }
}
