//! Synthetic external-style imports checked against hand-calculated references.
use std::{path::PathBuf, sync::Arc};
use tessera::{
    analysis::{self, AnalysisRequest, CompletedAnalysis, FrequencyTable, Statistic},
    data::{Cell, ColumnId, ColumnKind, Dataset, MissingRules},
    import::{commit_preview, preview, Delimiter},
    operation::JobControl,
    project::Project,
    survey::{generate_with_budget, inspect_with_budget, SurveyChoice, SurveyRequest, SurveyRole},
};

const FORMS: &[u8] = include_bytes!("fixtures/survey-reference/forms.csv");
const EDGES: &[u8] = include_bytes!("fixtures/survey-reference/edge-cases.csv");

fn csv(bytes: &[u8], missing: bool, edges: bool) -> Dataset {
    let control = JobControl::default();
    let input = preview(Arc::from(bytes), Delimiter::Comma, &control).unwrap();
    let mut choices = input.default_choices().unwrap();
    if missing {
        choices[3].missing_rules = MissingRules::Text(vec![String::new()]);
    }
    if edges {
        choices[2].kind = ColumnKind::Number;
    }
    commit_preview(&input, &choices, &control).unwrap()
}

fn batch(dataset: Dataset, roles: &[(u64, SurveyRole)]) -> Project {
    let project = Project::new("Synthetic reference", dataset).unwrap();
    let request = SurveyRequest {
        revision: project.dataset().revision(),
        choices: roles
            .iter()
            .map(|&(id, role)| SurveyChoice {
                column: ColumnId(id),
                role,
            })
            .collect(),
    };
    generate_with_budget(&project, &request, &JobControl::default(), &[], 0).unwrap()
}

fn close(actual: Option<f64>, expected: f64) {
    assert!(
        (actual.unwrap() - expected).abs() < 1e-10,
        "{actual:?} != {expected}"
    );
}

fn counts(table: &FrequencyTable, expected: &[(&str, u64)], valid: u64, missing: u64) {
    assert_eq!((table.valid_n, table.missing_n), (valid, missing));
    assert_eq!(table.categories.len(), expected.len());
    for &(label, count) in expected {
        let category = table
            .categories
            .iter()
            .find(|c| c.value == Cell::Text(label.into()))
            .unwrap();
        assert_eq!(category.frequency, count);
        close(
            category.percent.value,
            100.0 * count as f64 / (valid + missing) as f64,
        );
        close(
            category.valid_percent.value,
            100.0 * count as f64 / valid as f64,
        );
    }
}

fn frequencies(project: &Project) -> &[FrequencyTable] {
    project
        .runs()
        .iter()
        .find_map(|run| match &run.analysis {
            CompletedAnalysis::Frequencies(value) => Some(value.tables.as_slice()),
            _ => None,
        })
        .unwrap()
}

#[test]
fn realistic_forms_import_keeps_identifiers_labels_and_multiple_answers() {
    let dataset = csv(FORMS, false, false);
    assert_eq!(dataset.row_count(), 8);
    let review = inspect_with_budget(&dataset, &JobControl::default(), &[], 0).unwrap();
    for index in [0, 1, 2, 6] {
        assert_eq!(review.columns[index].suggested_role, SurveyRole::Exclude);
    }
    assert_eq!(review.columns[4].suggested_role, SurveyRole::Unresolved);
    assert_eq!(review.columns[7].suggested_role, SurveyRole::Unresolved);
    assert_eq!(
        dataset.cell(ColumnId(6), 7).unwrap(),
        Cell::Text(" reading ".into())
    );
    assert_eq!(
        dataset.cell(ColumnId(8), 0).unwrap(),
        Cell::Text("Red;Blue".into())
    );
    assert_eq!(dataset.cell(ColumnId(5), 7).unwrap(), Cell::Missing);
    assert_eq!(review.columns[3].valid, 8);
}

