use super::*;
use crate::{
    analysis::{ConfidenceLevel, Estimate, RowFilter},
    data::{Column, ColumnId, Dataset},
    operation::JobControl,
};

fn data(x: &[f64], y: &[f64], w: &[u64]) -> (Dataset, RegressionRequest) {
    (
        Dataset::new(vec![
            Column::numeric(ColumnId(1), "y", y.iter().copied().map(Some).collect()).unwrap(),
            Column::numeric(ColumnId(2), "x", x.iter().copied().map(Some).collect()).unwrap(),
            Column::numeric(
                ColumnId(3),
                "w",
                w.iter().map(|v| Some(*v as f64)).collect(),
            )
            .unwrap(),
        ])
        .unwrap(),
        RegressionRequest {
            outcome: ColumnId(1),
            predictors: vec![RegressionPredictor {
                column: ColumnId(2),
                role: PredictorRole::Numeric,
            }],
            confidence: ConfidenceLevel::P95,
            filter: RowFilter::default(),
            frequency_weight: Some(ColumnId(3)),
        },
    )
}
fn run(x: &[f64], y: &[f64], w: &[u64]) -> ComputedRegression {
    let (d, r) = data(x, y, w);
    let result =
        regression_with_budget(&d, &r, &JobControl::default(), &[], r.heap_bytes().unwrap())
            .unwrap();
    result.validate().unwrap();
    result
}
fn fit(r: &ComputedRegression) -> &RegressionFit {
    match &r.outcome {
        RegressionOutcome::Fitted(f) => f,
        other => panic!("{other:?}"),
    }
}
fn close(a: f64, b: f64) {
    assert!(
        (a - b).abs() <= 1e-10 * a.abs().max(b.abs()).max(1e-12),
        "{a} != {b}"
    );
}
fn compare(a: &Estimate, b: &Estimate) {
    match (a.value, b.value) {
        (Some(a), Some(b)) => close(a, b),
        (None, None) => assert_eq!(a.reason, b.reason),
        _ => panic!("{a:?} != {b:?}"),
    }
}
#[test]
fn per_copy_influence_matches_replication() {
    let x = [0., 1., 2., 4.];
    let y = [1., 3., 2., 6.];
    let w = [2, 3, 5, 7];
    let compressed = run(&x, &y, &w);
    let mut xx = vec![];
    let mut yy = vec![];
    for i in 0..4 {
        for _ in 0..w[i] {
            xx.push(x[i]);
            yy.push(y[i]);
        }
    }
    let expanded = run(&xx, &yy, &[1; 17]);
    let a = &fit(&compressed).residual_diagnostics;
    let b = &fit(&expanded).residual_diagnostics;
    let mut offset = 0;
    for (i, p) in a.residual_points.iter().enumerate() {
        let q = &b.residual_points[offset];
        for (v, z) in [
            (&p.leverage, &q.leverage),
            (&p.studentised, &q.studentised),
            (&p.cooks_distance, &q.cooks_distance),
        ] {
            compare(v, z);
        }
        // Independent original-design inverse for the replicated least-squares model.
        let mean = 41. / 17.;
        let sxx = 135. - 41. * 41. / 17.;
        let h = 1. / 17. + (x[i] - mean).powi(2) / sxx;
        close(p.leverage.value.unwrap(), h);
        let e = p.residual.value.unwrap();
        let s = fit(&compressed)
            .statistics
            .residual_standard_error
            .value
            .unwrap();
        close(p.studentised.value.unwrap(), e / (s * (1. - h).sqrt()));
        close(
            p.cooks_distance.value.unwrap(),
            e.powi(2) / s.powi(2) * h / (2. * (1. - h).powi(2)),
        );
        let mut deleted = w;
        deleted[i] -= 1;
        let refit = run(&x, &y, &deleted);
        let delta = [0, 1].map(|j| {
            fit(&compressed).coefficients[j].estimate.value.unwrap()
                - fit(&refit).coefficients[j].estimate.value.unwrap()
        });
        let deletion_cook = x
            .iter()
            .zip(w)
            .map(|(x, w)| w as f64 * (delta[0] + delta[1] * x).powi(2))
            .sum::<f64>()
            / (2. * s * s);
        close(p.cooks_distance.value.unwrap(), deletion_cook);
        offset += w[i] as usize;
    }
    compare(&a.residual_sample_sd, &b.residual_sample_sd);
    close(
        a.residual_sample_sd.value.unwrap(),
        fit(&compressed)
            .statistics
            .residual_standard_error
            .value
            .unwrap()
            * (15f64 / 16.).sqrt(),
    );
    assert!(a.qq_points.windows(2).any(|v| v[0].row == v[1].row));
}
#[test]
fn trace_weighted_h_is_p() {
    let r = run(&[0., 1., 2., 4.], &[1., 3., 2., 6.], &[2, 3, 5, 7]);
    close(
        fit(&r)
            .residual_diagnostics
            .residual_points
            .iter()
            .map(|p| p.frequency as f64 * p.leverage.value.unwrap())
            .sum(),
        2.,
    );
}
#[test]
fn full_summary_includes_undisplayed_extreme() {
    let x = (0..2001).map(|i| i as f64).collect::<Vec<_>>();
    let mut y = x.iter().map(|x| 2. + 0.5 * x).collect::<Vec<_>>();
    y[1999] += 10000.;
    let r = run(&x, &y, &vec![1; 2001]);
    let d = &fit(&r).residual_diagnostics;
    assert_eq!(d.residual_points.len(), 2000);
    assert!(!d.residual_points.iter().any(|p| p.row == 1999));
    assert!(
        d.residual_range[1].value.unwrap()
            > d.residual_points
                .iter()
                .filter_map(|p| p.residual.value)
                .fold(f64::NEG_INFINITY, f64::max)
    );
    assert_eq!(d.qq_points.last().unwrap().row, 1999);
    for (extent, values) in [
        (
            &d.studentised,
            d.residual_points
                .iter()
                .filter_map(|p| p.studentised.value)
                .collect::<Vec<_>>(),
        ),
        (
            &d.cooks_distance,
            d.residual_points
                .iter()
                .filter_map(|p| p.cooks_distance.value)
                .collect::<Vec<_>>(),
        ),
    ] {
        assert!(
            extent.range[1].value.unwrap() > values.into_iter().fold(f64::NEG_INFINITY, f64::max)
        );
    }
    assert!(d.residual_mean.value.unwrap().abs() < 1e-9);
    close(
        d.residual_sample_sd.value.unwrap(),
        fit(&r).statistics.residual_standard_error.value.unwrap() * (1999f64 / 2000.).sqrt(),
    );
}
#[test]
fn qq_integer_ranks_and_repeated_row() {
    for n in [1, 2, 17, 1u64 << 53] {
        let k = n.min(2000) as usize;
        let ranks = (0..k)
            .map(|i| qq_rank(n, k, i).unwrap())
            .collect::<Vec<_>>();
        assert_eq!(ranks[0], 0);
        assert_eq!(ranks[k - 1], n - 1);
        assert!(ranks.windows(2).all(|w| w[0] < w[1]));
        for rank in ranks {
            let z = qq_theoretical(n, rank).unwrap();
            assert!(z.is_finite());
            close(z, -qq_theoretical(n, n - 1 - rank).unwrap());
        }
    }
    assert_eq!(qq_theoretical(1, 0).unwrap(), 0.);
    close(qq_theoretical(2, 0).unwrap(), -0.6744897501960817);
    close(qq_theoretical(1 << 53, 0).unwrap(), -8.292361075813597);
}
#[test]
fn undefined_rows_and_frequencies_are_separate() {
    let r = run(&[0., 1., 2.], &[5., 5., 5.], &[2, 3, 7]);
    let d = &fit(&r).residual_diagnostics;
    assert_eq!(
        (
            d.studentised.undefined_rows,
            d.studentised.undefined_frequency
        ),
        (3, 12)
    );
    assert_eq!(
        (
            d.cooks_distance.undefined_rows,
            d.cooks_distance.undefined_frequency
        ),
        (3, 12)
    );
    assert_eq!(
        d.studentised.range[0].reason.as_deref(),
        Some("Residual variance is exactly zero.")
    );
}

