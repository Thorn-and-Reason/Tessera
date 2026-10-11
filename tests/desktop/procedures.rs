use super::*;
use tessera::{
    analysis::*,
    desktop::draft::{AnalysisDraft, Procedure},
};
#[test]
fn chooser_exposes_only_complete_procedures() {
    assert_eq!(Procedure::ALL.len(), 12);
    assert!(Procedure::ALL.contains(&Procedure::Anova));
    assert_eq!(Procedure::Anova.family(), "Comparisons");
    assert!(Procedure::ALL.contains(&Procedure::Regression));
    assert_eq!(Procedure::Regression.family(), "Relationships");
    let mut state = state();
    for procedure in Procedure::ALL {
        state = state
            .apply_action(UiAction::SetDraft(state.draft().for_procedure(procedure)))
            .unwrap();
        assert_eq!(state.draft().procedure(), procedure);
    }
}
#[test]
fn scalar_roles_never_commit_duplicate_or_zero_ids() {
    let state = state();
    for role in [None, Some(ColumnId(0)), Some(ColumnId(2))] {
        let draft = AnalysisDraft::PairedT {
            first: role,
            second: role,
            confidence: ConfidenceLevel::P95,
            filter: RowFilter::default(),
            frequency_weight: None,
        };
        assert!(state
            .apply_action(UiAction::SetDraft(draft))
            .unwrap()
            .checked_draft_request()
            .is_err());
    }
}
#[test]
fn selected_output_keeps_original_settings() {
    let mut state = state();
    let request = OneSampleTRequest {
        variable: ColumnId(1),
        reference_mean: -17.25,
        confidence: ConfidenceLevel::P99,
        filter: RowFilter::default(),
        frequency_weight: None,
    };
    let result = analyze_with_budget(
        state.project().dataset(),
        &AnalysisRequest::OneSampleT(request.clone()),
        &JobControl::default(),
        &[],
        4096,
    )
    .unwrap();
    state
        .handle_event(JobEvent::Completed {
            token: state.token(),
            cancelled: false,
            result: Ok(JobOutput::Analyzed(result)),
        })
        .unwrap();
    state = state
        .apply_action(UiAction::SetDraft(
            state.draft().for_procedure(Procedure::Pearson),
        ))
        .unwrap();
    assert_eq!(
        state.selected_run().unwrap().analysis.request(),
        AnalysisRequestRef::OneSampleT(&request)
    );
}
#[test]
fn lookup_completion_after_query_role_or_selected_value_change_is_discarded() {
    let mut state = state();
    let draft = AnalysisDraft::IndependentT {
        outcome: Some(ColumnId(1)),
        group: Some(ColumnId(2)),
        groups: [None, None],
        method: IndependentMethod::Welch,
        confidence: ConfidenceLevel::P95,
        filter: RowFilter::default(),
        frequency_weight: None,
    };
    state = state.apply_action(UiAction::SetDraft(draft)).unwrap();
    state.set_group_query("Carys".into()).unwrap();
    let old = GroupLookupResult {
        column: ColumnId(2),
        role_epoch: state.group_role_epoch(),
        query: "Carys".into(),
        choices: vec![],
        selected_present: [false; 2],
        more: false,
    };
    state.set_group_query("".into()).unwrap();
    state
        .handle_event(JobEvent::Completed {
            token: state.token(),
            cancelled: false,
            result: Ok(JobOutput::GroupValues(Arc::new(old))),
        })
        .unwrap();
    assert!(state.group_lookup().is_none());
}
#[test]
fn new_forms_keyboard_and_narrow_settings() {
    for procedure in Procedure::ALL {
        let ctx = egui::Context::default();
        tessera::desktop::configure_context(&ctx);
        let size = Vec2::new(760.0, 520.0);
        let mut state = state();
        state = state
            .apply_action(UiAction::Navigate(Workspace::Analyses))
            .unwrap();
        state = state
            .apply_action(UiAction::SetDraft(state.draft().for_procedure(procedure)))
            .unwrap();
        let mut jobs = JobRunner::default();
        frame(&ctx, &mut state, &mut jobs, size, vec![]);
        click(&ctx, &mut state, &mut jobs, "Settings", size);
        assert_eq!(state.dialog(), Some(Dialog::Settings));
        frame(&ctx, &mut state, &mut jobs, size, vec![]);
        assert!(state.control_rect("Procedure").is_some());
        frame(
            &ctx,
            &mut state,
            &mut jobs,
            size,
            vec![Event::Key {
                key: Key::Escape,
                physical_key: None,
                pressed: true,
                repeat: false,
                modifiers: Modifiers::NONE,
            }],
        );
        assert_eq!(state.dialog(), None);
        assert_eq!(ctx.memory(|m| m.focused()), state.control_id("Settings"));
    }
}

