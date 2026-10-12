use tessera::{analysis::*, data::*, error::AppError, operation::JobControl};
#[path = "boxplot/no_inliers.rs"]
mod no_inliers;

fn request(group: Option<ColumnId>) -> BoxplotRequest {
    BoxplotRequest {
        outcome: ColumnId(1),
        group,
        filter: RowFilter::default(),
        frequency_weight: None,
    }
}
fn dataset(values: Vec<Option<f64>>) -> Dataset {
    Dataset::new(vec![Column::numeric(ColumnId(1), "Value", values).unwrap()]).unwrap()
}
fn compute(d: &Dataset, r: &BoxplotRequest) -> ComputedBoxplot {
    let result =
        boxplot_with_budget(d, r, &JobControl::default(), &[], r.heap_bytes().unwrap()).unwrap();
    result.validate().unwrap();
    result
}
#[test]
fn portable_exact_rational_quartiles_fences_and_large_weighted_ranks() {
    let reference: serde_json::Value =
        serde_json::from_str(include_str!("fixtures/boxplot-reference.json")).unwrap();
    for case in reference["cases"].as_array().unwrap() {
        let values = case["values_bits"]
            .as_array()
            .unwrap()
            .iter()
            .map(|v| Some(f64::from_bits(v.as_u64().unwrap())))
            .collect();
        let weights = case["weights"]
            .as_array()
            .unwrap()
            .iter()
            .map(|v| Some(v.as_u64().unwrap() as f64))
            .collect();
        let d = Dataset::new(vec![
            Column::numeric(ColumnId(1), "Value", values).unwrap(),
            Column::numeric(ColumnId(2), "Weight", weights).unwrap(),
        ])
        .unwrap();
        let mut r = request(None);
        r.frequency_weight = Some(ColumnId(2));
        let result = compute(&d, &r);
        let g = &result.groups[0];
        assert_eq!(g.n, case["n"].as_u64().unwrap(), "{case}");
        // Independent rational reference; zero ULP tolerance, including signed-zero bits.
        for (actual, expected) in [&g.q1, &g.median, &g.q3]
            .into_iter()
            .zip(case["quartiles_bits"].as_array().unwrap())
        {
            assert_eq!(
                actual.value.unwrap().to_bits(),
                expected.as_u64().unwrap(),
                "{case}"
            );
        }
        for (actual, expected) in g
            .whiskers
            .iter()
            .zip(case["whiskers_bits"].as_array().unwrap())
        {
            assert_eq!(
                actual.value.unwrap().to_bits(),
                expected.as_u64().unwrap(),
                "{case}"
            );
        }
        assert_eq!(g.outlier_n, case["outlier_n"].as_u64().unwrap(), "{case}");
        let expected = case["outlier_rows"]
            .as_array()
            .unwrap()
            .iter()
            .map(|v| v.as_u64().unwrap() as usize)
            .collect::<Vec<_>>();
        assert_eq!(g.outlier_rows, expected.len(), "{case}");
        assert_eq!(
            result.points.iter().map(|p| p.row).collect::<Vec<_>>(),
            expected,
            "{case}"
        );
        for point in &result.points {
            assert_eq!(
                point.frequency,
                case["weights"][point.row].as_u64().unwrap(),
                "{case}"
            );
            assert_eq!(
                point.value.to_bits(),
                case["values_bits"][point.row].as_u64().unwrap(),
                "{case}"
            );
        }
    }
}
#[test]
fn ties_singletons_empty_shapes_and_distinct_quartile_whisker_orderings() {
    for (values, quartiles, whiskers, outliers) in [
        (vec![0., 1., 2., 3., 100.], [1., 2., 3.], [0., 3.], 1),
        (vec![1., 1., 1., 2.], [1., 1., 1.25], [1., 1.], 1),
        (vec![1., 1., 1., 1., 2.], [1., 1., 1.], [1., 1.], 1),
        (vec![0., 1., 2., 100.], [0.75, 1.5, 26.5], [0., 2.], 1),
        (vec![7.], [7., 7., 7.], [7., 7.], 0),
    ] {
        let result = compute(
            &dataset(values.into_iter().map(Some).collect()),
            &request(None),
        );
        let g = &result.groups[0];
        assert_eq!(
            [
                g.q1.value.unwrap(),
                g.median.value.unwrap(),
                g.q3.value.unwrap()
            ],
            quartiles
        );
        assert_eq!(g.whiskers.clone().map(|e| e.value.unwrap()), whiskers);
        assert_eq!(g.outlier_rows, outliers);
    }
    let d = Dataset::new(vec![
        Column::numeric(ColumnId(1), "Value", vec![None]).unwrap(),
        Column::text(ColumnId(2), "Group", vec![Some("A".into())]).unwrap(),
    ])
    .unwrap();
    let ungrouped = compute(&d, &request(None));
    assert_eq!(ungrouped.groups.len(), 1);
    let g = &ungrouped.groups[0];
    assert_eq!((g.rows, g.n, g.value.as_ref()), (0, 0, None));
    assert!([
        &g.minimum,
        &g.q1,
        &g.median,
        &g.q3,
        &g.maximum,
        &g.whiskers[0],
        &g.whiskers[1]
    ]
    .into_iter()
    .all(|e| e.value.is_none() && e.reason.is_some()));
    assert!(ungrouped.empty_reason.is_some());
    let grouped = compute(&d, &request(Some(ColumnId(2))));
    assert!(grouped.groups.is_empty() && grouped.empty_reason.is_some());
    assert_eq!(compute(&dataset(vec![]), &request(None)).groups.len(), 1);
}
#[test]
fn filter_weight_group_outcome_are_disjoint_ordered_partitions() {
    let d = Dataset::new(vec![
        Column::numeric(
            ColumnId(1),
            "Value",
            vec![None, None, None, Some(8.), Some(9.), None, Some(10.)],
        )
        .unwrap(),
        Column::text(
            ColumnId(2),
            "Group",
            vec![
                None,
                None,
                Some("A".into()),
                Some("A".into()),
                None,
                None,
                None,
            ],
        )
        .unwrap(),
        Column::numeric(
            ColumnId(3),
            "Weight",
            vec![
                Some(-1.),
                Some(2.),
                Some(3.),
                Some(4.),
                Some(0.),
                None,
                Some(5.),
            ],
        )
        .unwrap(),
    ])
    .unwrap();
    let mut r = request(Some(ColumnId(2)));
    r.frequency_weight = Some(ColumnId(3));
    assert!(boxplot_with_budget(&d, &r, &JobControl::default(), &[], 0).is_err());
    r.filter.predicates.push(RowPredicate::Number {
        column: ColumnId(3),
        comparison: NumericComparison::GreaterOrEqual,
        value: 0.,
    });
    let result = compute(&d, &r);
    assert_eq!(
        (
            result.missing_group_rows,
            result.missing_group_n,
            result.missing_outcome_rows,
            result.missing_outcome_n
        ),
        (2, 7, 1, 3)
    );
    assert_eq!((result.groups[0].rows, result.groups[0].n), (1, 4));
    assert_eq!(
        (
            result.diagnostics.filtered_rows,
            result.diagnostics.zero_weight_rows,
            result.diagnostics.complete_case_rows
        ),
        (2, 1, 0)
    );
    r.filter = RowFilter {
        predicates: vec![RowPredicate::Missing {
            column: ColumnId(3),
            missing: true,
        }],
    };
    assert_eq!(compute(&d, &r).diagnostics.missing_weight_rows, 1);
}
#[test]
fn grouped_identity_labels_whitespace_and_numeric_signed_zero() {
    let column = Column::text(
        ColumnId(2),
        "Group",
        ["a", " ", "", "a ", "a"].map(|v| Some(v.into())).to_vec(),
    )
    .unwrap();
    let column = column
        .with_metadata(ColumnMetadata {
            value_labels: vec![
                ValueLabel {
                    value: Cell::Text("a ".into()),
                    label: "Same".into(),
                },
                ValueLabel {
                    value: Cell::Text("a".into()),
                    label: "Same".into(),
                },
            ],
            ..column.metadata().clone()
        })
        .unwrap();
    let d = Dataset::new(vec![
        Column::numeric(
            ColumnId(1),
            "Value",
            vec![Some(1.), Some(2.), Some(3.), Some(4.), Some(5.)],
        )
        .unwrap(),
        column,
    ])
    .unwrap();
    let result = compute(&d, &request(Some(ColumnId(2))));
    assert_eq!(
        result
            .groups
            .iter()
            .map(|g| (g.value.clone(), g.value_label_index, g.rows))
            .collect::<Vec<_>>(),
        vec![
            (Some(Cell::Text("".into())), None, 1),
            (Some(Cell::Text(" ".into())), None, 1),
            (Some(Cell::Text("a".into())), Some(1), 2),
            (Some(Cell::Text("a ".into())), Some(0), 1),
        ]
    );
    let d = Dataset::new(vec![
        Column::numeric(ColumnId(1), "Value", vec![Some(3.), Some(1.), Some(2.)]).unwrap(),
        Column::numeric(ColumnId(2), "Group", vec![Some(-0.), Some(0.), Some(-1.)]).unwrap(),
    ])
    .unwrap();
    let result = compute(&d, &request(Some(ColumnId(2))));
    assert_eq!(result.groups.len(), 2);
    assert_eq!((result.groups[1].rows, result.groups[1].n), (2, 2));
    let Some(Cell::Number(zero)) = result.groups[1].value else {
        panic!()
    };
    assert_eq!(zero.to_bits(), 0);
}
#[test]
fn full_summaries_and_global_deterministic_outlier_cap_across_groups() {
    let mut values = Vec::new();
    let mut groups = Vec::new();
    // 1200 outliers in each group; full-data quartiles stay zero even under reduction.
    for group in ["B", "A"] {
        for row in 0..6000 {
            values.push(Some(if row < 4800 { 0. } else { (row - 4799) as f64 }));
            groups.push(Some(group.into()));
        }
    }
    let d = Dataset::new(vec![
        Column::numeric(ColumnId(1), "Value", values).unwrap(),
        Column::text(ColumnId(2), "Group", groups).unwrap(),
    ])
    .unwrap();
    let result = compute(&d, &request(Some(ColumnId(2))));
    assert!(result.reduced);
    assert_eq!(result.points.len(), 2000);
    for g in &result.groups {
        assert_eq!(
            (g.rows, g.n, g.outlier_rows, g.outlier_n),
            (6000, 6000, 1200, 1200)
        );
        assert_eq!([g.q1.value, g.median.value, g.q3.value], [Some(0.); 3]);
        assert_eq!(g.maximum.value, Some(1200.));
    }
    for (i, p) in result.points.iter().enumerate() {
        let ordinal = i * 2399 / 1999;
        let group = ordinal / 1200;
        let value = ordinal % 1200 + 1;
        assert_eq!(
            (p.group_index, p.value, p.frequency),
            (group, value as f64, 1)
        );
        assert_eq!(
            p.row,
            if group == 0 {
                10799 + value
            } else {
                4799 + value
            }
        );
    }
}
#[test]
fn finite_extremes_near_max_subnormals_and_frequency_replication() {
    for values in [
        vec![-f64::MAX, f64::MAX],
        vec![f64::MAX.next_down(), f64::MAX],
        vec![f64::from_bits(1), f64::from_bits(2)],
    ] {
        let result = compute(
            &dataset(values.iter().copied().map(Some).collect()),
            &request(None),
        );
        let g = &result.groups[0];
        assert_eq!(g.rows, 2);
        assert_eq!(g.outlier_rows, 0);
        assert_eq!(
            [g.minimum.value, g.maximum.value],
            [Some(values[0]), Some(values[1])]
        );
        assert!([&g.q1, &g.median, &g.q3]
            .into_iter()
            .all(|e| e.value.unwrap().is_finite()));
    }
    let d = Dataset::new(vec![
        Column::numeric(ColumnId(1), "Value", vec![Some(1.), Some(2.), Some(100.)]).unwrap(),
        Column::numeric(ColumnId(2), "W", vec![Some(2.), Some(3.), Some(1.)]).unwrap(),
    ])
    .unwrap();
    let mut r = request(None);
    r.frequency_weight = Some(ColumnId(2));
    let weighted = compute(&d, &r);
    let expanded = compute(
        &dataset([1., 1., 2., 2., 2., 100.].map(Some).to_vec()),
        &request(None),
    );
    let a = &weighted.groups[0];
    let b = &expanded.groups[0];
    assert_eq!(
        (&a.q1, &a.median, &a.q3, &a.whiskers, a.n, a.outlier_n),
        (&b.q1, &b.median, &b.q3, &b.whiskers, b.n, b.outlier_n)
    );
}
#[test]
fn invalid_roles_cancellation_retained_budget_and_caller_extra_once() {
    let d = dataset(vec![Some(0.), Some(4.)]);
    assert!(boxplot_with_budget(
        &d,
        &request(Some(ColumnId(1))),
        &JobControl::default(),
        &[],
        0
    )
    .is_err());
    let r = request(None);
    let control = JobControl::default();
    control.cancel();
    assert!(matches!(
        boxplot_with_budget(&d, &r, &control, &[], 0),
        Err(AppError::Cancelled)
    ));
    let retained = Dataset::new(vec![Column::text(
        ColumnId(9),
        "Old",
        vec![Some("r".repeat(2_000_000))],
    )
    .unwrap()])
    .unwrap();
    assert!(matches!(
        boxplot_with_budget(
            &d,
            &r,
            &JobControl::default(),
            &[&retained],
            MAX_OWNED_BYTES - 1_000_000
        ),
        Err(AppError::SizeLimit { .. })
    ));
    let live = Budget::dataset_bytes(&[&d], 0).unwrap();
    let result = boxplot_with_budget(
        &d,
        &r,
        &JobControl::default(),
        &[],
        MAX_OWNED_BYTES - live - 16_384,
    )
    .unwrap();
    result.validate().unwrap();
    let extra = MAX_OWNED_BYTES - live - r.validation_scratch_bytes().unwrap();
    assert!(matches!(
        boxplot_with_budget(&d, &r, &JobControl::default(), &[], extra),
        Err(AppError::SizeLimit { .. })
    ));
}

