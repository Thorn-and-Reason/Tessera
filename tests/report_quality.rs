//! A complete survey-output path uses captured values for every publication.
use tessera::{
    analysis::{
        analyze_with_budget, AnalysisRequest, BoxplotRequest, CompletedAnalysis,
        FrequenciesRequest, RowFilter,
    },
    data::{Cell, Column, ColumnId, Dataset},
    export::{
        export_report,
        figures::{export_figure, FigureFormat},
    },
    figures::{FigureKey, FigureKind},
    operation::{JobControl, RunId},
    preparation::{prepare_with_budget, Step},
    project::Project,
    project_io,
};
#[test]
fn complete_survey_scores_grouped_distributions_and_report_survive_reopen() {
    let control = JobControl::default();
    let data = Dataset::new(vec![
        Column::numeric(
            ColumnId(1),
            "Interest <&>",
            vec![Some(1.), Some(3.), Some(5.), None],
        )
        .unwrap(),
        Column::numeric(
            ColumnId(2),
            "Participation",
            vec![Some(3.), Some(5.), Some(1.), Some(5.)],
        )
        .unwrap(),
        Column::text(
            ColumnId(3),
            "School",
            vec![
                Some("A <&>".into()),
                Some("A <&>".into()),
                Some("B".into()),
                Some("B".into()),
            ],
        )
        .unwrap(),
    ])
    .unwrap();
    let mut project = Project::new("Survey <&>", data).unwrap();
    let result = analyze_with_budget(
        project.dataset(),
        &AnalysisRequest::Frequencies(FrequenciesRequest {
            columns: vec![ColumnId(1), ColumnId(2)],
            group: Some(ColumnId(3)),
            filter: RowFilter::default(),
            frequency_weight: None,
        }),
        &control,
        &[],
        0,
    )
    .unwrap();
    let CompletedAnalysis::Frequencies(ref counts) = result else {
        panic!()
    };
    assert_eq!(counts.tables[0].valid_n, 3);
    assert_eq!(counts.groups.iter().map(|g| g.rows).sum::<usize>(), 4);
    assert_eq!(counts.groups[1].tables[0].missing_rows, 1);
    project = project.with_run(result).unwrap();
    let likert = FigureKey {
        run: 1,
        kind: FigureKind::Likert,
        variant: 0,
    };
    let mut style = project.figure_settings(&likert);
    style.likert_points = Some(5);
    project = project.with_figure(&likert, style).unwrap();
    project = prepare_with_budget(
        &project,
        &Step::Mean {
            name: "Engagement average".into(),
            columns: vec![ColumnId(1), ColumnId(2)],
        },
        &control,
        &[],
        0,
    )
    .unwrap();
    assert_eq!(
        project.dataset().cell(ColumnId(4), 0).unwrap(),
        Cell::Number(2.)
    );
    assert_eq!(
        project.dataset().cell(ColumnId(4), 3).unwrap(),
        Cell::Missing
    );
    let result = analyze_with_budget(
        project.dataset(),
        &AnalysisRequest::Boxplot(BoxplotRequest {
            outcome: ColumnId(4),
            group: Some(ColumnId(3)),
            filter: RowFilter::default(),
            frequency_weight: None,
        }),
        &control,
        &[],
        0,
    )
    .unwrap();
    let CompletedAnalysis::Boxplot(ref summaries) = result else {
        panic!()
    };
    assert_eq!(summaries.groups[0].mean.value, Some(3.));
    assert_eq!(summaries.groups[1].mean.value, Some(3.));
    project = project.with_run(result).unwrap();
    let means = FigureKey {
        run: 2,
        kind: FigureKind::GroupMeans,
        variant: 0,
    };
    let mut style = project.figure_settings(&means);
    style.x_bounds = Some([1., 5.]);
    project = project.with_figure(&means, style).unwrap();
    let mut bytes = Vec::new();
    project_io::encode(&project, &mut bytes, &control).unwrap();
    let reopened = project_io::decode(bytes.as_slice(), &control).unwrap();
    assert_eq!(reopened.runs()[0].analysis, project.runs()[0].analysis);
    assert_eq!(reopened.runs()[1].analysis, project.runs()[1].analysis);
    let folder = std::env::temp_dir()
        .canonicalize()
        .unwrap()
        .join(format!("tessera-report-quality-{}", std::process::id()));
    std::fs::create_dir_all(&folder).unwrap();
    for key in [likert, means] {
        let path = folder.join(format!("{}.png", key.run));
        export_figure(&path, &reopened, &key, FigureFormat::Png, &control, &[], 0).unwrap();
        assert_eq!(&std::fs::read(path).unwrap()[..8], b"\x89PNG\r\n\x1a\n");
    }
    let path = folder.join("report.html");
    export_report(&path, &reopened, &[RunId(1), RunId(2)], &control).unwrap();
    let html = std::fs::read_to_string(path).unwrap();
    assert!(html.contains("A &lt;&amp;&gt;"));
    assert!(html.contains("Ordered response distributions"));
    assert!(html.contains("group means"));
    assert!(html.contains("Stale result"));
    assert!(!html.contains("Drawing unavailable"));
    assert!(html.contains("Exact captured value"));
    std::fs::remove_dir_all(folder).unwrap();
}
