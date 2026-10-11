use super::*;
fn named(name: &str) -> ComputedRegression {
    let case = reference_cases()
        .into_iter()
        .find(|c| c["name"] == name)
        .unwrap();
    let (data, request) = request_case(&case);
    run(&data, &request)
}
fn failure_input(
    outcomes: Vec<Option<f64>>,
    groups: &[&str],
    reference: &str,
) -> ComputedRegression {
    let data = Dataset::new(vec![
        Column::numeric(ColumnId(1), "Outcome", outcomes).unwrap(),
        Column::text(
            ColumnId(2),
            "Group",
            groups.iter().map(|v| Some((*v).into())).collect(),
        )
        .unwrap(),
    ])
    .unwrap();
    let request = RegressionRequest {
        outcome: ColumnId(1),
        predictors: vec![RegressionPredictor {
            column: ColumnId(2),
            role: PredictorRole::Categorical {
                reference: Cell::Text(reference.into()),
            },
        }],
        confidence: ConfidenceLevel::P95,
        filter: RowFilter::default(),
        frequency_weight: None,
    };
    run(&data, &request)
}
#[test]
fn actual_early_rank_numerical_failures_and_synthetic_nonconvergence_validate() {
    let cases = [
        failure_input(vec![None, None], &["A", "B"], "A"),
        failure_input(vec![Some(1.), Some(2.)], &["A", "B"], "C"),
        failure_input(vec![Some(1.), Some(2.)], &["A", "A"], "A"),
        named("constant_predictor"),
        named("n_lt_p"),
        named("reference_near_B_3.55e-13"),
    ];
    for good in &cases {
        let RegressionOutcome::Unavailable(f) = &good.outcome else {
            panic!("failure capture expected")
        };
        assert_eq!(
            decode_capture(&saved(good).to_string(), MAX_OWNED_BYTES).unwrap(),
            *good
        );
        let mut empty_reason = saved(good);
        empty_reason["outcome"]["unavailable"]["reason"] = json!(" ");
        assert!(decode_capture(&empty_reason.to_string(), MAX_OWNED_BYTES).is_err());
        let mut missing = saved(good);
        missing["outcome"]["unavailable"]
            .as_object_mut()
            .unwrap()
            .remove("rank");
        assert!(decode_capture(&missing.to_string(), MAX_OWNED_BYTES).is_err());
        let mut bad = saved(good);
        bad["outcome"]["unavailable"]["rank"] = json!(good.terms.len() + 1);
        assert!(decode_capture(&bad.to_string(), MAX_OWNED_BYTES).is_err());
        if f.expected_terms.is_none() {
            let mut bad = saved(good);
            bad["outcome"]["unavailable"]["expected_terms"] = json!(good.terms.len());
            assert!(decode_capture(&bad.to_string(), MAX_OWNED_BYTES).is_err());
        }
    }
    // Fixed public512-iteration SVD has no forced nonconvergence seam. This is a
    // retained-state control derived from a real numeric-failure capture, not an API outcome claim.
    let mut synthetic = saved(&cases[5]);
    let f = &mut synthetic["outcome"]["unavailable"];
    f["kind"] = json!("nonconvergence");
    f["reason"] = json!("The bounded decomposition did not converge.");
    f["rank"] = Value::Null;
    f["implicated_terms"] = json!([]);
    assert!(decode_capture(&synthetic.to_string(), MAX_OWNED_BYTES).is_ok());
    for field in ["rank", "implicated_terms"] {
        let mut bad = synthetic.clone();
        bad["outcome"]["unavailable"][field] = if field == "rank" {
            json!(1)
        } else {
            json!([0])
        };
        assert!(decode_capture(&bad.to_string(), MAX_OWNED_BYTES).is_err());
    }
}
#[test]
fn no_df_exact_zero_and_tiny_nonzero_states_reject_false_availability() {
    let no_df = named("n_eq_p");
    for field in ["standard_error", "t", "p", "interval"] {
        let mut bad = saved(&no_df);
        bad["outcome"]["fitted"]["coefficients"][0][field] = if field == "interval" {
            json!([estimate(0.), estimate(1.)])
        } else {
            estimate(0.5)
        };
        assert!(decode_capture(&bad.to_string(), MAX_OWNED_BYTES).is_err());
    }
    for name in ["n_eq_p", "perfect_fit", "zero_outcome"] {
        let good = named(name);
        for field in ["studentised", "cooks_distance"] {
            let mut bad = saved(&good);
            let d = &mut bad["outcome"]["fitted"]["residual_diagnostics"];
            d[field] = json!({"range":[estimate(0.),estimate(0.)],"undefined_rows":0,"undefined_frequency":0});
            for point in d["residual_points"].as_array_mut().unwrap() {
                point[field] = estimate(0.);
            }
            assert!(
                decode_capture(&bad.to_string(), MAX_OWNED_BYTES).is_err(),
                "{name} {field}"
            );
        }
    }
    for name in ["scale_1e-200_frequency_1", "overflow_square_only"] {
        let good = named(name);
        let RegressionOutcome::Fitted(f) = &good.outcome else {
            panic!()
        };
        assert!(f.statistics.sse.value.is_none());
        assert!(f.statistics.residual_standard_error.value.unwrap() > 0.);
        assert!(f.coefficients[0].standard_error.value.unwrap() > 0.);
        assert!(f.residual_diagnostics.residual_points[0]
            .studentised
            .value
            .is_some());
        assert_eq!(
            decode_capture(&saved(&good).to_string(), MAX_OWNED_BYTES).unwrap(),
            good
        );
        let mut bad = saved(&good);
        bad["outcome"]["fitted"]["statistics"]["sse"] = estimate(0.);
        assert!(decode_capture(&bad.to_string(), MAX_OWNED_BYTES).is_err());
    }
}
#[test]
fn condition_limit_rounding_and_saved_energy_envelope_have_boundaries() {
    let ceiling = 1e12 * (1. + 8. * f64::EPSILON);
    for (condition, allowed) in [
        (1e8, true),
        (1e8 * (1. + f64::EPSILON), true),
        (1e12, true),
        (ceiling, true),
        (f64::from_bits(ceiling.to_bits() + 1), false),
        (1e300, false),
    ] {
        let mut value = saved(&ordinary());
        value["outcome"]["fitted"]["condition"] = estimate(condition);
        value["outcome"]["fitted"]["condition_warned"] = json!(condition > 1e8);
        assert_eq!(
            decode_capture(&value.to_string(), MAX_OWNED_BYTES).is_ok(),
            allowed,
            "{condition}"
        );
    }
    // Saved-state validation must not depend on this OS accepting a boundary SVD.
    let fixtures: Vec<Value> = serde_json::from_str(include_str!(
        "../../../../../tests/fixtures/regression-schema-captures.json"
    ))
    .unwrap();
    let fixture = fixtures
        .iter()
        .find(|c| c["name"] == "reference_near_A_3.55e-13" && c["confidence"] == "P95")
        .unwrap();
    let good = decode_capture(&fixture["payload"].to_string(), MAX_OWNED_BYTES).unwrap();
    let RegressionOutcome::Fitted(f) = &good.outcome else {
        panic!()
    };
    let tolerance = 8. * f.rank as f64 * f64::EPSILON * f.condition.value.unwrap();
    let sum = f.statistics.sse.value.unwrap() + f.statistics.model_ss.value.unwrap();
    for (fraction, allowed) in [(0.5, true), (2., false)] {
        let mut value = saved(&good);
        let sst = sum / (1. - fraction * tolerance);
        let s = &mut value["outcome"]["fitted"]["statistics"];
        s["sst"] = estimate(sst);
        s["r_squared"] = estimate(f.statistics.model_ss.value.unwrap() / sst);
        s["adjusted_r_squared"] = estimate(
            1. - f.statistics.sse.value.unwrap() / sst * f.total_df as f64 / f.residual_df as f64,
        );
        value["outcome"]["fitted"]["outcome_scale"] =
            estimate((sst / good.frequency as f64).sqrt());
        assert_eq!(
            decode_capture(&value.to_string(), MAX_OWNED_BYTES).is_ok(),
            allowed,
            "fraction {fraction}"
        );
    }
    for condition in [1., 1e12, 1e300] {
        let mut bad = saved(&named("perfect_fit"));
        bad["outcome"]["fitted"]["condition"] = estimate(condition);
        bad["outcome"]["fitted"]["condition_warned"] = json!(condition > 1e8);
        bad["outcome"]["fitted"]["statistics"]["sst"] = estimate(20.);
        bad["outcome"]["fitted"]["statistics"]["model_ss"] = estimate(200.);
        assert!(decode_capture(&bad.to_string(), MAX_OWNED_BYTES).is_err());
    }
}
#[test]
fn current_metadata_and_types_do_not_reinterpret_guarded_capture() {
    let case = reference_cases()
        .into_iter()
        .find(|c| c["name"] == "simple")
        .unwrap();
    let (data, request) = request_case(&case);
    let captured = run(&data, &request);
    let text = serde_json::to_string(&Payload(captured.clone())).unwrap();
    let current = Dataset::new(vec![
        Column::text(ColumnId(1), "Changed outcome", vec![Some("changed".into())]).unwrap(),
        Column::text(ColumnId(2), "Changed predictor", vec![Some("other".into())]).unwrap(),
    ])
    .unwrap();
    assert!(request
        .validate_current(&current, &[], request.heap_bytes().unwrap())
        .is_err());
    assert_eq!(decode_capture(&text, MAX_OWNED_BYTES).unwrap(), captured);
}