#[test]
fn refusal_precedes_sorting_or_group_output_allocation() {
    use std::sync::{atomic::AtomicBool, mpsc::channel, Arc};
    let d = Dataset::new(vec![
        Column::numeric(ColumnId(1), "Value", vec![Some(0.); 10_000]).unwrap(),
        Column::numeric(
            ColumnId(2),
            "Group",
            (0..10_000).map(|i| Some(i as f64)).collect(),
        )
        .unwrap(),
    ])
    .unwrap();
    let r = request(Some(ColumnId(2)));
    let live = Budget::dataset_bytes(&[&d], 0).unwrap();
    for (headroom, sorted) in [(200_000, false), (500_000, true)] {
        let (sender, receiver) = channel();
        let control = JobControl::new(Arc::new(AtomicBool::new(false)), Some(sender));
        assert!(matches!(
            boxplot_with_budget(&d, &r, &control, &[], MAX_OWNED_BYTES - live - headroom),
            Err(AppError::SizeLimit { .. })
        ));
        let progress = receiver.try_iter().collect::<Vec<_>>();
        assert_eq!(
            progress
                .iter()
                .any(|p| p.done >= 20_000 && p.total == Some(50_000)),
            sorted
        );
        assert!(!progress
            .iter()
            .any(|p| p.done >= 30_000 && p.total == Some(50_000)));
    }
    assert_eq!(d.cell(ColumnId(2), 9999).unwrap(), Cell::Number(9999.));
}

