use super::*;
use tessera::{
    desktop::Workspace,
    jobs::{JobEvent, JobOutput},
    survey::{generate_with_budget, SurveyChoice, SurveyRequest, SurveyRole},
};

#[test]
fn survey_review_generate_and_revision_invalidation() {
    let mut state = imported();
    let mut jobs = JobRunner::default();
    let before = Arc::clone(state.project());
    state.enter_survey().unwrap();
    state.service_survey(&mut jobs).unwrap();
    finish(&mut state, &mut jobs);
    assert!(state.survey_review().is_some());
    assert!(Arc::ptr_eq(&before, state.project()));
    state
        .set_survey_role(ColumnId(1), SurveyRole::Exclude)
        .unwrap();
    state
        .set_survey_role(ColumnId(2), SurveyRole::NumericMeasure)
        .unwrap();
    assert!(state.survey_ready());
    state.submit_survey(&mut jobs).unwrap();
    finish(&mut state, &mut jobs);
    assert!(state.error().is_none(), "{:?}", state.error());
    assert_eq!(state.project().runs().len(), 2);
    assert_eq!(state.survey_run_ids().len(), 2);
    assert!(std::ptr::eq(before.dataset(), state.project().dataset()));
    assert_eq!(state.workspace(), Workspace::Results);
    assert_eq!(state.selected_run().unwrap().id, state.survey_run_ids()[0]);
    state = state
        .apply_action(UiAction::EditCell {
            column: ColumnId(2),
            row: 0,
            value: Cell::Number(99.),
        })
        .unwrap();
    assert!(state.survey_review().is_none());
    assert!(state.survey_run_ids().is_empty());
    state = state.apply_action(UiAction::Undo).unwrap();
    assert!(state.survey_review().is_none());
}

#[test]
fn survey_rejects_old_project_and_unresolved_cell_edits() {
    let mut state = imported();
    let old = Arc::clone(state.project());
    state = state
        .apply_action(UiAction::EditCell {
            column: ColumnId(2),
            row: 0,
            value: Cell::Number(99.),
        })
        .unwrap();
    let mut jobs = JobRunner::default();
    assert!(state
        .start_operation(&mut jobs, JobOperation::InspectSurvey { project: old })
        .is_err());
    state = state
        .apply_action(UiAction::BeginCell {
            column: ColumnId(2),
            row: 0,
        })
        .unwrap();
    state.set_cell_text("not a number".into()).unwrap();
    assert!(state.enter_survey().is_err());
    assert!(!jobs.is_running());
    assert!(state
        .start_operation(
            &mut jobs,
            JobOperation::InspectSurvey {
                project: Arc::clone(state.project())
            }
        )
        .is_err());
    assert_eq!(
        state.project().dataset().cell(ColumnId(2), 0).unwrap(),
        Cell::Number(99.)
    );
}

#[test]
fn survey_stale_cancelled_and_bad_candidate_do_not_publish() {
    let mut state = imported();
    let before = Arc::clone(state.project());
    let request = SurveyRequest {
        revision: before.dataset().revision(),
        choices: vec![SurveyChoice {
            column: ColumnId(2),
            role: SurveyRole::NumericMeasure,
        }],
    };
    let candidate =
        Arc::new(generate_with_budget(&before, &request, &JobControl::default(), &[], 0).unwrap());
    let mut stale = state.token();
    stale.generation += 1;
    for (token, cancelled) in [(stale, false), (state.token(), true)] {
        state
            .handle_event(JobEvent::Completed {
                token,
                cancelled,
                result: Ok(JobOutput::SurveyReport(Arc::clone(&candidate))),
            })
            .unwrap();
        assert!(Arc::ptr_eq(&before, state.project()));
    }
    let other =
        Arc::new(tessera::project::Project::new("Other", common::sample_dataset()).unwrap());
    assert!(state
        .handle_event(JobEvent::Completed {
            token: state.token(),
            cancelled: false,
            result: Ok(JobOutput::SurveyReport(other))
        })
        .is_err());
    assert!(Arc::ptr_eq(&before, state.project()));
    assert!(state.survey_run_ids().is_empty());
}

