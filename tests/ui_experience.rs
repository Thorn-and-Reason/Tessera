#![cfg(feature = "desktop")]
use eframe::egui::{self, Pos2, Rect, Vec2};
use std::{
    sync::Arc,
    time::{Duration, Instant},
};
use tessera::{
    desktop::{
        draft::{AnalysisDraft, Procedure},
        render, AnalysisMode, DesktopState, PendingAction, UiAction, Workspace,
    },
    jobs::{JobOperation, JobRunner},
    operation::JobControl,
    project_io,
};

fn saved() -> DesktopState {
    DesktopState::from_project(
        project_io::decode(
            &include_bytes!("../examples/regression-analysis.tessera")[..],
            &JobControl::default(),
        )
        .unwrap(),
    )
    .unwrap()
}
fn frame(
    ctx: &egui::Context,
    state: &mut DesktopState,
    jobs: &mut JobRunner,
    size: Vec2,
    events: Vec<egui::Event>,
) -> egui::FullOutput {
    let mut output = ctx.run_ui(
        egui::RawInput {
            screen_rect: Some(Rect::from_min_size(Pos2::ZERO, size)),
            events,
            ..Default::default()
        },
        |ui| render(ui, state, jobs),
    );
    output.textures_delta.clear();
    output
}
fn click(ctx: &egui::Context, state: &mut DesktopState, jobs: &mut JobRunner, label: &str) {
    let rect = state.control_rect(label).expect(label);
    frame(
        ctx,
        state,
        jobs,
        Vec2::new(1180., 780.),
        vec![
            egui::Event::PointerMoved(rect.center()),
            egui::Event::PointerButton {
                pos: rect.center(),
                button: egui::PointerButton::Primary,
                pressed: true,
                modifiers: egui::Modifiers::NONE,
            },
            egui::Event::PointerButton {
                pos: rect.center(),
                button: egui::PointerButton::Primary,
                pressed: false,
                modifiers: egui::Modifiers::NONE,
            },
        ],
    );
}
#[test]
fn mode_preserves_all_twelve_drafts_recipes_and_document_identity() {
    let base = saved();
    for procedure in Procedure::ALL {
        let mut draft = base.draft().for_procedure(procedure);
        if let AnalysisDraft::OneSampleT { reference_text, .. } = &mut draft {
            *reference_text = "-1e".into();
        }
        let original = base.apply_action(UiAction::SetDraft(draft)).unwrap();
        let request = original.checked_draft_request().map_err(|e| e.to_string());
        for mode in [AnalysisMode::Advanced, AnalysisMode::Guided] {
            let next = original
                .apply_action(UiAction::SetAnalysisMode(mode))
                .unwrap();
            assert_eq!(next.draft(), original.draft());
            assert!(Arc::ptr_eq(next.project(), original.project()));
            assert_eq!(next.token(), original.token());
            assert_eq!(next.is_dirty(), original.is_dirty());
            assert_eq!(
                next.checked_draft_request().map_err(|e| e.to_string()),
                request
            );
        }
    }
}
#[test]
fn modes_preserve_saved_recipes_and_active_worker_publication() {
    let mut state = saved();
    let request = state
        .checked_run_request(state.project().runs()[0].id)
        .unwrap();
    let mut jobs = JobRunner::default();
    state
        .start_operation(
            &mut jobs,
            JobOperation::Analyze {
                project: Arc::clone(state.project()),
                request: request.clone(),
            },
        )
        .unwrap();
    let token = state.token();
    let count = state.project().runs().len();
    state = state
        .apply_action(UiAction::SetAnalysisMode(AnalysisMode::Advanced))
        .unwrap();
    assert_eq!(state.token(), token);
    let deadline = Instant::now() + Duration::from_secs(10);
    while jobs.is_running() {
        state.poll(&mut jobs);
        assert!(Instant::now() < deadline);
        std::thread::yield_now();
    }
    state.poll(&mut jobs);
    assert_eq!(state.project().runs().len(), count + 1);
    assert_eq!(
        state.project().runs().last().unwrap().analysis.request(),
        request
    );
    let bytes = |state: &DesktopState| {
        let mut bytes = Vec::new();
        project_io::encode(state.project(), &mut bytes, &JobControl::default()).unwrap();
        bytes
    };
    let before = bytes(&state);
    state = state
        .apply_action(UiAction::SetAnalysisMode(AnalysisMode::Guided))
        .unwrap();
    assert_eq!(bytes(&state), before);
}
#[test]
fn new_document_keeps_preference_and_errors_survive_mode_changes() {
    let mut state = DesktopState::new()
        .unwrap()
        .apply_action(UiAction::SetAnalysisMode(AnalysisMode::Advanced))
        .unwrap();
    state = state
        .apply_action(UiAction::Request(PendingAction::New))
        .unwrap();
    frame(
        &egui::Context::default(),
        &mut state,
        &mut JobRunner::default(),
        Vec2::new(1180., 780.),
        vec![],
    );
    assert_eq!(state.analysis_mode(), AnalysisMode::Advanced);
    let mut jobs = JobRunner::default();
    state
        .start_operation(
            &mut jobs,
            JobOperation::Open {
                path: std::env::temp_dir()
                    .join(format!("missing-tessera-mode-{}", std::process::id())),
            },
        )
        .unwrap();
    let deadline = Instant::now() + Duration::from_secs(10);
    while jobs.is_running() {
        state.poll(&mut jobs);
        assert!(Instant::now() < deadline);
        std::thread::yield_now();
    }
    state.poll(&mut jobs);
    assert!(state.error().is_some());
    let next = state
        .apply_action(UiAction::SetAnalysisMode(AnalysisMode::Guided))
        .unwrap();
    assert_eq!(next.error(), state.error());
}
#[test]
fn invalid_bins_are_not_silently_corrected_by_either_mode() {
    for mode in [AnalysisMode::Guided, AnalysisMode::Advanced] {
        let mut state = saved()
            .apply_action(UiAction::Navigate(Workspace::Analyses))
            .unwrap()
            .apply_action(UiAction::SetAnalysisMode(mode))
            .unwrap();
        let mut draft = state.draft().for_procedure(Procedure::Histogram);
        if let AnalysisDraft::Histogram { bins, .. } = &mut draft {
            *bins = tessera::analysis::HistogramBins::Count(0);
        }
        state = state
            .apply_action(UiAction::SetDraft(draft.clone()))
            .unwrap();
        let ctx = egui::Context::default();
        frame(
            &ctx,
            &mut state,
            &mut JobRunner::default(),
            Vec2::new(1180., 780.),
            vec![],
        );
        assert_eq!(state.draft(), &draft);
    }
}
#[test]
fn hidden_invalid_weight_offers_advanced_disclosure_from_guided() {
    let mut state = saved()
        .apply_action(UiAction::Navigate(Workspace::Analyses))
        .unwrap()
        .apply_action(UiAction::SetAnalysisMode(AnalysisMode::Guided))
        .unwrap();
    let mut draft = state.draft().for_procedure(Procedure::OneSampleT);
    let AnalysisDraft::OneSampleT {
        variable,
        frequency_weight,
        ..
    } = &mut draft
    else {
        unreachable!()
    };
    *variable = Some(tessera::data::ColumnId(1));
    *frequency_weight = Some(tessera::data::ColumnId(999));
    state = state.apply_action(UiAction::SetDraft(draft)).unwrap();
    assert!(state.checked_draft_request().is_err());

    let ctx = egui::Context::default();
    ctx.enable_accesskit();
    let size = Vec2::new(1180., 1200.);
    let mut jobs = JobRunner::default();
    frame(&ctx, &mut state, &mut jobs, size, vec![]);
    assert!(state.control_rect("Frequency weight").is_none());
    assert!(state.control_rect("Open advanced settings").is_some());

    click(&ctx, &mut state, &mut jobs, "Open advanced settings");
    let mut output = frame(&ctx, &mut state, &mut jobs, size, vec![]);
    for _ in 0..2 {
        output = frame(&ctx, &mut state, &mut jobs, size, vec![]);
    }
    assert_eq!(state.analysis_mode(), AnalysisMode::Advanced);
    assert!(output
        .platform_output
        .accesskit_update
        .into_iter()
        .flat_map(|update| update.nodes)
        .any(|(_, node)| node.label() == Some("Frequency weight")));
    assert!(state.checked_draft_request().is_err());
}
#[test]
fn mode_controls_are_reachable_in_small_large_and_scaled_windows() {
    for (size, zoom) in [
        (Vec2::new(640., 440.), 1.),
        (Vec2::new(1180., 780.), 1.),
        (Vec2::new(1180., 780.), 1.5),
    ] {
        let mut state = saved();
        let mut jobs = JobRunner::default();
        let ctx = egui::Context::default();
        ctx.enable_accesskit();
        ctx.set_zoom_factor(zoom);
        for _ in 0..3 {
            frame(&ctx, &mut state, &mut jobs, size, vec![]);
        }
        let rect = state.control_rect("Analysis mode").unwrap();
        assert!(rect.min.x >= 0. && rect.max.x <= size.x / zoom);
        let click = |pos| {
            vec![
                egui::Event::PointerMoved(pos),
                egui::Event::PointerButton {
                    pos,
                    button: egui::PointerButton::Primary,
                    pressed: true,
                    modifiers: egui::Modifiers::NONE,
                },
                egui::Event::PointerButton {
                    pos,
                    button: egui::PointerButton::Primary,
                    pressed: false,
                    modifiers: egui::Modifiers::NONE,
                },
            ]
        };
        frame(&ctx, &mut state, &mut jobs, size, click(rect.center()));
        for _ in 0..3 {
            frame(&ctx, &mut state, &mut jobs, size, vec![]);
        }
        let choice = state.control_rect("Advanced").unwrap();
        frame(&ctx, &mut state, &mut jobs, size, click(choice.center()));
        assert_eq!(
            state.analysis_mode(),
            AnalysisMode::Advanced,
            "size={size:?} zoom={zoom} mode={rect:?} choice={choice:?}"
        );
        assert!(!egui::Popup::is_any_open(&ctx));
    }
}
