use super::{
    button, start,
    state::{MAX_DRAFT_BYTES, TEXT_LIMIT},
    theme, DesktopState, Dialog, UiAction,
};
use crate::{
    analysis::{MissingPolicy, NumericComparison, RowPredicate, Statistic},
    data::ColumnKind,
    jobs::{JobOperation, JobRunner},
};
use eframe::egui;
use std::sync::Arc;
pub const STATISTICS: [(Statistic, &str); 8] = [
    (Statistic::Mean, "Mean"),
    (Statistic::SampleStdDev, "Sample SD"),
    (Statistic::Variance, "Variance"),
    (Statistic::Minimum, "Minimum"),
    (Statistic::Maximum, "Maximum"),
    (Statistic::Sum, "Sum"),
    (Statistic::Range, "Range"),
    (Statistic::StdErrorMean, "SE of mean"),
];
pub fn statistic_label(statistic: Statistic) -> &'static str {
    STATISTICS
        .iter()
        .find(|(s, _)| *s == statistic)
        .map_or("", |(_, s)| s)
}
pub fn show(ui: &mut egui::Ui, state: &mut DesktopState, jobs: &mut JobRunner) {
    ui.horizontal_wrapped(|ui| {
        ui.heading("Analysis");
        super::experience::mode_switch(ui, state);
    });
    ui.separator();
    settings(ui, state, jobs);
}

pub fn settings(ui: &mut egui::Ui, state: &mut DesktopState, jobs: &mut JobRunner) {
    egui::Panel::bottom("analysis-run-footer").show(ui, |ui| {
        let label = if state.draft.procedure() == super::draft::Procedure::Describe {
            "Run descriptives"
        } else {
            "Run analysis"
        };
        run_button(ui, state, jobs, label);
    });
    if state.project().dataset().columns().is_empty() {
        ui.label("Import CSV, TSV or Excel (.xlsx), or add a variable to start your dataset.");
        ui.horizontal(|ui| {
            if button(ui, state, "Import data…").clicked() {
                super::dialogs::import(state);
            }
            if button(ui, state, "Add variable").clicked() {
                state.act(UiAction::Navigate(super::Workspace::Data));
                state.dialog = Some(Dialog::AddVariable);
            }
        });
    }
    let split = ui.available_width() >= 850.;
    if split {
        egui::Panel::left("analysis-catalog-panel")
            .exact_size(248.)
            .show(ui, |ui| super::procedure_forms::chooser(ui, state, true));
    } else {
        super::procedure_forms::chooser(ui, state, false);
    }
    egui::Frame::new()
        .inner_margin(egui::Margin {
            left: if split { 12 } else { 0 },
            ..Default::default()
        })
        .show(ui, |ui| form(ui, state, jobs));
}

