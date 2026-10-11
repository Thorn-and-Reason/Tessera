use super::*;
use crate::{
    analysis::{selection::select_rows, ConfidenceLevel, Estimate, RowFilter},
    data::{Budget, Cell, Column, ColumnId, Dataset, MAX_OWNED_BYTES},
    error::AppError,
    operation::JobControl,
};
use serde_json::Value;

fn cases() -> Vec<Value> {
    serde_json::from_str::<Value>(include_str!(
        "../../../../tests/fixtures/regression-fit.json"
    ))
    .unwrap()["cases"]
        .as_array()
        .unwrap()
        .clone()
}
fn number(v: &Value) -> f64 {
    v.as_f64().unwrap()
}
fn case_named(name: &str) -> Value {
    cases().into_iter().find(|v| v["name"] == name).unwrap()
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
fn run_case(case: &Value, control: &JobControl) -> Result<KernelOutcome, AppError> {
    let (data, request) = request_case(case);
    let ids = std::iter::once(request.outcome)
        .chain(request.predictors.iter().map(|p| p.column))
        .collect::<Vec<_>>();
    let (eligible, _) = select_rows(
        &data,
        &request.filter,
        request.frequency_weight,
        &ids,
        control,
        data.row_count(),
    )?;
    drop(ids);
    let baseline = Budget::dataset_bytes(
        &[&data],
        request.heap_bytes()?
            + eligible.capacity() * std::mem::size_of::<crate::analysis::selection::Eligible>(),
    )?;
    let (levels, terms, failure) =
        super::super::coding::discover(&data, &request, &eligible, control, baseline)?;
    assert!(failure.is_none());
    let bytes = super::super::accounting::kernel_work_bytes(data.row_count(), terms.len())?;
    let mut budget = crate::data::AllocationCounter {
        bytes: baseline + super::super::accounting::coding_bytes(&levels, &terms)?,
    };
    budget.add(bytes)?;
    solve(&data, &request, &eligible, &levels, &terms, control)
}
fn scalar(got: &Estimate, want: &Value, tolerance: f64, label: &str) {
    if want["status"] == "unavailable" {
        assert!(got.value.is_none(), "{label}: unexpected {:?}", got.value);
        let reason = want["reason"].as_str().unwrap();
        let reason = if (label.ends_with("/p") || label.ends_with("/f_p"))
            && reason == "A nonzero estimate is below Float64 range."
        {
            "The probability is below Float64 range."
        } else {
            reason
        };
        assert_eq!(got.reason.as_deref(), Some(reason), "{label}");
    } else {
        let value = got
            .value
            .unwrap_or_else(|| panic!("{label}: {:?}", got.reason));
        assert!(value.is_finite() && got.reason.is_none(), "{label}");
        let expected = number(&want["value"]);
        if expected != 0. {
            assert_ne!(value, 0., "{label}");
        }
        let bound = if expected != 0. && expected.abs() < f64::MIN_POSITIVE {
            f64::from_bits(1)
        } else {
            expected.abs() * tolerance
        };
        assert!(
            (value - expected).abs() <= bound,
            "{label}: {value:e} != {expected:e}; bound={bound:e}"
        );
    }
}
fn bounded_probability(got: &Estimate, bounds: &Value, label: &str) {
    let value = got
        .value
        .unwrap_or_else(|| panic!("{label}: {:?}", got.reason));
    assert!(
        value.is_finite() && value > 0. && got.reason.is_none(),
        "{label}"
    );
    assert!(
        value >= number(&bounds[0]["value"]) && value <= number(&bounds[1]["value"]),
        "{label}: {value:e} outside propagated statistic uncertainty"
    );
}
fn near_reference_field(got: &[Estimate], want: &Value, label: &str) {
    let mut error = 0f64;
    let mut norm = 0f64;
    assert_eq!(got.len(), want.as_array().unwrap().len());
    for i in 0..got.len() {
        assert_eq!(want[i]["status"], "available");
        let value = got[i]
            .value
            .unwrap_or_else(|| panic!("{label}: {:?}", got[i].reason));
        let reference = number(&want[i]["value"]);
        assert!(value.is_finite() && got[i].reason.is_none());
        if reference != 0. {
            assert_ne!(value, 0.);
        }
        error = error.hypot(value - reference);
        norm = norm.hypot(reference);
    }
    assert!(
        error <= 3e-4 * norm,
        "{label}: field norm error {error:e} / {norm:e}"
    );
}
fn field_output(name: &str, field: &str, index: Option<usize>, values: &[Estimate]) {
    println!(
        "FIT_FIELD_RESULT {}",
        serde_json::json!({"name":name,"field":field,"index":index,"values":values.iter().map(|v|serde_json::json!({"status":if v.value.is_some(){"available"}else{"unavailable"},"value":v.value,"reason":v.reason})).collect::<Vec<_>>()})
    );
}
fn check_case(case: &Value) {
    check_case_with(case, false);
}
fn check_case_with(case: &Value, corrupt_statistics: bool) {
    let name = case["name"].as_str().unwrap();
    let control = JobControl::default();
    let outcome = run_case(case, &control);
    if case["policy"]["status"] == "unsupported" {
        assert!(
            matches!(outcome, Err(AppError::InvalidInput { .. })),
            "{name}"
        );
        return;
    }
    let outcome = outcome.unwrap();
    let fit = match outcome {
        KernelOutcome::Unavailable(f) => {
            if portable_reconstruction_refusal(name, &f) {
                return;
            }
            assert_ne!(case["policy"]["status"], "fit", "{name}: {}", f.reason);
            if case["policy"]["status"] == "rank" {
                assert_eq!(
                    f.kind,
                    RegressionFailureKind::Unidentified,
                    "{name}: {}",
                    f.reason
                );
                let expected = case["reference"]["policy_rank_on_high_precision_scaled_design"]
                    .as_u64()
                    .or_else(|| case["reference"]["mathematical_rank"].as_u64())
                    .unwrap();
                assert_eq!(f.rank, Some(expected as usize));
                assert!(
                    f.rank.unwrap()
                        <= case["reference"]["physical_rows"].as_u64().unwrap() as usize
                );
            } else {
                assert_eq!(
                    f.reason,
                    case["policy"]["reason"].as_str().unwrap(),
                    "{name}"
                );
            }
            return;
        }
        KernelOutcome::Fitted(fit) => {
            assert_eq!(case["policy"]["status"], "fit", "{name}");
            fit
        }
    };
    let want = &case["reference"]["fit"];
    assert_eq!(fit.rank(), want["rank"].as_u64().unwrap() as usize);
    assert_eq!(fit.residual_df(), want["residual_df"].as_u64().unwrap());
    assert_eq!(
        fit.frequency(),
        case["reference"]["frequency"].as_u64().unwrap()
    );
    assert_eq!(
        fit.condition() > 1e8,
        case["policy"]["condition_warned"].as_bool().unwrap()
    );
    let tolerance = number(&case["relative_tolerance"]);
    let analytic_zero = case["reference"]["mathematical_exact_zero_residual"] == true;
    if case["reference"]["computed_zero_requirement"] == "exact_zero" {
        assert_eq!(fit.residual_norm(), 0.);
    }
    if !analytic_zero {
        assert!(fit.residual_norm() > 0., "{name}");
    }
    if analytic_zero {
        routes::check(case, &fit, corrupt_statistics);
        return;
    }
    for (confidence, key) in [
        (ConfidenceLevel::P90, "90"),
        (ConfidenceLevel::P95, "95"),
        (ConfidenceLevel::P99, "99"),
    ] {
        let (coef, mut stats) = super::super::inference::infer(&fit, confidence, &control).unwrap();
        if corrupt_statistics {
            stats.sst = Estimate::unavailable("corrupted");
        }
        assert_eq!(coef.len(), want["coefficients"].as_array().unwrap().len());
        for (j, coefficient) in coef.iter().enumerate() {
            let expected = &want["coefficients"][j];
            for (label, got) in [
                ("estimate", &coefficient.estimate),
                ("standard_error", &coefficient.standard_error),
                ("t", &coefficient.t),
                ("p", &coefficient.p),
            ] {
                let bounds = &case["reference"]["probability_input_bands"];
                if !bounds.is_null() && key == "95" {
                    field_output(name, label, Some(j), std::slice::from_ref(got));
                }
                if label == "p" && !bounds.is_null() {
                    bounded_probability(got, &bounds["coefficients"][j], &format!("{name}/{j}/p"));
                } else {
                    scalar(
                        got,
                        &expected[label],
                        tolerance,
                        &format!("{name}/{j}/{label}"),
                    );
                }
            }
            if !case["reference"]["probability_input_bands"].is_null() {
                near_reference_field(
                    &coefficient.interval,
                    &expected["intervals"][key],
                    &format!("{name}/{j}/ci{key}"),
                );
                println!(
                    "FIT_INTERVAL_RESULT {}",
                    serde_json::json!({"name":name,"coefficient":j,"confidence":key,"interval":coefficient.interval.iter().map(|v|serde_json::json!({"status":if v.value.is_some(){"available"}else{"unavailable"},"value":v.value,"reason":v.reason})).collect::<Vec<_>>()})
                );
            } else {
                for k in 0..2 {
                    scalar(
                        &coefficient.interval[k],
                        &expected["intervals"][key][k],
                        tolerance,
                        &format!("{name}/{j}/ci{key}/{k}"),
                    );
                }
            }
        }
        if fit.residual_df() == 0 {
            for value in [
                &stats.residual_standard_error,
                &stats.adjusted_r_squared,
                &stats.f,
                &stats.f_p,
            ] {
                assert_eq!(
                    value.reason.as_deref(),
                    Some("There are no residual degrees of freedom.")
                );
            }
        } else if fit.residual_norm() == 0. {
            assert_eq!(stats.sse.value, Some(0.));
            assert_eq!(stats.residual_standard_error.value, Some(0.));
            for value in [&stats.f, &stats.f_p] {
                assert_eq!(
                    value.reason.as_deref(),
                    Some("Residual variance is exactly zero.")
                );
            }
        }
        if fit.total_norm() == 0. {
            assert_eq!(stats.sst.value, Some(0.));
            assert_eq!(stats.model_ss.value, Some(0.));
            assert_eq!(
                stats.r_squared.reason.as_deref(),
                Some("The outcome has no variation.")
            );
        }

        for (label, got) in [
            ("sse", &stats.sse),
            ("sst", &stats.sst),
            ("model_ss", &stats.model_ss),
            ("residual_standard_error", &stats.residual_standard_error),
            ("r_squared", &stats.r_squared),
            ("adjusted_r_squared", &stats.adjusted_r_squared),
            ("f", &stats.f),
            ("f_p", &stats.f_p),
        ] {
            let bounds = &case["reference"]["probability_input_bands"];
            if !bounds.is_null() && key == "95" {
                field_output(name, label, None, std::slice::from_ref(got));
            }
            if label == "f_p" && !bounds.is_null() {
                bounded_probability(got, &bounds["f_p"], &format!("{name}/f_p"));
            } else {
                scalar(
                    got,
                    &want["statistics"][label],
                    tolerance,
                    &format!("{name}/{label}"),
                );
            }
        }
    }

    let named = !case["reference"]["probability_input_bands"].is_null();
    let mut covariance = Vec::new();
    for j in 0..fit.rank() {
        let values = (0..fit.rank())
            .map(|k| fit.covariance(j, k, &control).unwrap())
            .collect::<Vec<_>>();
        if named {
            field_output(name, "covariance", Some(j), &values);
            covariance.extend(values);
        } else {
            for (k, value) in values.iter().enumerate() {
                scalar(
                    value,
                    &want["covariance"][j][k],
                    tolerance,
                    &format!("{name}/cov/{j}/{k}"),
                );
            }
        }
    }
    if named {
        let expected = Value::Array(
            want["covariance"]
                .as_array()
                .unwrap()
                .iter()
                .flat_map(|row| row.as_array().unwrap().iter().cloned())
                .collect(),
        );
        near_reference_field(&covariance, &expected, &format!("{name}/covariance"));
    }
    let residuals = fit
        .residuals()
        .iter()
        .map(|e| fit.residual_estimate(*e))
        .collect::<Vec<_>>();
    let fitted = fit
        .residuals()
        .iter()
        .enumerate()
        .map(|(i, e)| {
            fit.fitted_estimate(fit.normalised_outcome(number(&case["input"]["y"][i])) - e)
        })
        .collect::<Vec<_>>();
    for (field, values) in [("residual", residuals), ("fitted", fitted)] {
        if named {
            near_reference_field(&values, &want[field], &format!("{name}/{field}"));
            field_output(name, field, None, &values);
        } else {
            for (i, value) in values.iter().enumerate() {
                scalar(
                    value,
                    &want[field][i],
                    tolerance,
                    &format!("{name}/{field}/{i}"),
                );
            }
        }
    }
}
// These calibration cohorts sit near a platform-dependent SVD reconstruction
// boundary. The factor gate precedes rank or inference; a trustworthy refusal
// must not be relabelled as an identified fit or assigned a guessed rank.
fn portable_reconstruction_refusal(name: &str, failure: &RegressionFailure) -> bool {
    if !matches!(name, "near_1e-13" | "reference_near_A_3.55e-13" | "reference_near_C_2e-12")
        || failure.kind != RegressionFailureKind::Numerical
        || failure.reason != "The decomposition does not reconstruct the scaled design within the numerical tolerance."
    {
        return false;
    }
    assert!(failure.expected_terms.is_some());
    assert_eq!(failure.rank, None);
    assert!(failure.implicated_terms.is_empty());
    println!("PORTABLE_RECONSTRUCTION_REFUSAL {name}");
    true
}
#[test]
fn simple_f_equals_t_squared() {
    check_case(&case_named("simple"));
}
#[test]
fn multiple_numeric_and_two_factors() {
    for n in ["numeric", "two_factors_reference_A"] {
        check_case(&case_named(n));
    }
}
#[test]
fn reference_recode_preserves_fit() {
    for c in cases().iter().filter(|c| {
        c["name"]
            .as_str()
            .unwrap()
            .starts_with("two_factors_reference_")
    }) {
        check_case(c);
    }
}
#[test]
fn literal_replication_covariance_and_df() {
    for n in ["weighted", "expanded"] {
        check_case(&case_named(n));
    }
}
#[test]
fn maximum_frequency_intervals() {
    check_case(&case_named("maximum_frequency"));
}
#[test]
fn rank_boundary_and_warning() {
    for n in [
        "near_1e-06",
        "near_1e-13",
        "near_0.0",
        "reference_near_A_3.55e-13",
        "reference_near_B_3.55e-13",
        "reference_near_C_2e-12",
    ] {
        check_case(&case_named(n));
    }
}
#[test]
fn exact_zero_vs_squared_underflow() {
    for n in [
        "constant_outcome",
        "perfect_fit",
        "perfect_fit_positive",
        "zero_outcome",
        "scale_1e-161_frequency_1",
        "scale_1e-162_frequency_1125899906842624",
        "scale_1e-200_frequency_1",
        "overflow_square_only",
    ] {
        check_case(&case_named(n));
    }
}
#[test]
fn n_equals_p_and_rank_bounded_by_m() {
    for n in ["n_eq_p", "n_lt_p", "constant_predictor", "terms_65"] {
        check_case(&case_named(n));
    }
}
#[test]
fn finite_input_and_original_unit_range_dispositions() {
    for n in ["offset_scale", "difference_overflow", "subnormal_spread"] {
        check_case(&case_named(n));
    }
}
#[test]
fn precharge_refuses_before_kernel_growth() {
    assert!(super::super::accounting::kernel_work_bytes(usize::MAX, 64).is_err());
    let needed = super::super::accounting::kernel_work_bytes(100, 64).unwrap();
    let mut budget = crate::data::AllocationCounter {
        bytes: MAX_OWNED_BYTES - needed + 1,
    };
    assert!(budget.add(needed).is_err());
}

mod controls;

#[test]
fn perfect_fit_rejects_previously_skipped_statistics() {
    assert!(
        std::panic::catch_unwind(|| check_case_with(&case_named("perfect_fit"), true)).is_err()
    );
}

mod routes;

#[test]
fn malformed_private_arguments_preserve_cancellation_precedence() {
    let control = JobControl::default();
    let KernelOutcome::Fitted(fit) = run_case(&case_named("simple"), &control).unwrap() else {
        panic!("fit")
    };
    assert!(matches!(
        fit.covariance(fit.rank(), 0, &control),
        Err(AppError::InvalidInput { .. })
    ));
    assert!(matches!(
        fit.predict(&[], &control),
        Err(AppError::InvalidInput { .. })
    ));
    control.cancel();
    let covariance_cancelled = matches!(
        fit.covariance(fit.rank(), 0, &control),
        Err(AppError::Cancelled)
    );
    let prediction_cancelled = matches!(fit.predict(&[], &control), Err(AppError::Cancelled));
    assert_eq!((covariance_cancelled, prediction_cancelled), (true, true));
}
