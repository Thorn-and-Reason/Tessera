use super::*;

// Catches completeness preceding invalid weight/base-frequency validation.
#[test]
fn weight_precedence_and_base_bound() {
    let data = Dataset::new(vec![
        Column::numeric(
            ColumnId(1),
            "Y",
            vec![None, Some(2.), Some(3.), None, Some(5.)],
        )
        .unwrap(),
        Column::numeric(ColumnId(2), "X", vec![Some(1.); 5]).unwrap(),
        Column::numeric(
            ColumnId(3),
            "W",
            vec![Some(-1.), Some(0.), None, Some(3.), Some(2.)],
        )
        .unwrap(),
    ])
    .unwrap();
    let request = RegressionRequest {
        frequency_weight: Some(ColumnId(3)),
        ..numeric_request()
    };
    assert!(select_rows(
        &data,
        &request.filter,
        request.frequency_weight,
        &[ColumnId(1), ColumnId(2)],
        &JobControl::default(),
        5
    )
    .is_err());
    let request = RegressionRequest {
        filter: RowFilter {
            predicates: vec![RowPredicate::Number {
                column: ColumnId(3),
                comparison: NumericComparison::GreaterOrEqual,
                value: 0.,
            }],
        },
        ..request
    };
    let (rows, d) = selected(&data, &request);
    assert_eq!(
        (
            d.filtered_rows,
            d.missing_weight_rows,
            d.zero_weight_rows,
            d.complete_case_rows
        ),
        (2, 0, 1, 1)
    );
    assert_eq!(rows.iter().filter(|r| r.complete).count(), 1);
    assert_eq!(
        rows.iter()
            .filter(|r| r.complete)
            .map(|r| r.weight)
            .sum::<u64>(),
        2
    );
    assert_eq!(
        rows.iter()
            .filter(|r| !r.complete)
            .map(|r| r.weight)
            .sum::<u64>(),
        3
    );
    let data = Dataset::new(vec![
        Column::numeric(ColumnId(1), "Y", vec![None, Some(2.)]).unwrap(),
        Column::numeric(ColumnId(2), "X", vec![Some(1.); 2]).unwrap(),
        Column::numeric(ColumnId(3), "W", vec![Some((1_u64 << 53) as f64), Some(1.)]).unwrap(),
    ])
    .unwrap();
    assert!(select_rows(
        &data,
        &RowFilter::default(),
        Some(ColumnId(3)),
        &[ColumnId(1), ColumnId(2)],
        &JobControl::default(),
        2
    )
    .is_err());
    let (_, d) = select_rows(
        &dataset(),
        &RowFilter::default(),
        None,
        &[ColumnId(1)],
        &JobControl::default(),
        4,
    )
    .unwrap();
    assert_eq!(d, RunDiagnostics::default());
}

// Catches reference substitution after exclusions and silently omitting one-level factors.
#[test]
fn reference_is_final_cohort_identity() {
    let data = Dataset::new(vec![
        Column::numeric(ColumnId(1), "Y", vec![None, Some(2.), Some(3.)]).unwrap(),
        Column::text(
            ColumnId(3),
            "G",
            ["ref", "b", "c"].map(|v| Some(v.into())).to_vec(),
        )
        .unwrap(),
    ])
    .unwrap();
    let result = captured(&data, category_request(Cell::Text("ref".into())));
    assert_eq!(
        result.levels[0].values,
        vec![Cell::Text("b".into()), Cell::Text("c".into())]
    );
    assert!(matches!(
        result.outcome,
        RegressionOutcome::Unavailable(RegressionFailure {
            kind: RegressionFailureKind::AbsentReference {
                column: ColumnId(3)
            },
            ..
        })
    ));
    result.validate().unwrap();
    let request = RegressionRequest {
        filter: RowFilter {
            predicates: vec![RowPredicate::Text {
                column: ColumnId(3),
                value: "b".into(),
                equal: true,
            }],
        },
        ..category_request(Cell::Text("b".into()))
    };
    let result = captured(&data, request);
    assert!(matches!(
        result.outcome,
        RegressionOutcome::Unavailable(RegressionFailure {
            kind: RegressionFailureKind::NoContrast { .. },
            ..
        })
    ));
    result.validate().unwrap();
}