#[test]
fn native_chooser_and_scalar_form_commit_real_roles_and_run() {
    let ctx = egui::Context::default();
    tessera::desktop::configure_context(&ctx);
    let size = Vec2::new(1180.0, 780.0);
    let mut state = state();
    state = state
        .apply_action(UiAction::Navigate(Workspace::Analyses))
        .unwrap();
    let mut jobs = JobRunner::default();
    frame(&ctx, &mut state, &mut jobs, size, vec![]);
    click(&ctx, &mut state, &mut jobs, "Procedure", size);
    frame(&ctx, &mut state, &mut jobs, size, vec![]);
    for p in Procedure::ALL {
        assert!(state.control_rect(p.label()).is_some());
    }
    // Family headers and all twelve entries can exceed the popup viewport; use its actual search.
    click(&ctx, &mut state, &mut jobs, "Search procedures", size);
    frame(
        &ctx,
        &mut state,
        &mut jobs,
        size,
        vec![Event::Text("One-sample".into())],
    );
    for _ in 0..2 {
        frame(&ctx, &mut state, &mut jobs, size, vec![]);
    }
    let result = state
        .control_rect("One-sample t-test")
        .expect("search must retain its open popup and matching procedure");
    assert!(
        result.min.y >= 0. && result.max.y <= size.y,
        "matching procedure is visibly reachable: {result:?}"
    );
    assert!(
        state.control_rect("ANOVA").is_none(),
        "search filters the actual procedure menu"
    );
    click(&ctx, &mut state, &mut jobs, "One-sample t-test", size);
    assert_eq!(state.draft().procedure(), Procedure::OneSampleT);
    frame(&ctx, &mut state, &mut jobs, size, vec![]);
    click(&ctx, &mut state, &mut jobs, "Variable", size);
    frame(&ctx, &mut state, &mut jobs, size, vec![]);
    assert!(state.control_rect("Variable: 1").is_some());
    assert!(state.control_rect("Variable: 2").is_none());
    click(&ctx, &mut state, &mut jobs, "Variable: 1", size);
    let AnalysisDraft::OneSampleT { variable, .. } = state.draft() else {
        panic!()
    };
    assert_eq!(*variable, Some(ColumnId(1)));
    click(&ctx, &mut state, &mut jobs, "Run analysis", size);
    let until = Instant::now() + Duration::from_secs(5);
    while jobs.is_running() {
        state.poll(&mut jobs);
        assert!(Instant::now() < until);
        std::thread::yield_now();
    }
    assert_eq!(state.project().runs().len(), 1);
    assert!(matches!(
        state.selected_run().unwrap().analysis,
        CompletedAnalysis::OneSampleT(_)
    ));
}

