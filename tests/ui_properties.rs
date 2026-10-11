#![cfg(feature = "desktop")]
use eframe::egui::{self, Pos2, Rect, Vec2};
use tessera::{
    data::{Column, ColumnId, Dataset},
    desktop::{render, DesktopState, UiAction, Workspace},
    jobs::JobRunner,
    project::Project,
};

fn labels(width: f32) -> Vec<String> {
    let ctx = egui::Context::default();
    ctx.enable_accesskit();
    let project = Project::new(
        "Variables",
        Dataset::new(vec![
            Column::numeric(ColumnId(1), "Score", vec![Some(1.0)]).unwrap()
        ])
        .unwrap(),
    )
    .unwrap();
    let mut state = DesktopState::from_project(project)
        .unwrap()
        .apply_action(UiAction::Navigate(Workspace::Variables))
        .unwrap();
    let mut jobs = JobRunner::default();
    let mut output = ctx.run_ui(
        egui::RawInput {
            screen_rect: Some(Rect::from_min_size(Pos2::ZERO, Vec2::new(width, 780.0))),
            ..Default::default()
        },
        |ui| render(ui, &mut state, &mut jobs),
    );
    output.textures_delta.clear();
    output
        .platform_output
        .accesskit_update
        .into_iter()
        .flat_map(|u| u.nodes)
        .filter_map(|(_, n)| n.label().or_else(|| n.value()).map(str::to_owned))
        .collect()
}
#[test]
fn variables_offer_search_and_structured_readonly_rows() {
    let actual = labels(1180.0);
    for expected in [
        "Find a variable",
        "Name",
        "Type",
        "Measurement",
        "Description",
        "Not set",
    ] {
        assert!(
            actual.iter().any(|s| s == expected),
            "missing {expected}: {actual:?}"
        );
    }
    assert!(
        actual
            .iter()
            .any(|s| s.starts_with("Variable Score, Number, Not set")),
        "{actual:?}"
    );
}

#[test]
fn empty_variables_guide_without_a_blank_inspector() {
    let ctx = egui::Context::default();
    ctx.enable_accesskit();
    let project = Project::new("Empty", Dataset::new(vec![]).unwrap()).unwrap();
    let mut state = DesktopState::from_project(project)
        .unwrap()
        .apply_action(UiAction::Navigate(Workspace::Variables))
        .unwrap();
    let mut jobs = JobRunner::default();
    let mut output = ctx.run_ui(
        egui::RawInput {
            screen_rect: Some(Rect::from_min_size(Pos2::ZERO, Vec2::new(1180.0, 780.0))),
            ..Default::default()
        },
        |ui| render(ui, &mut state, &mut jobs),
    );
    output.textures_delta.clear();
    let actual: Vec<_> = output
        .platform_output
        .accesskit_update
        .into_iter()
        .flat_map(|u| u.nodes)
        .filter_map(|(_, n)| n.label().or_else(|| n.value()).map(str::to_owned))
        .collect();
    assert!(actual.iter().any(|label| label == "Define your variables"));
    assert!(!actual.iter().any(
        |label| label == "Select a variable to edit its name, description and value settings."
    ));
}
