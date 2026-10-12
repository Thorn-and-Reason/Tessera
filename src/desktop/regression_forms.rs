//! Standalone controls edit a supplied draft; lookup choices do not certify fitted-cohort levels.
use super::{
    draft::regression::*,
    procedure_forms::{record, scalar},
    state::{MAX_DRAFT_BYTES, TEXT_LIMIT},
    theme, DesktopState,
};
use crate::{
    analysis::*,
    data::{AllocationCounter, Cell, ColumnId, ColumnKind, Dataset},
    error::{AppError, AppResult},
};
use eframe::egui;
use std::sync::Arc;

fn admitted(
    state: &DesktopState,
    draft: &RegressionDraft,
    lookup: Option<&GroupLookupResult>,
    growth: usize,
    overlap: usize,
    replaced: usize,
) -> AppResult<()> {
    let heap = draft.heap_bytes()?;
    let mut next = AllocationCounter::default();
    next.add(heap.checked_sub(replaced).ok_or(AppError::SizeLimit {
        limit: MAX_DRAFT_BYTES,
    })?)?;
    next.add(growth)?;
    if next.bytes > MAX_DRAFT_BYTES {
        return Err(AppError::SizeLimit {
            limit: MAX_DRAFT_BYTES,
        });
    }
    next.add(replaced)?;
    next.add(overlap)?;
    if let Some(lookup) = lookup {
        next.add(lookup.owned_bytes()?)?;
    }
    state.check_ui_copy(next.bytes)
}
fn action(
    ui: &mut egui::Ui,
    state: &mut DesktopState,
    text: &str,
    key: &str,
    column: ColumnId,
    enabled: bool,
) -> bool {
    let response = ui.add_enabled(enabled, egui::Button::new(text).small());
    response.widget_info(|| {
        egui::WidgetInfo::labeled(
            egui::WidgetType::Button,
            enabled,
            format!("{key} predictor {}", column.0),
        )
    });
    record(state, &format!("{key}: {}", column.0), &response);
    response.clicked()
}
fn add_predictor(
    ui: &mut egui::Ui,
    state: &mut DesktopState,
    draft: &mut RegressionDraft,
    data: &Dataset,
    lookup: Option<&GroupLookupResult>,
) {
    let mut selected = None;
    let mut bulk = false;
    let response = egui::ComboBox::from_id_salt("regression-add")
        .close_behavior(egui::PopupCloseBehavior::CloseOnClickOutside)
        .selected_text("Add predictor")
        .show_ui(ui, |ui| {
            let response = ui.add(
                egui::TextEdit::singleline(&mut state.search)
                    .id_salt("regression-predictor-search")
                    .hint_text("Search predictors")
                    .char_limit(TEXT_LIMIT),
            );
            response.widget_info(|| {
                egui::WidgetInfo::labeled(egui::WidgetType::TextEdit, true, "Search predictors")
            });
            record(state, "Search predictors", &response);
            let query = match theme::NameQuery::new(&state.search) {
                Ok(query) => query,
                Err(error) => {
                    state.fail(error);
                    return;
                }
            };
            let candidates = || {
                data.columns().iter().filter(|c| {
                    Some(c.id()) != draft.outcome
                        && !draft.predictors.iter().any(|p| p.column == c.id())
                        && query.matches(c.name())
                })
            };
            let label = if state.search.trim().is_empty() {
                "Select all"
            } else {
                "Select all matching"
            };
            let r = ui.add_enabled(candidates().count() > 0, egui::Button::new(label));
            r.widget_info(|| {
                egui::WidgetInfo::labeled(
                    egui::WidgetType::Button,
                    r.enabled(),
                    "Select all predictors",
                )
            });
            record(state, "Select all predictors", &r);
            if r.clicked() {
                bulk = true;
                ui.close();
            }
            super::paged_rows::show_rows(
                ui,
                "regression-predictor-picker",
                "predictor candidates",
                28.,
                candidates().count(),
                (168., 56.),
                |ui, range| {
                    for column in candidates().skip(range.start).take(range.len()) {
                        let response = ui
                            .push_id(column.id(), |ui| {
                                theme::selectable_label(
                                    ui,
                                    false,
                                    theme::caption_limit(column.name(), 128),
                                )
                            })
                            .inner;
                        record(
                            state,
                            &format!("Add predictor: {}", column.id().0),
                            &response,
                        );
                        if response.clicked() {
                            selected = Some(column);
                            ui.close();
                        }
                    }
                },
            )
            .record(state);
        })
        .response;
    record(state, "Add predictor", &response);
    if bulk {
        let query = match theme::NameQuery::new(&state.search) {
            Ok(query) => query,
            Err(error) => {
                state.fail(error);
                return;
            }
        };
        let eligible = |c: &&crate::data::Column| {
            Some(c.id()) != draft.outcome
                && !draft.predictors.iter().any(|p| p.column == c.id())
                && query.matches(c.name())
        };
        let count = data.columns().iter().filter(eligible).count();
        let size = std::mem::size_of::<RegressionPredictorDraft>();
        let growth =
            (draft.predictors.len() + count).saturating_sub(draft.predictors.capacity()) * size;
        let overlap = if growth > 0 {
            draft.predictors.capacity() * size
        } else {
            0
        };
        match admitted(state, draft, lookup, growth, overlap, 0) {
            Ok(()) => {
                draft.predictors.reserve_exact(count);
                for c in data.columns() {
                    if Some(c.id()) != draft.outcome
                        && query.matches(c.name())
                        && !draft.predictors.iter().any(|p| p.column == c.id())
                    {
                        draft.predictors.push(RegressionPredictorDraft {
                            column: c.id(),
                            role: if c.kind() == ColumnKind::Number {
                                RegressionDraftRole::Numeric
                            } else {
                                RegressionDraftRole::Categorical { reference: None }
                            },
                        });
                    }
                }
            }
            Err(error) => state.fail(error),
        }
    }
    if let Some(column) = selected {
        let grow = draft.predictors.len() == draft.predictors.capacity();
        let size = std::mem::size_of::<RegressionPredictorDraft>();
        let overlap = if grow {
            draft.predictors.capacity() * size
        } else {
            0
        };
        match admitted(state, draft, lookup, usize::from(grow) * size, overlap, 0) {
            Ok(()) => {
                if grow {
                    draft.predictors.reserve_exact(1);
                }
                draft.predictors.push(RegressionPredictorDraft {
                    column: column.id(),
                    role: if column.kind() == ColumnKind::Number {
                        RegressionDraftRole::Numeric
                    } else {
                        RegressionDraftRole::Categorical { reference: None }
                    },
                });
            }
            Err(error) => state.fail(error),
        }
    }
}