#[test]
fn actual_reduced_and_partial_expansions_roundtrip() {
    let rows = 2001;
    let data = Dataset::new(vec![
        Column::numeric(
            ColumnId(1),
            "Y",
            (0..rows)
                .map(|i| Some(1. + 2. * (i % 31) as f64 + (i % 17) as f64 * 0.1))
                .collect(),
        )
        .unwrap(),
        Column::numeric(
            ColumnId(2),
            "X",
            (0..rows).map(|i| Some((i % 31) as f64)).collect(),
        )
        .unwrap(),
    ])
    .unwrap();
    let request = RegressionRequest {
        outcome: ColumnId(1),
        predictors: vec![RegressionPredictor {
            column: ColumnId(2),
            role: PredictorRole::Numeric,
        }],
        confidence: ConfidenceLevel::P95,
        filter: RowFilter::default(),
        frequency_weight: None,
    };
    let good = run(&data, &request);
    let RegressionOutcome::Fitted(f) = &good.outcome else {
        panic!()
    };
    assert!(f.residual_diagnostics.residual_reduced && f.residual_diagnostics.qq_reduced);
    assert_eq!(
        decode_capture(&saved(&good).to_string(), MAX_OWNED_BYTES).unwrap(),
        good
    );
    for empty in [false, true] {
        let count = if empty { 70 } else { 64 };
        let mut columns = vec![Column::numeric(
            ColumnId(1),
            "Outcome",
            if empty {
                vec![None; 2]
            } else {
                vec![Some(1.), Some(2.)]
            },
        )
        .unwrap()];
        let mut predictors = Vec::new();
        for i in 0..count {
            let id = ColumnId(i as u64 + 2);
            columns.push(
                Column::text(
                    id,
                    &format!("Factor{i}"),
                    if i == 63 {
                        vec![Some("A".into()); 2]
                    } else {
                        vec![Some("A".into()), Some("B".into())]
                    },
                )
                .unwrap(),
            );
            predictors.push(RegressionPredictor {
                column: id,
                role: PredictorRole::Categorical {
                    reference: Cell::Text("A".into()),
                },
            });
        }
        let data = Dataset::new(columns).unwrap();
        let request = RegressionRequest {
            predictors,
            ..request.clone()
        };
        let good = run(&data, &request);
        if empty {
            assert!(good.levels.is_empty());
            assert_eq!(good.request.predictors.len(), 70);
        } else {
            assert_eq!(good.levels.len(), 64);
            assert_eq!(good.terms.len(), 64);
        }
        assert_eq!(
            decode_capture(&saved(&good).to_string(), MAX_OWNED_BYTES).unwrap(),
            good
        );
    }
}