#[test]
fn single_observation_is_unidentified_not_a_fitted_qq_plot() {
    let result = run(&[1.], &[2.], &[1]);
    assert!(matches!(
        result.outcome,
        RegressionOutcome::Unavailable(RegressionFailure {
            kind: RegressionFailureKind::Unidentified,
            ..
        })
    ));
}

#[test]
fn cancel_every_direct_checkpoint_before_return() {
    use crate::{error::AppError, operation::CHECKPOINT_HOOK};
    use std::{cell::Cell, rc::Rc};
    let (d, r) = data(&[0., 1., 2., 4.], &[1., 3., 2., 6.], &[2, 3, 5, 7]);
    let calls = Rc::new(Cell::new(0));
    let observed = calls.clone();
    CHECKPOINT_HOOK.with(|hook| {
        *hook.borrow_mut() = Some(Box::new(move |_| observed.set(observed.get() + 1)))
    });
    regression_with_budget(&d, &r, &JobControl::default(), &[], r.heap_bytes().unwrap()).unwrap();
    CHECKPOINT_HOOK.with(|hook| *hook.borrow_mut() = None);
    for target in 1..=calls.get() {
        let control = JobControl::default();
        let cancel = control.clone();
        let count = Cell::new(0);
        CHECKPOINT_HOOK.with(|hook| {
            *hook.borrow_mut() = Some(Box::new(move |_| {
                count.set(count.get() + 1);
                if count.get() == target {
                    cancel.cancel();
                }
            }))
        });
        let result = regression_with_budget(&d, &r, &control, &[], r.heap_bytes().unwrap());
        CHECKPOINT_HOOK.with(|hook| *hook.borrow_mut() = None);
        assert!(
            matches!(result, Err(AppError::Cancelled)),
            "checkpoint {target}/{}",
            calls.get()
        );
    }
    eprintln!("DIRECT_CANCELLATION_CHECKPOINTS {}", calls.get());
}

