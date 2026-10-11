#![cfg(all(feature = "desktop", target_os = "macos"))]
use eframe::egui::{self, Event, Modifiers, PointerButton, Pos2, Rect, Vec2};
use tessera::{
    desktop::{configure_context, render, DesktopState},
    jobs::JobRunner,
};
#[test]
fn dragging_unused_header_space_requests_native_window_movement() {
    let ctx = egui::Context::default();
    configure_context(&ctx);
    let mut state = DesktopState::new().unwrap();
    let mut jobs = JobRunner::default();
    let mut frame = |events| {
        let mut output = ctx.run_ui(
            egui::RawInput {
                screen_rect: Some(Rect::from_min_size(Pos2::ZERO, Vec2::new(1180., 780.))),
                events,
                ..Default::default()
            },
            |ui| render(ui, &mut state, &mut jobs),
        );
        output.textures_delta.clear();
        output
    };
    frame(vec![]);
    let start = Pos2::new(620., 27.);
    let pressed = frame(vec![
        Event::PointerMoved(start),
        Event::PointerButton {
            pos: start,
            button: PointerButton::Primary,
            pressed: true,
            modifiers: Modifiers::NONE,
        },
    ]);
    let output = frame(vec![Event::PointerMoved(Pos2::new(660., 27.))]);
    assert!([pressed, output]
        .iter()
        .any(|output| output.viewport_output.values().any(|v| v
            .commands
            .iter()
            .any(|c| matches!(c, egui::ViewportCommand::StartDrag)))));
}
