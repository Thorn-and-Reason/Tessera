use super::*;
type Mutation = (&'static str, fn(&mut Value));
fn require_refusal(good: &ComputedRegression, mutations: &[Mutation]) {
    let mut accepted = Vec::new();
    for (name, mutate) in mutations {
        let mut bad = saved(good);
        mutate(&mut bad);
        if decode_capture(&bad.to_string(), MAX_OWNED_BYTES).is_ok() {
            accepted.push(*name);
        }
    }
    assert!(
        accepted.is_empty(),
        "accepted captured mutations: {accepted:?}"
    );
}
#[test]
fn captured_numerical_relationships_reject_independent_mutations() {
    require_refusal(
        &ordinary(),
        &[
            ("SSE", |p| {
                p["outcome"]["fitted"]["statistics"]["sse"] = estimate(2.)
            }),
            ("SST", |p| {
                p["outcome"]["fitted"]["statistics"]["sst"] = estimate(8.)
            }),
            ("model SS", |p| {
                p["outcome"]["fitted"]["statistics"]["model_ss"] = estimate(8.)
            }),
            ("spread", |p| {
                p["outcome"]["fitted"]["statistics"]["residual_standard_error"] = estimate(2.)
            }),
            ("R squared", |p| {
                p["outcome"]["fitted"]["statistics"]["r_squared"] = estimate(0.5)
            }),
            ("adjusted R squared", |p| {
                p["outcome"]["fitted"]["statistics"]["adjusted_r_squared"] = estimate(0.5)
            }),
            ("F", |p| {
                p["outcome"]["fitted"]["statistics"]["f"] = estimate(5.)
            }),
            ("SE", |p| {
                p["outcome"]["fitted"]["coefficients"][0]["standard_error"] = estimate(2.)
            }),
            ("t", |p| {
                p["outcome"]["fitted"]["coefficients"][0]["t"] = estimate(2.)
            }),
            ("CI centre", |p| {
                for e in p["outcome"]["fitted"]["coefficients"][0]["interval"]
                    .as_array_mut()
                    .unwrap()
                {
                    e["value"] = json!(number(&e["value"]) + 0.05);
                }
            }),
            ("outcome scale", |p| {
                p["outcome"]["fitted"]["outcome_scale"] = estimate(2.)
            }),
            ("intercept scale", |p| {
                p["outcome"]["fitted"]["term_scales"][0] = estimate(2.)
            }),
            ("residual sample SD", |p| {
                p["outcome"]["fitted"]["residual_diagnostics"]["residual_sample_sd"] = estimate(2.)
            }),
            ("residual mean outside range", |p| {
                p["outcome"]["fitted"]["residual_diagnostics"]["residual_mean"] = estimate(100.)
            }),
        ],
    );
}
#[test]
fn captured_request_metadata_cohort_and_fit_shapes_reject_mutations() {
    require_refusal(
        &ordinary(),
        &[
            ("duplicate predictor", |p| {
                p["request"]["predictors"]
                    .as_array_mut()
                    .unwrap()
                    .push(json!({"column":"2","role":"numeric"}))
            }),
            ("outcome predictor", |p| {
                p["request"]["predictors"][0]["column"] = json!("1")
            }),
            ("absent predictor metadata", |p| {
                p["request"]["predictors"][0]["column"] = json!("999")
            }),
            ("role metadata", |p| p["columns"][1]["kind"] = json!("text")),
            ("captured column order", |p| {
                p["columns"].as_array_mut().unwrap().swap(0, 1)
            }),
            ("missing filter metadata", |p| {
                p["request"]["filter"]["predicates"] =
                    json!([{"number":{"column":"999","comparison":"equal","value":0}}])
            }),
            ("missing weight metadata", |p| {
                p["request"]["frequency_weight"] = json!("999")
            }),
            ("term order", |p| {
                p["terms"].as_array_mut().unwrap().reverse()
            }),
            ("term duplicate", |p| p["terms"][1] = json!("intercept")),
            ("term column", |p| {
                p["terms"][1]["numeric"]["column"] = json!("1")
            }),
            ("term count", |p| {
                p["terms"].as_array_mut().unwrap().pop();
            }),
            ("M", |p| p["fitted_rows"] = json!(3)),
            ("N", |p| p["frequency"] = json!(3)),
            ("source partition", |p| p["source_rows"] = json!(5)),
            ("incomplete frequency", |p| {
                p["incomplete_case_frequency"] = json!(1)
            }),
            ("base frequency bound", |p| {
                p["incomplete_case_frequency"] = json!(9007199254740992_u64)
            }),
            ("rank", |p| p["outcome"]["fitted"]["rank"] = json!(1)),
            ("model df", |p| {
                p["outcome"]["fitted"]["model_df"] = json!(2)
            }),
            ("residual df", |p| {
                p["outcome"]["fitted"]["residual_df"] = json!(3)
            }),
            ("total df", |p| {
                p["outcome"]["fitted"]["total_df"] = json!(2)
            }),
            ("coefficient count", |p| {
                p["outcome"]["fitted"]["coefficients"]
                    .as_array_mut()
                    .unwrap()
                    .pop();
            }),
            ("scale count", |p| {
                p["outcome"]["fitted"]["term_scales"]
                    .as_array_mut()
                    .unwrap()
                    .pop();
            }),
            ("zero scale", |p| {
                p["outcome"]["fitted"]["term_scales"][1] = estimate(0.)
            }),
            ("condition", |p| {
                p["outcome"]["fitted"]["condition"] = estimate(0.5)
            }),
            ("warning", |p| {
                p["outcome"]["fitted"]["condition_warned"] = json!(true)
            }),
            ("numerical version", |p| {
                p["method"]["numerical_version"] = json!(2)
            }),
            ("rank threshold", |p| {
                p["method"]["rank_threshold"] = json!(1e-10)
            }),
            ("warning threshold", |p| {
                p["method"]["condition_warning"] = json!(1e12)
            }),
            ("empty reason", |p| {
                p["outcome"]["fitted"]["statistics"]["sse"] = json!({"value":null,"reason":" "})
            }),
            ("malformed estimate", |p| {
                p["outcome"]["fitted"]["statistics"]["sse"] =
                    json!({"value":1,"reason":"contradiction"})
            }),
        ],
    );
}
#[test]
fn captured_diagnostic_identities_extents_and_reduction_reject_mutations() {
    require_refusal(
        &ordinary(),
        &[
            ("residual cardinality", |p| {
                p["outcome"]["fitted"]["residual_diagnostics"]["residual_points"]
                    .as_array_mut()
                    .unwrap()
                    .pop();
            }),
            ("QQ cardinality", |p| {
                p["outcome"]["fitted"]["residual_diagnostics"]["qq_points"]
                    .as_array_mut()
                    .unwrap()
                    .pop();
            }),
            ("cohort ordinal", |p| {
                p["outcome"]["fitted"]["residual_diagnostics"]["residual_points"][0]
                    ["cohort_ordinal"] = json!(1)
            }),
            ("source row", |p| {
                p["outcome"]["fitted"]["residual_diagnostics"]["residual_points"][0]["row"] =
                    json!(4)
            }),
            ("source order", |p| {
                p["outcome"]["fitted"]["residual_diagnostics"]["residual_points"][1]["row"] =
                    json!(0)
            }),
            ("frequency", |p| {
                p["outcome"]["fitted"]["residual_diagnostics"]["residual_points"][0]["frequency"] =
                    json!(9)
            }),
            ("residual reduction", |p| {
                p["outcome"]["fitted"]["residual_diagnostics"]["residual_reduced"] = json!(true)
            }),
            ("QQ reduction", |p| {
                p["outcome"]["fitted"]["residual_diagnostics"]["qq_reduced"] = json!(true)
            }),
            ("QQ rank", |p| {
                p["outcome"]["fitted"]["residual_diagnostics"]["qq_points"][0]["rank"] = json!(1)
            }),
            ("QQ row", |p| {
                p["outcome"]["fitted"]["residual_diagnostics"]["qq_points"][0]["row"] = json!(999)
            }),
            ("QQ frequency", |p| {
                p["outcome"]["fitted"]["residual_diagnostics"]["qq_points"][0]["frequency"] =
                    json!(3)
            }),
            ("QQ order", |p| {
                p["outcome"]["fitted"]["residual_diagnostics"]["qq_points"]
                    .as_array_mut()
                    .unwrap()
                    .reverse()
            }),
            ("QQ theoretical range", |p| {
                p["outcome"]["fitted"]["residual_diagnostics"]["theoretical_range"][0] =
                    estimate(-99.)
            }),
            ("fitted range", |p| {
                p["outcome"]["fitted"]["residual_diagnostics"]["fitted_range"][1] = estimate(1.)
            }),
            ("raw residual range", |p| {
                p["outcome"]["fitted"]["residual_diagnostics"]["residual_range"][1] = estimate(0.)
            }),
            ("undefined rows", |p| {
                p["outcome"]["fitted"]["residual_diagnostics"]["leverage"]["undefined_rows"] =
                    json!(1)
            }),
            ("undefined frequency", |p| {
                p["outcome"]["fitted"]["residual_diagnostics"]["studentised"]
                    ["undefined_frequency"] = json!(1)
            }),
            ("Cook extent", |p| {
                p["outcome"]["fitted"]["residual_diagnostics"]["cooks_distance"]["range"][1] =
                    estimate(999.)
            }),
        ],
    );
}
#[test]
fn captured_factors_preserve_exact_order_reference_and_numeric_zero() {
    let case = reference_cases()
        .into_iter()
        .find(|c| c["name"] == "two_factors_reference_A")
        .unwrap();
    let (data, request) = request_case(&case);
    let good = run(&data, &request);
    require_refusal(
        &good,
        &[
            ("duplicate levels", |p| {
                p["levels"][0]["values"][1] = p["levels"][0]["values"][0].clone()
            }),
            ("level order", |p| {
                p["levels"][0]["values"].as_array_mut().unwrap().reverse()
            }),
            ("level column order", |p| {
                p["levels"].as_array_mut().unwrap().reverse()
            }),
            ("missing reference", |p| {
                p["request"]["predictors"][2]["role"]["categorical"]["reference"] = json!("absent")
            }),
            ("term reference", |p| {
                p["terms"][3]["contrast"]["reference"] = json!("absent")
            }),
            ("term level", |p| {
                p["terms"][3]["contrast"]["level"] = json!("absent")
            }),
            ("reference kind", |p| {
                p["request"]["predictors"][2]["role"]["categorical"]["reference"] = json!(1)
            }),
        ],
    );
    let data = Dataset::new(vec![
        Column::numeric(
            ColumnId(1),
            "Y",
            vec![Some(0.), Some(1.), Some(2.), Some(4.)],
        )
        .unwrap(),
        Column::numeric(
            ColumnId(2),
            "C",
            vec![Some(-0.), Some(1.), Some(0.), Some(1.)],
        )
        .unwrap(),
    ])
    .unwrap();
    let request = RegressionRequest {
        outcome: ColumnId(1),
        predictors: vec![RegressionPredictor {
            column: ColumnId(2),
            role: PredictorRole::Categorical {
                reference: Cell::Number(-0.),
            },
        }],
        confidence: ConfidenceLevel::P95,
        filter: RowFilter::default(),
        frequency_weight: None,
    };
    let good = run(&data, &request);
    let mut bad = saved(&good);
    bad["levels"][0]["values"][0] = json!(-0.0);
    assert!(decode_capture(&bad.to_string(), MAX_OWNED_BYTES).is_err());
}

#[test]
fn qq_ranks_cannot_span_more_copies_than_their_source_frequency() {
    let case = reference_cases()
        .into_iter()
        .find(|c| c["name"] == "weighted")
        .unwrap();
    let (data, request) = request_case(&case);
    let good = run(&data, &request);
    let RegressionOutcome::Fitted(f) = &good.outcome else {
        panic!()
    };
    let qq = &f.residual_diagnostics.qq_points;
    let start = (1..qq.len())
        .find(|&i| qq[i].frequency > 1 && qq[i].row != qq[i - 1].row)
        .unwrap();
    let mut bad = saved(&good);
    for field in ["row", "frequency", "residual"] {
        bad["outcome"]["fitted"]["residual_diagnostics"]["qq_points"][start - 1][field] =
            bad["outcome"]["fitted"]["residual_diagnostics"]["qq_points"][start][field].clone();
    }
    assert!(decode_capture(&bad.to_string(), MAX_OWNED_BYTES).is_err());
}

#[test]
fn unavailable_qq_coordinates_cannot_interleave_a_physical_sources_copies() {
    let data = Dataset::new(vec![
        Column::numeric(
            ColumnId(1),
            "Y",
            vec![Some(1.), Some(2.), Some(2.), Some(4.)],
        )
        .unwrap(),
        Column::numeric(
            ColumnId(2),
            "X",
            vec![Some(0.), Some(1.), Some(2.), Some(3.)],
        )
        .unwrap(),
        Column::numeric(
            ColumnId(3),
            "W",
            vec![
                Some(1_000_000.),
                Some(1.),
                Some(1_000_000.),
                Some(1_000_000.),
            ],
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
        frequency_weight: Some(ColumnId(3)),
    };
    let good = run(&data, &request);
    let RegressionOutcome::Fitted(f) = &good.outcome else {
        panic!()
    };
    assert!(f.residual_diagnostics.qq_points.iter().all(|p| p.row != 1));
    let mut hidden = saved(&good);
    let d = &mut hidden["outcome"]["fitted"]["residual_diagnostics"];
    // Mutation projection preserves independent raw-coordinate unavailable states;
    // it does not claim these unavailable values were emitted by this ordinary API fit.
    d["residual_range"] = json!([absent(), absent()]);
    for point in d["residual_points"].as_array_mut().unwrap() {
        point["residual"] = absent();
    }
    for point in d["qq_points"].as_array_mut().unwrap() {
        point["residual"] = absent();
    }
    assert!(decode_capture(&hidden.to_string(), MAX_OWNED_BYTES).is_ok());
    let d = &mut hidden["outcome"]["fitted"]["residual_diagnostics"];
    let start = (1..1999)
        .find(|&i| {
            d["qq_points"][i - 1]["row"] == d["qq_points"][i + 1]["row"]
                && d["qq_points"][i]["row"] == d["qq_points"][i - 1]["row"]
        })
        .unwrap();
    d["qq_points"][start]["row"] = json!(1);
    d["qq_points"][start]["frequency"] = json!(1);
    assert!(decode_capture(&hidden.to_string(), MAX_OWNED_BYTES).is_err());
}
