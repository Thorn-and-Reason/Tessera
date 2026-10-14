#![cfg(feature = "desktop")]
use eframe::egui::{self, Pos2, Rect, Vec2};
use tessera::{
    analysis::*,
    data::{Column, ColumnId, Dataset},
    desktop::{render, DesktopState, UiAction, Workspace},
    jobs::{JobEvent, JobOutput, JobRunner},
    operation::JobControl,
    project::Project,
};

#[test]
fn grouped_boxplots_share_outcome_coordinates_and_exact_read_only_values() {
    let data = Dataset::new(vec![
        Column::numeric(
            ColumnId(1),
            "Outcome",
            vec![Some(1.), Some(2.), Some(4.), Some(5.)],
        )
        .unwrap(),
        Column::text(
            ColumnId(2),
            "Group",
            vec![
                Some("A".into()),
                Some("A".into()),
                Some("B".into()),
                Some("B".into()),
            ],
        )
        .unwrap(),
    ])
    .unwrap();
    let mut state = DesktopState::from_project(Project::new("Comparison", data).unwrap()).unwrap();
    let analysis = analyze_with_budget(
        state.project().dataset(),
        &AnalysisRequest::Boxplot(BoxplotRequest {
            outcome: ColumnId(1),
            group: Some(ColumnId(2)),
            filter: RowFilter::default(),
            frequency_weight: None,
        }),
        &JobControl::default(),
        &[],
        4096,
    )
    .unwrap();
    state
        .handle_event(JobEvent::Completed {
            token: state.token(),
            cancelled: false,
            result: Ok(JobOutput::Analyzed(analysis)),
        })
        .unwrap();
    state = state
        .apply_action(UiAction::Navigate(Workspace::Results))
        .unwrap();
    let ctx = egui::Context::default();
    tessera::desktop::configure_context(&ctx);
    ctx.set_theme(egui::Theme::Light);
    ctx.enable_accesskit();
    let mut jobs = JobRunner::default();
    let mut output = ctx.run_ui(
        egui::RawInput {
            screen_rect: Some(Rect::from_min_size(Pos2::ZERO, Vec2::new(1180., 780.))),
            ..Default::default()
        },
        |ui| render(ui, &mut state, &mut jobs),
    );
    output.textures_delta.clear();
    let boxes: Vec<_> = output
        .shapes
        .iter()
        .filter_map(|s| match &s.shape {
            egui::Shape::Rect(r)
                if r.fill == egui::Color32::from_rgb(242, 235, 220) && r.rect.height() == 36. =>
            {
                Some(r.rect)
            }
            _ => None,
        })
        .collect();
    assert_eq!(boxes.len(), 2);
    assert!(
        boxes[0].right() < boxes[1].left(),
        "disjoint outcome ranges must occupy disjoint coordinates: {boxes:?}"
    );
    let nodes = output.platform_output.accesskit_update.unwrap().nodes;
    assert!(nodes.iter().any(|(_, node)| node
        .label()
        .is_some_and(|label| label.contains("Boxplot")
            && label.contains("Outcome")
            && label.ends_with("Current data"))));
    assert!(
        boxes.iter().all(|r| r.bottom() <= 780.),
        "both groups remain visible at the supported window size: {boxes:?}"
    );
    assert!(
        !nodes
            .iter()
            .any(|(_, n)| n.value().is_some_and(|s| s.contains("Median: 1.5"))),
        "detailed values starts collapsed"
    );
    let size = Vec2::new(1180., 1600.);
    let mut output = ctx.run_ui(
        egui::RawInput {
            screen_rect: Some(Rect::from_min_size(Pos2::ZERO, size)),
            ..Default::default()
        },
        |ui| render(ui, &mut state, &mut jobs),
    );
    output.textures_delta.clear();
    let pos = state.control_rect("Detailed values").unwrap().center();
    for pressed in [true, false] {
        let mut output = ctx.run_ui(
            egui::RawInput {
                screen_rect: Some(Rect::from_min_size(Pos2::ZERO, size)),
                events: vec![
                    egui::Event::PointerMoved(pos),
                    egui::Event::PointerButton {
                        pos,
                        button: egui::PointerButton::Primary,
                        pressed,
                        modifiers: egui::Modifiers::NONE,
                    },
                ],
                ..Default::default()
            },
            |ui| render(ui, &mut state, &mut jobs),
        );
        output.textures_delta.clear();
    }
    for _ in 0..3 {
        output = ctx.run_ui(
            egui::RawInput {
                screen_rect: Some(Rect::from_min_size(Pos2::ZERO, size)),
                ..Default::default()
            },
            |ui| render(ui, &mut state, &mut jobs),
        );
        output.textures_delta.clear();
    }
    let nodes = output.platform_output.accesskit_update.unwrap().nodes;
    let node = nodes
        .iter()
        .find(|(_, n)| {
            n.label()
                .or_else(|| n.value())
                .is_some_and(|s| s.contains("Group 1") && s.contains("Median: 1.5"))
        })
        .expect("exact captured median");
    assert_ne!(node.1.role(), egui::accesskit::Role::Button);
    assert_ne!(node.1.role(), egui::accesskit::Role::CheckBox);
}

