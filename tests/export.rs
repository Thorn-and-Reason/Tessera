use std::{
    fs,
    sync::atomic::{AtomicU64, Ordering},
};
use tessera::{
    data::{Column, ColumnId, Dataset},
    export::{export_data, export_report},
    files::Publication,
    import::Delimiter,
    operation::JobControl,
    project::Project,
};
static NEXT: AtomicU64 = AtomicU64::new(0);
#[test]
fn exports_escape_and_never_clobber() {
    let dir = std::env::temp_dir().canonicalize().unwrap().join(format!(
        "tessera-export-test-{}-{}",
        std::process::id(),
        NEXT.fetch_add(1, Ordering::Relaxed)
    ));
    fs::create_dir(&dir).unwrap();
    let data = Dataset::new(vec![
        Column::text(
            ColumnId(1),
            "a,\"name",
            vec![
                Some("<script>alert(1)</script>\n\"quoted\"".into()),
                Some(String::new()),
                None,
            ],
        )
        .unwrap(),
        Column::numeric(
            ColumnId(2),
            "Number",
            vec![Some(1.2345678901234567), None, Some(-0.0)],
        )
        .unwrap(),
    ])
    .unwrap();
    let c = JobControl::default();
    let csv = dir.join("data.csv");
    assert_eq!(
        export_data(&csv, &data, Delimiter::Comma, &c)
            .unwrap()
            .publication,
        Publication::Committed
    );
    let text = fs::read_to_string(&csv).unwrap();
    assert!(text.contains("\"a,\"\"name\""));
    assert!(text.contains("1.2345678901234567"));
    assert!(text.contains("\"\",\r\n"));
    assert!(export_data(&csv, &data, Delimiter::Comma, &c).is_err());
    assert_eq!(fs::read_to_string(&csv).unwrap(), text);
    let p = Project::new("<script>alert(1)</script>", data).unwrap();
    let html = dir.join("report.html");
    export_report(&html, &p, &[], &c).unwrap();
    let text = fs::read_to_string(&html).unwrap();
    assert!(!text.contains("<script>alert(1)</script>"));
    assert!(text.contains("&lt;script&gt;"));
    assert!(export_report(&html, &p, &[], &c).is_err());
    fs::remove_dir_all(dir).unwrap();
}

