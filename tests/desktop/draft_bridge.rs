use super::*;
use tessera::{analysis::*, desktop::draft::AnalysisDraft};
fn mixed_state() -> DesktopState {
    DesktopState::from_project(
        tessera::project_io::decode(
            &include_bytes!("../../examples/mixed-analysis.tessera")[..],
            &JobControl::default(),
        )
        .unwrap(),
    )
    .unwrap()
}
#[test]
fn every_saved_recipe_builds_through_the_checked_draft_and_rerun_boundary() {
    let mut state = mixed_state();
    for i in 0..5 {
        let run = &state.project().runs()[i];
        let request = state.checked_run_request(run.id).unwrap();
        let draft = AnalysisDraft::from_request(request.clone());
        assert!(draft.matches_saved(run.analysis.request()));
        assert_eq!(
            draft.build_request(state.project().dataset()).unwrap(),
            request
        );
        state = state.apply_action(UiAction::SetDraft(draft)).unwrap();
        assert_eq!(state.checked_draft_request().unwrap(), request);
    }
    assert!(state.checked_run_request(RunId(999)).is_err());
}
#[test]
fn incomplete_roles_reference_text_and_oversized_drafts_never_form_fake_requests() {
    let state = mixed_state();
    let filter = RowFilter::default();
    let cases = [
        AnalysisDraft::Pearson(PearsonRequest {
            columns: vec![ColumnId(1)],
            missing_policy: CorrelationMissingPolicy::PairwiseComplete,
            filter: filter.clone(),
            frequency_weight: None,
        }),
        AnalysisDraft::OneSampleT {
            variable: None,
            reference_text: "1".into(),
            confidence: ConfidenceLevel::P95,
            filter: filter.clone(),
            frequency_weight: None,
        },
        AnalysisDraft::OneSampleT {
            variable: Some(ColumnId(1)),
            reference_text: "NaN".into(),
            confidence: ConfidenceLevel::P95,
            filter: filter.clone(),
            frequency_weight: None,
        },
        AnalysisDraft::PairedT {
            first: Some(ColumnId(1)),
            second: None,
            confidence: ConfidenceLevel::P95,
            filter: filter.clone(),
            frequency_weight: None,
        },
        AnalysisDraft::IndependentT {
            outcome: Some(ColumnId(1)),
            group: Some(ColumnId(3)),
            groups: [Some(Cell::Text("<A>".into())), None],
            method: IndependentMethod::Welch,
            confidence: ConfidenceLevel::P95,
            filter: filter.clone(),
            frequency_weight: None,
        },
    ];
    for draft in cases {
        assert!(draft.build_request(state.project().dataset()).is_err());
        let state = state.apply_action(UiAction::SetDraft(draft)).unwrap();
        assert!(state.checked_draft_request().is_err());
    }
    let draft = AnalysisDraft::OneSampleT {
        variable: Some(ColumnId(1)),
        reference_text: "1".repeat(512 * 1024 + 1),
        confidence: ConfidenceLevel::P95,
        filter,
        frequency_weight: None,
    };
    assert!(state.apply_action(UiAction::SetDraft(draft)).is_err());
}
#[test]
fn loaded_methods_render_keep_settings_and_rerun_without_new_chooser_items() {
    let mut state = mixed_state();
    let mut jobs = JobRunner::default();
    let ctx = egui::Context::default();
    ctx.enable_accesskit();
    for (id, title) in [
        (2, "Pearson correlation"),
        (3, "One-sample t-test"),
        (4, "Paired t-test"),
        (5, "Independent t-test"),
    ] {
        state = state
            .apply_action(UiAction::SelectRun(RunId(id)))
            .unwrap()
            .apply_action(UiAction::Navigate(Workspace::Results))
            .unwrap();
        let labels = super::review::access_labels(&ctx, &mut state, &mut jobs);
        assert!(
            labels
                .iter()
                .any(|s| s.starts_with(&format!("Saved result: {title}"))),
            "missing {title}: {labels:?}"
        );
        let request = state.checked_run_request(RunId(id)).unwrap();
        let project = Arc::clone(state.project());
        run(
            &mut state,
            &mut jobs,
            JobOperation::Analyze { project, request },
        );
        assert_eq!(state.project().runs().len(), (id + 4) as usize);
    }
}