#[test]
fn two_physical_rows_with_literal_replicates_preserve_fit_boundary() {
    let mut failures = Vec::new();
    for weights in [[1., 2.], [2., 3.], [1., 17.]] {
        for categorical in [false, true] {
            let predictor = if categorical {
                Column::text(
                    ColumnId(2),
                    "Group",
                    vec![Some("A".into()), Some("B".into())],
                )
                .unwrap()
            } else {
                Column::numeric(ColumnId(2), "X", vec![Some(0.), Some(1.)]).unwrap()
            };
            let data = Dataset::new(vec![
                Column::numeric(ColumnId(1), "Y", vec![Some(1.), Some(3.)]).unwrap(),
                predictor,
                Column::numeric(ColumnId(3), "Weight", weights.map(Some).to_vec()).unwrap(),
            ])
            .unwrap();
            let request = RegressionRequest {
                outcome: ColumnId(1),
                predictors: vec![RegressionPredictor {
                    column: ColumnId(2),
                    role: if categorical {
                        PredictorRole::Categorical {
                            reference: Cell::Text("A".into()),
                        }
                    } else {
                        PredictorRole::Numeric
                    },
                }],
                confidence: ConfidenceLevel::P95,
                filter: RowFilter::default(),
                frequency_weight: Some(ColumnId(3)),
            };
            let result = regression_with_budget(
                &data,
                &request,
                &JobControl::default(),
                &[],
                request.heap_bytes().unwrap(),
            );
            println!(
                "TWO_ROWS {:?} categorical={} outcome={:?}",
                weights, categorical, result
            );
            match result {
                Ok(good) => assert_eq!(
                    decode_capture(&saved(&good).to_string(), MAX_OWNED_BYTES).unwrap(),
                    good
                ),
                Err(error) => failures.push(format!(
                    "weights={weights:?} categorical={categorical} error={error:?}"
                )),
            }
        }
    }
    assert!(failures.is_empty(), "two-row boundaries: {failures:?}");
}

