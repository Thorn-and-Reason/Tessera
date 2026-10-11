//! Native presentation is bounded; authoritative captures remain exact in history/report.
use super::{paged_rows, theme, DesktopState};
use crate::analysis::*;
use eframe::egui;
fn name(r: &ReliabilityRun, id: crate::data::ColumnId) -> &str {
    r.columns
        .iter()
        .find(|c| c.id == id)
        .map_or("Unavailable variable", |c| c.name.as_str())
}
fn caption(ui: &mut egui::Ui, text: impl std::fmt::Display) {
    let text = theme::caption_limit(text, 128);
    let r = ui.add(
        egui::Label::new(&text)
            .sense(egui::Sense::focusable_noninteractive())
            .truncate(),
    );
    r.widget_info(|| egui::WidgetInfo::labeled(egui::WidgetType::Label, true, &text));
    r.on_hover_text(&text);
}
fn estimate(ui: &mut egui::Ui, label: &str, e: &Estimate) {
    ui.horizontal(|ui| {
        ui.label(label);
        super::result_tables::contextual_value(ui, label, e, "");
    });
}
fn rows(
    ui: &mut egui::Ui,
    state: &mut DesktopState,
    id: &str,
    title: &str,
    len: usize,
    lines: usize,
    mut row: impl FnMut(&mut egui::Ui, usize),
) {
    let h = lines as f32
        * (ui.text_style_height(&egui::TextStyle::Body) + ui.spacing().item_spacing.y)
        + 8.;
    let pages = paged_rows::show_rows(ui, id, title, h, len, (h * 2., h), |ui, range| {
        for i in range {
            ui.push_id(i, |ui| {
                ui.set_min_height(h);
                row(ui, i);
            });
        }
    });
    pages.record(state);
}
pub(super) fn show(ui: &mut egui::Ui, state: &mut DesktopState, r: &ReliabilityRun) {
    theme::section(ui, "Scale reliability");
    estimate(ui, "Raw alpha", &r.raw_alpha);
    estimate(ui, "Standardized alpha", &r.standardized_alpha);
    ui.label(format!(
        "{} complete physical rows; frequency N = {}",
        r.complete_rows, r.frequency
    ));
    ui.colored_label(
        ui.visuals().warn_fg_color,
        "Alpha does not establish unidimensionality.",
    );
    ui.label("Every deletion uses the same complete cohort.");
    ui.label("Exact values are available on focus or hover. Complete names, reasons and captured definitions: Export report…");
    egui::ScrollArea::horizontal()
        .id_salt("reliability-table-width")
        .show(ui, |ui| {
            ui.set_min_width(1180.);
            ui.horizontal(|ui| {
                for label in [
                    "Item",
                    "Scoring",
                    "Scored mean",
                    "Scored sample SD",
                    "Corrected item-total correlation",
                    "Raw alpha if removed",
                    "Standardized alpha if removed",
                ] {
                    ui.add_sized(
                        [160., 36.],
                        egui::Label::new(egui::RichText::new(label).strong()),
                    );
                }
            });
            rows(
                ui,
                state,
                "reliability-results",
                "item results",
                r.items.len(),
                1,
                |ui, i| {
                    let item = &r.items[i];
                    ui.horizontal(|ui| {
                        ui.allocate_ui(egui::vec2(160., 30.), |ui| {
                            caption(
                                ui,
                                format_args!("Item {} · {}", i + 1, name(r, item.column)),
                            )
                        });
                        ui.allocate_ui(egui::vec2(160., 30.), |ui| {
                            if let Some(v) =
                                r.request.reversals.iter().find(|v| v.column == item.column)
                            {
                                caption(ui, format_args!("Reversed [{}, {}]", v.lower, v.upper));
                            } else {
                                ui.label("Original item scoring");
                            }
                        });
                        for (label, e) in [
                            ("Scored mean", &item.mean),
                            ("Scored sample SD", &item.sample_sd),
                            (
                                "Corrected item-total correlation",
                                &item.corrected_item_total,
                            ),
                            ("Raw alpha if removed", &item.raw_alpha_if_removed),
                            (
                                "Standardized alpha if removed",
                                &item.standardized_alpha_if_removed,
                            ),
                        ] {
                            ui.allocate_ui(egui::vec2(160., 30.), |ui| {
                                super::result_tables::contextual_value(
                                    ui,
                                    label,
                                    e,
                                    &format!("Item {} · ", i + 1),
                                )
                            });
                        }
                    });
                },
            );
        });
}

