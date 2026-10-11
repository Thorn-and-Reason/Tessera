use super::*;

// Catches accepting invalid roles or mutating the supplied recipe during validation.
#[test]
fn invalid_roles_leave_input_unchanged() {
    let data = dataset();
    let cases = [
        RegressionRequest {
            outcome: ColumnId(3),
            ..numeric_request()
        },
        RegressionRequest {
            predictors: vec![],
            ..numeric_request()
        },
        RegressionRequest {
            predictors: vec![RegressionPredictor {
                column: ColumnId(1),
                role: PredictorRole::Numeric,
            }],
            ..numeric_request()
        },
        RegressionRequest {
            predictors: vec![numeric_request().predictors[0].clone(); 2],
            ..numeric_request()
        },
        RegressionRequest {
            predictors: vec![RegressionPredictor {
                column: ColumnId(3),
                role: PredictorRole::Numeric,
            }],
            ..numeric_request()
        },
        category_request(Cell::Missing),
        category_request(Cell::Number(1.)),
        category_request(Cell::Number(f64::INFINITY)),
        RegressionRequest {
            outcome: ColumnId(0),
            ..numeric_request()
        },
        RegressionRequest {
            outcome: ColumnId(999),
            ..numeric_request()
        },
    ];
    for request in cases {
        let before = request.clone();
        assert!(request
            .validate_current(&data, &[], request.heap_bytes().unwrap())
            .is_err());
        assert_eq!(request, before);
    }
    let request = numeric_request();
    assert!(request
        .validate_current(&data, &[], request.heap_bytes().unwrap())
        .is_ok());
}

// Catches validating history against edited live metadata or omitting filter/weight roles.
#[test]
fn metadata_snapshot_is_frozen() {
    let data = dataset();
    let request = category_request(Cell::Text("absent".into()));
    let result = captured(&data, request);
    assert!(result.validate().is_ok());
    let other = Dataset::new(vec![Column::text(
        ColumnId(1),
        "Changed outcome",
        vec![Some("x".into())],
    )
    .unwrap()])
    .unwrap();
    assert!(result.request.validate_current(&other, &[], 0).is_err());
    assert_eq!(result.columns[0].name, "Y");
    assert!(result.validate().is_ok());
    let mut broken = result.clone();
    broken.columns.pop();
    assert!(broken.validate().is_err());
    let mut broken = result;
    broken.columns[0].kind = ColumnKind::Text;
    assert!(broken.validate().is_err());
}

// Catches cardinality/df/rank drift and scalar values with absent or contradictory reasons.
#[test]
fn captured_fit_rejects_broken_state_without_live_data() {
    let good = fitted_fixture();
    good.validate().unwrap();
    for mutation in 0..9 {
        let mut bad = good.clone();
        let RegressionOutcome::Fitted(fit) = &mut bad.outcome else {
            unreachable!()
        };
        match mutation {
            0 => {
                fit.coefficients.pop();
            }
            1 => {
                fit.term_scales.pop();
            }
            2 => fit.rank = 1,
            3 => fit.residual_df = 3,
            4 => fit.coefficients[0].estimate.value = Some(f64::INFINITY),
            5 => fit.coefficients[0].estimate.reason = Some("contradiction".into()),
            6 => fit.coefficients[0].p.reason = Some(" ".into()),
            7 => bad.source_rows = 5,
            8 => bad.incomplete_case_frequency = 1,
            _ => unreachable!(),
        }
        assert!(bad.validate().is_err(), "mutation {mutation}");
    }
}

// Catches fabricated decomposition facts in early unavailable outcomes.
#[test]
fn unavailable_states_keep_only_known_facts() {
    let data = dataset();
    let result = captured(&data, category_request(Cell::Text("absent".into())));
    let RegressionOutcome::Unavailable(failure) = &result.outcome else {
        unreachable!()
    };
    assert_eq!(
        failure.kind,
        RegressionFailureKind::AbsentReference {
            column: ColumnId(3)
        }
    );
    assert_eq!(failure.expected_terms, None);
    assert_eq!(failure.rank, None);
    assert_eq!(result.terms, vec![RegressionTerm::Intercept]);
    result.validate().unwrap();
    let mut bad = result.clone();
    let RegressionOutcome::Unavailable(failure) = &mut bad.outcome else {
        unreachable!()
    };
    failure.rank = Some(0);
    assert!(bad.validate().is_err());
}

