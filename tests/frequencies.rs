use tessera::{analysis::*, data::*, error::AppError, operation::JobControl};

fn request(columns: &[u64]) -> FrequenciesRequest {
    FrequenciesRequest {
        group: None,
        columns: columns.iter().map(|id| ColumnId(*id)).collect(),
        filter: RowFilter::default(),
        frequency_weight: None,
    }
}

#[test]
fn multiple_tables_and_long_labels_keep_monotonic_public_progress() {
    use std::sync::{atomic::AtomicBool, mpsc::channel, Arc};
    let prefix = "ŵ".repeat(5000);
    let text = Column::text(
        ColumnId(2),
        "Text",
        (0..5000)
            .map(|i| Some(format!("{prefix}{}", i % 2)))
            .collect(),
    )
    .unwrap();
    let text = text
        .with_metadata(ColumnMetadata {
            value_labels: vec![
                ValueLabel {
                    value: Cell::Text(format!("{prefix}1")),
                    label: "ŵ".repeat(10_000),
                },
                ValueLabel {
                    value: Cell::Text(format!("{prefix}0")),
                    label: "ŵ".repeat(10_000),
                },
            ],
            ..text.metadata().clone()
        })
        .unwrap();
    let dataset = Dataset::new(vec![
        Column::numeric(
            ColumnId(1),
            "N",
            (0..5000).map(|i| Some((i % 3) as f64)).collect(),
        )
        .unwrap(),
        text,
    ])
    .unwrap();
    let r = request(&[1, 2]);
    let (sender, receiver) = channel();
    let control = JobControl::new(Arc::new(AtomicBool::new(false)), Some(sender));
    let result = frequencies_with_budget(&dataset, &r, &control, &[], 0).unwrap();
    result.validate().unwrap();
    let progress = receiver.try_iter().collect::<Vec<_>>();
    let regression = progress.windows(2).find(|p| p[0].done > p[1].done);
    assert!(
        regression.is_none(),
        "progress moved backward: {regression:?}"
    );
    assert!(progress.iter().all(|p| p.total == Some(15_000)));
    assert_eq!(progress.last().unwrap().done, 15_000);
    assert_eq!(result.tables.len(), 2);
    assert_eq!(result.tables[1].categories[0].value_label_index, Some(1));
}
fn compute(dataset: &Dataset, r: &FrequenciesRequest) -> ComputedFrequencies {
    frequencies_with_budget(
        dataset,
        r,
        &JobControl::default(),
        &[],
        r.heap_bytes().unwrap(),
    )
    .unwrap()
}
fn near(actual: Option<f64>, expected: f64) {
    assert!((actual.unwrap() - expected).abs() <= 1e-12);
}
#[test]
fn independently_counted_numeric_percentages_and_missing_denominators() {
    let dataset = Dataset::new(vec![Column::numeric(
        ColumnId(1),
        "Number",
        vec![Some(1.), Some(1.), Some(2.), None],
    )
    .unwrap()])
    .unwrap();
    let result = compute(&dataset, &request(&[1]));
    let t = &result.tables[0];
    assert_eq!(
        (result.source_rows, result.diagnostics.complete_case_rows),
        (4, 0)
    );
    assert_eq!(
        (t.valid_rows, t.missing_rows, t.valid_n, t.missing_n),
        (3, 1, 3, 1)
    );
    assert_eq!(
        t.categories
            .iter()
            .map(|c| (c.value.clone(), c.rows, c.frequency, c.cumulative_frequency))
            .collect::<Vec<_>>(),
        vec![
            (Cell::Number(1.), 2, 2, Some(2)),
            (Cell::Number(2.), 1, 1, Some(3))
        ]
    );
    near(t.categories[0].percent.value, 50.);
    near(t.categories[1].percent.value, 25.);
    near(t.categories[0].valid_percent.value, 200. / 3.);
    near(t.categories[1].valid_percent.value, 100. / 3.);
    near(
        t.categories[0].cumulative_percent.as_ref().unwrap().value,
        200. / 3.,
    );
    near(
        t.categories[1].cumulative_percent.as_ref().unwrap().value,
        100.,
    );
    near(t.missing_percent.value, 25.);
    result.validate().unwrap();
}
#[test]
fn exact_text_whitespace_signed_zero_labels_and_independent_missingness() {
    let n = Column::numeric(ColumnId(1), "N", vec![Some(-0.), Some(0.), None, Some(2.)]).unwrap();
    let n = n
        .with_metadata(ColumnMetadata {
            value_labels: vec![
                ValueLabel {
                    value: Cell::Number(2.),
                    label: "Same".into(),
                },
                ValueLabel {
                    value: Cell::Number(-0.),
                    label: "Same".into(),
                },
            ],
            ..n.metadata().clone()
        })
        .unwrap();
    let text = Column::text(
        ColumnId(2),
        "Text",
        ["", " ", "A", "A "].map(|v| Some(v.into())).to_vec(),
    )
    .unwrap();
    let result = compute(&Dataset::new(vec![n, text]).unwrap(), &request(&[1, 2]));
    let tables = &result.tables;
    assert_eq!(
        (
            tables[0].valid_n,
            tables[0].missing_n,
            tables[1].valid_n,
            tables[1].missing_n
        ),
        (3, 1, 4, 0)
    );
    assert_eq!(tables[0].categories[0].value, Cell::Number(0.));
    assert_eq!(tables[0].categories[0].value_label_index, Some(1));
    assert_eq!(tables[0].categories[1].value_label_index, Some(0));
    assert_eq!(
        tables[1]
            .categories
            .iter()
            .map(|c| c.value.clone())
            .collect::<Vec<_>>(),
        ["", " ", "A", "A "].map(|s| Cell::Text(s.into()))
    );
    assert!(tables[1]
        .categories
        .iter()
        .all(|c| c.cumulative_percent.is_none() && c.cumulative_frequency.is_none()));
    result.validate().unwrap();
}
#[test]
fn weights_are_literal_replication_and_validated_before_missing_outcomes() {
    let dataset = Dataset::new(vec![
        Column::numeric(ColumnId(1), "N", vec![Some(1.), Some(2.), None]).unwrap(),
        Column::numeric(ColumnId(2), "W", vec![Some(2.), Some(3.), Some(1.)]).unwrap(),
    ])
    .unwrap();
    let mut r = request(&[1]);
    r.frequency_weight = Some(ColumnId(2));
    let weighted = compute(&dataset, &r);
    let expanded = Dataset::new(vec![Column::numeric(
        ColumnId(1),
        "N",
        vec![Some(1.), Some(1.), Some(2.), Some(2.), Some(2.), None],
    )
    .unwrap()])
    .unwrap();
    let reference = compute(&expanded, &request(&[1]));
    for (a, b) in weighted.tables[0]
        .categories
        .iter()
        .zip(&reference.tables[0].categories)
    {
        assert_eq!(
            (
                a.frequency,
                &a.percent,
                &a.valid_percent,
                &a.cumulative_percent
            ),
            (
                b.frequency,
                &b.percent,
                &b.valid_percent,
                &b.cumulative_percent
            )
        );
    }
    assert_eq!(
        (weighted.tables[0].valid_n, weighted.tables[0].missing_n),
        (5, 1)
    );
    weighted.validate().unwrap();
    let bad = Dataset::new(vec![
        Column::numeric(ColumnId(1), "N", vec![None, Some(2.)]).unwrap(),
        Column::numeric(ColumnId(2), "W", vec![Some(-1.), Some(3.)]).unwrap(),
    ])
    .unwrap();
    assert!(frequencies_with_budget(&bad, &r, &JobControl::default(), &[], 0).is_err());
    r.filter.predicates.push(RowPredicate::Missing {
        column: ColumnId(1),
        missing: false,
    });
    let filtered = compute(&bad, &r);
    assert_eq!(filtered.diagnostics.filtered_rows, 1);
    assert_eq!(filtered.tables[0].valid_n, 3);
}
#[test]
fn empty_all_missing_complete_high_cardinality_and_budget_refusal() {
    for values in [vec![], vec![None, None]] {
        let d = Dataset::new(vec![Column::numeric(ColumnId(1), "N", values).unwrap()]).unwrap();
        let r = compute(&d, &request(&[1]));
        assert!(r.tables[0].categories.is_empty());
        assert!(r.tables[0].missing_percent.value.is_some() == (d.row_count() > 0));
        r.validate().unwrap();
    }
    let dataset = Dataset::new(vec![Column::numeric(
        ColumnId(1),
        "N",
        (0..10_000).map(|v| Some(v as f64)).collect(),
    )
    .unwrap()])
    .unwrap();
    let r = request(&[1]);
    assert_eq!(compute(&dataset, &r).tables[0].categories.len(), 10_000);
    let retained = Dataset::new(vec![Column::text(
        ColumnId(9),
        "Retained",
        vec![Some("r".repeat(2_000_000))],
    )
    .unwrap()])
    .unwrap();
    assert!(matches!(
        frequencies_with_budget(
            &dataset,
            &r,
            &JobControl::default(),
            &[&retained],
            MAX_OWNED_BYTES - 1_000_000
        ),
        Err(AppError::SizeLimit { .. })
    ));
    let control = JobControl::default();
    control.cancel();
    assert!(matches!(
        frequencies_with_budget(&dataset, &r, &control, &[], 0),
        Err(AppError::Cancelled)
    ));
}
#[test]
fn zero_missing_weights_and_total_frequency_boundary() {
    let dataset = Dataset::new(vec![
        Column::numeric(ColumnId(1), "N", vec![Some(1.), None, Some(2.), None]).unwrap(),
        Column::numeric(ColumnId(2), "W", vec![Some(2.), Some(0.), None, Some(3.)]).unwrap(),
    ])
    .unwrap();
    let mut r = request(&[1]);
    r.frequency_weight = Some(ColumnId(2));
    let result = compute(&dataset, &r);
    assert_eq!(
        (
            result.diagnostics.zero_weight_rows,
            result.diagnostics.missing_weight_rows
        ),
        (1, 1)
    );
    assert_eq!(
        (
            result.tables[0].valid_rows,
            result.tables[0].missing_rows,
            result.tables[0].valid_n,
            result.tables[0].missing_n
        ),
        (1, 1, 2, 3)
    );
    near(result.tables[0].categories[0].percent.value, 40.);
    result.validate().unwrap();
    let max = Dataset::new(vec![
        Column::numeric(ColumnId(1), "N", vec![Some(1.)]).unwrap(),
        Column::numeric(ColumnId(2), "W", vec![Some(MAX_FREQUENCY as f64)]).unwrap(),
    ])
    .unwrap();
    assert_eq!(compute(&max, &r).tables[0].valid_n, MAX_FREQUENCY);
}
#[test]
fn category_growth_refuses_complete_output_while_duplicate_values_fit() {
    let rows = 5000;
    for (distinct, should_fit) in [(true, false), (false, true)] {
        let d = Dataset::new(vec![Column::numeric(
            ColumnId(1),
            "N",
            (0..rows)
                .map(|i| Some(if distinct { i as f64 } else { 1. }))
                .collect(),
        )
        .unwrap()])
        .unwrap();
        let extra =
            MAX_OWNED_BYTES - Budget::dataset_bytes(&[&d], 0).unwrap() - rows * 64 - 100_000;
        let computed =
            frequencies_with_budget(&d, &request(&[1]), &JobControl::default(), &[], extra);
        if should_fit {
            assert_eq!(computed.unwrap().tables[0].categories.len(), 1);
        } else {
            assert!(matches!(computed, Err(AppError::SizeLimit { .. })));
        }
    }
}
#[test]
fn missing_rules_request_validation_and_overflow_refuse_without_source_changes() {
    let column = Column::text(
        ColumnId(1),
        "T",
        ["", " ", "A", "A "].map(|v| Some(v.into())).to_vec(),
    )
    .unwrap();
    let column = column
        .with_metadata(ColumnMetadata {
            missing_rules: MissingRules::Text(vec![" ".into()]),
            ..column.metadata().clone()
        })
        .unwrap();
    let dataset = Dataset::new(vec![column]).unwrap();
    let result = compute(&dataset, &request(&[1]));
    assert_eq!(
        (result.tables[0].valid_n, result.tables[0].missing_n),
        (3, 1)
    );
    assert_eq!(
        result.tables[0].categories[0].value,
        Cell::Text(String::new())
    );
    result.validate().unwrap();
    for r in [
        request(&[]),
        request(&[1, 1]),
        request(&[0]),
        request(&[999]),
    ] {
        assert!(frequencies_with_budget(&dataset, &r, &JobControl::default(), &[], 0).is_err());
    }
    let overflow = Dataset::new(vec![
        Column::numeric(ColumnId(1), "N", vec![Some(1.), None]).unwrap(),
        Column::numeric(ColumnId(2), "W", vec![Some(MAX_FREQUENCY as f64), Some(1.)]).unwrap(),
    ])
    .unwrap();
    let mut r = request(&[1]);
    r.frequency_weight = Some(ColumnId(2));
    assert!(frequencies_with_budget(&overflow, &r, &JobControl::default(), &[], 0).is_err());
    assert_eq!(overflow.cell(ColumnId(1), 1).unwrap(), Cell::Missing);
}

