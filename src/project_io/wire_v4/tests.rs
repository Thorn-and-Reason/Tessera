//! Direct guarded current regression payload controls.
use super::*;
use crate::{
    data::MAX_OWNED_BYTES,
    error::{AppError, AppResult},
    operation::JobControl,
    project_io::{
        bounded::{Accounting, Guard},
        MAX_DEPTH, MAX_PROJECT_BYTES,
    },
};
use serde_json::{json, Value};
use std::{cell::RefCell, mem::size_of};

#[derive(Debug, Serialize, Deserialize)]
struct Payload(#[serde(with = "ComputedRegressionWire")] ComputedRegression);

// Same parser scratch/depth admission and typed Guard as the production decoder.
fn guarded<T: for<'de> Deserialize<'de>>(input: &str, headroom: usize) -> AppResult<T> {
    guarded_checked(input, headroom, |_, _| Ok(()))
}
fn guarded_checked<T: for<'de> Deserialize<'de>>(
    input: &str,
    headroom: usize,
    check: impl FnOnce(&T, &RefCell<Accounting<'_>>) -> AppResult<()>,
) -> AppResult<T> {
    if input.len() > MAX_PROJECT_BYTES {
        return Err(AppError::SizeLimit {
            limit: MAX_PROJECT_BYTES,
        });
    }
    let control = JobControl::default();
    let state = RefCell::new(Accounting {
        bytes: MAX_OWNED_BYTES - headroom,
        control: &control,
        error: None,
        steps: 0,
    });
    let scratch = crate::project_io::decode::check_input(input.as_bytes())?
        .max(3 * MAX_DEPTH)
        .checked_add(size_of::<T>())
        .ok_or(AppError::SizeLimit {
            limit: MAX_OWNED_BYTES,
        })?;
    state.borrow_mut().add(input.len())?;
    state.borrow_mut().add(scratch)?;
    let mut json = serde_json::Deserializer::from_str(input);
    let result = T::deserialize(Guard {
        inner: &mut json,
        state: &state,
        depth: 0,
    });
    let decoded = result.map_err(|_| {
        state
            .borrow_mut()
            .error
            .take()
            .unwrap_or_else(|| AppError::invalid("fixture", "Invalid guarded payload."))
    })?;
    json.end()
        .map_err(|_| AppError::invalid("fixture", "Trailing payload data."))?;
    check(&decoded, &state)?;
    control.checkpoint(1, Some(1))?;
    Ok(decoded)
}
fn parse<T: for<'de> Deserialize<'de>>(value: &Value) -> AppResult<T> {
    guarded(&value.to_string(), MAX_OWNED_BYTES)
}
fn estimate(v: f64) -> Value {
    json!({"value": v, "reason": null})
}
fn absent() -> Value {
    json!({"value": null, "reason": "Structural fixture unavailable."})
}
fn failure(kind: Value) -> Value {
    json!({"kind": kind, "reason": "Structural fixture failure.",
           "expected_terms": null, "rank": null, "implicated_terms": []})
}
fn payload() -> Value {
    let e = estimate(1.);
    let extent = json!({"range": [estimate(0.), e.clone()],
                       "undefined_rows": 0, "undefined_frequency": 0});
    let coefficient = json!({"estimate": e.clone(), "standard_error": e.clone(),
        "t": e.clone(), "p": estimate(0.5), "interval": [estimate(0.), estimate(2.)]});
    let residual = json!({"cohort_ordinal": 0, "row": 0, "frequency": 1,
        "fitted": e.clone(), "residual": e.clone(), "leverage": estimate(0.5),
        "studentised": e.clone(), "cooks_distance": estimate(0.1)});
    let qq = json!({"rank": 0, "row": 0, "frequency": 1,
                   "theoretical": estimate(-1.), "residual": e.clone()});
    let diagnostic = json!({"residual_mean": estimate(0.), "residual_sample_sd": e.clone(),
        "residual_range": [estimate(-1.), e.clone()],
        "fitted_range": [estimate(0.), estimate(2.)],
        "leverage": extent.clone(), "studentised": extent.clone(), "cooks_distance": extent,
        "residual_points": [residual], "residual_reduced": false,
        "qq_points": [qq], "qq_reduced": false,
        "theoretical_range": [estimate(-1.), e.clone()]});
    let statistics = json!({"sse": e.clone(), "sst": e.clone(), "model_ss": e.clone(),
        "residual_standard_error": e.clone(), "r_squared": e.clone(),
        "adjusted_r_squared": e.clone(), "f": e.clone(), "f_p": estimate(0.5)});
    let fit = json!({"rank": 3, "model_df": 2, "residual_df": 1, "total_df": 3,
        "condition": estimate(2.), "condition_warned": false, "outcome_scale": e.clone(),
        "term_scales": [e.clone(), e.clone(), e],
        "coefficients": [coefficient.clone(), coefficient.clone(), coefficient],
        "statistics": statistics, "residual_diagnostics": diagnostic});
    json!({"revision": "1", "request": {"outcome": "1", "predictors": [
        {"column": "2", "role": "numeric"},
        {"column": "3", "role": {"categorical": {"reference": "A"}}}],
        "confidence": "95%", "filter": {"predicates": []}, "frequency_weight": null},
        "columns": [
            {"id": "1", "name": "Y", "kind": "number", "description": "",
             "missing_rules": "none", "value_labels": []},
            {"id": "2", "name": "X", "kind": "number", "description": "",
             "missing_rules": "none", "value_labels": []},
            {"id": "3", "name": "Category", "kind": "text", "description": "",
             "missing_rules": "none", "value_labels": []}],
        "source_rows": 4, "diagnostics": {"filtered_rows": 0, "missing_weight_rows": 0,
            "zero_weight_rows": 0, "complete_case_rows": 0},
        "fitted_rows": 4, "frequency": 4, "incomplete_case_frequency": 0,
        "levels": [{"column": "3", "values": ["A", "B"]}],
        "terms": ["intercept", {"numeric": {"column": "2"}},
            {"contrast": {"column": "3", "level": "B", "reference": "A"}}],
        "method": {"numerical_version": 1, "rank_threshold": 1e-12, "condition_warning": 1e8},
        "outcome": {"fitted": fit}})
}
fn record<T: for<'de> Deserialize<'de>>(value: &Value) {
    assert!(parse::<T>(value).is_ok());
    let object = value.as_object().expect("record fixture");
    assert!(parse::<T>(&Value::Array(object.values().cloned().collect())).is_err());
    let mut bad = value.clone();
    bad["unexpected"] = json!(0);
    assert!(parse::<T>(&bad).is_err());
    for key in object.keys() {
        let mut bad = value.clone();
        bad.as_object_mut().unwrap().remove(key);
        assert!(parse::<T>(&bad).is_err(), "omitted {key}");
        let text = value.to_string();
        let duplicate = format!("{{{}:{},{}", json!(key), object[key], &text[1..]);
        assert!(
            guarded::<T>(&duplicate, MAX_OWNED_BYTES).is_err(),
            "duplicate {key}"
        );
    }
}
macro_rules! check_record {
    ($ty:ty, $wire:literal, $value:expr) => {{
        #[derive(Deserialize)]
        struct Record(#[serde(with = $wire)] $ty);
        let _ = parse::<Record>($value).unwrap().0;
        record::<Record>($value);
    }};
}
#[test]
fn every_new_record_is_guarded_closed_and_requires_every_field() {
    let p = payload();
    let fit = &p["outcome"]["fitted"];
    let d = &fit["residual_diagnostics"];
    check_record!(ComputedRegression, "ComputedRegressionWire", &p);
    check_record!(RegressionRequest, "RegressionRequestWire", &p["request"]);
    check_record!(
        RegressionPredictor,
        "RegressionPredictorWire",
        &p["request"]["predictors"][1]
    );
    check_record!(RegressionLevels, "RegressionLevelsWire", &p["levels"][0]);
    check_record!(RegressionMethod, "RegressionMethodWire", &p["method"]);
    check_record!(
        RegressionFailure,
        "RegressionFailureWire",
        &failure(json!("numerical"))
    );
    check_record!(RegressionFit, "RegressionFitWire", fit);
    check_record!(
        RegressionCoefficient,
        "RegressionCoefficientWire",
        &fit["coefficients"][0]
    );
    check_record!(
        RegressionStatistics,
        "RegressionStatisticsWire",
        &fit["statistics"]
    );
    check_record!(Estimate, "EstimateWire", &fit["statistics"]["sse"]);
    check_record!(DiagnosticExtent, "DiagnosticExtentWire", &d["leverage"]);
    check_record!(RegressionDiagnostics, "RegressionDiagnosticsWire", d);
    check_record!(
        RegressionResidualPoint,
        "RegressionResidualPointWire",
        &d["residual_points"][0]
    );
    check_record!(
        RegressionQqPoint,
        "RegressionQqPointWire",
        &d["qq_points"][0]
    );
}
fn variants<T: for<'de> Deserialize<'de>>(values: Vec<Value>) {
    for value in values {
        assert!(parse::<T>(&value).is_ok(), "{value}");
        if let Some(object) = value.as_object() {
            let (tag, body) = object.iter().next().unwrap();
            let mut multiple = value.clone();
            multiple["unknown"] = Value::Null;
            assert!(parse::<T>(&multiple).is_err());
            let duplicate = format!("{{{}:{},{}:{}}}", json!(tag), body, json!(tag), body);
            assert!(guarded::<T>(&duplicate, MAX_OWNED_BYTES).is_err());
            let mut sequence = value.clone();
            sequence[tag] = Value::Array(body.as_object().unwrap().values().cloned().collect());
            assert!(parse::<T>(&sequence).is_err());
            let mut unknown = value.clone();
            unknown[tag]["unknown"] = json!(0);
            assert!(parse::<T>(&unknown).is_err());
            for field in body.as_object().unwrap().keys() {
                let mut omitted = value.clone();
                omitted[tag].as_object_mut().unwrap().remove(field);
                assert!(parse::<T>(&omitted).is_err());
            }
        } else {
            assert!(parse::<T>(&json!({(value.as_str().unwrap()): null})).is_err());
        }
    }
    for bad in [
        json!({}),
        json!({"unknown": {}}),
        json!(["unknown", null]),
        json!("unknown"),
    ] {
        assert!(parse::<T>(&bad).is_err());
    }
}
#[test]
fn all_tags_are_canonical_single_variants_with_guarded_payloads() {
    #[derive(Deserialize)]
    struct Role(#[serde(with = "PredictorRoleWire")] PredictorRole);
    #[derive(Deserialize)]
    struct Term(#[serde(with = "RegressionTermWire")] RegressionTerm);
    #[derive(Deserialize)]
    struct Kind(#[serde(with = "RegressionFailureKindWire")] RegressionFailureKind);
    #[derive(Deserialize)]
    struct Outcome(#[serde(with = "RegressionOutcomeWire")] RegressionOutcome);
    let _ = parse::<Role>(&json!("numeric")).unwrap().0;
    let _ = parse::<Term>(&json!("intercept")).unwrap().0;
    let _ = parse::<Kind>(&json!("numerical")).unwrap().0;
    let _ = parse::<Outcome>(&payload()["outcome"]).unwrap().0;
    variants::<Role>(vec![
        json!("numeric"),
        json!({"categorical": {"reference": ""}}),
        json!({"categorical": {"reference": 0.0}}),
    ]);
    variants::<Term>(payload()["terms"].as_array().unwrap().clone());
    variants::<Kind>(vec![
        json!("empty_cohort"),
        json!("unidentified"),
        json!("nonconvergence"),
        json!("numerical"),
        json!({"absent_reference": {"column": "3"}}),
        json!({"no_contrast": {"column": "3"}}),
    ]);
    variants::<Outcome>(vec![
        payload()["outcome"].clone(),
        json!({"unavailable": failure(json!("empty_cohort"))}),
    ]);
}
#[test]
fn structural_fit_failure_and_independent_unavailable_scalars_roundtrip() {
    let mut cases = vec![payload()];
    for kind in [
        json!("empty_cohort"),
        json!("unidentified"),
        json!("nonconvergence"),
        json!("numerical"),
        json!({"absent_reference": {"column": "3"}}),
        json!({"no_contrast": {"column": "3"}}),
    ] {
        let mut p = payload();
        p["outcome"] = json!({"unavailable": failure(kind)});
        cases.push(p);
    }
    let mut independent = payload();
    for name in ["sse", "sst", "model_ss"] {
        independent["outcome"]["fitted"]["statistics"][name] = absent();
    }
    cases.push(independent);
    for value in cases {
        let captured = parse::<Payload>(&value).unwrap();
        let encoded = serde_json::to_string(&captured).unwrap();
        let reopened = guarded::<Payload>(&encoded, MAX_OWNED_BYTES).unwrap();
        assert_eq!(captured.0, reopened.0);
        // No ComputedRegression::validate call: these are shapes, not solver captures.
    }
}
#[test]
fn malformed_id_cell_and_fixed_interval_shapes_are_refused() {
    for identity in [
        json!("0"),
        json!("01"),
        json!("-1"),
        json!("18446744073709551616"),
        json!(1),
    ] {
        let mut bad = payload();
        bad["request"]["outcome"] = identity;
        assert!(parse::<Payload>(&bad).is_err());
    }
    for reference in [json!(true), json!([]), json!({})] {
        let mut bad = payload();
        bad["request"]["predictors"][1]["role"]["categorical"]["reference"] = reference;
        assert!(parse::<Payload>(&bad).is_err());
    }
    for length in [0, 1, 3] {
        let mut bad = payload();
        bad["outcome"]["fitted"]["coefficients"][0]["interval"] =
            Value::Array(vec![estimate(0.); length]);
        assert!(parse::<Payload>(&bad).is_err());
    }
}
#[test]
fn string_and_array_growth_refuse_before_owning_excess_payload() {
    let mut large = payload();
    large["outcome"] = json!({"unavailable": failure(json!("numerical"))});
    large["outcome"]["unavailable"]["reason"] = json!("r".repeat(8192));
    let input = large.to_string();
    let (result, largest) =
        crate::data::measure_test_allocations(|| guarded::<Payload>(&input, 4096));
    assert!(matches!(result, Err(AppError::SizeLimit { .. })));
    assert!(largest < 8192);
    let control = JobControl::default();
    for (headroom, largest_allowed) in [
        (31, 64),
        (210, 4 * size_of::<Cell>()),
        (26 * size_of::<Cell>(), 8 * size_of::<Cell>()),
    ] {
        let state = RefCell::new(Accounting {
            bytes: MAX_OWNED_BYTES - headroom,
            control: &control,
            error: None,
            steps: 0,
        });
        let mut input = serde_json::Deserializer::from_str("[0,1,2,3,4,5,6,7,8]");
        let (result, largest) = crate::data::measure_test_allocations(|| {
            cells::deserialize(Guard {
                inner: &mut input,
                state: &state,
                depth: 0,
            })
        });
        assert!(result.is_err());
        assert!(matches!(
            state.borrow().error,
            Some(AppError::SizeLimit { .. })
        ));
        assert!(largest <= largest_allowed, "allocated {largest}");
    }
}
#[test]
fn guarded_depth_and_hostile_nested_types_are_refused() {
    let control = JobControl::default();
    let state = RefCell::new(Accounting {
        bytes: 0,
        control: &control,
        error: None,
        steps: 0,
    });
    let mut input = serde_json::Deserializer::from_str("{}");
    assert!(Payload::deserialize(Guard {
        inner: &mut input,
        state: &state,
        depth: MAX_DEPTH + 1,
    })
    .is_err());
    let nested = format!("{}0{}", "[".repeat(66), "]".repeat(66));
    assert!(guarded::<de::IgnoredAny>(&nested, MAX_OWNED_BYTES).is_err());
    let mut input = payload().to_string();
    input.push_str(" {}");
    assert!(guarded::<Payload>(&input, MAX_OWNED_BYTES).is_err());
}

mod captures;

#[test]
fn explicit_input_and_collection_ceilings_precede_excess_payload_allocation() {
    let oversized = " ".repeat(MAX_PROJECT_BYTES + 1);
    assert!(matches!(
        guarded::<Payload>(&oversized, MAX_OWNED_BYTES),
        Err(AppError::SizeLimit {
            limit: MAX_PROJECT_BYTES
        })
    ));
    drop(oversized);
    #[derive(Deserialize)]
    struct CellValues(#[serde(with = "cells")] Vec<Cell>);
    let input = format!("[{}0]", "0,".repeat(crate::data::MAX_CELLS));
    assert!(guarded::<CellValues>(&input, MAX_OWNED_BYTES).is_err());
    let valid = guarded::<CellValues>("[0]", MAX_OWNED_BYTES).unwrap();
    assert_eq!(valid.0.len(), 1);
}

#[test]
fn nested_typed_strings_and_parser_scratch_are_precharged_before_copying() {
    #[derive(Deserialize)]
    struct Role(#[serde(with = "PredictorRoleWire")] PredictorRole);
    let text = format!(
        r#"{{"categorical":{{"reference":"{}"}}}}"#,
        "r".repeat(8192)
    );
    let scratch = crate::project_io::decode::check_input(text.as_bytes())
        .unwrap()
        .max(3 * MAX_DEPTH);
    let headroom = text.len() + scratch + size_of::<Role>() + 4096;
    let (result, largest) =
        crate::data::measure_test_allocations(|| guarded::<Role>(&text, headroom));
    assert!(matches!(result, Err(AppError::SizeLimit { .. })));
    assert!(
        largest < 8192,
        "allocated the unadmitted typed reference: {largest}"
    );
    let text = format!(
        r#"{{"categorical":{{"reference":"{}"}}}}"#,
        "\\u0061".repeat(8192)
    );
    let (result, largest) =
        crate::data::measure_test_allocations(|| guarded::<Role>(&text, text.len() + 256));
    assert!(matches!(result, Err(AppError::SizeLimit { .. })));
    assert!(
        largest < 4096,
        "allocated unadmitted parser scratch: {largest}"
    );
    let _ = parse::<Role>(&json!({"categorical":{"reference":""}}))
        .unwrap()
        .0;
}

#[test]
fn fitted_box_is_admitted_before_payload_or_owner_allocation() {
    #[derive(Deserialize)]
    struct Fit(#[serde(with = "boxed_fit")] Box<RegressionFit>);
    let input = payload()["outcome"]["fitted"].to_string();
    let base = input.len()
        + crate::project_io::decode::check_input(input.as_bytes())
            .unwrap()
            .max(3 * MAX_DEPTH)
        + size_of::<Fit>();
    let (result, largest) = crate::data::measure_test_allocations(|| {
        guarded::<Fit>(&input, base + size_of::<RegressionFit>() - 1)
    });
    assert!(matches!(result, Err(AppError::SizeLimit { .. })));
    assert!(
        largest < size_of::<RegressionFit>(),
        "unadmitted fit box allocated: {largest}"
    );
    guarded_checked::<Fit>(&input, MAX_OWNED_BYTES, |fit, state| {
        assert!(state.borrow().bytes >= base + size_of::<RegressionFit>());
        assert!(fit.0.rank > 0);
        Ok(())
    })
    .unwrap();
}
