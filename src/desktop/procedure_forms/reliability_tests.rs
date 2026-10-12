use super::*;
use crate::{
    data::{Column, Dataset},
    project::Project,
};
#[test]
fn reliability_real_form_exposes_editable_items_and_reversal_bounds() {
    let data = Dataset::new(vec![
        Column::numeric(ColumnId(1), "First", vec![Some(1.), Some(2.)]).unwrap(),
        Column::numeric(ColumnId(2), "Second", vec![Some(2.), Some(3.)]).unwrap(),
    ])
    .unwrap();
    let mut state = DesktopState::from_project(Project::new("Reliability", data).unwrap()).unwrap();
    state = state
        .apply_action(UiAction::SetDraft(AnalysisDraft::from_request(
            AnalysisRequest::Reliability(ReliabilityRequest {
                items: vec![ColumnId(2), ColumnId(1)],
                filter: RowFilter::default(),
                frequency_weight: None,
                reversals: vec![ReliabilityReverse {
                    column: ColumnId(2),
                    lower: 0.,
                    upper: 5.,
                }],
            }),
        )))
        .unwrap();
    let ctx = egui::Context::default();
    let mut jobs = JobRunner::default();
    let mut out = ctx.run_ui(
        egui::RawInput {
            screen_rect: Some(egui::Rect::from_min_size(
                egui::Pos2::ZERO,
                egui::vec2(760., 900.),
            )),
            ..Default::default()
        },
        |ui| {
            egui::CentralPanel::default().show(ui, |ui| settings(ui, &mut state, &mut jobs));
        },
    );
    out.textures_delta.clear();
    for label in [
        "Add item",
        "Reverse item 2",
        "Lower bound: 2",
        "Upper bound: 2",
        "Move item 2 down",
        "Remove item 2",
    ] {
        assert!(
            state.control_rect(label).is_some(),
            "missing editable control {label}"
        );
    }
}

fn chooser_frame(
    ctx: &egui::Context,
    state: &mut DesktopState,
    jobs: &mut JobRunner,
    events: Vec<egui::Event>,
) {
    state.controls.clear();
    state.control_ids.clear();
    let mut out = ctx.run_ui(
        egui::RawInput {
            screen_rect: Some(egui::Rect::from_min_size(
                egui::Pos2::ZERO,
                egui::vec2(760., 1100.),
            )),
            events,
            ..Default::default()
        },
        |ui| {
            egui::CentralPanel::default().show(ui, |ui| analyses::settings(ui, state, jobs));
        },
    );
    out.textures_delta.clear();
}
fn chooser_click(ctx: &egui::Context, state: &mut DesktopState, jobs: &mut JobRunner, label: &str) {
    let pos = state
        .control_rect(label)
        .unwrap_or_else(|| panic!("missing chooser control {label}"))
        .center();
    for pressed in [true, false] {
        chooser_frame(
            ctx,
            state,
            jobs,
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
    chooser_frame(ctx, state, jobs, vec![]);
}
#[test]
fn reliability_chooser_selects_real_draft_and_submits_ordered_request() {
    let data = Dataset::new(vec![
        Column::numeric(ColumnId(1), "First", vec![Some(1.), Some(2.), Some(4.)]).unwrap(),
        Column::numeric(ColumnId(2), "Second", vec![Some(2.), Some(4.), Some(5.)]).unwrap(),
    ])
    .unwrap();
    let mut state = DesktopState::from_project(Project::new("Chooser", data).unwrap()).unwrap();
    let ctx = egui::Context::default();
    let mut jobs = JobRunner::default();
    chooser_frame(&ctx, &mut state, &mut jobs, vec![]);
    chooser_click(&ctx, &mut state, &mut jobs, "Search procedures");
    chooser_frame(
        &ctx,
        &mut state,
        &mut jobs,
        vec![egui::Event::Text("Scale reliability".into())],
    );
    chooser_frame(&ctx, &mut state, &mut jobs, vec![]);
    chooser_click(&ctx, &mut state, &mut jobs, "Scale reliability");
    assert!(matches!(state.draft, AnalysisDraft::Reliability(_)));
    for id in [2, 1] {
        chooser_click(&ctx, &mut state, &mut jobs, "Add item");
        chooser_click(&ctx, &mut state, &mut jobs, &format!("Add item: {id}"));
    }
    for _ in 0..3 {
        chooser_frame(&ctx, &mut state, &mut jobs, vec![]);
    }
    chooser_click(&ctx, &mut state, &mut jobs, "Run analysis");
    assert!(
        state.active,
        "request was not submitted: {:?}",
        state.error()
    );
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(10);
    while state.active && std::time::Instant::now() < deadline {
        state.poll(&mut jobs);
        std::thread::yield_now();
    }
    assert!(!state.active, "reliability worker did not complete");
    assert!(state.error().is_none(), "{:?}", state.error());
    let runs = state.project().runs();
    assert_eq!(runs.len(), 1);
    let CompletedAnalysis::Reliability(run) = &runs[0].analysis else {
        panic!("chooser submitted the wrong analysis family");
    };
    assert_eq!(run.request.items, [ColumnId(2), ColumnId(1)]);
    assert_eq!(run.request.filter, RowFilter::default());
    assert_eq!(run.request.frequency_weight, None);
    assert!(run.request.reversals.is_empty());
}