#[test]
fn grouped_weighted_tables_capture_missing_groups_and_absent_categories() {
    let dataset = Dataset::new(vec![
        Column::numeric(
            ColumnId(1),
            "Outcome",
            vec![Some(1.), None, Some(2.), Some(1.), None],
        )
        .unwrap(),
        Column::text(
            ColumnId(2),
            "Group",
            vec![
                Some("A".into()),
                Some("A".into()),
                Some("B".into()),
                None,
                None,
            ],
        )
        .unwrap(),
        Column::numeric(
            ColumnId(3),
            "Weight",
            vec![Some(2.), Some(3.), Some(4.), Some(5.), Some(6.)],
        )
        .unwrap(),
    ])
    .unwrap();
    let mut r = request(&[1]);
    r.group = Some(ColumnId(2));
    r.frequency_weight = Some(ColumnId(3));
    let result = compute(&dataset, &r);
    result.validate().unwrap();
    assert_eq!(
        result
            .groups
            .iter()
            .map(|g| (g.value.clone(), g.rows, g.frequency))
            .collect::<Vec<_>>(),
        vec![
            (Cell::Text("A".into()), 2, 5),
            (Cell::Text("B".into()), 1, 4),
            (Cell::Missing, 2, 11)
        ]
    );
    assert_eq!(result.groups[0].tables[0].categories.len(), 1);
    assert_eq!(result.groups[0].tables[0].missing_n, 3);
    assert_eq!(result.groups[2].tables[0].missing_n, 6);
    let mut broken = result.clone();
    broken.groups[0].frequency += 1;
    assert!(broken.validate().is_err());
    r.columns.push(ColumnId(2));
    assert!(r.validate_current(&dataset, &[], 0).is_err());
}

#[test]
fn numeric_group_labels_are_captured_and_empty_cohorts_have_no_groups() {
    let group = Column::numeric(ColumnId(2), "Group", vec![Some(2.), Some(1.), None]).unwrap();
    let group = group
        .with_metadata(ColumnMetadata {
            value_labels: vec![ValueLabel {
                value: Cell::Number(1.),
                label: "First".into(),
            }],
            ..group.metadata().clone()
        })
        .unwrap();
    let dataset = Dataset::new(vec![
        Column::numeric(ColumnId(1), "Outcome", vec![Some(1.), Some(2.), None]).unwrap(),
        group,
    ])
    .unwrap();
    let mut r = request(&[1]);
    r.group = Some(ColumnId(2));
    let result = compute(&dataset, &r);
    result.validate().unwrap();
    assert_eq!(result.groups[0].value, Cell::Number(1.));
    assert_eq!(result.groups[0].value_label_index, Some(0));
    assert_eq!(result.groups[2].value, Cell::Missing);
    r.filter.predicates.push(RowPredicate::Number {
        column: ColumnId(1),
        comparison: NumericComparison::Greater,
        value: 99.,
    });
    let result = compute(&dataset, &r);
    result.validate().unwrap();
    assert!(result.groups.is_empty());
}