#[test]
fn refusal_precedes_point_growth_after_complete_summary() {
    use std::sync::{atomic::AtomicBool, mpsc::channel, Arc};
    let d = dataset(
        (0..5000)
            .map(|i| Some(if i < 4000 { 0. } else { 100. }))
            .collect(),
    );
    let r = request(None);
    let live = Budget::dataset_bytes(&[&d], 0).unwrap();
    let (sender, receiver) = channel();
    let control = JobControl::new(Arc::new(AtomicBool::new(false)), Some(sender));
    assert!(matches!(
        boxplot_with_budget(&d, &r, &control, &[], MAX_OWNED_BYTES - live - 210_000),
        Err(AppError::SizeLimit { .. })
    ));
    let progress = receiver.try_iter().collect::<Vec<_>>();
    assert!(progress
        .iter()
        .any(|p| p.done == 15_000 && p.total == Some(25_000)));
    assert!(!progress
        .iter()
        .any(|p| p.done >= 20_000 && p.total == Some(25_000)));
    assert_eq!(d.cell(ColumnId(1), 4999).unwrap(), Cell::Number(100.));
}

#[test]
fn captured_definitions_request_labels_and_values_survive_edits() {
    let c = Column::text(ColumnId(2), "Original group", vec![Some("ŷ".into()); 5]).unwrap();
    let c = c
        .with_metadata(ColumnMetadata {
            description: "ŵ".repeat(10_000),
            value_labels: vec![ValueLabel {
                value: Cell::Text("ŷ".into()),
                label: "Original label".into(),
            }],
            ..c.metadata().clone()
        })
        .unwrap();
    let d = Dataset::new(vec![
        Column::numeric(
            ColumnId(1),
            "Original outcome",
            [0., 1., 2., 3., 100.].map(Some).to_vec(),
        )
        .unwrap(),
        c,
    ])
    .unwrap();
    let mut r = request(Some(ColumnId(2)));
    r.filter.predicates.push(RowPredicate::Text {
        column: ColumnId(2),
        value: "ŷ".into(),
        equal: true,
    });
    let result = compute(&d, &r);
    let changed = d.edit_cell(ColumnId(1), 4, Cell::Number(200.)).unwrap();
    r.filter.predicates.clear();
    assert_ne!(changed.revision(), result.revision);
    assert_eq!(result.groups[0].maximum.value, Some(100.));
    assert_eq!(result.points[0].value, 100.);
    assert_eq!(result.request.filter.predicates.len(), 1);
    let info = result
        .columns
        .iter()
        .find(|info| info.id == ColumnId(2))
        .unwrap();
    assert_eq!(info.description, "ŵ".repeat(10_000));
    assert_eq!(
        info.value_labels[result.groups[0].value_label_index.unwrap()].label,
        "Original label"
    );
}

