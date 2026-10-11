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
struct Payload(#[serde(with = "ComputedAnovaWire")] ComputedAnova);
mod captures;
fn encode(r: &ComputedAnova) -> String {
    r.validate(&JobControl::default()).unwrap();
    #[derive(Serialize)]
    struct Borrowed<'a>(#[serde(with = "ComputedAnovaWire")] &'a ComputedAnova);
    let owned = r.owned_bytes().unwrap();
    let bound = owned.checked_mul(8).unwrap().checked_add(4096).unwrap();
    Budget::check_datasets(&[], owned + bound + r.validation_scratch_bytes().unwrap()).unwrap();
    let mut output = Vec::with_capacity(bound);
    serde_json::to_writer(&mut output, &Borrowed(r)).unwrap();
    assert!(output.len() <= bound);
    String::from_utf8(output).unwrap()
}
fn decode(input: &str, headroom: usize, c: &JobControl) -> AppResult<ComputedAnova> {
    if input.len() > MAX_PROJECT_BYTES {
        return Err(AppError::SizeLimit {
            limit: MAX_PROJECT_BYTES,
        });
    }
    let state = RefCell::new(Accounting {
        bytes: MAX_OWNED_BYTES - headroom,
        control: c,
        error: None,
        steps: 0,
    });
    let scratch = crate::project_io::decode::check_input(input.as_bytes())?
        .max(3 * MAX_DEPTH)
        .checked_add(size_of::<Payload>())
        .ok_or(AppError::SizeLimit {
            limit: MAX_OWNED_BYTES,
        })?;
    state.borrow_mut().add(input.len())?;
    state.borrow_mut().add(scratch)?;
    let mut j = serde_json::Deserializer::from_str(input);
    let p = Payload::deserialize(Guard {
        inner: &mut j,
        state: &state,
        depth: 0,
    })
    .map_err(|_| {
        state
            .borrow_mut()
            .error
            .take()
            .unwrap_or_else(|| AppError::invalid("fixture", "Invalid ANOVA payload."))
    })?;
    j.end()
        .map_err(|_| AppError::invalid("fixture", "Trailing payload."))?;
    validate_decoded(&p.0, &state)?;
    Ok(p.0)
}
#[test]
fn all_designs_captured_identity() {
    for design in 0..4 {
        let r = captures::capture(design, 1., false);
        let got = decode(&encode(&r), MAX_OWNED_BYTES, &JobControl::default());
        assert_eq!(got.unwrap(), r);
    }
}

