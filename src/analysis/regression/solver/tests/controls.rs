use super::*;

#[test]
fn scaling_cancels_while_scanning_incomplete_rows() {
    use crate::operation::CHECKPOINT_HOOK;
    let (dataset, request) = request_case(&case_named("simple"));
    let rows = (0..4096)
        .map(|row| Eligible {
            row,
            weight: 1,
            complete: false,
        })
        .collect::<Vec<_>>();
    let input = Input {
        dataset: &dataset,
        request: &request,
        eligible: &rows,
        n: 1,
    };
    let control = JobControl::default();
    let cancel = control.clone();
    CHECKPOINT_HOOK.with(|h| *h.borrow_mut() = Some(Box::new(move |_| cancel.cancel())));
    let result = scaling::scale(&input, None, &mut Check::new(&control));
    CHECKPOINT_HOOK.with(|h| *h.borrow_mut() = None);
    assert!(matches!(
        result,
        Err(Abort::Operational(AppError::Cancelled))
    ));
}

#[test]
fn private_diagnostic_accessors_preserve_rows_prediction_and_per_copy_leverage() {
    let case = case_named("weighted");
    let control = JobControl::default();
    let KernelOutcome::Fitted(fit) = run_case(&case, &control).unwrap() else {
        panic!("fit")
    };
    let (dataset, request) = request_case(&case);
    let terms = std::iter::once(RegressionTerm::Intercept)
        .chain(
            request
                .predictors
                .iter()
                .map(|p| RegressionTerm::Numeric { column: p.column }),
        )
        .collect::<Vec<_>>();
    let mut row = vec![0.; fit.rank()];
    let mut trace = 0.;
    for (i, weight) in case["input"]["w"].as_array().unwrap().iter().enumerate() {
        fit.row(&dataset, &terms, i, &mut row, &control).unwrap();
        let pred = fit.predict(&row, &control).unwrap();
        scalar(
            &fit.fitted_estimate(pred),
            &case["reference"]["fit"]["fitted"][i],
            1e-10,
            "accessor/fitted",
        );
        let leverage = fit.leverage(&row, &control).unwrap();
        assert!((0.0..=1.0).contains(&leverage));
        trace += leverage * weight.as_u64().unwrap() as f64;
    }
    assert!((trace - fit.rank() as f64).abs() < 1e-12);
    for j in 0..fit.rank() {
        assert!(fit.term_unit(j).value() > 0.);
    }
    assert!(fit.row(&dataset, &terms, 0, &mut [], &control).is_err());
    assert!(fit.predict(&[], &control).is_err());
    assert!(fit.leverage(&[], &control).is_err());
}

