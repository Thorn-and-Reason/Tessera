use tessera::{
    analysis::*,
    data::{Column, ColumnId, Dataset},
    export::export_report,
    operation::{JobControl, RunId},
    project::Project,
};

fn project() -> Project {
    let data = Dataset::new(vec![
        Column::numeric(
            ColumnId(1),
            "Score <&>",
            vec![Some(1.), Some(2.), Some(3.), Some(4.)],
        )
        .unwrap(),
        Column::numeric(
            ColumnId(2),
            "Second score",
            vec![Some(2.), Some(4.), Some(6.), Some(8.)],
        )
        .unwrap(),
    ])
    .unwrap();
    let requests = [
        AnalysisRequest::Describe(DescribeRequest {
            columns: vec![ColumnId(1)],
            statistics: vec![Statistic::Mean],
            missing_policy: MissingPolicy::PerVariable,
            filter: RowFilter::default(),
            frequency_weight: None,
        }),
        AnalysisRequest::Frequencies(FrequenciesRequest {
            columns: vec![ColumnId(1)],
            filter: RowFilter::default(),
            frequency_weight: None,
        }),
        AnalysisRequest::Histogram(HistogramRequest {
            variable: ColumnId(1),
            bins: HistogramBins::Count(2),
            filter: RowFilter::default(),
            frequency_weight: None,
        }),
        AnalysisRequest::Scatter(ScatterRequest {
            x: ColumnId(1),
            y: ColumnId(2),
            filter: RowFilter::default(),
            frequency_weight: None,
        }),
        AnalysisRequest::Boxplot(BoxplotRequest {
            outcome: ColumnId(1),
            group: None,
            filter: RowFilter::default(),
            frequency_weight: None,
        }),
    ];
    requests.iter().fold(
        Project::new("Survey output", data).unwrap(),
        |p, request| {
            p.with_run(
                analyze_with_budget(p.dataset(), request, &JobControl::default(), &[], 4096)
                    .unwrap(),
            )
            .unwrap()
        },
    )
}

#[test]
fn survey_report_styles_all_output_without_changing_values() {
    let p = project();
    let runs = p.runs().to_vec();
    let path = std::env::temp_dir()
        .canonicalize()
        .unwrap()
        .join(format!("tessera-output-style-{}.html", std::process::id()));
    assert!(!path.exists());
    export_report(
        &path,
        &p,
        &[RunId(1), RunId(2), RunId(3), RunId(4), RunId(5)],
        &JobControl::default(),
    )
    .unwrap();
    let html = std::fs::read_to_string(&path).unwrap();
    std::fs::remove_file(path).unwrap();
    for rule in [
        "background:#F2EBDC",
        "#263449",
        "#C59B43",
        "#EEE6D6",
        "#D1C9B9",
        "#4E5A66",
        "tbody tr:nth-child(even)",
        "@media print",
        "print-color-adjust:exact",
    ] {
        assert!(html.contains(rule), "missing report style: {rule}");
    }
    for run in p.runs() {
        for key in tessera::figures::figure_keys(run) {
            let svg = tessera::figures::render::svg(
                run,
                &key,
                &p.figure_settings(&key),
                false,
                &JobControl::default(),
            )
            .unwrap();
            assert!(
                html.contains(&svg),
                "Report must use the same figure as standalone export"
            );
        }
    }
    assert!(html.contains("Tessera by Thorn &amp; Reason"));
    for legacy_colour in ["#023047", "#219EBC", "#8ECAE6", "#FFB703"] {
        assert!(
            !html.contains(legacy_colour),
            "legacy report colour: {legacy_colour}"
        );
    }
    assert!(html.contains("Score &lt;&amp;&gt;"));
    assert!(html.contains("<td>2.5</td>"));
    assert!(html.contains("scope=\"col\""));
    assert!(html.contains("role=\"img\""));
    assert!(html.contains("default-src 'none'"));
    for figure in html.split("<figure class=\"report-figure\">").skip(1) {
        let contents = figure.split("</figure>").next().unwrap();
        assert!(contents.contains("<svg") && contents.contains("<desc"));
        assert!(
            !contents.contains("<table>"),
            "Exact data tables must paginate independently of chart figures"
        );
    }
    assert_eq!(p.runs(), runs);
}

#[cfg(feature = "desktop")]
#[test]
fn native_output_styles_are_scoped_and_keep_stored_results() {
    use eframe::egui::{self, Color32, Pos2, Rect, Vec2};
    use std::sync::Arc;
    use tessera::{
        desktop::{configure_context, render, DesktopState, UiAction, Workspace},
        jobs::JobRunner,
    };
    let ctx = egui::Context::default();
    configure_context(&ctx);
    ctx.set_theme(egui::Theme::Light);
    let style = ctx.style_of(egui::Theme::Light);
    let mut state = DesktopState::from_project(project())
        .unwrap()
        .apply_action(UiAction::SelectRun(RunId(5)))
        .unwrap()
        .apply_action(UiAction::Navigate(Workspace::Results))
        .unwrap();
    let stored = Arc::clone(state.project());
    let mut jobs = JobRunner::default();
    let mut output = ctx.run_ui(
        egui::RawInput {
            screen_rect: Some(Rect::from_min_size(Pos2::ZERO, Vec2::new(1180., 1100.))),
            ..Default::default()
        },
        |ui| render(ui, &mut state, &mut jobs),
    );
    output.textures_delta.clear();
    assert!(output.shapes.iter().any(|s| matches!(&s.shape, egui::Shape::Rect(r) if r.fill == Color32::from_rgb(242, 235, 220))), "missing parchment quartile box");
    assert!(output.shapes.iter().any(|s| matches!(&s.shape, egui::Shape::LineSegment {stroke, ..} if stroke.color == Color32::from_rgb(197, 155, 67))), "missing gold median");
    assert!(output.shapes.iter().any(|s| matches!(&s.shape, egui::Shape::LineSegment {stroke, ..} if stroke.color == style.visuals.widgets.noninteractive.bg_stroke.color)), "missing themed grid");
    assert_eq!(ctx.style_of(egui::Theme::Light).visuals, style.visuals);
    assert_eq!(
        ctx.style_of(egui::Theme::Light).text_styles,
        style.text_styles
    );
    assert!(Arc::ptr_eq(state.project(), &stored));
}
