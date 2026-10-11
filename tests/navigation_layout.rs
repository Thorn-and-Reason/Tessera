#![cfg(feature = "desktop")]
use eframe::egui::{self, Pos2, Rect, Vec2};
use tessera::{
    desktop::{configure_context, render, DesktopState},
    jobs::JobRunner,
};

#[test]
fn navigation_controls_fit_the_sidebar_at_supported_sizes_and_scales() {
    for size in [Vec2::new(640., 440.), Vec2::new(1180., 780.)] {
        for zoom in [1., 1.5] {
            let ctx = egui::Context::default();
            configure_context(&ctx);
            ctx.set_zoom_factor(zoom);
            let mut state = DesktopState::new().unwrap();
            let mut jobs = JobRunner::default();
            let mut output = ctx.run_ui(
                egui::RawInput {
                    screen_rect: Some(Rect::from_min_size(Pos2::ZERO, size / zoom)),
                    ..Default::default()
                },
                |ui| render(ui, &mut state, &mut jobs),
            );
            output.textures_delta.clear();
            for label in ["Data", "Variables", "Analyses", "Results"] {
                let rect = state.control_rect(label).expect("navigation control");
                assert!(rect.left() >= 12., "{label}: {rect:?}");
                assert!(rect.right() <= 152., "{label}: {rect:?}");
            }
        }
    }
}
