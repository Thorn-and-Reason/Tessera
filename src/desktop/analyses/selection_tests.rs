use super::*;
use crate::{
    data::{Column, ColumnId, Dataset},
    project::Project,
};

#[test]
fn describe_pages_reach_candidates_after_256() {
    let data = Dataset::new(
        (1..=300)
            .map(|id| {
                Column::numeric(ColumnId(id), &format!("Variable {id}"), vec![Some(1.)]).unwrap()
            })
            .collect(),
    )
    .unwrap();
    let mut state =
        DesktopState::from_project(Project::new("Paged describe", data).unwrap()).unwrap();
    state.picker = true;
    let ctx = egui::Context::default();
    let mut jobs = JobRunner::default();
    let mut reached = false;
    for _ in 0..1800 {
        let focused = ctx.memory(|m| m.focused());
        let key =
            if focused.is_some() && focused == state.control_id("Next describe candidates page") {
                egui::Key::Enter
            } else {
                egui::Key::Tab
            };
        state.controls.clear();
        state.control_ids.clear();
        let mut output = ctx.run_ui(
            egui::RawInput {
                screen_rect: Some(egui::Rect::from_min_size(
                    egui::Pos2::ZERO,
                    egui::vec2(760., 520.),
                )),
                events: vec![egui::Event::Key {
                    key,
                    physical_key: None,
                    pressed: true,
                    repeat: false,
                    modifiers: egui::Modifiers::NONE,
                }],
                ..Default::default()
            },
            |ui| {
                egui::CentralPanel::default().show(ui, |ui| settings(ui, &mut state, &mut jobs));
            },
        );
        output.textures_delta.clear();
        if state.control_rect("Variable 300").is_some() {
            reached = true;
            break;
        }
    }
    assert!(
        reached,
        "Keyboard paging must reach candidates beyond the first 256"
    );
}

#[test]
fn describe_selected_rows_are_bounded_and_keep_all_saved_ids_and_long_names() {
    let exact_name = "ŵ identity ".repeat(30000);
    let data = Dataset::new(
        (1..=300)
            .map(|id| {
                let ordinary_name = format!("Variable {id}");
                Column::numeric(
                    ColumnId(id),
                    if id == 1 { &exact_name } else { &ordinary_name },
                    vec![Some(1.)],
                )
                .unwrap()
            })
            .collect(),
    )
    .unwrap();
    let mut state =
        DesktopState::from_project(Project::new("Saved selection", data).unwrap()).unwrap();
    let super::super::draft::AnalysisDraft::Describe(request) = &mut state.draft else {
        panic!()
    };
    request.columns = (1..=300).map(ColumnId).collect();
    let original = state.draft.clone();
    let ctx = egui::Context::default();
    let mut jobs = JobRunner::default();
    let mut output = ctx.run_ui(
        egui::RawInput {
            screen_rect: Some(egui::Rect::from_min_size(
                egui::Pos2::ZERO,
                egui::vec2(760., 520.),
            )),
            ..Default::default()
        },
        |ui| {
            egui::CentralPanel::default().show(ui, |ui| {
                let row_rect = egui::Rect::from_min_size(ui.cursor().min, egui::vec2(248., 488.));
                ui.scope_builder(egui::UiBuilder::new().max_rect(row_rect), |ui| {
                    settings(ui, &mut state, &mut jobs)
                });
            });
        },
    );
    output.textures_delta.clear();
    let remove = state
        .control_rect("Remove variable 1")
        .expect("visible first selected row");
    assert!(
        remove.max.x <= 256.,
        "Remove must fit the narrow settings row: {remove:?}"
    );
    assert_eq!(state.draft, original);
    assert_eq!(
        state
            .project()
            .dataset()
            .column(ColumnId(1))
            .unwrap()
            .name(),
        exact_name
    );
    let removes = state
        .controls
        .keys()
        .filter(|label| label.starts_with("Remove variable "))
        .count();
    assert!(
        removes > 0 && removes <= 10,
        "Only visible selected rows should be painted: {removes}"
    );
}

#[test]
fn selected_column_growth_is_refused_before_allocation_at_both_budgets() {
    for exhausted_project in [false, true] {
        let mut state = DesktopState::new().unwrap();
        if exhausted_project {
            state.active_recipe_bytes = crate::data::MAX_OWNED_BYTES;
        }
        let mut columns = Vec::new();
        super::super::procedure_forms::append_column(
            &mut state,
            &mut columns,
            if exhausted_project {
                0
            } else {
                MAX_DRAFT_BYTES
            },
            ColumnId(1),
        );
        assert!(columns.is_empty());
        assert_eq!(columns.capacity(), 0);
        assert!(state.error().is_some());
    }
}

