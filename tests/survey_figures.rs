use tessera::{
    analysis::{analyze_with_budget, AnalysisRequest, FrequenciesRequest, RowFilter},
    data::{Column, ColumnId, Dataset},
    figures::{render, FigureKey, FigureKind, FigureSettings},
    operation::JobControl,
    project::Project,
};
fn project(values: Vec<Option<f64>>) -> Project {
    let data = Dataset::new(vec![
        Column::numeric(ColumnId(1), "Question ŵ <&>", values).unwrap()
    ])
    .unwrap();
    let result = analyze_with_budget(
        &data,
        &AnalysisRequest::Frequencies(FrequenciesRequest {
            columns: vec![ColumnId(1)],
            group: None,
            filter: RowFilter::default(),
            frequency_weight: None,
        }),
        &JobControl::default(),
        &[],
        0,
    )
    .unwrap();
    Project::new("Survey", data)
        .unwrap()
        .with_run(result)
        .unwrap()
}
#[test]
fn explicit_scale_missing_categories_and_frozen_capture_render() {
    let p = project(vec![Some(1.), Some(3.), Some(5.), None]);
    let run = &p.runs()[0];
    let before = run.analysis.clone();
    let key = FigureKey {
        run: run.id.0,
        kind: FigureKind::Likert,
        variant: 0,
    };
    assert!(render::svg(
        run,
        &key,
        &FigureSettings::default(),
        false,
        &JobControl::default()
    )
    .is_err());
    let settings = FigureSettings {
        likert_points: Some(5),
        width_mm: 240.,
        height_mm: 180.,
        ..Default::default()
    };
    let svg = render::svg(run, &key, &settings, false, &JobControl::default()).unwrap();
    assert!(svg.contains("Question ŵ &lt;&amp;&gt;"));
    assert!(svg.contains("Disagree: 0"));
    assert!(svg.contains("valid rows 3"));
    assert!(svg.contains("missing rows 1"));
    assert!(!svg.contains("NaN"));
    assert_eq!(run.analysis, before);
    let png = render::png(&svg, &settings, &JobControl::default()).unwrap();
    assert_eq!(&png[..8], b"\x89PNG\r\n\x1a\n");
}
#[test]
fn incompatible_codes_rejected_and_zero_valid_retained() {
    let settings = FigureSettings {
        likert_points: Some(5),
        width_mm: 240.,
        height_mm: 180.,
        ..Default::default()
    };
    for (values, valid) in [(vec![Some(0.), Some(5.)], false), (vec![None, None], true)] {
        let p = project(values);
        let run = &p.runs()[0];
        let key = FigureKey {
            run: run.id.0,
            kind: FigureKind::Likert,
            variant: 0,
        };
        let result = render::svg(run, &key, &settings, false, &JobControl::default());
        assert_eq!(result.is_ok(), valid);
        if valid {
            assert!(result.unwrap().contains("No recorded answers"));
        }
    }
}

