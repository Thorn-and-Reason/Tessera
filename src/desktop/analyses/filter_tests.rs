use super::*;
use crate::data::ColumnId;

fn frame(
    ctx: &egui::Context,
    predicate: &mut RowPredicate,
    events: Vec<egui::Event>,
) -> egui::FullOutput {
    let mut out = ctx.run_ui(
        egui::RawInput {
            events,
            ..Default::default()
        },
        |ui| {
            egui::CentralPanel::default()
                .show(ui, |ui| edit_filter(ui, predicate, ColumnKind::Text));
        },
    );
    out.textures_delta.clear();
    out
}

#[test]
fn captured_filter_identity_is_preserved_until_explicit_replacement() {
    let ctx = egui::Context::default();
    let mut predicate = RowPredicate::Text {
        column: ColumnId(7),
        value: " ŵ ".repeat(100000),
        equal: false,
    };
    let before = predicate.clone();
    let out = frame(&ctx, &mut predicate, vec![]);
    assert_eq!(predicate, before);
    let pos = out
        .shapes
        .iter()
        .find_map(|s| match &s.shape {
            egui::epaint::Shape::Text(t) if t.galley.text() == "Replace saved filter value" => {
                Some(t.pos + egui::vec2(10., 5.))
            }
            _ => None,
        })
        .expect("explicit replacement control");
    for pressed in [true, false] {
        frame(
            &ctx,
            &mut predicate,
            vec![
                egui::Event::PointerMoved(pos),
                egui::Event::PointerButton {
                    pos,
                    button: egui::PointerButton::Primary,
                    pressed,
                    modifiers: egui::Modifiers::NONE,
                },
            ],
        );
    }
    assert_eq!(
        predicate,
        RowPredicate::Text {
            column: ColumnId(7),
            value: String::new(),
            equal: false
        }
    );
}

#[test]
fn admitted_short_saved_filter_values_remain_exact() {
    for value in [" ŵ ".to_owned(), "ŵ".repeat(256)] {
        let ctx = egui::Context::default();
        let mut predicate = RowPredicate::Text {
            column: ColumnId(7),
            value,
            equal: true,
        };
        let before = predicate.clone();
        frame(&ctx, &mut predicate, vec![]);
        assert_eq!(predicate, before);
    }
}

