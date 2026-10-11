use tessera::{data::Dataset, figures::*, operation::JobControl, project::Project, project_io};
#[test]
fn settings_roundtrip_and_omitted_current_presentations_default() {
    let p = Project::new("Figures", Dataset::new(vec![]).unwrap())
        .unwrap()
        .with_figure_preset("Report", FigureSettings::default())
        .unwrap();
    let mut bytes = vec![];
    project_io::encode(&p, &mut bytes, &JobControl::default()).unwrap();
    let q = project_io::decode(bytes.as_slice(), &JobControl::default()).unwrap();
    assert_eq!(q.presets(), p.presets());
    let mut json: serde_json::Value = serde_json::from_slice(&bytes).unwrap();
    json.as_object_mut().unwrap().remove("figures");
    json.as_object_mut().unwrap().remove("figure_presets");
    assert!(project_io::decode(
        serde_json::to_vec(&json).unwrap().as_slice(),
        &JobControl::default()
    )
    .unwrap()
    .presets()
    .is_empty());
}
#[test]
fn unsafe_settings_and_unknown_fields_are_rejected() {
    let s = FigureSettings {
        width_mm: f64::NAN,
        ..Default::default()
    };
    assert!(s.validate().is_err());
    let s = FigureSettings {
        x_bounds: Some([2., 1.]),
        ..Default::default()
    };
    assert!(s.validate().is_err());
    assert!(serde_json::from_str::<FigureSettings>("{\"unexpected\":true}").is_err());
    assert_eq!(
        FigureSettings::default().pixel_size().unwrap(),
        (1890, 1181)
    );
}
#[test]
fn saved_presentations_follow_runs_and_survive_edits() {
    use tessera::{
        analysis::*,
        data::{Column, ColumnId},
        operation::RunId,
    };
    let d = Dataset::new(vec![Column::numeric(
        ColumnId(1),
        "Value",
        vec![Some(1.), Some(2.), Some(3.)],
    )
    .unwrap()])
    .unwrap();
    let a = histogram_with_budget(
        &d,
        &HistogramRequest {
            variable: ColumnId(1),
            bins: HistogramBins::Automatic,
            filter: RowFilter::default(),
            frequency_weight: None,
        },
        &JobControl::default(),
        &[],
        0,
    )
    .unwrap();
    let p = Project::new("Figure", d)
        .unwrap()
        .with_run(CompletedAnalysis::Histogram(a))
        .unwrap();
    let key = figure_keys(&p.runs()[0])[0];
    let settings = FigureSettings {
        title: "Custom title".into(),
        ..Default::default()
    };
    let q = p
        .with_figure(&key, settings.clone())
        .unwrap()
        .duplicate_figure(&key)
        .unwrap()
        .with_provenance(vec!["Source".into()])
        .unwrap();
    assert!(p.figures().is_empty());
    assert_eq!(q.figures().len(), 2);
    assert_eq!(q.figure_settings(&key), settings);
    assert_eq!(q.runs()[0].id, RunId(1));
    assert_eq!(q.runs()[0].analysis, p.runs()[0].analysis);
    assert!(q
        .with_figure(&FigureKey { run: 999, ..key }, settings.clone())
        .is_err());
    assert!(q
        .with_figure_with_budget(&key, settings, &[], usize::MAX)
        .is_err());
    let mut bytes = vec![];
    project_io::encode(&q, &mut bytes, &JobControl::default()).unwrap();
    let restored = project_io::decode(bytes.as_slice(), &JobControl::default()).unwrap();
    assert_eq!(restored.figures(), q.figures());
    let mut invalid: serde_json::Value = serde_json::from_slice(&bytes).unwrap();
    invalid["figures"][0]["key"]["run"] = 999.into();
    assert!(project_io::decode(
        serde_json::to_vec(&invalid).unwrap().as_slice(),
        &JobControl::default()
    )
    .is_err());
    assert!(
        Project::retained_owned_bytes(&[&q, &restored]).unwrap()
            > Project::retained_owned_bytes(&[&q]).unwrap()
    );
}
#[test]
fn copying_style_preserves_figure_specific_text_and_axes() {
    let own = FigureSettings {
        title: "Own".into(),
        x_bounds: Some([0., 2.]),
        metric: FrequencyMetric::AllPercent,
        horizontal: false,
        ..Default::default()
    };
    let style = FigureSettings {
        title: "Other".into(),
        palette: FigurePalette::Monochrome,
        width_mm: 200.,
        ..Default::default()
    };
    let copied = own.copy_style_from(&style);
    assert_eq!(copied.title, own.title);
    assert_eq!(copied.x_bounds, own.x_bounds);
    assert_eq!(copied.metric, own.metric);
    assert!(!copied.horizontal);
    assert_eq!(copied.width_mm, 200.);
    assert_eq!(copied.palette, style.palette);
}
#[test]
fn presentation_limits_and_budgeted_preset_admission() {
    let p = Project::new("Limits", Dataset::new(vec![]).unwrap()).unwrap();
    assert!(p
        .with_figure_preset_with_budget("Name", FigureSettings::default(), &[], usize::MAX)
        .is_err());
    let mut bytes = vec![];
    project_io::encode(&p, &mut bytes, &JobControl::default()).unwrap();
    let mut value: serde_json::Value = serde_json::from_slice(&bytes).unwrap();
    value["figure_presets"] = serde_json::json!((0..257)
        .map(|i| serde_json::json!({"name":format!("Style {i}"),"settings":{}}))
        .collect::<Vec<_>>());
    assert!(project_io::decode(
        serde_json::to_vec(&value).unwrap().as_slice(),
        &JobControl::default()
    )
    .is_err());
    let c = JobControl::default();
    c.cancel();
    assert!(project_io::decode(bytes.as_slice(), &c).is_err());
}