// Catches omitting weight/filter metadata or treating a value label as a reference.
#[test]
fn metadata_roles_include_filter_and_weight_and_references_remain_typed() {
    let data = dataset();
    let request = RegressionRequest {
        frequency_weight: Some(ColumnId(2)),
        filter: RowFilter {
            predicates: vec![RowPredicate::Number {
                column: ColumnId(2),
                comparison: NumericComparison::Greater,
                value: 0.,
            }],
        },
        ..category_request(Cell::Text("absent".into()))
    };
    let result = captured(&data, request);
    assert_eq!(
        result.columns.iter().map(|c| c.id.0).collect::<Vec<_>>(),
        vec![1, 2, 3]
    );
    result.validate().unwrap();
    let mut bad = result;
    bad.columns.remove(1);
    assert!(bad.validate().is_err());
    let request = RegressionRequest {
        predictors: vec![RegressionPredictor {
            column: ColumnId(2),
            role: PredictorRole::Categorical {
                reference: Cell::Number(f64::NAN),
            },
        }],
        ..numeric_request()
    };
    assert!(request.validate_current(&data, &[], 0).is_err());
}

// Catches accepting malformed ordering/counts in deterministic saved diagnostics.
#[test]
fn saved_diagnostics_reject_identity_and_extent_corruption() {
    let good = fitted_fixture();
    for mutation in 0..10 {
        let mut bad = good.clone();
        let RegressionOutcome::Fitted(fit) = &mut bad.outcome else {
            unreachable!()
        };
        let d = &mut fit.residual_diagnostics;
        match mutation {
            0 => d.residual_points[1].row = 0,
            1 => d.residual_points[1].cohort_ordinal = 2,
            2 => d.residual_points[0].frequency = 2,
            3 => d.qq_points[1].rank = 0,
            4 => d.qq_points[0].frequency = 2,
            5 => d.studentised.undefined_rows = 3,
            6 => d.leverage.range[0] = available(-0.1),
            7 => d.residual_points[0].leverage = available(-0.1),
            8 => d.fitted_range[0] = available(2.),
            9 => d.residual_range[1] = available(1.),
            _ => unreachable!(),
        }
        assert!(bad.validate().is_err(), "mutation {mutation}");
    }
}

// Catches accepting missing numerical statuses or forged method interpretations.
#[test]
fn failure_and_method_metadata_have_coherent_status() {
    let good = fitted_fixture();
    for mutation in 0..8 {
        let mut bad = good.clone();
        match mutation {
            0 => bad.method.numerical_version = 2,
            1 => bad.method.rank_threshold = f64::NAN,
            2 => bad.method.condition_warning = 0.,
            3 => {
                let RegressionOutcome::Fitted(fit) = &mut bad.outcome else {
                    unreachable!()
                };
                fit.condition_warned = true;
            }
            4 => {
                bad.outcome = RegressionOutcome::Unavailable(RegressionFailure {
                    kind: RegressionFailureKind::Nonconvergence,
                    reason: "Not converged".into(),
                    expected_terms: Some(2),
                    rank: Some(2),
                    implicated_terms: vec![],
                })
            }
            5 => {
                bad.outcome = RegressionOutcome::Unavailable(RegressionFailure {
                    kind: RegressionFailureKind::Unidentified,
                    reason: "Not identified".into(),
                    expected_terms: Some(2),
                    rank: Some(2),
                    implicated_terms: vec![],
                })
            }
            6 => {
                bad.outcome = RegressionOutcome::Unavailable(RegressionFailure {
                    kind: RegressionFailureKind::AbsentReference {
                        column: ColumnId(999),
                    },
                    reason: "Absent".into(),
                    expected_terms: None,
                    rank: None,
                    implicated_terms: vec![],
                })
            }
            7 => {
                bad.outcome = RegressionOutcome::Unavailable(RegressionFailure {
                    kind: RegressionFailureKind::NoContrast {
                        column: ColumnId(2),
                    },
                    reason: "No contrast".into(),
                    expected_terms: None,
                    rank: None,
                    implicated_terms: vec![],
                })
            }
            _ => unreachable!(),
        }
        assert!(bad.validate().is_err(), "mutation {mutation}");
    }
    let good = ComputedRegression {
        outcome: RegressionOutcome::Unavailable(RegressionFailure {
            kind: RegressionFailureKind::Nonconvergence,
            reason: "Not converged".into(),
            expected_terms: Some(2),
            rank: None,
            implicated_terms: vec![],
        }),
        ..good
    };
    good.validate().unwrap();
}

