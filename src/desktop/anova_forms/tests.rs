use super::comparisons::add_comparison;
use super::*;
use crate::{
    data::{Column, Dataset},
    project::Project,
};
use egui::{Event, Modifiers, Pos2, Rect};
struct Form {
    ctx: egui::Context,
    state: DesktopState,
    draft: AnovaDraft,
    active: Option<ColumnId>,
    lookup: Option<GroupLookupResult>,
    members_only: bool,
    width: f32,
}
impl Form {
    fn new() -> Self {
        let data = Dataset::new(vec![
            Column::numeric(ColumnId(1), "Y", vec![Some(1.), Some(2.)]).unwrap(),
            Column::numeric(ColumnId(2), "Numeric factor", vec![Some(0.), Some(1.)]).unwrap(),
            Column::text(
                ColumnId(3),
                "Text factor",
                vec![Some(" A ".into()), Some("B".into())],
            )
            .unwrap(),
            Column::text(
                ColumnId(4),
                "Subject",
                vec![Some("id1".into()), Some("id2".into())],
            )
            .unwrap(),
        ])
        .unwrap();
        Self {
            ctx: egui::Context::default(),
            state: DesktopState::from_project(Project::new("Forms", data).unwrap()).unwrap(),
            draft: AnovaDraft::default(),
            active: None,
            lookup: None,
            members_only: false,
            width: 1600.,
        }
    }
    fn frame(&mut self, events: Vec<Event>) -> egui::FullOutput {
        self.state.controls.clear();
        let mut out = self.ctx.run_ui(
            egui::RawInput {
                screen_rect: Some(Rect::from_min_size(
                    Pos2::ZERO,
                    egui::vec2(self.width, 5000.),
                )),
                events,
                ..Default::default()
            },
            |ui| {
                egui::CentralPanel::default().show(ui, |ui| {
                    if self.members_only {
                        super::comparisons::member_rows(ui, &mut self.state, &mut self.draft);
                    } else {
                        settings(
                            ui,
                            &mut self.state,
                            &mut self.draft,
                            &mut self.active,
                            self.lookup.as_ref(),
                        );
                    }
                });
            },
        );
        out.textures_delta.clear();
        out
    }
    fn click(&mut self, label: &str) {
        let pos = self
            .state
            .control_rect(label)
            .unwrap_or_else(|| panic!("missing control {label}"))
            .center();
        for pressed in [true, false] {
            self.frame(vec![
                Event::PointerMoved(pos),
                Event::PointerButton {
                    pos,
                    button: egui::PointerButton::Primary,
                    pressed,
                    modifiers: Modifiers::NONE,
                },
            ]);
        }
        self.frame(vec![]);
        self.frame(vec![]);
    }
    fn select(&mut self, label: &str, id: u64) {
        self.click(label);
        self.click(&format!("{label}: {id}"));
    }
    fn text(&mut self, label: &str, text: &str) {
        let id = self.state.control_id(label).unwrap();
        self.ctx.memory_mut(|m| m.request_focus(id));
        self.frame(vec![
            Event::Key {
                key: egui::Key::A,
                physical_key: None,
                pressed: true,
                repeat: false,
                modifiers: Modifiers::COMMAND,
            },
            Event::Text(text.into()),
        ]);
        self.frame(vec![]);
    }
    fn lookup(&mut self, column: u64, values: Vec<Cell>) {
        self.lookup = Some(GroupLookupResult {
            column: ColumnId(column),
            role_epoch: 0,
            query: String::new(),
            choices: values
                .into_iter()
                .map(|value| GroupChoice {
                    value,
                    label: "same display".into(),
                })
                .collect(),
            selected_present: [true; 2],
            more: false,
        });
    }
}
#[test]
fn real_one_way_factorial_roles_and_mode_edits_clear_incompatible_comparisons() {
    let mut f = Form::new();
    f.frame(vec![]);
    f.select("ANOVA outcome", 1);
    f.select("ANOVA factor", 3);
    assert!(f.draft.build_request(f.state.project().dataset()).is_ok());
    f.click("Factorial ANOVA");
    f.select("ANOVA outcome", 1);
    f.select("Add ANOVA factor", 2);
    f.select("Add ANOVA factor", 3);
    let r = f.draft.build_request(f.state.project().dataset()).unwrap();
    assert!(
        matches!(r.design,AnovaDesign::Factorial{factors,..} if factors==vec![ColumnId(2),ColumnId(3)])
    );
    f.draft.planned_pairs.push(AnovaPairRequest {
        factor: 1,
        first: Cell::Text(" A ".into()),
        second: Cell::Text("B".into()),
        scope: AnovaScope::Marginal,
    });
    f.click("Remove ANOVA factor 1");
    assert!(f.draft.planned_pairs.is_empty());
    f.draft.frequency_weight = Some(ColumnId(1));
    f.click("Repeated measures ANOVA");
    assert!(f.draft.frequency_weight.is_none());
    assert!(f.draft.build_request(f.state.project().dataset()).is_err());
}
#[test]
fn real_repeated_declared_labels_mapping_keyboard_and_no_column_name_inference() {
    let mut f = Form::new();
    f.frame(vec![]);
    f.click("Repeated measures ANOVA");
    f.select("Subject ID", 4);
    f.click("Add within factor");
    f.text("Within factor 1 name", " Time ");
    f.click("Add within level 1");
    f.text("Within factor 1 level 1", " A ");
    f.click("Add within level 1");
    f.text("Within factor 1 level 2", "A");
    f.select("Measurement 1", 1);
    f.select("Measurement 2", 2);
    let r = f.draft.build_request(f.state.project().dataset()).unwrap();
    let AnovaDesign::Repeated {
        factors,
        measurements,
        ..
    } = r.design
    else {
        panic!()
    };
    assert_eq!(factors[0].name, " Time ");
    assert_eq!(factors[0].levels, [" A ", "A"]);
    assert_eq!(measurements, [ColumnId(1), ColumnId(2)]);
    f.text("Within factor 1 level 2", "ŵ");
    let AnovaDesignDraft::Repeated { measurements, .. } = &f.draft.design else {
        panic!()
    };
    assert_eq!(measurements, &vec![None, None]);
    assert!(f.draft.build_request(f.state.project().dataset()).is_err());
}
#[test]
fn actual_lookup_pair_controls_keep_raw_types_direction_and_reject_reversed_duplicate() {
    let mut f = Form::new();
    f.frame(vec![]);
    f.select("ANOVA outcome", 1);
    f.select("ANOVA factor", 3);
    f.click("ANOVA comparisons");
    f.click("Discover comparison levels");
    f.lookup(
        3,
        vec![
            Cell::Text("A".into()),
            Cell::Text(" A ".into()),
            Cell::Text("ŵ".into()),
            Cell::Number(1.),
        ],
    );
    f.frame(vec![]);
    f.click("First ANOVA level");
    f.click("First ANOVA level choice 1");
    f.click("Second ANOVA level");
    f.click("Second ANOVA level choice 0");
    f.click("Add planned ANOVA pair");
    assert_eq!(f.draft.planned_pairs.len(), 1);
    assert_eq!(f.draft.planned_pairs[0].first, Cell::Text(" A ".into()));
    f.click("First ANOVA level");
    f.click("First ANOVA level choice 0");
    f.click("Second ANOVA level");
    f.click("Second ANOVA level choice 1");
    f.click("Add planned ANOVA pair");
    assert_eq!(f.draft.planned_pairs.len(), 1);
    assert!(f.state.error().is_some());
    f.click("Add all-pairs ANOVA family");
    f.click("Add all-pairs ANOVA family");
    assert_eq!(f.draft.all_pairs.len(), 1);
    f.click("First ANOVA level");
    f.click("First ANOVA level choice 3");
    assert_eq!(f.draft.endpoints[0], Some(Cell::Text("A".into())));
}
#[test]
fn fixed_slices_remain_separate_from_marginal_and_refuse_missing_levels() {
    let mut f = Form::new();
    f.draft.design = AnovaDesignDraft::Factorial {
        outcome: Some(ColumnId(1)),
        factors: vec![ColumnId(2), ColumnId(3)],
    };
    f.draft.endpoints = [Some(Cell::Number(0.)), Some(Cell::Number(1.))];
    add_comparison(&f.state, &mut f.draft, true).unwrap();
    f.draft.fixed = Some(vec![None]);
    assert!(add_comparison(&f.state, &mut f.draft, true).is_err());
    f.draft.fixed = Some(vec![Some(Cell::Text(" A ".into()))]);
    add_comparison(&f.state, &mut f.draft, true).unwrap();
    assert_eq!(f.draft.planned_pairs.len(), 2);
    assert!(matches!(
        f.draft.planned_pairs[0].scope,
        AnovaScope::Marginal
    ));
    assert!(matches!(
        f.draft.planned_pairs[1].scope,
        AnovaScope::Fixed(_)
    ));
}
#[test]
fn pair_growth_and_long_endpoint_copy_refuse_before_allocating_at_live_limit() {
    let mut f = Form::new();
    f.draft.design = AnovaDesignDraft::OneWay {
        outcome: Some(ColumnId(1)),
        factor: Some(ColumnId(3)),
    };
    f.draft.endpoints = [
        Some(Cell::Text("long".repeat(75000))),
        Some(Cell::Text("B".into())),
    ];
    f.state.active_recipe_bytes = crate::data::MAX_OWNED_BYTES;
    let (r, largest) =
        crate::data::measure_test_allocations(|| add_comparison(&f.state, &mut f.draft, true));
    assert!(matches!(r, Err(AppError::SizeLimit { .. })));
    assert_eq!(largest, 0);
    assert!(f.draft.planned_pairs.is_empty());
}

