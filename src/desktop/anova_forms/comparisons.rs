//! Ordered typed endpoints and fixed slices share the single discovery worker.
use super::*;

fn value(
    ui: &mut egui::Ui,
    state: &mut DesktopState,
    heap: &mut usize,
    label: &str,
    selected: &mut Option<Cell>,
    kind: Option<ColumnKind>,
    declared: Option<&[String]>,
    choices: Option<&GroupLookupResult>,
) {
    let mut choice = None;
    let r = egui::ComboBox::from_id_salt(label)
        .selected_text(selected.as_ref().map_or_else(
            || "Select exact level".into(),
            |c| theme::caption(format_args!("{c:?}")),
        ))
        .show_ui(ui, |ui| {
            if theme::selectable_label(ui, selected.is_none(), "Select exact level").clicked() {
                *selected = None;
            }
            if let Some(levels) = declared {
                for (i, s) in levels.iter().enumerate() {
                    let r = theme::selectable_label(
                        ui,
                        matches!(selected,Some(Cell::Text(t)) if t==s),
                        theme::caption(format_args!("{s:?}")),
                    );
                    record(state, &format!("{label} choice {i}"), &r);
                    if r.clicked() {
                        choice = Some((i, true));
                        ui.close();
                    }
                }
            } else if let Some(lookup) = choices {
                for (i, c) in lookup.choices.iter().enumerate() {
                    let r = theme::selectable_label(
                        ui,
                        selected.as_ref() == Some(&c.value),
                        theme::caption(format_args!("{} · {:?}", c.label, c.value)),
                    );
                    record(state, &format!("{label} choice {i}"), &r);
                    if r.clicked() {
                        choice = Some((i, false));
                        ui.close();
                    }
                }
                if lookup.more {
                    ui.label("More levels: refine the search.");
                }
            } else {
                ui.label("Activate this factor to discover its current levels.");
            }
        })
        .response;
    r.widget_info(|| egui::WidgetInfo::labeled(egui::WidgetType::ComboBox, true, label));
    record(state, label, &r);
    if let Some((i, is_declared)) = choice {
        let c = if is_declared {
            None
        } else {
            choices.and_then(|l| l.choices.get(i)).map(|c| &c.value)
        };
        if !is_declared
            && !match (kind, c.unwrap()) {
                (Some(ColumnKind::Text), Cell::Text(_)) => true,
                (Some(ColumnKind::Number), Cell::Number(n)) => n.is_finite(),
                _ => false,
            }
        {
            state.fail(AppError::invalid(
                "level",
                "Select a value with the factor's current type.",
            ));
            return;
        }
        let bytes = if is_declared {
            declared.unwrap()[i].len()
        } else {
            match c.unwrap() {
                Cell::Text(s) => s.len(),
                _ => 0,
            }
        };
        match admit(state, *heap, bytes, 0) {
            Ok(()) => {
                *heap += bytes;
                *selected = Some(if is_declared {
                    Cell::Text(declared.unwrap()[i].clone())
                } else {
                    c.unwrap().clone()
                });
            }
            Err(e) => state.fail(e),
        }
    }
}
pub(super) fn show(
    ui: &mut egui::Ui,
    state: &mut DesktopState,
    draft: &mut AnovaDraft,
    active: &mut Option<ColumnId>,
    lookup: Option<&GroupLookupResult>,
    data: &Dataset,
) {
    let panel = ui.collapsing("ANOVA comparisons", |ui| {
        let count = draft.factor_count();
        if count == 0 {
            ui.label("Select or declare factors first.");
            return;
        }
        let old = draft.comparison_factor;
        let response = egui::ComboBox::from_id_salt("comparison-factor")
            .selected_text(format!("Factor {}", old + 1))
            .show_ui(ui, |ui| {
                for i in 0..count {
                    let name = match &draft.design {
                        AnovaDesignDraft::Repeated { factors, .. } => theme::caption(format_args!("{}: {:?}", i + 1, factors[i].name)),
                        _ => theme::caption(format_args!("{}: {}", i + 1, draft.independent_factor(i).and_then(|id| data.column(id).ok()).map_or("Unavailable", |c| c.name()))),
                    };
                    let r = theme::selectable_value(ui, &mut draft.comparison_factor, i, name);
                    record(state, &format!("Comparison factor choice {i}"), &r);
                }
            }).response;
        record(state, "Comparison factor", &response);
        if old != draft.comparison_factor {
            draft.endpoints = [None, None];
            draft.fixed = None;
            *active = None;
        }
        let column = draft.independent_factor(draft.comparison_factor);
        if column.is_some() && button(ui, state, "Discover comparison levels") { *active = column; }
        let mut heap = draft.heap_bytes().unwrap_or(usize::MAX);
        if column.is_some() { query(ui, state, heap); }
        let declared = match &draft.design {
            AnovaDesignDraft::Repeated { factors, .. } => Some(factors[draft.comparison_factor].levels.as_slice()),
            _ => None,
        };
        let choices = lookup.filter(|v| Some(v.column) == column && *active == column && state.group_query() == v.query);
        let kind = column.and_then(|id| data.column(id).ok()).map(|c| c.kind());
        value(ui, state, &mut heap, "First ANOVA level", &mut draft.endpoints[0], kind, declared, choices);
        value(ui, state, &mut heap, "Second ANOVA level", &mut draft.endpoints[1], kind, declared, choices);
        let mut fixed = draft.fixed.is_some();
        let before = fixed;
        let response = ui.checkbox(&mut fixed, "Fixed slice of other factors");
        record(state, "Fixed slice of other factors", &response);
        if before != fixed {
            if fixed {
                let growth = (count - 1) * size_of::<Option<Cell>>();
                match admit(state, heap, growth, 0) {
                    Ok(()) => { draft.fixed = Some(vec![None; count - 1]); heap += growth; },
                    Err(e) => state.fail(e),
                }
            } else { draft.fixed = None; }
        }
        if let Some(values) = &mut draft.fixed {
            for i in 0..count {
                if i == draft.comparison_factor { continue; }
                let j = i - usize::from(i > draft.comparison_factor);
                let column = match &draft.design {
                    AnovaDesignDraft::OneWay { factor, .. } => *factor,
                    AnovaDesignDraft::Factorial { factors, .. } => Some(factors[i]),
                    _ => None,
                };
                if column.is_some() && button(ui, state, &format!("Discover slice factor {}", i + 1)) { *active = column; }
                let declared = match &draft.design { AnovaDesignDraft::Repeated { factors, .. } => Some(factors[i].levels.as_slice()), _ => None };
                let kind = column.and_then(|id| data.column(id).ok()).map(|c| c.kind());
                let choices = lookup.filter(|v| Some(v.column) == column && *active == column && state.group_query() == v.query);
                value(ui, state, &mut heap, &format!("Slice factor {} level", i + 1), &mut values[j], kind, declared, choices);
            }
        }
        ui.label("Planned pairs retain their selected direction and order. All-pairs families use Holm adjustment; intervals are pointwise.");
        let pair = button(ui, state, "Add planned ANOVA pair");
        let family = button(ui, state, "Add all-pairs ANOVA family");
        if pair || family { if let Err(e) = add_comparison(state, draft, pair) { state.fail(e); } }
        member_rows(ui, state, draft);
    });
    record(state, "ANOVA comparisons", &panel.header_response);
}
fn query(ui: &mut egui::Ui, state: &mut DesktopState, heap: usize) {
    if state.group_query().chars().count() > 512 {
        ui.label(theme::caption(format_args!(
            "Captured search: {:?}",
            state.group_query()
        )));
        if button(ui, state, "Replace ANOVA level search") {
            if let Err(e) = state.set_group_query(String::new()) {
                state.fail(e);
            }
        }
        return;
    }
    let limit: usize = 512;
    let capacity = state.group_query().len().max(limit.saturating_mul(4));
    let copy = heap
        .checked_add(capacity)
        .filter(|_| capacity <= MAX_DRAFT_BYTES);
    let Some(copy) = copy else {
        state.fail(AppError::SizeLimit {
            limit: MAX_DRAFT_BYTES,
        });
        return;
    };
    if let Err(e) = state.check_ui_copy(copy) {
        state.fail(e);
        return;
    }
    let mut query = String::with_capacity(capacity);
    query.push_str(state.group_query());
    let response = ui.add(
        egui::TextEdit::singleline(&mut query)
            .char_limit(limit)
            .hint_text("Search observed levels"),
    );
    record(state, "Search ANOVA levels", &response);
    if response.changed() {
        if let Err(e) = state.set_group_query(query) {
            state.fail(e);
        }
    }
}
pub(super) fn member_rows(ui: &mut egui::Ui, state: &mut DesktopState, draft: &mut AnovaDraft) {
    let height = ui.spacing().interact_size.y;
    let mut remove = None;
    super::super::paged_rows::show_rows(
        ui,
        "planned-anova-members",
        "planned comparisons",
        height,
        draft.planned_pairs.len(),
        (180., 80.),
        |ui, range| {
            for i in range {
                let p = &draft.planned_pairs[i];
                if member_row(
                    ui,
                    state,
                    height,
                    &format!("Remove planned ANOVA pair {}", i + 1),
                    theme::caption(format_args!(
                        "{}: factor {} · {:?} minus {:?} · {:?}",
                        i + 1,
                        p.factor + 1,
                        p.first,
                        p.second,
                        p.scope
                    )),
                ) {
                    remove = Some(i);
                }
            }
        },
    )
    .record(state);
    if let Some(i) = remove {
        draft.planned_pairs.remove(i);
    }
    let mut remove = None;
    super::super::paged_rows::show_rows(
        ui,
        "all-anova-members",
        "all-pairs comparisons",
        height,
        draft.all_pairs.len(),
        (180., 80.),
        |ui, range| {
            for i in range {
                let p = &draft.all_pairs[i];
                if member_row(
                    ui,
                    state,
                    height,
                    &format!("Remove all-pairs ANOVA family {}", i + 1),
                    theme::caption(format_args!(
                        "All pairs {}: factor {} · {:?}",
                        i + 1,
                        p.factor + 1,
                        p.scope
                    )),
                ) {
                    remove = Some(i);
                }
            }
        },
    )
    .record(state);
    if let Some(i) = remove {
        draft.all_pairs.remove(i);
    }
}

