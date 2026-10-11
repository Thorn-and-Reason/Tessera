#![cfg(feature = "desktop")]
use eframe::egui::{self, Pos2, Rect, Vec2};
use tessera::{
    desktop::{render, DesktopState},
    jobs::JobRunner,
    operation::JobControl,
    project_io,
};

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
fn click(
    ctx: &egui::Context,
    state: &mut DesktopState,
    jobs: &mut JobRunner,
    size: Vec2,
    label: &str,
) {
    let pos = state
        .control_rect(label)
        .unwrap_or_else(|| panic!("missing {label}"))
        .center();
    for pressed in [true, false] {
        frame(
            ctx,
            state,
            jobs,
            size,
            vec![
                egui::Event::PointerMoved(pos),
                egui::Event::PointerButton {
                    pos,
                    button: egui::PointerButton::Primary,
                    pressed,
                    modifiers: egui::Modifiers::NONE,
                },
            ],
        );
    }
    for _ in 0..3 {
        frame(ctx, state, jobs, size, vec![]);
    }
}
#[test]
fn about_opens_by_keyboard_exposes_licence_and_closes_without_document_changes() {
    for size in [Vec2::new(640., 440.), Vec2::new(1180., 780.)] {
        let mut state = DesktopState::from_project(
            project_io::decode(
                &include_bytes!("../examples/mixed-analysis.tessera")[..],
                &JobControl::default(),
            )
            .unwrap(),
        )
        .unwrap();
        let before = state.project().clone();
        let token = state.token();
        let dirty = state.is_dirty();
        let ctx = egui::Context::default();
        ctx.enable_accesskit();
        let mut jobs = JobRunner::default();
        for _ in 0..3 {
            frame(&ctx, &mut state, &mut jobs, size, vec![]);
        }
        click(&ctx, &mut state, &mut jobs, size, "Help");
        let id = state.control_id("About Tessera").unwrap();
        ctx.memory_mut(|m| m.request_focus(id));
        frame(
            &ctx,
            &mut state,
            &mut jobs,
            size,
            vec![egui::Event::Key {
                key: egui::Key::Enter,
                physical_key: None,
                pressed: true,
                repeat: false,
                modifiers: egui::Modifiers::NONE,
            }],
        );
        let output = frame(&ctx, &mut state, &mut jobs, size, vec![]);
        let nodes = output.platform_output.accesskit_update.unwrap().nodes;
        assert!(nodes.iter().any(|(_, n)| n
            .label()
            .or_else(|| n.value())
            .is_some_and(|s| s.contains("GNU GPL version 3 only"))));
        assert!(nodes.iter().any(|(_, n)| n
            .label()
            .or_else(|| n.value())
            .is_some_and(|s| s.contains("No warranty"))));
        assert!(state.control_rect("Full GPLv3 licence").is_some());
        frame(
            &ctx,
            &mut state,
            &mut jobs,
            size,
            vec![egui::Event::Key {
                key: egui::Key::Escape,
                physical_key: None,
                pressed: true,
                repeat: false,
                modifiers: egui::Modifiers::NONE,
            }],
        );
        frame(&ctx, &mut state, &mut jobs, size, vec![]);
        assert!(state.control_rect("Full GPLv3 licence").is_none());
        assert!(std::sync::Arc::ptr_eq(&before, state.project()));
        assert_eq!(state.token(), token);
        assert_eq!(state.is_dirty(), dirty);
    }
}
