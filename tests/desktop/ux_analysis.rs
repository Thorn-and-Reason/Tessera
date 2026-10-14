use super::*;
use tessera::{
    analysis::*,
    data::{ColumnMetadata, Measurement},
    desktop::draft::AnalysisDraft,
};

fn review_state() -> DesktopState {
    let data = Dataset::new(vec![
        Column::numeric(ColumnId(1), "Q1", vec![Some(1.), Some(2.), Some(3.)])
            .unwrap()
            .with_metadata(ColumnMetadata {
                description: "Confidence in learning".into(),
                measurement: Measurement::Ordinal,
                ..ColumnMetadata::new("Q1")
            })
            .unwrap(),
        Column::numeric(ColumnId(2), "Weight", vec![Some(2.), Some(1.), Some(1.)]).unwrap(),
        Column::text(
            ColumnId(3),
            "Teaching group",
            vec![Some("A".into()), Some("B".into()), Some("A".into())],
        )
        .unwrap(),
    ])
    .unwrap();
    let request = DescribeRequest {
        columns: vec![ColumnId(1)],
        statistics: vec![Statistic::Mean],
        missing_policy: MissingPolicy::PerVariable,
        filter: RowFilter {
            predicates: vec![RowPredicate::Text {
                column: ColumnId(3),
                value: "A".into(),
                equal: true,
            }],
        },
        frequency_weight: Some(ColumnId(2)),
    };
    let result = describe(&data, &request, &JobControl::default()).unwrap();
    let project = Project::new("UX review", data)
        .unwrap()
        .with_run(CompletedAnalysis::Describe(result))
        .unwrap();
    DesktopState::from_project(project)
        .unwrap()
        .apply_action(UiAction::SetDraft(AnalysisDraft::Describe(request)))
        .unwrap()
        .apply_action(UiAction::SelectRun(RunId(1)))
        .unwrap()
        .apply_action(UiAction::Navigate(Workspace::Results))
        .unwrap()
}

fn text_frame(ctx: &egui::Context, state: &mut DesktopState, jobs: &mut JobRunner) -> String {
    let mut output = ctx.run_ui(
        egui::RawInput {
            screen_rect: Some(Rect::from_min_size(Pos2::ZERO, Vec2::new(1200., 900.))),
            ..Default::default()
        },
        |ui| render(ui, state, jobs),
    );
    output.textures_delta.clear();
    output
        .shapes
        .iter()
        .filter_map(|s| match &s.shape {
            egui::Shape::Text(t) => Some(t.galley.text()),
            _ => None,
        })
        .collect::<Vec<_>>()
        .join("\n")
}

#[test]
fn fresh_analysis_resets_roles_filter_weight_and_keeps_saved_results() {
    let mut state = review_state();
    let project = Arc::clone(state.project());
    let ctx = egui::Context::default();
    let mut jobs = JobRunner::default();
    frame(&ctx, &mut state, &mut jobs, Vec2::new(1200., 900.), vec![]);
    click(
        &ctx,
        &mut state,
        &mut jobs,
        "New analysis",
        Vec2::new(1200., 900.),
    );
    let AnalysisDraft::Describe(draft) = state.draft() else {
        panic!("wrong procedure");
    };
    assert!(draft.columns.is_empty());
    assert!(draft.filter.predicates.is_empty());
    assert_eq!(draft.frequency_weight, None);
    assert!(Arc::ptr_eq(state.project(), &project));
    assert_eq!(state.workspace(), Workspace::Analyses);
}