fn nondegenerate_fixture() -> ComputedRegression {
    let data = Dataset::new(vec![
        Column::numeric(
            ColumnId(1),
            "Y",
            vec![Some(1.), Some(3.), Some(2.), Some(6.)],
        )
        .unwrap(),
        Column::numeric(
            ColumnId(2),
            "X",
            vec![Some(0.), Some(1.), Some(2.), Some(3.)],
        )
        .unwrap(),
    ])
    .unwrap();
    let request = numeric_request();
    regression_with_budget(
        &data,
        &request,
        &JobControl::default(),
        &[],
        request.heap_bytes().unwrap(),
    )
    .unwrap()
}

// Catches accepting zero/invalid available probabilities instead of an explicit range reason.
#[test]
fn regression_probabilities_are_positive_finite_or_reasoned_unavailable() {
    let mut good = nondegenerate_fixture();
    for value in [0., -1., 1.1, f64::NAN, f64::INFINITY] {
        for omnibus in [false, true] {
            let mut bad = good.clone();
            let RegressionOutcome::Fitted(fit) = &mut bad.outcome else {
                unreachable!()
            };
            if omnibus {
                fit.statistics.f_p = available(value);
            } else {
                fit.coefficients[0].p = available(value);
            }
            assert!(
                bad.validate().is_err(),
                "probability {value}, omnibus {omnibus}"
            );
        }
    }
    let RegressionOutcome::Fitted(fit) = &mut good.outcome else {
        unreachable!()
    };
    fit.coefficients[0].p = Estimate {
        value: None,
        reason: Some("A nonzero estimate is below Float64 range.".into()),
    };
    fit.statistics.f_p = fit.coefficients[0].p.clone();
    good.validate().unwrap();
}

// Catches treating full-rank N=p as unidentified or pretending residual variance is known.
#[test]
fn full_rank_without_residual_df_has_coefficients_and_unavailable_inference() {
    let data = Dataset::new(vec![
        Column::numeric(ColumnId(1), "Y", vec![Some(1.), Some(3.)]).unwrap(),
        Column::numeric(ColumnId(2), "X", vec![Some(0.), Some(1.)]).unwrap(),
    ])
    .unwrap();
    let request = numeric_request();
    let mut good = regression_with_budget(
        &data,
        &request,
        &JobControl::default(),
        &[],
        request.heap_bytes().unwrap(),
    )
    .unwrap();
    good.validate().unwrap();
    let RegressionOutcome::Fitted(fit) = &mut good.outcome else {
        unreachable!()
    };
    fit.coefficients[0].standard_error = available(0.);
    assert!(good.validate().is_err());
}

// Actual weighted fit: row 1 has frequency 3, is omitted from residual marks,
// and remains the full-cohort residual maximum and Q–Q endpoint.
fn reduced_weighted_fixture() -> ComputedRegression {
    let data = Dataset::new(vec![
        Column::numeric(
            ColumnId(1),
            "Y",
            (0..4000)
                .map(|i| Some(1. + 2. * i as f64 + if i == 1 { 1000. } else { 0. }))
                .collect(),
        )
        .unwrap(),
        Column::numeric(
            ColumnId(2),
            "X",
            (0..4000).map(|i| Some(i as f64)).collect(),
        )
        .unwrap(),
        Column::numeric(
            ColumnId(4),
            "Weight",
            (0..4000)
                .map(|i| Some(if i == 1 { 3. } else { 1. }))
                .collect(),
        )
        .unwrap(),
    ])
    .unwrap();
    let request = RegressionRequest {
        frequency_weight: Some(ColumnId(4)),
        ..numeric_request()
    };
    regression_with_budget(
        &data,
        &request,
        &JobControl::default(),
        &[],
        request.heap_bytes().unwrap(),
    )
    .unwrap()
}