#[test]
fn saved_missing_rules_partition_once_and_aggregate_weight_limit_includes_exclusions() {
    let outcome =
        Column::numeric(ColumnId(1), "Outcome", vec![Some(99.), Some(99.), Some(2.)]).unwrap();
    let outcome = outcome
        .with_metadata(ColumnMetadata {
            missing_rules: MissingRules::Number {
                values: vec![99.],
                ranges: vec![],
            },
            ..outcome.metadata().clone()
        })
        .unwrap();
    let group = Column::text(
        ColumnId(2),
        "Group",
        vec![Some("".into()), Some("A".into()), Some("A".into())],
    )
    .unwrap();
    let group = group
        .with_metadata(ColumnMetadata {
            missing_rules: MissingRules::Text(vec!["".into()]),
            ..group.metadata().clone()
        })
        .unwrap();
    let d = Dataset::new(vec![
        outcome,
        group,
        Column::numeric(
            ColumnId(3),
            "Weight",
            vec![Some((MAX_FREQUENCY - 2) as f64), Some(1.), Some(1.)],
        )
        .unwrap(),
    ])
    .unwrap();
    let mut r = request(Some(ColumnId(2)));
    r.frequency_weight = Some(ColumnId(3));
    let result = compute(&d, &r);
    assert_eq!(
        (
            result.missing_group_rows,
            result.missing_outcome_rows,
            result.missing_group_n,
            result.missing_outcome_n
        ),
        (1, 1, MAX_FREQUENCY - 2, 1)
    );
    assert_eq!(result.groups[0].n, 1);
    let excess = d.edit_cell(ColumnId(3), 2, Cell::Number(2.)).unwrap();
    assert!(boxplot_with_budget(&excess, &r, &JobControl::default(), &[], 0).is_err());
}