#[test]
fn captured_long_declaration_remains_exact_until_real_explicit_replacement() {
    let mut f = Form::new();
    f.draft.design = AnovaDesignDraft::Repeated {
        subject: Some(ColumnId(4)),
        factors: vec![AnovaWithinFactor {
            name: "F".repeat(300000),
            levels: vec!["A".into(), "B".into()],
        }],
        measurements: vec![Some(ColumnId(1)), Some(ColumnId(2))],
    };
    let original = f.draft.clone();
    f.frame(vec![]);
    assert_eq!(f.draft, original);
    assert!(f.state.control_rect("Within factor 1 name").is_none());
    assert!(f
        .state
        .control_rect("Replace Within factor 1 name")
        .is_some());
    f.click("Replace Within factor 1 name");
    let AnovaDesignDraft::Repeated {
        factors,
        measurements,
        ..
    } = &f.draft.design
    else {
        panic!()
    };
    assert!(factors[0].name.is_empty());
    assert_eq!(measurements, &vec![None, None]);
    f.text("Within factor 1 name", "Replacement");
    let AnovaDesignDraft::Repeated { factors, .. } = &f.draft.design else {
        panic!()
    };
    assert_eq!(factors[0].name, "Replacement");
}

#[test]
fn virtual_declared_levels_reach_last_row_and_edit_by_keyboard() {
    let mut f = Form::new();
    f.draft.design = AnovaDesignDraft::Repeated {
        subject: Some(ColumnId(4)),
        factors: vec![AnovaWithinFactor {
            name: "Condition".into(),
            levels: (0..64).map(|i| format!("L{i:02}")).collect(),
        }],
        measurements: vec![None; 64],
    };
    f.frame(vec![]);
    assert!(f.state.control_rect("Within factor 1 level 64").is_none());
    let pos = f
        .state
        .control_rect("Within factor 1 level 1")
        .unwrap()
        .center();
    f.frame(vec![
        Event::PointerMoved(pos),
        Event::MouseWheel {
            phase: egui::TouchPhase::Move,
            unit: egui::MouseWheelUnit::Point,
            delta: egui::vec2(0., -10000.),
            modifiers: Modifiers::NONE,
        },
    ]);
    f.frame(vec![]);
    f.frame(vec![]);
    assert!(f.state.control_rect("Within factor 1 level 64").is_some());
    f.text("Within factor 1 level 64", " Last ŵ ");
    let AnovaDesignDraft::Repeated { factors, .. } = &f.draft.design else {
        panic!()
    };
    assert_eq!(factors[0].levels[63], " Last ŵ ");
    assert_eq!(factors[0].levels[0], "L00");
}