#[test]
fn invalid_reference_shortcut_does_not_append_or_replace_output() {
    let mut state = state();
    let original = describe(
        state.project().dataset(),
        &request(),
        &JobControl::default(),
    )
    .unwrap();
    state
        .handle_event(JobEvent::Completed {
            token: state.token(),
            cancelled: false,
            result: Ok(JobOutput::Analyzed(original.into())),
        })
        .unwrap();
    for reference_text in ["NaN", "inf", "1e999", ""] {
        state = state
            .apply_action(UiAction::SetDraft(AnalysisDraft::OneSampleT {
                variable: Some(ColumnId(1)),
                reference_text: reference_text.into(),
                confidence: ConfidenceLevel::P95,
                filter: RowFilter::default(),
                frequency_weight: None,
            }))
            .unwrap();
        let ctx = egui::Context::default();
        tessera::desktop::configure_context(&ctx);
        let mut jobs = JobRunner::default();
        frame(
            &ctx,
            &mut state,
            &mut jobs,
            Vec2::new(760.0, 520.0),
            vec![Event::Key {
                key: Key::Enter,
                physical_key: None,
                pressed: true,
                repeat: false,
                modifiers: Modifiers::COMMAND,
            }],
        );
        assert!(!jobs.is_running());
        assert_eq!(state.project().runs().len(), 1);
        assert!(state.error().is_some());
        assert!(matches!(
            state.selected_run().unwrap().analysis,
            CompletedAnalysis::Describe(_)
        ));
    }
}

#[test]
fn native_group_choices_keep_equal_labels_and_empty_text_distinct() {
    let mut state = state();
    let mut metadata = state
        .project()
        .dataset()
        .column(ColumnId(2))
        .unwrap()
        .metadata()
        .clone();
    metadata.value_labels = vec![
        tessera::data::ValueLabel {
            value: Cell::Text("".into()),
            label: "Same".into(),
        },
        tessera::data::ValueLabel {
            value: Cell::Text("Carys".into()),
            label: "Same".into(),
        },
    ];
    state = state
        .apply_action(UiAction::Metadata {
            column: ColumnId(2),
            metadata,
        })
        .unwrap();
    state = state
        .apply_action(UiAction::SetDraft(AnalysisDraft::IndependentT {
            outcome: Some(ColumnId(1)),
            group: Some(ColumnId(2)),
            groups: [None, None],
            method: IndependentMethod::Welch,
            confidence: ConfidenceLevel::P95,
            filter: RowFilter::default(),
            frequency_weight: None,
        }))
        .unwrap();
    state = state
        .apply_action(UiAction::Navigate(Workspace::Analyses))
        .unwrap();
    let ctx = egui::Context::default();
    tessera::desktop::configure_context(&ctx);
    let size = Vec2::new(1180.0, 780.0);
    let mut jobs = JobRunner::default();
    for (label, index) in [("First group", 0), ("Second group", 1)] {
        frame(&ctx, &mut state, &mut jobs, size, vec![]);
        let until = Instant::now() + Duration::from_secs(5);
        while jobs.is_running() {
            state.poll(&mut jobs);
            assert!(Instant::now() < until);
            std::thread::yield_now();
        }
        frame(&ctx, &mut state, &mut jobs, size, vec![]);
        let choices = state.group_lookup().unwrap();
        assert_eq!(choices.choices.len(), 2);
        assert!(choices.choices.iter().all(|c| c.label.contains("Same")));
        assert_ne!(choices.choices[0].value, choices.choices[1].value);
        frame(
            &ctx,
            &mut state,
            &mut jobs,
            size,
            vec![
                Event::PointerMoved(Pos2::new(300., 380.)),
                Event::MouseWheel {
                    unit: egui::MouseWheelUnit::Point,
                    phase: egui::TouchPhase::Move,
                    delta: Vec2::new(0., -180.),
                    modifiers: Modifiers::NONE,
                },
            ],
        );
        for _ in 0..20 {
            frame(&ctx, &mut state, &mut jobs, size, vec![]);
        }
        click(&ctx, &mut state, &mut jobs, label, size);
        frame(&ctx, &mut state, &mut jobs, size, vec![]);
        click(
            &ctx,
            &mut state,
            &mut jobs,
            &format!("{label}: choice {index}"),
            size,
        );
        frame(&ctx, &mut state, &mut jobs, size, vec![]);
    }
    let AnalysisDraft::IndependentT { groups, .. } = state.draft() else {
        panic!()
    };
    assert_eq!(
        groups,
        [
            Some(Cell::Text("".into())),
            Some(Cell::Text("Carys".into()))
        ]
        .as_ref()
    );
    assert!(state.checked_draft_request().is_ok());
}
