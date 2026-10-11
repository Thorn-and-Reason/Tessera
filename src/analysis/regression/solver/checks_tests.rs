use super::*;
use crate::analysis::{ConfidenceLevel, RowFilter};
use crate::data::{Column, ColumnId};

fn reason(result: Result<(), Abort>) -> &'static str {
    match result {
        Err(Abort::Numerical(reason)) => reason,
        _ => panic!("expected numerical refusal"),
    }
}
#[test]
fn factor_shapes_finiteness_singular_energy_and_all_orthogonality_gates() {
    let control = JobControl::default();
    let mut ck = Check::new(&control);
    let id = DMatrix::identity(2, 2);
    assert!(factors(&id, &id, &id, &[1., 1.], &mut ck).is_ok());
    assert_eq!(
        reason(factors(&id, &DMatrix::zeros(2, 1), &id, &[1., 1.], &mut ck)),
        SHAPE
    );
    for v in [f64::NAN, f64::INFINITY] {
        let mut u = id.clone();
        u[(0, 0)] = v;
        assert_eq!(reason(factors(&id, &u, &id, &[1., 1.], &mut ck)), NONFINITE);
    }
    for v in [-1., f64::NAN, f64::INFINITY] {
        assert_eq!(reason(factors(&id, &id, &id, &[v, 1.], &mut ck)), SINGULAR);
    }
    let mut u = id.clone();
    u[(0, 0)] = 2.;
    assert_eq!(
        reason(factors(&id, &u, &id, &[1., 1.], &mut ck)),
        REASONS[0]
    );
    assert_eq!(
        reason(factors(&id, &u, &id, &[0.5, 1.], &mut ck)),
        REASONS[1]
    );
    assert_eq!(
        reason(factors(&id, &id, &u, &[0.5, 1.], &mut ck)),
        REASONS[2]
    );
    assert_eq!(
        reason(factors(&DMatrix::zeros(2, 2), &id, &id, &[1., 1.], &mut ck)),
        REASONS[0]
    );
    assert!(factors(&DMatrix::zeros(2, 2), &id, &id, &[0., 0.], &mut ck).is_ok());
    let diagonal = DMatrix::from_diagonal(&nalgebra::DVector::from_vec(vec![1., 1e-9]));
    assert!(factors(&diagonal, &id, &id, &[1., 1e-9], &mut ck).is_ok());
    assert_eq!(ratio(0., 0.), 0.);
    assert!(!ratio(1., 0.).is_finite());
    for (metric, expected) in REASONS.iter().enumerate() {
        assert_eq!(reason(gate(metric, f64::NAN, 1e-12)), *expected);
        assert_eq!(reason(gate(metric, 1e-2, 1e-12)), *expected);
    }
}

