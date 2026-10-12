use tessera::{
    data::{Cell, Column, ColumnId, Dataset},
    operation::JobControl,
    preparation::{prepare_with_budget, RecodeRule},
    project::Project,
    survey::cleaning::{preview_with_budget, CleaningRequest, CleaningSpec},
};
#[test]
fn trim_then_exact_mapping_preserves_source_and_preview() {
    let data = Dataset::new(vec![Column::text(
        ColumnId(1),
        "Answer",
        vec![
            Some(" Agree ".into()),
            Some("agree".into()),
            Some(" NA ".into()),
            None,
        ],
    )
    .unwrap()])
    .unwrap();
    let request = CleaningRequest {
        revision: data.revision(),
        source: ColumnId(1),
        name: "Reviewed answer".into(),
        spec: CleaningSpec {
            trim_whitespace: true,
            rules: vec![RecodeRule {
                from: Cell::Text("NA".into()),
                to: Cell::Missing,
            }],
        },
    };
    let preview = preview_with_budget(&data, &request, &JobControl::default(), &[], 0).unwrap();
    assert_eq!(preview.changed, 2);
    assert_eq!(preview.examples.len(), 2);
    let project = Project::new("Survey", data).unwrap();
    let cleaned = prepare_with_budget(
        &project,
        &request.step().unwrap(),
        &JobControl::default(),
        &[],
        0,
    )
    .unwrap();
    assert_eq!(
        cleaned.dataset().cell(ColumnId(2), 0).unwrap(),
        Cell::Text("Agree".into())
    );
    assert_eq!(
        cleaned.dataset().cell(ColumnId(2), 1).unwrap(),
        Cell::Text("agree".into())
    );
    assert_eq!(
        cleaned.dataset().cell(ColumnId(2), 2).unwrap(),
        Cell::Missing
    );
    assert_eq!(
        cleaned.dataset().cell(ColumnId(1), 0).unwrap(),
        Cell::Text(" Agree ".into())
    );
    assert_eq!(project.dataset().column_count(), 1);
    let snapshot = project.data_snapshot_with_budget(&[], 0).unwrap();
    let restored = cleaned
        .with_restored_dataset_and_steps(&snapshot, &JobControl::default(), &[], 0)
        .unwrap();
    assert_eq!(restored.dataset().column_count(), 1);
    assert!(restored.steps().is_empty());
    let mut encoded = Vec::new();
    tessera::project_io::encode(&cleaned, &mut encoded, &JobControl::default()).unwrap();
    let reopened = tessera::project_io::decode(encoded.as_slice(), &JobControl::default()).unwrap();
    assert_eq!(reopened.steps(), cleaned.steps());
    assert_eq!(
        reopened.dataset().cell(ColumnId(2), 2).unwrap(),
        Cell::Missing
    );
}
#[test]
fn numeric_rules_and_missingness_are_explicit() {
    let data = Dataset::new(vec![Column::numeric(
        ColumnId(7),
        "Code",
        vec![Some(99.), Some(1.), None],
    )
    .unwrap()])
    .unwrap();
    let request = CleaningRequest {
        revision: data.revision(),
        source: ColumnId(7),
        name: "Reviewed code".into(),
        spec: CleaningSpec {
            trim_whitespace: false,
            rules: vec![RecodeRule {
                from: Cell::Number(99.),
                to: Cell::Missing,
            }],
        },
    };
    assert_eq!(
        preview_with_budget(&data, &request, &JobControl::default(), &[], 0)
            .unwrap()
            .changed,
        1
    );
    let bad = CleaningRequest {
        spec: CleaningSpec {
            trim_whitespace: true,
            ..request.spec.clone()
        },
        ..request.clone()
    };
    assert!(preview_with_budget(&data, &bad, &JobControl::default(), &[], 0).is_err());
    let p = Project::new("Codes", data).unwrap();
    let q =
        prepare_with_budget(&p, &request.step().unwrap(), &JobControl::default(), &[], 0).unwrap();
    assert_eq!(q.dataset().cell(ColumnId(8), 0).unwrap(), Cell::Missing);
    assert_eq!(q.dataset().cell(ColumnId(7), 0).unwrap(), Cell::Number(99.));
}
#[test]
fn bounded_preview_handles_high_cardinality_unicode_and_no_guessing() {
    let data = Dataset::new(vec![Column::text(
        ColumnId(1),
        "Answer",
        (0..100)
            .map(|n| Some(format!("\u{2003} ateb {n} \u{2003}")))
            .chain([Some("N/A".into()), Some("na".into())])
            .collect(),
    )
    .unwrap()])
    .unwrap();
    let request = CleaningRequest {
        revision: data.revision(),
        source: ColumnId(1),
        name: "Clean".into(),
        spec: CleaningSpec {
            trim_whitespace: true,
            rules: vec![],
        },
    };
    let preview = preview_with_budget(&data, &request, &JobControl::default(), &[], 0).unwrap();
    assert_eq!(preview.changed, 100);
    assert_eq!(preview.examples.len(), 8);
    assert_eq!(preview.examples[0].after, Cell::Text("ateb 0".into()));
    let p = Project::new("Survey", data).unwrap();
    let q =
        prepare_with_budget(&p, &request.step().unwrap(), &JobControl::default(), &[], 0).unwrap();
    assert_eq!(
        q.dataset().cell(ColumnId(2), 100).unwrap(),
        Cell::Text("N/A".into())
    );
}
#[test]
fn cancelled_stale_and_over_budget_requests_return_no_preview() {
    let data = Dataset::new(vec![Column::text(
        ColumnId(1),
        "Answer",
        vec![Some(" a ".into())],
    )
    .unwrap()])
    .unwrap();
    let request = CleaningRequest {
        revision: data.revision(),
        source: ColumnId(1),
        name: "Clean".into(),
        spec: CleaningSpec {
            trim_whitespace: true,
            rules: vec![],
        },
    };
    let control = JobControl::default();
    control.cancel();
    assert!(preview_with_budget(&data, &request, &control, &[], 0).is_err());
    let stale = CleaningRequest {
        revision: request.revision.next().unwrap(),
        ..request.clone()
    };
    assert!(preview_with_budget(&data, &stale, &JobControl::default(), &[], 0).is_err());
    assert!(preview_with_budget(
        &data,
        &request,
        &JobControl::default(),
        &[],
        tessera::data::MAX_OWNED_BYTES
    )
    .is_err());
}
#[test]
fn passive_specs_reject_ambiguous_and_untrusted_mappings() {
    assert!(serde_json::from_str::<CleaningSpec>(
        r#"{"trim_whitespace":true,"rules":[],"script":"run"}"#
    )
    .is_err());
    for rules in [
        vec![RecodeRule {
            from: Cell::Missing,
            to: Cell::Text("x".into()),
        }],
        vec![RecodeRule {
            from: Cell::Text(" a ".into()),
            to: Cell::Text("a".into()),
        }],
        vec![RecodeRule {
            from: Cell::Number(f64::NAN),
            to: Cell::Missing,
        }],
    ] {
        assert!(CleaningSpec {
            trim_whitespace: true,
            rules
        }
        .validate()
        .is_err());
    }
    let rule = RecodeRule {
        from: Cell::Text("a".into()),
        to: Cell::Text("A".into()),
    };
    assert!(CleaningSpec {
        trim_whitespace: false,
        rules: vec![rule.clone(), rule]
    }
    .validate()
    .is_err());
}
#[test]
fn existing_missing_rules_win_over_requested_replacements() {
    use tessera::data::MissingRules;
    let source = Column::text(
        ColumnId(1),
        "Answer",
        vec![
            Some("N/A".into()),
            Some(" Agree ".into()),
            Some("".into()),
            None,
        ],
    )
    .unwrap();
    let mut metadata = source.metadata().clone();
    metadata.missing_rules = MissingRules::Text(vec!["N/A".into()]);
    let data = Dataset::new(vec![source.with_metadata(metadata).unwrap()]).unwrap();
    let request = CleaningRequest {
        revision: data.revision(),
        source: ColumnId(1),
        name: "Clean".into(),
        spec: CleaningSpec {
            trim_whitespace: true,
            rules: vec![RecodeRule {
                from: Cell::Text("N/A".into()),
                to: Cell::Text("Answered".into()),
            }],
        },
    };
    let preview = preview_with_budget(&data, &request, &JobControl::default(), &[], 0).unwrap();
    assert_eq!(preview.changed, 2);
    assert_eq!(preview.examples[0].after, Cell::Missing);
    let p = Project::new("Survey", data).unwrap();
    let q =
        prepare_with_budget(&p, &request.step().unwrap(), &JobControl::default(), &[], 0).unwrap();
    assert_eq!(q.dataset().cell(ColumnId(2), 0).unwrap(), Cell::Missing);
    assert_eq!(
        q.dataset().cell(ColumnId(2), 2).unwrap(),
        Cell::Text("".into())
    );
    assert_eq!(
        q.dataset().cell(ColumnId(1), 0).unwrap(),
        Cell::Text("N/A".into())
    );
    assert_eq!(
        q.dataset().column(ColumnId(1)).unwrap().metadata(),
        p.dataset().column(ColumnId(1)).unwrap().metadata()
    );
}
#[test]
fn independent_preview_peak_accepts_exact_limit_and_rejects_one_more_byte() {
    use std::mem::size_of;
    use tessera::{
        data::{Budget, MAX_OWNED_BYTES},
        survey::cleaning::{CleaningExample, CleaningPreview},
    };
    // This example's string copies exceed the ownership-accounting scratch peak.
    let original = format!(" {} ", "x".repeat(1000));
    let data = Dataset::new(vec![Column::text(
        ColumnId(1),
        "Answer",
        vec![Some(original.clone())],
    )
    .unwrap()])
    .unwrap();
    let request = CleaningRequest {
        revision: data.revision(),
        source: ColumnId(1),
        name: "Clean".into(),
        spec: CleaningSpec {
            trim_whitespace: true,
            rules: vec![],
        },
    };
    let independently_calculated_peak = Budget::dataset_bytes(&[&data], 0).unwrap()
        + size_of::<CleaningSpec>()
        + request.name.capacity()
        + size_of::<CleaningPreview>()
        + 8 * size_of::<CleaningExample>()
        + original.len()
        + 1000;
    let extra = MAX_OWNED_BYTES - independently_calculated_peak;
    assert_eq!(
        preview_with_budget(&data, &request, &JobControl::default(), &[], extra)
            .unwrap()
            .changed,
        1
    );
    assert!(matches!(
        preview_with_budget(&data, &request, &JobControl::default(), &[], extra + 1),
        Err(tessera::error::AppError::SizeLimit {
            limit: MAX_OWNED_BYTES
        })
    ));
    assert_eq!(data.cell(ColumnId(1), 0).unwrap(), Cell::Text(original));
}