#[test]
fn forms_batch_matches_independent_counts_and_numeric_reference() {
    let project = batch(
        csv(FORMS, true, false),
        &[
            (4, SurveyRole::Categories),
            (5, SurveyRole::NumericMeasure),
            (6, SurveyRole::Categories),
            (8, SurveyRole::Categories),
        ],
    );
    let tables = frequencies(&project);
    counts(
        &tables[0],
        &[("Agree", 3), ("Disagree", 2), ("Neutral", 1), ("N/A", 1)],
        7,
        1,
    );
    counts(
        &tables[1],
        &[("Reading", 4), ("Walking", 3), (" reading ", 1)],
        8,
        0,
    );
    counts(
        &tables[2],
        &[
            ("Red;Blue", 3),
            ("Blue", 2),
            ("Red", 1),
            ("Blue;Red", 1),
            ("", 1),
        ],
        8,
        0,
    );
    let CompletedAnalysis::Describe(result) = &project.runs()[1].analysis else {
        panic!("missing descriptive summary")
    };
    let row = &result.rows[0];
    assert_eq!((row.valid_n, row.missing_n), (7, 1));
    close(row.estimates[&Statistic::Mean].value, 3.0);
    close(
        row.estimates[&Statistic::SampleStdDev].value,
        1.290_994_448_735_805_6,
    );
    close(row.estimates[&Statistic::Minimum].value, 1.0);
    close(row.estimates[&Statistic::Maximum].value, 5.0);
    let CompletedAnalysis::Histogram(histogram) = &project.runs()[2].analysis else {
        panic!("missing histogram")
    };
    assert_eq!((histogram.n, histogram.missing_n), (7, 1));
    assert_eq!(
        histogram.bins.iter().map(|bin| bin.frequency).sum::<u64>(),
        7
    );
    for run in project.runs() {
        let request = match &run.analysis {
            CompletedAnalysis::Frequencies(v) => AnalysisRequest::Frequencies(v.request.clone()),
            CompletedAnalysis::Describe(v) => AnalysisRequest::Describe(v.request.clone()),
            CompletedAnalysis::Histogram(v) => AnalysisRequest::Histogram(v.request.clone()),
            _ => panic!("unexpected analysis"),
        };
        let manual = analysis::analyze_with_budget(
            project.dataset(),
            &request,
            &JobControl::default(),
            &[],
            0,
        )
        .unwrap();
        assert_eq!(run.analysis, manual);
    }
}

#[test]
fn missing_rule_is_explicit_and_changes_the_denominator() {
    let project = batch(csv(FORMS, false, false), &[(4, SurveyRole::Categories)]);
    counts(
        &frequencies(&project)[0],
        &[
            ("Agree", 3),
            ("Disagree", 2),
            ("Neutral", 1),
            ("N/A", 1),
            ("", 1),
        ],
        8,
        0,
    );
    assert_eq!(
        project.dataset().cell(ColumnId(4), 4).unwrap(),
        Cell::Text("N/A".into())
    );
}

#[test]
fn constant_all_missing_and_large_identifiers_remain_honest() {
    let dataset = csv(EDGES, false, true);
    assert_eq!(
        dataset.cell(ColumnId(4), 0).unwrap(),
        Cell::Text("9007199254740993".into())
    );
    assert_eq!(
        dataset.cell(ColumnId(4), 3).unwrap(),
        Cell::Text("9007199254740996".into())
    );
    let project = batch(
        dataset,
        &[
            (1, SurveyRole::NumericMeasure),
            (2, SurveyRole::NumericMeasure),
            (3, SurveyRole::NumericMeasure),
        ],
    );
    let CompletedAnalysis::Describe(result) = &project.runs()[0].analysis else {
        panic!("missing descriptive summary")
    };
    let rows = &result.rows;
    assert_eq!((rows[0].valid_n, rows[0].missing_n), (2, 2));
    close(rows[0].estimates[&Statistic::Mean].value, 2.0);
    close(
        rows[0].estimates[&Statistic::SampleStdDev].value,
        std::f64::consts::SQRT_2,
    );
    close(rows[1].estimates[&Statistic::Mean].value, 5.0);
    close(rows[1].estimates[&Statistic::SampleStdDev].value, 0.0);
    assert_eq!((rows[2].valid_n, rows[2].missing_n), (0, 4));
    assert!(rows[2]
        .estimates
        .values()
        .all(|estimate| estimate.value.is_none()));
    for run in &project.runs()[1..] {
        let CompletedAnalysis::Histogram(value) = &run.analysis else {
            panic!("missing histogram")
        };
        assert_eq!(
            value.bins.iter().map(|bin| bin.frequency).sum::<u64>(),
            value.n
        );
    }
}