#[test]
fn continuation_keeps_context_and_names_filter_before_run() {
    let mut state = review_state();
    let mut current = request();
    current.columns = vec![ColumnId(2)];
    current.filter.predicates = vec![RowPredicate::Text {
        column: ColumnId(3),
        value: "A".into(),
        equal: true,
    }];
    current.frequency_weight = Some(ColumnId(2));
    state = state
        .apply_action(UiAction::SetDraft(current.into()))
        .unwrap();
    let before = state.draft().clone();
    let ctx = egui::Context::default();
    let mut jobs = JobRunner::default();
    frame(&ctx, &mut state, &mut jobs, Vec2::new(1200., 900.), vec![]);
    assert!(state.control_rect("Continue current setup").is_some());
    assert!(text_frame(&ctx, &mut state, &mut jobs).contains("Continue current setup"));
    click(
        &ctx,
        &mut state,
        &mut jobs,
        "Continue current setup",
        Vec2::new(1200., 900.),
    );
    assert_eq!(state.draft(), &before);
    let text = text_frame(&ctx, &mut state, &mut jobs);
    assert!(text.contains("Teaching group = \"A\""), "{text}");
    assert!(text.contains("Weight variable: Weight"), "{text}");
    assert!(text.contains("Required fields complete"), "{text}");
    assert!(state.control_rect("Clear filter and weight").is_some());
    assert!(state.control_rect("Assumptions and method").is_some());
}

#[test]
fn fresh_complex_analysis_clears_item_and_design_roles() {
    use tessera::desktop::draft::{
        anova::{AnovaDesignDraft, AnovaDraft},
        reliability::ReliabilityDraft,
    };
    for old in [
        AnalysisDraft::Anova(AnovaDraft {
            design: AnovaDesignDraft::OneWay {
                outcome: Some(ColumnId(1)),
                factor: Some(ColumnId(3)),
            },
            ..Default::default()
        }),
        AnalysisDraft::Reliability(ReliabilityDraft {
            items: vec![ColumnId(1), ColumnId(2)],
            ..Default::default()
        }),
    ] {
        let mut state = review_state()
            .apply_action(UiAction::SetDraft(old.clone()))
            .unwrap();
        let ctx = egui::Context::default();
        let mut jobs = JobRunner::default();
        frame(&ctx, &mut state, &mut jobs, Vec2::new(1200., 900.), vec![]);
        click(
            &ctx,
            &mut state,
            &mut jobs,
            "New analysis",
            Vec2::new(1200., 900.),
        );
        assert_ne!(state.draft(), &old);
        assert!(
            state.checked_draft_request().is_err(),
            "Fresh analysis must require roles again"
        );
    }
}

#[test]
fn scalar_selection_explains_ordinal_codes_and_modes() {
    let draft = AnalysisDraft::OneSampleT {
        variable: Some(ColumnId(1)),
        reference_text: "0".into(),
        confidence: ConfidenceLevel::P95,
        filter: RowFilter::default(),
        frequency_weight: None,
    };
    let mut state = review_state()
        .apply_action(UiAction::SetDraft(draft))
        .unwrap()
        .apply_action(UiAction::Navigate(Workspace::Analyses))
        .unwrap();
    let ctx = egui::Context::default();
    let mut jobs = JobRunner::default();
    let text = text_frame(&ctx, &mut state, &mut jobs);
    assert!(text.contains("Confidence in learning"), "{text}");
    assert!(text.contains("Ordinal"), "{text}");
    assert!(text.contains("numeric codes as quantities"), "{text}");
    assert!(text.contains("Guided keeps optional settings"), "{text}");
}

#[test]
fn reopened_results_offer_explicit_subset_selection() {
    let mut state = review_state();
    let ctx = egui::Context::default();
    let mut jobs = JobRunner::default();
    let text = text_frame(&ctx, &mut state, &mut jobs);
    assert!(text.contains("Export selected results"), "{text}");
    assert!(state.control_rect("Include Run 1 in report").is_some());
    assert!(text.contains("this session"), "{text}");
}

#[test]
fn variable_search_finds_descriptions_and_shows_measurement_context() {
    let mut state = review_state()
        .apply_action(UiAction::Navigate(Workspace::Analyses))
        .unwrap();
    let ctx = egui::Context::default();
    let mut jobs = JobRunner::default();
    frame(&ctx, &mut state, &mut jobs, Vec2::new(1200., 900.), vec![]);
    click(
        &ctx,
        &mut state,
        &mut jobs,
        "Remove variable 1",
        Vec2::new(1200., 900.),
    );
    click(
        &ctx,
        &mut state,
        &mut jobs,
        "Add variables…",
        Vec2::new(1200., 900.),
    );
    frame(
        &ctx,
        &mut state,
        &mut jobs,
        Vec2::new(1200., 900.),
        vec![Event::Text("Confidence in learning".into())],
    );
    let text = text_frame(&ctx, &mut state, &mut jobs);
    assert!(text.contains("Confidence in learning"), "{text}");
    assert!(text.contains("Ordinal"), "{text}");
    assert!(state.control_rect("Variable 1").is_some());
}