// Catches labels replacing identity, noncanonical zero and byte-insensitive text ordering.
#[test]
fn signed_zero_and_text_levels() {
    let column = Column::numeric(
        ColumnId(2),
        "Codes",
        vec![Some(-0.), Some(0.), Some(2.), Some(-1.)],
    )
    .unwrap();
    let column = column
        .with_metadata(crate::data::ColumnMetadata {
            value_labels: vec![
                ValueLabel {
                    value: Cell::Number(0.),
                    label: "Same".into(),
                },
                ValueLabel {
                    value: Cell::Number(2.),
                    label: "Same".into(),
                },
            ],
            ..column.metadata().clone()
        })
        .unwrap();
    let data = Dataset::new(vec![
        Column::numeric(ColumnId(1), "Y", vec![Some(1.); 4]).unwrap(),
        column,
    ])
    .unwrap();
    let request = RegressionRequest {
        predictors: vec![RegressionPredictor {
            column: ColumnId(2),
            role: PredictorRole::Categorical {
                reference: Cell::Number(-0.),
            },
        }],
        ..numeric_request()
    };
    let (rows, _) = selected(&data, &request);
    let (levels, terms, failure) = coding::discover(
        &data,
        &request,
        &rows,
        &JobControl::default(),
        coding_baseline(&data, &request, &rows),
    )
    .unwrap();
    assert!(failure.is_none());
    assert_eq!(
        levels[0].values,
        vec![Cell::Number(-1.), Cell::Number(0.), Cell::Number(2.)]
    );
    assert!(matches!(levels[0].values[1], Cell::Number(v) if v.to_bits() == 0));
    assert_eq!(terms.len(), 3);
    let data = Dataset::new(vec![
        Column::numeric(ColumnId(1), "Y", vec![Some(1.); 6]).unwrap(),
        Column::text(
            ColumnId(3),
            "G",
            ["a", "A", "a ", "", " ", "é"]
                .map(|v| Some(v.into()))
                .to_vec(),
        )
        .unwrap(),
    ])
    .unwrap();
    let request = category_request(Cell::Text("".into()));
    let (rows, _) = selected(&data, &request);
    let (levels, terms, failure) = coding::discover(
        &data,
        &request,
        &rows,
        &JobControl::default(),
        coding_baseline(&data, &request, &rows),
    )
    .unwrap();
    assert!(failure.is_none());
    assert_eq!(
        levels[0].values,
        ["", " ", "A", "a", "a ", "é"]
            .map(|v| Cell::Text(v.into()))
            .to_vec()
    );
    assert_eq!(terms.len(), 6);
}

// Catches an unnecessary ban on weight columns in model roles.
#[test]
fn model_weight_overlap() {
    let data = dataset();
    let request = RegressionRequest {
        frequency_weight: Some(ColumnId(2)),
        ..numeric_request()
    };
    request
        .validate_current(&data, &[], request.heap_bytes().unwrap())
        .unwrap();
    let (rows, d) = selected(&data, &request);
    assert_eq!(d.zero_weight_rows, 1);
    assert_eq!(rows.len(), 3);
    assert_eq!(rows.iter().map(|r| r.weight).sum::<u64>(), 6);
}

// Catches cancellation only at row boundaries when a row has thousands of completeness roles.
#[test]
fn wide_complete_column_cancel() {
    let data = dataset();
    let columns = vec![ColumnId(1); 20_000];
    let control = JobControl::default();
    let cancel = control.clone();
    crate::operation::CHECKPOINT_HOOK.with(|hook| {
        *hook.borrow_mut() = Some(Box::new(move |p| {
            if p.total.is_none() && p.done >= 4096 {
                cancel.cancel();
            }
        }))
    });
    let result = select_rows(&data, &RowFilter::default(), None, &columns, &control, 4);
    crate::operation::CHECKPOINT_HOOK.with(|hook| *hook.borrow_mut() = None);
    assert!(matches!(result, Err(AppError::Cancelled)));
}

