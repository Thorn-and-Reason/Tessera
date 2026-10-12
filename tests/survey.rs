use tessera::{
    data::{Column, ColumnId, Dataset, Measurement, MissingRules, RevisionId, MAX_OWNED_BYTES},
    operation::JobControl,
    survey::{inspect_with_budget, SurveyChoice, SurveyFinding, SurveyRequest, SurveyRole},
};
fn text(id: u64, name: &str, values: &[&str]) -> Column {
    Column::text(
        ColumnId(id),
        name,
        values.iter().map(|s| Some((*s).into())).collect(),
    )
    .unwrap()
}
fn inspect(d: &Dataset) -> tessera::survey::SurveyReview {
    inspect_with_budget(d, &JobControl::default(), &[], 0).unwrap()
}
#[test]
fn forms_columns_require_honest_roles() {
    let numeric = Column::numeric(ColumnId(1), "Answer", vec![Some(1.), Some(2.)]).unwrap();
    let mut metadata = numeric.metadata().clone();
    metadata.name = "Score".into();
    metadata.measurement = Measurement::Scale;
    let scale = Column::numeric(ColumnId(2), "Score", vec![Some(1.), Some(2.)])
        .unwrap()
        .with_metadata(metadata)
        .unwrap();
    let d=Dataset::new(vec![numeric,scale,text(3,"Choice",&["Yes","No"]),text(4,"Name",&["Joe","Ann"]),text(5,"Contact",&["a@example.org","b@example.org"]),text(6,"Identifier",&["9007199254740993","9007199254740994"]),text(7,"Timestamp",&["2026-10-12","2026-10-13"]),text(8,"Comment",&["A long answer describing how someone feels about this topic and its implications for their daily working life.","Short"]),text(9,"Options",&["Red;Blue","Blue"])]).unwrap();
    let r = inspect(&d);
    assert_eq!(r.columns[0].suggested_role, SurveyRole::Unresolved);
    assert_eq!(r.columns[1].suggested_role, SurveyRole::NumericMeasure);
    assert_eq!(r.columns[2].suggested_role, SurveyRole::Categories);
    for c in &r.columns[3..8] {
        assert_eq!(c.suggested_role, SurveyRole::Exclude);
    }
    assert_eq!(r.columns[8].suggested_role, SurveyRole::Unresolved);
}
#[test]
fn cardinality_boundary_is_30() {
    for n in [30, 31, 40] {
        let d = Dataset::new(vec![Column::text(
            ColumnId(1),
            "Choice",
            (0..n).map(|i| Some(format!("choice {i}"))).collect(),
        )
        .unwrap()])
        .unwrap();
        let r = inspect(&d);
        assert_eq!(r.columns[0].distinct_values, n.min(31));
        assert_eq!(
            r.columns[0].suggested_role,
            if n <= 30 {
                SurveyRole::Categories
            } else {
                SurveyRole::Unresolved
            }
        );
    }
}
#[test]
fn quality_findings_do_not_change_data() {
    let c = text(1, "Choice", &["Yes", " yes ", "N/A", "Missing", "Yes"]);
    let mut m = c.metadata().clone();
    m.missing_rules = MissingRules::Text(vec!["Missing".into()]);
    let d = Dataset::new(vec![c.with_metadata(m).unwrap()]).unwrap();
    let before = format!("{d:?}");
    let r = inspect(&d);
    assert_eq!(r.duplicate_rows, 1);
    assert_eq!((r.columns[0].valid, r.columns[0].missing), (4, 1));
    assert!(r.columns[0]
        .findings
        .contains(&SurveyFinding::CandidateMissingToken));
    assert!(r.columns[0]
        .findings
        .contains(&SurveyFinding::LabelVariants));
    assert_eq!(format!("{d:?}"), before);
}
#[test]
fn checked_request_rejects_invalid_roles() {
    let d = Dataset::new(vec![text(1, "Choice", &["Yes"])]).unwrap();
    for choices in [
        vec![],
        vec![SurveyChoice {
            column: ColumnId(2),
            role: SurveyRole::Categories,
        }],
        vec![SurveyChoice {
            column: ColumnId(1),
            role: SurveyRole::Unresolved,
        }],
        vec![SurveyChoice {
            column: ColumnId(1),
            role: SurveyRole::NumericMeasure,
        }],
        vec![
            SurveyChoice {
                column: ColumnId(1),
                role: SurveyRole::Categories
            };
            2
        ],
    ] {
        assert!(SurveyRequest {
            revision: d.revision(),
            choices
        }
        .validate(&d, &JobControl::default())
        .is_err());
    }
    let mut r = SurveyRequest {
        revision: d.revision(),
        choices: vec![SurveyChoice {
            column: ColumnId(1),
            role: SurveyRole::Categories,
        }],
    };
    r.validate(&d, &JobControl::default()).unwrap();
    r.revision = RevisionId(2);
    assert!(r.validate(&d, &JobControl::default()).is_err());
}
#[test]
fn inspection_cancel_and_budget_leave_input_intact() {
    let d = Dataset::new(vec![text(1, "Choice", &["Yes", "No"])]).unwrap();
    let before = format!("{d:?}");
    let c = JobControl::default();
    c.cancel();
    assert!(matches!(
        inspect_with_budget(&d, &c, &[], 0),
        Err(tessera::error::AppError::Cancelled)
    ));
    assert!(matches!(
        inspect_with_budget(&d, &JobControl::default(), &[], MAX_OWNED_BYTES),
        Err(tessera::error::AppError::SizeLimit { .. })
    ));
    let retained = Dataset::new(vec![text(1, "Retained", &["A retained response"])]).unwrap();
    assert!(matches!(
        inspect_with_budget(&d, &JobControl::default(), &[&retained], MAX_OWNED_BYTES),
        Err(tessera::error::AppError::SizeLimit { .. })
    ));
    assert_eq!(format!("{d:?}"), before);
}
#[test]
fn long_headers_and_all_missing_are_bounded() {
    let d = Dataset::new(vec![Column::text(
        ColumnId(1),
        &"x".repeat(10000),
        vec![None, None],
    )
    .unwrap()])
    .unwrap();
    let r = inspect(&d);
    assert_eq!(r.columns[0].suggested_role, SurveyRole::Exclude);
    assert_eq!(r.columns[0].missing, 2);
    assert_eq!(r.duplicate_rows, 1);
    assert!(r.owned_bytes().unwrap() < 1024);
}

