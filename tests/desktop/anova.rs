use super::*;
use tessera::{
    analysis::*,
    desktop::draft::{AnalysisDraft, Procedure},
};

#[test]
fn anova_captured_settings_can_be_edited_and_run_from_completed_chooser() {
    let data = Dataset::new(vec![
        Column::numeric(
            ColumnId(1),
            "Outcome",
            vec![Some(1.), Some(3.), Some(4.), Some(7.)],
        )
        .unwrap(),
        Column::text(
            ColumnId(2),
            "Factor",
            vec![
                Some(" A ".into()),
                Some(" A ".into()),
                Some("B".into()),
                Some("B".into()),
            ],
        )
        .unwrap(),
    ])
    .unwrap();
    let request = AnovaRequest {
        design: AnovaDesign::OneWay {
            outcome: ColumnId(1),
            factor: ColumnId(2),
        },
        confidence: ConfidenceLevel::P95,
        filter: RowFilter::default(),
        frequency_weight: None,
        planned_pairs: vec![],
        all_pairs: vec![],
    };
    let draft = AnalysisDraft::from_request(AnalysisRequest::Anova(request.clone()));
    assert!(draft.matches_saved(AnalysisRequestRef::Anova(&request)));
    assert_eq!(
        draft.build_request(&data).unwrap(),
        AnalysisRequest::Anova(request)
    );
    assert_eq!(
        draft
            .for_procedure(Procedure::Pearson)
            .for_procedure(Procedure::Anova)
            .procedure(),
        Procedure::Anova
    );
    assert!(Procedure::ALL.contains(&Procedure::Anova));
}

#[path = "../common/anova.rs"]
mod fixture;
fn frame(state: &mut DesktopState, jobs: &mut JobRunner, ctx: &egui::Context, events: Vec<Event>) {
    let mut out = ctx.run_ui(
        egui::RawInput {
            screen_rect: Some(Rect::from_min_size(Pos2::ZERO, Vec2::new(1180., 5000.))),
            events,
            ..Default::default()
        },
        |ui| render(ui, state, jobs),
    );
    out.textures_delta.clear();
}
fn click(state: &mut DesktopState, jobs: &mut JobRunner, ctx: &egui::Context, label: &str) {
    let pos = state
        .control_rect(label)
        .unwrap_or_else(|| panic!("control {label}"))
        .center();
    for pressed in [true, false] {
        frame(
            state,
            jobs,
            ctx,
            vec![
                Event::PointerMoved(pos),
                Event::PointerButton {
                    pos,
                    button: egui::PointerButton::Primary,
                    pressed,
                    modifiers: Modifiers::NONE,
                },
            ],
        );
    }
}
fn settle(state: &mut DesktopState, jobs: &mut JobRunner) {
    let until = Instant::now() + Duration::from_secs(10);
    while jobs.is_running() {
        state.poll(jobs);
        assert!(Instant::now() < until);
        std::thread::yield_now();
    }
}
#[test]
fn all_three_editable_designs_run_through_production_button_and_keyboard_with_frozen_history() {
    for design in 0..3 {
        let project = fixture::captured(design);
        let original = project.runs()[0].analysis.clone();
        let request = original.request().clone_owned();
        let mut state = DesktopState::from_project(project)
            .unwrap()
            .apply_action(UiAction::SetDraft(AnalysisDraft::from_request(
                request.clone(),
            )))
            .unwrap()
            .apply_action(UiAction::Navigate(Workspace::Analyses))
            .unwrap();
        let ctx = egui::Context::default();
        tessera::desktop::configure_context(&ctx);
        let mut jobs = JobRunner::default();
        frame(&mut state, &mut jobs, &ctx, vec![]);
        click(&mut state, &mut jobs, &ctx, "Run analysis");
        settle(&mut state, &mut jobs);
        assert_eq!(state.project().runs().len(), 2);
        assert_eq!(state.project().runs()[0].analysis, original);
        assert_eq!(state.project().runs()[1].analysis.request(), request);
        frame(
            &mut state,
            &mut jobs,
            &ctx,
            vec![Event::Key {
                key: Key::Enter,
                physical_key: None,
                pressed: true,
                repeat: false,
                modifiers: Modifiers::COMMAND,
            }],
        );
        settle(&mut state, &mut jobs);
        assert_eq!(state.project().runs().len(), 3);
        assert!(state
            .project()
            .runs()
            .iter()
            .all(|r| matches!(r.analysis, CompletedAnalysis::Anova(_))));
    }
}
#[test]
fn independent_anova_lookup_uses_existing_worker_and_rejects_previous_factor_epoch() {
    let mut state = DesktopState::from_project(fixture::captured(1))
        .unwrap()
        .apply_action(UiAction::SetDraft(AnalysisDraft::from_request(
            AnalysisRequest::Anova(fixture::input(1).1),
        )))
        .unwrap()
        .apply_action(UiAction::Navigate(Workspace::Analyses))
        .unwrap();
    state
        .set_active_regression_reference(Some(ColumnId(2)))
        .unwrap();
    let ctx = egui::Context::default();
    tessera::desktop::configure_context(&ctx);
    let mut jobs = JobRunner::default();
    frame(&mut state, &mut jobs, &ctx, vec![]);
    let old = state.token();
    let epoch = state.group_role_epoch();
    state
        .set_active_regression_reference(Some(ColumnId(3)))
        .unwrap();
    assert!(state.group_role_epoch() > epoch);
    state
        .handle_event(JobEvent::Completed {
            token: old,
            cancelled: false,
            result: Ok(JobOutput::GroupValues(Arc::new(GroupLookupResult {
                column: ColumnId(2),
                role_epoch: epoch,
                query: String::new(),
                choices: vec![GroupChoice {
                    value: Cell::Text("late".into()),
                    label: "late".into(),
                }],
                selected_present: [false; 2],
                more: false,
            }))),
        })
        .unwrap();
    assert!(state.group_lookup().is_none());
    settle(&mut state, &mut jobs);
    frame(&mut state, &mut jobs, &ctx, vec![]);
    settle(&mut state, &mut jobs);
    assert_eq!(state.group_lookup().unwrap().column, ColumnId(3));
    state.set_group_query("Y".into()).unwrap();
    assert!(state.group_lookup().is_none());
    frame(&mut state, &mut jobs, &ctx, vec![]);
    settle(&mut state, &mut jobs);
    assert_eq!(state.group_lookup().unwrap().query, "Y");
}