#[test]
fn report_retains_historical_settings_statistics_definitions_and_accessible_plot_data() {
    use tessera::{
        analysis::{describe, DescribeRequest, MissingPolicy, RowFilter, RowPredicate, Statistic},
        data::{Cell, MissingRules, ValueLabel},
        operation::RunId,
    };
    let dir = std::env::temp_dir().canonicalize().unwrap().join(format!(
        "tessera-report-test-{}-{}",
        std::process::id(),
        NEXT.fetch_add(1, Ordering::Relaxed)
    ));
    fs::create_dir(&dir).unwrap();
    let column = Column::numeric(
        ColumnId(1),
        "Score <&> \"'",
        vec![Some(1.0), Some(2.0), Some(3.0), None],
    )
    .unwrap();
    let mut metadata = column.metadata().clone();
    metadata.description = "<script>alert(1)</script>".into();
    metadata.missing_rules = MissingRules::Number {
        values: vec![99.0],
        ranges: vec![(100.0, 110.0)],
    };
    metadata.value_labels = vec![ValueLabel {
        value: Cell::Number(1.0),
        label: "<script>alert(1)</script>".into(),
    }];
    let data = Dataset::new(vec![
        column.with_metadata(metadata).unwrap(),
        Column::numeric(
            ColumnId(2),
            "Weight",
            vec![Some(1.0), Some(2.0), Some(1.0), None],
        )
        .unwrap(),
        Column::text(
            ColumnId(3),
            "Group",
            vec![Some("<script>alert(1)</script>".into()); 4],
        )
        .unwrap(),
    ])
    .unwrap();
    let control = JobControl::default();
    let request = DescribeRequest {
        columns: vec![ColumnId(1)],
        statistics: vec![Statistic::Mean, Statistic::SampleStdDev],
        missing_policy: MissingPolicy::CompleteCase,
        filter: RowFilter {
            predicates: vec![RowPredicate::Text {
                column: ColumnId(3),
                value: "<script>alert(1)</script>".into(),
                equal: true,
            }],
        },
        frequency_weight: Some(ColumnId(2)),
    };
    let analysis = describe(&data, &request, &control).unwrap();
    let project = Project::new("Report", data)
        .unwrap()
        .with_run(analysis)
        .unwrap()
        .with_provenance(vec!["<script>alert(1)</script>".into()])
        .unwrap();
    let changed = project
        .dataset()
        .edit_cell(ColumnId(1), 0, Cell::Number(999.0))
        .unwrap();
    let project = project.with_dataset(changed).unwrap();
    let path = dir.join("report.html");
    export_report(&path, &project, &[RunId(1)], &control).unwrap();
    let html = fs::read_to_string(path).unwrap();
    assert!(!html.contains("<script>alert(1)</script>"));
    assert!(html.contains("&lt;script&gt;alert(1)&lt;/script&gt;"));
    assert!(html.contains("Historical results"));
    assert!(html.contains("complete case"));
    assert!(html.contains("Frequency weight: Weight"));
    assert!(html.contains("<td>2</td>"));
    assert!(html.contains("99, inclusive range 100 to 110"));
    assert!(html.contains("role=\"img\""));
    assert!(html.contains("scope=\"col\""));
    assert!(html.contains("Exact plotted values"));
    assert!(html.contains("Sample standard deviation"));
    assert!(html.contains("Frequency</th>"));
    assert!(export_report(
        &dir.join("bad.html"),
        &project,
        &[RunId(1), RunId(1)],
        &control
    )
    .is_err());
    assert!(export_report(&dir.join("missing.html"), &project, &[RunId(999)], &control).is_err());
    assert!(!dir.join("bad.html").exists());
    fs::remove_dir_all(dir).unwrap();
}
#[test]
fn tsv_escaping_numeric_roundtrip_and_budget_cancellation() {
    use tessera::{
        data::{parse_number, MAX_OWNED_BYTES},
        error::AppError,
        export::{export_data_with_budget, export_report_with_budget},
    };
    let dir = std::env::temp_dir().canonicalize().unwrap().join(format!(
        "tessera-tsv-test-{}-{}",
        std::process::id(),
        NEXT.fetch_add(1, Ordering::Relaxed)
    ));
    fs::create_dir(&dir).unwrap();
    let path = dir.join("data.tsv");
    let values = [
        f64::MAX,
        f64::MIN_POSITIVE,
        f64::from_bits(1),
        -0.0,
        1.2345678901234567,
    ];
    let data = Dataset::new(vec![
        Column::numeric(
            ColumnId(1),
            "Number",
            values.iter().copied().map(Some).collect(),
        )
        .unwrap(),
        Column::text(
            ColumnId(2),
            "Text",
            vec![Some("a\tb\"c\n".into()); values.len()],
        )
        .unwrap(),
    ])
    .unwrap();
    let c = JobControl::default();
    export_data(&path, &data, Delimiter::Tab, &c).unwrap();
    let text = fs::read_to_string(path).unwrap();
    assert!(text.contains("\"a\tb\"\"c\n\""));
    for (line, value) in text.split("\r\n").skip(1).zip(values) {
        let number = line.split('\t').next().unwrap();
        assert_eq!(parse_number(number).unwrap().to_bits(), value.to_bits());
    }
    assert!(matches!(
        export_data_with_budget(
            &dir.join("budget.csv"),
            &data,
            Delimiter::Comma,
            &c,
            &[],
            MAX_OWNED_BYTES
        ),
        Err(AppError::SizeLimit { .. })
    ));
    assert!(!dir.join("budget.csv").exists());
    let project = Project::new("Report", data).unwrap();
    assert!(matches!(
        export_report_with_budget(
            &dir.join("budget.html"),
            &project,
            &[],
            &c,
            &[],
            MAX_OWNED_BYTES
        ),
        Err(AppError::SizeLimit { .. })
    ));
    c.cancel();
    assert!(matches!(
        export_data(
            &dir.join("cancel.csv"),
            project.dataset(),
            Delimiter::Comma,
            &c
        ),
        Err(AppError::Cancelled)
    ));
    assert!(!dir.join("cancel.csv").exists());
    fs::remove_dir_all(dir).unwrap();
}