#[test]
fn survey_import_continues_only_after_success() {
    let mut state = DesktopState::new().unwrap();
    state.enter_survey().unwrap();
    assert!(state.survey_import_requested());
    let input = Arc::new(
        preview(
            Arc::from(&b"Choice\nYes\nNo\n"[..]),
            Delimiter::Comma,
            &JobControl::default(),
        )
        .unwrap(),
    );
    let choices = input.default_choices().unwrap();
    let mut jobs = JobRunner::default();
    run(
        &mut state,
        &mut jobs,
        JobOperation::CommitImport {
            preview: input,
            choices,
        },
    );
    finish(&mut state, &mut jobs);
    assert!(state.survey_review().is_some());
    assert!(!state.survey_import_requested());
}

#[test]
fn survey_second_batch_preserves_prefix_and_failed_batch() {
    let mut state = imported();
    let mut jobs = JobRunner::default();
    for _ in 0..2 {
        state.enter_survey().unwrap();
        state.service_survey(&mut jobs).unwrap();
        finish(&mut state, &mut jobs);
        state
            .set_survey_role(ColumnId(1), SurveyRole::Exclude)
            .unwrap();
        state
            .set_survey_role(ColumnId(2), SurveyRole::NumericMeasure)
            .unwrap();
        let before = Arc::clone(state.project());
        state.submit_survey(&mut jobs).unwrap();
        finish(&mut state, &mut jobs);
        assert!(state.error().is_none());
        assert!(before
            .runs()
            .iter()
            .zip(state.project().runs())
            .all(|(a, b)| Arc::ptr_eq(a, b)));
        assert_eq!(
            state.survey_run_ids(),
            &state.project().runs()[before.runs().len()..]
                .iter()
                .map(|r| r.id)
                .collect::<Vec<_>>()
        );
    }
    let ids = state.survey_run_ids().to_vec();
    let key = tessera::figures::FigureKey {
        run: ids[1].0,
        kind: tessera::figures::FigureKind::Histogram,
        variant: 0,
    };
    state
        .apply_figure_settings(
            &key,
            tessera::figures::FigureSettings {
                title: "Edited survey figure".into(),
                ..Default::default()
            },
        )
        .unwrap();
    assert_eq!(state.survey_run_ids(), ids);
    assert_eq!(state.survey_export_runs().unwrap(), ids);
    let before = Arc::clone(state.project());
    state
        .start_operation(
            &mut jobs,
            JobOperation::SurveyReport {
                project: Arc::clone(&before),
                request: SurveyRequest {
                    revision: before.dataset().revision(),
                    choices: vec![SurveyChoice {
                        column: ColumnId(1),
                        role: SurveyRole::NumericMeasure,
                    }],
                },
            },
        )
        .unwrap();
    finish(&mut state, &mut jobs);
    assert!(state.error().is_some());
    assert!(Arc::ptr_eq(&before, state.project()));
    assert_eq!(state.survey_run_ids(), ids);
    run(
        &mut state,
        &mut jobs,
        JobOperation::Analyze {
            project: Arc::clone(&before),
            request: request().into(),
        },
    );
    assert_eq!(state.survey_run_ids(), ids);
}

#[test]
fn survey_cancellation_and_import_cancel_keep_source() {
    let mut state = imported();
    let before = Arc::clone(state.project());
    let mut jobs = JobRunner::default();
    state.enter_survey().unwrap();
    state.service_survey(&mut jobs).unwrap();
    jobs.cancel();
    finish(&mut state, &mut jobs);
    assert!(state.survey_review().is_none());
    assert!(Arc::ptr_eq(&before, state.project()));
    let mut empty = DesktopState::new().unwrap();
    empty.enter_survey().unwrap();
    empty.cancel_survey_import();
    assert!(!empty.survey_import_requested());
    empty.service_survey(&mut jobs).unwrap();
    assert!(!jobs.is_running());
}

