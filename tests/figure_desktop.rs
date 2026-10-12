#![cfg(feature = "desktop")]
use eframe::egui::{self, Pos2, Rect, Vec2};
use tessera::{
    analysis::*,
    data::{Column, ColumnId, Dataset},
    desktop::{render, DesktopState, UiAction, Workspace},
    figures::*,
    jobs::{JobOperation, JobRunner},
    operation::JobControl,
    project::Project,
};

fn state() -> DesktopState {
    let data = Dataset::new(vec![Column::numeric(
        ColumnId(1),
        "Sgôr ŵ",
        vec![Some(1.), Some(2.), Some(3.)],
    )
    .unwrap()])
    .unwrap();
    let run = histogram_with_budget(
        &data,
        &HistogramRequest {
            variable: ColumnId(1),
            bins: HistogramBins::Count(2),
            filter: RowFilter::default(),
            frequency_weight: None,
        },
        &JobControl::default(),
        &[],
        0,
    )
    .unwrap();
    DesktopState::from_project(
        Project::new("Report", data)
            .unwrap()
            .with_run(CompletedAnalysis::Histogram(run))
            .unwrap(),
    )
    .unwrap()
}
#[test]
fn figure_edit_marks_project_dirty_without_changing_captured_analysis() {
    let mut state = state();
    let mut jobs = JobRunner::default();
    let dir = std::env::temp_dir()
        .canonicalize()
        .unwrap()
        .join(format!("figure-edit-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let path = dir.join("edited.tessera");
    let project = state.project().clone();
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
    let key = figure_keys(&state.project().runs()[0])[0].clone();
    let original = state.project().runs()[0].clone();
    let settings = FigureSettings {
        title: "A report figure".into(),
        palette: FigurePalette::Monochrome,
        ..Default::default()
    };
    state.apply_figure_settings(&key, settings.clone()).unwrap();
    assert!(state.is_dirty());
    assert_eq!(state.project().figure_settings(&key), settings);
    assert!(std::sync::Arc::ptr_eq(
        &original,
        &state.project().runs()[0]
    ));
    let invalid = FigureSettings {
        width_mm: f64::NAN,
        ..Default::default()
    };
    assert!(state.apply_figure_settings(&key, invalid).is_err());
    assert_eq!(state.project().figure_settings(&key), settings);
    let expected = Some(tessera::files::stamp(&path).unwrap());
    let project = state.project().clone();
    run(
        &mut state,
        &mut jobs,
        JobOperation::Save {
            project,
            path: path.clone(),
            expected,
        },
    );
    assert!(!state.is_dirty());
    run(
        &mut state,
        &mut jobs,
        JobOperation::Open { path: path.clone() },
    );
    assert_eq!(state.project().figure_settings(&key), settings);
    std::fs::remove_dir_all(dir).unwrap();
}
fn run(state: &mut DesktopState, jobs: &mut JobRunner, operation: JobOperation) {
    state.start_operation(jobs, operation).unwrap();
    let until = std::time::Instant::now() + std::time::Duration::from_secs(10);
    while jobs.is_running() {
        state.poll(jobs);
        assert!(std::time::Instant::now() < until);
        std::thread::yield_now();
    }
    assert!(state.error().is_none(), "{:?}", state.error());
}
fn frame(
    ctx: &egui::Context,
    state: &mut DesktopState,
    jobs: &mut JobRunner,
    events: Vec<egui::Event>,
) {
    let mut output = ctx.run_ui(
        egui::RawInput {
            screen_rect: Some(Rect::from_min_size(Pos2::ZERO, Vec2::new(1280., 900.))),
            events,
            ..Default::default()
        },
        |ui| render(ui, state, jobs),
    );
    output.textures_delta.clear();
}
fn click(ctx: &egui::Context, state: &mut DesktopState, jobs: &mut JobRunner, label: &str) {
    if label == "Edit figure" && state.control_rect(label).is_none() {
        click(ctx, state, jobs, "Figures");
        for _ in 0..5 {
            frame(ctx, state, jobs, vec![]);
        }
    }
    let p = state.control_rect(label).expect(label).center();
    for pressed in [true, false] {
        frame(
            ctx,
            state,
            jobs,
            vec![
                egui::Event::PointerMoved(p),
                egui::Event::PointerButton {
                    pos: p,
                    button: egui::PointerButton::Primary,
                    pressed,
                    modifiers: egui::Modifiers::NONE,
                },
            ],
        );
    }
    frame(ctx, state, jobs, vec![]);
}
#[test]
fn editor_title_apply_and_cancel_use_the_captured_run() {
    let mut state = state()
        .apply_action(UiAction::SelectRun(tessera::operation::RunId(1)))
        .unwrap()
        .apply_action(UiAction::Navigate(Workspace::Results))
        .unwrap();
    let key = figure_keys(&state.project().runs()[0])[0];
    let ctx = egui::Context::default();
    tessera::desktop::configure_context(&ctx);
    let mut jobs = JobRunner::default();
    frame(&ctx, &mut state, &mut jobs, vec![]);
    click(&ctx, &mut state, &mut jobs, "Edit figure");
    click(&ctx, &mut state, &mut jobs, "Figure title");
    frame(
        &ctx,
        &mut state,
        &mut jobs,
        vec![egui::Event::Text("Sgôr in the report".into())],
    );
    click(&ctx, &mut state, &mut jobs, "Apply figure");
    assert_eq!(
        state.project().figure_settings(&key).title,
        "Sgôr in the report"
    );
    click(&ctx, &mut state, &mut jobs, "Figure title");
    frame(
        &ctx,
        &mut state,
        &mut jobs,
        vec![egui::Event::Text(" unsaved edit".into())],
    );
    click(&ctx, &mut state, &mut jobs, "Cancel figure edit");
    assert_eq!(
        state.project().figure_settings(&key).title,
        "Sgôr in the report"
    );
}
#[test]
fn report_figure_controls_are_accessible_in_results() {
    let mut state = state()
        .apply_action(UiAction::SelectRun(tessera::operation::RunId(1)))
        .unwrap()
        .apply_action(UiAction::Navigate(Workspace::Results))
        .unwrap();
    let ctx = egui::Context::default();
    tessera::desktop::configure_context(&ctx);
    let mut jobs = JobRunner::default();
    for _ in 0..2 {
        let mut output = ctx.run_ui(
            egui::RawInput {
                screen_rect: Some(Rect::from_min_size(Pos2::ZERO, Vec2::new(1180., 780.))),
                ..Default::default()
            },
            |ui| render(ui, &mut state, &mut jobs),
        );
        output.textures_delta.clear();
    }
    assert!(state.control_rect("Figures").is_some());
    assert!(
        state.control_rect("Edit figure").is_none(),
        "figure tools start disclosed only on request"
    );
    click(&ctx, &mut state, &mut jobs, "Figures");
    assert!(state.control_rect("Edit figure").is_some());
    assert!(state.control_rect("Export figure…").is_some());
}

#[test]
fn undo_reverts_applied_presentation_before_the_editor_closes() {
    for saved in [false, true] {
        let mut state = state()
            .apply_action(UiAction::SelectRun(tessera::operation::RunId(1)))
            .unwrap()
            .apply_action(UiAction::Navigate(Workspace::Results))
            .unwrap();
        let key = figure_keys(&state.project().runs()[0])[0];
        if saved {
            let mut settings = state.project().figure_settings(&key);
            settings.title = "Previously saved title".into();
            state.apply_figure_settings(&key, settings).unwrap();
        }
        let original = state.project().figure_settings(&key);
        let ctx = egui::Context::default();
        tessera::desktop::configure_context(&ctx);
        let mut jobs = JobRunner::default();
        frame(&ctx, &mut state, &mut jobs, vec![]);
        click(&ctx, &mut state, &mut jobs, "Edit figure");
        click(&ctx, &mut state, &mut jobs, "Figure title");
        frame(
            &ctx,
            &mut state,
            &mut jobs,
            vec![egui::Event::Text("Applied title".into())],
        );
        click(&ctx, &mut state, &mut jobs, "Apply figure");
        assert_eq!(
            state.project().figure_settings(&key).title,
            format!("{}Applied title", original.title)
        );
        click(&ctx, &mut state, &mut jobs, "Undo figure edit");
        click(&ctx, &mut state, &mut jobs, "Cancel figure edit");
        assert_eq!(state.project().figure_settings(&key), original);
        assert_eq!(
            state.project().figures().is_empty(),
            !saved,
            "Undo must restore saved presentation presence"
        );
    }
}

#[test]
fn editor_actions_fit_a_small_desktop_window() {
    let mut state = state()
        .apply_action(UiAction::SelectRun(tessera::operation::RunId(1)))
        .unwrap()
        .apply_action(UiAction::Navigate(Workspace::Results))
        .unwrap();
    let ctx = egui::Context::default();
    tessera::desktop::configure_context(&ctx);
    let mut jobs = JobRunner::default();
    frame(&ctx, &mut state, &mut jobs, vec![]);
    click(&ctx, &mut state, &mut jobs, "Edit figure");
    for _ in 0..3 {
        let mut output = ctx.run_ui(
            egui::RawInput {
                screen_rect: Some(Rect::from_min_size(Pos2::ZERO, Vec2::new(640., 440.))),
                ..Default::default()
            },
            |ui| render(ui, &mut state, &mut jobs),
        );
        output.textures_delta.clear();
    }
    let apply = state.control_rect("Apply figure").unwrap();
    assert!(
        apply.max.y <= 440. && apply.max.x <= 640.,
        "Figure actions must stay visible: {apply:?}"
    );
}

#[test]
fn escape_closes_figure_editor_without_applying_draft_changes() {
    let mut state = state()
        .apply_action(UiAction::SelectRun(tessera::operation::RunId(1)))
        .unwrap()
        .apply_action(UiAction::Navigate(Workspace::Results))
        .unwrap();
    let key = figure_keys(&state.project().runs()[0])[0];
    let original = state.project().figure_settings(&key);
    let ctx = egui::Context::default();
    tessera::desktop::configure_context(&ctx);
    let mut jobs = JobRunner::default();
    frame(&ctx, &mut state, &mut jobs, vec![]);
    click(&ctx, &mut state, &mut jobs, "Edit figure");
    click(&ctx, &mut state, &mut jobs, "Figure title");
    frame(
        &ctx,
        &mut state,
        &mut jobs,
        vec![egui::Event::Text("Unsaved draft".into())],
    );
    frame(
        &ctx,
        &mut state,
        &mut jobs,
        vec![egui::Event::Key {
            key: egui::Key::Escape,
            physical_key: None,
            pressed: true,
            repeat: false,
            modifiers: egui::Modifiers::NONE,
        }],
    );
    frame(&ctx, &mut state, &mut jobs, vec![]);
    assert!(state.control_rect("Figure title").is_none());
    assert_eq!(state.project().figure_settings(&key), original);
}