#[test]
fn public_progress_is_monotonic_across_sort_group_summary_and_points() {
    use std::sync::{atomic::AtomicBool, mpsc::channel, Arc};
    let prefix = "ŵ".repeat(5000);
    let group = Column::text(
        ColumnId(2),
        "Group",
        (0..5000)
            .map(|i| Some(format!("{prefix}{}", i % 2)))
            .collect(),
    )
    .unwrap();
    let group = group
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
            ..group.metadata().clone()
        })
        .unwrap();
    let d = Dataset::new(vec![
        Column::numeric(
            ColumnId(1),
            "Outcome",
            (0..5000)
                .rev()
                .map(|i| Some(if i < 4000 { 0. } else { i as f64 }))
                .collect(),
        )
        .unwrap(),
        group,
    ])
    .unwrap();
    let r = request(Some(ColumnId(2)));
    let (sender, receiver) = channel();
    let control = JobControl::new(Arc::new(AtomicBool::new(false)), Some(sender));
    let result = boxplot_with_budget(&d, &r, &control, &[], 0).unwrap();
    result.validate().unwrap();
    let progress = receiver.try_iter().collect::<Vec<_>>();
    let regression = progress.windows(2).find(|p| p[0].done > p[1].done);
    assert!(
        regression.is_none(),
        "progress moved backward: {regression:?}"
    );
    let leak = progress.iter().find(|p| p.total != Some(25_000));
    assert!(leak.is_none(), "local helper progress leaked: {leak:?}");
    assert_eq!(progress.last().unwrap().done, 25_000);
}

