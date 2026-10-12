#![cfg(feature = "desktop")]
use eframe::egui::{self, Pos2, Rect, Vec2};
use tessera::{
    desktop::{configure_context, render, DesktopState},
    jobs::JobRunner,
};

fn settled_controls(
    ctx: &egui::Context,
    state: &mut DesktopState,
    jobs: &mut JobRunner,
    size: Vec2,
    pointer: Pos2,
) -> Vec<(&'static str, Rect)> {
    for _ in 0..3 {
        let mut output = ctx.run_ui(
            egui::RawInput {
                screen_rect: Some(Rect::from_min_size(Pos2::ZERO, size)),
                events: vec![egui::Event::PointerMoved(pointer)],
                ..Default::default()
            },
            |ui| render(ui, state, jobs),
        );
        output.textures_delta.clear();
    }
    [
        "Help",
        "Open",
        "Import",
        "Data",
        "Prepare",
        "Analyse",
        "Results",
        "Variables",
    ]
    .into_iter()
    .map(|label| (label, state.control_rect(label).expect("visible control")))
    .collect()
}

#[test]
fn hovering_buttons_preserves_their_and_neighboring_controls_geometry() {
    for theme in [egui::Theme::Light, egui::Theme::Dark] {
        for (size, zoom) in [
            (Vec2::new(1180., 780.), 1.),
            (Vec2::new(640., 440.), 1.),
            (Vec2::new(1180., 780.), 1.5),
        ] {
            let ctx = egui::Context::default();
            configure_context(&ctx);
            ctx.set_theme(theme);
            ctx.set_zoom_factor(zoom);
            let mut state = DesktopState::new().unwrap();
            let mut jobs = JobRunner::default();
            let viewport = size / zoom;
            let off = Pos2::new(viewport.x - 8., viewport.y - 8.);
            let baseline = settled_controls(&ctx, &mut state, &mut jobs, viewport, off);
            for target in ["Help", "Open", "Prepare"] {
                let pointer = baseline
                    .iter()
                    .find(|(label, _)| *label == target)
                    .unwrap()
                    .1
                    .center();
                for (phase, pointer) in [("hover", pointer), ("leave", off)] {
                    let controls = settled_controls(&ctx, &mut state, &mut jobs, viewport, pointer);
                    for ((label, before), (_, after)) in baseline.iter().zip(&controls) {
                        assert_eq!(
                            before, after,
                            "{label} moved during {target} {phase}; theme={theme:?}, size={size:?}, zoom={zoom}"
                        );
                    }
                }
            }
        }
    }
}