#[test]
fn helper_links_expose_regression_and_reliability_without_running() {
    let mut state = review_state()
        .apply_action(UiAction::Navigate(Workspace::Analyses))
        .unwrap();
    let ctx = egui::Context::default();
    let mut jobs = JobRunner::default();
    frame(&ctx, &mut state, &mut jobs, Vec2::new(1200., 900.), vec![]);
    click(
        &ctx,
        &mut state,
        &mut jobs,
        "Help choose an analysis",
        Vec2::new(1200., 900.),
    );
    assert!(state.control_rect("Open linear regression form").is_some());
    assert!(state.control_rect("Open scale reliability form").is_some());
    click(
        &ctx,
        &mut state,
        &mut jobs,
        "Open linear regression form",
        Vec2::new(1200., 900.),
    );
    assert_eq!(
        state.draft().procedure(),
        tessera::desktop::draft::Procedure::Regression
    );
    assert!(!jobs.is_running());
    assert_eq!(state.project().runs().len(), 1);
}

#[test]
fn clear_context_preserves_roles_and_history() {
    let mut state = review_state()
        .apply_action(UiAction::Navigate(Workspace::Analyses))
        .unwrap();
    let project = Arc::clone(state.project());
    let ctx = egui::Context::default();
    let mut jobs = JobRunner::default();
    let size = Vec2::new(1200., 900.);
    frame(&ctx, &mut state, &mut jobs, size, vec![]);
    click(&ctx, &mut state, &mut jobs, "Clear filter and weight", size);
    let AnalysisDraft::Describe(draft) = state.draft() else {
        panic!("wrong procedure")
    };
    assert_eq!(draft.columns, vec![ColumnId(1)]);
    assert!(draft.filter.predicates.is_empty());
    assert_eq!(draft.frequency_weight, None);
    assert!(Arc::ptr_eq(state.project(), &project));
    assert!(!jobs.is_running());
}

#[test]
fn reopened_subset_export_preserves_history_order_and_scope() {
    let original = review_state();
    let mut project = (**original.project()).clone();
    for _ in 1..6 {
        let result = describe(project.dataset(), &request(), &JobControl::default()).unwrap();
        project = project
            .with_run(CompletedAnalysis::Describe(result))
            .unwrap();
    }
    let control = JobControl::default();
    let mut encoded = Vec::new();
    tessera::project_io::encode(&project, &mut encoded, &control).unwrap();
    let reopened = tessera::project_io::decode(encoded.as_slice(), &control).unwrap();
    let mut state = DesktopState::from_project(reopened).unwrap();
    assert!(state.report_export_runs().is_err());
    for id in [6, 4, 5] {
        state.toggle_report_run(RunId(id)).unwrap();
    }
    let selected = state.report_export_runs().unwrap();
    assert_eq!(selected, vec![RunId(4), RunId(5), RunId(6)]);
    assert_eq!(selected.capacity(), selected.len());
    let path = std::env::temp_dir().canonicalize().unwrap().join(format!(
        "tessera-selected-history-{}-{:?}.html",
        std::process::id(),
        std::thread::current().id()
    ));
    tessera::export::export_report(&path, state.project(), &selected, &control).unwrap();
    let html = std::fs::read_to_string(&path).unwrap();
    std::fs::remove_file(path).unwrap();
    for id in 1..=6 {
        assert_eq!(
            html.contains(&format!("<h2 id=\"run-{id}\">Analysis run {id}</h2>")),
            id >= 4
        );
    }
    state.toggle_report_run(RunId(5)).unwrap();
    assert_eq!(
        state.report_export_runs().unwrap(),
        vec![RunId(4), RunId(6)]
    );
    assert!(state.toggle_report_run(RunId(7)).is_err());
}

