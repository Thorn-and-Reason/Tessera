//! Native forms share the same checked draft and single worker as shortcuts.
use super::{
    analyses,
    draft::{AnalysisDraft, Procedure},
    state::TEXT_LIMIT,
    theme, DesktopState, UiAction,
};
use crate::{
    analysis::*,
    data::{Cell, ColumnId, ColumnKind, Dataset},
    jobs::JobRunner,
};
use eframe::egui;
use std::sync::Arc;
pub(super) fn record(state: &mut DesktopState, label: &str, response: &egui::Response) {
    state.controls.insert(label.into(), response.rect);
    state.control_ids.insert(label.into(), response.id);
}
pub(super) fn append_column(
    state: &mut DesktopState,
    columns: &mut Vec<ColumnId>,
    heap: usize,
    id: ColumnId,
) {
    let grow = columns.len() == columns.capacity();
    let size = std::mem::size_of::<ColumnId>();
    let next = heap
        .checked_add(usize::from(grow) * size)
        .filter(|n| *n <= super::state::MAX_DRAFT_BYTES)
        .and_then(|n| {
            n.checked_add(if grow {
                columns.capacity().saturating_mul(size)
            } else {
                0
            })
        })
        .ok_or(crate::error::AppError::SizeLimit {
            limit: super::state::MAX_DRAFT_BYTES,
        })
        .and_then(|n| state.check_ui_copy(n));
    if let Err(error) = next {
        state.fail(error);
        return;
    }
    if grow {
        columns.reserve_exact(1);
    }
    columns.push(id);
}
pub(super) fn chooser(ui: &mut egui::Ui, state: &mut DesktopState, full_height: bool) {
    theme::section(ui, "Methods");
    let mut procedure = state.draft.procedure();
    let response = ui.add(
        egui::TextEdit::singleline(&mut state.procedure_search)
            .id_salt("analysis-procedure-search")
            .hint_text("Search methods")
            .desired_width(ui.available_width())
            .char_limit(TEXT_LIMIT),
    );
    response.widget_info(|| {
        egui::WidgetInfo::labeled(egui::WidgetType::TextEdit, true, "Search analyses")
    });
    record(state, "Search procedures", &response);
    record(state, "Procedure", &response);
    let query = match theme::NameQuery::new(&state.procedure_search) {
        Ok(q) => q,
        Err(e) => {
            state.fail(e);
            return;
        }
    };
    egui::ScrollArea::vertical()
        .id_salt("analysis-catalog")
        .max_height(if full_height {
            ui.available_height()
        } else {
            72.
        })
        .show(ui, |ui| {
            let mut matches = 0;
            for family in ["Summaries", "Relationships", "Comparisons"] {
                let choices: Vec<_> = Procedure::ALL
                    .into_iter()
                    .filter(|p| {
                        p.family() == family && super::experience::procedure_matches(*p, &query)
                    })
                    .collect();
                if choices.is_empty() {
                    continue;
                }
                ui.small(family);
                for p in choices {
                    matches += 1;
                    let response = theme::list_row(ui, procedure == p, p.label(), None)
                        .on_hover_text(super::experience::task_label(p));
                    record(state, p.label(), &response);
                    if response.clicked() {
                        procedure = p;
                    }
                }
            }
            if matches == 0 {
                ui.label("No analyses match. Try a different question or method name.");
            }
        });
    if procedure != state.draft.procedure() {
        match state.checked_editable_draft() {
            Ok(draft) => {
                state.act(UiAction::SetDraft(draft.for_procedure(procedure)));
                state.picker = false;
                state.search.clear();
            }
            Err(e) => state.fail(e),
        }
    }
}
pub(super) fn scalar(
    ui: &mut egui::Ui,
    state: &mut DesktopState,
    data: &Dataset,
    label: &str,
    role: &mut Option<ColumnId>,
    numeric: bool,
    other: Option<ColumnId>,
) {
    ui.label(label);
    let selected = role.and_then(|id| data.column(id).ok()).map_or_else(
        || {
            role.map_or_else(
                || "Select variable".into(),
                |id| format!("Unavailable variable {}", id.0),
            )
        },
        |c| theme::caption(c.name()),
    );
    let response = egui::ComboBox::from_id_salt(label)
        .close_behavior(egui::PopupCloseBehavior::CloseOnClickOutside)
        .selected_text(selected)
        .show_ui(ui, |ui| {
            let response = ui.add(
                egui::TextEdit::singleline(&mut state.search)
                    .id_salt(("analysis-role-search", label))
                    .hint_text("Search variables")
                    .char_limit(TEXT_LIMIT),
            );
            response.widget_info(|| {
                egui::WidgetInfo::labeled(
                    egui::WidgetType::TextEdit,
                    true,
                    format!("Search {label}"),
                )
            });
            let query = match theme::NameQuery::new(&state.search) {
                Ok(q) => q,
                Err(e) => {
                    state.fail(e);
                    return;
                }
            };
            if theme::selectable_value(ui, role, None, "Select variable").clicked() {
                ui.close();
            }
            let candidates = || {
                data.columns().iter().filter(|c| {
                    (!numeric || c.kind() == ColumnKind::Number)
                        && Some(c.id()) != other
                        && query.matches(c.name())
                })
            };
            super::paged_rows::show_rows(
                ui,
                ("scalar-columns", label),
                &format!("{label} variables"),
                28.,
                candidates().count(),
                (168., 56.),
                |ui, range| {
                    for c in candidates().skip(range.start).take(range.len()) {
                        let response = ui
                            .push_id(c.id(), |ui| {
                                theme::selectable_value(
                                    ui,
                                    role,
                                    Some(c.id()),
                                    theme::caption_limit(c.name(), 128),
                                )
                            })
                            .inner;
                        record(state, &format!("{label}: {}", c.id().0), &response);
                        if response.clicked() {
                            ui.close();
                        }
                    }
                },
            )
            .record(state);
        })
        .response;
    response.widget_info(|| egui::WidgetInfo::labeled(egui::WidgetType::ComboBox, true, label));
    record(state, label, &response);
    if role.is_none()
        && label != "Grouping variable (optional)"
        && label != "Add frequency variable"
    {
        ui.small(if numeric {
            "Select a numeric variable."
        } else {
            "Select a variable."
        });
    }
}
pub(super) fn confidence(ui: &mut egui::Ui, state: &mut DesktopState, value: &mut ConfidenceLevel) {
    ui.label("Confidence interval");
    let label = |c| match c {
        ConfidenceLevel::P90 => "90%",
        ConfidenceLevel::P95 => "95%",
        ConfidenceLevel::P99 => "99%",
    };
    let response = egui::ComboBox::from_id_salt("confidence")
        .selected_text(label(*value))
        .show_ui(ui, |ui| {
            for c in [
                ConfidenceLevel::P90,
                ConfidenceLevel::P95,
                ConfidenceLevel::P99,
            ] {
                theme::selectable_value(ui, value, c, label(c));
            }
        })
        .response;
    response.widget_info(|| {
        egui::WidgetInfo::labeled(egui::WidgetType::ComboBox, true, "Confidence interval")
    });
    record(state, "Confidence interval", &response);
}
fn pearson_roles(
    ui: &mut egui::Ui,
    state: &mut DesktopState,
    data: &Dataset,
    request: &mut PearsonRequest,
) {
    theme::section(ui, "Variables");
    if request.columns.len() < 2 {
        ui.small("Select at least two numeric variables.");
    }
    ui.small(format!("Selected ({})", request.columns.len()));
    let mut remove = None;
    super::paged_rows::show_rows(
        ui,
        "pearson-selected",
        "selected correlation variables",
        28.,
        request.columns.len(),
        (168., 56.),
        |ui, range| {
            for i in range {
                let id = &request.columns[i];
                ui.horizontal(|ui| {
                    let name = data.column(*id).map_or_else(
                        |_| format!("Unavailable variable {}", id.0),
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
                                "Remove variable {} · {}",
                                id.0,
                                data.column(*id).map_or_else(
                                    |_| id.0.to_string(),
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
        },
    )
    .record(state);
    if let Some(i) = remove {
        request.columns.remove(i);
    }
    {
        ui.small("Available numeric variables");
        let response = ui.add(
            egui::TextEdit::singleline(&mut state.search)
                .id_salt("pearson-variable-search")
                .hint_text("Search variables")
                .char_limit(TEXT_LIMIT),
        );
        super::procedure_forms::record(state, "Add variables…", &response);
        response.widget_info(|| {
            egui::WidgetInfo::labeled(egui::WidgetType::TextEdit, true, "Search variables")
        });
        if let Ok(query) = theme::NameQuery::new(&state.search) {
            let candidates = || {
                data.columns().iter().filter(|c| {
                    c.kind() == ColumnKind::Number
                        && !request.columns.contains(&c.id())
                        && query.matches(c.name())
                })
            };
            let mut selected = None;
            super::paged_rows::show_rows(
                ui,
                "pearson-picker",
                "correlation candidates",
                28.,
                candidates().count(),
                (168., 56.),
                |ui, range| {
                    for c in candidates().skip(range.start).take(range.len()) {
                        let response = ui
                            .push_id(c.id(), |ui| {
                                theme::list_row(
                                    ui,
                                    false,
                                    &theme::caption_limit(c.name(), 128),
                                    None,
                                )
                            })
                            .inner;
                        record(
                            state,
                            &format!("Correlation variable {}", c.id().0),
                            &response,
                        );
                        if response.clicked() {
                            selected = Some(c.id());
                        }
                    }
                },
            )
            .record(state);
            if let Some(id) = selected {
                let heap = AnalysisRequestRef::Pearson(request)
                    .heap_bytes()
                    .unwrap_or(usize::MAX);
                append_column(state, &mut request.columns, heap, id);
            }
        }
    }
}
fn pearson_options(ui: &mut egui::Ui, request: &mut PearsonRequest) {
    ui.label("Missing values");
    let response = egui::ComboBox::from_id_salt("correlation-missing")
        .selected_text(match request.missing_policy {
            CorrelationMissingPolicy::PairwiseComplete => "Pairwise complete",
            CorrelationMissingPolicy::CompleteCases => "Complete cases",
        })
        .show_ui(ui, |ui| {
            theme::selectable_value(
                ui,
                &mut request.missing_policy,
                CorrelationMissingPolicy::PairwiseComplete,
                "Pairwise complete",
            );
            theme::selectable_value(
                ui,
                &mut request.missing_policy,
                CorrelationMissingPolicy::CompleteCases,
                "Complete cases",
            );
        })
        .response;
    response.widget_info(|| {
        egui::WidgetInfo::labeled(egui::WidgetType::ComboBox, true, "Missing values")
    });
}
fn cell_text(cell: &Cell) -> String {
    match cell {
        Cell::Number(n) => n.to_string(),
        Cell::Text(s) => theme::caption(format_args!("{s:?}")),
        Cell::Missing => "Missing".into(),
    }
}
fn groups(
    ui: &mut egui::Ui,
    state: &mut DesktopState,
    values: &mut [Option<Cell>; 2],
    draft_bytes: usize,
) {
    ui.label("Search observed groups");
    let mut query = state.group_query().to_owned();
    let response = ui.add(
        egui::TextEdit::singleline(&mut query)
            .id_salt("analysis-group-query")
            .hint_text("Search groups")
            .char_limit(TEXT_LIMIT),
    );
    response.widget_info(|| {
        egui::WidgetInfo::labeled(egui::WidgetType::TextEdit, true, "Search observed groups")
    });
    state
        .controls
        .insert("Search observed groups".into(), response.rect);
    state
        .control_ids
        .insert("Search observed groups".into(), response.id);
    if response.changed() {
        if let Err(e) = state.set_group_query(query) {
            state.fail(e);
        }
    }
    let result = state.group_lookup().cloned();
    for i in 0..2 {
        let label = if i == 0 {
            "First group"
        } else {
            "Second group"
        };
        ui.label(label);
        let selected = values[i]
            .as_ref()
            .map_or_else(|| "Select group".into(), cell_text);
        let response = egui::ComboBox::from_id_salt(label)
            .selected_text(selected)
            .show_ui(ui, |ui| {
                if theme::selectable_label(ui, values[i].is_none(), "Select group").clicked() {
                    values[i] = None;
                }
                if let Some(result) = &result {
                    egui::ScrollArea::vertical()
                        .max_height(180.0)
                        .show(ui, |ui| {
                            for (index, choice) in result.choices.iter().enumerate() {
                                if values[1 - i].as_ref() == Some(&choice.value) {
                                    continue;
                                }
                                let text =
                                    theme::caption(format_args!("{}. {}", index + 1, choice.label));
                                let response = ui
                                    .push_id(index, |ui| {
                                        theme::selectable_label(
                                            ui,
                                            values[i].as_ref() == Some(&choice.value),
                                            text,
                                        )
                                    })
                                    .inner;
                                record(state, &format!("{label}: choice {index}"), &response);
                                if response.clicked()
                                    && values[1 - i].as_ref() != Some(&choice.value)
                                {
                                    let bytes = match &choice.value {
                                        Cell::Text(s) => s.len(),
                                        _ => 0,
                                    };
                                    if draft_bytes.saturating_add(bytes)
                                        <= super::state::MAX_DRAFT_BYTES
                                    {
                                        match crate::data::Budget::check_datasets(
                                            &[state.project().dataset()],
                                            state
                                                .project()
                                                .owned_bytes()
                                                .unwrap_or(usize::MAX)
                                                .saturating_add(state.extra())
                                                .saturating_add(draft_bytes)
                                                .saturating_add(bytes),
                                        ) {
                                            Ok(()) => values[i] = Some(choice.value.clone()),
                                            Err(e) => state.fail(e),
                                        }
                                    } else {
                                        state.fail(crate::error::AppError::SizeLimit {
                                            limit: super::state::MAX_DRAFT_BYTES,
                                        });
                                    }
                                }
                            }
                        });
                    if result.more {
                        ui.label("More groups · refine the search");
                    }
                }
            })
            .response;
        response.widget_info(|| egui::WidgetInfo::labeled(egui::WidgetType::ComboBox, true, label));
        record(state, label, &response);
        if values[i].is_none() {
            ui.small("Select an observed group.");
        }
        if values[i].is_some() && result.as_ref().is_some_and(|r| !r.selected_present[i]) {
            ui.label("Selected group absent from current data");
        }
    }
}
pub(super) fn settings(ui: &mut egui::Ui, state: &mut DesktopState, jobs: &mut JobRunner) {
    let mut draft = match state.checked_editable_draft() {
        Ok(d) => d,
        Err(e) => {
            state.fail(e);
            return;
        }
    };
    let bytes = draft.heap_bytes().unwrap_or(usize::MAX);
    let mut active_reference = state.active_regression_reference;
    let lookup = state.group_lookup().cloned();
    let project = Arc::clone(state.project());
    let data = project.dataset();
    egui::ScrollArea::vertical().id_salt("procedure-form").max_height(ui.available_height().max(0.)).show(ui,|ui| {
        theme::section(ui, "Variables");
        match &mut draft {
            AnalysisDraft::Reliability(r) => super::reliability_forms::settings(ui,state,r),
            AnalysisDraft::Anova(r) => super::anova_forms::settings(ui,state,r,&mut active_reference,lookup.as_deref()),
            AnalysisDraft::Regression(r) => super::regression_forms::settings(ui, state, r, &mut active_reference, lookup.as_deref()),
            AnalysisDraft::Pearson(r) => pearson_roles(ui,state,data,r),
            AnalysisDraft::OneSampleT { variable, reference_text, .. } => {
                scalar(ui,state,data,"Variable",variable,true,None);
                ui.label("Reference mean");
                let response = ui.add(egui::TextEdit::singleline(reference_text).id_salt("analysis-reference-mean").char_limit(512));
                response.widget_info(|| egui::WidgetInfo::labeled(egui::WidgetType::TextEdit,true,"Reference mean"));
                state.controls.insert("Reference mean".into(),response.rect);
                state.control_ids.insert("Reference mean".into(),response.id);
                if reference_text.trim().parse::<f64>().ok().filter(|n| n.is_finite()).is_none() {
                    ui.small("Enter a finite reference mean, for example 0 or 2.5.");
                }
                ui.small("Difference: sample mean minus reference mean.");
            }
            AnalysisDraft::PairedT { first,second,.. } => {
                scalar(ui,state,data,"First variable",first,true,*second);
                scalar(ui,state,data,"Second variable",second,true,*first);
                ui.small("Difference: First minus Second, using complete pairs.");
            }
            AnalysisDraft::IndependentT { outcome,group,groups: values,.. } => {
                scalar(ui,state,data,"Outcome",outcome,true,*group);
                let previous = *group;
                scalar(ui,state,data,"Grouping variable",group,false,*outcome);
                if previous != *group { *values = [None,None]; }
                if group.is_some() && previous == *group { groups(ui,state,values,bytes); }
                ui.small("Difference: First group minus Second group.");
            }
            AnalysisDraft::Frequencies(_) | AnalysisDraft::Histogram {..} | AnalysisDraft::Scatter {..} | AnalysisDraft::Boxplot {..} => super::exploration_forms::show(ui,state,data,&mut draft),
            AnalysisDraft::Describe(_) => unreachable!(),
        }
        super::experience::summary(ui, state, super::experience::settings_summary(&draft), draft.common().1);
        let header = super::experience::detailed_settings(ui, state.analysis_mode(), egui::Id::new("procedure-details"), |ui| {
            match &mut draft {
                AnalysisDraft::Pearson(r) => pearson_options(ui, r),
                AnalysisDraft::OneSampleT { confidence: c, .. } | AnalysisDraft::PairedT { confidence: c, .. } => confidence(ui, state, c),
                AnalysisDraft::IndependentT { confidence: c, method, .. } => {
                    confidence(ui, state, c);
                    ui.label("Independent method");
                    ui.radio_value(method, IndependentMethod::Welch, "Welch");
                    ui.radio_value(method, IndependentMethod::Pooled, "Pooled equal variance");
                }
                AnalysisDraft::Regression(r) => confidence(ui, state, &mut r.confidence),
                AnalysisDraft::Anova(r) => confidence(ui, state, &mut r.confidence),
                AnalysisDraft::Histogram { bins, .. } => super::exploration_forms::bins(ui, state, bins),
                _ => {}
            }
            let repeated = matches!(&draft, AnalysisDraft::Anova(r) if matches!(r.design, super::draft::anova::AnovaDesignDraft::Repeated { .. }));
            let (filter,weight) = draft.common_mut();
            analyses::common_settings_with_weight(ui,&project,filter,weight,!repeated).record(state);
            if repeated { ui.label("Frequency weights are disabled for repeated measures: each row is one independent subject."); }
        });
        if let Some(response) = header { record(state, "More settings", &response); }
        ui.collapsing("Method",|ui| {
            ui.label(match draft.procedure() {
                Procedure::Reliability => "Alpha does not establish unidimensionality.",
                Procedure::Anova => crate::export::report_anova::METHOD,
                Procedure::Regression => crate::export::report_regression::METHOD,
                Procedure::Pearson => crate::export::report_analyses::PEARSON_METHOD,
                Procedure::OneSampleT => "Difference is the sample mean minus the reference mean. Inference is two-sided.",
                Procedure::PairedT => "Difference is First minus Second for complete pairs. Inference is two-sided.",
                Procedure::IndependentT => "Difference is First group minus Second group. Welch allows unequal variances; pooled inference assumes equal variances. Inference is two-sided.",
                Procedure::Frequencies => crate::export::report_exploration::FREQUENCY_METHOD,
                Procedure::Histogram => crate::export::report_exploration::HISTOGRAM_METHOD,
                Procedure::Scatter => crate::export::report_exploration::SCATTER_METHOD,
                Procedure::Boxplot => crate::export::report_exploration::BOXPLOT_METHOD,
                Procedure::Describe => unreachable!(),
            });
            ui.label("Frequency weights mean literal replication of observations.");
        });
    });
    drop(lookup);
    if draft != state.draft || active_reference != state.active_regression_reference {
        if let Err(error) = state.set_draft_reference(draft, active_reference) {
            state.fail(error);
        }
    }
    state.service_lookup(jobs);
}

#[cfg(test)]
mod reliability_tests;