#[test]
fn cancel_sort_and_factor() {
    use crate::{analysis::selection::select_rows, error::AppError, operation::CHECKPOINT_HOOK};
    // A 64-term orthogonal design makes an individual diagnostic factor product long.
    let m = 128usize;
    let mut columns = vec![Column::numeric(
        ColumnId(1),
        "y",
        (0..m).map(|i| Some(2. + (i as f64) * 0.01)).collect(),
    )
    .unwrap()];
    let mut predictors = vec![];
    for j in 1..64 {
        let id = ColumnId(j + 1);
        columns.push(
            Column::numeric(
                id,
                &format!("x{j}"),
                (0..m)
                    .map(|i| {
                        Some(if (i & j as usize).count_ones().is_multiple_of(2) {
                            1.
                        } else {
                            -1.
                        })
                    })
                    .collect(),
            )
            .unwrap(),
        );
        predictors.push(RegressionPredictor {
            column: id,
            role: PredictorRole::Numeric,
        });
    }
    let d = Dataset::new(columns).unwrap();
    let r = RegressionRequest {
        outcome: ColumnId(1),
        predictors,
        confidence: ConfidenceLevel::P95,
        filter: RowFilter::default(),
        frequency_weight: None,
    };
    let ids = (1..=64).map(ColumnId).collect::<Vec<_>>();
    let c = JobControl::default();
    let (eligible, _) = select_rows(&d, &r.filter, None, &ids, &c, m).unwrap();
    let live = crate::data::Budget::dataset_bytes(
        &[&d],
        r.heap_bytes().unwrap()
            + eligible.capacity() * std::mem::size_of::<crate::analysis::selection::Eligible>()
            + ids.capacity() * std::mem::size_of::<ColumnId>(),
    )
    .unwrap();
    let (levels, terms, _) = super::super::coding::discover(&d, &r, &eligible, &c, live).unwrap();
    crate::data::Budget::check_datasets(
        &[&d],
        r.heap_bytes().unwrap()
            + eligible.capacity() * std::mem::size_of::<crate::analysis::selection::Eligible>()
            + ids.capacity() * std::mem::size_of::<ColumnId>()
            + super::super::accounting::coding_bytes(&levels, &terms).unwrap()
            + super::super::accounting::kernel_work_bytes(m, terms.len()).unwrap()
            + super::super::accounting::diagnostics_work_bytes(m, m as u64, terms.len()).unwrap(),
    )
    .unwrap();
    let solver::KernelOutcome::Fitted(scaled) =
        solver::solve(&d, &r, &eligible, &levels, &terms, &c).unwrap()
    else {
        panic!("fit required")
    };
    let control = JobControl::default();
    let cancel = control.clone();
    CHECKPOINT_HOOK.with(|hook| {
        *hook.borrow_mut() = Some(Box::new(move |p| {
            // Prediction stops at 64 operations; 128 is inside the leverage factor product.
            if p.done == 128 && p.total.is_none() {
                cancel.cancel()
            }
        }))
    });
    let result = compute(&d, &r, &eligible, &levels, &terms, &scaled, &control);
    CHECKPOINT_HOOK.with(|hook| *hook.borrow_mut() = None);
    assert!(matches!(result, Err(AppError::Cancelled)));
    let control = JobControl::default();
    let cancel = control.clone();
    CHECKPOINT_HOOK.with(|hook| {
        *hook.borrow_mut() = Some(Box::new(move |p| {
            if p.total == Some(m) {
                cancel.cancel()
            }
        }))
    });
    let result = compute(&d, &r, &eligible, &levels, &terms, &scaled, &control);
    CHECKPOINT_HOOK.with(|hook| *hook.borrow_mut() = None);
    assert!(matches!(result, Err(AppError::Cancelled)));
}