#[test]
fn empty_results_explain_the_next_action_in_both_themes() {
    for theme in [egui::Theme::Light, egui::Theme::Dark] {
        let mut state =
            DesktopState::from_project(Project::new("New", Dataset::new(vec![]).unwrap()).unwrap())
                .unwrap()
                .apply_action(UiAction::Navigate(Workspace::Results))
                .unwrap();
        let ctx = egui::Context::default();
        tessera::desktop::configure_context(&ctx);
        ctx.set_theme(theme);
        let mut jobs = JobRunner::default();
        let mut output = ctx.run_ui(
            egui::RawInput {
                screen_rect: Some(Rect::from_min_size(Pos2::ZERO, Vec2::new(760., 520.))),
                ..Default::default()
            },
            |ui| render(ui, &mut state, &mut jobs),
        );
        output.textures_delta.clear();
        assert!(state.control_rect("Go to Analyses").is_some());
        assert!(output.shapes.iter().any(|s| matches!(&s.shape,egui::Shape::Text(t) if t.galley.text()=="Your results will appear here")), "{:?}", output.shapes.iter().filter_map(|s| if let egui::Shape::Text(t) = &s.shape {Some(t.galley.text())} else {None}).collect::<Vec<_>>());
    }
}

#[test]
fn choosing_a_saved_result_closes_history_popup() {
    let data = Dataset::new(vec![Column::numeric(
        ColumnId(1),
        "Outcome",
        vec![Some(1.), Some(2.), Some(3.)],
    )
    .unwrap()])
    .unwrap();
    let mut project = Project::new("History", data).unwrap();
    for bins in [1, 2] {
        let analysis = analyze_with_budget(
            project.dataset(),
            &AnalysisRequest::Histogram(HistogramRequest {
                variable: ColumnId(1),
                bins: HistogramBins::Count(bins),
                filter: RowFilter::default(),
                frequency_weight: None,
            }),
            &JobControl::default(),
            &[],
            4096,
        )
        .unwrap();
        project = project.with_run(analysis).unwrap();
    }
    let mut state = DesktopState::from_project(project)
        .unwrap()
        .apply_action(UiAction::Navigate(Workspace::Results))
        .unwrap();
    let ctx = egui::Context::default();
    tessera::desktop::configure_context(&ctx);
    ctx.enable_accesskit();
    let mut jobs = JobRunner::default();
    compact_result_frame(&ctx, &mut state, &mut jobs, vec![]);
    let pos = state.control_rect("History").unwrap().center();
    for pressed in [true, false] {
        compact_result_frame(&ctx, &mut state, &mut jobs, pointer_click(pos, pressed));
    }
    let output = compact_result_frame(&ctx, &mut state, &mut jobs, vec![]);
    let nodes = output.platform_output.accesskit_update.unwrap().nodes;
    assert!(nodes
        .iter()
        .any(|(_, n)| n.label() == Some("Search analysis history")));
    let bounds = nodes
        .iter()
        .find(|(_, n)| {
            n.role() == egui::accesskit::Role::Button
                && n.label()
                    .is_some_and(|s| s.starts_with("Run 1 · Histogram ·"))
        })
        .unwrap()
        .1
        .bounds()
        .unwrap();
    let pos = Pos2::new(
        ((bounds.x0 + bounds.x1) / 2.) as f32,
        ((bounds.y0 + bounds.y1) / 2.) as f32,
    );
    for pressed in [true, false] {
        compact_result_frame(&ctx, &mut state, &mut jobs, pointer_click(pos, pressed));
    }
    let output = compact_result_frame(&ctx, &mut state, &mut jobs, vec![]);
    assert_eq!(state.selected_run().unwrap().id.0, 1);
    assert!(
        !output
            .platform_output
            .accesskit_update
            .unwrap()
            .nodes
            .iter()
            .any(|(_, n)| n.label() == Some("Search analysis history")),
        "choosing a run dismisses the popup"
    );
}
fn pointer_click(pos: Pos2, pressed: bool) -> Vec<egui::Event> {
    vec![
        egui::Event::PointerMoved(pos),
        egui::Event::PointerButton {
            pos,
            button: egui::PointerButton::Primary,
            pressed,
            modifiers: egui::Modifiers::NONE,
        },
    ]
}
fn result_frame(
    ctx: &egui::Context,
    state: &mut DesktopState,
    jobs: &mut JobRunner,
    events: Vec<egui::Event>,
) -> egui::FullOutput {
    let mut output = ctx.run_ui(
        egui::RawInput {
            screen_rect: Some(Rect::from_min_size(Pos2::ZERO, Vec2::new(1180., 780.))),
            events,
            ..Default::default()
        },
        |ui| render(ui, state, jobs),
    );
    output.textures_delta.clear();
    output
}

