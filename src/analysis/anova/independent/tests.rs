use super::*;
use crate::analysis::regression::f_upper_tail;
use crate::{
    analysis::{ConfidenceLevel, RowFilter},
    data::{Column, ColumnId},
    error::AppError,
    operation::CHECKPOINT_HOOK,
};
fn request(factors: usize, weighted: bool) -> AnovaRequest {
    AnovaRequest {
        design: if factors == 1 {
            AnovaDesign::OneWay {
                outcome: ColumnId(1),
                factor: ColumnId(2),
            }
        } else {
            AnovaDesign::Factorial {
                outcome: ColumnId(1),
                factors: (0..factors).map(|i| ColumnId(i as u64 + 2)).collect(),
            }
        },
        confidence: ConfidenceLevel::P95,
        filter: RowFilter::default(),
        frequency_weight: weighted.then_some(ColumnId(20)),
        planned_pairs: vec![],
        all_pairs: vec![],
    }
}
fn dataset(rows: &[(Vec<f64>, f64, u64)]) -> Dataset {
    let mut columns = vec![Column::numeric(
        ColumnId(1),
        "Outcome",
        rows.iter().map(|r| Some(r.1)).collect(),
    )
    .unwrap()];
    for j in 0..rows[0].0.len() {
        columns.push(
            Column::numeric(
                ColumnId(j as u64 + 2),
                &format!("Factor {j}"),
                rows.iter().map(|r| Some(r.0[j])).collect(),
            )
            .unwrap(),
        );
    }
    columns.push(
        Column::numeric(
            ColumnId(20),
            "Frequency",
            rows.iter().map(|r| Some(r.2 as f64)).collect(),
        )
        .unwrap(),
    );
    Dataset::new(columns).unwrap()
}
fn run(rows: &[(Vec<f64>, f64, u64)], weighted: bool) -> IndependentResult {
    let d = dataset(rows);
    let r = request(rows[0].0.len(), weighted);
    let c = JobControl::default();
    let s = cohort::select(&d, &r, &c, 0).unwrap();
    compute(&d, &r, &s, &c, 0).unwrap()
}
fn fitted(result: &IndependentResult) -> &AnovaModelResult {
    match &result.outcome {
        AnovaOutcome::Fitted(m) => m,
        _ => panic!("unavailable model"),
    }
}
fn close(actual: f64, expected: f64) {
    assert!(
        (actual - expected).abs() <= 1e-10 * expected.abs().max(1.),
        "{actual:e} != {expected:e}"
    );
}
#[test]
fn balanced_one_way_complete_inference() {
    let rows = vec![
        (vec![0.], 1., 1),
        (vec![0.], 2., 1),
        (vec![0.], 3., 1),
        (vec![1.], 4., 1),
        (vec![1.], 5., 1),
        (vec![1.], 6., 1),
        (vec![2.], 7., 1),
        (vec![2.], 8., 1),
        (vec![2.], 9., 1),
    ];
    let result = run(&rows, false);
    let m = fitted(&result);
    close(m.error_ss.value.value.unwrap(), 6.);
    close(m.model_ss.value.value.unwrap(), 54.);
    close(m.total_ss.value.value.unwrap(), 60.);
    let e = &m.effects[0];
    close(e.test.f.value.unwrap(), 27.);
    close(e.test.p.value.unwrap(), 0.001);
    close(e.eta_squared.value.unwrap(), 0.9);
    close(e.partial_eta_squared.value.unwrap(), 0.9);
}
#[test]
fn unequal_counts_and_literal_frequency_replication() {
    let compressed = vec![
        (vec![0.], 1., 2),
        (vec![0.], 3., 3),
        (vec![1.], 4., 1),
        (vec![1.], 8., 4),
        (vec![2.], 2., 2),
        (vec![2.], 9., 1),
    ];
    let expanded = compressed
        .iter()
        .flat_map(|(g, y, w)| (0..*w).map(move |_| (g.clone(), *y, 1)))
        .collect::<Vec<_>>();
    let a = run(&compressed, true);
    let b = run(&expanded, false);
    let (a, b) = (fitted(&a), fitted(&b));
    close(
        a.total_ss.value.value.unwrap(),
        b.total_ss.value.value.unwrap(),
    );
    close(
        a.effects[0].ss.value.value.unwrap(),
        b.effects[0].ss.value.value.unwrap(),
    );
    close(
        a.effects[0].test.p.value.unwrap(),
        b.effects[0].test.p.value.unwrap(),
    );
    assert_eq!(a.residual_df, 10);
}
#[test]
fn zero_numerator_zero_error_and_insufficient_df() {
    let result = run(
        &[
            (vec![0.], -1., 1),
            (vec![0.], 1., 1),
            (vec![1.], -1., 1),
            (vec![1.], 1., 1),
        ],
        false,
    );
    let e = &fitted(&result).effects[0];
    assert_eq!(e.ss.state, AnovaQuantityState::ExactZero);
    assert_eq!(e.test.f.value, Some(0.));
    assert_eq!(e.test.p.value, Some(1.));
    let result = run(
        &[
            (vec![0.], 1., 1),
            (vec![0.], 1., 1),
            (vec![1.], 2., 1),
            (vec![1.], 2., 1),
        ],
        false,
    );
    let e = &fitted(&result).effects[0];
    assert!(e.test.f.value.is_none());
    assert!(e.test.p.value.is_none());
    assert_eq!(
        e.failure.as_ref().unwrap().kind,
        AnovaFailureKind::ZeroError
    );
    let result = run(&[(vec![0.], 1., 1), (vec![1.], 2., 1)], false);
    assert_eq!(fitted(&result).residual_df, 0);
    assert!(fitted(&result).effects[0].test.p.value.is_none());
}
#[test]
fn offset_extreme_units_and_empty_grid() {
    let original = vec![
        (vec![0.], 1., 1),
        (vec![0.], 3., 1),
        (vec![1.], 4., 1),
        (vec![1.], 6., 1),
    ];
    let base = run(&original, false);
    let f = fitted(&base).effects[0].test.f.value.unwrap();
    for (offset, scale) in [
        (2f64.powi(45), 1.),
        (0., 2f64.powi(1000)),
        (0., f64::from_bits(1)),
    ] {
        let rows = original
            .iter()
            .map(|(g, y, w)| (g.clone(), offset + y * scale, *w))
            .collect::<Vec<_>>();
        let result = run(&rows, false);
        close(fitted(&result).effects[0].test.f.value.unwrap(), f);
        close(
            fitted(&result).effects[0].test.p.value.unwrap(),
            fitted(&base).effects[0].test.p.value.unwrap(),
        );
        if scale != 1. {
            assert_eq!(fitted(&result).error_ss.state, AnovaQuantityState::Positive);
            assert!(fitted(&result).error_ss.value.value.is_none());
        }
    }
    let result = run(
        &[
            (vec![0., 0.], 1., 1),
            (vec![0., 1.], 2., 1),
            (vec![1., 0.], 3., 1),
        ],
        false,
    );
    assert!(
        matches!(&result.outcome,AnovaOutcome::Unavailable(f) if f.kind==AnovaFailureKind::EmptyCells)
    );
    assert_eq!(result.cell_means.len(), 4);
    assert!(result.cell_means[3].value.is_none());
}
#[test]
fn factorial_interactions_and_unbalanced_nonpartition() {
    let mut rows = Vec::new();
    for a in 0..2 {
        for b in 0..3 {
            for delta in [-1., 1.] {
                rows.push((
                    vec![a as f64, b as f64],
                    (3 * a + 2 * b + a * b) as f64 + delta,
                    1,
                ));
            }
        }
    }
    let result = run(&rows, false);
    let m = fitted(&result);
    assert_eq!(m.effects.len(), 3);
    assert_eq!(
        m.effects.iter().map(|e| e.dimension).collect::<Vec<_>>(),
        vec![1, 2, 2]
    );
    close(
        m.effects.iter().map(|e| e.ss.value.value.unwrap()).sum(),
        m.model_ss.value.value.unwrap(),
    );
    for (i, row) in rows.iter_mut().enumerate() {
        row.2 = (i % 5 + 1) as u64;
    }
    let result = run(&rows, true);
    let m = fitted(&result);
    assert!(!m.balanced);
    let sum = m
        .effects
        .iter()
        .map(|e| e.ss.value.value.unwrap())
        .sum::<f64>();
    assert!((sum - m.model_ss.value.value.unwrap()).abs() > 1.);
}
#[test]
fn budget_and_real_control_cancellation() {
    let rows = vec![
        (vec![0.], 1., 1),
        (vec![0.], 2., 1),
        (vec![1.], 3., 1),
        (vec![1.], 4., 1),
    ];
    let d = dataset(&rows);
    let r = request(1, false);
    let s = cohort::select(&d, &r, &JobControl::default(), 0).unwrap();
    assert!(matches!(
        compute(&d, &r, &s, &JobControl::default(), 256 * 1024 * 1024),
        Err(AppError::SizeLimit { .. })
    ));
    let c = JobControl::default();
    let cancel = c.clone();
    CHECKPOINT_HOOK.with(|h| {
        *h.borrow_mut() = Some(Box::new(move |p| {
            if p.done == 2 {
                cancel.cancel()
            }
        }))
    });
    let result = compute(&d, &r, &s, &c, 0);
    CHECKPOINT_HOOK.with(|h| *h.borrow_mut() = None);
    assert!(matches!(result, Err(AppError::Cancelled)));
}
#[test]
fn independent_exact_input_rational_reference_fixtures() {
    let file: serde_json::Value = serde_json::from_str(include_str!(
        "../../../../tests/fixtures/anova-independent.json"
    ))
    .unwrap();
    let mut available = 0;
    let mut refused = 0;
    let (mut p_referenced, mut p_published) = (0, 0);
    for fixture in file["fixtures"].as_array().unwrap() {
        let name = fixture["name"].as_str().unwrap();
        let rows = fixture["rows"]
            .as_array()
            .unwrap()
            .iter()
            .map(|r| {
                (
                    r["g"]
                        .as_array()
                        .unwrap()
                        .iter()
                        .map(|v| v.as_f64().unwrap())
                        .collect::<Vec<_>>(),
                    f64::from_bits(r["bits"].as_u64().unwrap()),
                    r["w"].as_u64().unwrap(),
                )
            })
            .collect::<Vec<_>>();
        let result = run(&rows, fixture["weighted"].as_bool().unwrap());
        let model = fitted(&result);
        assert_eq!(model.residual_df, fixture["df"].as_u64().unwrap());
        let numerical = result.numerical.as_ref().unwrap();
        if let Some(expected) = fixture["sse_log"].as_f64() {
            assert!(
                (numerical.error.value.log_abs() - expected).abs() < 1e-10,
                "{name}: error log"
            );
        }
        for (effect, reference) in model
            .effects
            .iter()
            .zip(fixture["effects"].as_array().unwrap())
        {
            assert_eq!(effect.dimension, reference["q"].as_u64().unwrap() as usize);
            if reference["p"].as_f64().is_some() {
                p_referenced += 1;
                if effect.test.p.value.is_some() {
                    p_published += 1;
                } else {
                    eprintln!(
                        "p withheld {name}, factors {:?}: {:?}",
                        effect.factors, effect.test.p.reason
                    );
                }
            }
            match effect.ss.state {
                AnovaQuantityState::Unavailable(_) => {
                    eprintln!("refused {name}: {:?}", effect.failure);
                    refused += 1;
                    continue;
                }
                _ => available += 1,
            }
            if let Some(expected) = reference["ss"].as_f64().filter(|v| *v > 0.) {
                if let Some(actual) = effect.ss.value.value {
                    assert!(
                        (actual / expected - 1.).abs() < 1e-8,
                        "{name}: ss {actual:e} {expected:e}"
                    );
                }
            }
            for (actual, key) in [
                (&effect.test.f, "f"),
                (&effect.test.p, "p"),
                (&effect.eta_squared, "eta"),
                (&effect.partial_eta_squared, "partial"),
            ] {
                if let (Some(a), Some(expected)) = (actual.value, reference[key].as_f64()) {
                    assert!(
                        (a - expected).abs() <= 1e-8 * expected.abs().max(f64::MIN_POSITIVE),
                        "{name}: {key} {a:e} {expected:e}"
                    );
                }
            }
        }
    }
    assert_eq!((p_referenced, p_published), (83, 75));
    eprintln!("ordinary referenced probabilities: {p_published}/{p_referenced} published");
    assert_eq!(available, 90);
    assert_eq!(refused, 0);
    eprintln!("rational fixtures: available {available}, legitimate refused {refused}");
}
#[test]
fn integer_f_upstream_sensitivity_and_publication_dependencies() {
    let c = JobControl::default();
    let ss = Energy {
        value: Scaled::new(54.),
        relative_error: 1e-13,
    };
    let error = Energy {
        value: Scaled::new(6.),
        relative_error: 1e-13,
    };
    let tight = integer_f_test(ss, error, 2, 6, &c).unwrap();
    close(tight.p.value.unwrap(), 0.001);
    for (allowance, available) in [(8e-12, true), (1e-11, false)] {
        let t = integer_f_test(
            Energy {
                relative_error: allowance,
                ..ss
            },
            Energy {
                relative_error: allowance,
                ..error
            },
            2,
            6,
            &c,
        )
        .unwrap();
        assert_eq!(t.p.value.is_some(), available);
    }
    let loose = integer_f_test(
        Energy {
            relative_error: MAX_RELATIVE,
            ..ss
        },
        Energy {
            relative_error: MAX_RELATIVE,
            ..error
        },
        2,
        6,
        &c,
    )
    .unwrap();
    assert!(loose.f.value.is_some());
    assert!(loose.p.value.is_none());
    let extreme = integer_f_test(
        Energy {
            value: Scaled {
                fraction: 1.,
                exponent: 4000,
            },
            relative_error: 0.,
        },
        error,
        2,
        6,
        &c,
    )
    .unwrap();
    assert!(extreme.f.value.is_none());
    assert!(extreme.p.value.is_none());
    let tiny = integer_f_test(
        Energy {
            value: Scaled {
                fraction: 1.,
                exponent: -4000,
            },
            relative_error: 0.,
        },
        error,
        2,
        6,
        &c,
    )
    .unwrap();
    assert!(tiny.f.value.is_none());
    assert_eq!(tiny.p.value, Some(1.));
    // This ordinary SS overflows publication but its carried F is finite.
    let huge_ss = Energy {
        value: Scaled {
            fraction: 1.,
            exponent: 2048,
        },
        relative_error: 0.,
    };
    let huge_error = Energy {
        value: Scaled {
            fraction: 1.,
            exponent: 2046,
        },
        relative_error: 0.,
    };
    let test = integer_f_test(huge_ss, huge_error, 2, 6, &c).unwrap();
    assert!(huge_ss.captured().value.value.is_none());
    close(test.f.value.unwrap(), 12.);
    close(test.p.value.unwrap(), 0.008);
}
#[test]
fn integer_tail_full_domain_exact_input_reference_controls() {
    let fixture: serde_json::Value = serde_json::from_str(include_str!(
        "../../../../tests/fixtures/anova-independent-integer-tails.json"
    ))
    .unwrap();
    let mut available = 0;
    let mut refused = 0;
    for record in fixture["records"].as_array().unwrap() {
        let log = f64::from_bits(record["log_bits"].as_u64().unwrap());
        let df1 = record["df1"].as_u64().unwrap();
        let df2 = record["df2"].as_u64().unwrap();
        let candidate = f_upper_tail(log, df1, df2, &JobControl::default()).unwrap();
        if let Some(actual) = candidate.value {
            let expected = record["p"].as_f64().unwrap();
            if expected >= f64::MIN_POSITIVE {
                assert!(
                    (actual / expected - 1.).abs() < 1e-9,
                    "df {df1} {df2}, log {log}: {actual:e} {expected:e}"
                );
            }
            available += 1;
        } else {
            refused += 1;
        }
        let f = Scaled::new(log.exp());
        let ss = Energy {
            value: f.times(df1 as f64),
            relative_error: 0.,
        };
        let error = Energy {
            value: Scaled::new(df2 as f64),
            relative_error: 0.,
        };
        let test = integer_f_test(ss, error, df1, df2, &JobControl::default()).unwrap();
        if !record["normal"].as_bool().unwrap() {
            assert!(test.p.value.is_none());
        }
        if let Some(p) = test.p.value {
            assert!(p >= f64::MIN_POSITIVE);
        }
    }
    eprintln!("integer tail domain: available {available}, refused {refused}");
    assert!(available > 40);
}
#[test]
fn unequal_minimum_subnormal_units_keep_zero_anchor_and_full_inference() {
    let rows = vec![
        (vec![0.], 1., 2),
        (vec![0.], 3., 3),
        (vec![1.], 4., 1),
        (vec![1.], 8., 4),
        (vec![2.], 2., 2),
        (vec![2.], 9., 1),
    ];
    let base = run(&rows, true);
    let tiny = rows
        .iter()
        .map(|(g, y, w)| (g.clone(), y * f64::from_bits(1), *w))
        .collect::<Vec<_>>();
    let result = run(&tiny, true);
    let e = &fitted(&result).effects[0];
    assert_eq!(e.ss.state, AnovaQuantityState::Positive);
    close(
        e.test.f.value.unwrap(),
        fitted(&base).effects[0].test.f.value.unwrap(),
    );
    close(
        e.test.p.value.unwrap(),
        fitted(&base).effects[0].test.p.value.unwrap(),
    );
}
#[test]
fn captured_outputs_accept_actual_engine_values() {
    for rows in [
        vec![
            (vec![0.], 1., 1),
            (vec![0.], 2., 1),
            (vec![1.], 3., 1),
            (vec![1.], 4., 1),
        ],
        vec![(vec![0.], 1., 2), (vec![1.], 2., 2), (vec![2.], 5., 2)],
        vec![
            (vec![0., 0.], 1., 1),
            (vec![0., 1.], 2., 1),
            (vec![1., 0.], 3., 1),
        ],
        vec![
            (vec![0.], 2f64.powi(550), 1),
            (vec![1.], -1., 1),
            (vec![1.], 1., 1),
        ],
    ] {
        let d = dataset(&rows);
        let r = request(rows[0].0.len(), true);
        let c = JobControl::default();
        let s = cohort::select(&d, &r, &c, 0).unwrap();
        let result = compute(&d, &r, &s, &c, 0).unwrap();
        let ids = validation::request(&r, |id| Ok(d.column(id)?.kind()), &c).unwrap();
        let grid = AnovaGrid {
            factors: s.factors.clone(),
            cells: s
                .cells
                .iter()
                .zip(result.cell_means)
                .map(|(count, mean)| AnovaCell {
                    physical_rows: count.physical_rows,
                    frequency: count.frequency,
                    mean,
                })
                .collect(),
            complete: s.failure.is_none(),
        };
        let capture = ComputedAnova {
            revision: d.revision(),
            request: r,
            columns: crate::analysis::support::capture_columns(&d, &ids, &c).unwrap(),
            source_rows: d.row_count(),
            diagnostics: s.diagnostics.clone(),
            fitted_rows: s.rows.len(),
            frequency: s.frequency,
            incomplete_case_frequency: s.incomplete_case_frequency,
            observed_measurements: s.observed_measurements,
            grid,
            method: AnovaMethod {
                numerical_version: 1,
                basis_version: 1,
            },
            outcome: result.outcome,
            families: vec![],
        };
        validation::completed(&capture, &c).unwrap();
    }
}
#[test]
fn unpublishable_f_retains_normal_carried_probability() {
    let result = run(
        &[
            (vec![0.], 2f64.powi(550), 1),
            (vec![1.], -1., 1),
            (vec![1.], 1., 1),
        ],
        false,
    );
    let e = &fitted(&result).effects[0];
    assert!(e.ss.value.value.is_none());
    assert!(e.test.f.value.is_none());
    assert!(e.test.p.value.is_some_and(|p| p.is_normal() && p > 0.));
}
#[test]
fn empty_and_collapsed_cohorts_are_captured_outcomes() {
    let d = Dataset::new(vec![
        Column::numeric(ColumnId(1), "Outcome", vec![]).unwrap(),
        Column::numeric(ColumnId(2), "Factor", vec![]).unwrap(),
        Column::numeric(ColumnId(20), "Weight", vec![]).unwrap(),
    ])
    .unwrap();
    let r = request(1, true);
    let c = JobControl::default();
    let s = cohort::select(&d, &r, &c, 0).unwrap();
    let result = compute(&d, &r, &s, &c, 0).unwrap();
    assert!(
        matches!(result.outcome,AnovaOutcome::Unavailable(f) if f.kind==AnovaFailureKind::EmptyCohort)
    );
    assert!(result.cell_means.is_empty());
    let result = run(&[(vec![0.], 1., 1), (vec![0.], 2., 1)], false);
    assert!(
        matches!(result.outcome,AnovaOutcome::Unavailable(f) if f.kind==AnovaFailureKind::CollapsedFactor)
    );
    assert_eq!(result.cell_means[0].value, Some(1.5));
}
#[test]
fn balanced_subnormal_zero_basis_coordinate_keeps_nonzero_effect() {
    let tiny = f64::from_bits(1);
    let rows = vec![
        (vec![0.], 0., 1),
        (vec![0.], 2. * tiny, 1),
        (vec![1.], 0., 1),
        (vec![1.], 2. * tiny, 1),
        (vec![2.], 2. * tiny, 1),
        (vec![2.], 4. * tiny, 1),
    ];
    let result = run(&rows, false);
    let e = &fitted(&result).effects[0];
    assert_eq!(e.ss.state, AnovaQuantityState::Positive);
    assert!(e.test.f.value.is_some());
    assert!(e.test.p.value.is_some());
}
#[test]
fn extreme_imbalance_at_frequency_ceiling_retains_direct_energies() {
    let n = 1u64 << 53;
    let result = run(
        &[
            (vec![0.], 1., n - 3),
            (vec![0.], 3., 1),
            (vec![1.], 4., 1),
            (vec![1.], 8., 1),
        ],
        true,
    );
    let m = fitted(&result);
    assert_eq!(m.residual_df, n - 2);
    close(m.error_ss.value.value.unwrap(), 12.);
    close(m.model_ss.value.value.unwrap(), 50.);
    close(m.total_ss.value.value.unwrap(), 62.);
    // The reviewed count-lower input allowance refuses this extreme solve.
    assert_eq!(
        m.effects[0].ss.state,
        AnovaQuantityState::Unavailable(AnovaFailureKind::Numerical)
    );
    assert!(m.effects[0].test.p.value.is_none());
}
#[test]
fn nonzero_unbalanced_score_normalization_underflow_refuses_effect() {
    let tiny = f64::from_bits(1);
    let result = run(
        &[
            (vec![0.], 0., 2),
            (vec![1.], f64::MIN_POSITIVE + tiny, 3),
            (vec![2.], f64::MIN_POSITIVE / 2., 2),
            (vec![3.], 1., 2),
        ],
        true,
    );
    let m = fitted(&result);
    let e = &m.effects[0];
    assert_eq!(
        e.ss.state,
        AnovaQuantityState::Unavailable(AnovaFailureKind::Numerical)
    );
    assert!(e.ss.value.value.is_none());
    assert!(e.eta_squared.value.is_none());
    assert!(e.partial_eta_squared.value.is_none());
    assert_eq!(e.dimension, 3);
    assert_eq!(m.error_ss.state, AnovaQuantityState::ExactZero);
}