#[test]
fn unit_changes_preserve_dimensionless_diagnostics_when_squared_energy_is_unavailable() {
    let base = run(&[0., 1., 2., 4.], &[1., 3., 2., 6.], &[2, 3, 5, 7]);
    for scale in [1e-200, 1e200] {
        let y = [1., 3., 2., 6.].map(|y| y * scale);
        let current = run(&[0., 1., 2., 4.], &y, &[2, 3, 5, 7]);
        assert!(fit(&current).statistics.sse.value.is_none());
        let a = &fit(&base).residual_diagnostics;
        let b = &fit(&current).residual_diagnostics;
        close(
            b.residual_sample_sd.value.unwrap() / scale,
            a.residual_sample_sd.value.unwrap(),
        );
        for (a, b) in a.residual_points.iter().zip(&b.residual_points) {
            close(b.residual.value.unwrap() / scale, a.residual.value.unwrap());
            compare(&a.leverage, &b.leverage);
            compare(&a.studentised, &b.studentised);
            compare(&a.cooks_distance, &b.cooks_distance);
        }
    }
}

#[test]
fn leverage_at_one_and_no_degrees_have_precise_separate_reasons() {
    let result = run(&[0., 0., 0., 1.], &[1., 2., 3., 4.], &[2, 3, 5, 1]);
    let d = &fit(&result).residual_diagnostics;
    let last = d.residual_points.last().unwrap();
    assert!(last.leverage.value.unwrap() >= 1.);
    assert_eq!(last.studentised.reason.as_deref(), Some(DOMAIN));
    assert_eq!(last.cooks_distance.reason.as_deref(), Some(DOMAIN));
    assert_eq!(
        (
            d.studentised.undefined_rows,
            d.studentised.undefined_frequency
        ),
        (1, 1)
    );
    let result = run(&[0., 1.], &[1., 2.], &[1, 1]);
    let d = &fit(&result).residual_diagnostics;
    assert!(d.residual_sample_sd.value.is_some());
    assert_eq!(d.studentised.range[0].reason.as_deref(), Some(NO_DF));
    assert_eq!(
        (
            d.studentised.undefined_rows,
            d.studentised.undefined_frequency
        ),
        (2, 2)
    );
}

#[test]
fn extra_and_retained_budget_refuse_before_source_copy_or_kernel() {
    use crate::{
        data::{Budget, MAX_OWNED_BYTES},
        error::AppError,
        operation::CHECKPOINT_HOOK,
    };
    use std::{cell::Cell, rc::Rc};
    let (dataset, request) = data(&[0., 1., 2., 4.], &[1., 3., 2., 6.], &[2, 3, 5, 7]);
    let retained = Dataset::new(vec![Column::text(
        ColumnId(9),
        "prior",
        vec![Some("prior".repeat(20000))],
    )
    .unwrap()])
    .unwrap();
    let baseline = Budget::dataset_bytes(&[&dataset, &retained], 0).unwrap();
    let calls = Rc::new(Cell::new(0));
    let observed = calls.clone();
    CHECKPOINT_HOOK.with(|hook| {
        *hook.borrow_mut() = Some(Box::new(move |_| observed.set(observed.get() + 1)))
    });
    let result = regression_with_budget(
        &dataset,
        &request,
        &JobControl::default(),
        &[&retained],
        MAX_OWNED_BYTES - baseline + 1,
    );
    CHECKPOINT_HOOK.with(|hook| *hook.borrow_mut() = None);
    assert!(matches!(result, Err(AppError::SizeLimit { .. })));
    assert_eq!(
        calls.get(),
        2,
        "only entry and terminal checks; no capture or kernel"
    );
}
