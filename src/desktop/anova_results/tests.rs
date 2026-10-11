use super::*;
use crate::{
    analysis::anova::compute::ENTRIES,
    data::{Column, ColumnId, Dataset},
    operation::JobControl,
    project::Project,
};
#[test]
fn actual_passive_egui_display_and_retained_settings_preserve_capture() {
    for design in 0..3 {
        let columns = (1..=5)
            .map(|id| {
                Column::numeric(
                    ColumnId(id),
                    &format!("Column {id}"),
                    vec![Some(1.), Some(2.), Some(4.), Some(8.)],
                )
                .unwrap()
            })
            .collect();
        let data = Dataset::new(columns).unwrap();
        let r = AnovaRequest {
            design: match design {
                0 => AnovaDesign::OneWay {
                    outcome: ColumnId(1),
                    factor: ColumnId(2),
                },
                1 => AnovaDesign::Factorial {
                    outcome: ColumnId(1),
                    factors: vec![ColumnId(2), ColumnId(3)],
                },
                _ => AnovaDesign::Repeated {
                    subject: ColumnId(5),
                    factors: vec![AnovaWithinFactor {
                        name: " Time ".into(),
                        levels: vec![" First ".into(), "Second".into()],
                    }],
                    measurements: vec![ColumnId(1), ColumnId(2)],
                },
            },
            confidence: ConfidenceLevel::P95,
            filter: RowFilter::default(),
            frequency_weight: None,
            planned_pairs: vec![],
            all_pairs: vec![],
        };
        let result = anova_with_budget(
            &data,
            &r,
            &JobControl::default(),
            &[],
            std::mem::size_of::<AnovaRequest>() + r.heap_bytes().unwrap(),
        )
        .unwrap();
        let original = result.clone();
        let project = Project::new("Passive", data)
            .unwrap()
            .with_run(CompletedAnalysis::Anova(result))
            .unwrap();
        let mut bytes = Vec::new();
        crate::project_io::encode(&project, &mut bytes, &JobControl::default()).unwrap();
        let start = ENTRIES.with(|n| n.get());
        let reopened = crate::project_io::decode(bytes.as_slice(), &JobControl::default()).unwrap();
        let CompletedAnalysis::Anova(capture) = &reopened.runs()[0].analysis else {
            panic!()
        };
        let ctx = egui::Context::default();
        crate::desktop::configure_context(&ctx);
        let mut output = ctx.run_ui(
            egui::RawInput {
                screen_rect: Some(egui::Rect::from_min_size(
                    egui::Pos2::ZERO,
                    egui::vec2(1000., 1200.),
                )),
                ..Default::default()
            },
            |ui| {
                egui::CentralPanel::default().show(ui, |ui| {
                    show(ui, capture);
                    settings(ui, &capture.request, &capture.columns);
                });
            },
        );
        assert!(!output.shapes.is_empty());
        output.textures_delta.clear();
        assert_eq!(*capture, original);
        assert_eq!(ENTRIES.with(|n| n.get()), start);
        let draft = crate::desktop::draft::AnalysisDraft::from_request(AnalysisRequest::Anova(
            capture.request.clone(),
        ));
        assert_eq!(draft.procedure(), crate::desktop::draft::Procedure::Anova);
        assert!(draft.matches_saved(AnalysisRequestRef::Anova(&capture.request)));
        assert!(crate::desktop::draft::Procedure::ALL
            .contains(&crate::desktop::draft::Procedure::Anova));
        assert_eq!(
            draft
                .for_procedure(crate::desktop::draft::Procedure::Pearson)
                .procedure(),
            crate::desktop::draft::Procedure::Pearson
        );
        assert!(draft.build_request(reopened.dataset()).is_ok());
    }
}
#[test]
fn loaded_same_recipe_uses_the_selected_capture_metadata() {
    use crate::desktop::{DesktopState, UiAction};
    let data = Dataset::new(vec![
        Column::numeric(
            ColumnId(1),
            "First outcome",
            vec![Some(1.), Some(2.), Some(4.), Some(8.)],
        )
        .unwrap(),
        Column::numeric(
            ColumnId(2),
            "Factor",
            vec![Some(0.), Some(0.), Some(1.), Some(1.)],
        )
        .unwrap(),
    ])
    .unwrap();
    let r = AnovaRequest {
        design: AnovaDesign::OneWay {
            outcome: ColumnId(1),
            factor: ColumnId(2),
        },
        confidence: ConfidenceLevel::P95,
        filter: RowFilter::default(),
        frequency_weight: None,
        planned_pairs: vec![],
        all_pairs: vec![],
    };
    let a = anova_with_budget(
        &data,
        &r,
        &JobControl::default(),
        &[],
        std::mem::size_of::<AnovaRequest>() + r.heap_bytes().unwrap(),
    )
    .unwrap();
    let p = Project::new("History", data)
        .unwrap()
        .with_run(CompletedAnalysis::Anova(a))
        .unwrap();
    let old = p.dataset().column(ColumnId(1)).unwrap();
    let metadata = crate::data::ColumnMetadata {
        name: "Second outcome".into(),
        ..old.metadata().clone()
    };
    let d = p
        .dataset()
        .replace_column(old.with_metadata(metadata).unwrap())
        .unwrap();
    let b = anova_with_budget(
        &d,
        &r,
        &JobControl::default(),
        &[],
        std::mem::size_of::<AnovaRequest>() + r.heap_bytes().unwrap(),
    )
    .unwrap();
    let p = p
        .with_dataset(d)
        .unwrap()
        .with_run(CompletedAnalysis::Anova(b))
        .unwrap();
    let mut state = DesktopState::from_project(p)
        .unwrap()
        .apply_action(UiAction::SelectRun(crate::operation::RunId(2)))
        .unwrap();
    state.load_selected_request();
    let ctx = egui::Context::default();
    crate::desktop::configure_context(&ctx);
    let mut jobs = crate::jobs::JobRunner::default();
    let mut output = ctx.run_ui(
        egui::RawInput {
            screen_rect: Some(egui::Rect::from_min_size(
                egui::Pos2::ZERO,
                egui::vec2(1000., 1500.),
            )),
            ..Default::default()
        },
        |ui| {
            egui::CentralPanel::default().show(ui, |ui| {
                super::super::procedure_forms::settings(ui, &mut state, &mut jobs)
            });
        },
    );
    let labels = output
        .shapes
        .iter()
        .filter_map(|s| match &s.shape {
            egui::Shape::Text(v) => Some(v.galley.text().to_owned()),
            _ => None,
        })
        .collect::<Vec<_>>();
    output.textures_delta.clear();
    assert!(
        labels.iter().any(|s| s.contains("Second outcome")),
        "selected capture missing; labels={labels:?}"
    );
    assert!(!labels.iter().any(|s| s.contains("First outcome")));
}
#[test]
fn multiline_virtual_rows_can_scroll_to_the_final_member_value() {
    let ctx = egui::Context::default();
    crate::desktop::configure_context(&ctx);
    let mut visible = false;
    let mut reported = 0.;
    let mut expected = 0.;
    for frame in 0..5 {
        let events = if frame > 0 {
            vec![
                egui::Event::PointerMoved(egui::pos2(100., 100.)),
                egui::Event::MouseWheel {
                    unit: egui::MouseWheelUnit::Point,
                    phase: egui::TouchPhase::Move,
                    delta: egui::vec2(0., -100000.),
                    modifiers: egui::Modifiers::default(),
                },
            ]
        } else {
            vec![]
        };
        let mut output = ctx.run_ui(
            egui::RawInput {
                screen_rect: Some(egui::Rect::from_min_size(
                    egui::Pos2::ZERO,
                    egui::vec2(800., 300.),
                )),
                events,
                ..Default::default()
            },
            |ui| {
                egui::CentralPanel::default().show(ui, |ui| {
                    expected = 100.
                        * 12.
                        * (ui.text_style_height(&egui::TextStyle::Body)
                            + ui.spacing().item_spacing.y)
                        - ui.spacing().item_spacing.y;
                    reported = rows(ui, "last-member-proof", 100, 12, |ui, i| {
                        for j in 0..12 {
                            ui.label(format!("Member {i} value {j}"));
                        }
                    })
                    .content_size
                    .y;
                });
            },
        );
        visible |= output.shapes.iter().any(|s| match &s.shape {
            egui::Shape::Text(t) if t.galley.text() == "Member 99 value 11" => s
                .clip_rect
                .intersects(egui::Rect::from_min_size(t.pos, t.galley.size())),
            _ => false,
        });
        output.textures_delta.clear();
    }
    assert!(
        reported >= expected - 1.,
        "scroll content height {reported} is smaller than full multiline content {expected}"
    );
    assert!(
        visible,
        "final member's last value never enters the viewport"
    );
}
#[test]
fn actual_anova_desktop_worker_completion_cancellation_and_stale_tokens() {
    use crate::{
        desktop::{DesktopState, UiAction},
        jobs::{JobEvent, JobOperation, JobOutput, JobRunner},
        operation::RunId,
    };
    use std::{
        sync::Arc,
        time::{Duration, Instant},
    };
    let data = Dataset::new(vec![
        Column::numeric(
            ColumnId(1),
            "Outcome",
            vec![Some(1.), Some(2.), Some(4.), Some(8.)],
        )
        .unwrap(),
        Column::numeric(
            ColumnId(2),
            "Factor",
            vec![Some(0.), Some(0.), Some(1.), Some(1.)],
        )
        .unwrap(),
    ])
    .unwrap();
    let request = AnovaRequest {
        design: AnovaDesign::OneWay {
            outcome: ColumnId(1),
            factor: ColumnId(2),
        },
        confidence: ConfidenceLevel::P95,
        filter: RowFilter::default(),
        frequency_weight: None,
        planned_pairs: vec![],
        all_pairs: vec![],
    };
    let result = CompletedAnalysis::Anova(
        anova_with_budget(
            &data,
            &request,
            &JobControl::default(),
            &[],
            std::mem::size_of::<AnovaRequest>() + request.heap_bytes().unwrap(),
        )
        .unwrap(),
    );
    for cancelled in [false, true] {
        let mut state =
            DesktopState::from_project(Project::new("Worker", data.clone()).unwrap()).unwrap();
        let original = Arc::clone(state.project());
        let mut jobs = JobRunner::default();
        state
            .start_operation(
                &mut jobs,
                JobOperation::Analyze {
                    project: Arc::clone(&original),
                    request: AnalysisRequest::Anova(request.clone()),
                },
            )
            .unwrap();
        if cancelled {
            jobs.cancel();
        }
        let deadline = Instant::now();
        while jobs.is_running() {
            state.poll(&mut jobs);
            assert!(deadline.elapsed() < Duration::from_secs(10));
            std::thread::sleep(Duration::from_millis(2));
        }
        state.poll(&mut jobs);
        assert!(original.runs().is_empty());
        assert_eq!(state.project().runs().len(), usize::from(!cancelled));
        if !cancelled {
            assert_eq!(state.project().runs()[0].analysis, result);
            assert!(state.project().runs().iter().any(|r| r.id == RunId(1)));
        }
    }
    let mut state = DesktopState::from_project(Project::new("Stale", data).unwrap()).unwrap();
    let token = state.token();
    for altered in 0..4 {
        let mut stale = token;
        match altered {
            0 => stale.document += 1,
            1 => stale.revision.0 += 1,
            2 => stale.generation += 1,
            _ => {}
        }
        state
            .handle_event(JobEvent::Completed {
                token: stale,
                cancelled: altered == 3,
                result: Ok(JobOutput::Analyzed(result.clone())),
            })
            .unwrap();
        assert!(state.project().runs().is_empty());
    }
    assert!(state.apply_action(UiAction::SelectRun(RunId(1))).is_err());
}