// Catches truncating categorical expansion rather than operationally refusing it.
#[test]
fn oversized_model_refuses_without_partial_coding() {
    let data = Dataset::new(vec![
        Column::numeric(ColumnId(1), "Y", vec![Some(1.); 65]).unwrap(),
        Column::text(
            ColumnId(3),
            "G",
            (0..65).map(|i| Some(format!("level {i:03}"))).collect(),
        )
        .unwrap(),
    ])
    .unwrap();
    let request = category_request(Cell::Text("level 000".into()));
    let (rows, _) = selected(&data, &request);
    let error = coding::discover(
        &data,
        &request,
        &rows,
        &JobControl::default(),
        coding_baseline(&data, &request, &rows),
    )
    .unwrap_err();
    assert!(
        matches!(error, AppError::InvalidInput { location, message } if location == "regression model" && message.contains("64"))
    );
}

// Catches skipping dataset-defined missing codes or conflating physical rows with frequency.
#[test]
fn missing_rules_zero_and_missing_weights_partition_source_rows() {
    let outcome = Column::numeric(
        ColumnId(1),
        "Y",
        vec![Some(-99.), Some(2.), Some(3.), Some(4.), Some(5.)],
    )
    .unwrap();
    let outcome = outcome
        .with_metadata(crate::data::ColumnMetadata {
            missing_rules: crate::data::MissingRules::Number {
                values: vec![-99.],
                ranges: vec![],
            },
            ..outcome.metadata().clone()
        })
        .unwrap();
    let data = Dataset::new(vec![
        outcome,
        Column::numeric(ColumnId(2), "X", vec![Some(1.); 5]).unwrap(),
        Column::numeric(
            ColumnId(3),
            "W",
            vec![Some(3.), None, Some(0.), Some(2.), Some(4.)],
        )
        .unwrap(),
    ])
    .unwrap();
    let request = RegressionRequest {
        frequency_weight: Some(ColumnId(3)),
        ..numeric_request()
    };
    let (rows, d) = selected(&data, &request);
    assert_eq!(
        (
            d.filtered_rows,
            d.missing_weight_rows,
            d.zero_weight_rows,
            d.complete_case_rows
        ),
        (0, 1, 1, 1)
    );
    assert_eq!(rows.iter().filter(|r| r.complete).count(), 2);
    assert_eq!(
        rows.iter()
            .filter(|r| r.complete)
            .map(|r| r.weight)
            .sum::<u64>(),
        6
    );
    assert_eq!(
        rows.iter()
            .filter(|r| !r.complete)
            .map(|r| r.weight)
            .sum::<u64>(),
        3
    );
}

// Catches empty-cohort states inventing categorical levels or decomposition facts.
#[test]
fn empty_complete_cohort_has_no_discovered_levels_or_rank() {
    let data = Dataset::new(vec![
        Column::numeric(ColumnId(1), "Y", vec![None; 2]).unwrap(),
        Column::text(ColumnId(3), "G", vec![Some("a".into()); 2]).unwrap(),
    ])
    .unwrap();
    let result = captured(&data, category_request(Cell::Text("a".into())));
    assert_eq!(
        (
            result.fitted_rows,
            result.frequency,
            result.incomplete_case_frequency
        ),
        (0, 0, 2)
    );
    assert!(result.levels.is_empty());
    assert_eq!(result.terms, vec![RegressionTerm::Intercept]);
    result.validate().unwrap();
}