#[test]
fn history_and_figures_use_captured_names_and_explicit_context() {
    let mut state = review_state();
    let metadata = ColumnMetadata::new("Renamed current question");
    state = state
        .apply_action(UiAction::Metadata {
            column: ColumnId(1),
            metadata,
        })
        .unwrap();
    let ctx = egui::Context::default();
    ctx.all_styles_mut(|s| s.animation_time = 0.);
    let mut jobs = JobRunner::default();
    let size = Vec2::new(1200., 900.);
    let text = text_frame(&ctx, &mut state, &mut jobs);
    assert!(text.contains("Analysis history"), "{text}");
    assert!(text.contains("Recorded result"), "{text}");
    assert!(!text.contains("Saved result"), "{text}");
    assert!(text.contains("Selected: Run 1"), "{text}");
    assert!(text.contains("Teaching group = \"A\""), "{text}");
    assert!(text.contains("Frequency weight: Weight"), "{text}");
    click(&ctx, &mut state, &mut jobs, "Figures", size);
    let text = text_frame(&ctx, &mut state, &mut jobs);
    assert!(text.contains("Distribution · Q1"), "{text}");
    assert!(!text.contains("Distribution · Renamed current question"));
    assert!(!text.contains("Distribution · variable 1"));
}

#[test]
fn regression_and_reliability_search_select_stable_ids_from_descriptions() {
    use tessera::desktop::draft::{regression::RegressionDraft, reliability::ReliabilityDraft};
    for (draft, opener, search, candidate) in [
        (
            AnalysisDraft::Regression(RegressionDraft {
                outcome: Some(ColumnId(2)),
                ..Default::default()
            }),
            "Add predictor",
            "Search predictors",
            "Add predictor: 1",
        ),
        (
            AnalysisDraft::Reliability(ReliabilityDraft::default()),
            "Add item",
            "Search numeric items",
            "Add item: 1",
        ),
    ] {
        let mut state = review_state()
            .apply_action(UiAction::SetDraft(draft))
            .unwrap()
            .apply_action(UiAction::Navigate(Workspace::Analyses))
            .unwrap();
        let ctx = egui::Context::default();
        ctx.all_styles_mut(|s| s.animation_time = 0.);
        let mut jobs = JobRunner::default();
        let size = Vec2::new(1200., 900.);
        frame(&ctx, &mut state, &mut jobs, size, vec![]);
        click(&ctx, &mut state, &mut jobs, opener, size);
        click(&ctx, &mut state, &mut jobs, search, size);
        frame(
            &ctx,
            &mut state,
            &mut jobs,
            size,
            vec![Event::Text("Confidence in learning".into())],
        );
        let text = text_frame(&ctx, &mut state, &mut jobs);
        assert!(text.contains("Confidence in learning"), "{text}");
        assert!(text.contains("Ordinal"), "{text}");
        assert!(state.control_rect(candidate).is_some());
        click(&ctx, &mut state, &mut jobs, candidate, size);
        match state.draft() {
            AnalysisDraft::Regression(d) => assert_eq!(d.predictors[0].column, ColumnId(1)),
            AnalysisDraft::Reliability(d) => assert_eq!(d.items, vec![ColumnId(1)]),
            _ => panic!("wrong draft"),
        }
        assert!(!jobs.is_running());
    }
}

fn poll_review(state: &mut DesktopState, jobs: &mut JobRunner) {
    let deadline = Instant::now() + Duration::from_secs(10);
    while jobs.is_running() {
        state.poll(jobs);
        assert!(Instant::now() < deadline);
        std::thread::yield_now();
    }
    assert!(state.error().is_none(), "{:?}", state.error());
}