#[test]
fn nested_mapping_viewport_keeps_complete_controls_visible_and_reaches_last_row() {
    let mut f = Form::new();
    f.draft.design = AnovaDesignDraft::Repeated {
        subject: Some(ColumnId(4)),
        factors: vec![
            AnovaWithinFactor {
                name: "Time".into(),
                levels: vec!["Before".into(), "After".into()],
            },
            AnovaWithinFactor {
                name: "Condition".into(),
                levels: vec!["A".into(), "B".into(), "C".into()],
            },
        ],
        measurements: vec![None; 6],
    };
    let mut offset = 0.;
    let mut events = vec![];
    let mut seen = std::collections::BTreeSet::new();
    for step in 0..40 {
        f.state.controls.clear();
        let mut out = f.ctx.run_ui(
            egui::RawInput {
                screen_rect: Some(Rect::from_min_size(Pos2::ZERO, egui::vec2(1180., 780.))),
                events: std::mem::take(&mut events),
                ..Default::default()
            },
            |ui| {
                egui::CentralPanel::default().show(ui, |ui| {
                    ui.set_width(288.);
                    egui::ScrollArea::vertical()
                        .id_salt("mapping-test-outer")
                        .max_height(400.)
                        .vertical_scroll_offset(offset)
                        .show(ui, |ui| {
                            settings(
                                ui,
                                &mut f.state,
                                &mut f.draft,
                                &mut f.active,
                                f.lookup.as_ref(),
                            );
                        });
                });
            },
        );
        out.textures_delta.clear();
        if step == 0 {
            offset = f.state.control_rect("Measurement 1").unwrap().top() - 100.;
            continue;
        }
        let mut hover = None;
        for i in 1..=6 {
            if let Some(rect) = f.state.control_rect(&format!("Measurement {i}")) {
                // Rendering and hit testing must agree: a recorded rectangle alone may be clipped.
                let visible = out.shapes.iter().any(|s| match &s.shape {
                    egui::epaint::Shape::Text(t) => {
                        t.galley.text() == "Select variable"
                            && rect.contains(t.pos)
                            && s.clip_rect.contains(t.pos + t.galley.size() * 0.5)
                    }
                    _ => false,
                });
                if visible {
                    seen.insert(i);
                    hover = hover.or(Some(rect.center()));
                }
            }
        }
        if step == 2 {
            assert!(
                seen.contains(&1),
                "first mapping selector clipped by nested viewport"
            );
        }
        if step >= 3 {
            if let Some(pos) = hover {
                events = vec![
                    Event::PointerMoved(pos),
                    Event::MouseWheel {
                        phase: egui::TouchPhase::Move,
                        unit: egui::MouseWheelUnit::Point,
                        delta: egui::vec2(0., -500.),
                        modifiers: Modifiers::NONE,
                    },
                ];
            }
        }
    }
    assert_eq!(
        seen,
        (1..=6).collect(),
        "every mapping selector must be physically visible through inner scrolling"
    );
}

