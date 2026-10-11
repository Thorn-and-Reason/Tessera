use super::*;
fn p(t: f64, df: f64) -> f64 {
    student_t_two_sided_p(t, df, &JobControl::default())
        .unwrap()
        .value
        .unwrap()
}
fn close(actual: f64, expected: f64, abs: f64) {
    assert!(
        (actual - expected).abs() <= abs,
        "{actual:e} vs {expected:e}"
    );
    assert!(
        (actual / expected - 1.0).abs() <= 2e-10,
        "{actual:e} vs {expected:e}"
    );
}
#[test]
fn representable_extreme_tails_are_not_zero() {
    for (df, expected) in [
        (1.0, 6.366197723675814e-201),
        (1.25, 6.813022261377278e-251),
        (1.5, 7.541704864032493e-301),
    ] {
        close(p(1e200, df), expected, 2e-13);
        close(p(-1e200, df), expected, 2e-13);
    }
}
#[test]
fn large_df_small_t_is_not_one() {
    close(p(0.01, 2_f64.powi(53)), 0.9920212873707368, 2e-13);
}
#[test]
fn critical_values_match_each_confidence() {
    for (confidence, alpha) in [
        (ConfidenceLevel::P90, 0.1),
        (ConfidenceLevel::P95, 0.05),
        (ConfidenceLevel::P99, 0.01),
    ] {
        for df in [1.0, 2.0, 127.99, 128.0, 1e5, 2_f64.powi(53)] {
            let t = student_t_critical(df, confidence, &JobControl::default())
                .unwrap()
                .value
                .unwrap();
            close(p(t, df), alpha, 2e-14);
            if df == 1.0 {
                close(t, 1.0 / (std::f64::consts::PI * alpha / 2.0).tan(), 2e-11);
            }
        }
    }
}
#[test]
fn subnormal_rounding_and_underflow() {
    assert!(student_t_two_sided_p(1e162, 2.0, &JobControl::default())
        .unwrap()
        .value
        .is_none());
    assert_eq!(p(38.5, 2_f64.powi(53)), f64::from_bits(1));
}
#[test]
fn unrepresentable_t_can_have_representable_p() {
    let log_t = 720.0;
    let actual = student_t_two_sided_p_from_log(log_t, 1.0, &JobControl::default())
        .unwrap()
        .value
        .unwrap();
    close(actual, (-log_t).exp() * 2.0 / std::f64::consts::PI, 2e-13);
}
#[test]
fn bounded_nonconvergence_and_cancel() {
    let control = JobControl::default();
    control.cancel();
    assert!(matches!(
        student_t_two_sided_p(2.0, 4.0, &control),
        Err(crate::error::AppError::Cancelled)
    ));
}
#[test]
fn independent_reference_grid() {
    let fixture: serde_json::Value = serde_json::from_str(include_str!(
        "../../../tests/fixtures/inference-reference.json"
    ))
    .unwrap();
    for row in fixture["tails"].as_array().unwrap() {
        let t = row["t"].as_f64().unwrap();
        let df = row["df"].as_f64().unwrap();
        let expected = row["rounded"].as_f64().unwrap();
        for sign in [-1.0, 1.0] {
            let estimate = student_t_two_sided_p(sign * t, df, &JobControl::default()).unwrap();
            if expected == 0.0 {
                assert!(estimate.value.is_none(), "{row}");
            } else if expected < f64::MIN_POSITIVE {
                let actual = estimate.value.unwrap();
                assert!(actual > 0.0);
                assert!(
                    actual.to_bits().abs_diff(expected.to_bits()) <= 2,
                    "{row}: {actual:e}"
                );
            } else {
                close(estimate.value.unwrap(), expected, 2e-13);
            }
        }
    }
    for row in fixture["log_tails"].as_array().unwrap() {
        let estimate = student_t_two_sided_p_from_log(
            row["log_abs_t"].as_f64().unwrap(),
            row["df"].as_f64().unwrap(),
            &JobControl::default(),
        )
        .unwrap();
        let expected = row["rounded"].as_f64().unwrap();
        if expected == 0.0 {
            assert!(estimate.value.is_none());
        } else {
            let actual = estimate.value.unwrap();
            assert!(actual > 0.0);
            assert!(
                actual.to_bits().abs_diff(expected.to_bits()) <= 2,
                "{row}: {actual:e}"
            );
        }
    }
    for row in fixture["critical"].as_array().unwrap() {
        let confidence = match row["confidence"].as_str().unwrap() {
            "P90" => ConfidenceLevel::P90,
            "P95" => ConfidenceLevel::P95,
            _ => ConfidenceLevel::P99,
        };
        let actual = student_t_critical(
            row["df"].as_f64().unwrap(),
            confidence,
            &JobControl::default(),
        )
        .unwrap()
        .value
        .unwrap();
        close(actual, row["rounded"].as_f64().unwrap(), 2e-11);
    }
}
#[test]
fn monotonicity_continuity_and_invalid_inputs() {
    for df in [1.0, 1.25, 2.0, 127.999999, 128.0, 1e5, 2_f64.powi(53)] {
        let mut previous = 1.0;
        for i in 0..101 {
            let current = p(f64::from(i) / 10.0, df);
            assert!(current <= previous);
            previous = current;
        }
        close(p(2.0 - 1e-12, df), p(2.0 + 1e-12, df), 1e-12);
    }
    let control = JobControl::default();
    for df in [0.0, -1.0, 0.999, 2_f64.powi(54), f64::NAN, f64::INFINITY] {
        assert!(student_t_two_sided_p(1.0, df, &control).is_err());
        assert!(student_t_critical(df, ConfidenceLevel::P95, &control).is_err());
    }
    for t in [f64::NAN, f64::INFINITY, f64::NEG_INFINITY] {
        assert!(student_t_two_sided_p(t, 1.0, &control).is_err());
        assert!(student_t_two_sided_p_from_log(t, 1.0, &control).is_err());
    }
    assert_eq!(p(-0.0, 1.0), 1.0);
    assert!(student_t_two_sided_p_from_log(f64::MAX, 1.0, &control)
        .unwrap()
        .value
        .is_none());
    assert_eq!(
        student_t_two_sided_p_from_log(-f64::MAX, 1.0, &control)
            .unwrap()
            .value,
        Some(1.0)
    );
}
#[test]
fn bounded_work_precision_and_midflight_cancellation() {
    use super::quadrature::integrate_with_test_limits;
    let control = JobControl::default();
    assert!(
        integrate_with_test_limits(|x| x.exp(), 40.0, &control, 14, 24)
            .unwrap()
            .is_none()
    );
    assert!(
        integrate_with_test_limits(|x| (-x).exp(), 40.0, &control, 32768, 0)
            .unwrap()
            .is_none()
    );
    assert!(
        integrate_with_test_limits(|_| f64::NAN, 1.0, &control, 32768, 24)
            .unwrap()
            .is_none()
    );
    let evaluations = std::cell::Cell::new(0);
    let (sender, receiver) = std::sync::mpsc::channel();
    let control = JobControl::new(Default::default(), Some(sender));
    let result = integrate_with_test_limits(
        |x| {
            evaluations.set(evaluations.get() + 1);
            if evaluations.get() == 20 {
                control.cancel();
            }
            (-x).exp()
        },
        40.0,
        &control,
        32768,
        24,
    );
    assert!(matches!(result, Err(crate::error::AppError::Cancelled)));
    assert!(evaluations.get() <= 84);
    assert!(receiver.try_recv().is_err());
    assert!(probability(-1075.0 * std::f64::consts::LN_2, 0.0, 1e-12)
        .reason
        .unwrap()
        .contains("rounding boundary"));
    assert!(probability(f64::NAN, 0.0, 0.0).value.is_none());
}