pub(super) fn settings(ui: &mut egui::Ui, state: &mut DesktopState, r: &ReliabilityRun) {
    ui.label("Captured settings · saved labels and bounds");
    ui.label("Export report… contains complete captured definitions and exact values.");
    ui.label("Complete cases across all selected items · stable centred calculations");
    caption(
        ui,
        r.request.frequency_weight.map_or_else(
            || "Unweighted observations".into(),
            |id| {
                theme::caption_limit(
                    format_args!(
                        "Frequency weight · Column {} · {} · literal replication",
                        id.0,
                        name(r, id)
                    ),
                    128,
                )
            },
        ),
    );
    rows(
        ui,
        state,
        "reliability-captured-items",
        "captured items",
        r.request.items.len(),
        2,
        |ui, i| {
            let id = r.request.items[i];
            caption(
                ui,
                format_args!("Item {} · Column {} · {}", i + 1, id.0, name(r, id)),
            );
            if let Some(v) = r.request.reversals.iter().find(|v| v.column == id) {
                caption(
                    ui,
                    format_args!("Reverse bounds [{}, {}]", v.lower, v.upper),
                );
            } else {
                ui.label("Original scoring");
            }
        },
    );
    rows(
        ui,
        state,
        "reliability-captured-filter",
        "captured filter conditions",
        r.request.filter.predicates.len(),
        1,
        |ui, i| {
            let p = &r.request.filter.predicates[i];
            let n = theme::caption_limit(name(r, p.column()), 32);
            match p {
                RowPredicate::Number {
                    comparison, value, ..
                } => caption(
                    ui,
                    format_args!("Condition {} · {n} · {comparison:?} {value}", i + 1),
                ),
                RowPredicate::Text { equal, value, .. } => caption(
                    ui,
                    format_args!(
                        "Condition {} · {n} · {} {value:?}",
                        i + 1,
                        if *equal { "equals" } else { "does not equal" }
                    ),
                ),
                RowPredicate::Missing { missing, .. } => caption(
                    ui,
                    format_args!(
                        "Condition {} · {n} · is {}",
                        i + 1,
                        if *missing { "missing" } else { "present" }
                    ),
                ),
            }
        },
    );
    let d = &r.diagnostics;
    caption(ui,format_args!("Excluded physical rows: {} filtered; {} missing weights; {} zero weights; {} incomplete cases",d.filtered_rows,d.missing_weight_rows,d.zero_weight_rows,d.complete_case_rows));
    let total = r.columns.iter().map(|c| 2 + c.value_labels.len()).sum();
    rows(
        ui,
        state,
        "reliability-captured-metadata",
        "captured definitions",
        total,
        1,
        |ui, index| {
            let mut offset = 0;
            for c in &r.columns {
                let end = offset + 2 + c.value_labels.len();
                if index < end {
                    match index - offset {
                        0 => caption(
                            ui,
                            format_args!(
                                "Column {} · {} · {:?} · {}",
                                c.id.0, c.name, c.kind, c.description
                            ),
                        ),
                        1 => caption(
                            ui,
                            format_args!(
                                "Column {} · Missing definitions: {:?}",
                                c.id.0, c.missing_rules
                            ),
                        ),
                        i => {
                            let l = &c.value_labels[i - 2];
                            caption(
                                ui,
                                format_args!(
                                    "Column {} · Value {:?}: {}",
                                    c.id.0, l.value, l.label
                                ),
                            );
                        }
                    }
                    break;
                }
                offset = end;
            }
        },
    );
}
#[cfg(test)]
mod tests;
