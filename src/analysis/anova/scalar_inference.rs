//! Test-only ANOVA caller coverage; existing Student numerics stay unchanged.
use crate::{
    analysis::{inference, ConfidenceLevel},
    operation::JobControl,
};
#[test]
fn integer_degrees_and_confidence_references() {
    let fixture: serde_json::Value =
        serde_json::from_str(include_str!("../../../tests/fixtures/anova-tails.json")).unwrap();
    for c in fixture["student"].as_array().unwrap() {
        let df = c["df"].as_f64().unwrap();
        assert!(df >= 1. && df <= 9_007_199_254_740_991. && df.fract() == 0.);
        let v = c["v"].as_f64().unwrap();
        let estimate = if c["kind"] == "C" {
            inference::student_t_critical(
                df,
                match v as u32 {
                    90 => ConfidenceLevel::P90,
                    95 => ConfidenceLevel::P95,
                    99 => ConfidenceLevel::P99,
                    _ => panic!("confidence"),
                },
                &JobControl::default(),
            )
            .unwrap()
        } else {
            inference::student_t_two_sided_p_from_log(v, df, &JobControl::default()).unwrap()
        };
        assert_eq!(estimate.value.is_some(), c["status"] == "computed", "{c}");
        if let Some(value) = estimate.value {
            let reference: f64 = c["reference"].as_str().unwrap().parse().unwrap();
            if c["kind"] == "C" {
                assert!((value - reference).abs() < 3e-11, "{c}");
            } else {
                assert!((value / reference - 1.).abs() < 2e-9, "{c}");
            }
        }
    }
}