#[test]
fn validation_scratch_and_nested_guard_cancellation_use_the_same_live_budget() {
    let good = named("maximum_frequency");
    let input = saved(&good).to_string();
    let charged = std::cell::Cell::new(0);
    let parsed: Payload = guarded_checked(&input, MAX_OWNED_BYTES, |p: &Payload, state| {
        charged.set(state.borrow().bytes);
        state.borrow_mut().add(p.0.validation_scratch_bytes()?)?;
        p.0.validate()
    })
    .unwrap();
    assert_eq!(parsed.0, good);
    let admitted = charged.get() + good.validation_scratch_bytes().unwrap();
    assert!(decode_capture(&input, admitted).is_ok());
    assert!(matches!(
        decode_capture(&input, admitted - 1),
        Err(AppError::SizeLimit { .. })
    ));
    let control = JobControl::default();
    control.cancel();
    let state = RefCell::new(Accounting {
        bytes: input.len()
            + crate::project_io::decode::check_input(input.as_bytes())
                .unwrap()
                .max(3 * MAX_DEPTH)
            + size_of::<Payload>(),
        control: &control,
        error: None,
        steps: 0,
    });
    let mut json = serde_json::Deserializer::from_str(&input);
    assert!(Payload::deserialize(Guard {
        inner: &mut json,
        state: &state,
        depth: 0
    })
    .is_err());
    assert!(matches!(state.borrow().error, Some(AppError::Cancelled)));
}
