use super::*;
use crate::{
    data::{Column, ColumnId, Dataset},
    operation::JobControl,
    project::Project,
};
#[test]
fn reliability_actual_captured_results_include_all_metrics_and_unchanged_historical_labels() {
    let data = Dataset::new(vec![
        Column::numeric(ColumnId(1), "Saved first", vec![Some(0.), Some(1.)]).unwrap(),
        Column::numeric(ColumnId(2), "Saved second", vec![Some(3.), Some(3.)]).unwrap(),
    ])
    .unwrap();
    let request = ReliabilityRequest {
        items: vec![ColumnId(2), ColumnId(1)],
        reversals: vec![],
        filter: RowFilter::default(),
        frequency_weight: None,
    };
    let r = crate::analysis::reliability_with_budget(
        &data,
        &request,
        &JobControl::default(),
        &[],
        std::mem::size_of::<ReliabilityRequest>() + request.heap_bytes().unwrap(),
    )
    .unwrap();
    let p = Project::new("Saved", data)
        .unwrap()
        .with_run(CompletedAnalysis::Reliability(r.clone()))
        .unwrap();
    let current = Dataset::from_revision(
        vec![Column::text(
            ColumnId(1),
            "Present changed",
            vec![Some("x".into()), Some("y".into())],
        )
        .unwrap()],
        crate::data::RevisionId(2),
    )
    .unwrap();
    let p = p.with_dataset(current).unwrap();
    let mut state = DesktopState::from_project(p).unwrap();
    let before = r.clone();
    let ctx = egui::Context::default();
    let mut out = ctx.run_ui(
        egui::RawInput {
            screen_rect: Some(egui::Rect::from_min_size(
                egui::Pos2::ZERO,
                egui::vec2(1600., 1600.),
            )),
            ..Default::default()
        },
        |ui| {
            egui::CentralPanel::default().show(ui, |ui| {
                show(ui, &mut state, &r);
                settings(ui, &mut state, &r);
            });
        },
    );
    let labels = out
        .shapes
        .iter()
        .filter_map(|s| {
            if let egui::Shape::Text(t) = &s.shape {
                Some(t.galley.text().to_owned())
            } else {
                None
            }
        })
        .collect::<Vec<_>>()
        .join("\n");
    out.textures_delta.clear();
    for label in [
        "complete physical rows; frequency N",
        "Raw alpha if removed",
        "Standardized alpha if removed",
        "Scored mean",
        "Scored sample SD",
        "Corrected item-total correlation",
        "Saved second",
        "Unavailable",
        "Alpha does not establish unidimensionality.",
        "complete captured definitions",
    ] {
        assert!(labels.contains(label), "missing {label}: {labels}");
    }
    assert!(!labels.contains("Present changed"));
    assert_eq!(r, before);
    let draft = super::super::draft::reliability::ReliabilityDraft::from_request(r.request.clone());
    assert!(draft.matches_saved(&r.request));
    assert!(draft.build_request(state.project().dataset()).is_err());
}

#[test]
fn alpha_presentation_is_compact_read_only_and_preserves_negative_exact_value() {
    let ctx = egui::Context::default();
    ctx.enable_accesskit();
    let exact = -12.3456789012345;
    let mut out = ctx.run_ui(egui::RawInput::default(), |ui| {
        estimate(
            ui,
            "Raw alpha",
            &Estimate {
                value: Some(exact),
                reason: None,
            },
        );
    });
    out.textures_delta.clear();
    assert!(out
        .shapes
        .iter()
        .any(|s| matches!(&s.shape, egui::Shape::Text(t) if t.galley.text().contains("-12.35"))));
    let nodes = out.platform_output.accesskit_update.unwrap().nodes;
    let node = nodes
        .iter()
        .find(|(_, n)| {
            n.label()
                .or_else(|| n.value())
                .is_some_and(|s| s.contains(&exact.to_string()))
        })
        .expect("full precision alpha remains inspectable");
    assert_ne!(node.1.role(), egui::accesskit::Role::Button);
}

#[test]
fn narrow_reliability_table_scrolls_to_deletion_metrics() {
    let data = Dataset::new(vec![
        Column::numeric(ColumnId(1), "First", vec![Some(1.), Some(3.)]).unwrap(),
        Column::numeric(ColumnId(2), "Second", vec![Some(2.), Some(4.)]).unwrap(),
    ])
    .unwrap();
    let request = ReliabilityRequest {
        items: vec![ColumnId(1), ColumnId(2)],
        reversals: vec![],
        filter: RowFilter::default(),
        frequency_weight: None,
    };
    let r = crate::analysis::reliability_with_budget(
        &data,
        &request,
        &JobControl::default(),
        &[],
        std::mem::size_of::<ReliabilityRequest>() + request.heap_bytes().unwrap(),
    )
    .unwrap();
    let mut state = DesktopState::from_project(Project::new("Narrow", data).unwrap()).unwrap();
    let ctx = egui::Context::default();
    let mut frame = |events| {
        let mut out = ctx.run_ui(
            egui::RawInput {
                screen_rect: Some(egui::Rect::from_min_size(
                    egui::Pos2::ZERO,
                    egui::vec2(760., 520.),
                )),
                events,
                ..Default::default()
            },
            |ui| {
                egui::CentralPanel::default().show(ui, |ui| show(ui, &mut state, &r));
            },
        );
        out.textures_delta.clear();
        out
    };
    let initial = frame(vec![]);
    let point = initial
        .shapes
        .iter()
        .find_map(|s| match &s.shape {
            egui::Shape::Text(t) if t.galley.text() == "Item" => Some(t.pos + egui::vec2(8., 8.)),
            _ => None,
        })
        .unwrap();
    frame(vec![
        egui::Event::PointerMoved(point),
        egui::Event::MouseWheel {
            unit: egui::MouseWheelUnit::Point,
            phase: egui::TouchPhase::Move,
            delta: egui::vec2(-1200., 0.),
            modifiers: egui::Modifiers::NONE,
        },
    ]);
    for _ in 0..20 {
        frame(vec![]);
    }
    let out = frame(vec![]);
    assert!(out.shapes.iter().any(|s| matches!(&s.shape, egui::Shape::Text(t) if t.galley.text().contains("Standardized alpha if removed") && s.clip_rect.contains(t.pos + egui::vec2(8., 8.)))), "the last table column is reachable at the supported narrow width");
    assert_eq!(r.request, request);
}
