use super::*;
use tessera::{analysis::*, desktop::draft::Procedure};
#[test]
fn exploration_native_chooser_exposes_completed_families() {
    let ctx = egui::Context::default();
    tessera::desktop::configure_context(&ctx);
    ctx.set_theme(egui::Theme::Light);
    let size = Vec2::new(1180., 780.);
    let mut state = state()
        .apply_action(UiAction::Navigate(Workspace::Analyses))
        .unwrap();
    let mut jobs = JobRunner::default();
    frame(&ctx, &mut state, &mut jobs, size, vec![]);
    click(&ctx, &mut state, &mut jobs, "Procedure", size);
    frame(&ctx, &mut state, &mut jobs, size, vec![]);
    for procedure in [
        Procedure::Frequencies,
        Procedure::Histogram,
        Procedure::Scatter,
        Procedure::Boxplot,
    ] {
        assert!(
            state.control_rect(procedure.label()).is_some(),
            "missing {}",
            procedure.label()
        );
    }
}
#[test]
fn exploration_native_histogram_draws_stored_bins_and_axes() {
    let mut state = state();
    let analysis = analyze_with_budget(
        state.project().dataset(),
        &AnalysisRequest::Histogram(HistogramRequest {
            variable: ColumnId(1),
            bins: HistogramBins::Count(2),
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
    let mut jobs = JobRunner::default();
    let mut output = ctx.run_ui(
        egui::RawInput {
            screen_rect: Some(Rect::from_min_size(Pos2::ZERO, Vec2::new(1180., 780.))),
            ..Default::default()
        },
        |ui| render(ui, &mut state, &mut jobs),
    );
    output.textures_delta.clear();
    assert!(
        output
            .shapes
            .iter()
            .any(|s| matches!(&s.shape,egui::Shape::Text(t) if t.galley.text()=="Frequency")),
        "missing drawn frequency axis"
    );
}

fn explore_state() -> DesktopState {
    let data = Dataset::new(vec![
        Column::numeric(
            ColumnId(1),
            "X",
            vec![Some(0.), Some(1.), Some(2.), Some(100.)],
        )
        .unwrap(),
        Column::numeric(
            ColumnId(2),
            "Y",
            vec![Some(10.), Some(11.), Some(12.), Some(13.)],
        )
        .unwrap(),
        Column::text(
            ColumnId(3),
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
    DesktopState::from_project(Project::new("Explore", data).unwrap()).unwrap()
}
fn key(key: Key, modifiers: Modifiers) -> Event {
    Event::Key {
        key,
        physical_key: None,
        pressed: true,
        repeat: false,
        modifiers,
    }
}
fn wait(state: &mut DesktopState, jobs: &mut JobRunner) {
    let until = Instant::now() + Duration::from_secs(10);
    while jobs.is_running() {
        state.poll(jobs);
        assert!(Instant::now() < until);
        std::thread::yield_now();
    }
}
#[test]
fn exploration_forms_choose_roles_bins_group_and_run_with_keys() {
    for p in [
        Procedure::Frequencies,
        Procedure::Histogram,
        Procedure::Scatter,
        Procedure::Boxplot,
    ] {
        let ctx = egui::Context::default();
        tessera::desktop::configure_context(&ctx);
        ctx.set_theme(egui::Theme::Light);
        let size = Vec2::new(1180., 900.);
        let mut state = explore_state()
            .apply_action(UiAction::Navigate(Workspace::Analyses))
            .unwrap();
        state = state
            .apply_action(UiAction::SetAnalysisMode(
                tessera::desktop::AnalysisMode::Advanced,
            ))
            .unwrap();
        let mut jobs = JobRunner::default();
        frame(&ctx, &mut state, &mut jobs, size, vec![]);
        click(&ctx, &mut state, &mut jobs, "Search procedures", size);
        frame(
            &ctx,
            &mut state,
            &mut jobs,
            size,
            vec![Event::Text(p.label().into())],
        );
        frame(&ctx, &mut state, &mut jobs, size, vec![]);
        click(&ctx, &mut state, &mut jobs, p.label(), size);
        frame(&ctx, &mut state, &mut jobs, size, vec![]);
        assert_eq!(state.draft().procedure(), p);
        let roles = match p {
            Procedure::Frequencies => {
                vec![("Add frequency variable", 1), ("Add frequency variable", 3)]
            }
            Procedure::Histogram => vec![("Variable", 1)],
            Procedure::Scatter => vec![("X variable", 1), ("Y variable", 2)],
            Procedure::Boxplot => vec![("Outcome", 1), ("Grouping variable (optional)", 3)],
            _ => unreachable!(),
        };
        for (label, id) in roles {
            assert!(state.control_rect(label).is_some(), "missing {p:?} {label}");
            click(&ctx, &mut state, &mut jobs, label, size);
            frame(&ctx, &mut state, &mut jobs, size, vec![]);
            click(&ctx, &mut state, &mut jobs, &format!("{label}: {id}"), size);
            frame(&ctx, &mut state, &mut jobs, size, vec![]);
        }
        if p == Procedure::Histogram {
            click(&ctx, &mut state, &mut jobs, "Automatic (Sturges)", size);
            frame(&ctx, &mut state, &mut jobs, size, vec![]);
            let id = state.control_id("Bin count").unwrap();
            ctx.memory_mut(|m| m.request_focus(id));
            frame(
                &ctx,
                &mut state,
                &mut jobs,
                size,
                vec![
                    key(Key::A, Modifiers::COMMAND),
                    Event::Text("3".into()),
                    key(Key::Enter, Modifiers::NONE),
                ],
            );
        }
        let request = state.checked_draft_request().unwrap();
        if p == Procedure::Histogram {
            assert!(
                matches!(&request,AnalysisRequest::Histogram(r) if r.bins==HistogramBins::Count(3))
            );
        }
        // Every form uses the same checked request path for command-Enter.
        frame(
            &ctx,
            &mut state,
            &mut jobs,
            size,
            vec![key(Key::Enter, Modifiers::COMMAND)],
        );
        wait(&mut state, &mut jobs);
        assert_eq!(state.project().runs().len(), 1);
        assert!(state
            .draft()
            .matches_saved(state.selected_run().unwrap().analysis.request()));
    }
}
fn frequency_state() -> DesktopState {
    let data = Dataset::new(vec![
        Column::text(
            ColumnId(1),
            "Category",
            (0..600).map(|i| Some(format!("Category {i:03}"))).collect(),
        )
        .unwrap(),
        Column::text(
            ColumnId(2),
            "Other",
            (0..600).map(|i| Some(format!("Other {i:03}"))).collect(),
        )
        .unwrap(),
    ])
    .unwrap();
    let result = analyze_with_budget(
        &data,
        &AnalysisRequest::Frequencies(FrequenciesRequest {
            columns: vec![ColumnId(1), ColumnId(2)],
            filter: RowFilter::default(),
            frequency_weight: None,
        }),
        &JobControl::default(),
        &[],
        4096,
    )
    .unwrap();
    DesktopState::from_project(
        Project::new("Search", data)
            .unwrap()
            .with_run(result.clone())
            .unwrap()
            .with_run(result)
            .unwrap(),
    )
    .unwrap()
    .apply_action(UiAction::SelectRun(RunId(1)))
    .unwrap()
}
fn start_search(
    state: &mut DesktopState,
    jobs: &mut JobRunner,
    column: ColumnId,
    query: &str,
    offset: usize,
) {
    state
        .set_frequency_search(column, query.into(), offset)
        .unwrap();
    // poll dispatches the latest queued search on the single existing worker.
    state.poll(jobs);
}
#[test]
fn exploration_search_replacement_aba_table_run_page_and_document_reject_late_rows() {
    let mut state = frequency_state();
    let mut jobs = JobRunner::default();
    start_search(&mut state, &mut jobs, ColumnId(1), "Category", 0);
    let oldtoken = state.token();
    let oldgeneration = state.frequency_query_generation();
    let oldrequest = state.frequency_request().unwrap().clone();
    let late = state
        .project()
        .frequency_rows_with_budget(&oldrequest, &JobControl::default(), &[], 4096)
        .unwrap();
    state
        .set_frequency_search(ColumnId(1), "Other".into(), 0)
        .unwrap();
    state
        .set_frequency_search(ColumnId(1), "Category".into(), 0)
        .unwrap();
    assert!(state.frequency_query_generation() > oldgeneration);
    state
        .handle_event(JobEvent::Completed {
            token: oldtoken,
            cancelled: false,
            result: Ok(JobOutput::FrequencyRows(late.clone().into())),
        })
        .unwrap();
    assert!(state.frequency_rows().is_none());
    wait(&mut state, &mut jobs);
    state.poll(&mut jobs);
    wait(&mut state, &mut jobs);
    assert_eq!(state.frequency_rows().unwrap().matched, 600);
    let a = state.frequency_rows().unwrap().clone();
    start_search(&mut state, &mut jobs, ColumnId(1), "Category", 256);
    wait(&mut state, &mut jobs);
    assert_eq!(state.frequency_rows().unwrap().indices[0], 256);
    state
        .handle_event(JobEvent::Completed {
            token: oldtoken,
            cancelled: false,
            result: Ok(JobOutput::FrequencyRows((*a).clone().into())),
        })
        .unwrap();
    assert_eq!(state.frequency_rows().unwrap().request.offset, 256);
    start_search(&mut state, &mut jobs, ColumnId(2), "Other", 0);
    wait(&mut state, &mut jobs);
    assert_eq!(state.frequency_rows().unwrap().request.column, ColumnId(2));
    state = state.apply_action(UiAction::SelectRun(RunId(2))).unwrap();
    assert!(state.frequency_rows().is_none());
    state
        .handle_event(JobEvent::Completed {
            token: state.token(),
            cancelled: false,
            result: Ok(JobOutput::FrequencyRows(late.into())),
        })
        .unwrap();
    assert!(state.frequency_rows().is_none());
    state = state
        .apply_action(UiAction::Request(PendingAction::New))
        .unwrap()
        .apply_action(UiAction::ResolveUnsaved(UnsavedChoice::Discard))
        .unwrap();
    finish_pending_frame(&mut state);
    assert!(state.frequency_request().is_none());
    assert!(state.error().is_none());
}

#[test]
fn exploration_search_queues_latest_behind_analysis_and_success_switch_invalidates() {
    for valid in [false, true] {
        let mut state = frequency_state();
        let mut jobs = JobRunner::default();
        let request = AnalysisRequest::Frequencies(FrequenciesRequest {
            columns: if valid { vec![ColumnId(1)] } else { vec![] },
            filter: RowFilter::default(),
            frequency_weight: None,
        });
        state
            .start_operation(
                &mut jobs,
                JobOperation::Analyze {
                    project: Arc::clone(state.project()),
                    request,
                },
            )
            .unwrap();
        let token = state.token();
        for query in ["Category 0", "Category 5"] {
            state
                .set_frequency_search(ColumnId(1), query.into(), 0)
                .unwrap();
        }
        assert_eq!(state.token(), token);
        assert!(jobs.is_running());
        state.poll(&mut jobs);
        wait(&mut state, &mut jobs);
        state.poll(&mut jobs);
        wait(&mut state, &mut jobs);
        if valid {
            assert_eq!(state.project().runs().len(), 3);
            assert!(state.frequency_request().is_none());
            assert!(state.frequency_rows().is_none());
        } else {
            assert_eq!(state.project().runs().len(), 2);
            assert_eq!(state.frequency_rows().unwrap().request.query, "Category 5");
            assert_eq!(state.frequency_rows().unwrap().matched, 100);
        }
    }
}
#[test]
fn exploration_rendered_search_edit_and_page_use_real_controls() {
    let mut state = frequency_state()
        .apply_action(UiAction::Navigate(Workspace::Results))
        .unwrap();
    let ctx = egui::Context::default();
    tessera::desktop::configure_context(&ctx);
    ctx.set_theme(egui::Theme::Light);
    let mut jobs = JobRunner::default();
    let size = Vec2::new(1180., 900.);
    frame(&ctx, &mut state, &mut jobs, size, vec![]);
    wait(&mut state, &mut jobs);
    frame(&ctx, &mut state, &mut jobs, size, vec![]);
    wait(&mut state, &mut jobs);
    frame(&ctx, &mut state, &mut jobs, size, vec![]);
    assert_eq!(state.frequency_rows().unwrap().matched, 600);
    click(&ctx, &mut state, &mut jobs, "Next categories", size);
    wait(&mut state, &mut jobs);
    frame(&ctx, &mut state, &mut jobs, size, vec![]);
    wait(&mut state, &mut jobs);
    assert_eq!(state.frequency_rows().unwrap().request.offset, 256);
    let id = state.control_id("Search categories").unwrap();
    ctx.memory_mut(|m| m.request_focus(id));
    frame(
        &ctx,
        &mut state,
        &mut jobs,
        size,
        vec![
            key(Key::A, Modifiers::COMMAND),
            Event::Text("Category 599".into()),
        ],
    );
    wait(&mut state, &mut jobs);
    frame(&ctx, &mut state, &mut jobs, size, vec![]);
    wait(&mut state, &mut jobs);
    let result = state.frequency_rows().unwrap();
    assert_eq!(result.indices, vec![599]);
    assert_eq!(result.request.offset, 0);
}

fn result_state(data: Dataset, request: AnalysisRequest) -> DesktopState {
    let result = analyze_with_budget(&data, &request, &JobControl::default(), &[], 4096).unwrap();
    DesktopState::from_project(
        Project::new("Drawing", data)
            .unwrap()
            .with_run(result)
            .unwrap(),
    )
    .unwrap()
    .apply_action(UiAction::SelectRun(RunId(1)))
    .unwrap()
    .apply_action(UiAction::Navigate(Workspace::Results))
    .unwrap()
}
fn draw(
    ctx: &egui::Context,
    state: &mut DesktopState,
    jobs: &mut JobRunner,
    size: Vec2,
    events: Vec<Event>,
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
    let opened_id = egui::Id::new("test-detailed-values-opened");
    if state.control_rect("Detailed values").is_some()
        && !ctx.data(|d| d.get_temp::<bool>(opened_id).unwrap_or(false))
    {
        // Exercise the real disclosure control before inspecting virtual exact rows.
        ctx.data_mut(|d| d.insert_temp(opened_id, true));
        click(ctx, state, jobs, "Detailed values", size);
        for _ in 0..3 {
            frame(ctx, state, jobs, size, vec![]);
        }
        return draw(ctx, state, jobs, size, vec![]);
    }
    output
}
fn box_state(values: Vec<Option<f64>>) -> DesktopState {
    result_state(
        Dataset::new(vec![
            Column::numeric(ColumnId(1), "Outcome", values).unwrap()
        ])
        .unwrap(),
        AnalysisRequest::Boxplot(BoxplotRequest {
            outcome: ColumnId(1),
            group: None,
            filter: RowFilter::default(),
            frequency_weight: None,
        }),
    )
}
#[test]
fn exploration_boxplot_draws_q3_and_inside_observed_whisker_at_separate_exact_positions() {
    let mut state = box_state(vec![Some(0.), Some(1.), Some(2.), Some(100.)]);
    let CompletedAnalysis::Boxplot(v) = &state.selected_run().unwrap().analysis else {
        panic!()
    };
    assert_eq!(v.groups[0].q3.value, Some(26.5));
    assert_eq!(v.groups[0].whiskers[1].value, Some(2.));
    let ctx = egui::Context::default();
    tessera::desktop::configure_context(&ctx);
    ctx.set_theme(egui::Theme::Light);
    let mut jobs = JobRunner::default();
    let output = draw(&ctx, &mut state, &mut jobs, Vec2::new(1180., 1100.), vec![]);
    let outer = state.control_rect("Chart").unwrap();
    let plot = Rect::from_min_max(
        outer.min + egui::vec2(68., 12.),
        outer.max - egui::vec2(30., 54.),
    );
    let wx = plot.left() + 0.02 * plot.width();
    let qx = plot.left() + 0.265 * plot.width();
    let lane = plot.center().y + 30.;
    let line = |a: Pos2, b: Pos2| {
        output.shapes.iter().any(|s|matches!(&s.shape,egui::Shape::LineSegment {points,..} if points[0].distance(a)<0.02 && points[1].distance(b)<0.02))
    };
    assert!(
        line(Pos2::new(qx, lane), Pos2::new(wx, lane)),
        "upper whisker connects to actual 2 without clamping to Q3"
    );
    assert!(
        line(Pos2::new(wx, lane - 7.), Pos2::new(wx, lane + 7.)),
        "observed whisker cap retains actual position"
    );
    assert!(
        output
            .shapes
            .iter()
            .any(|s| matches!(&s.shape,egui::Shape::Rect(r) if (r.rect.right()-qx).abs()<0.02)),
        "quartile box retains Q3"
    );
}
#[test]
fn exploration_all_outlier_subnormals_keep_quartiles_extrema_and_both_unavailable_whiskers() {
    let tiny = f64::from_bits(1);
    let mut state = box_state(vec![Some(-tiny), Some(tiny)]);
    let CompletedAnalysis::Boxplot(v) = &state.selected_run().unwrap().analysis else {
        panic!()
    };
    let g = &v.groups[0];
    assert_eq!(g.n, 2);
    assert_eq!(g.outlier_n, 2);
    assert_eq!(v.points.len(), 2);
    assert_eq!(g.minimum.value, Some(-tiny));
    assert_eq!(g.maximum.value, Some(tiny));
    for e in [&g.q1, &g.median, &g.q3] {
        assert!(e.value.unwrap().is_finite());
    }
    for e in &g.whiskers {
        assert!(e.value.is_none());
        assert!(e.reason.is_some());
    }
    let ctx = egui::Context::default();
    tessera::desktop::configure_context(&ctx);
    ctx.set_theme(egui::Theme::Light);
    ctx.enable_accesskit();
    let mut jobs = JobRunner::default();
    let output = draw(&ctx, &mut state, &mut jobs, Vec2::new(1180., 1100.), vec![]);
    assert_eq!(
        output
            .shapes
            .iter()
            .filter(|s| matches!(&s.shape,egui::Shape::Circle(c) if c.radius==4.))
            .count(),
        2
    );
    let labels = super::review::access_labels(&ctx, &mut state, &mut jobs);
    assert!(
        labels.iter().any(|s| s.contains("Lower observed whisker:")
            && s.contains("No observed values lie within the quartile fences.")),
        "{labels:?}"
    );
    assert!(
        labels.iter().any(|s| s.contains("Upper observed whisker:")
            && s.contains("No observed values lie within the quartile fences.")),
        "{labels:?}"
    );
}
#[test]
fn exploration_histogram_empty_constant_extreme_adjacent_ranges_remain_exact() {
    for values in [
        vec![None, None],
        vec![Some(2.), Some(2.)],
        vec![Some(-f64::MAX), Some(f64::MAX)],
        vec![Some(1.), Some(f64::from_bits(1f64.to_bits() + 1))],
        vec![Some(f64::from_bits(f64::MAX.to_bits() - 1)), Some(f64::MAX)],
        vec![Some(f64::from_bits(1)), Some(f64::from_bits(2))],
        vec![Some(f64::from_bits(1)), Some(f64::from_bits(1))],
    ] {
        let mut state = result_state(
            Dataset::new(vec![
                Column::numeric(ColumnId(1), "Range", values.clone()).unwrap()
            ])
            .unwrap(),
            AnalysisRequest::Histogram(HistogramRequest {
                variable: ColumnId(1),
                bins: HistogramBins::Count(128),
                filter: RowFilter::default(),
                frequency_weight: None,
            }),
        );
        let before = state.project().clone();
        let ctx = egui::Context::default();
        tessera::desktop::configure_context(&ctx);
        ctx.set_theme(egui::Theme::Light);
        ctx.set_pixels_per_point(2.);
        let mut jobs = JobRunner::default();
        let output = draw(&ctx, &mut state, &mut jobs, Vec2::new(760., 1100.), vec![]);
        assert!(Arc::ptr_eq(&before, state.project()));
        assert!(!output.shapes.iter().any(|s|matches!(&s.shape,egui::Shape::Text(t) if t.galley.text().contains("Drawing unavailable"))));
        if values[1] == Some(f64::MAX) && values[0].is_some_and(|v| v > 0.) {
            assert!(output.shapes.iter().any(|s|matches!(&s.shape,egui::Shape::Text(t) if t.galley.text()=="1.79769313486231571e308")),"adjacent MAX tick remains distinct");
        }
        if values[0] == Some(1.) && values[1] != values[0] {
            assert!(output.shapes.iter().any(|s|matches!(&s.shape,egui::Shape::Text(t) if t.galley.text()=="1.00000000000000022e0")),"adjacent endpoints retain distinct tick captions");
        }
        if values[0].is_none() {
            assert!(output.shapes.iter().any(|s|matches!(&s.shape,egui::Shape::Text(t) if t.galley.text()=="No eligible observations")));
        }
        let CompletedAnalysis::Histogram(v) = &state.selected_run().unwrap().analysis else {
            panic!()
        };
        assert_eq!(v.range[0].value, values[0]);
        assert_eq!(v.range[1].value, values[1]);
    }
}

#[test]
fn exploration_scatter_reduction_axes_zoom_and_keyboard_reach_exact_row_context() {
    let count = 2500;
    let data = Dataset::new(vec![
        Column::numeric(
            ColumnId(1),
            "X axis",
            (0..count)
                .map(|i| Some(if i == 1001 { 1e100 } else { i as f64 }))
                .collect(),
        )
        .unwrap(),
        Column::numeric(
            ColumnId(2),
            "Y axis",
            (0..count).map(|i| Some(i as f64)).collect(),
        )
        .unwrap(),
    ])
    .unwrap();
    let mut state = result_state(
        data,
        AnalysisRequest::Scatter(ScatterRequest {
            x: ColumnId(1),
            y: ColumnId(2),
            filter: RowFilter::default(),
            frequency_weight: None,
        }),
    );
    let CompletedAnalysis::Scatter(v) = &state.selected_run().unwrap().analysis else {
        panic!()
    };
    assert!(v.reduced);
    assert_eq!(v.ranges[0][1].value, Some(1e100));
    let sample = v.points[0].clone();
    let ctx = egui::Context::default();
    tessera::desktop::configure_context(&ctx);
    ctx.set_theme(egui::Theme::Light);
    ctx.enable_accesskit();
    ctx.set_pixels_per_point(2.);
    let mut jobs = JobRunner::default();
    let size = Vec2::new(1180., 1100.);
    let output = draw(&ctx, &mut state, &mut jobs, size, vec![]);
    assert!(output
        .shapes
        .iter()
        .any(|s| matches!(&s.shape,egui::Shape::Text(t) if t.galley.text()=="X axis")));
    assert!(output
        .shapes
        .iter()
        .any(|s| matches!(&s.shape,egui::Shape::Text(t) if t.galley.text()=="Y axis")));
    let update = output.platform_output.accesskit_update.unwrap();
    let target = format!(
        "X axis / Y axis · Source row: {} · X: {} · Y: {} · Frequency: {}",
        sample.row + 1,
        sample.x,
        sample.y,
        sample.frequency
    );
    let row = update
        .nodes
        .iter()
        .find(|(_, node)| node.label().or_else(|| node.value()) == Some(target.as_str()))
        .expect("exact keyboard row context")
        .0;
    let mut reached = false;
    for _ in 0..120 {
        let output = draw(
            &ctx,
            &mut state,
            &mut jobs,
            size,
            vec![key(Key::Tab, Modifiers::NONE)],
        );
        if let Some(update) = output.platform_output.accesskit_update {
            if update.focus == row
                || update
                    .nodes
                    .iter()
                    .find(|(id, _)| *id == update.focus)
                    .is_some_and(|(_, node)| {
                        node.label().or_else(|| node.value()) == Some(target.as_str())
                    })
            {
                reached = true;
                break;
            }
        }
    }
    assert!(reached, "Tab reaches the exact observation table");
    state = state
        .apply_action(UiAction::Navigate(Workspace::Analyses))
        .unwrap();
    frame(&ctx, &mut state, &mut jobs, Vec2::new(760., 520.), vec![]);
    assert!(state.dialog().is_none());
    assert!(state.control_rect("Search procedures").is_some());
    assert!(state.control_rect("Settings").is_none());
}
#[test]
fn exploration_visible_captured_categories_keep_labels_after_current_metadata_edit() {
    let data = Dataset::new(vec![Column::text(
        ColumnId(1),
        "Category",
        vec![Some("".into()), Some(" A ".into())],
    )
    .unwrap()])
    .unwrap();
    let mut metadata = data.column(ColumnId(1)).unwrap().metadata().clone();
    metadata.value_labels = vec![
        tessera::data::ValueLabel {
            value: Cell::Text("".into()),
            label: "Same".into(),
        },
        tessera::data::ValueLabel {
            value: Cell::Text(" A ".into()),
            label: "Same".into(),
        },
    ];
    let data = data
        .with_column_metadata_with_budget(ColumnId(1), metadata.clone(), &[], 4096)
        .unwrap();
    let mut state = result_state(
        data,
        AnalysisRequest::Frequencies(FrequenciesRequest {
            columns: vec![ColumnId(1)],
            filter: RowFilter::default(),
            frequency_weight: None,
        }),
    );
    metadata.name = "Edited".into();
    metadata.value_labels[0].label = "Changed".into();
    state = state
        .apply_action(UiAction::Metadata {
            column: ColumnId(1),
            metadata,
        })
        .unwrap();
    let ctx = egui::Context::default();
    tessera::desktop::configure_context(&ctx);
    ctx.set_theme(egui::Theme::Light);
    ctx.enable_accesskit();
    let mut jobs = JobRunner::default();
    let size = Vec2::new(1180., 900.);
    for _ in 0..3 {
        frame(&ctx, &mut state, &mut jobs, size, vec![]);
        wait(&mut state, &mut jobs);
    }
    let labels = super::review::access_labels(&ctx, &mut state, &mut jobs);
    assert!(
        labels.iter().any(|s| s.contains("Category")
            && s.contains("\"\" · Same")
            && s.contains("Frequency: 1")),
        "{labels:?}"
    );
    assert!(
        labels
            .iter()
            .any(|s| s.contains("\" A \" · Same") && s.contains("Valid percent: 50")),
        "{labels:?}"
    );
    assert!(!labels.iter().any(|s| s.contains("Changed")));
}

#[path = "exploration_scroll.rs"]
mod scrolling;
