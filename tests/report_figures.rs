use tessera::{
    analysis::{analyze_with_budget, AnalysisRequest, HistogramBins, HistogramRequest, RowFilter},
    data::{Column, ColumnId, Dataset},
    export::export_report,
    figures::{figure_keys, render, FigureSettings},
    operation::{JobControl, RunId},
    project::Project,
};
#[test]
fn report_uses_saved_and_duplicate_figures_without_changing_exact_tables() {
    let data = Dataset::new(vec![Column::numeric(
        ColumnId(1),
        "Score",
        vec![Some(1.), Some(2.), Some(3.)],
    )
    .unwrap()])
    .unwrap();
    let c = JobControl::default();
    let result = analyze_with_budget(
        &data,
        &AnalysisRequest::Histogram(HistogramRequest {
            variable: ColumnId(1),
            bins: HistogramBins::Count(2),
            filter: RowFilter::default(),
            frequency_weight: None,
        }),
        &c,
        &[],
        4096,
    )
    .unwrap();
    let project = Project::new("Report", data)
        .unwrap()
        .with_run(result)
        .unwrap();
    let key = figure_keys(&project.runs()[0])[0];
    let style = FigureSettings {
        title: "Edited <&> graph".into(),
        x_label: "Edited axis".into(),
        ..Default::default()
    };
    let project = project
        .with_figure(&key, style)
        .unwrap()
        .duplicate_figure(&key)
        .unwrap();
    let path = std::env::temp_dir()
        .canonicalize()
        .unwrap()
        .join(format!("report-figures-{}.html", std::process::id()));
    let _ = std::fs::remove_file(&path);
    export_report(&path, &project, &[RunId(1)], &c).unwrap();
    let html = std::fs::read_to_string(&path).unwrap();
    std::fs::remove_file(path).unwrap();
    assert_eq!(html.matches("<svg ").count(), 2);
    assert!(html.contains("Complete histogram bins"));
    assert!(html.contains("Edited axis"));
    assert!(!html.contains("Edited <&> graph"));
    for record in project.figures() {
        let svg =
            render::svg(&project.runs()[0], &record.key, &record.settings, false, &c).unwrap();
        assert!(html.contains(&svg));
    }
}

#[test]
fn empty_histogram_keeps_exact_table_and_explains_unavailable_drawing() {
    let data = Dataset::new(vec![
        Column::numeric(ColumnId(1), "Empty", vec![None]).unwrap()
    ])
    .unwrap();
    let c = JobControl::default();
    let result = analyze_with_budget(
        &data,
        &AnalysisRequest::Histogram(HistogramRequest {
            variable: ColumnId(1),
            bins: HistogramBins::Count(2),
            filter: RowFilter::default(),
            frequency_weight: None,
        }),
        &c,
        &[],
        4096,
    )
    .unwrap();
    let project = Project::new("Empty report", data)
        .unwrap()
        .with_run(result)
        .unwrap();
    let path = std::env::temp_dir()
        .canonicalize()
        .unwrap()
        .join(format!("report-empty-figures-{}.html", std::process::id()));
    let _ = std::fs::remove_file(&path);
    export_report(&path, &project, &[RunId(1)], &c).unwrap();
    let html = std::fs::read_to_string(&path).unwrap();
    std::fs::remove_file(path).unwrap();
    assert!(html.contains("Complete histogram bins"));
    assert!(html.contains("Drawing unavailable"));
    assert!(html.contains("Missing rows: 1"));
    assert!(!html.contains("<svg "));
}

#[test]
fn shared_report_drawing_reserves_memory_before_publication() {
    use tessera::{data::MAX_OWNED_BYTES, export::export_report_with_budget};
    let data = Dataset::new(vec![Column::numeric(
        ColumnId(1),
        "Score",
        vec![Some(1.), Some(2.)],
    )
    .unwrap()])
    .unwrap();
    let c = JobControl::default();
    let result = analyze_with_budget(
        &data,
        &AnalysisRequest::Histogram(HistogramRequest {
            variable: ColumnId(1),
            bins: HistogramBins::Count(2),
            filter: RowFilter::default(),
            frequency_weight: None,
        }),
        &c,
        &[],
        0,
    )
    .unwrap();
    let project = Project::new("Report", data)
        .unwrap()
        .with_run(result)
        .unwrap();
    let baseline = project.owned_bytes().unwrap();
    let path = std::env::temp_dir()
        .canonicalize()
        .unwrap()
        .join(format!("report-figure-budget-{}.html", std::process::id()));
    let result = export_report_with_budget(
        &path,
        &project,
        &[RunId(1)],
        &c,
        &[],
        MAX_OWNED_BYTES - baseline - 32 * 1024 * 1024 + 1024,
    );
    assert!(
        result.is_err(),
        "SVG geometry needs reserved working memory"
    );
    assert!(!path.exists());
}