#[test]
fn results_have_visible_history_and_clear_analysis_actions() {
    let project = tessera::project_io::decode(
        &include_bytes!("../examples/mixed-analysis.tessera")[..],
        &JobControl::default(),
    )
    .unwrap();
    let selected = project.runs().last().unwrap().id;
    let mut state = DesktopState::from_project(project)
        .unwrap()
        .apply_action(UiAction::Navigate(Workspace::Results))
        .unwrap()
        .apply_action(UiAction::SelectRun(selected))
        .unwrap();
    let ctx = egui::Context::default();
    tessera::desktop::configure_context(&ctx);
    ctx.enable_accesskit();
    let mut jobs = JobRunner::default();
    let output = result_frame(&ctx, &mut state, &mut jobs, vec![]);
    assert!(state.control_rect("Search analysis history").is_some());
    assert!(state.control_rect("New analysis").is_some());
    assert!(state.control_rect("Edit selected analysis").is_some());
    assert!(output.shapes.iter().any(|shape| matches!(&shape.shape,
        egui::Shape::Text(text) if text.galley.text().contains("Analysis history"))));
    let nodes = &output
        .platform_output
        .accesskit_update
        .as_ref()
        .unwrap()
        .nodes;
    let saved_rows = nodes
        .iter()
        .filter(|(_, node)| {
            node.role() == egui::accesskit::Role::Button
                && node.label().or_else(|| node.value()).is_some_and(|label| {
                    label.contains(" · Run ") && label.ends_with("Current data")
                })
        })
        .collect::<Vec<_>>();
    assert_eq!(
        saved_rows.len(),
        state.project().runs().len(),
        "{:?}",
        saved_rows
            .iter()
            .map(|(_, node)| (node.role(), node.label(), node.value()))
            .collect::<Vec<_>>()
    );
    assert!(
        saved_rows
            .iter()
            .all(|(_, node)| node.bounds().is_some_and(|rect| rect.y1 - rect.y0 <= 42.)),
        "saved-result rows stay compact"
    );
    assert!(saved_rows.iter().any(|(_, node)| node.toggled()
        == Some(egui::accesskit::Toggled::True)
        && node
            .label()
            .or_else(|| node.value())
            .is_some_and(|label| label.contains(&format!("Run {}", selected.0)))));
    let pos = state
        .control_rect("Edit selected analysis")
        .unwrap()
        .center();
    for pressed in [true, false] {
        result_frame(&ctx, &mut state, &mut jobs, pointer_click(pos, pressed));
    }
    assert_eq!(
        state.workspace(),
        Workspace::Analyses,
        "{:?}",
        state.error()
    );
    assert!(state.checked_draft_request().is_ok());
}

fn compact_result_frame(
    ctx: &egui::Context,
    state: &mut DesktopState,
    jobs: &mut JobRunner,
    events: Vec<egui::Event>,
) -> egui::FullOutput {
    let mut output = ctx.run_ui(
        egui::RawInput {
            screen_rect: Some(Rect::from_min_size(Pos2::ZERO, Vec2::new(760., 780.))),
            events,
            ..Default::default()
        },
        |ui| render(ui, state, jobs),
    );
    output.textures_delta.clear();
    output
}
