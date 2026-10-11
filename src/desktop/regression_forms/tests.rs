use super::*;
use crate::{
    data::{Cell, Column, ColumnId, Dataset},
    project::Project,
};
use eframe::egui::{self, Event, Modifiers, PointerButton, Pos2, Rect};

struct Form {
    ctx: egui::Context,
    state: DesktopState,
    draft: RegressionDraft,
    active: Option<ColumnId>,
    lookup: Option<GroupLookupResult>,
}
impl Form {
    fn new() -> Self {
        let data = Dataset::new(vec![
            Column::numeric(ColumnId(1), "Y", vec![Some(1.), Some(2.)]).unwrap(),
            Column::numeric(ColumnId(2), "X", vec![Some(0.), Some(1.)]).unwrap(),
            Column::numeric(ColumnId(3), "Code", vec![Some(0.), Some(1.)]).unwrap(),
            Column::text(ColumnId(4), "Text", vec![Some("".into()), Some("ŵ".into())]).unwrap(),
        ])
        .unwrap();
        Self {
            ctx: egui::Context::default(),
            state: DesktopState::from_project(Project::new("Controls", data).unwrap()).unwrap(),
            draft: RegressionDraft::default(),
            active: None,
            lookup: None,
        }
    }
    fn frame(&mut self, events: Vec<Event>) {
        self.state.controls.clear();
        self.state.control_ids.clear();
        let mut out = self.ctx.run_ui(
            egui::RawInput {
                screen_rect: Some(Rect::from_min_size(Pos2::ZERO, egui::vec2(1180., 1000.))),
                events,
                ..Default::default()
            },
            |ui| {
                egui::CentralPanel::default().show(ui, |ui| {
                    settings(
                        ui,
                        &mut self.state,
                        &mut self.draft,
                        &mut self.active,
                        self.lookup.as_ref(),
                    )
                });
            },
        );
        out.textures_delta.clear();
    }
    fn click(&mut self, label: &str) {
        let pos = self
            .state
            .control_rect(label)
            .unwrap_or_else(|| panic!("missing {label}"))
            .center();
        for pressed in [true, false] {
            self.frame(vec![
                Event::PointerMoved(pos),
                Event::PointerButton {
                    pos,
                    button: PointerButton::Primary,
                    pressed,
                    modifiers: Modifiers::NONE,
                },
            ]);
        }
        self.frame(vec![]);
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
                    label: "same label".into(),
                })
                .collect(),
            selected_present: [true, false],
            more: false,
        });
    }
    fn add(&mut self, id: u64) {
        self.click("Add predictor");
        self.click(&format!("Add predictor: {id}"));
    }
    fn reference(&mut self, id: u64, choice: usize) {
        self.click(&format!("Reference level: {id}"));
        self.click(&format!("Reference choice: {id}: {choice}"));
    }
}
#[test]
fn real_controls_build_ordered_numeric_and_two_explicit_factors() {
    let mut f = Form::new();
    f.frame(vec![]);
    f.click("Outcome");
    f.click("Outcome: 1");
    f.add(2);
    f.add(3);
    f.add(4);
    f.click("Categorical: 3");
    assert!(f.draft.build_request(f.state.project().dataset()).is_err());
    f.lookup(
        3,
        vec![Cell::Number(0.), Cell::Number(1.), Cell::Number(1.)],
    );
    f.frame(vec![]);
    f.reference(3, 1);

    f.lookup(
        4,
        vec![
            Cell::Text("".into()),
            Cell::Text(" ".into()),
            Cell::Text("ŵ".into()),
        ],
    );
    f.frame(vec![]);
    f.reference(4, 2);

    assert_ne!(
        f.state.control_id("Reference level: 3"),
        f.state.control_id("Reference level: 4")
    );
    let request = f.draft.build_request(f.state.project().dataset()).unwrap();
    assert_eq!(
        request
            .predictors
            .iter()
            .map(|p| p.column.0)
            .collect::<Vec<_>>(),
        [2, 3, 4]
    );
    assert_eq!(
        request.predictors[1].role,
        PredictorRole::Categorical {
            reference: Cell::Number(1.)
        }
    );
    assert_eq!(
        request.predictors[2].role,
        PredictorRole::Categorical {
            reference: Cell::Text("ŵ".into())
        }
    );
    assert_eq!(super::super::draft::Procedure::ALL.len(), 12);
    f.click("Move down: 2");
    assert_eq!(
        f.draft
            .predictors
            .iter()
            .map(|p| p.column.0)
            .collect::<Vec<_>>(),
        [3, 2, 4]
    );
    f.click("Move up: 2");
    f.click("Move up: 4");
    assert_eq!(
        f.draft
            .predictors
            .iter()
            .map(|p| p.column.0)
            .collect::<Vec<_>>(),
        [2, 4, 3]
    );
    f.click("Numeric: 3");
    assert_eq!(f.draft.predictors[2].role, RegressionDraftRole::Numeric);
    f.click("Remove: 4");
    assert_eq!(f.active, None);
    assert_eq!(f.draft.predictors.len(), 2);
}
#[test]
fn raw_empty_whitespace_unicode_and_duplicate_labels_keep_typed_identity() {
    let mut f = Form::new();
    f.frame(vec![]);
    f.add(4);
    let values = vec![
        Cell::Text("".into()),
        Cell::Text(" ".into()),
        Cell::Text("ŵ".into()),
        Cell::Text("ŵ".into()),
    ];
    f.lookup(4, values.clone());
    f.frame(vec![]);
    for (i, value) in values.into_iter().enumerate() {
        f.reference(4, i);
        assert_eq!(
            f.draft.predictors[0].role,
            RegressionDraftRole::Categorical {
                reference: Some(value)
            }
        );
    }
}
#[test]
fn long_reference_copies_only_on_admitted_selection() {
    let mut f = Form::new();
    f.frame(vec![]);
    f.add(4);
    let long = "ŵ".repeat(80_000);
    f.lookup(4, vec![Cell::Text(long.clone())]);
    f.frame(vec![]);
    f.reference(4, 0);
    assert_eq!(
        f.draft.predictors[0].role,
        RegressionDraftRole::Categorical {
            reference: Some(Cell::Text(long))
        }
    );
    f.frame(vec![]);
    let (_, largest) = crate::data::measure_test_allocations(|| f.frame(vec![]));
    assert!(
        largest < 160_000,
        "reference copied during frame: {largest}"
    );
    f.lookup(
        4,
        vec![Cell::Text(
            "x".repeat(super::super::state::MAX_DRAFT_BYTES + 1),
        )],
    );
    f.frame(vec![]);
    f.reference(4, 0);
    assert!(f.state.error().is_some());
    let RegressionDraftRole::Categorical {
        reference: Some(Cell::Text(s)),
    } = &f.draft.predictors[0].role
    else {
        panic!()
    };
    assert_eq!(s.len(), 160_000);
}