// I1: a fully retained residual payload establishes the entire fitted membership.
#[test]
fn full_cohort_qq_rejects_excluded_source_row() {
    let mut result = fitted_fixture();
    result.source_rows = 5;
    result.diagnostics.filtered_rows = 1;
    result.validate().unwrap();
    let RegressionOutcome::Fitted(fit) = &mut result.outcome else {
        unreachable!()
    };
    fit.residual_diagnostics.qq_points[3].row = 4;
    assert!(result.validate().is_err());
}

// I2: finite full-cohort points establish both extrema, even if a forged reason is supplied.
#[test]
fn finite_full_cohort_requires_both_saved_extrema() {
    for fitted in [false, true] {
        for mask in [1, 2, 3] {
            let mut bad = fitted_fixture();
            let RegressionOutcome::Fitted(fit) = &mut bad.outcome else {
                unreachable!()
            };
            let d = &mut fit.residual_diagnostics;
            let range = if fitted {
                &mut d.fitted_range
            } else {
                &mut d.residual_range
            };
            for (bound, estimate) in range.iter_mut().enumerate() {
                if mask & (1 << bound) != 0 {
                    *estimate = Estimate {
                        value: None,
                        reason: Some("The estimate is outside finite Float64 range.".into()),
                    };
                }
            }
            assert!(
                bad.validate().is_err(),
                "fitted {fitted}, missing bound mask {mask}"
            );
        }
    }
}

// Preserve independent unavailable raw values and valid reduced rows absent from the display.
#[test]
fn incomplete_extrema_evidence_and_reduced_membership_remain_valid() {
    for fitted in [false, true] {
        let mut result = nondegenerate_fixture();
        let RegressionOutcome::Fitted(fit) = &mut result.outcome else {
            unreachable!()
        };
        let d = &mut fit.residual_diagnostics;
        let absent = Estimate {
            value: None,
            reason: Some("The estimate is outside finite Float64 range.".into()),
        };
        if fitted {
            d.residual_points[0].fitted = absent.clone();
            d.fitted_range[0] = absent;
        } else {
            d.residual_points[0].residual = absent.clone();
            for point in d.qq_points.iter_mut().filter(|p| p.row == 0) {
                point.residual = absent.clone();
            }
            d.residual_range[0] = absent;
        }
        result.validate().unwrap();
    }
    let mut result = reduced_weighted_fixture();
    result.validate().unwrap();
    let RegressionOutcome::Fitted(fit) = &mut result.outcome else {
        unreachable!()
    };
    let d = &mut fit.residual_diagnostics;
    assert!(d.residual_points.iter().all(|p| p.row != 1));
    assert!(d.qq_points.iter().any(|p| p.row == 1 && p.frequency == 3));
    d.fitted_range = [unavailable(), unavailable()];
    d.residual_range = [unavailable(), unavailable()];
    result.validate().unwrap();
}

// I4: N=4,002 and M=4,000 leave at most 3 copies for any single physical row.
#[test]
fn unseen_qq_row_frequency_cannot_consume_other_fitted_rows() {
    let good = reduced_weighted_fixture();
    good.validate().unwrap();
    for frequency in [4, 4003] {
        let mut bad = good.clone();
        let RegressionOutcome::Fitted(fit) = &mut bad.outcome else {
            unreachable!()
        };
        let point = fit
            .residual_diagnostics
            .qq_points
            .iter_mut()
            .find(|p| p.row == 1)
            .unwrap();
        point.frequency = frequency;
        assert!(bad.validate().is_err(), "unseen row frequency {frequency}");
    }
}
