use super::*;
use crate::{desktop::DesktopState, export::report_regression::controls, operation::RunId};
use eframe::egui;

#[test]
fn actual_results_and_settings_use_only_frozen_captures() {
    for case in controls::cases() {
        let (data, result) = controls::run(case);
        let before = result.clone();
        let mut state =
            DesktopState::from_project(crate::project::Project::new("Current", data).unwrap())
                .unwrap();
        let metadata = crate::data::ColumnMetadata {
            name: "Current renamed outcome".into(),
            ..state
                .project()
                .dataset()
                .column(crate::data::ColumnId(1))
                .unwrap()
                .metadata()
                .clone()
        };
        state = state
            .apply_action(crate::desktop::UiAction::Metadata {
                column: crate::data::ColumnId(1),
                metadata,
            })
            .unwrap();
        state = state
            .apply_action(crate::desktop::UiAction::EditCell {
                column: crate::data::ColumnId(1),
                row: 0,
                value: crate::data::Cell::Number(999.),
            })
            .unwrap();
        state = state
            .apply_action(crate::desktop::UiAction::SetDraft(
                crate::desktop::draft::AnalysisDraft::Describe(crate::analysis::DescribeRequest {
                    columns: vec![crate::data::ColumnId(2)],
                    statistics: vec![crate::analysis::Statistic::Maximum],
                    missing_policy: crate::analysis::MissingPolicy::PerVariable,
                    filter: crate::analysis::RowFilter::default(),
                    frequency_weight: None,
                }),
            ))
            .unwrap();
        let ctx = egui::Context::default();
        crate::desktop::configure_context(&ctx);
        ctx.enable_accesskit();
        let mut out = ctx.run_ui(
            egui::RawInput {
                screen_rect: Some(egui::Rect::from_min_size(
                    egui::Pos2::ZERO,
                    egui::vec2(1400., 18000.),
                )),
                ..Default::default()
            },
            |ui| {
                egui::CentralPanel::default().show(ui, |ui| {
                    show(ui, &mut state, RunId(17), &result);
                    settings(ui, &state, &result);
                });
            },
        );
        out.textures_delta.clear();
        let chart_ids = out
            .platform_output
            .accesskit_update
            .as_ref()
            .unwrap()
            .nodes
            .iter()
            .filter_map(|(id, n)| {
                n.label()
                    .is_some_and(|label| label.contains(" · Exact values in the table"))
                    .then_some(*id)
            })
            .collect::<Vec<_>>();
        let labels = out
            .platform_output
            .accesskit_update
            .unwrap()
            .nodes
            .into_iter()
            .filter_map(|(_, n)| n.label().or_else(|| n.value()).map(str::to_owned))
            .collect::<Vec<_>>()
            .join("\n");
        assert!(labels.contains("Captured outcome <Y>"), "{case}: {labels}");
        assert!(!labels.contains("Current renamed outcome"));
        assert_eq!(result, before);
        if let RegressionOutcome::Fitted(f) = &result.outcome {
            for coefficient in &f.coefficients {
                for e in [
                    &coefficient.estimate,
                    &coefficient.standard_error,
                    &coefficient.t,
                    &coefficient.p,
                    &coefficient.interval[0],
                    &coefficient.interval[1],
                ] {
                    assert!(
                        labels.contains(&estimate(e)),
                        "{case}: omitted coefficient {e:?}"
                    );
                }
            }
            let s = &f.statistics;
            for e in [
                &s.sse,
                &s.sst,
                &s.model_ss,
                &s.residual_standard_error,
                &s.r_squared,
                &s.adjusted_r_squared,
                &s.f,
                &s.f_p,
                &f.condition,
                &f.outcome_scale,
            ] {
                assert!(
                    labels.contains(&estimate(e)),
                    "{case}: omitted fit value {e:?}"
                );
            }
            assert_eq!(chart_ids.len(), 2);
            assert_ne!(chart_ids[0], chart_ids[1]);
        }
        match &result.outcome {
            crate::analysis::RegressionOutcome::Fitted(_) => assert!(
                labels.contains("Residual standard error"),
                "{case}: {labels}"
            ),
            crate::analysis::RegressionOutcome::Unavailable(f) => {
                assert!(labels.contains(&f.reason), "{case}: {labels}")
            }
        }
    }
}
fn labels(output: &egui::FullOutput) -> Vec<String> {
    output
        .platform_output
        .accesskit_update
        .as_ref()
        .unwrap()
        .nodes
        .iter()
        .filter_map(|(_, n)| n.label().or_else(|| n.value()).map(str::to_owned))
        .collect()
}
#[test]
fn captured_definition_labels_are_fixed_height_virtual_rows() {
    use crate::data::{Cell, ColumnId, ColumnMetadata, Dataset, ValueLabel};
    let (data, initial) = controls::many(4);
    let column = data.column(ColumnId(1)).unwrap();
    let outcome = column
        .with_metadata(ColumnMetadata {
            value_labels: (0..600)
                .map(|i| ValueLabel {
                    value: Cell::Number(i as f64),
                    label: format!("Captured label {i:03}"),
                })
                .collect(),
            ..column.metadata().clone()
        })
        .unwrap();
    let data = Dataset::new(vec![outcome, data.column(ColumnId(2)).unwrap().clone()]).unwrap();
    let r = crate::analysis::regression_with_budget(
        &data,
        &initial.request,
        &crate::operation::JobControl::default(),
        &[],
        initial.request.heap_bytes().unwrap(),
    )
    .unwrap();
    r.validate().unwrap();
    let ctx = egui::Context::default();
    crate::desktop::configure_context(&ctx);
    ctx.enable_accesskit();
    let draw = |events| {
        let mut out = ctx.run_ui(
            egui::RawInput {
                screen_rect: Some(egui::Rect::from_min_size(
                    egui::Pos2::ZERO,
                    egui::vec2(1400., 1100.),
                )),
                events,
                ..Default::default()
            },
            |ui| {
                egui::CentralPanel::default().show(ui, |ui| {
                    definitions(ui, &r);
                });
            },
        );
        out.textures_delta.clear();
        out
    };
    let initial = draw(vec![]);
    let before = labels(&initial);
    let point = initial
        .shapes
        .iter()
        .find_map(|s| match &s.shape {
            egui::Shape::Text(t) if t.galley.text().contains("Captured label 000") => {
                Some(t.pos + egui::vec2(8., 8.))
            }
            _ => None,
        })
        .unwrap();
    draw(vec![
        egui::Event::PointerMoved(point),
        egui::Event::MouseWheel {
            unit: egui::MouseWheelUnit::Point,
            phase: egui::TouchPhase::Move,
            delta: egui::vec2(0., -600.),
            modifiers: egui::Modifiers::NONE,
        },
    ]);
    for _ in 0..20 {
        draw(vec![]);
    }
    let after = labels(&draw(vec![]));
    assert!(
        after.iter().any(|s| s.contains("Captured label")
            && s.contains("Value label")
            && !before.contains(s)),
        "definition viewport changed"
    );
    assert!(after.len() < 30, "only visible rows are rendered");
}
fn frame(
    ctx: &egui::Context,
    state: &mut DesktopState,
    result: &ComputedRegression,
    events: Vec<egui::Event>,
) -> egui::FullOutput {
    let mut output = ctx.run_ui(
        egui::RawInput {
            screen_rect: Some(egui::Rect::from_min_size(
                egui::Pos2::ZERO,
                egui::vec2(1400., 1100.),
            )),
            events,
            ..Default::default()
        },
        |ui| {
            egui::CentralPanel::default().show(ui, |ui| {
                let RegressionOutcome::Fitted(f) = &result.outcome else {
                    unreachable!()
                };
                plots::show(ui, state, RunId(17), result, &f.residual_diagnostics);
            });
        },
    );
    output.textures_delta.clear();
    output
}
#[test]
fn virtual_exact_rows_keep_source_frequency_ordinals_and_keyboard_focus() {
    let (data, result) = controls::many(600);
    let mut state =
        DesktopState::from_project(crate::project::Project::new("Scroll", data).unwrap()).unwrap();
    let ctx = egui::Context::default();
    crate::desktop::configure_context(&ctx);
    ctx.enable_accesskit();
    let initial = frame(&ctx, &mut state, &result, vec![]);
    let old = labels(&initial);
    let point = initial
        .shapes
        .iter()
        .find_map(|s| match &s.shape {
            egui::Shape::Text(t) if t.galley.text().starts_with("Cohort ordinal:") => {
                Some(t.pos + egui::vec2(20., 8.))
            }
            _ => None,
        })
        .unwrap();
    frame(
        &ctx,
        &mut state,
        &result,
        vec![
            egui::Event::PointerMoved(point),
            egui::Event::MouseWheel {
                unit: egui::MouseWheelUnit::Point,
                phase: egui::TouchPhase::Move,
                delta: egui::vec2(0., -600.),
                modifiers: egui::Modifiers::NONE,
            },
        ],
    );
    for _ in 0..20 {
        frame(&ctx, &mut state, &result, vec![]);
    }
    let scrolled = frame(&ctx, &mut state, &result, vec![]);
    let (row, _point) = scrolled
        .shapes
        .iter()
        .find_map(|s| match &s.shape {
            egui::Shape::Text(t)
                if t.galley.text().starts_with("Cohort ordinal:")
                    && !old.iter().any(|old| old == t.galley.text())
                    && s.clip_rect.contains(t.pos + egui::vec2(8., 8.)) =>
            {
                Some((t.galley.text().to_owned(), t.pos + egui::vec2(8., 8.)))
            }
            _ => None,
        })
        .expect("visible virtual viewport changed");
    assert!(
        row.contains("Source row:")
            && row.contains("Frequency: 1")
            && row.contains("Fitted value:")
            && row.contains("Cook’s distance:"),
        "{row}"
    );
    let mut focused_rows = vec![];
    for _ in 0..5 {
        let next = frame(
            &ctx,
            &mut state,
            &result,
            vec![egui::Event::Key {
                key: egui::Key::Tab,
                physical_key: None,
                pressed: true,
                repeat: false,
                modifiers: egui::Modifiers::NONE,
            }],
        );
        let update = next.platform_output.accesskit_update.unwrap();
        if let Some(label) = update.nodes.iter().find_map(|(id, node)| {
            (*id == update.focus)
                .then(|| node.label().or_else(|| node.value()))
                .flatten()
        }) {
            if label.starts_with("Cohort ordinal:") {
                focused_rows.push(label.to_owned());
            }
        }
        frame(
            &ctx,
            &mut state,
            &result,
            vec![egui::Event::Key {
                key: egui::Key::Tab,
                physical_key: None,
                pressed: false,
                repeat: false,
                modifiers: egui::Modifiers::NONE,
            }],
        );
    }
    assert!(
        focused_rows.len() >= 2,
        "Tab reaches distinct exact rows: {focused_rows:?}"
    );
    assert_ne!(focused_rows[0], focused_rows[1]);
    assert!(focused_rows
        .iter()
        .all(|row| row.contains("Frequency: 1") && row.contains("Cook’s distance:")));
}
