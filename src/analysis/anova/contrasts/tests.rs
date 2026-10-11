use super::*;
use crate::{
    analysis::{ConfidenceLevel, RowFilter},
    data::{Column, ColumnId},
};
fn run(
    repeated: bool,
    offset: f64,
    scale: f64,
    zero: bool,
    absent: bool,
) -> Vec<AnovaContrastFamily> {
    let profiles = if zero {
        vec![[1., 3.], [2., 4.], [4., 6.], [7., 9.]]
    } else {
        vec![[1., 3.], [2., 5.], [4., 5.], [5., 9.]]
    };
    let c = JobControl::default();
    let (d, design) = if repeated {
        (
            Dataset::new(vec![
                Column::numeric(ColumnId(1), "id", (0..4).map(|i| Some(i as f64)).collect())
                    .unwrap(),
                Column::numeric(
                    ColumnId(2),
                    "a",
                    profiles
                        .iter()
                        .map(|r| Some((r[0] + offset) * scale))
                        .collect(),
                )
                .unwrap(),
                Column::numeric(
                    ColumnId(3),
                    "b",
                    profiles
                        .iter()
                        .map(|r| Some((r[1] + offset) * scale))
                        .collect(),
                )
                .unwrap(),
            ])
            .unwrap(),
            AnovaDesign::Repeated {
                subject: ColumnId(1),
                factors: vec![AnovaWithinFactor {
                    name: "time".into(),
                    levels: vec!["a".into(), "b".into()],
                }],
                measurements: vec![ColumnId(2), ColumnId(3)],
            },
        )
    } else {
        (
            Dataset::new(vec![
                Column::numeric(
                    ColumnId(1),
                    "y",
                    profiles
                        .iter()
                        .flat_map(|r| r.iter().map(|v| Some((v + offset) * scale)))
                        .collect(),
                )
                .unwrap(),
                Column::text(
                    ColumnId(2),
                    "group",
                    (0..4)
                        .flat_map(|_| [Some("a".into()), Some("b".into())])
                        .collect(),
                )
                .unwrap(),
            ])
            .unwrap(),
            AnovaDesign::OneWay {
                outcome: ColumnId(1),
                factor: ColumnId(2),
            },
        )
    };
    let r = AnovaRequest {
        design,
        confidence: ConfidenceLevel::P95,
        filter: RowFilter::default(),
        frequency_weight: None,
        planned_pairs: vec![AnovaPairRequest {
            factor: 0,
            first: Cell::Text(if absent { "missing" } else { "a" }.into()),
            second: Cell::Text("b".into()),
            scope: AnovaScope::Marginal,
        }],
        all_pairs: vec![AnovaFamilyRequest {
            factor: 0,
            scope: AnovaScope::Marginal,
        }],
    };
    let s = cohort::select(&d, &r, &c, 0).unwrap();
    let (outcome, means, families) = if repeated {
        let result = super::super::repeated::compute(&d, &r, &s, &c, 0).unwrap();
        let families = compute(&d, &r, &s, &result.outcome, Input::Repeated, &c, 0).unwrap();
        (result.outcome, result.cell_means, families)
    } else {
        let result = super::super::independent::compute(&d, &r, &s, &c, 0).unwrap();
        let families = compute(
            &d,
            &r,
            &s,
            &result.outcome,
            Input::Independent(result.numerical.as_ref()),
            &c,
            0,
        )
        .unwrap();
        (result.outcome, result.cell_means, families)
    };
    let ids = validation::request(&r, |id| Ok(d.column(id)?.kind()), &c).unwrap();
    let captured = ComputedAnova {
        revision: d.revision(),
        request: r,
        columns: crate::analysis::support::capture_columns(&d, &ids, &c).unwrap(),
        source_rows: d.row_count(),
        diagnostics: s.diagnostics.clone(),
        fitted_rows: s.rows.len(),
        frequency: s.frequency,
        incomplete_case_frequency: s.incomplete_case_frequency,
        observed_measurements: s.observed_measurements,
        grid: AnovaGrid {
            factors: s.factors.clone(),
            cells: s
                .cells
                .iter()
                .zip(means)
                .map(|(count, mean)| AnovaCell {
                    physical_rows: count.physical_rows,
                    frequency: count.frequency,
                    mean,
                })
                .collect(),
            complete: s.failure.is_none(),
        },
        method: AnovaMethod {
            numerical_version: 1,
            basis_version: 1,
        },
        outcome,
        families: families.clone(),
    };
    assert!(
        validation::completed(&captured, &c).is_ok(),
        "repeated={repeated} offset={offset} scale={scale} captured={captured:?}"
    );
    families
}
#[test]
fn actual_independent_and_paired_inference_offsets_extreme_units() {
    for repeated in [false, true] {
        for offset in [0., 2f64.powi(45), 2f64.powi(55)] {
            for scale in [1., 2f64.powi(550), f64::from_bits(1)] {
                let families = run(repeated, offset, scale, false, false);
                assert_eq!(families.len(), 2);
                for f in &families {
                    assert_eq!(f.m, 1);
                    assert_eq!(f.members.len(), 1);
                    let m = &f.members[0];
                    assert!(m.df.value.is_some());
                    if offset == 0. && scale == 1. {
                        assert_eq!(m.estimate.value, Some(-2.5));
                        assert!(m.standard_error.value.unwrap() > 0.);
                        assert!(m.raw_p.value.is_some());
                        assert_eq!(m.raw_p, m.adjusted_p);
                        assert!(
                            m.interval[0].value.unwrap() < -2.5
                                && m.interval[1].value.unwrap() > -2.5
                        );
                    }
                }
            }
        }
    }
}
#[test]
fn absent_planned_retains_fixed_membership_and_omnibus() {
    let f = run(false, 0., 1., false, true);
    assert_eq!(f[0].m, 1);
    let m = &f[0].members[0];
    assert!(matches!(
        m.state,
        AnovaComparisonState::AbsentRequestedLevel(_)
    ));
    assert_eq!(m.ordinal, 0);
    assert_eq!(m.request.first, Cell::Text("missing".into()));
    assert!(m.estimate.value.is_none());
    assert!(f[0].adjustment_failure.is_some());
    assert!(f[1].members[0].raw_p.value.is_some());
}
#[test]
fn paired_zero_variance_keeps_point_interval() {
    let f = run(true, 0., 1., true, false);
    let m = &f[0].members[0];
    assert_eq!(m.standard_error.value, Some(0.));
    assert_eq!(m.interval[0].value, m.estimate.value);
    assert_eq!(m.interval[1].value, m.estimate.value);
    assert!(m.t.value.is_none());
    assert!(m.raw_p.value.is_none());
}
#[test]
fn holm_ties_and_unavailable_raw() {
    assert_eq!(
        holm(&[Some(0.01), Some(0.01), Some(0.04)]),
        Some(vec![0.03, 0.03, 0.04])
    );
    assert_eq!(holm(&[Some(0.01), None]), None);
}
#[test]
fn critical_values_have_checked_conditional_brackets() {
    for df in [1., 2., 3., 10., 1_000_000., (1u64 << 53) as f64 - 1.] {
        for confidence in [
            ConfidenceLevel::P90,
            ConfidenceLevel::P95,
            ConfidenceLevel::P99,
        ] {
            let c = JobControl::default();
            let q = crate::analysis::inference::student_t_critical(df, confidence, &c).unwrap();
            if let Some(q) = q.value {
                assert!(critical_allowance(q, df, confidence, &c).unwrap().is_some());
            }
        }
    }
}
#[test]
fn exact_input_full_scalar_reference_controls() {
    let fixtures: serde_json::Value = serde_json::from_str(include_str!(
        "../../../../tests/fixtures/anova-contrasts.json"
    ))
    .unwrap();
    let mut available = 0;
    let mut counts = [0usize; 6];
    let mut max_relative = 0f64;
    for f in fixtures.as_array().unwrap() {
        let repeated = f["repeated"].as_bool().unwrap();
        let rows = f["bits"]
            .as_array()
            .unwrap()
            .iter()
            .map(|r| {
                r.as_array()
                    .unwrap()
                    .iter()
                    .map(|v| f64::from_bits(v.as_u64().unwrap()))
                    .collect()
            })
            .collect::<Vec<Vec<f64>>>();
        let n = rows.len();
        let k = 6;
        let (d, design) = if repeated {
            let mut cols =
                vec![
                    Column::numeric(ColumnId(1), "id", (0..n).map(|i| Some(i as f64)).collect())
                        .unwrap(),
                ];
            for j in 0..k {
                cols.push(
                    Column::numeric(
                        ColumnId(j as u64 + 2),
                        &format!("y{j}"),
                        rows.iter().map(|r| Some(r[j])).collect(),
                    )
                    .unwrap(),
                );
            }
            (
                Dataset::new(cols).unwrap(),
                AnovaDesign::Repeated {
                    subject: ColumnId(1),
                    factors: [2, 3]
                        .iter()
                        .enumerate()
                        .map(|(i, l)| AnovaWithinFactor {
                            name: format!("f{i}"),
                            levels: (0..*l).map(|j| j.to_string()).collect(),
                        })
                        .collect(),
                    measurements: (0..k).map(|j| ColumnId(j as u64 + 2)).collect(),
                },
            )
        } else {
            let weights = f["weights"].as_array().unwrap();
            (
                Dataset::new(vec![
                    Column::numeric(
                        ColumnId(1),
                        "y",
                        rows.iter()
                            .flat_map(|r| r.iter().map(|v| Some(*v)))
                            .collect(),
                    )
                    .unwrap(),
                    Column::numeric(
                        ColumnId(2),
                        "A",
                        (0..n)
                            .flat_map(|_| (0..k).map(|j| Some((j / 3) as f64)))
                            .collect(),
                    )
                    .unwrap(),
                    Column::numeric(
                        ColumnId(3),
                        "B",
                        (0..n)
                            .flat_map(|_| (0..k).map(|j| Some((j % 3) as f64)))
                            .collect(),
                    )
                    .unwrap(),
                    Column::numeric(
                        ColumnId(20),
                        "w",
                        (0..n)
                            .flat_map(|_| weights.iter().map(|w| Some(w.as_f64().unwrap())))
                            .collect(),
                    )
                    .unwrap(),
                ])
                .unwrap(),
                AnovaDesign::Factorial {
                    outcome: ColumnId(1),
                    factors: vec![ColumnId(2), ColumnId(3)],
                },
            )
        };
        let label = |j: usize| {
            if repeated {
                Cell::Text(j.to_string())
            } else {
                Cell::Number(j as f64)
            }
        };
        let pair = AnovaPairRequest {
            factor: 0,
            first: label(0),
            second: label(1),
            scope: if f["scope"] == "fixed" {
                AnovaScope::Fixed(vec![label(0)])
            } else {
                AnovaScope::Marginal
            },
        };
        let r = AnovaRequest {
            design,
            confidence: match f["confidence"].as_u64().unwrap() {
                90 => ConfidenceLevel::P90,
                99 => ConfidenceLevel::P99,
                _ => ConfidenceLevel::P95,
            },
            filter: RowFilter::default(),
            frequency_weight: if repeated { None } else { Some(ColumnId(20)) },
            planned_pairs: vec![pair],
            all_pairs: vec![],
        };
        let c = JobControl::default();
        let s = cohort::select(&d, &r, &c, 0).unwrap();
        let families = if repeated {
            let m = super::super::repeated::compute(&d, &r, &s, &c, 0).unwrap();
            compute(&d, &r, &s, &m.outcome, Input::Repeated, &c, 0).unwrap()
        } else {
            let m = super::super::independent::compute(&d, &r, &s, &c, 0).unwrap();
            compute(
                &d,
                &r,
                &s,
                &m.outcome,
                Input::Independent(m.numerical.as_ref()),
                &c,
                0,
            )
            .unwrap()
        };
        let m = &families[0].members[0];
        for (a, key) in [
            (&m.estimate, "estimate"),
            (&m.standard_error, "se"),
            (&m.t, "t"),
            (&m.raw_p, "p"),
        ] {
            if let Some(a) = a.value {
                let b = f["expected"][key].as_f64().unwrap();
                assert!(
                    (a - b).abs() < 2e-9 * b.abs().max(f64::MIN_POSITIVE),
                    "{} {key}: {a} != {b}",
                    f["name"]
                );
                let index = ["estimate", "se", "t", "p"]
                    .iter()
                    .position(|name| *name == key)
                    .unwrap();
                counts[index] += 1;
                max_relative = max_relative.max((a - b).abs() / b.abs().max(f64::MIN_POSITIVE));
                available += 1;
            }
        }
        for i in 0..2 {
            if let Some(a) = m.interval[i].value {
                let b = f["expected"]["interval"][i].as_f64().unwrap();
                assert!(
                    (a - b).abs() < 2e-9 * b.abs().max(f64::MIN_POSITIVE),
                    "{} interval {i}: {a} != {b}",
                    f["name"]
                );
                counts[4 + i] += 1;
                max_relative = max_relative.max((a - b).abs() / b.abs().max(f64::MIN_POSITIVE));
                available += 1;
            }
        }
    }
    assert_eq!(counts, [48, 48, 60, 60, 48, 48]);
    println!(
        "comparison availability estimate/SE/t/p/lo/hi={counts:?} max_relative={max_relative:e}"
    );
    assert!(available > 180);
    println!("comparison reference scalars available={available}");
}
fn three_levels() -> (
    Dataset,
    AnovaRequest,
    AnovaCohort,
    super::super::independent::IndependentResult,
) {
    let d = Dataset::new(vec![
        Column::numeric(
            ColumnId(1),
            "y",
            vec![Some(1.), Some(2.), Some(3.), Some(4.), Some(5.), Some(6.)],
        )
        .unwrap(),
        Column::text(
            ColumnId(2),
            "g",
            vec![
                Some("a".into()),
                Some("a".into()),
                Some("b".into()),
                Some("b".into()),
                Some("c".into()),
                Some("c".into()),
            ],
        )
        .unwrap(),
    ])
    .unwrap();
    let r = AnovaRequest {
        design: AnovaDesign::OneWay {
            outcome: ColumnId(1),
            factor: ColumnId(2),
        },
        confidence: ConfidenceLevel::P95,
        filter: RowFilter::default(),
        frequency_weight: None,
        planned_pairs: vec![AnovaPairRequest {
            factor: 0,
            first: Cell::Text("a".into()),
            second: Cell::Text("c".into()),
            scope: AnovaScope::Marginal,
        }],
        all_pairs: vec![AnovaFamilyRequest {
            factor: 0,
            scope: AnovaScope::Marginal,
        }],
    };
    let c = JobControl::default();
    let s = cohort::select(&d, &r, &c, 0).unwrap();
    let m = super::super::independent::compute(&d, &r, &s, &c, 0).unwrap();
    (d, r, s, m)
}
#[test]
fn direct_boundary_rejects_reversed_duplicates_and_incompatible_state() {
    let (d, mut r, s, m) = three_levels();
    let pair = r.planned_pairs[0].clone();
    r.planned_pairs.push(AnovaPairRequest {
        first: pair.second,
        second: pair.first,
        ..pair
    });
    assert!(compute(
        &d,
        &r,
        &s,
        &m.outcome,
        Input::Independent(m.numerical.as_ref()),
        &JobControl::default(),
        0
    )
    .is_err());
    r.planned_pairs.pop();
    assert!(compute(
        &d,
        &r,
        &s,
        &m.outcome,
        Input::Independent(None),
        &JobControl::default(),
        0
    )
    .is_err());
}
#[test]
fn reversal_preserves_se_and_tail_and_all_pairs_cannot_shrink() {
    let (d, mut r, s, m) = three_levels();
    let c = JobControl::default();
    let a = compute(
        &d,
        &r,
        &s,
        &m.outcome,
        Input::Independent(m.numerical.as_ref()),
        &c,
        0,
    )
    .unwrap();
    let pair = r.planned_pairs[0].clone();
    r.planned_pairs[0] = AnovaPairRequest {
        first: pair.second,
        second: pair.first,
        ..pair
    };
    let b = compute(
        &d,
        &r,
        &s,
        &m.outcome,
        Input::Independent(m.numerical.as_ref()),
        &c,
        0,
    )
    .unwrap();
    assert_eq!(
        a[0].members[0].estimate.value,
        b[0].members[0].estimate.value.map(|v| -v)
    );
    assert_eq!(
        a[0].members[0].standard_error,
        b[0].members[0].standard_error
    );
    assert_eq!(a[0].members[0].raw_p, b[0].members[0].raw_p);
    assert_eq!(a[1].m, 3);
    assert_eq!(
        a[1].members.iter().map(|m| m.ordinal).collect::<Vec<_>>(),
        vec![0, 1, 2]
    );
}
#[test]
fn actual_absent_slice_keeps_all_pairs_and_valid_omnibus() {
    let d = Dataset::new(vec![
        Column::numeric(ColumnId(1), "y", (0..12).map(|i| Some(i as f64)).collect()).unwrap(),
        Column::text(
            ColumnId(2),
            "A",
            (0..12)
                .map(|i| Some(["a", "b", "c"][(i / 2) / 2].into()))
                .collect(),
        )
        .unwrap(),
        Column::text(
            ColumnId(3),
            "B",
            (0..12)
                .map(|i| Some(["x", "y"][(i / 2) % 2].into()))
                .collect(),
        )
        .unwrap(),
    ])
    .unwrap();
    let scope = AnovaScope::Fixed(vec![Cell::Text("absent".into())]);
    let r = AnovaRequest {
        design: AnovaDesign::Factorial {
            outcome: ColumnId(1),
            factors: vec![ColumnId(2), ColumnId(3)],
        },
        confidence: ConfidenceLevel::P95,
        filter: RowFilter::default(),
        frequency_weight: None,
        planned_pairs: vec![AnovaPairRequest {
            factor: 0,
            first: Cell::Text("a".into()),
            second: Cell::Text("b".into()),
            scope: scope.clone(),
        }],
        all_pairs: vec![AnovaFamilyRequest {
            factor: 0,
            scope: scope.clone(),
        }],
    };
    let c = JobControl::default();
    let s = cohort::select(&d, &r, &c, 0).unwrap();
    let m = super::super::independent::compute(&d, &r, &s, &c, 0).unwrap();
    assert!(matches!(m.outcome, AnovaOutcome::Fitted(_)));
    let families = compute(
        &d,
        &r,
        &s,
        &m.outcome,
        Input::Independent(m.numerical.as_ref()),
        &c,
        0,
    )
    .unwrap();
    assert_eq!((families[0].m, families[1].m), (1, 3));
    for f in &families {
        assert!(f.adjustment_failure.is_some());
        for (i, m) in f.members.iter().enumerate() {
            assert_eq!(m.ordinal, i);
            assert_eq!(m.request.scope, scope);
            assert!(matches!(
                m.state,
                AnovaComparisonState::AbsentRequestedLevel(_)
            ));
            assert!(m.raw_p.value.is_none());
        }
    }
    let ids = validation::request(&r, |id| Ok(d.column(id)?.kind()), &c).unwrap();
    let captured = ComputedAnova {
        revision: d.revision(),
        request: r,
        columns: crate::analysis::support::capture_columns(&d, &ids, &c).unwrap(),
        source_rows: d.row_count(),
        diagnostics: s.diagnostics.clone(),
        fitted_rows: s.rows.len(),
        frequency: s.frequency,
        incomplete_case_frequency: s.incomplete_case_frequency,
        observed_measurements: s.observed_measurements,
        grid: AnovaGrid {
            factors: s.factors.clone(),
            cells: s
                .cells
                .iter()
                .zip(m.cell_means)
                .map(|(count, mean)| AnovaCell {
                    physical_rows: count.physical_rows,
                    frequency: count.frequency,
                    mean,
                })
                .collect(),
            complete: true,
        },
        method: AnovaMethod {
            numerical_version: 1,
            basis_version: 1,
        },
        outcome: m.outcome,
        families,
    };
    validation::completed(&captured, &c).unwrap();
}
#[test]
fn actual_unavailable_raw_keeps_three_member_family() {
    let (old, r, _, _) = three_levels();
    let d = Dataset::new(vec![
        Column::numeric(
            ColumnId(1),
            "y",
            vec![Some(1.), Some(1.), Some(2.), Some(2.), Some(3.), Some(3.)],
        )
        .unwrap(),
        old.column(ColumnId(2)).unwrap().clone(),
    ])
    .unwrap();
    let c = JobControl::default();
    let s = cohort::select(&d, &r, &c, 0).unwrap();
    let m = super::super::independent::compute(&d, &r, &s, &c, 0).unwrap();
    let f = compute(
        &d,
        &r,
        &s,
        &m.outcome,
        Input::Independent(m.numerical.as_ref()),
        &c,
        0,
    )
    .unwrap();
    assert_eq!(f[1].m, 3);
    assert!(f[1].adjustment_failure.is_some());
    for m in &f[1].members {
        assert!(m.estimate.value.is_some());
        assert_eq!(m.standard_error.value, Some(0.));
        assert!(m.raw_p.value.is_none());
        assert!(m.adjusted_p.value.is_none());
    }
}
#[test]
fn comparison_cancellation_and_budget_refuse_before_family_copy() {
    use crate::{error::AppError, operation::CHECKPOINT_HOOK};
    let (d, r, s, m) = three_levels();
    let c = JobControl::default();
    c.cancel();
    assert!(matches!(
        compute(
            &d,
            &r,
            &s,
            &m.outcome,
            Input::Independent(m.numerical.as_ref()),
            &c,
            0
        ),
        Err(AppError::Cancelled)
    ));
    assert!(matches!(
        compute(
            &d,
            &r,
            &s,
            &m.outcome,
            Input::Independent(m.numerical.as_ref()),
            &JobControl::default(),
            256 * 1024 * 1024
        ),
        Err(AppError::SizeLimit { .. })
    ));
    let c = JobControl::default();
    let cancel = c.clone();
    let count = std::cell::Cell::new(0);
    CHECKPOINT_HOOK.with(|h| {
        *h.borrow_mut() = Some(Box::new(move |_| {
            count.set(count.get() + 1);
            if count.get() == 50 {
                cancel.cancel();
            }
        }))
    });
    let result = compute(
        &d,
        &r,
        &s,
        &m.outcome,
        Input::Independent(m.numerical.as_ref()),
        &c,
        0,
    );
    CHECKPOINT_HOOK.with(|h| *h.borrow_mut() = None);
    assert!(matches!(result, Err(AppError::Cancelled)));
}