#[test]
fn survey_membership_tracks_reviewed_ids_and_actual_empty_text_policy() {
    use tessera::{data::MissingRules, survey::SurveyRole};
    let columns = (1..=6)
        .map(|id| {
            let name = if id == 1 {
                "Responses".into()
            } else {
                format!("Question {id}")
            };
            let mut metadata = ColumnMetadata::new(&name);
            if id == 2 {
                metadata.missing_rules = MissingRules::Text(vec![String::new()]);
            }
            Column::text(
                ColumnId(id),
                &name,
                vec![Some(" A ".into()), Some(String::new()), None],
            )
            .unwrap()
            .with_metadata(metadata)
            .unwrap()
        })
        .collect();
    let mut state = DesktopState::from_project(
        Project::new("Membership", Dataset::new(columns).unwrap()).unwrap(),
    )
    .unwrap();
    let mut jobs = JobRunner::default();
    state.enter_survey().unwrap();
    state.service_survey(&mut jobs).unwrap();
    poll_review(&mut state, &mut jobs);
    for id in 1..=6 {
        state
            .set_survey_role(
                ColumnId(id),
                if id <= 2 {
                    SurveyRole::Categories
                } else {
                    SurveyRole::Exclude
                },
            )
            .unwrap();
    }
    let mut draft = state.copy_survey_cleaning_draft().unwrap();
    draft.column = Some(ColumnId(1));
    draft.trim = true;
    state.set_survey_cleaning_draft(draft).unwrap();
    state.preview_survey_cleaning_draft(&mut jobs).unwrap();
    poll_review(&mut state, &mut jobs);
    assert_eq!(state.survey_cleaning_preview().unwrap().changed, 1);
    state.apply_survey_cleaning(&mut jobs).unwrap();
    poll_review(&mut state, &mut jobs);
    let relationships = state.survey_reviewed_sources().collect::<Vec<_>>();
    assert_eq!(relationships, vec![(ColumnId(1), ColumnId(7))]);
    assert_eq!(
        state.project().dataset().cell(ColumnId(1), 0).unwrap(),
        Cell::Text(" A ".into())
    );
    assert_eq!(
        state.project().dataset().cell(ColumnId(7), 0).unwrap(),
        Cell::Text("A".into())
    );
    let ctx = egui::Context::default();
    ctx.all_styles_mut(|s| s.animation_time = 0.);
    let size = Vec2::new(1200., 1000.);
    frame(&ctx, &mut state, &mut jobs, size, vec![]);
    click(
        &ctx,
        &mut state,
        &mut jobs,
        "Review membership and blank responses",
        size,
    );
    let text = text_frame(&ctx, &mut state, &mut jobs);
    assert!(
        text.contains("2 included · 5 excluded · 0 unresolved"),
        "{text}"
    );
    assert!(text.contains("Empty text counts as valid"), "{text}");
    assert!(text.contains("Empty text is declared missing"), "{text}");
    let mut found = false;
    for _ in 0..7 {
        let text = text_frame(&ctx, &mut state, &mut jobs);
        if text.contains("Original Responses → reviewed copy Responses (reviewed)") {
            found = true;
            break;
        }
        click(
            &ctx,
            &mut state,
            &mut jobs,
            "Next report membership page",
            size,
        );
    }
    assert!(
        found,
        "tracked reviewed copy was not reachable through membership paging"
    );
    assert!(!jobs.is_running());
}

#[test]
fn compact_analysis_keeps_run_method_and_active_context_reachable() {
    for zoom in [1., 1.5] {
        let mut state = review_state()
            .apply_action(UiAction::Navigate(Workspace::Analyses))
            .unwrap();
        let ctx = egui::Context::default();
        tessera::desktop::configure_context(&ctx);
        ctx.set_zoom_factor(zoom);
        let mut jobs = JobRunner::default();
        let size = Vec2::new(640., 440.);
        frame(&ctx, &mut state, &mut jobs, size, vec![]);
        let output =
            super::survey_improvements::review_frame(&ctx, &mut state, &mut jobs, size, vec![]);
        for label in [
            "Run descriptives",
            "Assumptions and method",
            "Clear filter and weight",
        ] {
            let rect = state.control_rect(label).unwrap();
            assert!(output.shapes.iter().any(|shape| matches!(&shape.shape, egui::Shape::Text(text) if text.galley.text() == label) && shape.clip_rect.contains_rect(rect)), "{label} is clipped at zoom {zoom}: {rect:?}");
        }
        assert!(output.shapes.iter().any(|shape| matches!(&shape.shape, egui::Shape::Text(text) if text.galley.text().contains("Teaching group = \"A\"")) && shape.clip_rect.intersects(shape.shape.visual_bounding_rect())), "filter is not visible at zoom {zoom}");
        assert!(output.shapes.iter().any(|shape| matches!(&shape.shape, egui::Shape::Text(text) if text.galley.text().contains("Weight variable: Weight")) && shape.clip_rect.intersects(shape.shape.visual_bounding_rect())), "weight is not visible at zoom {zoom}");
        assert!(!jobs.is_running());
    }
}

