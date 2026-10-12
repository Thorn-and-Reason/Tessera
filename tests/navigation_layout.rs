#![cfg(feature = "desktop")]
use eframe::egui::{self, Pos2, Rect, Vec2};
use tessera::{
    desktop::{configure_context, render, DesktopState},
    jobs::JobRunner,
};

#[test]
fn desktop_chrome_leaves_room_for_the_work_and_uses_compact_source_rows() {
    let ctx = egui::Context::default();
    configure_context(&ctx);
    let mut state = DesktopState::new().unwrap();
    let mut jobs = JobRunner::default();
    for _ in 0..3 {
        let mut output = ctx.run_ui(
            egui::RawInput {
                screen_rect: Some(Rect::from_min_size(Pos2::ZERO, Vec2::new(1180., 780.))),
                ..Default::default()
            },
            |ui| render(ui, &mut state, &mut jobs),
        );
        output.textures_delta.clear();
    }
    assert!(state.control_rect("Help").unwrap().bottom() <= 48.);
    let data = state.control_rect("Data").unwrap();
    let prepare = state.control_rect("Prepare").unwrap();
    let analyse = state.control_rect("Analyse").unwrap();
    let results = state.control_rect("Results").unwrap();
    let variables = state.control_rect("Variables").unwrap();

    assert!(
        variables.bottom() - data.top() <= 150.,
        "Navigation should be a compact source list"
    );
    assert!(data.height() <= 30.);
    assert!(data.top() < prepare.top());
    assert!(prepare.top() < analyse.top());
    assert!(analyse.top() < results.top());
    assert!(results.top() < variables.top());
}

#[test]
fn workflow_navigation_stays_visible_at_supported_sizes_and_scales() {
    for size in [Vec2::new(640., 440.), Vec2::new(1180., 780.)] {
        for zoom in [1., 1.5] {
            let ctx = egui::Context::default();
            configure_context(&ctx);
            ctx.set_zoom_factor(zoom);
            let mut state = DesktopState::new().unwrap();
            let mut jobs = JobRunner::default();
            for _ in 0..3 {
                let mut output = ctx.run_ui(
                    egui::RawInput {
                        screen_rect: Some(Rect::from_min_size(Pos2::ZERO, size / zoom)),
                        ..Default::default()
                    },
                    |ui| render(ui, &mut state, &mut jobs),
                );
                output.textures_delta.clear();
            }
            for label in ["Data", "Prepare", "Variables", "Analyses", "Results"] {
                let rect = state.control_rect(label).expect("navigation control");
                assert!(
                    rect.left() >= 0.,
                    "{label}: {rect:?}; viewport {:?}, size {size:?}, zoom {zoom}",
                    ctx.content_rect()
                );
                assert!(
                    rect.right() <= size.x / zoom,
                    "{label}: {rect:?}; viewport {:?}, size {size:?}, zoom {zoom}",
                    ctx.content_rect()
                );
                assert!(
                    rect.top() >= 0. && rect.bottom() <= size.y / zoom,
                    "{label}: {rect:?}; viewport {:?}, size {size:?}, zoom {zoom}",
                    ctx.content_rect()
                );
            }
        }
    }
}