fn member_row(
    ui: &mut egui::Ui,
    state: &mut DesktopState,
    height: f32,
    label: &str,
    caption: String,
) -> bool {
    let mut remove = false;
    ui.horizontal(|ui| {
        ui.set_min_height(height);
        ui.style_mut().wrap_mode = Some(egui::TextWrapMode::Truncate);
        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
            remove = button(ui, state, label);
            ui.add(egui::Label::new(caption).truncate());
        });
    });
    remove
}

pub(super) fn add_comparison(
    state: &DesktopState,
    d: &mut AnovaDraft,
    pair: bool,
) -> AppResult<()> {
    let heap = d.heap_bytes()?;
    // Admit endpoint/slice copies and old/new vector overlap before constructing a member.
    let mut strings = 0usize;
    for c in d
        .endpoints
        .iter()
        .flatten()
        .filter(|_| pair)
        .chain(d.fixed.iter().flat_map(|v| v.iter().flatten()))
    {
        if let Cell::Text(s) = c {
            strings = strings.checked_add(s.len()).ok_or(AppError::SizeLimit {
                limit: MAX_DRAFT_BYTES,
            })?;
        }
    }
    let slice = d.fixed.as_ref().map_or(0, |v| v.len() * size_of::<Cell>());
    let (item, overlap) = if pair {
        (
            size_of::<AnovaPairRequest>(),
            d.planned_pairs.capacity() * size_of::<AnovaPairRequest>(),
        )
    } else {
        (
            size_of::<AnovaFamilyRequest>(),
            d.all_pairs.capacity() * size_of::<AnovaFamilyRequest>(),
        )
    };
    let growth = strings
        .checked_add(slice)
        .and_then(|n| n.checked_add(item))
        .ok_or(AppError::SizeLimit {
            limit: MAX_DRAFT_BYTES,
        })?;
    admit(state, heap, growth, overlap)?;
    let scope = match &d.fixed {
        None => AnovaScope::Marginal,
        Some(v) => AnovaScope::Fixed(
            v.iter()
                .map(|c| {
                    c.clone().ok_or_else(|| {
                        AppError::invalid("slice", "Select every fixed-slice level.")
                    })
                })
                .collect::<AppResult<_>>()?,
        ),
    };
    if pair {
        let first = d.endpoints[0]
            .as_ref()
            .ok_or_else(|| AppError::invalid("pair", "Select both endpoints."))?;
        let second = d.endpoints[1]
            .as_ref()
            .ok_or_else(|| AppError::invalid("pair", "Select both endpoints."))?;
        if first == second
            || d.planned_pairs.iter().any(|p| {
                p.factor == d.comparison_factor
                    && p.scope == scope
                    && ((&p.first == first && &p.second == second)
                        || (&p.first == second && &p.second == first))
            })
        {
            return Err(AppError::invalid(
                "pair",
                "Pairs must be distinct; a reversed duplicate is the same planned comparison.",
            ));
        }
        if d.planned_pairs.len() == d.planned_pairs.capacity() {
            d.planned_pairs.reserve_exact(1);
        }
        d.planned_pairs.push(AnovaPairRequest {
            factor: d.comparison_factor,
            first: first.clone(),
            second: second.clone(),
            scope,
        });
    } else {
        if d.all_pairs
            .iter()
            .any(|p| p.factor == d.comparison_factor && p.scope == scope)
        {
            return Err(AppError::invalid(
                "family",
                "This factor and scope already have an all-pairs family.",
            ));
        }
        if d.all_pairs.len() == d.all_pairs.capacity() {
            d.all_pairs.reserve_exact(1);
        }
        d.all_pairs.push(AnovaFamilyRequest {
            factor: d.comparison_factor,
            scope,
        });
    }
    Ok(())
}