#[test]
fn grouped_ten_items_keep_shared_categories_and_complete_rows() {
    let mut columns: Vec<_> = (1..=10)
        .map(|id| {
            Column::numeric(
                ColumnId(id),
                &format!("Item {id}: a longer question about engagement"),
                vec![Some(1.), Some(2.), Some(3.), Some(4.), Some(5.), None],
            )
            .unwrap()
        })
        .collect();
    columns.push(
        Column::text(
            ColumnId(11),
            "School",
            vec![
                Some("Alpha <&>".into()),
                Some("Alpha <&>".into()),
                Some("Alpha <&>".into()),
                Some("Beta".into()),
                Some("Beta".into()),
                Some("Beta".into()),
            ],
        )
        .unwrap(),
    );
    let data = Dataset::new(columns).unwrap();
    let result = analyze_with_budget(
        &data,
        &AnalysisRequest::Frequencies(FrequenciesRequest {
            columns: (1..=10).map(ColumnId).collect(),
            group: Some(ColumnId(11)),
            filter: RowFilter::default(),
            frequency_weight: None,
        }),
        &JobControl::default(),
        &[],
        0,
    )
    .unwrap();
    let p = Project::new("Grouped", data)
        .unwrap()
        .with_run(result)
        .unwrap();
    let run = &p.runs()[0];
    let key = FigureKey {
        run: run.id.0,
        kind: FigureKind::Likert,
        variant: 0,
    };
    let settings = FigureSettings {
        likert_points: Some(5),
        ..p.figure_settings(&key)
    };
    let svg = render::svg(run, &key, &settings, false, &JobControl::default()).unwrap();
    assert!(svg.contains("Item 10: a longer question about engagement"));
    assert!(svg.contains("Alpha &lt;&amp;&gt;"));
    assert_eq!(svg.matches("valid rows").count(), 20);
}
#[test]
fn group_means_use_captured_means_and_legacy_unavailable_fails() {
    use tessera::analysis::{BoxplotRequest, CompletedAnalysis};
    let data = Dataset::new(vec![Column::numeric(
        ColumnId(1),
        "Domain score",
        vec![Some(1.), Some(1.), Some(5.)],
    )
    .unwrap()])
    .unwrap();
    let result = analyze_with_budget(
        &data,
        &AnalysisRequest::Boxplot(BoxplotRequest {
            outcome: ColumnId(1),
            group: None,
            filter: RowFilter::default(),
            frequency_weight: None,
        }),
        &JobControl::default(),
        &[],
        0,
    )
    .unwrap();
    let p = Project::new("Means", data)
        .unwrap()
        .with_run(result)
        .unwrap();
    let mut run = (*p.runs()[0]).clone();
    let key = FigureKey {
        run: run.id.0,
        kind: FigureKind::GroupMeans,
        variant: 0,
    };
    let s = p.figure_settings(&key);
    let svg = render::svg(&run, &key, &s, false, &JobControl::default()).unwrap();
    assert!(svg.contains("Mean: 2.3333333333333335"));
    let CompletedAnalysis::Boxplot(v) = &mut run.analysis else {
        panic!()
    };
    v.groups[0].mean.value = None;
    assert!(render::svg(&run, &key, &s, false, &JobControl::default())
        .unwrap_err()
        .to_string()
        .contains("Rerun"));
}
#[test]
fn old_settings_and_styles_preserve_explicit_scale() {
    let old: FigureSettings = serde_json::from_str("{}").unwrap();
    assert_eq!(old.likert_points, None);
    let selected = FigureSettings {
        likert_points: Some(7),
        ..Default::default()
    };
    assert_eq!(selected.copy_style_from(&old).likert_points, Some(7));
    assert!(FigureSettings {
        likert_points: Some(4),
        ..Default::default()
    }
    .validate()
    .is_err());
}

#[test]
fn borrowed_label_budget_rejects_large_captures_before_drawing() {
    use tessera::analysis::CompletedAnalysis;
    let p = project(vec![Some(1.), Some(5.)]);
    let mut run = (*p.runs()[0]).clone();
    let CompletedAnalysis::Frequencies(v) = &mut run.analysis else {
        panic!()
    };
    v.tables = vec![v.tables[0].clone(); 1025];
    let key = FigureKey {
        run: run.id.0,
        kind: FigureKind::Likert,
        variant: 0,
    };
    let s = FigureSettings {
        likert_points: Some(5),
        ..Default::default()
    };
    let error = render::svg(&run, &key, &s, false, &JobControl::default()).unwrap_err();
    assert!(error.to_string().contains("1024-row"));
    let CompletedAnalysis::Frequencies(v) = &mut run.analysis else {
        panic!()
    };
    v.tables.truncate(10);
    v.columns[0].description = "x".repeat(2000);
    let error = render::svg(&run, &key, &s, false, &JobControl::default()).unwrap_err();
    assert!(error.to_string().contains("text budget"));
}
