use super::*;
use crate::{
    analysis::{ConfidenceLevel, RowFilter},
    data::{Column, ColumnId},
};
fn run(values: &[Vec<f64>], levels: &[usize]) -> RepeatedResult {
    let n = values.len();
    let k = values[0].len();
    let mut cols =
        vec![Column::numeric(ColumnId(1), "id", (0..n).map(|i| Some(i as f64)).collect()).unwrap()];
    for j in 0..k {
        cols.push(
            Column::numeric(
                ColumnId(j as u64 + 2),
                &format!("y{j}"),
                values.iter().map(|r| Some(r[j])).collect(),
            )
            .unwrap(),
        );
    }
    let d = Dataset::new(cols).unwrap();
    let r = AnovaRequest {
        design: AnovaDesign::Repeated {
            subject: ColumnId(1),
            factors: levels
                .iter()
                .enumerate()
                .map(|(i, l)| AnovaWithinFactor {
                    name: format!("f{i}"),
                    levels: (0..*l).map(|v| v.to_string()).collect(),
                })
                .collect(),
            measurements: (0..k).map(|i| ColumnId(i as u64 + 2)).collect(),
        },
        confidence: ConfidenceLevel::P95,
        filter: RowFilter::default(),
        frequency_weight: None,
        planned_pairs: vec![],
        all_pairs: vec![],
    };
    let c = JobControl::default();
    let s = cohort::select(&d, &r, &c, 0).unwrap();
    {
        let result = compute(&d, &r, &s, &c, 0).unwrap();
        let ids = validation::request(&r, |id| Ok(d.column(id)?.kind()), &c).unwrap();
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
            grid: AnovaGrid {
                factors: s.factors.clone(),
                cells: s
                    .cells
                    .iter()
                    .zip(&result.cell_means)
                    .map(|(count, mean)| AnovaCell {
                        physical_rows: count.physical_rows,
                        frequency: count.frequency,
                        mean: mean.clone(),
                    })
                    .collect(),
                complete: s.failure.is_none(),
            },
            method: AnovaMethod {
                numerical_version: 1,
                basis_version: 1,
            },
            outcome: result.outcome.clone(),
            families: vec![],
        };
        assert!(
            validation::completed(&capture, &c).is_ok(),
            "captured={capture:?}"
        );
        result
    }
}
fn model(r: &RepeatedResult) -> &AnovaModelResult {
    let AnovaOutcome::Fitted(m) = &r.outcome else {
        panic!("not fitted")
    };
    m
}
fn near(v: f64, w: f64) {
    assert!((v - w).abs() <= 2e-12 * w.abs().max(1.), "{v} != {w}");
}
#[test]
fn two_levels_paired_t_squared_and_offsets() {
    let base = vec![vec![1., 3.], vec![2., 5.], vec![4., 5.], vec![5., 9.]];
    for offset in [0., 2f64.powi(45), 2f64.powi(55)] {
        let values = base
            .iter()
            .map(|r| r.iter().map(|v| v + offset).collect())
            .collect::<Vec<Vec<f64>>>();
        let differences = values.iter().map(|r| r[1] - r[0]).collect::<Vec<_>>();
        let mean = differences.iter().sum::<f64>() / 4.;
        let error = differences.iter().map(|x| (x - mean).powi(2)).sum::<f64>();
        let r = run(&values, &[2]);
        let m = model(&r);
        let e = &m.effects[0];
        near(e.ss.value.value.unwrap(), 4. * mean * mean / 2.);
        near(e.error_ss.value.value.unwrap(), error / 2.);
        if error > 0. {
            near(e.test.f.value.unwrap(), 4. * mean * mean / (error / 3.));
        } else {
            assert!(e.test.f.value.is_none());
        }
        assert_eq!(e.test.df2.value, Some(3.));
        if error > 0. {
            assert!(e.test.p.value.is_some());
        }
    }
}
#[test]
fn factorial_specific_errors_and_direct_decomposition() {
    let values = vec![
        vec![1., 3., 4., 8.],
        vec![2., 5., 3., 6.],
        vec![0., 4., 7., 9.],
        vec![3., 2., 5., 11.],
    ];
    let r = run(&values, &[2, 2]);
    let m = model(&r);
    assert_eq!(m.effects.len(), 3);
    for (i, h) in [[1., 1., -1., -1.], [1., -1., 1., -1.], [1., -1., -1., 1.]]
        .iter()
        .enumerate()
    {
        let z = values
            .iter()
            .map(|r| r.iter().zip(h).map(|(y, c)| y * c / 2.).sum::<f64>())
            .collect::<Vec<_>>();
        let mean = z.iter().sum::<f64>() / 4.;
        let err = z.iter().map(|x| (x - mean).powi(2)).sum::<f64>();
        near(m.effects[i].ss.value.value.unwrap(), 4. * mean * mean);
        near(m.effects[i].error_ss.value.value.unwrap(), err);
    }
    near(
        m.total_ss.value.value.unwrap(),
        m.model_ss.value.value.unwrap()
            + m.error_ss.value.value.unwrap()
            + m.subject_ss.as_ref().unwrap().value.value.unwrap(),
    );
}
#[test]
fn nonzero_singular_gg_and_zero_error() {
    let r = run(&[vec![1., 2., 6.], vec![2., 4., 9.]], &[3]);
    let e = &model(&r).effects[0];
    let gg = e.gg.as_ref().unwrap();
    assert_eq!(gg.epsilon.value, Some(0.5));
    assert_eq!(gg.test.df1.value, Some(1.));
    assert_eq!(gg.test.df2.value, Some(1.));
    let r = run(
        &[vec![1., 2., 3.], vec![2., 3., 4.], vec![3., 4., 5.]],
        &[3],
    );
    let e = &model(&r).effects[0];
    assert_eq!(e.error_ss.state, AnovaQuantityState::ExactZero);
    assert!(e.test.p.value.is_none());
    assert!(e.gg.as_ref().unwrap().epsilon.value.is_none());
}
#[test]
fn independent_rational_projection_references() {
    let fixtures: serde_json::Value = serde_json::from_str(include_str!(
        "../../../../tests/fixtures/anova-repeated.json"
    ))
    .unwrap();
    let mut available = 0;
    let mut counts = [0usize; 6];
    let mut max_relative = 0f64;
    let mut withheld = 0;
    for f in fixtures.as_array().unwrap() {
        let levels = f["levels"]
            .as_array()
            .unwrap()
            .iter()
            .map(|v| v.as_u64().unwrap() as usize)
            .collect::<Vec<_>>();
        let values = f["bits"]
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
        println!("fixture {}", f["name"]);
        let result = run(&values, &levels);
        let m = model(&result);
        for (e, expected) in m.effects.iter().zip(f["effects"].as_array().unwrap()) {
            for (actual, key) in [
                (&e.ss.value, "ss"),
                (&e.error_ss.value, "error"),
                (&e.test.f, "f"),
                (&e.test.p, "p"),
                (&e.gg.as_ref().unwrap().epsilon, "epsilon"),
                (&e.gg.as_ref().unwrap().test.p, "gg_p"),
            ] {
                if let Some(v) = actual.value {
                    let reference = expected[key].as_f64().unwrap();
                    assert!(
                        (v - reference).abs() <= 2e-9 * reference.abs().max(f64::MIN_POSITIVE),
                        "{} {key}: {v} != {reference}",
                        f["name"]
                    );
                    let index = ["ss", "error", "f", "p", "epsilon", "gg_p"]
                        .iter()
                        .position(|name| *name == key)
                        .unwrap();
                    counts[index] += 1;
                    max_relative = max_relative
                        .max((v - reference).abs() / reference.abs().max(f64::MIN_POSITIVE));
                    available += 1;
                } else {
                    println!("withheld {} {key} reference={}", f["name"], expected[key]);
                    if (key == "ss" || key == "error") && expected[key].is_number() {
                        panic!("ordinary energy unexpectedly withheld")
                    }
                    if key == "epsilon" && values.len() == 2 && expected[key].is_number() {
                        panic!("N=2 structural epsilon was withheld")
                    }
                    withheld += 1;
                }
            }
        }
    }
    assert_eq!(counts, [236, 236, 242, 242, 242, 242]);
    assert_eq!(withheld, 12);
    println!(
        "repeated availability ss/error/F/p/epsilon/GGp={counts:?} max_relative={max_relative:e}"
    );
    assert!(available > 200);
    println!("repeated reference scalars available={available} withheld={withheld}");
}
#[test]
fn singleton_and_constant_profiles_keep_structural_quantities() {
    let r = run(&[vec![1., 2.]], &[2]);
    let e = &model(&r).effects[0];
    assert!(e.ss.value.value.is_some());
    assert!(e.test.p.value.is_none());
    assert_eq!(e.gg.as_ref().unwrap().epsilon.value, Some(1.));
    let r = run(&[vec![3., 3.], vec![3., 3.], vec![3., 3.]], &[2]);
    let e = &model(&r).effects[0];
    assert_eq!(e.ss.state, AnovaQuantityState::ExactZero);
    assert_eq!(e.error_ss.state, AnovaQuantityState::ExactZero);
}
fn setup() -> (Dataset, AnovaRequest, AnovaCohort) {
    let n = 128;
    let k = 8;
    let mut cols =
        vec![Column::numeric(ColumnId(1), "id", (0..n).map(|i| Some(i as f64)).collect()).unwrap()];
    for j in 0..k {
        cols.push(
            Column::numeric(
                ColumnId(j as u64 + 2),
                &format!("y{j}"),
                (0..n)
                    .map(|i| Some((i * j + 3 * j + i % 7) as f64))
                    .collect(),
            )
            .unwrap(),
        );
    }
    let d = Dataset::new(cols).unwrap();
    let r = AnovaRequest {
        design: AnovaDesign::Repeated {
            subject: ColumnId(1),
            factors: vec![AnovaWithinFactor {
                name: "time".into(),
                levels: (0..k).map(|i| i.to_string()).collect(),
            }],
            measurements: (0..k).map(|j| ColumnId(j as u64 + 2)).collect(),
        },
        confidence: ConfidenceLevel::P95,
        filter: RowFilter::default(),
        frequency_weight: None,
        planned_pairs: vec![],
        all_pairs: vec![],
    };
    let s = cohort::select(&d, &r, &JobControl::default(), 0).unwrap();
    (d, r, s)
}
#[test]
fn actual_control_cancellation_and_budget_precede_work() {
    use crate::{error::AppError, operation::CHECKPOINT_HOOK};
    let (d, r, s) = setup();
    let c = JobControl::default();
    c.cancel();
    assert!(matches!(
        compute(&d, &r, &s, &c, 0),
        Err(AppError::Cancelled)
    ));
    assert!(matches!(
        compute(&d, &r, &s, &JobControl::default(), 256 * 1024 * 1024),
        Err(AppError::SizeLimit { .. })
    ));
    let checks = std::rc::Rc::new(std::cell::Cell::new(0));
    let counted = checks.clone();
    CHECKPOINT_HOOK
        .with(|h| *h.borrow_mut() = Some(Box::new(move |_| counted.set(counted.get() + 1))));
    let baseline = compute(&d, &r, &s, &JobControl::default(), 0).unwrap();
    CHECKPOINT_HOOK.with(|h| *h.borrow_mut() = None);
    assert!(matches!(baseline.outcome, AnovaOutcome::Fitted(_)));
    let total = checks.get();
    println!("uncancelled actual checkpoints={total}");
    assert!(total > 1000);
    for cut in [10, total / 3, 2 * total / 3] {
        let c = JobControl::default();
        let cancel = c.clone();
        let checks = std::rc::Rc::new(std::cell::Cell::new(0));
        let counted = checks.clone();
        CHECKPOINT_HOOK.with(|h| {
            *h.borrow_mut() = Some(Box::new(move |_| {
                counted.set(counted.get() + 1);
                if counted.get() == cut {
                    cancel.cancel();
                }
            }))
        });
        let result = compute(&d, &r, &s, &c, 0);
        CHECKPOINT_HOOK.with(|h| *h.borrow_mut() = None);
        assert!(matches!(result, Err(AppError::Cancelled)), "cut {cut}");
        assert_eq!(checks.get(), cut);
        println!("cancelled at reachable checkpoint={cut}");
    }
}
#[test]
fn direct_repeated_boundary_rejects_outside_mapping_labels() {
    let (d, mut r, s) = setup();
    r.planned_pairs.push(AnovaPairRequest {
        factor: 0,
        first: Cell::Text("outside".into()),
        second: Cell::Text("1".into()),
        scope: AnovaScope::Marginal,
    });
    assert!(compute(&d, &r, &s, &JobControl::default(), 0).is_err());
}
#[test]
fn mapping_permutation_and_complete_profile_cohort() {
    let (d, mut r, s) = setup();
    let c = JobControl::default();
    let first = compute(&d, &r, &s, &c, 0).unwrap();
    let AnovaDesign::Repeated { measurements, .. } = &mut r.design else {
        unreachable!()
    };
    measurements.reverse();
    let s = cohort::select(&d, &r, &c, 0).unwrap();
    let second = compute(&d, &r, &s, &c, 0).unwrap();
    for (a, b) in model(&first).effects.iter().zip(&model(&second).effects) {
        near(a.ss.value.value.unwrap(), b.ss.value.value.unwrap());
        near(
            a.error_ss.value.value.unwrap(),
            b.error_ss.value.value.unwrap(),
        );
    }
    let d = Dataset::new(vec![
        Column::numeric(ColumnId(1), "id", vec![Some(0.), Some(1.), Some(2.)]).unwrap(),
        Column::numeric(ColumnId(2), "a", vec![Some(1.), Some(99.), Some(3.)]).unwrap(),
        Column::numeric(ColumnId(3), "b", vec![Some(2.), None, Some(5.)]).unwrap(),
    ])
    .unwrap();
    let r = AnovaRequest {
        design: AnovaDesign::Repeated {
            subject: ColumnId(1),
            factors: vec![AnovaWithinFactor {
                name: "time".into(),
                levels: vec!["a".into(), "b".into()],
            }],
            measurements: vec![ColumnId(2), ColumnId(3)],
        },
        confidence: ConfidenceLevel::P95,
        filter: RowFilter::default(),
        frequency_weight: None,
        planned_pairs: vec![],
        all_pairs: vec![],
    };
    let s = cohort::select(&d, &r, &c, 0).unwrap();
    assert_eq!(s.rows.len(), 2);
    assert_eq!(s.incomplete_case_frequency, 1);
    let actual = compute(&d, &r, &s, &c, 0).unwrap();
    let expected = run(&[vec![1., 2.], vec![3., 5.]], &[2]);
    near(
        model(&actual).effects[0].test.f.value.unwrap(),
        model(&expected).effects[0].test.f.value.unwrap(),
    );
}
#[test]
fn positive_energy_combination_cannot_omit_components() {
    assert!(combine(
        Energy {
            value: Scaled::new(2f64.powi(1000)),
            relative_error: 0.
        },
        Energy {
            value: Scaled::new(f64::from_bits(1)),
            relative_error: 0.
        }
    )
    .is_err());
    assert!(
        combine(
            Energy::default(),
            Energy {
                value: Scaled::new(f64::from_bits(1)),
                relative_error: 0.
            }
        )
        .unwrap()
        .value
        .fraction
            > 0.
    );
}