fn keyboard_page_to_last(f: &mut Form, next: &str, last: &str, text: &str) {
    let original = f.draft.clone();
    let mut reached = false;
    for _ in 0..1600 {
        let focused = f.ctx.memory(|m| m.focused());
        let key = if focused.is_some() && focused == f.state.control_id(next) {
            egui::Key::Enter
        } else {
            egui::Key::Tab
        };
        let out = f.frame(vec![Event::Key {
            key,
            physical_key: None,
            pressed: true,
            repeat: false,
            modifiers: Modifiers::NONE,
        }]);
        if let Some(rect) = f.state.control_rect(last) {
            reached = out.shapes.iter().any(|s| match &s.shape {
                egui::epaint::Shape::Text(t) => {
                    t.galley.text() == text
                        && rect.contains(t.pos)
                        && s.clip_rect
                            .contains(t.pos + egui::vec2(2., t.galley.size().y * 0.5))
                }
                _ => false,
            });
        }
        f.frame(vec![Event::Key {
            key,
            physical_key: None,
            pressed: false,
            repeat: false,
            modifiers: Modifiers::NONE,
        }]);
        if reached {
            break;
        }
    }
    assert!(
        reached,
        "keyboard pages must reach physically visible {last}"
    );
    assert_eq!(
        f.draft, original,
        "navigation must preserve exact draft identities"
    );
}

