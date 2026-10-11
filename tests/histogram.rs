use tessera::{analysis::*, data::*, error::AppError, operation::JobControl};

fn request(bins: HistogramBins) -> HistogramRequest {
    HistogramRequest {
        variable: ColumnId(1),
        bins,
        filter: RowFilter::default(),
        frequency_weight: None,
    }
}
fn dataset(values: Vec<Option<f64>>) -> Dataset {
    Dataset::new(vec![Column::numeric(ColumnId(1), "Value", values).unwrap()]).unwrap()
}
fn compute(d: &Dataset, r: &HistogramRequest) -> ComputedHistogram {
    let result =
        histogram_with_budget(d, r, &JobControl::default(), &[], r.heap_bytes().unwrap()).unwrap();
    result.validate().unwrap();
    result
}
#[test]
fn independent_exact_fraction_boundaries_and_counts() {
    // Portable fixture: Python Fraction on exact input bits, independently generated before this engine.
    let reference: serde_json::Value =
        serde_json::from_str(include_str!("fixtures/histogram-reference.json")).unwrap();
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
        let mut r = request(HistogramBins::Count(
            case["requested_bins"].as_u64().unwrap() as u16,
        ));
        r.frequency_weight = Some(ColumnId(2));
        let actual = compute(&d, &r);
        assert_eq!(actual.n, case["n"].as_u64().unwrap(), "{case}");
        assert_eq!(
            actual.bins.len(),
            case["actual_bins"].as_u64().unwrap() as usize,
            "{case}"
        );
        for (bin, expected) in actual.bins.iter().zip(case["bins"].as_array().unwrap()) {
            // Zero ULP tolerance, including signed-zero identity.
            assert_eq!(
                bin.left.to_bits(),
                expected["left_bits"].as_u64().unwrap(),
                "{case}"
            );
            assert_eq!(
                bin.right.to_bits(),
                expected["right_bits"].as_u64().unwrap(),
                "{case}"
            );
            assert_eq!(
                bin.rows,
                expected["rows"].as_u64().unwrap() as usize,
                "{case}"
            );
            assert_eq!(
                bin.frequency,
                expected["frequency"].as_u64().unwrap(),
                "{case}"
            );
        }
        let adjustment = actual.valid_rows > 0
            && actual.range[0].value != actual.range[1].value
            && actual.bins.len() < case["requested_bins"].as_u64().unwrap() as usize;
        assert_eq!(actual.bin_adjustment.is_some(), adjustment, "{case}");
    }
}
#[test]
fn automatic_bins_use_exact_integer_frequency_around_powers() {
    for power in [1, 20, 52, 53] {
        let n = 1_u64 << power;
        for total in [n - 1, n, n + u64::from(power < 53)] {
            let d = Dataset::new(vec![
                Column::numeric(ColumnId(1), "Value", vec![Some(0.), Some(1.)]).unwrap(),
                Column::numeric(
                    ColumnId(2),
                    "Weight",
                    vec![Some(1.), Some((total - 1) as f64)],
                )
                .unwrap(),
            ])
            .unwrap();
            let mut r = request(HistogramBins::Automatic);
            r.frequency_weight = Some(ColumnId(2));
            let actual = compute(&d, &r);
            let expected = if total == 1 {
                1
            } else {
                (1 + u64::BITS - (total - 1).leading_zeros()) as usize
            };
            assert_eq!(actual.bins.len(), expected);
            assert_eq!(actual.n, total);
            assert_eq!(actual.bins.iter().map(|b| b.frequency).sum::<u64>(), total);
        }
    }
}
#[test]
fn complete_cohort_missing_partition_and_histogram_cannot_sample() {
    let d = dataset((0..10_001).map(|v| Some(v as f64)).chain([None]).collect());
    let result = compute(&d, &request(HistogramBins::Count(2)));
    assert_eq!(
        (
            result.valid_rows,
            result.n,
            result.missing_rows,
            result.missing_n
        ),
        (10_001, 10_001, 1, 1)
    );
    assert_eq!(result.diagnostics.complete_case_rows, 0);
    assert_eq!((result.bins[0].rows, result.bins[1].rows), (5000, 5001));
    assert_eq!(
        [result.range[0].value, result.range[1].value],
        [Some(0.), Some(10_000.)]
    );
    for values in [vec![], vec![None, None]] {
        let result = compute(&dataset(values), &request(HistogramBins::Automatic));
        assert!(result.bins.is_empty() && result.range.iter().all(|e| e.value.is_none()));
    }
    let result = compute(
        &dataset(vec![Some(5.); 3000]),
        &request(HistogramBins::Count(128)),
    );
    assert_eq!(
        (
            result.bins.len(),
            result.bins[0].rows,
            result.bins[0].frequency
        ),
        (1, 3000, 3000)
    );
}
#[test]
fn weights_are_checked_before_missingness_and_after_filter() {
    let d = Dataset::new(vec![
        Column::numeric(
            ColumnId(1),
            "Value",
            vec![None, Some(1.), None, Some(2.), None],
        )
        .unwrap(),
        Column::numeric(
            ColumnId(2),
            "Weight",
            vec![Some(-1.), Some(2.), Some(3.), Some(0.), None],
        )
        .unwrap(),
    ])
    .unwrap();
    let mut r = request(HistogramBins::Count(1));
    r.frequency_weight = Some(ColumnId(2));
    assert!(
        histogram_with_budget(&d, &r, &JobControl::default(), &[], r.heap_bytes().unwrap())
            .is_err()
    );
    r.filter.predicates.push(RowPredicate::Number {
        column: ColumnId(2),
        comparison: NumericComparison::GreaterOrEqual,
        value: 0.,
    });
    let result = compute(&d, &r);
    assert_eq!((result.n, result.missing_rows, result.missing_n), (2, 1, 3));
    assert_eq!(
        (
            result.diagnostics.filtered_rows,
            result.diagnostics.zero_weight_rows
        ),
        (2, 1)
    );
}
#[test]
fn invalid_settings_cancellation_and_retained_budget_refuse() {
    let d = dataset(vec![Some(1.), Some(2.)]);
    for bins in [HistogramBins::Count(0), HistogramBins::Count(129)] {
        assert!(histogram_with_budget(&d, &request(bins), &JobControl::default(), &[], 0).is_err());
    }
    let r = request(HistogramBins::Automatic);
    let control = JobControl::default();
    control.cancel();
    assert!(matches!(
        histogram_with_budget(&d, &r, &control, &[], 0),
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
        histogram_with_budget(
            &d,
            &r,
            &JobControl::default(),
            &[&retained],
            MAX_OWNED_BYTES - 1_000_000
        ),
        Err(AppError::SizeLimit { .. })
    ));
}
#[test]
fn caller_extra_is_charged_once_and_output_growth_is_precharged() {
    let d = dataset(vec![Some(0.), Some(4.)]);
    let r = request(HistogramBins::Count(128));
    let live = Budget::dataset_bytes(&[&d], 0).unwrap();
    let result = histogram_with_budget(
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
        histogram_with_budget(&d, &r, &JobControl::default(), &[], extra),
        Err(AppError::SizeLimit { .. })
    ));
}
#[test]
fn captured_definitions_and_request_survive_data_edits() {
    let column = Column::numeric(ColumnId(1), "Original", vec![Some(0.), Some(2.)]).unwrap();
    let column = column
        .with_metadata(ColumnMetadata {
            description: "ŷ".repeat(10_000),
            value_labels: vec![ValueLabel {
                value: Cell::Number(2.),
                label: "Two".into(),
            }],
            ..column.metadata().clone()
        })
        .unwrap();
    let d = Dataset::new(vec![column]).unwrap();
    let mut r = request(HistogramBins::Count(2));
    r.filter.predicates.push(RowPredicate::Number {
        column: ColumnId(1),
        comparison: NumericComparison::GreaterOrEqual,
        value: 0.,
    });
    let result = compute(&d, &r);
    let changed = d.edit_cell(ColumnId(1), 1, Cell::Number(100.)).unwrap();
    r.filter.predicates.clear();
    assert_ne!(result.revision, changed.revision());
    assert_eq!(result.range[1].value, Some(2.));
    assert_eq!(result.request.filter.predicates.len(), 1);
    assert_eq!(result.columns[0].description, "ŷ".repeat(10_000));
    assert_eq!(result.columns[0].value_labels[0].label, "Two");
}
