use tessera::{
    analysis::*,
    data::{Cell, Column, ColumnId, Dataset, MAX_OWNED_BYTES},
    error::AppError,
    operation::JobControl,
};

fn input() -> (Dataset, RegressionRequest) {
    let data = Dataset::new(vec![
        Column::numeric(
            ColumnId(1),
            "Outcome",
            vec![Some(1.), Some(3.), Some(2.), Some(6.)],
        )
        .unwrap(),
        Column::numeric(
            ColumnId(2),
            "Predictor",
            vec![Some(0.), Some(1.), Some(2.), Some(3.)],
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
    (data, request)
}

#[test]
fn captured_result_is_immutable() {
    let (data, request) = input();
    let captured = regression_with_budget(
        &data,
        &request,
        &JobControl::default(),
        &[],
        request.heap_bytes().unwrap(),
    )
    .unwrap();
    captured.validate().unwrap();
    let expected = captured.clone();
    let old = data.column(ColumnId(1)).unwrap();
    let updated = old
        .with_metadata(tessera::data::ColumnMetadata {
            name: "Changed outcome".into(),
            description: "Current data differ from the captured analysis.".into(),
            ..old.metadata().clone()
        })
        .unwrap();
    let current = Dataset::new(vec![
        updated,
        Column::numeric(
            ColumnId(2),
            "New predictor",
            vec![Some(3.), Some(1.), Some(4.), Some(8.)],
        )
        .unwrap(),
    ])
    .unwrap();
    let newer = regression_with_budget(
        &current,
        &request,
        &JobControl::default(),
        &[&data],
        request.heap_bytes().unwrap(),
    )
    .unwrap();
    newer.validate().unwrap();
    assert_ne!(captured.columns, newer.columns);
    assert_ne!(captured.outcome, newer.outcome);
    assert_eq!(captured, expected);
    captured.validate().unwrap();
    assert_eq!(data.cell(ColumnId(1), 0).unwrap(), Cell::Number(1.));
}

#[test]
fn direct_api_refuses_explicit_live_budget_and_cancellation() {
    let (data, request) = input();
    assert!(matches!(
        regression_with_budget(
            &data,
            &request,
            &JobControl::default(),
            &[&data],
            MAX_OWNED_BYTES
        ),
        Err(AppError::SizeLimit { .. })
    ));
    assert!(matches!(
        regression_with_budget(&data, &request, &JobControl::default(), &[], usize::MAX),
        Err(AppError::SizeLimit { .. })
    ));
    let cancelled = JobControl::default();
    cancelled.cancel();
    assert!(matches!(
        regression_with_budget(&data, &request, &cancelled, &[], 0),
        Err(AppError::Cancelled)
    ));
}

use serde_json::Value;
fn number(v: &Value) -> f64 {
    v.as_f64().unwrap()
}
fn request_case(case: &Value) -> (Dataset, RegressionRequest) {
    let data = &case["input"];
    let x = data["x"].as_array().unwrap();
    let mut cols = vec![Column::numeric(
        ColumnId(1),
        "y",
        data["y"]
            .as_array()
            .unwrap()
            .iter()
            .map(|v| Some(number(v)))
            .collect(),
    )
    .unwrap()];
    let mut predictors = Vec::new();
    let coding = &case["coding"];
    if coding["kind"] == "treatment" {
        let raw = coding["source_rows"].as_array().unwrap();
        for j in 0..4 {
            let id = ColumnId(j as u64 + 2);
            let factor = coding["factors"]
                .as_array()
                .unwrap()
                .iter()
                .find(|f| f["column"] == j);
            let text = raw[0][j].is_string();
            cols.push(if text {
                Column::text(
                    id,
                    &format!("group{j}"),
                    raw.iter()
                        .map(|r| Some(r[j].as_str().unwrap().into()))
                        .collect(),
                )
                .unwrap()
            } else {
                Column::numeric(
                    id,
                    &format!("x{j}"),
                    raw.iter().map(|r| Some(number(&r[j]))).collect(),
                )
                .unwrap()
            });
            predictors.push(RegressionPredictor {
                column: id,
                role: factor.map_or(PredictorRole::Numeric, |f| PredictorRole::Categorical {
                    reference: if text {
                        Cell::Text(f["reference"].as_str().unwrap().into())
                    } else {
                        Cell::Number(number(&f["reference"]))
                    },
                }),
            });
        }
    } else {
        for j in 0..x[0].as_array().unwrap().len() {
            let id = ColumnId(j as u64 + 2);
            cols.push(
                Column::numeric(
                    id,
                    &format!("x{j}"),
                    x.iter().map(|r| Some(number(&r[j]))).collect(),
                )
                .unwrap(),
            );
            predictors.push(RegressionPredictor {
                column: id,
                role: PredictorRole::Numeric,
            });
        }
    }
    let weight = ColumnId(cols.len() as u64 + 1);
    cols.push(
        Column::numeric(
            weight,
            "frequency",
            data["w"]
                .as_array()
                .unwrap()
                .iter()
                .map(|v| Some(v.as_u64().unwrap() as f64))
                .collect(),
        )
        .unwrap(),
    );
    (
        Dataset::new(cols).unwrap(),
        RegressionRequest {
            outcome: ColumnId(1),
            predictors,
            confidence: ConfidenceLevel::P95,
            filter: RowFilter::default(),
            frequency_weight: Some(weight),
        },
    )
}

#[test]
fn all_reference_cohorts_validate_through_the_public_api() {
    let fixtures: Value =
        serde_json::from_str(include_str!("fixtures/regression-fit.json")).unwrap();
    for case in fixtures["cases"].as_array().unwrap() {
        let (dataset, request) = request_case(case);
        for confidence in [
            ConfidenceLevel::P90,
            ConfidenceLevel::P95,
            ConfidenceLevel::P99,
        ] {
            let request = RegressionRequest {
                confidence,
                ..request.clone()
            };
            let result = regression_with_budget(
                &dataset,
                &request,
                &JobControl::default(),
                &[],
                request.heap_bytes().unwrap(),
            );
            if case["policy"]["status"] == "unsupported" {
                assert!(result.is_err());
                continue;
            }
            let result = result.unwrap_or_else(|e| panic!("{}: {e:?}", case["name"]));
            result.validate().unwrap();
            match &result.outcome {
                RegressionOutcome::Fitted(fit) => {
                    assert_eq!(case["policy"]["status"], "fit", "{}", case["name"]);
                    assert_eq!(
                        fit.condition_warned,
                        case["policy"]["condition_warned"].as_bool().unwrap()
                    );
                    assert_eq!(
                        result.frequency,
                        case["reference"]["frequency"].as_u64().unwrap()
                    );
                }
                RegressionOutcome::Unavailable(failure) => {
                    if case["policy"]["status"] == "fit"
                        && matches!(
                            case["name"].as_str(),
                            Some("reference_near_A_3.55e-13" | "reference_near_C_2e-12")
                        )
                    {
                        assert_eq!(
                            failure.kind,
                            tessera::analysis::RegressionFailureKind::Numerical
                        );
                        assert_eq!(failure.reason, "The decomposition does not reconstruct the scaled design within the numerical tolerance.");
                        assert_eq!(failure.rank, None);
                        assert!(failure.expected_terms.is_some());
                        assert!(failure.implicated_terms.is_empty());
                    } else {
                        assert_ne!(case["policy"]["status"], "fit", "{}", case["name"])
                    }
                }
            }
        }
    }
}

#[test]
fn independent_reference_diagnostics_cover_numeric_factors_and_frequency() {
    let fixtures: Value =
        serde_json::from_str(include_str!("fixtures/regression-fit.json")).unwrap();
    let names = [
        "simple",
        "numeric",
        "weighted",
        "expanded",
        "maximum_frequency",
        "two_factors_reference_A",
        "two_factors_reference_B",
        "two_factors_reference_C",
    ];
    for name in names {
        let case = fixtures["cases"]
            .as_array()
            .unwrap()
            .iter()
            .find(|c| c["name"] == name)
            .unwrap();
        let (dataset, request) = request_case(case);
        let result = regression_with_budget(
            &dataset,
            &request,
            &JobControl::default(),
            &[],
            request.heap_bytes().unwrap(),
        )
        .unwrap();
        let RegressionOutcome::Fitted(fit) = &result.outcome else {
            panic!("{name}")
        };
        let reference = &case["reference"]["fit"];
        let variance = number(&reference["variance"]["value"]);
        let n = result.frequency as f64;
        let mut weighted_h = 0.;
        for point in &fit.residual_diagnostics.residual_points {
            let mut x = vec![1.];
            x.extend(
                case["input"]["x"][point.row]
                    .as_array()
                    .unwrap()
                    .iter()
                    .map(number),
            );
            let mut h = 0.;
            for j in 0..x.len() {
                for k in 0..x.len() {
                    h += x[j] * x[k] * number(&reference["covariance"][j][k]["value"]) / variance;
                }
            }
            let e = number(&reference["residual"][point.row]["value"]);
            let prediction = number(&reference["fitted"][point.row]["value"]);
            let studentised = e / (variance * (1. - h)).sqrt();
            let cook = e.powi(2) / variance * h / (x.len() as f64 * (1. - h).powi(2));
            for (field, actual, expected) in [
                ("fitted", &point.fitted, prediction),
                ("residual", &point.residual, e),
                ("leverage", &point.leverage, h),
                ("studentised", &point.studentised, studentised),
                ("Cook", &point.cooks_distance, cook),
            ] {
                let actual = actual
                    .value
                    .unwrap_or_else(|| panic!("{name}/{field} unavailable"));
                assert!(
                    (actual - expected).abs() <= 1e-10 * expected.abs().max(1e-12),
                    "{name}/{field}/row {}: {actual} != {expected}",
                    point.row
                );
            }
            weighted_h += point.frequency as f64 * point.leverage.value.unwrap();
        }
        assert!((weighted_h - fit.rank as f64).abs() < 1e-10, "{name}");
        let sd = (number(&reference["statistics"]["sse"]["value"]) / (n - 1.)).sqrt();
        assert!(
            (fit.residual_diagnostics.residual_sample_sd.value.unwrap() / sd - 1.).abs() < 1e-10,
            "{name}"
        );
    }
}

#[test]
fn retained_datasets_are_charged_and_shared_chunks_are_deduplicated() {
    let (dataset, request) = input();
    let retained = Dataset::new(vec![Column::text(
        ColumnId(20),
        "Retained",
        vec![Some("r".repeat(20000))],
    )
    .unwrap()])
    .unwrap();
    let combined = tessera::data::Budget::dataset_bytes(&[&dataset, &retained], 0).unwrap();
    let extra = MAX_OWNED_BYTES - combined + 1;
    assert!(matches!(
        regression_with_budget(
            &dataset,
            &request,
            &JobControl::default(),
            &[&retained],
            extra
        ),
        Err(AppError::SizeLimit { .. })
    ));
    let a = regression_with_budget(
        &dataset,
        &request,
        &JobControl::default(),
        &[],
        request.heap_bytes().unwrap(),
    )
    .unwrap();
    let b = regression_with_budget(
        &dataset,
        &request,
        &JobControl::default(),
        &[&dataset, &dataset],
        request.heap_bytes().unwrap(),
    )
    .unwrap();
    assert_eq!(a, b);
}
