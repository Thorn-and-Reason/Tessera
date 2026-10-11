use super::*;
use crate::{error::AppError, operation::CHECKPOINT_HOOK};
use std::{cell::Cell, rc::Rc};

struct ClearHook;
impl Drop for ClearHook {
    fn drop(&mut self) {
        CHECKPOINT_HOOK.with(|hook| *hook.borrow_mut() = None);
    }
}

fn cancel_on(control: &JobControl, target: usize) -> (ClearHook, Rc<Cell<usize>>) {
    let count = Rc::new(Cell::new(0));
    let seen = Rc::clone(&count);
    let cancellation = control.clone();
    CHECKPOINT_HOOK.with(|hook| {
        *hook.borrow_mut() = Some(Box::new(move |_| {
            seen.set(seen.get() + 1);
            if seen.get() == target {
                cancellation.cancel();
            }
        }));
    });
    (ClearHook, count)
}

#[test]
fn every_independent_reference_keeps_its_status_and_tolerance() {
    let fixture: serde_json::Value = serde_json::from_str(include_str!(
        "../../../../tests/fixtures/regression-tails.json"
    ))
    .unwrap();
    let rows = fixture["cases"].as_array().unwrap();
    assert_eq!(rows.len(), 317);
    let mut failures = Vec::new();
    for row in rows {
        let name = row["name"].as_str().unwrap();
        let expected_status = row["expected_status"].as_str().unwrap();
        let expected = row["rounded"].as_f64().unwrap();
        assert!(row["reference"].as_str().is_some());
        let relative = row["relative_tolerance"].as_f64().unwrap();
        let ulps = row["subnormal_ulps"].as_u64().unwrap();
        let control = JobControl::default();
        let (_guard, _) = cancel_on(
            &control,
            row["cancel_at"].as_u64().map_or(usize::MAX, |v| v as usize),
        );
        let result = f_upper_tail(
            row["log_f"].as_f64().unwrap(),
            row["df1"].as_u64().unwrap(),
            row["df2"].as_u64().unwrap(),
            &control,
        );
        let (status, value, reason) = match result {
            Err(AppError::Cancelled) => ("cancelled", None, Some("cancelled".to_owned())),
            Err(error) => panic!("{name}: {error:?}"),
            Ok(estimate) => {
                let status = if estimate.value.is_some() {
                    "available"
                } else if estimate
                    .reason
                    .as_deref()
                    .unwrap()
                    .contains("below Float64")
                {
                    "underflow"
                } else {
                    "precision"
                };
                (status, estimate.value, estimate.reason)
            }
        };
        println!(
            "TAIL_RESULT {}",
            serde_json::json!({
                "name": name, "status": status, "value": value, "reason": reason
            })
        );
        if status != expected_status {
            failures.push(format!(
                "{name}: expected {expected_status}, got {status} ({reason:?})"
            ));
            continue;
        }
        if status == "available" {
            let actual = value.unwrap();
            let accurate = if expected < f64::MIN_POSITIVE {
                actual.to_bits().abs_diff(expected.to_bits()) <= ulps
            } else {
                (actual / expected - 1.0).abs() <= relative
            };
            if !actual.is_finite() || actual <= 0.0 || actual > 1.0 || !accurate || reason.is_some()
            {
                failures.push(format!("{name}: {actual:e} vs {expected:e}"));
            }
        } else {
            assert!(value.is_none() && !reason.unwrap().is_empty());
        }
    }
    assert!(failures.is_empty(), "{}", failures.join("\n"));
}

#[test]
fn real_job_control_cancels_base_and_each_increment() {
    println!("LIVE_SIZES TailFloat={} StudentLogTail={} LowerTailBound={} JobControl={} Estimate={} TailEvaluationTuple={}",
        std::mem::size_of::<D>(), std::mem::size_of::<crate::analysis::inference::StudentLogTail>(),
        std::mem::size_of::<LowerTailBound>(), std::mem::size_of::<JobControl>(),
        std::mem::size_of::<Estimate>(), std::mem::size_of::<(crate::analysis::inference::StudentLogTail, Estimate)>());
    for df1 in [1, 2, 3, 62, 63] {
        let control = JobControl::default();
        let (guard, count) = cancel_on(&control, usize::MAX);
        assert!(f_upper_tail(2.25_f64.ln(), df1, 37, &control)
            .unwrap()
            .value
            .is_some());
        let checkpoints = count.get();
        println!("CANCEL_CHECKPOINTS df1={df1} count={checkpoints}");
        drop(guard);
        assert!(
            checkpoints >= 3 + (df1 - 1) as usize / 2,
            "df1={df1}: {checkpoints}"
        );
        for target in 1..=checkpoints {
            let control = JobControl::default();
            let (_guard, seen) = cancel_on(&control, target);
            assert!(
                matches!(
                    f_upper_tail(2.25_f64.ln(), df1, 37, &control),
                    Err(AppError::Cancelled)
                ),
                "df1={df1}, checkpoint={target}/{}",
                seen.get()
            );
        }
    }
}