#[test]
fn legacy_descriptives_keep_monotonic_public_progress() {
    use std::sync::{atomic::AtomicBool, mpsc::channel, Arc};
    let d = Dataset::new(vec![
        Column::numeric(ColumnId(1), "X", vec![Some(1.); 5000]).unwrap(),
        Column::numeric(ColumnId(2), "Y", vec![Some(2.); 5000]).unwrap(),
    ])
    .unwrap();
    let r = DescribeRequest {
        columns: vec![ColumnId(1), ColumnId(2)],
        statistics: vec![Statistic::Mean],
        missing_policy: MissingPolicy::PerVariable,
        filter: RowFilter::default(),
        frequency_weight: None,
    };
    let (sender, receiver) = channel();
    let control = JobControl::new(Arc::new(AtomicBool::new(false)), Some(sender));
    describe_with_budget(&d, &r, &control, &[], 0).unwrap();
    let progress = receiver.try_iter().collect::<Vec<_>>();
    assert!(progress.windows(2).all(|p| p[0].done <= p[1].done));
    assert!(progress.iter().all(|p| p.total == Some(35_000)));
    assert_eq!(progress.last().unwrap().done, 35_000);
}

#[test]
fn grouped_means_use_complete_scores_and_frequency_weights() {
    let d = Dataset::new(vec![
        Column::numeric(
            ColumnId(1),
            "Score",
            vec![Some(1.), Some(5.), None, Some(3.), Some(5.)],
        )
        .unwrap(),
        Column::text(
            ColumnId(2),
            "School",
            vec![
                Some("A".into()),
                Some("A".into()),
                Some("A".into()),
                Some("B".into()),
                None,
            ],
        )
        .unwrap(),
        Column::numeric(
            ColumnId(3),
            "Weight",
            vec![Some(1.), Some(3.), Some(8.), Some(2.), Some(9.)],
        )
        .unwrap(),
    ])
    .unwrap();
    let r = BoxplotRequest {
        frequency_weight: Some(ColumnId(3)),
        ..request(Some(ColumnId(2)))
    };
    let result = compute(&d, &r);
    assert_eq!(result.groups[0].mean.value, Some(4.));
    assert_eq!(result.groups[0].n, 4);
    assert_eq!(result.groups[1].mean.value, Some(3.));
    assert_eq!(result.missing_outcome_n, 8);
    assert_eq!(result.missing_group_n, 9);
}

#[test]
fn captured_mean_uses_exact_sum_for_cancellation_and_extreme_values() {
    for (values, expected) in [
        (vec![Some(f64::MAX), Some(f64::MAX)], f64::MAX),
        (vec![Some(-f64::MAX), Some(3.), Some(f64::MAX)], 1.),
    ] {
        let result = compute(&dataset(values), &request(None));
        assert_eq!(result.groups[0].mean.value, Some(expected));
    }
    assert!(compute(&dataset(vec![None]), &request(None)).groups[0]
        .mean
        .value
        .is_none());
}
