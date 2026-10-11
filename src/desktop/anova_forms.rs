//! Explicit roles and declared measurements; discovery never infers a within design.
use super::{
    draft::anova::*,
    procedure_forms::{record, scalar},
    state::MAX_DRAFT_BYTES,
    theme, DesktopState,
};
use crate::{
    analysis::*,
    data::{Cell, ColumnId, ColumnKind, Dataset},
    error::{AppError, AppResult},
};
use eframe::egui;
use std::{mem::size_of, sync::Arc};

fn admit(state: &DesktopState, heap: usize, growth: usize, overlap: usize) -> AppResult<()> {
    let next = heap
        .checked_add(growth)
        .filter(|n| *n <= MAX_DRAFT_BYTES)
        .ok_or(AppError::SizeLimit {
            limit: MAX_DRAFT_BYTES,
        })?;
    state.check_ui_copy(next.checked_add(overlap).ok_or(AppError::SizeLimit {
        limit: MAX_DRAFT_BYTES,
    })?)
}
fn button(ui: &mut egui::Ui, state: &mut DesktopState, label: &str) -> bool {
    let r = ui.button(label);
    record(state, label, &r);
    r.clicked()
}
fn grow<T>(state: &DesktopState, heap: &mut usize, v: &mut Vec<T>) -> AppResult<()> {
    if v.len() == v.capacity() {
        admit(
            state,
            *heap,
            size_of::<T>(),
            v.capacity()
                .checked_mul(size_of::<T>())
                .ok_or(AppError::SizeLimit {
                    limit: MAX_DRAFT_BYTES,
                })?,
        )?;
        v.reserve_exact(1);
        *heap = heap
            .checked_add(size_of::<T>())
            .ok_or(AppError::SizeLimit {
                limit: MAX_DRAFT_BYTES,
            })?;
    }
    Ok(())
}
fn edit_text(
    ui: &mut egui::Ui,
    state: &mut DesktopState,
    heap: &mut usize,
    label: &str,
    text: &mut String,
) -> bool {
    // Bound editable text layout; longer captured identities stay exact until explicit replacement.
    if text.chars().count() > 512 {
        ui.label(label);
        ui.label(theme::caption(format_args!(
            "Captured exact label: {text:?}"
        )));
        if button(ui, state, &format!("Replace {label}")) {
            let Some(remaining) = heap.checked_sub(text.capacity()) else {
                state.fail(AppError::SizeLimit {
                    limit: MAX_DRAFT_BYTES,
                });
                return false;
            };
            *heap = remaining;
            *text = String::new();
            return true;
        }
        return false;
    }
    ui.label(label);
    let mut buffer = admitted_text::EditBuffer::new(text, state, heap);
    let r = ui.add(
        egui::TextEdit::singleline(&mut buffer)
            .id_salt(("anova-text", label))
            .char_limit(512),
    );
    let changed = match buffer.finish() {
        Ok(changed) => changed,
        Err(error) => {
            state.fail(error);
            false
        }
    };
    r.widget_info(|| egui::WidgetInfo::labeled(egui::WidgetType::TextEdit, true, label));
    record(state, label, &r);
    if !text.is_empty() {
        ui.label(theme::caption(format_args!("Exact label: {text:?}")));
    }
    changed
}
pub(super) fn settings(
    ui: &mut egui::Ui,
    state: &mut DesktopState,
    draft: &mut AnovaDraft,
    active: &mut Option<ColumnId>,
    lookup: Option<&GroupLookupResult>,
) {
    let project = Arc::clone(state.project());
    let data = project.dataset();
    let mut mode = match draft.design {
        AnovaDesignDraft::OneWay { .. } => 0,
        AnovaDesignDraft::Factorial { .. } => 1,
        AnovaDesignDraft::Repeated { .. } => 2,
    };
    let previous = mode;
    for (i, label) in [
        "One-way ANOVA",
        "Factorial ANOVA",
        "Repeated measures ANOVA",
    ]
    .iter()
    .enumerate()
    {
        let r = ui.radio_value(&mut mode, i, *label);
        record(state, label, &r);
    }
    if mode != previous {
        draft.design = match mode {
            0 => AnovaDesignDraft::OneWay {
                outcome: None,
                factor: None,
            },
            1 => AnovaDesignDraft::Factorial {
                outcome: None,
                factors: vec![],
            },
            _ => AnovaDesignDraft::Repeated {
                subject: None,
                factors: vec![],
                measurements: vec![],
            },
        };
        draft.clear_comparisons();
        *active = None;
        if mode == 2 {
            draft.frequency_weight = None;
        }
        ui.label("Changing design clears roles, mappings and comparisons. Repeated measures also remove frequency weights.");
    }
    let mut heap = draft.heap_bytes().unwrap_or(usize::MAX);
    let mut roles_changed = false;
    match &mut draft.design {
        AnovaDesignDraft::OneWay { outcome, factor } => {
            let old = (*outcome, *factor);
            scalar(ui, state, data, "ANOVA outcome", outcome, true, *factor);
            scalar(ui, state, data, "ANOVA factor", factor, false, *outcome);
            roles_changed = old != (*outcome, *factor);
        }
        AnovaDesignDraft::Factorial { outcome, factors } => {
            let old = *outcome;
            scalar(ui, state, data, "ANOVA outcome", outcome, true, None);
            roles_changed = old != *outcome;
            let mut remove = None;
            for (i, id) in factors.iter().enumerate() {
                ui.horizontal(|ui| {
                    ui.label(format!(
                        "Factor {}: {}",
                        i + 1,
                        data.column(*id).map_or("Unavailable", |c| c.name())
                    ));
                    if button(ui, state, &format!("Remove ANOVA factor {}", i + 1)) {
                        remove = Some(i);
                    }
                });
            }
            if let Some(i) = remove {
                factors.remove(i);
                roles_changed = true;
            }
            if factors.len() < 6 {
                let mut selected = None;
                scalar(
                    ui,
                    state,
                    data,
                    "Add ANOVA factor",
                    &mut selected,
                    false,
                    *outcome,
                );
                if let Some(id) = selected {
                    if factors.contains(&id) {
                        state.fail(AppError::invalid(
                            "factor",
                            "Each factor must use a distinct column.",
                        ));
                    } else {
                        match grow(state, &mut heap, factors) {
                            Ok(()) => {
                                factors.push(id);
                                roles_changed = true;
                            }
                            Err(e) => state.fail(e),
                        }
                    }
                }
            }
        }
        AnovaDesignDraft::Repeated {
            subject,
            factors,
            measurements,
        } => {
            scalar(ui, state, data, "Subject ID", subject, false, None);
            ui.label("One independent subject per row. Declare ordered factors and levels; then map every combination to a distinct numeric column.");
            let mut declaration_changed = false;
            let mut remove_factor = None;
            for (i, factor) in factors.iter_mut().enumerate() {
                ui.push_id(("within", i), |ui| {
                    declaration_changed |= edit_text(
                        ui,
                        state,
                        &mut heap,
                        &format!("Within factor {} name", i + 1),
                        &mut factor.name,
                    );
                    let mut remove_level = None;
                    let row_height = ui.spacing().interact_size.y * 2.
                        + ui.text_style_height(&egui::TextStyle::Body) * 2.
                        + ui.spacing().item_spacing.y * 4.;
                    super::paged_rows::show_rows(
                        ui,
                        ("within-levels", i),
                        &format!("within factor {} levels", i + 1),
                        row_height,
                        factor.levels.len(),
                        (180., 0.),
                        |ui, range| {
                            for j in range {
                                ui.push_id(("level", j), |ui| {
                                    ui.set_min_height(row_height);
                                    ui.style_mut().wrap_mode = Some(egui::TextWrapMode::Truncate);
                                    declaration_changed |= edit_text(
                                        ui,
                                        state,
                                        &mut heap,
                                        &format!("Within factor {} level {}", i + 1, j + 1),
                                        &mut factor.levels[j],
                                    );
                                    if button(
                                        ui,
                                        state,
                                        &format!("Remove within level {}:{}", i + 1, j + 1),
                                    ) {
                                        remove_level = Some(j);
                                    }
                                });
                            }
                        },
                    )
                    .record(state);
                    if let Some(j) = remove_level {
                        factor.levels.remove(j);
                        declaration_changed = true;
                    }
                    if factor.levels.len() < 64
                        && button(ui, state, &format!("Add within level {}", i + 1))
                    {
                        match grow(state, &mut heap, &mut factor.levels) {
                            Ok(()) => {
                                factor.levels.push(String::new());
                                declaration_changed = true;
                            }
                            Err(e) => state.fail(e),
                        }
                    }
                    if button(ui, state, &format!("Remove within factor {}", i + 1)) {
                        remove_factor = Some(i);
                    }
                });
            }
            if let Some(i) = remove_factor {
                factors.remove(i);
                declaration_changed = true;
            }
            if factors.len() < 6 && button(ui, state, "Add within factor") {
                match grow(state, &mut heap, factors) {
                    Ok(()) => {
                        factors.push(AnovaWithinFactor {
                            name: String::new(),
                            levels: vec![],
                        });
                        declaration_changed = true;
                    }
                    Err(e) => state.fail(e),
                }
            }
            if declaration_changed {
                measurements.clear();
                roles_changed = true;
            }
            let grid = if factors.is_empty() {
                None
            } else {
                factors.iter().try_fold(1usize, |n, f| {
                    if f.levels.len() < 2 {
                        None
                    } else {
                        n.checked_mul(f.levels.len()).filter(|n| *n <= 64)
                    }
                })
            };
            if let Some(grid) = grid {
                if measurements.is_empty() {
                    match admit(
                        state,
                        heap,
                        grid.saturating_sub(measurements.capacity())
                            * size_of::<Option<ColumnId>>(),
                        if grid > measurements.capacity() {
                            measurements.capacity() * size_of::<Option<ColumnId>>()
                        } else {
                            0
                        },
                    ) {
                        Ok(()) => {
                            measurements.reserve_exact(grid);
                            measurements.resize(grid, None);
                        }
                        Err(e) => state.fail(e),
                    }
                }
                if measurements.len() == grid {
                    let row_height = ui.spacing().interact_size.y
                        + ui.text_style_height(&egui::TextStyle::Body)
                            * (factors.len() as f32 + 1.)
                        + ui.spacing().item_spacing.y * (factors.len() as f32 + 3.);
                    super::paged_rows::show_rows(
                        ui,
                        "within-measurements",
                        "measurements",
                        row_height,
                        grid,
                        (300., 240.),
                        |ui, range| {
                            for i in range {
                                ui.push_id(("measurement", i), |ui| {
                                    ui.set_min_height(row_height);
                                    ui.style_mut().wrap_mode = Some(egui::TextWrapMode::Truncate);
                                    let mut divisor = grid;
                                    for f in factors.iter() {
                                        divisor /= f.levels.len();
                                        let level = (i / divisor) % f.levels.len();
                                        ui.label(theme::caption(format_args!(
                                            "{:?}={:?}",
                                            f.name, f.levels[level]
                                        )));
                                    }
                                    scalar(
                                        ui,
                                        state,
                                        data,
                                        &format!("Measurement {}", i + 1),
                                        &mut measurements[i],
                                        true,
                                        *subject,
                                    );
                                });
                            }
                        },
                    )
                    .record(state);
                }
            } else {
                ui.label("Declare at least two levels per factor; the complete grid may contain at most 64 measurements.");
            }
            ui.label("Repeated measures do not accept frequency weights. Missing or duplicate retained subject IDs are invalid setup.");
        }
    }
    if roles_changed {
        draft.clear_comparisons();
        *active = None;
        ui.label("Role or declaration changes clear comparisons. Declaration changes also clear measurement mappings.");
    }
    comparisons::show(ui, state, draft, active, lookup, data);
}

#[cfg(test)]
mod tests;

mod comparisons;

mod admitted_text;