#[test]
fn invalid_inputs_and_terminal_refusals_preserve_cancellation() {
    for (log_f, df1, df2) in [
        (f64::NAN, 1, 1),
        (f64::INFINITY, 1, 1),
        (f64::NEG_INFINITY, 1, 1),
        (0.0, 0, 1),
        (0.0, 64, 1),
        (0.0, 1, 0),
        (0.0, 1, (1 << 53) + 1),
    ] {
        let control = JobControl::default();
        assert!(f_upper_tail(log_f, df1, df2, &control).is_err());
        control.cancel();
        assert!(matches!(
            f_upper_tail(log_f, df1, df2, &control),
            Err(AppError::Cancelled)
        ));
    }
    for (log_f, df1, df2) in [
        (1000.0, 63, 10),
        (f64::MAX, 63, 1000),
        (2.0 * 37.6_f64.ln(), 1, 1 << 53),
    ] {
        let control = JobControl::default();
        let (guard, count) = cancel_on(&control, usize::MAX);
        assert!(f_upper_tail(log_f, df1, df2, &control)
            .unwrap()
            .value
            .is_none());
        let checkpoints = count.get();
        drop(guard);
        assert!(checkpoints > 0);
        let control = JobControl::default();
        let (_guard, _) = cancel_on(&control, checkpoints);
        assert!(matches!(
            f_upper_tail(log_f, df1, df2, &control),
            Err(AppError::Cancelled)
        ));
    }
}

#[test]
fn student_log_base_survives_underflow_and_retains_compensated_input() {
    use crate::analysis::inference::{
        arithmetic::TailFloat as D, student_t_log_tail, student_t_log_tail_precise,
    };
    let control = JobControl::default();
    for log_t in [-4.0, 0.0, 2.0, 8.0, 368.0, 1000.0] {
        let scalar = student_t_log_tail(log_t, None, 10.0, &control)
            .unwrap()
            .unwrap();
        let precise = student_t_log_tail_precise(D(log_t, 0.0), None, 10.0, &control)
            .unwrap()
            .unwrap();
        assert_eq!(scalar.log_hi, precise.log_hi);
        assert_eq!(scalar.log_lo, precise.log_lo);
        assert_eq!(scalar.uncertainty, precise.uncertainty);
        assert!(scalar.log_hi.is_finite() && scalar.log_lo.is_finite());
        assert!(scalar.uncertainty.is_finite() && scalar.uncertainty >= 0.0);
        if log_t >= 368.0 {
            assert!(scalar.probability().value.is_none());
        }
    }
    // At this magnitude df=2 has log p = -2 log t to better than 1e-300.
    let base = student_t_log_tail_precise(D(368.0, 0.0), None, 2.0, &control)
        .unwrap()
        .unwrap();
    let shifted = student_t_log_tail_precise(D(368.0, 1e-14), None, 2.0, &control)
        .unwrap()
        .unwrap();
    let delta = D(shifted.log_hi, shifted.log_lo).sub(D(base.log_hi, base.log_lo));
    assert!((delta.0 / -2e-14 - 1.0).abs() < 1e-12);
}