fn parse<T: for<'de> Deserialize<'de>>(input: &str) -> AppResult<T> {
    let c = JobControl::default();
    let state = RefCell::new(Accounting {
        bytes: input.len() + size_of::<T>() + 3 * MAX_DEPTH,
        control: &c,
        error: None,
        steps: 0,
    });
    let mut j = serde_json::Deserializer::from_str(input);
    let value = T::deserialize(Guard {
        inner: &mut j,
        state: &state,
        depth: 0,
    })
    .map_err(|_| AppError::invalid("shape", "Invalid shape."))?;
    j.end()
        .map_err(|_| AppError::invalid("shape", "Trailing shape."))?;
    Ok(value)
}
fn record<T: for<'de> Deserialize<'de>>(v: &Value) {
    assert!(parse::<T>(&v.to_string()).is_ok());
    let object = v.as_object().unwrap();
    assert!(parse::<T>(&Value::Array(object.values().cloned().collect()).to_string()).is_err());
    let mut bad = v.clone();
    bad["unexpected"] = json!(0);
    assert!(parse::<T>(&bad.to_string()).is_err());
    for key in object.keys() {
        let mut bad = v.clone();
        bad.as_object_mut().unwrap().remove(key);
        assert!(parse::<T>(&bad.to_string()).is_err(), "omitted {key}");
        let text = v.to_string();
        let duplicate = format!("{{{}:{},{}", json!(key), object[key], &text[1..]);
        assert!(parse::<T>(&duplicate).is_err(), "duplicate {key}");
    }
}
macro_rules! closed {
    ($ty:ty,$wire:literal,$v:expr) => {{
        #[derive(Deserialize)]
        struct Record(#[serde(with=$wire)] $ty);
        record::<Record>($v);
    }};
}
#[test]
fn every_record_is_closed_map_with_required_nullable_fields() {
    let p: Value = serde_json::from_str(&encode(&captures::capture(3, 1., false))).unwrap();
    let m = &p["outcome"]["fitted"];
    let effect = &m["effects"][0];
    let family = &p["families"][0];
    let comparison = &family["members"][0];
    closed!(ComputedAnova, "ComputedAnovaWire", &p);
    closed!(AnovaRequest, "AnovaRequestWire", &p["request"]);
    closed!(
        AnovaWithinFactor,
        "AnovaWithinFactorWire",
        &p["request"]["design"]["repeated"]["factors"][0]
    );
    closed!(
        AnovaPairRequest,
        "AnovaPairRequestWire",
        &comparison["request"]
    );
    closed!(
        AnovaFamilyRequest,
        "AnovaFamilyRequestWire",
        &p["request"]["all_pairs"][0]
    );
    closed!(
        AnovaFactorLevels,
        "AnovaFactorLevelsWire",
        &p["grid"]["factors"][0]
    );
    closed!(AnovaCell, "AnovaCellWire", &p["grid"]["cells"][0]);
    closed!(AnovaGrid, "AnovaGridWire", &p["grid"]);
    closed!(AnovaEnergy, "AnovaEnergyWire", &m["error_ss"]);
    closed!(AnovaTest, "AnovaTestWire", &effect["test"]);
    closed!(AnovaEffect, "AnovaEffectWire", effect);
    closed!(AnovaModelResult, "AnovaModelResultWire", m);
    closed!(AnovaComparison, "AnovaComparisonWire", comparison);
    closed!(AnovaContrastFamily, "AnovaContrastFamilyWire", family);
    closed!(AnovaMethod, "AnovaMethodWire", &p["method"]);
    closed!(
        AnovaFailure,
        "AnovaFailureWire",
        &json!({"kind":"numerical","reason":"calculation unavailable"})
    );
    let r: Value = serde_json::from_str(&encode(&captures::capture(2, 1., false))).unwrap();
    closed!(
        AnovaCorrection,
        "AnovaCorrectionWire",
        &r["outcome"]["fitted"]["effects"][0]["gg"]
    );
}
macro_rules! tags {
    ($ty:ty,$wire:literal,$good:expr,$bad:expr) => {{
        #[derive(Deserialize)]
        struct Tagged(#[serde(with=$wire)] $ty);
        for text in $good {
            assert!(parse::<Tagged>(text).is_ok(), "{text}");
        }
        for text in $bad {
            assert!(parse::<Tagged>(text).is_err(), "{text}");
        }
    }};
}
#[test]
fn canonical_mixed_tags_and_ids() {
    tags!(
        AnovaScope,
        "AnovaScopeWire",
        [r#""marginal""#, r#"{"fixed":["a",2]}"#],
        [
            r#"{"marginal":null}"#,
            r#"{"fixed":[],"fixed":[]}"#,
            r#"{"fixed":[],"other":0}"#,
            r#"["fixed",[]]"#,
            r#"{"fixed":[true]}"#,
            r#""unknown""#
        ]
    );
    tags!(
        AnovaQuantityState,
        "AnovaQuantityStateWire",
        [
            r#""positive""#,
            r#""exact_zero""#,
            r#"{"unavailable":"numerical"}"#
        ],
        [
            r#"{"positive":null}"#,
            r#"{"exact_zero":null}"#,
            r#"{"unavailable":"bad"}"#,
            r#"{"unavailable":"numerical","unavailable":"numerical"}"#
        ]
    );
    tags!(
        AnovaComparisonState,
        "AnovaComparisonStateWire",
        [
            r#""available""#,
            r#"{"unavailable":{"kind":"numerical","reason":"r"}}"#,
            r#"{"absent_requested_level":{"kind":"absent_requested_level","reason":"r"}}"#
        ],
        [
            r#"{"available":null}"#,
            r#"{"unavailable":{"kind":"numerical","reason":"r"},"available":null}"#,
            r#"["available"]"#
        ]
    );
    tags!(
        AnovaFamilyKind,
        "AnovaFamilyKindWire",
        [
            r#""planned""#,
            r#"{"all_pairs":{"factor":0,"scope":"marginal"}}"#
        ],
        [
            r#"{"planned":null}"#,
            r#"{"all_pairs":{"factor":0,"scope":"marginal"},"all_pairs":{"factor":0,"scope":"marginal"}}"#
        ]
    );
    tags!(
        AnovaDesign,
        "AnovaDesignWire",
        [r#"{"one_way":{"outcome":"1","factor":"2"}}"#],
        [
            r#"{"one_way":["1","2"]}"#,
            r#"{"one_way":{"outcome":"1","factor":"2","extra":0}}"#,
            r#"{"one_way":{"outcome":"1","factor":"2"},"one_way":{"outcome":"1","factor":"2"}}"#,
            r#"{"bad":{}}"#
        ]
    );
    tags!(
        AnovaOutcome,
        "AnovaOutcomeWire",
        [r#"{"unavailable":{"kind":"numerical","reason":"r"}}"#],
        [
            r#"{"unavailable":{"kind":"numerical","reason":"r"},"unavailable":{"kind":"numerical","reason":"r"}}"#,
            r#"{"unavailable":["numerical","r"]}"#,
            r#"{"unknown":null}"#
        ]
    );
    tags!(
        ColumnId,
        "column_id",
        [r#""1""#, r#""18446744073709551615""#],
        [
            r#""0""#,
            r#""01""#,
            r#""+1""#,
            r#""18446744073709551616""#,
            r#"1"#
        ]
    );
    tags!(
        AnovaFailureKind,
        "AnovaFailureKindWire",
        [r#""empty_cells""#],
        [r#"{"empty_cells":null}"#, r#""other""#]
    );
}
mod mutations;
mod work;

#[test]
fn export_validated_private_schema_fixtures() {
    let mut fixtures = vec![];
    for design in 0..4 {
        for confidence in [
            ConfidenceLevel::P90,
            ConfidenceLevel::P95,
            ConfidenceLevel::P99,
        ] {
            let r = captures::capture_confidence(design, 1., false, confidence);
            r.validate(&JobControl::default()).unwrap();
            fixtures.push(serde_json::from_str::<Value>(&encode(&r)).unwrap());
        }
    }
    fixtures.push(serde_json::from_str(&encode(&captures::capture(1, 1., true))).unwrap());
    let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join(".build/anova-task6/schema-fixtures.json");
    std::fs::create_dir_all(path.parent().unwrap()).unwrap();
    std::fs::write(path, serde_json::to_vec(&fixtures).unwrap()).unwrap();
    let tracked_path = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests/fixtures/anova-v5-payloads.json");
    let tracked: Vec<Value> =
        serde_json::from_slice(&std::fs::read(tracked_path).unwrap()).unwrap();
    let selected: Vec<Value> = [0, 3, 6, 9, 12]
        .into_iter()
        .map(|i| fixtures[i].clone())
        .collect();
    // Shapes, identities, integer counts, statuses and reasons remain exact.
    // Calculated floating fields may differ by a few ULPs across OS libraries;
    // these captures are schema fixtures, not a platform-independent bit oracle.
    assert!(
        same_schema_capture(&Value::Array(tracked), &Value::Array(selected)),
        "tracked schema captures changed beyond platform rounding"
    );
}
fn same_schema_capture(expected: &Value, actual: &Value) -> bool {
    match (expected, actual) {
        (Value::Object(e), Value::Object(a)) => {
            e.len() == a.len()
                && e.iter()
                    .all(|(k, v)| a.get(k).is_some_and(|other| same_schema_capture(v, other)))
        }
        (Value::Array(e), Value::Array(a)) => {
            e.len() == a.len() && e.iter().zip(a).all(|(e, a)| same_schema_capture(e, a))
        }
        (Value::Number(e), Value::Number(a)) if e.is_f64() && a.is_f64() => {
            let (e, a) = (e.as_f64().unwrap(), a.as_f64().unwrap());
            if !e.is_finite() || !a.is_finite() {
                return false;
            }
            if e == 0. || a == 0. {
                return e.to_bits() == a.to_bits();
            }
            let ordered = |v: f64| {
                let b = v.to_bits();
                if b >> 63 != 0 {
                    !b
                } else {
                    b ^ (1 << 63)
                }
            };
            ordered(e).abs_diff(ordered(a)) <= 4
        }
        _ => expected == actual,
    }
}
#[test]
fn schema_capture_comparison_preserves_strict_shape_and_numeric_boundaries() {
    let one = 1f64;
    assert!(same_schema_capture(
        &json!(one),
        &json!(f64::from_bits(one.to_bits() + 4))
    ));
    assert!(!same_schema_capture(
        &json!(one),
        &json!(f64::from_bits(one.to_bits() + 5))
    ));
    assert!(!same_schema_capture(&json!(1u64), &json!(2u64)));
    assert!(!same_schema_capture(&json!(0.), &json!(-0.)));
    assert!(!same_schema_capture(&json!(0.), &json!(f64::from_bits(1))));
    assert!(!same_schema_capture(
        &json!({"reason":null}),
        &json!({"reason":"changed"})
    ));
    assert!(!same_schema_capture(
        &json!({"a":1}),
        &json!({"a":1,"extra":0})
    ));
    assert!(!same_schema_capture(&json!([1, 2]), &json!([1])));
}
mod edges;