#[test]
fn reference_report_escapes_external_headers_and_responses() {
    let project = batch(csv(EDGES, false, true), &[(5, SurveyRole::Categories)]);
    let folder = std::fs::canonicalize(std::env::temp_dir())
        .unwrap()
        .join(format!("tessera-survey-reference-{}", std::process::id()));
    std::fs::create_dir(&folder).unwrap();
    let path = folder.join("report.html");
    let result = tessera::export::export_report(
        &path,
        &project,
        &[project.runs()[0].id],
        &JobControl::default(),
    );
    let html = std::fs::read_to_string(&path);
    std::fs::remove_dir_all(&folder).unwrap();
    result.unwrap();
    let html = html.unwrap();
    assert!(html.contains("&lt;script&gt;header&lt;/script&gt;"));
    assert!(html.contains("&lt;script&gt;alert(1)&lt;/script&gt;"));
    assert!(html.contains("A&amp;B"));
    assert!(!html.contains("<script>header</script>"));
    assert!(!html.contains("<script>alert(1)</script>"));
    assert_eq!(
        project.dataset().cell(ColumnId(5), 0).unwrap(),
        Cell::Text("<script>alert(1)</script>".into())
    );
}

#[test]
#[ignore = "requires TESSERA_WORKBOOK_WORKER pointing to the supervised workbook helper"]
fn xlsx_fixture_matches_csv_numeric_and_category_references() {
    use std::{sync::atomic::AtomicBool, time::Duration};
    use tessera::import::excel::{self, ExcelDecisions, ExcelSelection};
    let source = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("tests/fixtures/survey-reference/forms.xlsx");
    let config = tessera_workbook::supervisor::Config {
        worker: std::env::var_os("TESSERA_WORKBOOK_WORKER")
            .expect("set workbook helper path")
            .into(),
        source: source.clone(),
        range: [0, 0, 8, 7],
        worksheet_index: 0,
        quota: 16 * 1024 * 1024,
        retained_bytes: 0,
        timeout: Duration::from_secs(20),
        cancel: Arc::new(AtomicBool::new(false)),
        probe: None,
    };
    let control = JobControl::default();
    let workbook =
        Arc::new(excel::inspect_with_budget(&source, &config, &control, &[], 0).unwrap());
    let input = excel::preview_with_budget(
        workbook,
        ExcelSelection {
            worksheet_index: 0,
            range: [0, 0, 8, 7],
            header: true,
        },
        &config,
        &control,
        &[],
        0,
    )
    .unwrap();
    let mut choices = input.default_choices().unwrap();
    for choice in &mut choices {
        choice.acknowledge_precision_loss = true;
    }
    let dataset = excel::commit_preview_with_budget(
        &input,
        &choices,
        ExcelDecisions::default(),
        &control,
        &[],
        0,
    )
    .unwrap();
    let csv = csv(FORMS, false, false);
    assert_eq!(dataset.row_count(), csv.row_count());
    for column in 1..=8 {
        for row in 0..8 {
            if (column == 4 || column == 8) && row == 7 {
                continue;
            }
            assert_eq!(
                dataset.cell(ColumnId(column), row).unwrap(),
                csv.cell(ColumnId(column), row).unwrap()
            );
        }
    }
    let project = batch(
        dataset,
        &[
            (4, SurveyRole::Categories),
            (5, SurveyRole::NumericMeasure),
            (6, SurveyRole::Categories),
            (8, SurveyRole::Categories),
        ],
    );
    counts(
        &frequencies(&project)[0],
        &[("Agree", 3), ("Disagree", 2), ("Neutral", 1), ("N/A", 1)],
        7,
        1,
    );
    let CompletedAnalysis::Describe(value) = &project.runs()[1].analysis else {
        panic!()
    };
    close(value.rows[0].estimates[&Statistic::Mean].value, 3.0);
    close(
        value.rows[0].estimates[&Statistic::SampleStdDev].value,
        1.290_994_448_735_805_6,
    );
}