#[test]
fn real_job_control_cancels_wide_work_and_every_terminal() {
    use crate::operation::CHECKPOINT_HOOK;
    use std::{cell::Cell as Count, rc::Rc};
    let mut case = case_named("numeric");
    case["input"]["x"] = serde_json::json!((0..70)
        .map(|i| (0..31)
            .map(|j| (((i + 3) * (j + 7)) % 73) as f64)
            .collect::<Vec<_>>())
        .collect::<Vec<_>>());
    case["input"]["y"] = serde_json::json!((0..70).map(|i| i as f64).collect::<Vec<_>>());
    case["input"]["w"] = serde_json::json!(vec![1; 70]);
    let count = Rc::new(Count::new(0));
    let observed = count.clone();
    CHECKPOINT_HOOK
        .with(|h| *h.borrow_mut() = Some(Box::new(move |_| observed.set(observed.get() + 1))));
    let _ = run_case(&case, &JobControl::default());
    CHECKPOINT_HOOK.with(|h| *h.borrow_mut() = None);
    assert!(count.get() > 200);
    for target in [
        1,
        30,
        100,
        200,
        500,
        count.get() / 2,
        count.get() - 1,
        count.get(),
    ] {
        let control = JobControl::default();
        let cancel = control.clone();
        let calls = Rc::new(Count::new(0));
        let observed = calls.clone();
        CHECKPOINT_HOOK.with(|h| {
            *h.borrow_mut() = Some(Box::new(move |_| {
                observed.set(observed.get() + 1);
                if observed.get() == target {
                    cancel.cancel();
                }
            }))
        });
        let result = run_case(&case, &control);
        CHECKPOINT_HOOK.with(|h| *h.borrow_mut() = None);
        assert!(
            matches!(result, Err(AppError::Cancelled)),
            "checkpoint {target}"
        );
    }
    for name in ["difference_overflow", "near_1e-13", "simple"] {
        let case = case_named(name);
        let count = Rc::new(Count::new(0));
        let observed = count.clone();
        CHECKPOINT_HOOK
            .with(|h| *h.borrow_mut() = Some(Box::new(move |_| observed.set(observed.get() + 1))));
        let _ = run_case(&case, &JobControl::default());
        CHECKPOINT_HOOK.with(|h| *h.borrow_mut() = None);
        let target = count.get();
        let control = JobControl::default();
        let cancel = control.clone();
        let calls = Rc::new(Count::new(0));
        let observed = calls.clone();
        CHECKPOINT_HOOK.with(|h| {
            *h.borrow_mut() = Some(Box::new(move |_| {
                observed.set(observed.get() + 1);
                if observed.get() == target {
                    cancel.cancel();
                }
            }))
        });
        let result = run_case(&case, &control);
        CHECKPOINT_HOOK.with(|h| *h.borrow_mut() = None);
        assert!(
            matches!(result, Err(AppError::Cancelled)),
            "terminal {name}"
        );
    }
}
#[test]
fn bounded_svd_failure_and_post_call_cancellation_are_genuine() {
    use crate::operation::CHECKPOINT_HOOK;
    use std::{cell::Cell as Count, rc::Rc};
    let hilbert = || DMatrix::from_fn(8, 8, |i, j| 1. / (i + j + 1) as f64);
    let control = JobControl::default();
    assert!(matches!(
        decompose(hilbert(), 0., 1, &Check::new(&control)),
        Err(Abort::Nonconvergence)
    ));
    for failure in [false, true] {
        let control = JobControl::default();
        let cancel = control.clone();
        let calls = Rc::new(Count::new(0));
        let seen = calls.clone();
        CHECKPOINT_HOOK.with(|h| {
            *h.borrow_mut() = Some(Box::new(move |_| {
                seen.set(seen.get() + 1);
                if seen.get() == 2 {
                    cancel.cancel();
                }
            }))
        });
        let result = if failure {
            decompose(hilbert(), 0., 1, &Check::new(&control))
        } else {
            decompose(
                DMatrix::identity(2, 2),
                5. * f64::EPSILON,
                512,
                &Check::new(&control),
            )
        };
        CHECKPOINT_HOOK.with(|h| *h.borrow_mut() = None);
        assert_eq!(calls.get(), 2);
        assert!(matches!(
            result,
            Err(Abort::Operational(AppError::Cancelled))
        ));
    }
}

#[test]
fn named_high_condition_probability_inputs() {
    for name in ["reference_near_A_3.55e-13", "reference_near_C_2e-12"] {
        let case = case_named(name);
        let control = JobControl::default();
        let fit = match run_case(&case, &control).unwrap() {
            KernelOutcome::Fitted(fit) => fit,
            KernelOutcome::Unavailable(failure) => {
                assert!(
                    portable_reconstruction_refusal(name, &failure),
                    "fit {name}: {}",
                    failure.reason
                );
                continue;
            }
        };
        let (coefficients, stats) =
            super::super::inference::infer(&fit, ConfidenceLevel::P95, &control).unwrap();
        let ratio = Scaled::new(fit.explained_norm()).ratio(Scaled::new(fit.residual_norm()));
        let f = super::scaling::multiply(ratio, ratio)
            .times(fit.residual_df() as f64 / (fit.rank() - 1) as f64);
        let log = super::scaling::log(f);
        println!(
            "FIT_F_RESULT {}",
            serde_json::json!({"name":name,"df1":fit.rank()-1,"df2":fit.residual_df(),"f":stats.f.value,"log_f":log.0+log.1,"p":stats.f_p.value,"reason":stats.f_p.reason})
        );
        for (j, c) in coefficients.iter().enumerate() {
            let t = fit.coefficients()[j].ratio(fit.standard_error(j));
            let log = super::scaling::log(t);
            println!(
                "FIT_TAIL_RESULT {}",
                serde_json::json!({"name":name,"coefficient":j,"df":fit.residual_df(),"t":c.t.value,"log_hi":log.0,"log_lo":log.1,"p":c.p.value,"reason":c.p.reason})
            );
            scalar(
                &c.t,
                &case["reference"]["fit"]["coefficients"][j]["t"],
                3e-4,
                &format!("{name}/{j}/t"),
            );
        }
    }
}