#[test]
fn pearson_accessible_estimates_and_counts_keep_captured_pair() {
    let mut state = mixed_state()
        .apply_action(UiAction::SelectRun(RunId(2)))
        .unwrap()
        .apply_action(UiAction::Navigate(Workspace::Results))
        .unwrap();
    let mut jobs = JobRunner::default();
    let ctx = egui::Context::default();
    ctx.enable_accesskit();
    let run = state.selected_run().unwrap();
    let CompletedAnalysis::Pearson(v) = &run.analysis else {
        panic!()
    };
    let row = &v.rows[0];
    let names = [row.first, row.second].map(|id| {
        run.analysis
            .columns()
            .iter()
            .find(|c| c.id == id)
            .unwrap()
            .name
            .clone()
    });
    let expected = [
        ("N", row.n.to_string()),
        (
            "r",
            row.r
                .value
                .map_or_else(|| row.r.reason.clone().unwrap(), |v| v.to_string()),
        ),
        (
            "Two-sided p",
            row.p
                .value
                .map_or_else(|| row.p.reason.clone().unwrap(), |v| v.to_string()),
        ),
        ("Missing rows", row.missing_rows.to_string()),
        (
            "Complete-case excluded rows",
            row.complete_case_excluded_rows.to_string(),
        ),
    ];
    let labels = super::review::access_labels(&ctx, &mut state, &mut jobs);
    for (metric, value) in expected {
        assert!(
            labels.iter().any(|s| s.contains(&names[0])
                && s.contains(&names[1])
                && s.contains(&format!("{metric}: {value}"))),
            "missing {metric}: {labels:?}"
        );
    }
    assert!(labels
        .iter()
        .any(|s| s.contains("Run 2") && s.contains("Pearson correlation")));
}

#[test]
fn scrolled_pearson_cells_retain_pair_metric_and_exact_result() {
    let data = Dataset::new(
        (1..=16)
            .map(|i| {
                Column::numeric(
                    ColumnId(i),
                    &format!("Variable {i:02}"),
                    if i == 16 {
                        vec![Some(3.); 4]
                    } else {
                        vec![Some(i as f64), Some(3.), Some(7.), Some(9.)]
                    },
                )
                .unwrap()
            })
            .collect(),
    )
    .unwrap();
    let request = AnalysisRequest::Pearson(PearsonRequest {
        columns: (1..=16).map(ColumnId).collect(),
        missing_policy: CorrelationMissingPolicy::PairwiseComplete,
        filter: RowFilter::default(),
        frequency_weight: None,
    });
    let result = analyze_with_budget(&data, &request, &JobControl::default(), &[], 4096).unwrap();
    let project = Project::new("Scroll", data)
        .unwrap()
        .with_run(result)
        .unwrap();
    let mut state = DesktopState::from_project(project)
        .unwrap()
        .apply_action(UiAction::SelectRun(RunId(1)))
        .unwrap()
        .apply_action(UiAction::Navigate(Workspace::Results))
        .unwrap();
    let ctx = egui::Context::default();
    ctx.enable_accesskit();
    let mut jobs = JobRunner::default();
    let before = super::review::access_labels(&ctx, &mut state, &mut jobs);
    for _ in 0..5 {
        frame(
            &ctx,
            &mut state,
            &mut jobs,
            Vec2::new(760., 520.),
            vec![
                Event::PointerMoved(Pos2::new(580., 300.)),
                Event::MouseWheel {
                    unit: egui::MouseWheelUnit::Point,
                    delta: Vec2::new(0., -900.),
                    modifiers: Modifiers::NONE,
                    phase: egui::TouchPhase::Move,
                },
            ],
        );
    }
    let after = super::review::access_labels(&ctx, &mut state, &mut jobs);
    let CompletedAnalysis::Pearson(v) = &state.selected_run().unwrap().analysis else {
        panic!()
    };
    let row = v
        .rows
        .iter()
        .find(|r| {
            let pair = format!(
                "Variable {:02} / Variable {:02} · N:",
                r.first.0, r.second.0
            );
            after.iter().any(|s| s.starts_with(&pair))
                && !before.iter().any(|s| s.starts_with(&pair))
        })
        .expect("scroll exposes a different virtual row");
    assert!(
        v.rows.iter().any(|r| r
            .p
            .reason
            .as_ref()
            .is_some_and(|reason| after.contains(&format!(
                "Variable {:02} / Variable {:02} · Two-sided p: {reason}",
                r.first.0, r.second.0
            )))),
        "scrolled unavailable estimate keeps its pair and reason"
    );
    let pair = format!(
        "Variable {:02} / Variable {:02} · ",
        row.first.0, row.second.0
    );
    for (metric, value) in [
        ("N", row.n.to_string()),
        (
            "r",
            row.r
                .value
                .map_or_else(|| row.r.reason.clone().unwrap(), |v| v.to_string()),
        ),
        (
            "Two-sided p",
            row.p
                .value
                .map_or_else(|| row.p.reason.clone().unwrap(), |v| v.to_string()),
        ),
        ("Missing rows", row.missing_rows.to_string()),
        (
            "Complete-case excluded rows",
            row.complete_case_excluded_rows.to_string(),
        ),
    ] {
        assert!(
            after.contains(&format!("{pair}{metric}: {value}")),
            "missing {pair}{metric}: {after:?}"
        );
    }
}