fn cell_caption(cell: &Cell) -> String {
    match cell {
        Cell::Text(s) => theme::caption(format_args!("{s:?}")),
        Cell::Number(n) => theme::caption(n),
        Cell::Missing => "Missing".into(),
    }
}
fn references(
    ui: &mut egui::Ui,
    state: &mut DesktopState,
    draft: &mut RegressionDraft,
    active: &mut Option<ColumnId>,
    lookup: Option<&GroupLookupResult>,
    index: usize,
) {
    let column = draft.predictors[index].column;
    let RegressionDraftRole::Categorical { reference } = &draft.predictors[index].role else {
        return;
    };
    ui.label("Reference level");
    let selected = reference
        .as_ref()
        .map_or_else(|| "Select reference level".into(), cell_caption);
    let choices = lookup.filter(|r| r.column == column);
    let mut selected_index = None;
    let mut clear = false;
    let response = egui::ComboBox::from_id_salt(("regression-reference", column))
        .height(180.0)
        .selected_text(selected)
        .show_ui(ui, |ui| {
            clear = theme::selectable_label(ui, reference.is_none(), "Select reference level")
                .clicked();
            if clear {
                ui.close();
            }
            if let Some(choices) = choices {
                for (i, choice) in choices.choices.iter().enumerate() {
                    let text = theme::caption(format_args!(
                        "{} · {}",
                        choice.label,
                        cell_caption(&choice.value)
                    ));
                    let response = ui
                        .push_id(i, |ui| {
                            theme::selectable_label(
                                ui,
                                reference.as_ref() == Some(&choice.value),
                                text,
                            )
                        })
                        .inner;
                    record(
                        state,
                        &format!("Reference choice: {}: {i}", column.0),
                        &response,
                    );
                    if response.clicked() {
                        selected_index = Some(i);
                        ui.close();
                    }
                }
                if choices.more {
                    ui.label("More levels available");
                }
            }
        })
        .response;
    response.widget_info(|| {
        egui::WidgetInfo::labeled(egui::WidgetType::ComboBox, true, "Reference level")
    });
    record(state, &format!("Reference level: {}", column.0), &response);
    if response.clicked() {
        *active = Some(column);
    }
    if clear {
        draft.predictors[index].role = RegressionDraftRole::Categorical { reference: None };
    }
    if let Some(choice) = selected_index.and_then(|i| choices.map(|r| &r.choices[i])) {
        let kind = state
            .project()
            .dataset()
            .column(column)
            .ok()
            .map(|c| c.kind());
        let valid = matches!(
            (&choice.value, kind),
            (Cell::Text(_), Some(ColumnKind::Text))
        ) || matches!((&choice.value, kind), (Cell::Number(v), Some(ColumnKind::Number)) if v.is_finite());
        if !valid {
            state.fail(AppError::invalid(
                "reference level",
                "Select an observed reference level.",
            ));
            return;
        }
        let bytes = match &choice.value {
            Cell::Text(s) => s.len(),
            _ => 0,
        };
        let replaced = match &draft.predictors[index].role {
            RegressionDraftRole::Categorical {
                reference: Some(Cell::Text(s)),
            } => s.capacity(),
            _ => 0,
        };
        match admitted(state, draft, lookup, bytes, 0, replaced) {
            Ok(()) => {
                draft.predictors[index].role = RegressionDraftRole::Categorical {
                    reference: Some(choice.value.clone()),
                }
            }
            Err(error) => state.fail(error),
        }
    }
}
fn predictors(
    ui: &mut egui::Ui,
    state: &mut DesktopState,
    draft: &mut RegressionDraft,
    active: &mut Option<ColumnId>,
    lookup: Option<&GroupLookupResult>,
    data: &Dataset,
) {
    let mut remove = None;
    let mut reorder = None;
    for index in 0..draft.predictors.len() {
        let column = draft.predictors[index].column;
        ui.push_id(column, |ui| {
            ui.label(data.column(column).map_or_else(
                |_| format!("Unavailable variable {}", column.0),
                |c| theme::caption(c.name()),
            ));
            ui.horizontal(|ui| {
                if action(ui, state, "↑", "Move up", column, index > 0) {
                    reorder = Some((index, index - 1));
                }
                if action(
                    ui,
                    state,
                    "↓",
                    "Move down",
                    column,
                    index + 1 < draft.predictors.len(),
                ) {
                    reorder = Some((index, index + 1));
                }
                if action(ui, state, "Remove", "Remove", column, true) {
                    remove = Some(index);
                }
            });
            if data
                .column(column)
                .is_ok_and(|c| c.kind() == ColumnKind::Number)
            {
                ui.horizontal(|ui| {
                    let numeric =
                        matches!(draft.predictors[index].role, RegressionDraftRole::Numeric);
                    for (label, selected) in [("Numeric", numeric), ("Categorical", !numeric)] {
                        let response = theme::selectable_label(ui, selected, label);
                        record(state, &format!("{label}: {}", column.0), &response);
                        if response.clicked() && !selected {
                            draft.predictors[index].role = if numeric {
                                RegressionDraftRole::Categorical { reference: None }
                            } else {
                                RegressionDraftRole::Numeric
                            };
                            if *active == Some(column) {
                                *active = None;
                            }
                        }
                    }
                });
            }
            references(ui, state, draft, active, lookup, index);
        });
    }
    if let Some(index) = remove {
        if *active == Some(draft.predictors[index].column) {
            *active = None;
        }
        draft.predictors.remove(index);
    } else if let Some((a, b)) = reorder {
        draft.predictors.swap(a, b);
    }
    add_predictor(ui, state, draft, data, lookup);
}
pub(super) fn settings(
    ui: &mut egui::Ui,
    state: &mut DesktopState,
    draft: &mut RegressionDraft,
    active: &mut Option<ColumnId>,
    lookup: Option<&GroupLookupResult>,
) {
    let project = Arc::clone(state.project());
    scalar(
        ui,
        state,
        project.dataset(),
        "Outcome",
        &mut draft.outcome,
        true,
        None,
    );
    ui.strong("Predictors");
    predictors(ui, state, draft, active, lookup, project.dataset());
    if draft.group_roles(*active).is_none() {
        *active = None;
    }
}
#[cfg(test)]
mod tests;