#[test]
fn sixty_four_terms_fit_with_precharged_orthogonal_design() {
    let mut case = case_named("simple");
    let sign = |i: usize, j: usize| {
        if (i & j).count_ones().is_multiple_of(2) {
            1.
        } else {
            -1.
        }
    };
    case["input"]["x"] = serde_json::json!((0..128)
        .map(|i| (1..64).map(|j| sign(i, j)).collect::<Vec<_>>())
        .collect::<Vec<_>>());
    case["input"]["y"] = serde_json::json!((0..128)
        .map(|i| 2. + (1..64).map(|j| sign(i, j) / j as f64).sum::<f64>() + 0.3 * sign(i, 64))
        .collect::<Vec<_>>());
    case["input"]["w"] = serde_json::json!(vec![1; 128]);
    let control = JobControl::default();
    let KernelOutcome::Fitted(fit) = run_case(&case, &control).unwrap() else {
        panic!("full rank orthogonal design must fit")
    };
    assert_eq!(fit.rank(), 64);
    assert_eq!(fit.residual_df(), 64);
    assert!((fit.condition() - 1.).abs() < 1e-12);
    let (coefficients, statistics) =
        super::super::inference::infer(&fit, ConfidenceLevel::P95, &control).unwrap();
    for (j, c) in coefficients.iter().enumerate() {
        let expected = if j == 0 { 2. } else { 1. / j as f64 };
        assert!((c.estimate.value.unwrap() / expected - 1.).abs() < 1e-10);
        assert!((c.standard_error.value.unwrap() / 0.0375 - 1.).abs() < 1e-10);
    }
    assert!((statistics.sse.value.unwrap() / 11.52 - 1.).abs() < 1e-10);
}

#[test]
fn strict_rank_and_warning_boundaries_use_declared_rules() {
    for (small, expected) in [
        (1e-12f64, 1),
        ((1e-12f64).next_up(), 2),
        ((1e-12f64).next_down(), 1),
    ] {
        assert_eq!(rank_and_condition(&[1., small]).0, expected);
    }
    let KernelOutcome::Fitted(mut fit) =
        run_case(&case_named("simple"), &JobControl::default()).unwrap()
    else {
        panic!("fit")
    };
    for (value, expected) in [
        (1e8f64, false),
        ((1e8f64).next_up(), true),
        ((1e8f64).next_down(), false),
    ] {
        fit.condition = value;
        assert_eq!(fit.condition_warned(), expected);
    }
}
#[test]
fn negative_adjusted_r_squared_is_retained() {
    let mut case = case_named("simple");
    case["input"]["y"] = serde_json::json!([1., -1., -1., 1.]);
    let control = JobControl::default();
    let KernelOutcome::Fitted(fit) = run_case(&case, &control).unwrap() else {
        panic!("fit")
    };
    let (_, statistics) =
        super::super::inference::infer(&fit, ConfidenceLevel::P95, &control).unwrap();
    assert!((statistics.adjusted_r_squared.value.unwrap() + 0.5).abs() < 1e-12);
}