#[test]
fn filter_keyboard_tab_enter_reaches_last_exact_condition_without_pointer_or_focus_assignment() {
    use crate::{
        analysis::RowFilter,
        data::{Column, Dataset},
        project::Project,
    };
    let project = Project::new(
        "Keyboard filters",
        Dataset::new(vec![Column::text(
            ColumnId(7),
            "Text",
            vec![Some("x".into())],
        )
        .unwrap()])
        .unwrap(),
    )
    .unwrap();
    let mut state = DesktopState::from_project(project.clone()).unwrap();
    let ctx = egui::Context::default();
    let mut filter = RowFilter {
        predicates: (0..256)
            .map(|i| RowPredicate::Text {
                column: ColumnId(7),
                value: format!(" F{i:03} ŵ "),
                equal: true,
            })
            .collect(),
    };
    let before = filter.clone();
    let mut weight = None;
    let mut reached = false;
    for _ in 0..2400 {
        let focused = ctx.memory(|m| m.focused());
        let key = if focused.is_some() && focused == state.control_id("Next filter conditions page")
        {
            egui::Key::Enter
        } else {
            egui::Key::Tab
        };
        let mut out = ctx.run_ui(
            egui::RawInput {
                screen_rect: Some(egui::Rect::from_min_size(
                    egui::Pos2::ZERO,
                    egui::vec2(1180., 780.),
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
                egui::CentralPanel::default().show(ui, |ui| {
                    common_settings(ui, &project, &mut filter, &mut weight).record(&mut state)
                });
            },
        );
        out.textures_delta.clear();
        reached = out.shapes.iter().any(|s| match &s.shape {
            egui::epaint::Shape::Text(t) => {
                t.galley.text().contains("F255")
                    && s.clip_rect.contains(t.pos + t.galley.size() * 0.5)
            }
            _ => false,
        });
        let mut release = ctx.run_ui(
            egui::RawInput {
                screen_rect: Some(egui::Rect::from_min_size(
                    egui::Pos2::ZERO,
                    egui::vec2(1180., 780.),
                )),
                events: vec![egui::Event::Key {
                    key,
                    physical_key: None,
                    pressed: false,
                    repeat: false,
                    modifiers: egui::Modifiers::NONE,
                }],
                ..Default::default()
            },
            |ui| {
                egui::CentralPanel::default().show(ui, |ui| {
                    common_settings(ui, &project, &mut filter, &mut weight).record(&mut state)
                });
            },
        );
        release.textures_delta.clear();
        if reached {
            break;
        }
    }
    assert!(
        reached,
        "keyboard pages must reach physically visible final filter value"
    );
    assert_eq!(filter, before);
    assert_eq!(weight, None);
}

#[test]
fn shared_paged_popups_reach_final_numeric_weight_and_typed_filter_column() {
    use crate::{
        data::{Column, Dataset},
        project::Project,
    };
    use egui::{Event, Key, Modifiers, PointerButton};
    let mut columns = (1..20)
        .map(|i| Column::numeric(ColumnId(i), &format!("Column {i}"), vec![Some(1.)]).unwrap())
        .collect::<Vec<_>>();
    columns.push(Column::text(ColumnId(20), "Final text", vec![Some("x".into())]).unwrap());
    let project = Project::new("Popup paging", Dataset::new(columns).unwrap()).unwrap();
    let mut state = DesktopState::from_project(project.clone()).unwrap();
    let ctx = egui::Context::default();
    let mut filter = crate::analysis::RowFilter::default();
    let mut weight = None;
    let render = |events,
                  filter: &mut crate::analysis::RowFilter,
                  weight: &mut Option<ColumnId>,
                  state: &mut DesktopState| {
        state.controls.clear();
        state.control_ids.clear();
        let mut o = ctx.run_ui(
            egui::RawInput {
                screen_rect: Some(egui::Rect::from_min_size(
                    egui::Pos2::ZERO,
                    egui::vec2(760., 780.),
                )),
                events,
                ..Default::default()
            },
            |ui| {
                egui::CentralPanel::default().show(ui, |ui| {
                    common_settings(ui, &project, filter, weight).record(state)
                });
            },
        );
        o.textures_delta.clear();
        o
    };
    let locate = |o: &egui::FullOutput, label: &str| {
        o.shapes.iter().find_map(|s| {
            if let egui::Shape::Text(t) = &s.shape {
                if t.galley.text() == label
                    && s.clip_rect
                        .intersects(egui::Rect::from_min_size(t.pos, t.galley.size()))
                {
                    Some(t.pos + t.galley.size() * 0.5)
                } else {
                    None
                }
            } else {
                None
            }
        })
    };
    let mut out = render(vec![], &mut filter, &mut weight, &mut state);
    for (opener, pager, target) in [
        ("None", "Next frequency weight columns page", "Column 19"),
        ("Add condition", "Next filter columns page", "Final text"),
    ] {
        let pos = locate(&out, opener).unwrap_or_else(|| panic!("missing opener {opener}"));
        for pressed in [true, false] {
            out = render(
                vec![
                    Event::PointerMoved(pos),
                    Event::PointerButton {
                        pos,
                        button: PointerButton::Primary,
                        pressed,
                        modifiers: Modifiers::NONE,
                    },
                ],
                &mut filter,
                &mut weight,
                &mut state,
            );
        }
        for _ in 0..20 {
            if locate(&out, target).is_some() {
                break;
            }
            let id = state.control_id(pager).expect("real page control");
            ctx.memory_mut(|m| m.request_focus(id));
            for pressed in [true, false] {
                out = render(
                    vec![Event::Key {
                        key: Key::Enter,
                        physical_key: None,
                        pressed,
                        repeat: false,
                        modifiers: Modifiers::NONE,
                    }],
                    &mut filter,
                    &mut weight,
                    &mut state,
                );
            }
        }
        let pos = locate(&out, target).expect("last catalog column physically visible");
        let laid_out=out.shapes.iter().filter(|s|matches!(&s.shape,egui::Shape::Text(t) if t.galley.text().starts_with("Column "))).count();
        assert!(laid_out <= 10, "popup laid out entire catalog: {laid_out}");
        for pressed in [true, false] {
            let _ = render(
                vec![
                    Event::PointerMoved(pos),
                    Event::PointerButton {
                        pos,
                        button: PointerButton::Primary,
                        pressed,
                        modifiers: Modifiers::NONE,
                    },
                ],
                &mut filter,
                &mut weight,
                &mut state,
            );
        }
        out = render(
            vec![Event::Key {
                key: Key::Escape,
                physical_key: None,
                pressed: true,
                repeat: false,
                modifiers: Modifiers::NONE,
            }],
            &mut filter,
            &mut weight,
            &mut state,
        );
    }
    assert_eq!(weight, Some(ColumnId(19)));
    assert_eq!(
        filter.predicates,
        vec![RowPredicate::Text {
            column: ColumnId(20),
            value: String::new(),
            equal: true
        }]
    );
    assert_eq!(project.dataset().row_count(), 1);
}
