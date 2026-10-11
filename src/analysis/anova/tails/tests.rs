use super::*;
use crate::{
    error::AppError,
    operation::{JobControl, CHECKPOINT_HOOK},
};
#[test]
fn exact_binary64_reference_fixtures() {
    let fixtures: serde_json::Value =
        serde_json::from_str(include_str!("../../../../tests/fixtures/anova-tails.json")).unwrap();
    for c in fixtures["cases"].as_array().unwrap() {
        let number = |k: &str| f64::from_bits(c[k].as_u64().unwrap());
        let result = evaluate(
            number("log_bits"),
            number("df1_bits"),
            number("df2_bits"),
            &JobControl::default(),
        )
        .unwrap();
        assert_eq!(
            result.estimate.value.is_some(),
            c["available"].as_bool().unwrap(),
            "{c}"
        );
        if let Some(p) = result.estimate.value {
            let reference: f64 = c["reference"].as_str().unwrap().parse().unwrap();
            assert!((p / reference - 1.).abs() <= 2e-9, "{c}: {p}");
            let range = result.range.unwrap();
            assert!(range.lower <= p && p <= range.upper);
        } else {
            assert!(result.range.is_none());
        }
    }
}
#[test]
fn domain_work_and_cancellation() {
    let ck = JobControl::default();
    for (l, a, b) in [
        (f64::NEG_INFINITY, 1., 2.),
        (f64::INFINITY, 1., 2.),
        (0., 0.99, 2.),
        (0., 64., 64.),
        (0., 2., 1.),
        (0., 1., 2_000_001.),
    ] {
        assert!(f_upper_from_log(l, a, b, &ck).is_err());
    }
    for limit in [0, 1, 3] {
        assert!(evaluate_with_limit(0., 7.4, 140.6, limit, &ck)
            .unwrap()
            .estimate
            .value
            .is_none());
    }
    ck.cancel();
    assert!(matches!(
        f_upper_from_log(0., 1., 2., &ck),
        Err(AppError::Cancelled)
    ));
    let ck = JobControl::default();
    let cancel = ck.clone();
    CHECKPOINT_HOOK.with(|h| {
        *h.borrow_mut() = Some(Box::new(move |p| {
            if p.done == 2 {
                cancel.cancel()
            }
        }))
    });
    let result = evaluate(0., 7.4, 140.6, &ck);
    CHECKPOINT_HOOK.with(|h| *h.borrow_mut() = None);
    assert!(matches!(result, Err(AppError::Cancelled)));
}
#[test]
fn transformed_and_combined_ranges() {
    for log_f in [-4., 0., 1.] {
        let center = evaluate(log_f, 7.4, 19. * 7.4, &JobControl::default()).unwrap();
        for (error, expected) in [(1e-12, true), (1e-3, false)] {
            let mut endpoints = Vec::new();
            for sign in [-1., 1.] {
                for df_sign in [-1., 1.] {
                    let df = 7.4 * (1. + df_sign * error);
                    endpoints.push(
                        evaluate(log_f + sign * error, df, 19. * df, &JobControl::default())
                            .unwrap(),
                    );
                }
            }
            assert_eq!(
                combined_probability(&center, &endpoints, Some(2e-9))
                    .value
                    .is_some(),
                expected
            );
        }
        assert!(combined_probability(&center, &[], None).value.is_none());
        assert!(combined_probability(&center, &[], Some(2e-9))
            .value
            .is_none());
    }
    let direct = probability_range(-2., 2e-9, false);
    assert_eq!(direct.lower, (-2. - 2e-9_f64).exp());
    let complement = probability_range(-2., 2e-9, true);
    assert_eq!(complement.lower, -(-2. + 2e-9_f64).exp_m1());
    assert_eq!(complement.upper, -(-2. - 2e-9_f64).exp_m1());
}

#[test]
fn unavailable_unknown_and_crossing_ranges_refuse() {
    let ck = JobControl::default();
    let center = evaluate(0., 7.4, 140.6, &ck).unwrap();
    let mut endpoints: Vec<_> = (0..4)
        .map(|_| evaluate(0., 7.4, 140.6, &ck).unwrap())
        .collect();
    for allowance in [None, Some(f64::NAN), Some(-1.), Some(3e-9)] {
        assert!(combined_probability(&center, &endpoints, allowance)
            .value
            .is_none());
    }
    endpoints[0] = evaluate(532.622383088778, 1.4, 2.8, &ck).unwrap();
    assert!(combined_probability(&center, &endpoints, Some(0.))
        .value
        .is_none());
    endpoints[0] = EvaluatedTail {
        estimate: Estimate::available(0.5),
        range: Some(ProbabilityRange {
            lower: 0.,
            upper: 1.,
        }),
    };
    assert!(combined_probability(&center, &endpoints, Some(0.))
        .value
        .is_none());
}

#[test]
fn combined_nonfinite_endpoint_refuses() {
    let narrow = || EvaluatedTail {
        estimate: Estimate::available(0.5),
        range: Some(ProbabilityRange {
            lower: 0.4999999999,
            upper: 0.5000000001,
        }),
    };
    let center = narrow();
    let mut endpoints: Vec<_> = (0..4).map(|_| narrow()).collect();
    endpoints[0].estimate = Estimate::available(f64::NAN);
    assert!(combined_probability(&center, &endpoints, Some(0.))
        .value
        .is_none());
}

#[test]
fn combined_central_conditional_width_refuses() {
    let center = EvaluatedTail {
        estimate: Estimate::available(0.5),
        range: Some(ProbabilityRange {
            lower: 0.49,
            upper: 0.51,
        }),
    };
    let endpoints: Vec<_> = (0..4)
        .map(|_| EvaluatedTail {
            estimate: Estimate::available(0.5),
            range: Some(ProbabilityRange {
                lower: 0.4999999999,
                upper: 0.5000000001,
            }),
        })
        .collect();
    assert!(combined_probability(&center, &endpoints, Some(0.))
        .value
        .is_none());
}
