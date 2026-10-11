#[path = "../common/regression.rs"]
mod fixture;
use super::*;
use tessera::{
    analysis::*,
    desktop::draft::{
        regression::{RegressionDraftRole, RegressionPredictorDraft},
        AnalysisDraft, Procedure,
    },
};

fn regression_state() -> DesktopState {
    DesktopState::from_project(fixture::captured())
        .unwrap()
        .apply_action(UiAction::SetDraft(AnalysisDraft::from_request(
            fixture::request(),
        )))
        .unwrap()
        .apply_action(UiAction::Navigate(Workspace::Analyses))
        .unwrap()
}
fn frame(
    state: &mut DesktopState,
    jobs: &mut JobRunner,
    ctx: &egui::Context,
    events: Vec<Event>,
) -> egui::FullOutput {
    let mut out = ctx.run_ui(
        egui::RawInput {
            screen_rect: Some(Rect::from_min_size(Pos2::ZERO, Vec2::new(1180., 5000.))),
            events,
            ..Default::default()
        },
        |ui| render(ui, state, jobs),
    );
    out.textures_delta.clear();
    out
}
fn click(state: &mut DesktopState, jobs: &mut JobRunner, ctx: &egui::Context, key: &str) {
    let pos = state
        .control_rect(key)
        .unwrap_or_else(|| panic!("control {key}"))
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
fn exposed_procedure_runs_from_native_button_and_shortcut() {
    assert_eq!(Procedure::ALL.len(), 12);
    assert!(Procedure::ALL.contains(&Procedure::Regression));
    let mut state = regression_state();
    let ctx = egui::Context::default();
    tessera::desktop::configure_context(&ctx);
    let mut jobs = JobRunner::default();
    frame(&mut state, &mut jobs, &ctx, vec![]);
    click(&mut state, &mut jobs, &ctx, "Run analysis");
    settle(&mut state, &mut jobs);
    assert_eq!(state.project().runs().len(), 2);
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
        .all(|r| matches!(r.analysis, CompletedAnalysis::Regression(_))));
}
#[test]
fn active_reference_switch_rejects_late_lookup() {
    let mut state = regression_state();
    let AnalysisDraft::Regression(mut draft) = state.draft().clone() else {
        panic!()
    };
    draft.predictors.push(RegressionPredictorDraft {
        column: ColumnId(4),
        role: RegressionDraftRole::Categorical {
            reference: Some(Cell::Text("north".into())),
        },
    });
    state = state
        .apply_action(UiAction::SetDraft(AnalysisDraft::Regression(draft)))
        .unwrap();
    state
        .set_active_regression_reference(Some(ColumnId(3)))
        .unwrap();
    let ctx = egui::Context::default();
    tessera::desktop::configure_context(&ctx);
    let mut jobs = JobRunner::default();
    frame(&mut state, &mut jobs, &ctx, vec![]);
    let token = state.token();
    let epoch = state.group_role_epoch();
    click(&mut state, &mut jobs, &ctx, "Reference level: 4");
    assert_eq!(state.active_regression_reference(), Some(ColumnId(4)));
    assert!(state.group_role_epoch() > epoch);
    state
        .handle_event(JobEvent::Completed {
            token,
            cancelled: false,
            result: Ok(JobOutput::GroupValues(Arc::new(GroupLookupResult {
                column: ColumnId(3),
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
    assert!(state.group_lookup().is_none_or(|r| r.column == ColumnId(4)));
    settle(&mut state, &mut jobs);
    frame(&mut state, &mut jobs, &ctx, vec![]);
    settle(&mut state, &mut jobs);
    assert_eq!(state.group_lookup().unwrap().column, ColumnId(4));
    let epoch = state.group_role_epoch();
    let AnalysisDraft::Regression(mut draft) = state.draft().clone() else {
        panic!()
    };
    draft.predictors.swap(0, 2);
    state = state
        .apply_action(UiAction::SetDraft(AnalysisDraft::Regression(draft)))
        .unwrap();
    assert_eq!(state.group_role_epoch(), epoch);
    assert!(state.group_lookup().is_some());
    state.set_group_query("south".into()).unwrap();
    assert!(state.group_lookup().is_none());
}
#[test]
fn cancelled_and_stale_analysis_completions_never_append() {
    for cancelled in [false, true] {
        let mut state = regression_state();
        let token = state.token();
        let result = fixture::captured().runs()[0].analysis.clone();
        if !cancelled {
            state = state
                .apply_action(UiAction::EditCell {
                    column: ColumnId(1),
                    row: 0,
                    value: Cell::Number(42.),
                })
                .unwrap();
        }
        state
            .handle_event(JobEvent::Completed {
                token,
                cancelled,
                result: Ok(JobOutput::Analyzed(result)),
            })
            .unwrap();
        assert_eq!(state.project().runs().len(), 1);
    }
}
#[test]
fn saved_settings_load_when_current_outcome_type_is_stale_but_rerun_refuses() {
    let p = fixture::captured();
    let data = p
        .dataset()
        .convert_column_with_budget(
            ColumnId(1),
            tessera::data::ColumnKind::Text,
            &JobControl::default(),
            &[],
            p.owned_bytes().unwrap(),
        )
        .unwrap();
    let mut state = DesktopState::from_project(p.with_dataset(data).unwrap())
        .unwrap()
        .apply_action(UiAction::Navigate(Workspace::Results))
        .unwrap()
        .apply_action(UiAction::SelectRun(RunId(1)))
        .unwrap();
    let ctx = egui::Context::default();
    tessera::desktop::configure_context(&ctx);
    let mut jobs = JobRunner::default();
    frame(&mut state, &mut jobs, &ctx, vec![]);
    click(&mut state, &mut jobs, &ctx, "Use this run in Analyses");
    assert!(state
        .draft()
        .matches_saved(state.project().runs()[0].analysis.request()));
    assert_eq!(state.active_regression_reference(), Some(ColumnId(3)));
    assert!(state.checked_draft_request().is_err());
    assert!(state.checked_run_request(RunId(1)).is_err());
}

#[test]
fn native_dispatch_reports_frozen_coefficients_after_current_data_changes() {
    let p = fixture::captured();
    let before = p.runs()[0].analysis.clone();
    let data = p
        .dataset()
        .edit_cell(ColumnId(1), 0, Cell::Number(999.))
        .unwrap();
    let mut state = DesktopState::from_project(p.with_dataset(data).unwrap())
        .unwrap()
        .apply_action(UiAction::Navigate(Workspace::Results))
        .unwrap()
        .apply_action(UiAction::SelectRun(RunId(1)))
        .unwrap();
    let ctx = egui::Context::default();
    tessera::desktop::configure_context(&ctx);
    ctx.enable_accesskit();
    let mut jobs = JobRunner::default();
    let out = frame(&mut state, &mut jobs, &ctx, vec![]);
    assert!(out.shapes.iter().any(|s| matches!(&s.shape, egui::Shape::Text(t) if t.galley.text().contains("Linear regression"))));
    assert!(out.shapes.iter().any(|s| matches!(&s.shape, egui::Shape::Text(t) if t.galley.text().contains("Data changed since this run"))));
    let labels = out
        .platform_output
        .accesskit_update
        .unwrap()
        .nodes
        .into_iter()
        .filter_map(|(_, n)| n.label().or_else(|| n.value()).map(str::to_owned))
        .collect::<Vec<_>>()
        .join("\n");
    let CompletedAnalysis::Regression(r) = &before else {
        panic!()
    };
    let RegressionOutcome::Fitted(f) = &r.outcome else {
        panic!()
    };
    for c in &f.coefficients {
        assert!(
            labels.contains(&c.estimate.value.unwrap().to_string()),
            "coefficient not retained: {labels}"
        );
    }
    assert_eq!(state.project().runs()[0].analysis, before);
    assert!(!jobs.is_running());
}
#[test]
fn genuine_cancelled_worker_preserves_history() {
    let mut state = regression_state();
    let mut jobs = JobRunner::default();
    let request = state.checked_draft_request().unwrap();
    state
        .start_operation(
            &mut jobs,
            JobOperation::Analyze {
                project: Arc::clone(state.project()),
                request,
            },
        )
        .unwrap();
    jobs.cancel();
    settle(&mut state, &mut jobs);
    assert_eq!(state.project().runs().len(), 1);
}

#[test]
fn retained_regression_settings_switch_to_editable_independent_controls() {
    let mut state = DesktopState::from_project(fixture::captured())
        .unwrap()
        .apply_action(UiAction::Navigate(Workspace::Results))
        .unwrap()
        .apply_action(UiAction::SelectRun(RunId(1)))
        .unwrap();
    let ctx = egui::Context::default();
    tessera::desktop::configure_context(&ctx);
    let mut jobs = JobRunner::default();
    frame(&mut state, &mut jobs, &ctx, vec![]);
    click(&mut state, &mut jobs, &ctx, "Use this run in Analyses");
    assert_eq!(state.active_regression_reference(), Some(ColumnId(3)));
    frame(&mut state, &mut jobs, &ctx, vec![]);
    let token = state.token();
    let epoch = state.group_role_epoch();
    // The chooser's exact action; its popup is not pointer-tested here.
    state = state
        .apply_action(UiAction::SetDraft(
            state.draft().for_procedure(Procedure::IndependentT),
        ))
        .unwrap();
    assert_eq!(state.active_regression_reference(), None);
    assert!(state.group_role_epoch() > epoch);
    frame(&mut state, &mut jobs, &ctx, vec![]);
    click(&mut state, &mut jobs, &ctx, "Outcome");
    frame(&mut state, &mut jobs, &ctx, vec![]);
    click(&mut state, &mut jobs, &ctx, "Outcome: 1");
    frame(&mut state, &mut jobs, &ctx, vec![]);
    click(&mut state, &mut jobs, &ctx, "Grouping variable");
    frame(&mut state, &mut jobs, &ctx, vec![]);
    click(&mut state, &mut jobs, &ctx, "Grouping variable: 3");
    assert!(state.error().is_none(), "{:?}", state.error());
    assert!(matches!(
        state.draft(),
        AnalysisDraft::IndependentT {
            outcome: Some(ColumnId(1)),
            group: Some(ColumnId(3)),
            ..
        }
    ));
    state
        .handle_event(JobEvent::Completed {
            token,
            cancelled: false,
            result: Ok(JobOutput::GroupValues(Arc::new(GroupLookupResult {
                column: ColumnId(3),
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
    assert!(state.group_lookup().is_none_or(|r| r.role_epoch != epoch));
    settle(&mut state, &mut jobs);
}

#[test]
fn exposed_filtered_weighted_two_factor_history_survives_fresh_open_and_report() {
    let mut state = regression_state();
    for (column, values) in [
        (ColumnId(3), [" a ", "b"]),
        (ColumnId(4), ["north", "south"]),
    ] {
        let mut metadata = state
            .project()
            .dataset()
            .column(column)
            .unwrap()
            .metadata()
            .clone();
        metadata.value_labels = values
            .into_iter()
            .map(|value| tessera::data::ValueLabel {
                value: Cell::Text(value.into()),
                label: "Same label".into(),
            })
            .collect();
        state = state
            .apply_action(UiAction::Metadata { column, metadata })
            .unwrap();
    }
    let AnalysisRequest::Regression(mut request) = fixture::request() else {
        panic!()
    };
    request.predictors.push(RegressionPredictor {
        column: ColumnId(4),
        role: PredictorRole::Categorical {
            reference: Cell::Text("north".into()),
        },
    });
    request.frequency_weight = Some(ColumnId(2));
    request.filter.predicates.push(RowPredicate::Number {
        column: ColumnId(2),
        comparison: NumericComparison::LessOrEqual,
        value: 5.,
    });
    state = state
        .apply_action(UiAction::SetDraft(AnalysisDraft::from_request(
            AnalysisRequest::Regression(request),
        )))
        .unwrap();
    let dataset = state.project().dataset().revision();
    let mut jobs = JobRunner::default();
    let project = Arc::clone(state.project());
    let request = state.checked_draft_request().unwrap();
    super::run(
        &mut state,
        &mut jobs,
        JobOperation::Analyze { project, request },
    );
    let CompletedAnalysis::Regression(capture) = &state.selected_run().unwrap().analysis else {
        panic!()
    };
    assert_eq!((capture.fitted_rows, capture.frequency), (5, 15));
    assert_eq!(capture.terms.len(), 4);
    assert!(matches!(capture.outcome, RegressionOutcome::Fitted(_)));
    assert_eq!(dataset, state.project().dataset().revision());
    let dir = std::env::temp_dir()
        .canonicalize()
        .unwrap()
        .join(format!("tessera-task10-flow-{}", std::process::id()));
    std::fs::create_dir(&dir).unwrap();
    let path = dir.join("saved.tessera");
    let project = Arc::clone(state.project());
    super::run(
        &mut state,
        &mut jobs,
        JobOperation::Save {
            project,
            path: path.clone(),
            expected: None,
        },
    );
    let before = Arc::clone(state.project());
    let mut reopened = DesktopState::new().unwrap();
    super::run(&mut reopened, &mut jobs, JobOperation::Open { path });
    assert_eq!(before.runs(), reopened.project().runs());
    for column in before.dataset().columns() {
        assert_eq!(
            column.metadata(),
            reopened
                .project()
                .dataset()
                .column(column.id())
                .unwrap()
                .metadata()
        );
        for row in 0..before.dataset().row_count() {
            assert_eq!(
                column.cell(row).unwrap(),
                reopened.project().dataset().cell(column.id(), row).unwrap()
            );
        }
    }
    assert!(!reopened.is_dirty());
    let project = Arc::clone(reopened.project());
    let report = dir.join("report.html");
    super::run(
        &mut reopened,
        &mut jobs,
        JobOperation::ExportReport {
            project,
            path: report.clone(),
            runs: vec![RunId(2)],
        },
    );
    let html = std::fs::read_to_string(report).unwrap();
    for expected in [
        "Same label",
        "north",
        "south",
        "First factor",
        "Second factor",
        "Fitted source rows M: 5",
    ] {
        assert!(html.contains(expected), "{expected}");
    }
    assert_eq!(before.runs(), reopened.project().runs());
    std::fs::remove_dir_all(dir).unwrap();
}

#[test]
fn wide_regression_pending_new_and_close_cancel_without_publishing() {
    for pending in [PendingAction::New, PendingAction::Close] {
        let columns = (1..=32)
            .map(|id| {
                Column::numeric(
                    ColumnId(id),
                    &format!("X{id}"),
                    (0..20_000)
                        .map(|row| Some(((row * (id as usize + 1)) % 101) as f64))
                        .collect(),
                )
                .unwrap()
            })
            .collect();
        let project = Project::new(
            "Disposable wide cancellation",
            Dataset::new(columns).unwrap(),
        )
        .unwrap();
        let mut state = DesktopState::from_project(project).unwrap();
        let before = Arc::clone(state.project());
        let mut jobs = JobRunner::default();
        let request = AnalysisRequest::Regression(RegressionRequest {
            outcome: ColumnId(1),
            predictors: (2..=32)
                .map(|id| RegressionPredictor {
                    column: ColumnId(id),
                    role: PredictorRole::Numeric,
                })
                .collect(),
            confidence: ConfidenceLevel::P95,
            filter: RowFilter::default(),
            frequency_weight: None,
        });
        state
            .start_operation(
                &mut jobs,
                JobOperation::Analyze {
                    project: Arc::clone(&before),
                    request,
                },
            )
            .unwrap();
        state = state.apply_action(UiAction::Request(pending)).unwrap();
        let ctx = egui::Context::default();
        frame(&mut state, &mut jobs, &ctx, vec![]);
        settle(&mut state, &mut jobs);
        assert!(state.project().runs().is_empty());
        assert!(Arc::ptr_eq(state.project(), &before));
        assert!(state.pending().is_some());
        assert_eq!(state.dialog(), Some(Dialog::Unsaved));
        state = state
            .apply_action(UiAction::ResolveUnsaved(UnsavedChoice::Cancel))
            .unwrap();
        assert!(state.pending().is_none());
        assert!(Arc::ptr_eq(state.project(), &before));
    }
}
