//! Actual scroll, keyboard and pointer events over captured virtual rows.
use super::*;
fn labels(output: egui::FullOutput) -> Vec<String> {
    output
        .platform_output
        .accesskit_update
        .into_iter()
        .flat_map(|u| u.nodes)
        .filter_map(|(_, n)| n.label().or_else(|| n.value()).map(str::to_owned))
        .collect()
}
fn scroll(
    ctx: &egui::Context,
    state: &mut DesktopState,
    jobs: &mut JobRunner,
    size: Vec2,
    position: Pos2,
    delta: f32,
) -> Vec<String> {
    let events = vec![
        Event::PointerMoved(position),
        Event::MouseWheel {
            unit: egui::MouseWheelUnit::Point,
            phase: egui::TouchPhase::Move,
            delta: Vec2::new(0., delta),
            modifiers: Modifiers::NONE,
        },
    ];
    draw(ctx, state, jobs, size, events);
    for _ in 0..20 {
        draw(ctx, state, jobs, size, vec![]);
    }
    labels(draw(ctx, state, jobs, size, vec![]))
}
#[test]
fn exploration_scrolled_categories_and_points_retain_exact_context() {
    let size = Vec2::new(1180., 1100.);
    let mut state = frequency_state()
        .apply_action(UiAction::Navigate(Workspace::Results))
        .unwrap();
    let ctx = egui::Context::default();
    tessera::desktop::configure_context(&ctx);
    ctx.set_theme(egui::Theme::Light);
    ctx.enable_accesskit();
    let mut jobs = JobRunner::default();
    for _ in 0..3 {
        draw(&ctx, &mut state, &mut jobs, size, vec![]);
        wait(&mut state, &mut jobs);
    }
    let initial = draw(&ctx, &mut state, &mut jobs, size, vec![]);
    let first = initial
        .shapes
        .iter()
        .find_map(|s| match &s.shape {
            egui::Shape::Text(t)
                if t.galley.text().starts_with("Category · Category ")
                    && t.galley.text().contains(" · Rows:") =>
            {
                Some(t.pos)
            }
            _ => None,
        })
        .unwrap();
    let before = labels(initial);
    let after = scroll(
        &ctx,
        &mut state,
        &mut jobs,
        size,
        first + egui::vec2(8., 7.),
        -500.,
    );
    let row = after
        .iter()
        .find(|s| s.contains("Category") && s.contains("Frequency: 1") && !before.contains(s))
        .expect("category viewport changed");
    assert!(
        row.contains("Category") && row.contains("Percent:") && row.contains("Valid percent:"),
        "{row}"
    );
    let data = Dataset::new(vec![
        Column::numeric(ColumnId(1), "X", (0..600).map(|i| Some(i as f64)).collect()).unwrap(),
        Column::numeric(
            ColumnId(2),
            "Y",
            (0..600).map(|i| Some((i * 2) as f64)).collect(),
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
    let ctx = egui::Context::default();
    tessera::desktop::configure_context(&ctx);
    ctx.set_theme(egui::Theme::Light);
    ctx.enable_accesskit();
    let before = labels(draw(&ctx, &mut state, &mut jobs, size, vec![]));
    let first = draw(&ctx, &mut state, &mut jobs, size, vec![])
        .shapes
        .into_iter()
        .find_map(|s| match s.shape {
            egui::Shape::Text(t) if t.galley.text().starts_with("X / Y · Source row:") => {
                Some(t.pos)
            }
            _ => None,
        })
        .unwrap();
    let after = scroll(
        &ctx,
        &mut state,
        &mut jobs,
        size,
        first + egui::vec2(20., 20.),
        -600.,
    );

    let row = after
        .iter()
        .find(|s| s.starts_with("X / Y · Source row:") && !before.contains(s))
        .expect("point viewport changed");
    assert!(
        row.contains(" · X:") && row.contains(" · Y:") && row.contains(" · Frequency: 1"),
        "{row}"
    );
}
#[test]
fn exploration_scrolled_groups_keep_group_variable_quartiles_and_counts() {
    let n = 40;
    let data = Dataset::new(vec![
        Column::numeric(
            ColumnId(1),
            "Outcome",
            (0..n).map(|i| Some(i as f64)).collect(),
        )
        .unwrap(),
        Column::text(
            ColumnId(2),
            "Class",
            (0..n).map(|i| Some(format!("Group {i:02}"))).collect(),
        )
        .unwrap(),
    ])
    .unwrap();
    let mut state = result_state(
        data,
        AnalysisRequest::Boxplot(BoxplotRequest {
            outcome: ColumnId(1),
            group: Some(ColumnId(2)),
            filter: RowFilter::default(),
            frequency_weight: None,
        }),
    );
    let ctx = egui::Context::default();
    tessera::desktop::configure_context(&ctx);
    ctx.set_theme(egui::Theme::Light);
    ctx.enable_accesskit();
    let mut jobs = JobRunner::default();
    let size = Vec2::new(1180., 1100.);
    let before = labels(draw(&ctx, &mut state, &mut jobs, size, vec![]));
    let after = scroll(
        &ctx,
        &mut state,
        &mut jobs,
        size,
        Pos2::new(600., 450.),
        -1500.,
    );
    let row = after
        .iter()
        .find(|s| s.contains(" · Class · ") && s.contains(" · Rows:") && !before.contains(s))
        .expect("group viewport changed");
    assert!(
        row.contains("Outcome · Group")
            && row.contains("Frequency: 1")
            && row.contains("Outlier rows: 0"),
        "{row}"
    );
    assert!(after
        .iter()
        .any(|s| s.contains(" · Class · ") && s.contains(" · Q3:")));
}
#[test]
fn exploration_scatter_hover_retains_exact_source_pair_and_frequency() {
    let mut state = explore_state();
    let result = analyze_with_budget(
        state.project().dataset(),
        &AnalysisRequest::Scatter(ScatterRequest {
            x: ColumnId(1),
            y: ColumnId(2),
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
            result: Ok(JobOutput::Analyzed(result)),
        })
        .unwrap();
    state = state
        .apply_action(UiAction::Navigate(Workspace::Results))
        .unwrap();
    let ctx = egui::Context::default();
    tessera::desktop::configure_context(&ctx);
    ctx.set_theme(egui::Theme::Light);
    let mut jobs = JobRunner::default();
    let size = Vec2::new(1180., 1100.);
    draw(&ctx, &mut state, &mut jobs, size, vec![]);
    let outer = state.control_rect("Chart").unwrap();
    let point = Pos2::new(outer.left() + 68., outer.bottom() - 52.);
    let output = draw(
        &ctx,
        &mut state,
        &mut jobs,
        size,
        vec![Event::PointerMoved(point)],
    );
    assert!(output.shapes.iter().any(|s|matches!(&s.shape,egui::Shape::Text(t) if t.galley.text()=="X / Y · Source row: 1 · X: 0 · Y: 10 · Frequency: 1")),"pointer hover keeps exact captured point");
}

#[test]
fn exploration_precise_native_ticks_stay_inside_compact_canvas_without_overlapping() {
    let lo = f64::from_bits(f64::MAX.to_bits() - 1);
    let data = Dataset::new(vec![
        Column::numeric(ColumnId(1), "X", vec![Some(lo), Some(f64::MAX)]).unwrap(),
        Column::numeric(ColumnId(2), "Y", vec![Some(lo), Some(f64::MAX)]).unwrap(),
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
    let ctx = egui::Context::default();
    tessera::desktop::configure_context(&ctx);
    ctx.set_theme(egui::Theme::Light);
    ctx.set_pixels_per_point(2.);
    let mut jobs = JobRunner::default();
    let output = draw(&ctx, &mut state, &mut jobs, Vec2::new(760., 1100.), vec![]);
    let canvas = state.control_rect("Chart").unwrap();
    let ticks = output
        .shapes
        .iter()
        .filter_map(|s| match &s.shape {
            egui::Shape::Text(t) if t.galley.text().starts_with("1.797693134862315") => {
                Some(Rect::from_min_size(t.pos, t.galley.size()))
            }
            _ => None,
        })
        .collect::<Vec<_>>();
    assert_eq!(ticks.len(), 4);
    assert!(
        ticks.iter().all(|r| canvas.contains_rect(*r)),
        "{canvas:?}: {ticks:?}"
    );
    for i in 0..ticks.len() {
        for j in i + 1..ticks.len() {
            assert!(
                !ticks[i].intersects(ticks[j]),
                "precise ticks overlap: {:?} {:?}",
                ticks[i],
                ticks[j]
            );
        }
    }
}

#[test]
fn review_fix_long_categories_keep_absolute_captured_identity_and_full_report_route() {
    let prefix = "A".repeat(280);
    let x = format!("{prefix}X");
    let y = format!("{prefix}Y");
    let mut values = (0..260)
        .map(|i| Some(format!("{i:03}")))
        .collect::<Vec<_>>();
    values.extend([Some(x.clone()), Some(y.clone())]);
    let data = Dataset::new(vec![
        Column::text(ColumnId(1), "Categories", values).unwrap()
    ])
    .unwrap();
    let mut state = result_state(
        data,
        AnalysisRequest::Frequencies(FrequenciesRequest {
            columns: vec![ColumnId(1)],
            filter: RowFilter::default(),
            frequency_weight: None,
        }),
    );
    let ctx = egui::Context::default();
    tessera::desktop::configure_context(&ctx);
    ctx.set_theme(egui::Theme::Light);
    ctx.enable_accesskit();
    ctx.style_mut_of(egui::Theme::Light, |s| s.interaction.tooltip_delay = 0.);
    let mut jobs = JobRunner::default();
    let size = Vec2::new(1180., 1100.);
    start_search(&mut state, &mut jobs, ColumnId(1), &prefix, 0);
    wait(&mut state, &mut jobs);
    let output = draw(&ctx, &mut state, &mut jobs, size, vec![]);
    let rows = labels(output)
        .into_iter()
        .filter(|s| s.starts_with("Categories ·") && s.contains(" · Rows: 1"))
        .collect::<Vec<_>>();
    assert_eq!(rows.len(), 2);
    assert_ne!(
        rows[0], rows[1],
        "bounded collision must remain distinguishable"
    );
    assert!(rows
        .iter()
        .any(|s| s.starts_with("Categories · Category 261 ·")));
    assert!(rows
        .iter()
        .any(|s| s.starts_with("Categories · Category 262 ·")));
    let output = draw(&ctx, &mut state, &mut jobs, size, vec![]);
    let text = output
        .shapes
        .iter()
        .find_map(|s| match &s.shape {
            egui::Shape::Text(t) if t.galley.text().starts_with("Categories · Category 261 ·") => {
                Some(t.pos)
            }
            _ => None,
        })
        .unwrap();
    for _ in 0..3 {
        draw(
            &ctx,
            &mut state,
            &mut jobs,
            size,
            vec![Event::PointerMoved(text + egui::vec2(8., 7.))],
        );
    }
    let output = draw(&ctx, &mut state, &mut jobs, size, vec![]);
    assert!(output.shapes.iter().any(|s|matches!(&s.shape,egui::Shape::Text(t) if t.galley.text().starts_with("Categories · Category 261 ·")&&t.galley.size().y>20.)),"hover context includes captured ordinal");
    state = state
        .apply_action(UiAction::EditCell {
            column: ColumnId(1),
            row: 260,
            value: Cell::Text("Changed current value".into()),
        })
        .unwrap();
    start_search(&mut state, &mut jobs, ColumnId(1), &prefix, 1);
    wait(&mut state, &mut jobs);
    assert_eq!(state.frequency_rows().unwrap().indices, vec![261]);
    let output = draw(&ctx, &mut state, &mut jobs, size, vec![]);
    assert!(output.shapes.iter().any(|s|matches!(&s.shape,egui::Shape::Text(t) if t.galley.text()=="Complete category values: Export report…")));
    let names = labels(output);
    assert!(
        names
            .iter()
            .any(|s| s.starts_with("Categories · Category 262 ·")),
        "subset offset must not become source identity"
    );
    let path = std::env::temp_dir().canonicalize().unwrap().join(format!(
        "tessera-review-category-{}.html",
        std::process::id()
    ));
    let _ = std::fs::remove_file(&path);
    tessera::export::export_report(&path, state.project(), &[RunId(1)], &JobControl::default())
        .unwrap();
    let html = std::fs::read_to_string(&path).unwrap();
    std::fs::remove_file(path).unwrap();
    assert!(html.contains(&x) && html.contains(&y));
    assert!(!html.contains("Changed current value"));
}

#[test]
fn review_fix_long_axis_names_fit_named_canvas_and_preserve_numeric_coordinates() {
    for family in [Procedure::Histogram, Procedure::Scatter, Procedure::Boxplot] {
        for precise in [false, true] {
            let xn = "X".repeat(60);
            let yn = "W".repeat(28);
            let bounds = if precise {
                [f64::from_bits(f64::MAX.to_bits() - 1), f64::MAX]
            } else {
                [0., 1.]
            };
            let data = Dataset::new(vec![
                Column::numeric(ColumnId(1), &xn, bounds.into_iter().map(Some).collect()).unwrap(),
                Column::numeric(ColumnId(2), &yn, bounds.into_iter().map(Some).collect()).unwrap(),
            ])
            .unwrap();
            let request = match family {
                Procedure::Histogram => AnalysisRequest::Histogram(HistogramRequest {
                    variable: ColumnId(1),
                    bins: HistogramBins::Count(2),
                    filter: RowFilter::default(),
                    frequency_weight: None,
                }),
                Procedure::Scatter => AnalysisRequest::Scatter(ScatterRequest {
                    x: ColumnId(1),
                    y: ColumnId(2),
                    filter: RowFilter::default(),
                    frequency_weight: None,
                }),
                _ => AnalysisRequest::Boxplot(BoxplotRequest {
                    outcome: ColumnId(1),
                    group: None,
                    filter: RowFilter::default(),
                    frequency_weight: None,
                }),
            };
            let mut state = result_state(data, request);
            let ctx = egui::Context::default();
            tessera::desktop::configure_context(&ctx);
            ctx.set_theme(egui::Theme::Light);
            ctx.set_pixels_per_point(2.);
            let mut jobs = JobRunner::default();
            let output = draw(&ctx, &mut state, &mut jobs, Vec2::new(760., 1100.), vec![]);
            let canvas = state.control_rect("Chart").unwrap();
            let mut found = 0;
            for s in &output.shapes {
                if let egui::Shape::Text(t) = &s.shape {
                    if t.galley.text() == xn || t.galley.text() == yn {
                        let rect = Rect::from_min_size(t.pos, t.galley.size());
                        assert!(
                            canvas.contains_rect(rect),
                            "{family:?} precise={precise}: {rect:?} outside {canvas:?}"
                        );
                        assert!(s.clip_rect.contains_rect(rect), "name outside actual clip");
                        found += 1;
                    }
                }
            }
            assert_eq!(found, if family == Procedure::Scatter { 2 } else { 1 });
            if family == Procedure::Scatter {
                assert!(output.shapes.iter().any(|s|matches!(&s.shape,egui::Shape::Circle(c) if (c.center.y-(canvas.top()+178.)).abs()<0.01)));
                assert!(output.shapes.iter().any(|s|matches!(&s.shape,egui::Shape::Circle(c) if (c.center.y-(canvas.top()+12.)).abs()<0.01)));
            }
        }
    }
}
