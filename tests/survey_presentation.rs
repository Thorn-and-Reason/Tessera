use tessera::{
    data::{Column, ColumnId, Dataset, MAX_OWNED_BYTES},
    figures::{render, FigureKind, FigurePalette},
    operation::JobControl,
    project::Project,
    survey::{
        generate_presented_with_budget, SurveyChoice, SurveyPresentation, SurveyRequest, SurveyRole,
    },
};

fn fixture() -> (Project, SurveyRequest) {
    let project = Project::new(
        "Survey",
        Dataset::new(vec![
            Column::text(
                ColumnId(1),
                "Choice <&>",
                vec![Some("Yes".into()), None, Some("No".into())],
            )
            .unwrap(),
            Column::numeric(ColumnId(2), "Hours", vec![Some(1.), Some(2.), None]).unwrap(),
        ])
        .unwrap(),
    )
    .unwrap();
    let request = SurveyRequest {
        revision: project.dataset().revision(),
        choices: vec![
            SurveyChoice {
                column: ColumnId(1),
                role: SurveyRole::Categories,
            },
            SurveyChoice {
                column: ColumnId(2),
                role: SurveyRole::NumericMeasure,
            },
        ],
    };
    (project, request)
}

#[test]
fn presentation_explains_denominators_and_preserves_results() {
    let (original, request) = fixture();
    let report = generate_presented_with_budget(
        &original,
        &request,
        SurveyPresentation::HighContrast,
        &JobControl::default(),
        &[],
        0,
    )
    .unwrap();
    assert!(original.figures().is_empty());
    assert!(report.figures().len() >= 2);
    for record in report.figures() {
        assert_eq!(record.settings.palette, FigurePalette::Accessible);
        assert_eq!(record.settings.decimals, 2);
        assert!(record
            .settings
            .caption
            .contains("2 valid responses; 1 missing"));
    }
    let record = report
        .figures()
        .iter()
        .find(|f| f.key.kind == FigureKind::Frequency(1))
        .unwrap();
    assert_eq!(record.settings.title, "Choice <&>");
    assert!(record.settings.caption.contains("all 3 responses"));
    let svg = render::svg(
        &report.runs()[0],
        &record.key,
        &record.settings,
        false,
        &JobControl::default(),
    )
    .unwrap();
    assert!(svg.contains("Choice &lt;&amp;&gt;"));
    let next = generate_presented_with_budget(
        &report,
        &request,
        SurveyPresentation::Report,
        &JobControl::default(),
        &[],
        0,
    )
    .unwrap();
    assert_eq!(&next.figures()[..report.figures().len()], report.figures());
    assert!(std::ptr::eq(report.dataset(), original.dataset()));
    for (before, after) in report
        .runs()
        .iter()
        .zip(&next.runs()[report.runs().len()..])
    {
        assert_eq!(before.analysis, after.analysis);
    }
    let control = JobControl::default();
    let mut encoded = Vec::new();
    tessera::project_io::encode(&report, &mut encoded, &control).unwrap();
    let reopened = tessera::project_io::decode(encoded.as_slice(), &control).unwrap();
    assert_eq!(reopened.figures(), report.figures());
}

#[test]
fn long_unicode_headers_are_bounded_without_changing_source() {
    let name = "ŵ".repeat(3000);
    let project = Project::new(
        "Survey",
        Dataset::new(vec![Column::text(ColumnId(1), &name, vec![None]).unwrap()]).unwrap(),
    )
    .unwrap();
    let request = SurveyRequest {
        revision: project.dataset().revision(),
        choices: vec![SurveyChoice {
            column: ColumnId(1),
            role: SurveyRole::Categories,
        }],
    };
    let report = generate_presented_with_budget(
        &project,
        &request,
        SurveyPresentation::Report,
        &JobControl::default(),
        &[],
        0,
    )
    .unwrap();
    assert_eq!(report.figures()[0].settings.title.len(), 4096);
    assert!(report.figures()[0]
        .settings
        .caption
        .contains("0 valid responses; 1 missing"));
    assert_eq!(report.dataset().column(ColumnId(1)).unwrap().name(), name);
}

#[test]
fn presentation_is_checked_and_generation_respects_limits() {
    assert_eq!(
        serde_json::from_str::<SurveyPresentation>("\"report\"").unwrap(),
        SurveyPresentation::default()
    );
    assert!(serde_json::from_str::<SurveyPresentation>("\"unknown\"").is_err());
    let (project, request) = fixture();
    let control = JobControl::default();
    control.cancel();
    assert!(generate_presented_with_budget(
        &project,
        &request,
        SurveyPresentation::Report,
        &control,
        &[],
        0
    )
    .is_err());
    assert!(generate_presented_with_budget(
        &project,
        &request,
        SurveyPresentation::Report,
        &JobControl::default(),
        &[],
        MAX_OWNED_BYTES
    )
    .is_err());
}
