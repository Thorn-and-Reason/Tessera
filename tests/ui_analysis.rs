#![cfg(feature = "desktop")]
use tessera::{
    analysis::{DescribeRequest, MissingPolicy, RowFilter, Statistic},
    data::{Column, ColumnId, Dataset},
    desktop::{
        draft::{AnalysisDraft, Procedure},
        DesktopState, UiAction,
    },
    project::Project,
};
fn state() -> DesktopState {
    DesktopState::from_project(
        Project::new(
            "Test",
            Dataset::new(vec![
                Column::numeric(ColumnId(1), "Score", vec![Some(1.0)]).unwrap(),
                Column::text(ColumnId(2), "Group", vec![Some("A".into())]).unwrap(),
            ])
            .unwrap(),
        )
        .unwrap(),
    )
    .unwrap()
}
fn describe(columns: Vec<ColumnId>) -> AnalysisDraft {
    DescribeRequest {
        columns,
        statistics: vec![Statistic::Mean],
        missing_policy: MissingPolicy::PerVariable,
        filter: RowFilter::default(),
        frequency_weight: None,
    }
    .into()
}
#[test]
fn missing_roles_explain_correction_for_every_procedure() {
    let mut state = state();
    let draft = describe(vec![]);
    for procedure in Procedure::ALL {
        state = state
            .apply_action(UiAction::SetDraft(draft.for_procedure(procedure)))
            .unwrap();
        let message = state.checked_draft_request().unwrap_err().to_string();
        assert!(message.contains("Select"), "{procedure:?}: {message}");
        assert!(!message.contains("retained analysis"));
    }
}
#[test]
fn unavailable_and_wrong_type_variables_have_actionable_errors() {
    let mut state = state();
    for (id, expected) in [(ColumnId(99), "unavailable"), (ColumnId(2), "numeric")] {
        state = state
            .apply_action(UiAction::SetDraft(describe(vec![id])))
            .unwrap();
        assert!(state
            .checked_draft_request()
            .unwrap_err()
            .to_string()
            .contains(expected));
    }
    state = state
        .apply_action(UiAction::SetDraft(describe(vec![ColumnId(1)])))
        .unwrap();
    assert!(state.checked_draft_request().is_ok());
}
#[test]
fn reference_mean_and_weight_errors_are_specific() {
    let mut state = state();
    let mut draft = describe(vec![ColumnId(1)]).for_procedure(Procedure::OneSampleT);
    if let AnalysisDraft::OneSampleT {
        variable,
        reference_text,
        ..
    } = &mut draft
    {
        *variable = Some(ColumnId(1));
        *reference_text = "NaN".into();
    }
    state = state.apply_action(UiAction::SetDraft(draft)).unwrap();
    assert!(state
        .checked_draft_request()
        .unwrap_err()
        .to_string()
        .contains("finite reference mean"));
    let AnalysisDraft::Describe(mut request) = describe(vec![ColumnId(1)]) else {
        unreachable!()
    };
    request.frequency_weight = Some(ColumnId(2));
    state = state
        .apply_action(UiAction::SetDraft(request.into()))
        .unwrap();
    assert!(state
        .checked_draft_request()
        .unwrap_err()
        .to_string()
        .contains("frequency weight"));
}
#[test]
fn every_saved_recipe_still_runs_through_the_draft_boundary() {
    use tessera::{operation::JobControl, project_io};
    for bytes in [
        &include_bytes!("../examples/mixed-analysis.tessera")[..],
        &include_bytes!("../examples/exploration.tessera")[..],
    ] {
        let project = project_io::decode(bytes, &JobControl::default()).unwrap();
        for run in project.runs() {
            let state = DesktopState::from_project(project.clone())
                .unwrap()
                .apply_action(UiAction::SetDraft(AnalysisDraft::from_request(
                    run.analysis.request().clone_owned(),
                )))
                .unwrap();
            assert_eq!(
                state.checked_draft_request().unwrap(),
                run.analysis.request()
            );
        }
    }
}
#[test]
fn invalid_filter_and_duplicate_roles_explain_the_correction() {
    use tessera::analysis::{NumericComparison, RowPredicate};
    let mut state = state();
    let AnalysisDraft::Describe(mut request) = describe(vec![ColumnId(1)]) else {
        unreachable!()
    };
    request.filter.predicates.push(RowPredicate::Number {
        column: ColumnId(1),
        comparison: NumericComparison::Equal,
        value: f64::INFINITY,
    });
    state = state
        .apply_action(UiAction::SetDraft(request.into()))
        .unwrap();
    assert!(state
        .checked_draft_request()
        .unwrap_err()
        .to_string()
        .contains("finite numeric filter"));
    let mut draft = describe(vec![]).for_procedure(Procedure::PairedT);
    if let AnalysisDraft::PairedT { first, second, .. } = &mut draft {
        *first = Some(ColumnId(1));
        *second = Some(ColumnId(1));
    }
    state = state.apply_action(UiAction::SetDraft(draft)).unwrap();
    assert!(state
        .checked_draft_request()
        .unwrap_err()
        .to_string()
        .contains("different numeric variables"));
}
#[test]
fn disabled_run_and_keyboard_submission_share_draft_guidance() {
    use eframe::egui::{self, Event, Key, Modifiers};
    use tessera::{
        desktop::{configure_context, render, Workspace},
        jobs::JobRunner,
    };
    let mut state = state()
        .apply_action(UiAction::Navigate(Workspace::Analyses))
        .unwrap();
    let context = egui::Context::default();
    configure_context(&context);
    let mut jobs = JobRunner::default();
    let frame = |state: &mut DesktopState, jobs: &mut JobRunner, events| {
        let mut output = context.run_ui(
            egui::RawInput {
                screen_rect: Some(egui::Rect::from_min_size(
                    egui::Pos2::ZERO,
                    egui::vec2(1180.0, 780.0),
                )),
                events,
                ..Default::default()
            },
            |ui| render(ui, state, jobs),
        );
        output.textures_delta.clear();
    };
    frame(&mut state, &mut jobs, vec![]);
    let position = state.control_rect("Run descriptives").unwrap().center();
    for pressed in [true, false] {
        frame(
            &mut state,
            &mut jobs,
            vec![
                Event::PointerMoved(position),
                Event::PointerButton {
                    pos: position,
                    button: egui::PointerButton::Primary,
                    pressed,
                    modifiers: Modifiers::NONE,
                },
            ],
        );
    }
    assert!(!jobs.is_running());
    assert!(state.error().is_none());
    frame(
        &mut state,
        &mut jobs,
        vec![Event::Key {
            key: Key::Enter,
            physical_key: None,
            pressed: true,
            repeat: false,
            modifiers: Modifiers::COMMAND,
        }],
    );
    assert!(!jobs.is_running());
    assert!(state
        .error()
        .unwrap()
        .contains("Select at least one numeric variable"));
}
#[test]
fn settings_footer_is_reachable_at_minimum_size_and_editor_stays_single_after_resize() {
    use eframe::egui::{self, Event, Modifiers};
    use tessera::{
        desktop::{configure_context, render, Dialog, Workspace},
        jobs::JobRunner,
    };
    let mut state = state()
        .apply_action(UiAction::Navigate(Workspace::Analyses))
        .unwrap();
    let context = egui::Context::default();
    configure_context(&context);
    let mut jobs = JobRunner::default();
    let frame = |state: &mut DesktopState, jobs: &mut JobRunner, width, events| {
        let mut output = context.run_ui(
            egui::RawInput {
                screen_rect: Some(egui::Rect::from_min_size(
                    egui::Pos2::ZERO,
                    egui::vec2(width, 440.0),
                )),
                events,
                ..Default::default()
            },
            |ui| render(ui, state, jobs),
        );
        output.textures_delta.clear();
        output
    };
    let _ = frame(&mut state, &mut jobs, 640.0, vec![]);
    let position = state.control_rect("Settings").unwrap().center();
    for pressed in [true, false] {
        let _ = frame(
            &mut state,
            &mut jobs,
            640.0,
            vec![
                Event::PointerMoved(position),
                Event::PointerButton {
                    pos: position,
                    button: egui::PointerButton::Primary,
                    pressed,
                    modifiers: Modifiers::NONE,
                },
            ],
        );
    }
    assert_eq!(state.dialog(), Some(Dialog::Settings));
    for procedure in Procedure::ALL {
        state = state
            .apply_action(UiAction::SetDraft(
                describe(vec![]).for_procedure(procedure),
            ))
            .unwrap();
        let _ = frame(&mut state, &mut jobs, 640.0, vec![]);
        let _ = frame(&mut state, &mut jobs, 640.0, vec![]);
        let label = if procedure == Procedure::Describe {
            "Run descriptives"
        } else {
            "Run analysis"
        };
        let footer = state.control_rect(label).unwrap();
        assert!(
            footer.min.y >= 0.0 && footer.max.y <= 440.0,
            "{procedure:?}: {footer:?}"
        );
    }
    let output = frame(&mut state, &mut jobs, 1180.0, vec![]);
    let editor_titles = output
        .shapes
        .iter()
        .filter(|shape| match &shape.shape {
            egui::epaint::Shape::Text(text) => text.galley.text() == "New analysis",
            _ => false,
        })
        .count();
    assert_eq!(
        editor_titles, 1,
        "A resized settings dialog must not duplicate the inspector"
    );
}
#[test]
fn reference_mean_keeps_focus_when_saved_settings_become_an_invalid_draft() {
    use eframe::egui::{self, Event};
    use tessera::{
        analysis::AnalysisRequestRef,
        desktop::{configure_context, render, Workspace},
        jobs::JobRunner,
        operation::JobControl,
        project_io,
    };
    let project = project_io::decode(
        &include_bytes!("../examples/mixed-analysis.tessera")[..],
        &JobControl::default(),
    )
    .unwrap();
    let run = project
        .runs()
        .iter()
        .find(|run| matches!(run.analysis.request(), AnalysisRequestRef::OneSampleT(_)))
        .unwrap();
    let mut state = DesktopState::from_project(project.clone())
        .unwrap()
        .apply_action(UiAction::Navigate(Workspace::Analyses))
        .unwrap()
        .apply_action(UiAction::SelectRun(run.id))
        .unwrap()
        .apply_action(UiAction::SetDraft(AnalysisDraft::from_request(
            run.analysis.request().clone_owned(),
        )))
        .unwrap();
    let context = egui::Context::default();
    configure_context(&context);
    let mut jobs = JobRunner::default();
    let frame = |state: &mut DesktopState, jobs: &mut JobRunner, events| {
        let mut output = context.run_ui(
            egui::RawInput {
                screen_rect: Some(egui::Rect::from_min_size(
                    egui::Pos2::ZERO,
                    egui::vec2(1180.0, 780.0),
                )),
                events,
                ..Default::default()
            },
            |ui| render(ui, state, jobs),
        );
        output.textures_delta.clear();
    };
    frame(&mut state, &mut jobs, vec![]);
    let original = state.control_id("Reference mean").unwrap();
    context.memory_mut(|memory| memory.request_focus(original));
    frame(&mut state, &mut jobs, vec![Event::Text("X".into())]);
    frame(&mut state, &mut jobs, vec![Event::Text("Y".into())]);
    assert_eq!(state.control_id("Reference mean"), Some(original));
    assert!(context.memory(|memory| memory.has_focus(original)));
    let AnalysisDraft::OneSampleT { reference_text, .. } = state.draft() else {
        unreachable!()
    };
    assert!(
        reference_text.contains("XY"),
        "Both keystrokes must stay in the field: {reference_text}"
    );
    assert!(state
        .checked_draft_request()
        .unwrap_err()
        .to_string()
        .contains("finite reference mean"));
}