#[test]
fn survey_other_import_clears_review_and_old_completion() {
    let mut state = imported();
    let mut jobs = JobRunner::default();
    state.enter_survey().unwrap();
    state.service_survey(&mut jobs).unwrap();
    finish(&mut state, &mut jobs);
    let review = Arc::clone(state.survey_review().unwrap());
    let token = state.token();
    let input = Arc::new(
        preview(
            Arc::from(&b"Other\nA\nB\n"[..]),
            Delimiter::Comma,
            &JobControl::default(),
        )
        .unwrap(),
    );
    let choices = input.default_choices().unwrap();
    run(
        &mut state,
        &mut jobs,
        JobOperation::CommitImport {
            preview: input,
            choices,
        },
    );
    assert!(state.survey_review().is_none());
    assert!(state.survey_run_ids().is_empty());
    state
        .handle_event(JobEvent::Completed {
            token,
            cancelled: false,
            result: Ok(JobOutput::SurveyReview(review)),
        })
        .unwrap();
    assert!(state.survey_review().is_none());
}

#[test]
fn survey_actual_export_excludes_prior_batch_escapes_and_reopens_figure() {
    let directory = Temp::new();
    let input = Arc::new(
        preview(
            Arc::from(&b"<script>header</script>\n<img src=x onerror=evil>\nSafe\n"[..]),
            Delimiter::Comma,
            &JobControl::default(),
        )
        .unwrap(),
    );
    let mut state = DesktopState::new().unwrap();
    let mut jobs = JobRunner::default();
    let choices = input.default_choices().unwrap();
    run(
        &mut state,
        &mut jobs,
        JobOperation::CommitImport {
            preview: input,
            choices,
        },
    );
    let mut old = Vec::new();
    for batch in 0..2 {
        state.enter_survey().unwrap();
        state.service_survey(&mut jobs).unwrap();
        finish(&mut state, &mut jobs);
        state
            .set_survey_role(ColumnId(1), SurveyRole::Categories)
            .unwrap();
        state.submit_survey(&mut jobs).unwrap();
        finish(&mut state, &mut jobs);
        assert!(state.error().is_none(), "{:?}", state.error());
        if batch == 0 {
            old = state.survey_export_runs().unwrap();
        }
    }
    let runs = state.survey_export_runs().unwrap();
    assert!(runs.iter().all(|id| !old.contains(id)));
    let key = tessera::figures::FigureKey {
        run: runs[0].0,
        kind: tessera::figures::FigureKind::Frequency(1),
        variant: 0,
    };
    let settings = tessera::figures::FigureSettings {
        title: "Edited <survey> chart".into(),
        ..Default::default()
    };
    state.apply_figure_settings(&key, settings.clone()).unwrap();
    let path = directory.0.join("selected.html");
    let project = Arc::clone(state.project());
    run(
        &mut state,
        &mut jobs,
        JobOperation::ExportReport {
            project,
            path: path.clone(),
            runs: runs.clone(),
        },
    );
    let html = fs::read_to_string(path).unwrap();
    assert!(html.contains("&lt;script&gt;header&lt;/script&gt;"));
    assert!(html.contains("&lt;img src=x onerror=evil&gt;"));
    assert!(!html.contains("<script>header</script>"));
    assert!(!html.contains("<img src=x onerror=evil>"));
    for id in &old {
        assert!(
            !html.contains(&format!("id=\"run-{}\"", id.0)),
            "old run exported"
        );
    }
    for id in &runs {
        assert!(
            html.contains(&format!("id=\"run-{}\"", id.0)),
            "current run absent"
        );
    }
    let path = directory.0.join("survey.tessera");
    tessera::files::save_project(&path, state.project(), None, &JobControl::default()).unwrap();
    let (reopened, _) = read_project(&path, &JobControl::default()).unwrap();
    assert_eq!(reopened.figure_settings(&key), settings);
    assert_eq!(reopened.runs(), state.project().runs());
}