#[test]
fn duplicates_compare_raw_missing_codes_and_every_column() {
    let c = text(1, "Choice", &["Missing", "N/A", "Missing", "Missing"]);
    let mut metadata = c.metadata().clone();
    metadata.missing_rules = MissingRules::Text(vec!["Missing".into(), "N/A".into()]);
    let d = Dataset::new(vec![
        c.with_metadata(metadata).unwrap(),
        text(2, "Other", &["A", "A", "A", "B"]),
    ])
    .unwrap();
    let r = inspect(&d);
    assert_eq!(r.duplicate_rows, 1);
    assert_eq!(r.columns[0].missing, 4);
    assert!(!r.columns[0]
        .findings
        .contains(&SurveyFinding::CandidateMissingToken));
    assert!(r.columns[0]
        .findings
        .contains(&SurveyFinding::ExplicitMissingRules));
}
#[test]
fn empty_constant_ordinal_and_explicit_multiple_answer_choice() {
    let empty = Dataset::new(vec![]).unwrap();
    assert_eq!(inspect(&empty).duplicate_rows, 0);
    let c = Column::numeric(ColumnId(1), "Constant", vec![Some(3.); 3]).unwrap();
    let mut m = c.metadata().clone();
    m.measurement = Measurement::Ordinal;
    let d = Dataset::new(vec![
        c.with_metadata(m).unwrap(),
        text(2, "Options", &["A;B", "A", "B"]),
    ])
    .unwrap();
    let r = inspect(&d);
    assert_eq!(r.columns[0].suggested_role, SurveyRole::Categories);
    assert_eq!(r.columns[0].distinct_values, 1);
    assert_eq!(r.columns[1].suggested_role, SurveyRole::Unresolved);
    SurveyRequest {
        revision: d.revision(),
        choices: vec![SurveyChoice {
            column: ColumnId(2),
            role: SurveyRole::Categories,
        }],
    }
    .validate(&d, &JobControl::default())
    .unwrap();
}

#[test]
fn raw_blank_cells_are_findings_without_changing_missing_rules() {
    for explicit_rules in [false, true] {
        let columns = [None, Some(""), Some(" \t\u{2003}"), Some("Missing")]
            .into_iter()
            .enumerate()
            .map(|(i, value)| {
                let column = Column::text(
                    ColumnId(i as u64 + 1),
                    &format!("Choice {i}"),
                    vec![value.map(str::to_owned)],
                )
                .unwrap();
                let mut metadata = column.metadata().clone();
                if explicit_rules {
                    metadata.missing_rules = MissingRules::Text(vec!["Missing".into()]);
                }
                column.with_metadata(metadata).unwrap()
            })
            .collect();
        let dataset = Dataset::new(columns).unwrap();
        let before = format!("{dataset:?}");
        let review = inspect(&dataset);
        for column in &review.columns[..3] {
            assert!(column.findings.contains(&SurveyFinding::EmptyCells));
        }
        assert_eq!((review.columns[0].valid, review.columns[0].missing), (0, 1));
        for column in &review.columns[1..3] {
            assert_eq!((column.valid, column.missing), (1, 0));
        }
        assert_eq!(
            (review.columns[3].valid, review.columns[3].missing),
            if explicit_rules { (0, 1) } else { (1, 0) }
        );
        assert_eq!(
            review.columns[3]
                .findings
                .contains(&SurveyFinding::ExplicitMissingRules),
            explicit_rules
        );
        assert!(!review.columns[3]
            .findings
            .contains(&SurveyFinding::EmptyCells));
        assert_eq!(format!("{dataset:?}"), before);
    }
}

#[path = "survey/generation.rs"]
mod generation;