#[test]
fn declared_levels_keyboard_tab_enter_reaches_last_exact_row_without_pointer_or_focus_assignment() {
    let mut f = Form::new();
    f.draft.design = AnovaDesignDraft::Repeated {
        subject: Some(ColumnId(4)),
        factors: vec![AnovaWithinFactor {
            name: "Condition".into(),
            levels: (0..64).map(|i| format!(" L{i:02} ŵ ")).collect(),
        }],
        measurements: vec![None; 64],
    };
    f.frame(vec![]);
    keyboard_page_to_last(
        &mut f,
        "Next within factor 1 levels page",
        "Within factor 1 level 64",
        " L63 ŵ ",
    );
}

#[test]
fn measurement_keyboard_tab_enter_reaches_last_exact_mapping_without_pointer_or_focus_assignment() {
    let mut f = Form::new();
    f.draft.design = AnovaDesignDraft::Repeated {
        subject: Some(ColumnId(4)),
        factors: vec![AnovaWithinFactor {
            name: "Condition".into(),
            levels: (0..64).map(|i| format!("L{i:02}")).collect(),
        }],
        measurements: vec![None; 64],
    };
    f.frame(vec![]);
    keyboard_page_to_last(
        &mut f,
        "Next measurements page",
        "Measurement 64",
        "Select variable",
    );
}

#[test]
fn comparisons_keyboard_tab_enter_reaches_last_member_without_pointer_or_focus_assignment() {
    for planned in [true, false] {
        let mut f = Form::new();
        f.members_only = true;
        f.width = 288.;
        if planned {
            f.draft.planned_pairs = (0..64)
                .map(|i| AnovaPairRequest {
                    factor: 0,
                    first: Cell::Text(format!(" P{i} ŵ ")),
                    second: Cell::Text("Other".into()),
                    scope: AnovaScope::Marginal,
                })
                .collect();
        } else {
            f.draft.all_pairs = (0..64)
                .map(|i| AnovaFamilyRequest {
                    factor: i,
                    scope: AnovaScope::Marginal,
                })
                .collect();
        }
        f.frame(vec![]);
        let (next, last) = if planned {
            (
                "Next planned comparisons page",
                "Remove planned ANOVA pair 64",
            )
        } else {
            (
                "Next all-pairs comparisons page",
                "Remove all-pairs ANOVA family 64",
            )
        };
        keyboard_page_to_last(&mut f, next, last, last);
    }
}

#[test]
fn keyboard_navigation_of_admitted_six_factor_draft_preserves_capacities_without_admission_errors()
{
    let mut f = Form::new();
    f.draft.design = AnovaDesignDraft::Repeated {
        subject: Some(ColumnId(4)),
        factors: (0..6)
            .map(|i| AnovaWithinFactor {
                name: format!("F{i}{}", "N".repeat(510)),
                levels: (0..64)
                    .map(|j| format!("{i}{j:02}{}", "L".repeat(509)))
                    .collect(),
            })
            .collect(),
        measurements: vec![],
    };
    f.draft.filter.predicates = vec![RowPredicate::Text {
        column: ColumnId(3),
        value: "X".repeat(300000),
        equal: true,
    }];
    let before = f.draft.heap_bytes().unwrap();
    assert!(before <= MAX_DRAFT_BYTES);
    f.frame(vec![]);
    assert_eq!(
        f.draft.heap_bytes().unwrap(),
        before,
        "browsing must not reserve maximum edit capacity"
    );
    assert!(
        f.state.error().is_none(),
        "admitted identity must remain browsable"
    );
    keyboard_page_to_last(
        &mut f,
        "Next within factor 6 levels page",
        "Within factor 6 level 64",
        &format!("563{}", "L".repeat(509)),
    );
    assert_eq!(f.draft.heap_bytes().unwrap(), before);
    assert!(
        f.state.error().is_none(),
        "page navigation must not cause admission errors"
    );
}

