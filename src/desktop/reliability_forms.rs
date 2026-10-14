//! Ordered items and explicit bounds edit a checked copy of the saved recipe.
use super::{
    draft::reliability::{ReliabilityDraft, ReverseDraft},
    paged_rows,
    procedure_forms::record,
    state::{MAX_DRAFT_BYTES, TEXT_LIMIT},
    theme, DesktopState,
};
use crate::{
    data::{ColumnId, ColumnKind},
    error::AppError,
};
use eframe::egui;
use std::mem::size_of;
fn action(
    ui: &mut egui::Ui,
    state: &mut DesktopState,
    label: &str,
    text: &str,
    enabled: bool,
) -> bool {
    let r = ui.add_enabled(enabled, egui::Button::new(text).truncate());
    r.widget_info(|| egui::WidgetInfo::labeled(egui::WidgetType::Button, enabled, label));
    record(state, label, &r);
    r.clicked()
}
fn admit(
    state: &mut DesktopState,
    draft: &ReliabilityDraft,
    growth: usize,
    overlap: usize,
) -> bool {
    let n = draft.heap_bytes().unwrap_or(usize::MAX);
    let result = n
        .checked_add(growth)
        .filter(|n| *n <= MAX_DRAFT_BYTES)
        .and_then(|n| n.checked_add(overlap))
        .ok_or(AppError::SizeLimit {
            limit: MAX_DRAFT_BYTES,
        })
        .and_then(|n| state.check_ui_copy(n));
    if let Err(e) = result {
        state.fail(e);
        false
    } else {
        true
    }
}
fn bound(
    ui: &mut egui::Ui,
    state: &mut DesktopState,
    id: ColumnId,
    label: &str,
    text: &mut String,
) {
    ui.horizontal(|ui| {
        ui.label(label);
        let r = ui.add(
            egui::TextEdit::singleline(text)
                .id_salt(("reliability-bound", id, label))
                .desired_width(115.)
                .char_limit(512),
        );
        let label = format!("{label} bound: {}", id.0);
        r.widget_info(|| egui::WidgetInfo::labeled(egui::WidgetType::TextEdit, true, &label));
        record(state, &label, &r);
    });
}
pub(super) fn settings(ui: &mut egui::Ui, state: &mut DesktopState, draft: &mut ReliabilityDraft) {
    let project = state.project().clone();
    let data = project.dataset();
    ui.label("2–64 numeric items · complete cases");
    let height =
        7. * (ui.text_style_height(&egui::TextStyle::Body) + ui.spacing().item_spacing.y) + 8.;
    let mut remove = None;
    let mut movement = None;
    let pages = paged_rows::show_rows(
        ui,
        "reliability-selected",
        "selected items",
        height,
        draft.items.len(),
        (height * 2., height),
        |ui, range| {
            for i in range {
                let id = draft.items[i];
                ui.push_id(id, |ui| {
                    ui.set_min_height(height);
                    ui.add(
                        egui::Label::new(theme::caption_limit(
                            format_args!(
                                "Item {} · Column {} · {}",
                                i + 1,
                                id.0,
                                data.column(id).map_or("Unavailable variable", |c| c.name())
                            ),
                            128,
                        ))
                        .truncate(),
                    );
                    ui.horizontal(|ui| {
                        if action(ui, state, &format!("Move item {} up", id.0), "Up", i > 0) {
                            movement = Some((i, i - 1));
                        }
                        if action(
                            ui,
                            state,
                            &format!("Move item {} down", id.0),
                            "Down",
                            i + 1 < draft.items.len(),
                        ) {
                            movement = Some((i, i + 1));
                        }
                        if action(ui, state, &format!("Remove item {}", id.0), "Remove", true) {
                            remove = Some(i);
                        }
                    });
                    let mut reverse = draft.reversals.iter().any(|r| r.column == id);
                    let r = ui
                        .checkbox(&mut reverse, "Reverse this item")
                        .on_hover_text(
                            "Score as lower + upper − value. Enter both bounds explicitly.",
                        );
                    let label = format!("Reverse item {}", id.0);
                    r.widget_info(|| {
                        egui::WidgetInfo::selected(
                            egui::WidgetType::Checkbox,
                            r.enabled(),
                            reverse,
                            &label,
                        )
                    });
                    record(state, &label, &r);
                    if r.changed() {
                        if reverse {
                            let overlap = draft.reversals.capacity() * size_of::<ReverseDraft>();
                            if admit(state, draft, size_of::<ReverseDraft>(), overlap) {
                                draft.reversals.reserve_exact(1);
                                draft.reversals.push(ReverseDraft {
                                    column: id,
                                    lower: String::new(),
                                    upper: String::new(),
                                });
                            }
                        } else {
                            draft.reversals.retain(|r| r.column != id);
                        }
                    }
                    if reverse && admit(state, draft, 1024, 0) {
                        if let Some(r) = draft.reversals.iter_mut().find(|r| r.column == id) {
                            bound(ui, state, id, "Lower", &mut r.lower);
                            bound(ui, state, id, "Upper", &mut r.upper);
                        }
                    }
                });
            }
        },
    );
    pages.record(state);
    if let Some((a, b)) = movement {
        draft.items.swap(a, b);
    }
    if let Some(i) = remove {
        let id = draft.items.remove(i);
        draft.reversals.retain(|r| r.column != id);
    }
    if action(ui, state, "Add item", "Add item", draft.items.len() < 64) {
        state.picker = !state.picker;
    }
    if state.picker {
        let r = ui.add(
            egui::TextEdit::singleline(&mut state.search)
                .id_salt("reliability-item-search")
                .hint_text("Search numeric items")
                .char_limit(TEXT_LIMIT),
        );
        record(state, "Search numeric items", &r);
        let query = match theme::NameQuery::new(&state.search) {
            Ok(q) => q,
            Err(e) => {
                state.fail(e);
                return;
            }
        };
        let candidates = || {
            data.columns().iter().filter(|c| {
                c.kind() == ColumnKind::Number
                    && !draft.items.contains(&c.id())
                    && super::analyses::variable_matches(c, &query)
            })
        };
        let count = candidates().count();
        let label = if state.search.trim().is_empty() {
            "Select all"
        } else {
            "Select all matching"
        };
        let enabled = count > 0 && draft.items.len() + count <= 64;
        let r = ui.add_enabled(enabled, egui::Button::new(label))
            .on_disabled_hover_text(if count == 0 { "All matching eligible items are selected." } else { "Select all would exceed the 64-item limit. Narrow the search or select items individually." });
        r.widget_info(|| {
            egui::WidgetInfo::labeled(egui::WidgetType::Button, r.enabled(), "Select all items")
        });
        record(state, "Select all items", &r);
        let bulk = r.clicked();
        let mut selected = None;
        let pages = paged_rows::show_rows(
            ui,
            "reliability-picker",
            "available items",
            40.,
            count,
            (168., 56.),
            |ui, range| {
                for c in candidates().skip(range.start).take(range.len()) {
                    let label = format!("Add item: {}", c.id().0);
                    let response = ui
                        .push_id(c.id(), |ui| {
                            theme::list_row(
                                ui,
                                false,
                                &theme::caption_limit(c.name(), 128),
                                Some(&super::analyses::variable_context(c)),
                            )
                        })
                        .inner;
                    record(state, &label, &response);
                    if response.clicked() {
                        selected = Some(c.id());
                    }
                }
            },
        );
        pages.record(state);
        if bulk {
            let growth = (draft.items.len() + count).saturating_sub(draft.items.capacity())
                * size_of::<ColumnId>();
            let overlap = if growth > 0 {
                draft.items.capacity() * size_of::<ColumnId>()
            } else {
                0
            };
            if admit(state, draft, growth, overlap) {
                draft.items.reserve_exact(count);
                for c in data.columns() {
                    if c.kind() == ColumnKind::Number
                        && super::analyses::variable_matches(c, &query)
                        && !draft.items.contains(&c.id())
                    {
                        draft.items.push(c.id());
                    }
                }
                state.picker = false;
            }
        }
        if let Some(id) = selected {
            let overlap = draft.items.capacity() * size_of::<ColumnId>();
            if admit(state, draft, size_of::<ColumnId>(), overlap) {
                draft.items.reserve_exact(1);
                draft.items.push(id);
                state.picker = false;
            }
        }
    }
}
#[cfg(test)]
mod tests;