#[test]
fn finite_extreme_logs_and_probability_boundary_are_reasoned() {
    use crate::analysis::inference::{arithmetic::TailFloat as D, StudentLogTail};
    let control = JobControl::default();
    for df1 in [1, 2, 63] {
        for df2 in [1, 37, 1 << 53] {
            assert!(f_upper_tail(f64::MAX, df1, df2, &control)
                .unwrap()
                .value
                .is_none());
            assert_eq!(
                f_upper_tail(-f64::MAX, df1, df2, &control).unwrap().value,
                Some(1.0)
            );
        }
    }
    let edge = D::LN_2.mul(D(-1075.0, 0.0));
    let estimate = StudentLogTail {
        log_hi: edge.0,
        log_lo: edge.1,
        uncertainty: 1e-14,
    }
    .probability();
    assert!(estimate.value.is_none());
    assert!(estimate.reason.unwrap().contains("rounding boundary"));
    for (hi, lo, error) in [
        (f64::NAN, 0.0, 0.0),
        (-1.0, f64::NAN, 0.0),
        (-1.0, 0.0, f64::INFINITY),
        (-1.0, 0.0, -1.0),
    ] {
        assert!(StudentLogTail {
            log_hi: hi,
            log_lo: lo,
            uncertainty: error
        }
        .probability()
        .value
        .is_none());
    }
}

#[test]
fn lower_tail_bound_matches_independent_controls_and_rounding_midpoint() {
    let fixture: serde_json::Value = serde_json::from_str(include_str!(
        "../../../../tests/fixtures/regression-tails.json"
    ))
    .unwrap();
    for row in fixture["cases"].as_array().unwrap() {
        let df1 = row["df1"].as_u64().unwrap();
        let df2 = row["df2"].as_u64().unwrap();
        let b = D(df2 as f64 * 0.5, 0.0);
        let ratio = D(row["log_f"].as_f64().unwrap(), 0.0)
            .add(D(df1 as f64, 0.0).ln())
            .sub(D(df2 as f64, 0.0).ln());
        let log_q = softplus(ratio.neg()).neg();
        let log_z = softplus(ratio).neg();
        let mut bound = LowerTailBound::new(b, df1 % 2 == 1);
        let mut a = if df1 % 2 == 1 { 0.5 } else { 1.0 };
        while a < df1 as f64 * 0.5 {
            bound = bound.advance(b.add(D(a, 0.0)).ln(), D(a + 1.0, 0.0).ln());
            a += 1.0;
        }
        let (log, margin) = bound.log_bound(a, b, log_q, log_z);
        let certified = bound.rounds_to_one(a, b, log_q, log_z);
        println!(
            "BOUND_RESULT {}",
            serde_json::json!({
                "name": row["name"], "log_hi":log.0, "log_lo":log.1,
                "margin":margin, "certified":certified
            })
        );
        assert!(log.0.is_finite() && log.1.is_finite() && margin.is_finite() && margin > 0.0);
        if row["name"].as_str().unwrap().starts_with("near_one_") {
            assert_eq!(certified, row["name"] == "near_one_-1e-07");
            assert_eq!(
                f_upper_tail(
                    row["log_f"].as_f64().unwrap(),
                    df1,
                    df2,
                    &JobControl::default()
                )
                .unwrap()
                .value,
                row["rounded"].as_f64()
            );
        }
    }
}

#[test]
fn certified_one_keeps_zero_increment_and_final_cancellation() {
    for (df1, df2, log_f) in [(1, 1, -230.0), (2, 1, -230.0), (63, 1000, 0.001_f64.ln())] {
        let control = JobControl::default();
        let (guard, count) = cancel_on(&control, usize::MAX);
        assert_eq!(
            f_upper_tail(log_f, df1, df2, &control).unwrap().value,
            Some(1.0)
        );
        let checkpoints = count.get();
        drop(guard);
        for target in 1..=checkpoints {
            let control = JobControl::default();
            let (_guard, _) = cancel_on(&control, target);
            assert!(matches!(
                f_upper_tail(log_f, df1, df2, &control),
                Err(AppError::Cancelled)
            ));
        }
    }
}

#[test]
fn small_student_log_base_keeps_mass_below_probability_rounding() {
    use crate::analysis::inference::{student_t_log_tail, StudentLogTail};
    let base = student_t_log_tail(-100.0, None, 1.0, &JobControl::default())
        .unwrap()
        .unwrap();
    // Cauchy: log(1 - 2 atan(exp(-100))/pi), independently at 160 digits.
    let expected = -2.3682739210444925e-44;
    assert!(
        (base.log_hi / expected - 1.0).abs() < 1e-14,
        "{}",
        base.log_hi
    );
    assert_eq!(base.probability().value, Some(1.0));
    assert!(StudentLogTail {
        log_hi: -1.0,
        log_lo: 0.0,
        uncertainty: 0.1
    }
    .probability()
    .value
    .is_none());
}