fn form(ui: &mut egui::Ui, state: &mut DesktopState, jobs: &mut JobRunner) {
    let project = Arc::clone(state.project());
    let columns = project.dataset().columns();
    ui.strong(state.draft.procedure().label());
    ui.label(super::experience::description(state.draft.procedure()));
    ui.separator();
    let super::draft::AnalysisDraft::Describe(_) = &state.draft else {
        super::procedure_forms::settings(ui, state, jobs);
        return;
    };
    let mut draft = match state.checked_editable_draft() {
        Ok(super::draft::AnalysisDraft::Describe(draft)) => draft,
        Ok(_) => unreachable!(),
        Err(error) => {
            state.fail(error);
            return;
        }
    };
    egui::ScrollArea::vertical()
        .id_salt("analysis-settings-scroll")
        .max_height(ui.available_height().max(0.))
        .show(ui, |ui| {
            theme::section(ui, "Variables");
            if draft.columns.is_empty() {
                ui.small("Select at least one numeric variable.");
            }
            ui.small(format!("Selected ({})", draft.columns.len()));
            let mut remove = None;
            super::paged_rows::show_rows(
                ui,
                "describe-selected",
                "selected describe variables",
                28.,
                draft.columns.len(),
                (168., 56.),
                |ui, range| {
                    for index in range {
                        let id = draft.columns[index];
                        ui.push_id(("describe-selected", id), |ui| {
                            ui.horizontal(|ui| {
                                let name = project.dataset().column(id).map_or_else(
                                    |_| format!("Unavailable variable {}", id.0),
                                    |c| theme::caption_limit(c.name(), 128),
                                );
                                ui.add_sized(
                                    egui::vec2((ui.available_width() - 70.).max(0.), 28.),
                                    egui::Label::new(&name).truncate(),
                                );
                                let response = ui.small_button("Remove");
                                response.widget_info(|| {
                                    egui::WidgetInfo::labeled(
                                        egui::WidgetType::Button,
                                        true,
                                        format!("Remove variable {} · {name}", id.0),
                                    )
                                });
                                super::procedure_forms::record(
                                    state,
                                    &format!("Remove variable {}", id.0),
                                    &response,
                                );
                                if response.clicked() {
                                    remove = Some(index);
                                }
                            })
                        });
                    }
                },
            )
            .record(state);
            if let Some(index) = remove {
                draft.columns.remove(index);
            }
            {
                ui.small("Available numeric variables");
                let response = ui.add(
                    egui::TextEdit::singleline(&mut state.search)
                        .id_salt("describe-variable-search")
                        .hint_text("Search variables")
                        .char_limit(TEXT_LIMIT),
                );
                super::procedure_forms::record(state, "Add variables…", &response);
                response.widget_info(|| {
                    egui::WidgetInfo::labeled(egui::WidgetType::TextEdit, true, "Search variables")
                });
                let query = match theme::NameQuery::new(&state.search) {
                    Ok(query) => query,
                    Err(error) => {
                        state.fail(error);
                        return;
                    }
                };
                let candidates = || {
                    columns.iter().filter(|c| {
                        c.kind() == ColumnKind::Number
                            && !draft.columns.contains(&c.id())
                            && query.matches(c.name())
                    })
                };
                let mut selected = None;
                super::paged_rows::show_rows(
                    ui,
                    "describe-picker",
                    "describe candidates",
                    28.,
                    candidates().count(),
                    (168., 56.),
                    |ui, range| {
                        for column in candidates().skip(range.start).take(range.len()) {
                            let response = ui
                                .push_id(("picker-column", column.id()), |ui| {
                                    theme::list_row(
                                        ui,
                                        false,
                                        &theme::caption_limit(column.name(), 128),
                                        None,
                                    )
                                })
                                .inner;
                            super::procedure_forms::record(
                                state,
                                &format!("Variable {}", column.id().0),
                                &response,
                            );
                            if response.clicked() {
                                selected = Some(column.id());
                            }
                        }
                    },
                )
                .record(state);
                if let Some(id) = selected {
                    let heap = crate::analysis::AnalysisRequestRef::Describe(&draft)
                        .heap_bytes()
                        .unwrap_or(usize::MAX);
                    super::procedure_forms::append_column(state, &mut draft.columns, heap, id);
                }
            }
            ui.separator();
            super::experience::summary(
                ui,
                state,
                super::experience::describe_summary(&draft),
                draft.frequency_weight,
            );
            theme::section(ui, "Statistics");
            egui::Grid::new("describe-statistics")
                .num_columns(2)
                .spacing(egui::vec2(18., 4.))
                .show(ui, |ui| {
                    for (index, (statistic, label)) in STATISTICS.into_iter().enumerate() {
                        let mut selected = draft.statistics.contains(&statistic);
                        if ui.checkbox(&mut selected, label).changed() {
                            if selected {
                                draft.statistics.push(statistic);
                            } else {
                                draft.statistics.retain(|s| *s != statistic);
                            }
                        }
                        if index % 2 == 1 {
                            ui.end_row();
                        }
                    }
                });
            let header = super::experience::detailed_settings(
                ui,
                state.analysis_mode(),
                egui::Id::new("describe-details"),
                |ui| {
                    ui.label("Missing values");
                    let response = egui::ComboBox::from_id_salt("missing-policy")
                        .selected_text(match draft.missing_policy {
                            MissingPolicy::PerVariable => "Per variable",
                            MissingPolicy::CompleteCase => "Complete cases",
                        })
                        .show_ui(ui, |ui| {
                            ui.selectable_value(
                                &mut draft.missing_policy,
                                MissingPolicy::PerVariable,
                                "Per variable",
                            );
                            ui.selectable_value(
                                &mut draft.missing_policy,
                                MissingPolicy::CompleteCase,
                                "Complete cases",
                            );
                        })
                        .response;
                    response.widget_info(|| {
                        egui::WidgetInfo::labeled(
                            egui::WidgetType::ComboBox,
                            true,
                            "Missing values",
                        )
                    });
                    common_settings(ui, &project, &mut draft.filter, &mut draft.frequency_weight)
                        .record(state);
                },
            );
            if let Some(response) = header {
                super::procedure_forms::record(state, "More settings", &response);
            }
        });
    if !state
        .draft
        .matches_saved(crate::analysis::AnalysisRequestRef::Describe(&draft))
        && crate::analysis::AnalysisRequestRef::Describe(&draft)
            .heap_bytes()
            .is_ok_and(|n| n <= MAX_DRAFT_BYTES)
    {
        state.act(UiAction::SetDraft(draft.into()));
    }
}
pub(super) fn run_button(
    ui: &mut egui::Ui,
    state: &mut DesktopState,
    jobs: &mut JobRunner,
    label: &str,
) {
    ui.separator();
    let requirement = state.draft_requirement();
    let response = ui
        .horizontal(|ui| {
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                let response = ui.add_enabled(
                    !state.active && requirement.is_none(),
                    theme::primary(ui, label).min_size(egui::vec2(140., 28.)),
                );
                let message = requirement.unwrap_or(if state.active {
                    "Analysis running…"
                } else {
                    "Ready to run"
                });
                ui.add(
                    egui::Label::new(egui::RichText::new(message).color(theme::muted(ui)))
                        .truncate(),
                )
                .on_hover_text(message);
                response
            })
            .inner
        })
        .inner;
    super::procedure_forms::record(state, label, &response);
    if response.clicked() {
        match state.checked_draft_request() {
            Ok(request) => start(
                state,
                jobs,
                JobOperation::Analyze {
                    project: Arc::clone(state.project()),
                    request,
                },
            ),
            Err(error) => state.fail(error),
        }
    }
}
pub(super) fn common_settings(
    ui: &mut egui::Ui,
    project: &crate::project::Project,
    filter: &mut crate::analysis::RowFilter,
    frequency_weight: &mut Option<crate::data::ColumnId>,
) -> super::paged_rows::PageControls {
    common_settings_with_weight(ui, project, filter, frequency_weight, true)
}
pub(super) fn common_settings_with_weight(
    ui: &mut egui::Ui,
    project: &crate::project::Project,
    filter: &mut crate::analysis::RowFilter,
    frequency_weight: &mut Option<crate::data::ColumnId>,
    weight_enabled: bool,
) -> super::paged_rows::PageControls {
    let columns = project.dataset().columns();
    let mut popup_pages = super::paged_rows::PageControls::default();
    ui.add_enabled_ui(weight_enabled, |ui| {
        ui.label("Frequency weight");
        let response = egui::ComboBox::from_id_salt("frequency-weight")
            .selected_text(
                (*frequency_weight)
                    .and_then(|id| project.dataset().column(id).ok())
                    .map_or_else(|| "None".into(), |c| theme::caption(c.name())),
            )
            .show_ui(ui, |ui| {
                ui.selectable_value(frequency_weight, None, "None");
                let count = columns
                    .iter()
                    .filter(|c| c.kind() == ColumnKind::Number)
                    .count();
                let height = ui.spacing().interact_size.y;
                popup_pages = super::paged_rows::show_rows(
                    ui,
                    "frequency-weight-columns",
                    "frequency weight columns",
                    height,
                    count,
                    (180., 60.),
                    |ui, range| {
                        for column in columns
                            .iter()
                            .filter(|c| c.kind() == ColumnKind::Number)
                            .skip(range.start)
                            .take(range.len())
                        {
                            ui.push_id(column.id(), |ui| {
                                ui.selectable_value(
                                    frequency_weight,
                                    Some(column.id()),
                                    theme::caption_limit(column.name(), 128),
                                );
                            });
                        }
                    },
                );
            })
            .response;
        response.widget_info(|| {
            egui::WidgetInfo::labeled(
                egui::WidgetType::ComboBox,
                weight_enabled,
                "Frequency weight",
            )
        });
    });
    ui.label("Filters · all conditions");
    let mut remove = None;
    let row_height = ui.text_style_height(&egui::TextStyle::Body) * 3.
        + ui.spacing().interact_size.y * 3.
        + ui.spacing().item_spacing.y * 7.;
    let mut pages = super::paged_rows::show_rows(
        ui,
        "filter-conditions",
        "filter conditions",
        row_height,
        filter.predicates.len(),
        (280., 160.),
        |ui, range| {
            for index in range {
                let predicate = &mut filter.predicates[index];
                ui.push_id(("filter", index), |ui| {
                    ui.set_min_height(row_height);
                    ui.style_mut().wrap_mode = Some(egui::TextWrapMode::Truncate);
                    let id = predicate.column();
                    ui.strong(project.dataset().column(id).map_or_else(
                        |_| "Unavailable variable".into(),
                        |c| theme::caption(c.name()),
                    ));
                    edit_filter(
                        ui,
                        predicate,
                        project
                            .dataset()
                            .column(id)
                            .map_or(ColumnKind::Number, |c| c.kind()),
                    );
                    let response = ui.small_button("Remove condition");
                    response.widget_info(|| {
                        egui::WidgetInfo::labeled(
                            egui::WidgetType::Button,
                            true,
                            format!("Remove filter condition {}", index + 1),
                        )
                    });
                    if response.clicked() {
                        remove = Some(index);
                    }
                    ui.separator();
                });
            }
        },
    );
    if let Some(index) = remove {
        filter.predicates.remove(index);
    }
    if filter.predicates.len() < 256 {
        ui.menu_button("Add condition", |ui| {
            let height = ui.spacing().interact_size.y.max(
                ui.text_style_height(&egui::TextStyle::Body) + 2. * ui.spacing().button_padding.y,
            );
            let choices = super::paged_rows::show_rows(
                ui,
                "filter-column-choices",
                "filter columns",
                height,
                columns.len(),
                (180., 60.),
                |ui, range| {
                    for column in &columns[range] {
                        if ui
                            .push_id(column.id(), |ui| {
                                ui.add(
                                    egui::Button::new(theme::caption_limit(column.name(), 128))
                                        .truncate(),
                                )
                            })
                            .inner
                            .clicked()
                        {
                            filter.predicates.push(match column.kind() {
                                ColumnKind::Number => RowPredicate::Number {
                                    column: column.id(),
                                    comparison: NumericComparison::Equal,
                                    value: 0.,
                                },
                                ColumnKind::Text => RowPredicate::Text {
                                    column: column.id(),
                                    value: String::new(),
                                    equal: true,
                                },
                            });
                            ui.close();
                        }
                    }
                },
            );
            pages.append(choices);
        });
    }
    pages.append(popup_pages);
    pages
}