#[test]
fn wrong_column_and_invalid_typed_choices_never_become_references() {
    let mut f = Form::new();
    f.frame(vec![]);
    f.add(4);
    f.lookup(3, vec![Cell::Number(0.)]);
    f.frame(vec![]);
    f.click("Reference level: 4");
    assert!(f.state.control_rect("Reference choice: 4: 0").is_none());
    f.click("Reference level: 4");
    for invalid in [Cell::Missing, Cell::Number(1.), Cell::Number(f64::NAN)] {
        f.lookup(4, vec![invalid]);
        f.frame(vec![]);
        f.reference(4, 0);
        assert_eq!(
            f.draft.predictors[0].role,
            RegressionDraftRole::Categorical { reference: None }
        );
        assert!(f.state.error().is_some());
    }
}
#[test]
fn reference_replacement_obeys_final_draft_bound_and_live_copy_headroom() {
    let mut f = Form::new();
    f.frame(vec![]);
    f.add(4);
    f.lookup(
        4,
        vec![
            Cell::Text("x".repeat(300_000)),
            Cell::Text("y".repeat(300_000)),
        ],
    );
    f.frame(vec![]);
    f.reference(4, 0);
    f.reference(4, 1);
    let RegressionDraftRole::Categorical {
        reference: Some(Cell::Text(s)),
    } = &f.draft.predictors[0].role
    else {
        panic!()
    };
    assert!(s.starts_with('y'));
    let pointer = s.as_ptr();
    f.click("Reference level: 4");
    f.state.active_recipe_bytes = crate::data::MAX_OWNED_BYTES;
    let (_, largest) = crate::data::measure_test_allocations(|| f.click("Reference choice: 4: 0"));
    assert!(
        largest < 300_000,
        "reference copied before refusal: {largest}"
    );
    let RegressionDraftRole::Categorical {
        reference: Some(Cell::Text(s)),
    } = &f.draft.predictors[0].role
    else {
        panic!()
    };
    assert_eq!(s.as_ptr(), pointer);
    assert!(f.state.error().is_some());
}
#[test]
fn predictor_growth_refuses_before_exceeding_draft_capacity_bound() {
    let mut f = Form::new();
    f.frame(vec![]);
    f.add(4);
    let size = std::mem::size_of::<RegressionPredictorDraft>();
    assert_eq!(f.draft.predictors.len(), f.draft.predictors.capacity());
    let len = super::super::state::MAX_DRAFT_BYTES - size * f.draft.predictors.capacity();
    f.draft.predictors[0].role = RegressionDraftRole::Categorical {
        reference: Some(Cell::Text("x".repeat(len))),
    };
    let capacity = f.draft.predictors.capacity();
    f.frame(vec![]);
    f.add(2);
    assert_eq!(f.draft.predictors.len(), 1);
    assert_eq!(f.draft.predictors.capacity(), capacity);
    assert!(f.state.error().is_some());
}

#[test]
fn bounded_predictor_menu_search_reaches_later_columns_through_real_input() {
    let mut f = Form::new();
    let data = Dataset::new(
        (1..=270)
            .map(|id| Column::numeric(ColumnId(id), &format!("Field {id}"), vec![]).unwrap())
            .collect(),
    )
    .unwrap();
    f.state = DesktopState::from_project(Project::new("Wide controls", data).unwrap()).unwrap();
    f.frame(vec![]);
    f.click("Add predictor");
    f.click("Search predictors");
    f.frame(vec![Event::Text("Field 270".into())]);
    f.frame(vec![]);
    f.click("Add predictor: 270");
    assert_eq!(
        f.draft.predictors,
        [RegressionPredictorDraft {
            column: ColumnId(270),
            role: RegressionDraftRole::Numeric
        }]
    );
}