#[test]
fn perturbed_runtime_checks_refuse() {
    for mutation in [
        "none",
        "solve",
        "stationarity",
        "records",
        "energy",
        "leverage",
        "leverage-upper",
        "basis",
        "symmetry",
        "correlation",
        "diagonal",
        "se",
        "norm-zero",
        "norm-nan",
        "basis-nan",
        "correlation-nan",
        "shape",
    ] {
        let data = Dataset::new(vec![
            Column::numeric(
                ColumnId(1),
                "y",
                vec![Some(-2.), Some(0.), Some(0.), Some(2.)],
            )
            .unwrap(),
            Column::numeric(
                ColumnId(2),
                "x",
                vec![Some(-1.), Some(-1.), Some(1.), Some(1.)],
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
        let rows = (0..4)
            .map(|row| Eligible {
                row,
                weight: 1,
                complete: true,
            })
            .collect::<Vec<_>>();
        let terms = [
            RegressionTerm::Intercept,
            RegressionTerm::Numeric {
                column: ColumnId(2),
            },
        ];
        let input = Input {
            dataset: &data,
            request: &request,
            eligible: &rows,
            n: 4,
        };
        let control = JobControl::default();
        let mut ck = Check::new(&control);
        let scales = vec![scaling::scale(&input, Some(&terms[1]), &mut ck)
            .ok()
            .unwrap()];
        let outcome = scaling::scale(&input, None, &mut ck).ok().unwrap();
        let slope = std::f64::consts::FRAC_1_SQRT_2;
        let mut fit = ScaledFit {
            n: 4,
            p: 2,
            scales,
            outcome,
            gamma: vec![0., slope],
            basis: vec![1., 0., 0., 1.],
            original_basis: vec![],
            original_norms: vec![],
            coefficients: vec![],
            residuals: vec![-slope, slope, -slope, slope],
            e: slope,
            explained: slope,
            total: 1.,
            condition: 1.,
        };
        let r = DMatrix::identity(2, 2);
        let rhs = [0., slope];
        let vt = DMatrix::identity(2, 2);
        let s = [1., 1.];
        let mut h = vec![0.5; 4];
        let mut corr = vec![1., 0., 0., 1.];
        let mut norms = vec![1., 1.];
        let mut tail = slope;
        match mutation {
            "solve" => fit.gamma[0] += 0.01,
            "stationarity" => fit.residuals[0] += 0.01,
            "records" => {
                for (a, b) in fit.residuals.iter_mut().zip([1., -1., -1., 1.]) {
                    *a += b * 0.01;
                }
            }
            "energy" => tail += 0.01,
            "leverage" => h[0] = -0.1,
            "leverage-upper" => h = vec![1.1, 0.3, 0.3, 0.3],
            "basis" => fit.basis[0] += 0.1,
            "symmetry" => corr[1] += 0.1,
            "correlation" => {
                corr[1] += 0.1;
                corr[2] += 0.1;
            }
            "diagonal" => corr[0] = -1.,
            "se" => norms[0] *= 1.01,
            "norm-zero" => norms[0] = 0.,
            "norm-nan" => norms[0] = f64::NAN,
            "basis-nan" => fit.basis[0] = f64::NAN,
            "correlation-nan" => corr[1] = f64::NAN,
            "shape" => {
                fit.residuals.pop();
            }
            _ => {}
        }
        let result = super::fit(
            fit,
            &input,
            &terms,
            Evidence {
                r: &r,
                rhs: &rhs,
                h: &h,
                vt: &vt,
                s: &s,
                corr: &corr,
                norms: &norms,
                tail,
            },
            &mut ck,
        );
        if mutation == "none" {
            assert!(result.is_ok());
            continue;
        }
        let reason = match result {
            Err(Abort::Numerical(r)) => r,
            _ => panic!("accepted {mutation}"),
        };
        let expected = match mutation {
            "solve" => REASONS[3],
            "stationarity" => REASONS[4],
            "records" => REASONS[5],
            "energy" => REASONS[6],
            "leverage" => "A per-copy leverage is negative or nonfinite.",
            "leverage-upper" => REASONS[10],
            "basis" => REASONS[11],
            "symmetry" => REASONS[12],
            "correlation" | "diagonal" => REASONS[13],
            "se" => REASONS[15],
            "norm-zero" | "norm-nan" => NORM,
            "basis-nan" | "correlation-nan" => COVFINITE,
            "shape" => SHAPE,
            _ => unreachable!(),
        };
        assert_eq!(reason, expected, "{mutation}");
    }
}

#[test]
fn cancellation_occurs_inside_wide_factor_products_and_long_text_comparisons() {
    use crate::operation::CHECKPOINT_HOOK;
    let control = JobControl::default();
    let cancel = control.clone();
    CHECKPOINT_HOOK.with(|h| {
        *h.borrow_mut() = Some(Box::new(move |p| {
            if p.done >= 20_000 {
                cancel.cancel();
            }
        }))
    });
    let id = DMatrix::identity(64, 64);
    let result = factors(&id, &id, &id, &[1.; 64], &mut Check::new(&control));
    CHECKPOINT_HOOK.with(|h| *h.borrow_mut() = None);
    assert!(matches!(
        result,
        Err(Abort::Operational(AppError::Cancelled))
    ));
    let text = "Welsh/Cymraeg ".repeat(10_000);
    let data = Dataset::new(vec![Column::text(
        ColumnId(1),
        "factor",
        vec![Some(text.clone())],
    )
    .unwrap()])
    .unwrap();
    let term = RegressionTerm::Contrast {
        column: ColumnId(1),
        level: Cell::Text(text),
        reference: Cell::Text(String::new()),
    };
    let control = JobControl::default();
    let cancel = control.clone();
    CHECKPOINT_HOOK.with(|h| *h.borrow_mut() = Some(Box::new(move |_| cancel.cancel())));
    let result = term_value(&data, 0, &term, &mut Check::new(&control));
    CHECKPOINT_HOOK.with(|h| *h.borrow_mut() = None);
    assert!(matches!(
        result,
        Err(Abort::Operational(AppError::Cancelled))
    ));
}