#[test]
fn numeric_variable_choices_are_visible_without_opening_a_picker() {
    let data = Dataset::new(vec![
        Column::numeric(ColumnId(1), "Score", vec![Some(1.)]).unwrap()
    ])
    .unwrap();
    let mut state =
        DesktopState::from_project(Project::new("Visible choices", data).unwrap()).unwrap();
    let ctx = egui::Context::default();
    let mut jobs = JobRunner::default();
    let mut output = ctx.run_ui(
        egui::RawInput {
            screen_rect: Some(egui::Rect::from_min_size(
                egui::Pos2::ZERO,
                egui::vec2(1180., 780.),
            )),
            ..Default::default()
        },
        |ui| {
            egui::CentralPanel::default().show(ui, |ui| settings(ui, &mut state, &mut jobs));
        },
    );
    output.textures_delta.clear();
    assert!(state.control_rect("Variable 1").is_some());
}

#[test]
fn select_all_describe_includes_every_page_without_duplicates_or_text() {
    for search in ["", "Variable 29"] {
        let data = Dataset::new(
            (1..=300)
                .map(|id| {
                    Column::numeric(ColumnId(id), &format!("Variable {id}"), vec![Some(1.)])
                        .unwrap()
                })
                .chain([Column::text(ColumnId(301), "Text", vec![Some("hello".into())]).unwrap()])
                .collect(),
        )
        .unwrap();
        let mut state =
            DesktopState::from_project(Project::new("Bulk selection", data).unwrap()).unwrap();
        state.search = search.into();
        let label = if search.is_empty() {
            "Select all"
        } else {
            "Select all matching"
        };
        let ctx = egui::Context::default();
        let mut jobs = JobRunner::default();
        for frame in 0..4 {
            if frame == 2 {
                ctx.memory_mut(|m| m.request_focus(state.control_id(label).expect("bulk action")));
            }
            let mut output = ctx.run_ui(
                egui::RawInput {
                    screen_rect: Some(egui::Rect::from_min_size(
                        egui::Pos2::ZERO,
                        egui::vec2(1000., 1000.),
                    )),
                    events: if frame == 2 {
                        vec![egui::Event::Key {
                            key: egui::Key::Enter,
                            physical_key: None,
                            pressed: true,
                            repeat: false,
                            modifiers: egui::Modifiers::NONE,
                        }]
                    } else {
                        vec![]
                    },
                    ..Default::default()
                },
                |ui| {
                    settings(ui, &mut state, &mut jobs);
                },
            );
            output.textures_delta.clear();
        }
        let super::super::draft::AnalysisDraft::Describe(request) = &state.draft else {
            panic!()
        };
        assert_eq!(
            request.columns,
            (1..=300)
                .filter(|id| format!("Variable {id}").contains(search))
                .map(ColumnId)
                .collect::<Vec<_>>()
        );
    }
}

#[test]
fn bulk_selection_preserves_order_and_refuses_both_budgets_atomically() {
    for budget in 0..3 {
        let mut state = DesktopState::new().unwrap();
        state.search = "match".into();
        if budget == 2 {
            state.active_recipe_bytes = crate::data::MAX_OWNED_BYTES;
        }
        let mut columns = vec![ColumnId(3)];
        let original_capacity = columns.capacity();
        let ctx = egui::Context::default();
        for frame in 0..4 {
            if frame == 2 {
                ctx.memory_mut(|m| {
                    m.request_focus(state.control_id("Select all matching").unwrap())
                });
            }
            let mut output = ctx.run_ui(
                egui::RawInput {
                    events: if frame == 2 {
                        vec![egui::Event::Key {
                            key: egui::Key::Enter,
                            physical_key: None,
                            pressed: true,
                            repeat: false,
                            modifiers: egui::Modifiers::NONE,
                        }]
                    } else {
                        vec![]
                    },
                    ..Default::default()
                },
                |ui| {
                    super::super::procedure_forms::select_all_columns(
                        ui,
                        &mut state,
                        &mut columns,
                        if budget == 1 { MAX_DRAFT_BYTES } else { 8 },
                        "Select all matching",
                        [ColumnId(1), ColumnId(3), ColumnId(4)].into_iter(),
                    );
                },
            );
            output.textures_delta.clear();
        }
        if budget == 0 {
            assert_eq!(columns, vec![ColumnId(3), ColumnId(1), ColumnId(4)]);
        } else {
            assert_eq!(columns, vec![ColumnId(3)]);
            assert_eq!(columns.capacity(), original_capacity);
            assert!(state.error().is_some());
        }
    }
}