#[test]
fn visible_filter_distinguishes_empty_and_whitespace_text() {
    let mut draft = request();
    draft.filter.predicates = vec![
        RowPredicate::Text {
            column: ColumnId(3),
            value: String::new(),
            equal: true,
        },
        RowPredicate::Text {
            column: ColumnId(3),
            value: " A ".into(),
            equal: false,
        },
    ];
    let mut state = review_state()
        .apply_action(UiAction::SetDraft(draft.into()))
        .unwrap()
        .apply_action(UiAction::Navigate(Workspace::Analyses))
        .unwrap();
    let text = text_frame(
        &egui::Context::default(),
        &mut state,
        &mut JobRunner::default(),
    );
    assert!(text.contains("Teaching group = \"\""), "{text}");
    assert!(text.contains("Teaching group ≠ \" A \""), "{text}");
}

#[test]
fn selected_regression_predictor_retains_meaning_and_advises_only_numeric_roles() {
    use tessera::desktop::draft::regression::{
        RegressionDraft, RegressionDraftRole, RegressionPredictorDraft,
    };
    for role in [
        RegressionDraftRole::Numeric,
        RegressionDraftRole::Categorical {
            reference: Some(Cell::Number(1.)),
        },
    ] {
        let numeric = matches!(role, RegressionDraftRole::Numeric);
        let draft = AnalysisDraft::Regression(RegressionDraft {
            outcome: Some(ColumnId(2)),
            predictors: vec![RegressionPredictorDraft {
                column: ColumnId(1),
                role,
            }],
            ..Default::default()
        });
        let mut state = review_state()
            .apply_action(UiAction::SetDraft(draft.clone()))
            .unwrap()
            .apply_action(UiAction::Navigate(Workspace::Analyses))
            .unwrap();
        let mut jobs = JobRunner::default();
        let text = text_frame(&egui::Context::default(), &mut state, &mut jobs);
        assert!(text.contains("Ordinal · Confidence in learning"), "{text}");
        assert_eq!(
            text.contains("numeric codes as quantities"),
            numeric,
            "{text}"
        );
        assert_eq!(state.draft(), &draft);
        assert!(!jobs.is_running());
    }
}

#[test]
fn report_selection_checkbox_exposes_actual_accessible_toggled_state() {
    let mut state = review_state();
    let ctx = egui::Context::default();
    ctx.enable_accesskit();
    ctx.all_styles_mut(|style| style.animation_time = 0.);
    let mut jobs = JobRunner::default();
    let size = Vec2::new(1200., 900.);
    for (selected, toggled) in [
        (false, egui::accesskit::Toggled::False),
        (true, egui::accesskit::Toggled::True),
        (false, egui::accesskit::Toggled::False),
    ] {
        frame(&ctx, &mut state, &mut jobs, size, vec![]);
        let output =
            super::survey_improvements::review_frame(&ctx, &mut state, &mut jobs, size, vec![]);
        let tree = output.platform_output.accesskit_update.unwrap();
        let checkbox = tree
            .nodes
            .iter()
            .find(|(_, node)| {
                node.role() == egui::accesskit::Role::CheckBox
                    && node.label() == Some("Include Run 1 in report")
            })
            .map(|(_, node)| node)
            .expect("report checkbox has an accessible name");
        assert_eq!(checkbox.toggled(), Some(toggled));
        assert_eq!(state.report_run_ids().contains(&RunId(1)), selected);
        click(&ctx, &mut state, &mut jobs, "Include Run 1 in report", size);
    }
    assert!(!jobs.is_running());
}
