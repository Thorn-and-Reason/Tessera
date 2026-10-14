use super::{
    button, start,
    state::{MAX_DRAFT_BYTES, TEXT_LIMIT},
    theme, DesktopState, Dialog, UiAction,
};
use crate::{
    analysis::{MissingPolicy, NumericComparison, RowPredicate, Statistic},
    data::{ColumnKind, Measurement},
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
    if state.survey_visible() {
        super::survey::show(ui, state, jobs);
        return;
    }
    if super::guidance::show(ui, state) {
        return;
    }
    super::survey::entry(ui, state);
    super::guidance::entry(ui, state);
    ui.horizontal_wrapped(|ui| {
        ui.heading("Analysis");
        super::experience::mode_switch(ui, state);
    });
    ui.separator();
    settings(ui, state, jobs);
}

pub fn settings(ui: &mut egui::Ui, state: &mut DesktopState, jobs: &mut JobRunner) {
    egui::Panel::bottom("analysis-run-footer")
        .exact_size(126.)
        .show(ui, |ui| {
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
            let add_variable = button(ui, state, "Add variable");
            if add_variable.clicked() {
                state.act(UiAction::Navigate(super::Workspace::Data));
                state.return_focus = Some(add_variable.id);
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

pub(super) fn variable_matches(column: &crate::data::Column, query: &theme::NameQuery) -> bool {
    query.matches(column.name()) || query.matches(&column.metadata().description)
}

pub(super) fn variable_context(column: &crate::data::Column) -> String {
    let measurement = match column.metadata().measurement {
        Measurement::Unknown => "Measurement not set",
        Measurement::Scale => "Scale",
        Measurement::Ordinal => "Ordinal",
        Measurement::Nominal => "Nominal",
    };
    if column.metadata().description.trim().is_empty() {
        measurement.into()
    } else {
        format!(
            "{measurement} · {}",
            theme::caption_limit(&column.metadata().description, 128)
        )
    }
}
pub(super) fn quantity_advice(
    ui: &mut egui::Ui,
    data: &crate::data::Dataset,
    ids: &[crate::data::ColumnId],
) {
    let mut coded = 0;
    let mut undeclared = 0;
    for id in ids {
        if let Ok(column) = data.column(*id) {
            match column.metadata().measurement {
                Measurement::Nominal | Measurement::Ordinal => coded += 1,
                Measurement::Unknown => undeclared += 1,
                Measurement::Scale => {}
            }
        }
    }
    if coded != 0 {
        ui.small(format!("{coded} selected variables have Nominal or Ordinal measurement. This method uses numeric codes as quantities; check whether that matches their meaning."));
    } else if undeclared != 0 {
        ui.small("Measurement levels are not declared for some selected variables. Check their coding and units before treating them as quantities.");
    }
}

pub(super) fn filter_summary(
    ui: &mut egui::Ui,
    project: &crate::project::Project,
    filter: &crate::analysis::RowFilter,
) {
    if filter.predicates.is_empty() {
        ui.small("Filter: all eligible rows.");
        return;
    }
    egui::ScrollArea::vertical()
        .id_salt("active-filter-summary")
        .max_height(44.)
        .show_rows(ui, 22., filter.predicates.len(), |ui, range| {
            for predicate in &filter.predicates[range] {
                let name = project.dataset().column(predicate.column()).map_or_else(
                    |_| format!("Unavailable variable {}", predicate.column().0),
                    |column| theme::caption(column.name()),
                );
                let text = match predicate {
                    RowPredicate::Text { value, equal, .. } => format!(
                        "{name} {} {}",
                        if *equal { "=" } else { "≠" },
                        theme::caption_limit(format_args!("{value:?}"), 160)
                    ),
                    RowPredicate::Missing { missing, .. } => {
                        format!("{name} is {}", if *missing { "missing" } else { "present" })
                    }
                    RowPredicate::Number {
                        comparison, value, ..
                    } => format!("{name} {} {value}", comparison_label(*comparison)),
                };
                ui.add(
                    egui::Label::new(egui::RichText::new(&text).small())
                        .wrap_mode(egui::TextWrapMode::Extend),
                )
                .on_hover_text(text);
            }
        });
}

fn comparison_label(comparison: NumericComparison) -> &'static str {
    match comparison {
        NumericComparison::Equal => "=",
        NumericComparison::NotEqual => "≠",
        NumericComparison::Less => "<",
        NumericComparison::LessOrEqual => "≤",
        NumericComparison::Greater => ">",
        NumericComparison::GreaterOrEqual => "≥",
    }
}

fn form(ui: &mut egui::Ui, state: &mut DesktopState, jobs: &mut JobRunner) {
    let project = Arc::clone(state.project());
    let columns = project.dataset().columns();
    ui.strong(state.draft.procedure().label());
    super::experience::prose(ui, super::experience::description(state.draft.procedure()));
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
            quantity_advice(ui, project.dataset(), &draft.columns);
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
                let heap = crate::analysis::AnalysisRequestRef::Describe(&draft)
                    .heap_bytes()
                    .unwrap_or(usize::MAX);
                let label = if state.search.trim().is_empty() {
                    "Select all"
                } else {
                    "Select all matching"
                };
                super::procedure_forms::select_all_columns(
                    ui,
                    state,
                    &mut draft.columns,
                    heap,
                    label,
                    columns
                        .iter()
                        .filter(|c| c.kind() == ColumnKind::Number && variable_matches(c, &query))
                        .map(|c| c.id()),
                );
                let candidates = || {
                    columns.iter().filter(|c| {
                        c.kind() == ColumnKind::Number
                            && !draft.columns.contains(&c.id())
                            && variable_matches(c, &query)
                    })
                };
                let mut selected = None;
                super::paged_rows::show_rows(
                    ui,
                    "describe-picker",
                    "describe candidates",
                    40.,
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
                                        Some(&variable_context(column)),
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
                            theme::selectable_value(
                                ui,
                                &mut draft.missing_policy,
                                MissingPolicy::PerVariable,
                                "Per variable",
                            );
                            theme::selectable_value(
                                ui,
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
    let project = Arc::clone(state.project());
    filter_summary(ui, &project, state.draft.common().0);
    let weight = state.draft.common().1;
    let weighting = weight.map_or_else(
        || "Unweighted observations".into(),
        |id| {
            let name = project.dataset().column(id).map_or_else(
                |_| "Unavailable variable".into(),
                |c| theme::caption_limit(c.name(), 128),
            );
            format!("Weight variable: {name} · frequency weights")
        },
    );
    ui.add(egui::Label::new(egui::RichText::new(&weighting).small()).truncate())
        .on_hover_text(format!(
            "{weighting}. Frequency weights represent repeated observations, not survey weights."
        ));
    ui.horizontal_wrapped(|ui| {
        ui.style_mut().wrap_mode = Some(egui::TextWrapMode::Extend);
        let method = ui.menu_button("Assumptions and method", |ui| {
            ui.set_max_width(420.);
            ui.label(super::experience::method_help(state.draft.procedure()));
            ui.small("Required fields being complete does not establish whether the method suits your study.");
        });
        super::procedure_forms::record(state, "Assumptions and method", &method.response);
        if (!state.draft.common().0.predicates.is_empty() || weight.is_some())
            && button(ui, state, "Clear filter and weight").clicked()
        {
            match state.checked_editable_draft() {
                Ok(mut draft) => {
                    let (filter, weight) = draft.common_mut();
                    *filter = crate::analysis::RowFilter::default();
                    *weight = None;
                    state.act(UiAction::SetDraft(draft));
                }
                Err(error) => state.fail(error),
            }
        }
    });
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
                    "Required fields complete"
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
                theme::selectable_value(ui, frequency_weight, None, "None");
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
                                theme::selectable_value(
                                    ui,
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
                    theme::selectable_value(ui, &mut operator, index, *label);
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