// Catches ignoring a numeric predictor's known term when categorical expansion reaches the ceiling.
#[test]
fn numeric_terms_count_toward_categorical_ceiling_before_text_copy() {
    let data = Dataset::new(vec![
        Column::numeric(ColumnId(1), "Y", vec![Some(1.); 64]).unwrap(),
        Column::numeric(ColumnId(2), "X", vec![Some(1.); 64]).unwrap(),
        Column::text(
            ColumnId(3),
            "G",
            (0..64)
                .map(|i| Some(format!("{i:03}{}", "x".repeat(20_000))))
                .collect(),
        )
        .unwrap(),
    ])
    .unwrap();
    let request = RegressionRequest {
        predictors: vec![
            RegressionPredictor {
                column: ColumnId(3),
                role: PredictorRole::Categorical {
                    reference: Cell::Text(format!("000{}", "x".repeat(20_000))),
                },
            },
            RegressionPredictor {
                column: ColumnId(2),
                role: PredictorRole::Numeric,
            },
        ],
        ..numeric_request()
    };
    let (rows, _) = selected(&data, &request);
    let control = JobControl::default();
    let cancel = control.clone();
    crate::operation::CHECKPOINT_HOOK.with(|hook| {
        *hook.borrow_mut() = Some(Box::new(move |p| {
            if p.done >= 8192 {
                cancel.cancel();
            }
        }))
    });
    let result = coding::discover(
        &data,
        &request,
        &rows,
        &control,
        coding_baseline(&data, &request, &rows),
    );
    crate::operation::CHECKPOINT_HOOK.with(|hook| *hook.borrow_mut() = None);
    assert!(
        matches!(result, Err(AppError::InvalidInput { location, .. }) if location == "regression model")
    );
}

// Catches refusing by raw predictor count when a one-level factor has no expansion.
#[test]
fn one_level_factor_at_the_term_ceiling_keeps_its_actual_failure() {
    let mut columns = vec![Column::numeric(ColumnId(1), "Y", vec![Some(1.)]).unwrap()];
    columns.extend(
        (2..=64)
            .map(|id| Column::numeric(ColumnId(id), &format!("X{id}"), vec![Some(1.)]).unwrap()),
    );
    columns.push(Column::text(ColumnId(65), "G", vec![Some("a".into())]).unwrap());
    let data = Dataset::new(columns).unwrap();
    let mut predictors = vec![RegressionPredictor {
        column: ColumnId(65),
        role: PredictorRole::Categorical {
            reference: Cell::Text("a".into()),
        },
    }];
    predictors.extend((2..=64).map(|id| RegressionPredictor {
        column: ColumnId(id),
        role: PredictorRole::Numeric,
    }));
    let request = RegressionRequest {
        predictors,
        ..numeric_request()
    };
    let (rows, _) = selected(&data, &request);
    let (_, _, failure) = coding::discover(
        &data,
        &request,
        &rows,
        &JobControl::default(),
        coding_baseline(&data, &request, &rows),
    )
    .unwrap();
    assert!(matches!(
        failure,
        Some(RegressionFailure {
            kind: RegressionFailureKind::NoContrast {
                column: ColumnId(65)
            },
            ..
        })
    ));
    captured(&data, request).validate().unwrap();
}

// Catches letting 64 numeric predictors grow the 64-term buffer past its charged capacity.
#[test]
fn genuine_numeric_expansion_refuses_before_coding_growth() {
    let data = Dataset::new(
        (1..=65)
            .map(|id| Column::numeric(ColumnId(id), &format!("N{id}"), vec![Some(1.)]).unwrap())
            .collect(),
    )
    .unwrap();
    let request = RegressionRequest {
        predictors: (2..=65)
            .map(|id| RegressionPredictor {
                column: ColumnId(id),
                role: PredictorRole::Numeric,
            })
            .collect(),
        ..numeric_request()
    };
    let (rows, _) = selected(&data, &request);
    let result = coding::discover(
        &data,
        &request,
        &rows,
        &JobControl::default(),
        coding_baseline(&data, &request, &rows),
    );
    assert!(
        matches!(result, Err(AppError::InvalidInput { location, message }) if location == "regression model" && message.contains("64"))
    );
}