#[test]
fn loaded_whitespace_groups_have_distinct_quoted_captions() {
    for groups in [["", " "], [" ", "  "], ["A", "A "], ["A\t", "A\n"]] {
        let data = Dataset::new(vec![
            Column::numeric(
                ColumnId(1),
                "Score",
                vec![Some(1.), Some(2.), Some(3.), Some(4.)],
            )
            .unwrap(),
            Column::text(
                ColumnId(2),
                "Group",
                vec![
                    Some(groups[0].into()),
                    Some(groups[0].into()),
                    Some(groups[1].into()),
                    Some(groups[1].into()),
                ],
            )
            .unwrap(),
        ])
        .unwrap();
        let request = AnalysisRequest::IndependentT(IndependentTRequest {
            outcome: ColumnId(1),
            group: ColumnId(2),
            groups: groups.map(|g| Cell::Text(g.into())),
            method: IndependentMethod::Welch,
            confidence: ConfidenceLevel::P95,
            filter: RowFilter::default(),
            frequency_weight: None,
        });
        let result =
            analyze_with_budget(&data, &request, &JobControl::default(), &[], 4096).unwrap();
        let project = Project::new("Whitespace", data)
            .unwrap()
            .with_run(result)
            .unwrap();
        let mut state = DesktopState::from_project(project)
            .unwrap()
            .apply_action(UiAction::SelectRun(RunId(1)))
            .unwrap()
            .apply_action(UiAction::Navigate(Workspace::Results))
            .unwrap();
        let ctx = egui::Context::default();
        ctx.enable_accesskit();
        let labels = super::review::access_labels(&ctx, &mut state, &mut JobRunner::default());
        for group in groups {
            assert!(labels.contains(&format!("{group:?}")), "{labels:?}");
        }
    }
}

#[test]
fn pearson_method_panel_explains_test_assumptions() {
    let mut state = mixed_state()
        .apply_action(UiAction::SelectRun(RunId(2)))
        .unwrap()
        .apply_action(UiAction::Navigate(Workspace::Results))
        .unwrap();
    let ctx = egui::Context::default();
    ctx.enable_accesskit();
    let mut jobs = JobRunner::default();
    let mut output = ctx.run_ui(
        egui::RawInput {
            screen_rect: Some(Rect::from_min_size(Pos2::ZERO, Vec2::new(760., 520.))),
            ..Default::default()
        },
        |ui| render(ui, &mut state, &mut jobs),
    );
    output.textures_delta.clear();
    let bounds = output
        .platform_output
        .accesskit_update
        .unwrap()
        .nodes
        .into_iter()
        .find_map(|(_, node)| {
            if node.label() == Some("Method") {
                node.bounds()
            } else {
                None
            }
        })
        .unwrap();
    let pos = Pos2::new(
        ((bounds.x0 + bounds.x1) / 2.) as f32,
        ((bounds.y0 + bounds.y1) / 2.) as f32,
    );
    for pressed in [true, false] {
        frame(
            &ctx,
            &mut state,
            &mut jobs,
            Vec2::new(760., 520.),
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
    let labels = super::review::access_labels(&ctx, &mut state, &mut jobs);
    for phrase in [
        "zero population correlation",
        "independent observations",
        "bivariate normal",
        "linear association",
        "unadjusted across pairs",
    ] {
        assert!(
            labels.iter().any(|s| s.contains(phrase)),
            "missing {phrase}: {labels:?}"
        );
    }
}

#[test]
fn selected_history_identifies_each_procedure() {
    let mut state = mixed_state()
        .apply_action(UiAction::Navigate(Workspace::Results))
        .unwrap();
    let ctx = egui::Context::default();
    ctx.enable_accesskit();
    let mut jobs = JobRunner::default();
    for (id, method) in [
        (1, "Descriptive statistics"),
        (2, "Pearson correlation"),
        (3, "One-sample t-test"),
        (4, "Paired t-test"),
        (5, "Independent t-test · Welch"),
    ] {
        state = state.apply_action(UiAction::SelectRun(RunId(id))).unwrap();
        let labels = super::review::access_labels(&ctx, &mut state, &mut jobs);
        assert!(
            labels
                .iter()
                .any(|s| s.starts_with(&format!("Run {id} · {method} ·"))
                    && s.contains(&state.selected_run().unwrap().analysis.columns()[0].name)
                    && s.ends_with("Current data")),
            "{labels:?}"
        );
    }
    state = state
        .apply_action(UiAction::EditCell {
            column: ColumnId(1),
            row: 0,
            value: Cell::Number(99.),
        })
        .unwrap();
    let labels = super::review::access_labels(&ctx, &mut state, &mut jobs);
    assert!(
        labels
            .iter()
            .any(|s| s.starts_with("Run 5 · Independent t-test · Welch ·")
                && s.ends_with("Data changed")),
        "{labels:?}"
    );
}
