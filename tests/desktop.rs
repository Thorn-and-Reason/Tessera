#![cfg(feature = "desktop")]
#[path = "desktop/export_confirmation.rs"]
mod export_confirmation;
#[path = "desktop/guidance.rs"]
mod guidance;
#[path = "desktop/survey_improvements.rs"]
mod survey_improvements;
#[path = "desktop/ux_access.rs"]
mod ux_access;
#[path = "desktop/ux_analysis.rs"]
mod ux_analysis;
#[path = "desktop/ux_import.rs"]
mod ux_import;
#[path = "desktop/ux_preparation.rs"]
mod ux_preparation;
#[path = "desktop/ux_shell.rs"]
mod ux_shell;
#[path = "desktop/ux_state.rs"]
mod ux_state;
use eframe::egui::{self, Event, Key, Modifiers, Pos2, Rect, Vec2};
use std::{
    sync::Arc,
    time::{Duration, Instant},
};
use tessera::{
    analysis::{describe, DescribeRequest, MissingPolicy, RowFilter, Statistic},
    data::{Cell, Column, ColumnId, Dataset},
    desktop::{render, DesktopState, Dialog, PendingAction, UiAction, UnsavedChoice, Workspace},
    jobs::{JobEvent, JobOperation, JobOutput, JobRunner},
    operation::{JobControl, RunId},
    project::Project,
};
fn state() -> DesktopState {
    DesktopState::from_project(
        Project::new(
            "Prifysgol",
            Dataset::new(vec![
                Column::numeric(
                    ColumnId(1),
                    "Sgôr ŵ",
                    vec![Some(17.0), Some(25.0), Some(25.0)],
                )
                .unwrap(),
                Column::text(
                    ColumnId(2),
                    "Enw",
                    vec![Some("".into()), None, Some("Carys".into())],
                )
                .unwrap(),
            ])
            .unwrap(),
        )
        .unwrap(),
    )
    .unwrap()
}
fn request() -> DescribeRequest {
    DescribeRequest {
        columns: vec![ColumnId(1)],
        statistics: vec![Statistic::Mean, Statistic::SampleStdDev],
        missing_policy: MissingPolicy::PerVariable,
        filter: RowFilter::default(),
        frequency_weight: None,
    }
}
fn run(state: &mut DesktopState, jobs: &mut JobRunner, operation: JobOperation) {
    state.start_operation(jobs, operation).unwrap();
    let until = Instant::now() + Duration::from_secs(10);
    while jobs.is_running() {
        state.poll(jobs);
        assert!(Instant::now() < until);
        std::thread::yield_now();
    }
    assert!(state.error().is_none(), "{:?}", state.error());
}
#[test]
fn native_workflow_actions_preserve_data() {
    let dir = std::env::temp_dir()
        .canonicalize()
        .unwrap()
        .join(format!("tessera-desktop-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let path = dir.join("workflow.tessera");
    let report = dir.join("workflow.html");
    let _ = std::fs::remove_file(&path);
    let _ = std::fs::remove_file(&report);
    let mut state = state();
    let mut jobs = JobRunner::default();
    assert_eq!(state.workspace(), Workspace::Data);
    state = state
        .apply_action(UiAction::EditCell {
            column: ColumnId(1),
            row: 0,
            value: Cell::Number(20.125000000000004),
        })
        .unwrap();
    assert_eq!(
        state.project().dataset().cell(ColumnId(2), 0).unwrap(),
        Cell::Text("".into())
    );
    assert_eq!(
        state.project().dataset().cell(ColumnId(2), 1).unwrap(),
        Cell::Missing
    );
    let project = Arc::clone(state.project());
    run(
        &mut state,
        &mut jobs,
        JobOperation::Analyze {
            project,
            request: request().into(),
        },
    );
    assert_eq!(state.project().runs().len(), 1);
    assert_eq!(state.workspace(), Workspace::Results);
    assert!(state.is_dirty());
    let project = Arc::clone(state.project());
    run(
        &mut state,
        &mut jobs,
        JobOperation::Save {
            project,
            path: path.clone(),
            expected: None,
        },
    );
    assert!(!state.is_dirty());
    let revision = state.project().dataset().revision();
    run(
        &mut state,
        &mut jobs,
        JobOperation::Open { path: path.clone() },
    );
    assert_eq!(state.project().dataset().revision(), revision);
    assert_eq!(
        state.project().dataset().cell(ColumnId(1), 0).unwrap(),
        Cell::Number(20.125000000000004)
    );
    let project = Arc::clone(state.project());
    run(
        &mut state,
        &mut jobs,
        JobOperation::ExportReport {
            project,
            path: report.clone(),
            runs: vec![RunId(1)],
        },
    );
    assert!(std::fs::read_to_string(report).unwrap().contains("Sgôr ŵ"));
    std::fs::remove_dir_all(dir).unwrap();
}
#[test]
fn unsaved_save_discard_cancel() {
    let before = state();
    let pending = before
        .apply_action(UiAction::Request(PendingAction::New))
        .unwrap();
    assert!(pending.pending().is_some());
    let cancelled = pending
        .apply_action(UiAction::ResolveUnsaved(UnsavedChoice::Cancel))
        .unwrap();
    assert!(Arc::ptr_eq(cancelled.project(), before.project()));
    let mut discarded = pending
        .apply_action(UiAction::ResolveUnsaved(UnsavedChoice::Discard))
        .unwrap();
    assert_eq!(discarded.document_id(), before.document_id());
    finish_pending_frame(&mut discarded);
    assert_eq!(discarded.project().dataset().row_count(), 0);
    assert_ne!(discarded.document_id(), before.document_id());
    let saving = pending
        .apply_action(UiAction::ResolveUnsaved(UnsavedChoice::Save))
        .unwrap();
    assert!(saving.pending().is_some());
    assert!(saving.save_requested());
}
#[test]
fn selected_run_and_draft_settings_are_distinct() {
    let mut state = state();
    let original = request();
    let analysis = describe(state.project().dataset(), &original, &JobControl::default()).unwrap();
    state
        .handle_event(JobEvent::Completed {
            token: state.token(),
            cancelled: false,
            result: Ok(JobOutput::Analyzed(analysis.into())),
        })
        .unwrap();
    state = state
        .apply_action(UiAction::SetDraft(
            DescribeRequest {
                statistics: vec![Statistic::Sum],
                ..original.clone()
            }
            .into(),
        ))
        .unwrap();
    assert_eq!(
        state.selected_run().unwrap().analysis.request(),
        tessera::analysis::AnalysisRequestRef::Describe(&original)
    );
    state = state
        .apply_action(UiAction::EditCell {
            column: ColumnId(1),
            row: 0,
            value: Cell::Number(18.0),
        })
        .unwrap();
    assert!(state.selected_run_is_stale());
    assert_eq!(
        state.selected_run().unwrap().analysis.request(),
        tessera::analysis::AnalysisRequestRef::Describe(&original)
    );
}
fn finish_pending_frame(state: &mut DesktopState) {
    let ctx = egui::Context::default();
    tessera::desktop::configure_context(&ctx);
    frame(
        &ctx,
        state,
        &mut JobRunner::default(),
        Vec2::new(1180., 780.),
        vec![],
    );
}
fn frame(
    ctx: &egui::Context,
    state: &mut DesktopState,
    jobs: &mut JobRunner,
    size: Vec2,
    events: Vec<Event>,
) {
    let mut output = ctx.run_ui(
        egui::RawInput {
            screen_rect: Some(Rect::from_min_size(Pos2::ZERO, size)),
            events,
            ..Default::default()
        },
        |ui| render(ui, state, jobs),
    );
    output.textures_delta.clear();
}
fn click(
    ctx: &egui::Context,
    state: &mut DesktopState,
    jobs: &mut JobRunner,
    label: &str,
    size: Vec2,
) {
    let p = state.control_rect(label).unwrap().center();
    frame(
        ctx,
        state,
        jobs,
        size,
        vec![
            Event::PointerMoved(p),
            Event::PointerButton {
                pos: p,
                button: egui::PointerButton::Primary,
                pressed: true,
                modifiers: Modifiers::NONE,
            },
        ],
    );
    frame(
        ctx,
        state,
        jobs,
        size,
        vec![Event::PointerButton {
            pos: p,
            button: egui::PointerButton::Primary,
            pressed: false,
            modifiers: Modifiers::NONE,
        }],
    );
    for _ in 0..2 {
        frame(ctx, state, jobs, size, vec![]);
    }
}
#[test]
fn narrow_layout_keeps_panels_accessible() {
    for scale in [1.0, 2.0] {
        for size in [Vec2::new(760.0, 520.0), Vec2::new(1180.0, 780.0)] {
            let ctx = egui::Context::default();
            tessera::desktop::configure_context(&ctx);
            ctx.set_pixels_per_point(scale);
            let mut state = state();
            let mut jobs = JobRunner::default();
            for _ in 0..3 {
                frame(&ctx, &mut state, &mut jobs, size, vec![]);
            }
            for name in ["Variables", "Prepare", "Analyses", "Results", "Data"] {
                click(&ctx, &mut state, &mut jobs, name, size);
                assert_eq!(
                    state.workspace().label(),
                    name,
                    "scale {scale} size {size:?} control {:?}",
                    state.control_rect(name)
                );
            }
            assert!(state.control_rect("Variable details").is_some());
            click(&ctx, &mut state, &mut jobs, "Variable details", size);
            assert_eq!(state.workspace(), Workspace::Variables);
            assert!(
                state.control_rect("Apply properties").is_some(),
                "scale {scale} size {size:?} should show editable variable details"
            );
        }
    }
}
#[test]
fn unicode_precision_and_focus() {
    let mut state = state();
    let mut jobs = JobRunner::default();
    let ctx = egui::Context::default();
    tessera::desktop::configure_context(&ctx);
    let size = Vec2::new(760.0, 520.0);
    state = state
        .apply_action(UiAction::BeginCell {
            column: ColumnId(1),
            row: 0,
        })
        .unwrap();
    frame(&ctx, &mut state, &mut jobs, size, vec![]);
    state.set_cell_text("0.12345678901234566".into()).unwrap();
    frame(
        &ctx,
        &mut state,
        &mut jobs,
        size,
        vec![Event::Key {
            key: Key::Enter,
            physical_key: None,
            pressed: true,
            repeat: false,
            modifiers: Modifiers::NONE,
        }],
    );
    assert_eq!(
        state.project().dataset().cell(ColumnId(1), 0).unwrap(),
        Cell::Number(0.12345678901234566)
    );
    state = state
        .apply_action(UiAction::BeginCell {
            column: ColumnId(2),
            row: 2,
        })
        .unwrap();
    state.set_cell_text("ŵŶé".into()).unwrap();
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
    assert_eq!(
        state.project().dataset().cell(ColumnId(2), 2).unwrap(),
        Cell::Text("Carys".into())
    );
}
#[test]
fn active_worker_blocks_authoritative_changes_but_not_drafts_or_navigation() {
    let mut state = state();
    let mut jobs = JobRunner::default();
    state
        .start_operation(
            &mut jobs,
            JobOperation::Analyze {
                project: Arc::clone(state.project()),
                request: request().into(),
            },
        )
        .unwrap();
    assert!(state.apply_action(UiAction::AppendRow).is_err());
    assert!(state
        .apply_action(UiAction::EditCell {
            column: ColumnId(1),
            row: 0,
            value: Cell::Number(99.0)
        })
        .is_err());
    assert!(state
        .apply_action(UiAction::Metadata {
            column: ColumnId(1),
            metadata: state.project().dataset().columns()[0].metadata().clone()
        })
        .is_err());
    state = state
        .apply_action(UiAction::SetDraft(request().into()))
        .unwrap();
    state = state
        .apply_action(UiAction::Navigate(Workspace::Variables))
        .unwrap();
    assert_eq!(state.workspace(), Workspace::Variables);
    jobs.begin_shutdown();
    let until = Instant::now() + Duration::from_secs(5);
    while !jobs.is_shutdown_complete() {
        state.poll(&mut jobs);
        assert!(Instant::now() < until);
        std::thread::yield_now();
    }
}
#[test]
fn stamped_conflicts_and_superseded_files_never_clear_dirty_or_adopt_path() {
    use tessera::{
        data::RevisionId,
        files::{FileIdentity, FileStamp, Publication, SaveOutcome},
    };
    let mut state = state();
    let token = state.token();
    for publication in [
        Publication::Conflict,
        Publication::Uncertain,
        Publication::Committed,
    ] {
        state
            .handle_event(JobEvent::Completed {
                token,
                cancelled: publication == Publication::Committed,
                result: Ok(JobOutput::Saved(SaveOutcome {
                    publication,
                    revision: Some(RevisionId(1)),
                    committed_stamp: Some(FileStamp {
                        identity: FileIdentity { volume: 1, file: 1 },
                        digest: [0; 32],
                        size: 1,
                    }),
                    destination: "/not-adopted.tessera".into(),
                    recovery: None,
                    warning: Some("Preserved".into()),
                })),
            })
            .unwrap();
        assert!(state.is_dirty());
        assert_eq!(state.dialog(), Some(Dialog::FileOutcome));
    }
    let old = state.token();
    state = state
        .apply_action(UiAction::Request(PendingAction::New))
        .unwrap()
        .apply_action(UiAction::ResolveUnsaved(UnsavedChoice::Discard))
        .unwrap();
    finish_pending_frame(&mut state);
    let analysis = describe(
        &Dataset::new(vec![
            Column::numeric(ColumnId(1), "Score", vec![Some(1.0)]).unwrap()
        ])
        .unwrap(),
        &request(),
        &JobControl::default(),
    )
    .unwrap();
    state
        .handle_event(JobEvent::Completed {
            token: old,
            cancelled: false,
            result: Ok(JobOutput::Analyzed(analysis.into())),
        })
        .unwrap();
    assert!(state.project().runs().is_empty());
    assert_eq!(state.project().dataset().row_count(), 0);
}
#[test]
fn metadata_rows_and_variables_are_real_transactional_actions() {
    use tessera::data::{ColumnKind, Measurement, MissingRules, ValueLabel};
    let original = state();
    let mut state = original.apply_action(UiAction::AppendRow).unwrap();
    state = state
        .apply_action(UiAction::AddColumn {
            name: "Ymateb ŷ".into(),
            kind: ColumnKind::Text,
        })
        .unwrap();
    assert_eq!(
        state.project().dataset().cell(ColumnId(3), 3).unwrap(),
        Cell::Missing
    );
    let mut metadata = state
        .project()
        .dataset()
        .column(ColumnId(1))
        .unwrap()
        .metadata()
        .clone();
    metadata.name = "Cyfanswm".into();
    metadata.description = "Sgôr lawn".into();
    metadata.measurement = Measurement::Scale;
    metadata.display_decimals = 5;
    metadata.missing_rules = MissingRules::Number {
        values: vec![17.0],
        ranges: vec![],
    };
    metadata.value_labels = vec![ValueLabel {
        value: Cell::Number(25.0),
        label: "Uchel".into(),
    }];
    state = state
        .apply_action(UiAction::Metadata {
            column: ColumnId(1),
            metadata,
        })
        .unwrap();
    assert!(state
        .project()
        .dataset()
        .column(ColumnId(1))
        .unwrap()
        .is_missing(0)
        .unwrap());
    assert_eq!(
        original
            .project()
            .dataset()
            .column(ColumnId(1))
            .unwrap()
            .name(),
        "Sgôr ŵ"
    );
    assert!(state
        .apply_action(UiAction::EditCell {
            column: ColumnId(1),
            row: 0,
            value: Cell::Text("bad".into())
        })
        .is_err());
}
#[test]
fn actual_settings_picker_starts_descriptives() {
    let ctx = egui::Context::default();
    tessera::desktop::configure_context(&ctx);
    let size = Vec2::new(1180.0, 780.0);
    let mut state = state();
    let mut jobs = JobRunner::default();
    frame(&ctx, &mut state, &mut jobs, size, vec![]);
    click(&ctx, &mut state, &mut jobs, "Analyses", size);
    assert!(state.control_rect("Variable 1").is_some());
    click(&ctx, &mut state, &mut jobs, "Variable 1", size);
    let tessera::desktop::draft::AnalysisDraft::Describe(draft) = state.draft() else {
        panic!("expected descriptives");
    };
    assert_eq!(draft.columns, vec![ColumnId(1)]);
    click(&ctx, &mut state, &mut jobs, "Run descriptives", size);
    let until = Instant::now() + Duration::from_secs(5);
    while jobs.is_running() {
        state.poll(&mut jobs);
        assert!(Instant::now() < until);
        std::thread::yield_now();
    }
    assert_eq!(state.project().runs().len(), 1);
}
#[test]
fn worksheet_virtualizes_both_axes_and_scrolls() {
    let columns = (1..=250)
        .map(|id| {
            Column::numeric(ColumnId(id), &format!("Column {id}"), vec![Some(1.0); 8000]).unwrap()
        })
        .collect();
    let mut state =
        DesktopState::from_project(Project::new("Wide", Dataset::new(columns).unwrap()).unwrap())
            .unwrap();
    let mut jobs = JobRunner::default();
    let ctx = egui::Context::default();
    tessera::desktop::configure_context(&ctx);
    let size = Vec2::new(760.0, 520.0);
    frame(&ctx, &mut state, &mut jobs, size, vec![]);
    assert!(state.control_rect("Cell 1:0").is_some());
    assert!(state.control_rect("Cell 250:7999").is_none());
    let position = state.control_rect("Cell 2:2").unwrap().center();
    frame(
        &ctx,
        &mut state,
        &mut jobs,
        size,
        vec![
            Event::PointerMoved(position),
            Event::MouseWheel {
                unit: egui::MouseWheelUnit::Point,
                phase: egui::TouchPhase::Move,
                delta: Vec2::new(-800.0, -1600.0),
                modifiers: Modifiers::NONE,
            },
        ],
    );
    for _ in 0..20 {
        frame(&ctx, &mut state, &mut jobs, size, vec![]);
    }
    assert!(state.control_rect("Cell 1:0").is_none());
    assert!(state.project().dataset().column_count() == 250);
}
#[test]
fn history_alone_marks_a_confirmed_save_dirty() {
    let mut state = state();
    let mut jobs = JobRunner::default();
    let dir = std::env::temp_dir()
        .canonicalize()
        .unwrap()
        .join(format!("tessera-history-dirty-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let path = dir.join("project.tessera");
    let _ = std::fs::remove_file(&path);
    let project = Arc::clone(state.project());
    run(
        &mut state,
        &mut jobs,
        JobOperation::Save {
            project,
            path,
            expected: None,
        },
    );
    assert!(!state.is_dirty());
    let revision = state.project().dataset().revision();
    let project = Arc::clone(state.project());
    run(
        &mut state,
        &mut jobs,
        JobOperation::Analyze {
            project,
            request: request().into(),
        },
    );
    assert_eq!(state.project().dataset().revision(), revision);
    assert!(state.is_dirty());
    std::fs::remove_dir_all(dir).unwrap();
}
#[path = "desktop/review.rs"]
mod review;

#[path = "desktop/draft_bridge.rs"]
mod draft_bridge;
#[path = "desktop/procedures.rs"]
mod procedures;

#[path = "desktop/exploration.rs"]
mod exploration;

#[path = "desktop/regression.rs"]
mod regression;

#[path = "desktop/anova.rs"]
mod anova;

#[path = "desktop/survey.rs"]
mod survey;