#[test]
fn refused_widget_selected_paste_and_following_undo_redo_preserve_exact_dependencies() {
    let mut f = Form::new();
    f.draft.design = AnovaDesignDraft::Repeated {
        subject: Some(ColumnId(4)),
        factors: vec![AnovaWithinFactor {
            name: "F".into(),
            levels: vec!["A".into(), "B".into()],
        }],
        measurements: vec![Some(ColumnId(1)), Some(ColumnId(2))],
    };
    f.draft.planned_pairs = vec![AnovaPairRequest {
        factor: 0,
        first: Cell::Text("A".into()),
        second: Cell::Text("B".into()),
        scope: AnovaScope::Marginal,
    }];
    f.draft.filter.predicates = vec![RowPredicate::Text {
        column: ColumnId(3),
        value: String::new(),
        equal: true,
    }];
    let fill = MAX_DRAFT_BYTES - f.draft.heap_bytes().unwrap();
    let RowPredicate::Text { value, .. } = &mut f.draft.filter.predicates[0] else {
        panic!()
    };
    *value = "X".repeat(fill);
    assert_eq!(f.draft.heap_bytes().unwrap(), MAX_DRAFT_BYTES);
    let original = f.draft.clone();
    f.frame(vec![]);
    f.frame(vec![]);
    let id = f.state.control_id("Within factor 1 level 1").unwrap();
    f.ctx.memory_mut(|memory| memory.request_focus(id));
    f.frame(vec![
        Event::Key {
            key: egui::Key::A,
            physical_key: None,
            pressed: true,
            repeat: false,
            modifiers: Modifiers::COMMAND,
        },
        Event::Paste("Longer ŵ".into()),
    ]);
    f.frame(vec![Event::Key {
        key: egui::Key::A,
        physical_key: None,
        pressed: false,
        repeat: false,
        modifiers: Modifiers::COMMAND,
    }]);
    assert!(f.state.error().is_some());
    assert_eq!(f.draft, original);
    for modifiers in [
        Modifiers::COMMAND,
        Modifiers {
            shift: true,
            ..Modifiers::COMMAND
        },
    ] {
        f.frame(vec![Event::Key {
            key: egui::Key::Z,
            physical_key: None,
            pressed: true,
            repeat: false,
            modifiers,
        }]);
        f.frame(vec![Event::Key {
            key: egui::Key::Z,
            physical_key: None,
            pressed: false,
            repeat: false,
            modifiers,
        }]);
        assert_eq!(
            f.draft, original,
            "undo/redo must not resurrect refused partial deletion"
        );
        assert_eq!(f.draft.heap_bytes().unwrap(), MAX_DRAFT_BYTES);
    }
}

#[test]
fn anova_text_identity_survives_status_and_preceding_layout_changes() {
    let mut f = Form::new();
    let mut text = String::from("ŵ factor");
    let mut heap = text.capacity();
    let mut prior = None;
    for (index, prefix) in [false, true, false].into_iter().enumerate() {
        f.state.controls.clear();
        let mut output = f.ctx.run_ui(
            egui::RawInput {
                events: if index == 0 {
                    vec![]
                } else {
                    vec![Event::Text("x".into())]
                },
                ..Default::default()
            },
            |ui| {
                egui::CentralPanel::default().show(ui, |ui| {
                    if prefix {
                        ui.label("Validation changed");
                    }
                    edit_text(ui, &mut f.state, &mut heap, "Factor name", &mut text);
                });
            },
        );
        output.textures_delta.clear();
        let id = f.state.control_id("Factor name").unwrap();
        if let Some(prior) = prior {
            assert_eq!(id, prior);
            assert_eq!(f.ctx.memory(|memory| memory.focused()), Some(id));
        } else {
            f.ctx.memory_mut(|memory| memory.request_focus(id));
        }
        prior = Some(id);
    }
    assert_eq!(
        text, "ŵ factorxx",
        "Typing must continue in the admitted buffer across layout changes"
    );
}