fn edit_filter(ui: &mut egui::Ui, predicate: &mut RowPredicate, kind: ColumnKind) {
    let column = predicate.column();
    let mut operator = match predicate {
        RowPredicate::Number { comparison, .. } => match comparison {
            NumericComparison::Equal => 0,
            NumericComparison::NotEqual => 1,
            NumericComparison::Less => 2,
            NumericComparison::LessOrEqual => 3,
            NumericComparison::Greater => 4,
            NumericComparison::GreaterOrEqual => 5,
        },
        RowPredicate::Text { equal, .. } => {
            if *equal {
                0
            } else {
                1
            }
        }
        RowPredicate::Missing { missing, .. } => {
            if *missing {
                6
            } else {
                7
            }
        }
    };
    let previous = operator;
    let response = egui::ComboBox::from_id_salt("operator")
        .selected_text(
            [
                "Equals",
                "Does not equal",
                "Less than",
                "At most",
                "Greater than",
                "At least",
                "Is missing",
                "Is present",
            ][operator],
        )
        .show_ui(ui, |ui| {
            for (index, label) in [
                "Equals",
                "Does not equal",
                "Less than",
                "At most",
                "Greater than",
                "At least",
                "Is missing",
                "Is present",
            ]
            .iter()
            .enumerate()
            {
                if kind == ColumnKind::Number || !(2..6).contains(&index) {
                    ui.selectable_value(&mut operator, index, *label);
                }
            }
        })
        .response;
    response.widget_info(|| {
        egui::WidgetInfo::labeled(egui::WidgetType::ComboBox, true, "Filter comparison")
    });
    if operator != previous {
        *predicate = match operator {
            6 | 7 => RowPredicate::Missing {
                column,
                missing: operator == 6,
            },
            _ if kind == ColumnKind::Text => RowPredicate::Text {
                column,
                value: match predicate {
                    RowPredicate::Text { value, .. } => value.clone(),
                    _ => String::new(),
                },
                equal: operator == 0,
            },
            _ => RowPredicate::Number {
                column,
                comparison: [
                    NumericComparison::Equal,
                    NumericComparison::NotEqual,
                    NumericComparison::Less,
                    NumericComparison::LessOrEqual,
                    NumericComparison::Greater,
                    NumericComparison::GreaterOrEqual,
                ][operator],
                value: match predicate {
                    RowPredicate::Number { value, .. } => *value,
                    _ => 0.0,
                },
            },
        };
    }
    match predicate {
        RowPredicate::Number { value, .. } => {
            let response =
                ui.add(egui::DragValue::new(value).custom_formatter(|value, _| value.to_string()));
            response.widget_info(|| {
                egui::WidgetInfo::labeled(egui::WidgetType::DragValue, true, "Filter value")
            });
        }
        RowPredicate::Text { value, .. } => {
            if value.chars().count() > 256 {
                // Captured predicates are exact identities, not editable display captions.
                ui.label(theme::caption(format_args!("Saved exact value: {value:?}")));
                if ui.small_button("Replace saved filter value").clicked() {
                    *value = String::new();
                }
            } else {
                let response = ui.add(
                    egui::TextEdit::singleline(value)
                        .id_salt("filter-value")
                        .char_limit(256),
                );
                response.widget_info(|| {
                    egui::WidgetInfo::labeled(egui::WidgetType::TextEdit, true, "Filter value")
                });
            }
        }
        _ => {}
    }
}

#[cfg(test)]
mod filter_tests;
#[cfg(test)]
mod selection_tests;