#[test]
fn review_subnormal_regressions() {
    let fixture: serde_json::Value = serde_json::from_str(include_str!(
        "../../../tests/fixtures/inference-reference.json"
    ))
    .unwrap();
    let mut failures = Vec::new();
    for key in ["tails", "log_tails"] {
        for row in fixture[key].as_array().unwrap() {
            let expected = row["rounded"].as_f64().unwrap();
            if expected <= 0.0 || expected >= f64::MIN_POSITIVE {
                continue;
            }
            let df = row["df"].as_f64().unwrap();
            let actual = if key == "tails" {
                p(row["t"].as_f64().unwrap(), df)
            } else {
                student_t_two_sided_p_from_log(
                    row["log_abs_t"].as_f64().unwrap(),
                    df,
                    &JobControl::default(),
                )
                .unwrap()
                .value
                .unwrap()
            };
            let ulps = actual.to_bits().abs_diff(expected.to_bits());
            if ulps > 2 {
                failures.push(format!("{row}: {ulps} ulps"));
            }
        }
    }
    assert!(failures.is_empty(), "{}", failures.join("\n"));
}
#[test]
fn compensated_tail_arithmetic_retains_low_components() {
    use super::arithmetic::TailFloat as D;
    let sum = D(1e16, 0.0).add(D::ONE).sub(D(1e16, 0.0));
    assert_eq!(sum.0, 1.0);
    let product = D(1.0 + f64::EPSILON, 0.0).mul(D(1.0 - f64::EPSILON, 0.0));
    assert_eq!(product.0, 1.0);
    assert_eq!(product.1, -f64::EPSILON * f64::EPSILON);
    let logarithm = D(2_f64.powi(-53), 0.0).ln_1p();
    assert_eq!(logarithm.0, 2_f64.powi(-53));
    assert!((logarithm.1 / (-2_f64.powi(-107)) - 1.0).abs() < 1e-15);
    for x in [0.5, 1.0, 1.1, 2.0, 128.0, 1e100, 3e307] {
        let logarithm = D(x, 0.0).ln();
        let reconstructed = logarithm.mul(D(-1.0, 0.0)).exp().mul(D(x, 0.0));
        assert!((reconstructed.0 + reconstructed.1 - 1.0).abs() <= f64::EPSILON);
    }
    assert_eq!(D(-1000.0, 0.0).exp().0, 0.0);
}