#[test]
fn student_log_nonfinite_intermediate_is_a_reasoned_refusal() {
    use crate::analysis::inference::student_t_log_tail;
    let control = JobControl::default();
    let refused = student_t_log_tail(f64::MAX, None, 10.0, &control).unwrap();
    assert!(refused.is_err());
    assert!(!refused.unwrap_err().reason.unwrap().is_empty());
    for exact in [0.0, -1.0, f64::NAN, f64::INFINITY] {
        assert!(student_t_log_tail(0.0, Some(exact), 10.0, &control).is_err());
    }
}

#[test]
fn one_predictor_f_agrees_with_existing_student_inference() {
    use crate::analysis::inference::student_t_two_sided_p;
    let control = JobControl::default();
    for df in [1, 2, 37, 1 << 53] {
        for t in [0.01_f64, 1.0, 2.0, 12.0, 1e150] {
            let student = student_t_two_sided_p(t, df as f64, &control).unwrap();
            let f = f_upper_tail(2.0 * t.ln(), 1, df, &control).unwrap();
            assert_eq!(student.value.is_some(), f.value.is_some());
            if let Some(expected) = student.value {
                assert!((f.value.unwrap() / expected - 1.0).abs() <= 1e-10);
            }
        }
    }
}

#[test]
fn subnormal_f_tail_refuses_uncertainty_larger_than_half_an_ulp() {
    use crate::analysis::inference::student_t_two_sided_p;
    let control = JobControl::default();
    let t = 37.6_f64;
    let legacy = student_t_two_sided_p(t, (1_u64 << 53) as f64, &control).unwrap();
    assert!(legacy
        .value
        .is_some_and(|p| p > 0.0 && p < f64::MIN_POSITIVE));
    let result = f_upper_tail(2.0 * t.ln(), 1, 1 << 53, &control).unwrap();
    assert!(result.value.is_none());
    assert!(result.reason.unwrap().contains("precision"));
}

#[test]
fn scaled_coefficient_keeps_probability_when_t_overflows() {
    let c = coefficient(
        Scaled::new(1e150),
        Some(Scaled::new(1e-159)),
        &Estimate::available(1.),
        1,
        &JobControl::default(),
    )
    .unwrap();
    assert_eq!(
        c.t.reason.as_deref(),
        Some("The estimate is outside finite Float64 range.")
    );
    let expected = 6.36619772367583e-310;
    assert!((c.p.value.unwrap() - expected).abs() <= f64::from_bits(1));
    assert_eq!(c.standard_error.value, Some(1e-159));
}
#[test]
fn scaled_f_keeps_tail_when_squared_statistic_overflows_and_handles_exact_zero() {
    let control = JobControl::default();
    let (f, p) = f_inference(1., 1e-155, 1, 1, &control).unwrap();
    assert_eq!(
        f.reason.as_deref(),
        Some("The estimate is outside finite Float64 range.")
    );
    assert!((p.value.unwrap() / 6.366197723675813e-156 - 1.).abs() < 1e-10);
    let (f, p) = f_inference(0., 1., 1, 1, &control).unwrap();
    assert_eq!(f.value, Some(0.));
    assert_eq!(p.value, Some(1.));
    assert_eq!(
        f_inference(1., 0., 1, 1, &control)
            .unwrap()
            .1
            .reason
            .as_deref(),
        Some(ZERO_VARIANCE)
    );
    assert_eq!(
        f_inference(1., 1., 1, 0, &control)
            .unwrap()
            .1
            .reason
            .as_deref(),
        Some(NO_DF)
    );
}
#[test]
fn coefficient_bounds_have_independent_range_and_df_states() {
    let control = JobControl::default();
    let c = coefficient(
        Scaled::new(1.7e308),
        Some(Scaled::new(1e308)),
        &Estimate::available(1.),
        10,
        &control,
    )
    .unwrap();
    assert!(c.interval[0].value.is_some());
    assert_eq!(
        c.interval[1].reason.as_deref(),
        Some("The estimate is outside finite Float64 range.")
    );
    let c = coefficient(
        Scaled::new(3.),
        None,
        &Estimate::unavailable(NO_DF),
        0,
        &control,
    )
    .unwrap();
    assert_eq!(c.estimate.value, Some(3.));
    for estimate in [
        &c.standard_error,
        &c.t,
        &c.p,
        &c.interval[0],
        &c.interval[1],
    ] {
        assert_eq!(estimate.reason.as_deref(), Some(NO_DF));
    }
}
