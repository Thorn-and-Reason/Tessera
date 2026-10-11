use super::*;
use crate::{
    analysis::{ConfidenceLevel, RowFilter},
    error::AppError,
    operation::CHECKPOINT_HOOK,
};
fn request() -> AnovaRequest {
    AnovaRequest {
        design: AnovaDesign::OneWay {
            outcome: ColumnId(1),
            factor: ColumnId(2),
        },
        confidence: ConfidenceLevel::P95,
        filter: RowFilter::default(),
        frequency_weight: None,
        planned_pairs: vec![],
        all_pairs: vec![],
    }
}
fn cancels(r: &AnovaRequest, threshold: usize) {
    let c = JobControl::default();
    let cancel = c.clone();
    CHECKPOINT_HOOK.with(|hook| {
        *hook.borrow_mut() = Some(Box::new(move |p| {
            if p.total.is_none() && p.done >= threshold {
                cancel.cancel()
            }
        }))
    });
    let result = request_validation(r, &c);
    CHECKPOINT_HOOK.with(|hook| *hook.borrow_mut() = None);
    assert!(matches!(result, Err(AppError::Cancelled)));
}
fn request_validation(r: &AnovaRequest, c: &JobControl) -> AppResult<BTreeSet<ColumnId>> {
    super::request(
        r,
        |id| {
            Ok(if id == ColumnId(1) {
                ColumnKind::Number
            } else {
                ColumnKind::Text
            })
        },
        c,
    )
}
#[test]
fn semantic_validation_cancels_within_long_labels_and_families() {
    let mut r = request();
    r.planned_pairs.push(AnovaPairRequest {
        factor: 0,
        first: Cell::Text("ŵ".repeat(20000)),
        second: Cell::Text("b".into()),
        scope: AnovaScope::Marginal,
    });
    cancels(&r, 8192);
    let mut r = request();
    for i in 0..2000 {
        r.planned_pairs.push(AnovaPairRequest {
            factor: 0,
            first: Cell::Text(format!("a{i}")),
            second: Cell::Text("b".into()),
            scope: AnovaScope::Marginal,
        });
    }
    cancels(&r, 1000);
}
#[test]
fn declared_unicode_blank_and_exact_names() {
    let mut r = request();
    r.design = AnovaDesign::Repeated {
        subject: ColumnId(2),
        factors: vec![AnovaWithinFactor {
            name: "Time".into(),
            levels: vec!["é".into(), "e\u{301}".into(), " é".into(), "É".into()],
        }],
        measurements: vec![ColumnId(1), ColumnId(3), ColumnId(4), ColumnId(5)],
    };
    let kinds = |id| {
        Ok(if id == ColumnId(2) {
            ColumnKind::Text
        } else {
            ColumnKind::Number
        })
    };
    assert!(super::request(&r, kinds, &JobControl::default()).is_ok());
    if let AnovaDesign::Repeated { factors, .. } = &mut r.design {
        factors[0].levels[0] = "\u{2003}\t".into();
    }
    assert!(super::request(&r, kinds, &JobControl::default()).is_err());
}
#[test]
fn metadata_validation_cancels_inside_long_strings() {
    let info = crate::analysis::RunColumnInfo {
        id: ColumnId(1),
        name: "x".into(),
        kind: ColumnKind::Number,
        description: "ŵ".repeat(20000),
        missing_rules: crate::data::MissingRules::None,
        value_labels: vec![],
    };
    let c = JobControl::default();
    let cancel = c.clone();
    CHECKPOINT_HOOK.with(|h| {
        *h.borrow_mut() = Some(Box::new(move |p| {
            if p.done >= 8192 {
                cancel.cancel()
            }
        }))
    });
    let result = snapshot(&info, &c);
    CHECKPOINT_HOOK.with(|h| *h.borrow_mut() = None);
    assert!(matches!(result, Err(AppError::Cancelled)));
}
#[test]
fn gg_unpublishable_f_still_requires_valid_effect_energy() {
    // Two finite subject profiles: [2^550,-1], [2^550,1]. The sole effect
    // has SS=2^1100, errorSS=1, df=(1,1), F overflow and normal p.
    let available = |v| crate::analysis::Estimate {
        value: Some(v),
        reason: None,
    };
    let unavailable = || crate::analysis::Estimate {
        value: None,
        reason: Some("The positive scalar exceeds Float64 range.".into()),
    };
    let positive = |v| AnovaEnergy {
        state: AnovaQuantityState::Positive,
        value: available(v),
    };
    let carried = || AnovaEnergy {
        state: AnovaQuantityState::Positive,
        value: unavailable(),
    };
    let test = AnovaTest {
        df1: available(1.),
        df2: available(1.),
        f: unavailable(),
        p: available((2. / std::f64::consts::PI) * 2f64.powi(-550).atan()),
    };
    let effect = AnovaEffect {
        factors: vec![0],
        dimension: 1,
        ss: carried(),
        ms: unavailable(),
        error_ss: positive(1.),
        error_ms: available(1.),
        test: test.clone(),
        gg: Some(AnovaCorrection {
            epsilon: available(1.),
            covariance: AnovaQuantityState::Positive,
            test,
            failure: None,
        }),
        eta_squared: unavailable(),
        partial_eta_squared: available(1.),
        failure: None,
    };
    let mut model_result = AnovaModelResult {
        effects: vec![effect],
        error_ss: positive(1.),
        model_ss: carried(),
        total_ss: carried(),
        subject_ss: Some(positive(1.)),
        residual_df: 1,
        total_df: 3,
        balanced: true,
    };
    let r = ComputedAnova {
        revision: crate::data::RevisionId(1),
        request: AnovaRequest {
            design: AnovaDesign::Repeated {
                subject: ColumnId(2),
                factors: vec![AnovaWithinFactor {
                    name: "Time".into(),
                    levels: vec!["a".into(), "b".into()],
                }],
                measurements: vec![ColumnId(1), ColumnId(3)],
            },
            ..request()
        },
        columns: [
            (1, "y", ColumnKind::Number),
            (2, "id", ColumnKind::Text),
            (3, "z", ColumnKind::Number),
        ]
        .map(|(id, name, kind)| crate::analysis::RunColumnInfo {
            id: ColumnId(id),
            name: name.into(),
            kind,
            description: String::new(),
            missing_rules: crate::data::MissingRules::None,
            value_labels: vec![],
        })
        .to_vec(),
        source_rows: 2,
        diagnostics: crate::analysis::RunDiagnostics::default(),
        fitted_rows: 2,
        frequency: 2,
        incomplete_case_frequency: 0,
        observed_measurements: 4,
        grid: AnovaGrid {
            factors: vec![AnovaFactorLevels {
                column: None,
                name: "Time".into(),
                values: vec![Cell::Text("a".into()), Cell::Text("b".into())],
            }],
            cells: [2f64.powi(550), 0.]
                .map(|mean| AnovaCell {
                    physical_rows: 2,
                    frequency: 2,
                    mean: available(mean),
                })
                .to_vec(),
            complete: true,
        },
        method: AnovaMethod {
            numerical_version: 1,
            basis_version: 1,
        },
        outcome: AnovaOutcome::Fitted(model_result.clone()),
        families: vec![],
    };
    assert!(r.validate(&JobControl::default()).is_ok());
    assert!(model(&r, &model_result, true, &JobControl::default()).is_ok());
    let effect = &mut model_result.effects[0];
    effect.error_ss = AnovaEnergy {
        state: AnovaQuantityState::Unavailable(AnovaFailureKind::Numerical),
        value: unavailable(),
    };
    effect.error_ms = unavailable();
    effect.partial_eta_squared = unavailable();
    effect.test.p = unavailable();
    assert!(
        model(&r, &model_result, true, &JobControl::default()).is_err(),
        "corrected p retained after underlying error-energy failure"
    );
}
fn linked_gg_fixture(subjects: u64, levels: &[usize], interaction_epsilon: f64) -> ComputedAnova {
    let available = |v| crate::analysis::Estimate {
        value: Some(v),
        reason: None,
    };
    let unavailable = || crate::analysis::Estimate {
        value: None,
        reason: Some("Probability unavailable after publication uncertainty screening.".into()),
    };
    let energy = |v| AnovaEnergy {
        state: if v == 0. {
            AnovaQuantityState::ExactZero
        } else {
            AnovaQuantityState::Positive
        },
        value: available(v),
    };
    let factors = levels
        .iter()
        .enumerate()
        .map(|(i, n)| AnovaWithinFactor {
            name: format!("factor{i}"),
            levels: (0..*n).map(|j| format!("level{j}")).collect(),
        })
        .collect::<Vec<_>>();
    let grid = levels.iter().product::<usize>();
    let effects = (1..1usize << levels.len())
        .map(|mask| {
            let factors = (0..levels.len())
                .filter(|i| mask & (1 << i) != 0)
                .collect::<Vec<_>>();
            let rank = factors.iter().map(|i| levels[*i] - 1).product::<usize>();
            let df2 = (subjects - 1) * rank as u64;
            let epsilon = if rank == 1 {
                1.
            } else if subjects == 2 {
                1. / rank as f64
            } else if factors.len() > 1 {
                interaction_epsilon
            } else if factors[0] == 1 {
                0.7732998756781242
            } else {
                0.5351570711373831
            };
            let corrected_df1 = if rank == 1 || subjects == 2 {
                1.
            } else {
                epsilon * rank as f64
            };
            let test = AnovaTest {
                df1: available(rank as f64),
                df2: available(df2 as f64),
                f: available(1. / rank as f64),
                p: unavailable(),
            };
            AnovaEffect {
                factors,
                dimension: rank,
                ss: energy(1.),
                ms: available(1. / rank as f64),
                error_ss: energy(df2 as f64),
                error_ms: available(1.),
                test: test.clone(),
                gg: Some(AnovaCorrection {
                    epsilon: available(epsilon),
                    covariance: AnovaQuantityState::Positive,
                    test: AnovaTest {
                        df1: available(corrected_df1),
                        df2: available((subjects - 1) as f64 * corrected_df1),
                        ..test
                    },
                    failure: None,
                }),
                eta_squared: unavailable(),
                partial_eta_squared: available(1. / (1. + df2 as f64)),
                failure: None,
            }
        })
        .collect::<Vec<_>>();
    let error = effects
        .iter()
        .map(|e| e.error_ss.value.value.unwrap())
        .sum::<f64>();
    let model_ss = effects.len() as f64;
    let columns = std::iter::once(crate::analysis::RunColumnInfo {
        id: ColumnId(1),
        name: "subject".into(),
        kind: ColumnKind::Text,
        description: String::new(),
        missing_rules: crate::data::MissingRules::None,
        value_labels: vec![],
    })
    .chain((0..grid).map(|i| crate::analysis::RunColumnInfo {
        id: ColumnId(i as u64 + 2),
        name: format!("measurement{i}"),
        kind: ColumnKind::Number,
        description: String::new(),
        missing_rules: crate::data::MissingRules::None,
        value_labels: vec![],
    }))
    .collect();
    let captured_factors = factors
        .iter()
        .map(|f| AnovaFactorLevels {
            column: None,
            name: f.name.clone(),
            values: f.levels.iter().map(|s| Cell::Text(s.clone())).collect(),
        })
        .collect();
    let u = |i: usize| {
        if i == 0 {
            1. / 2f64.sqrt()
        } else if i == 1 {
            -1. / 2f64.sqrt()
        } else {
            0.
        }
    };
    // One unit-energy mean coordinate in each orthogonal effect subspace.
    let cells = (0..grid)
        .map(|i| {
            let mean = if levels.len() == 1 {
                u(i)
            } else {
                let a = u(i / levels[1]);
                let b = u(i % levels[1]);
                a / 2. + b / 2. + a * b
            } / (subjects as f64).sqrt();
            AnovaCell {
                physical_rows: subjects as usize,
                frequency: subjects,
                mean: available(mean),
            }
        })
        .collect();
    ComputedAnova {
        revision: crate::data::RevisionId(1),
        request: AnovaRequest {
            design: AnovaDesign::Repeated {
                subject: ColumnId(1),
                factors,
                measurements: (0..grid).map(|i| ColumnId(i as u64 + 2)).collect(),
            },
            ..request()
        },
        columns,
        source_rows: subjects as usize,
        diagnostics: crate::analysis::RunDiagnostics::default(),
        fitted_rows: subjects as usize,
        frequency: subjects,
        incomplete_case_frequency: 0,
        observed_measurements: subjects as usize * grid,
        grid: AnovaGrid {
            factors: captured_factors,
            cells,
            complete: true,
        },
        method: AnovaMethod {
            numerical_version: 1,
            basis_version: 1,
        },
        outcome: AnovaOutcome::Fitted(AnovaModelResult {
            effects,
            error_ss: energy(error),
            model_ss: energy(model_ss),
            total_ss: energy(error + model_ss),
            subject_ss: Some(energy(0.)),
            residual_df: subjects - 1,
            total_df: subjects * grid as u64 - 1,
            balanced: true,
        }),
        families: vec![],
    }
}
#[test]
fn gg_degrees_use_linked_multiplication_and_structural_identities() {
    let epsilon = 0.36279609553293163;
    let r = linked_gg_fixture(8, &[4, 4], epsilon);
    let interaction_df2 = 7. * (epsilon * 9.);
    assert_eq!(interaction_df2, 22.856154018574692);
    let main_epsilon = 0.7732998756781242;
    let linked = 7. * (main_epsilon * 3.);
    let alternate = main_epsilon * (7. * 3.);
    assert_eq!(linked, 16.239297389240605);
    assert_ne!(linked, alternate);
    assert!(r.validate(&JobControl::default()).is_ok());
    for df2 in [alternate, 0., linked + 1.] {
        let mut bad = r.clone();
        let AnovaOutcome::Fitted(m) = &mut bad.outcome else {
            panic!()
        };
        m.effects[1].gg.as_mut().unwrap().test.df2.value = Some(df2);
        assert!(
            bad.validate(&JobControl::default()).is_err(),
            "incorrect linked df2{df2}"
        );
    }
    let mut bad = r.clone();
    let AnovaOutcome::Fitted(m) = &mut bad.outcome else {
        panic!()
    };
    m.effects[2].gg.as_mut().unwrap().epsilon.value = Some(0.1);
    assert!(bad.validate(&JobControl::default()).is_err());
    let mut bad = r.clone();
    let AnovaOutcome::Fitted(m) = &mut bad.outcome else {
        panic!()
    };
    m.effects[2].gg.as_mut().unwrap().covariance =
        AnovaQuantityState::Unavailable(AnovaFailureKind::InvalidCovariance);
    assert!(bad.validate(&JobControl::default()).is_err());
    let mut bad = r.clone();
    let AnovaOutcome::Fitted(m) = &mut bad.outcome else {
        panic!()
    };
    let gg = m.effects[2].gg.as_mut().unwrap();
    gg.test.df1.value = gg.test.df1.value.map(|v| f64::from_bits(v.to_bits() + 1));
    assert!(bad.validate(&JobControl::default()).is_err());
    for mut structural in [
        linked_gg_fixture(2, &[4, 4], 1. / 9.),
        linked_gg_fixture(8, &[2], 1.),
    ] {
        assert!(structural.validate(&JobControl::default()).is_ok());
        let AnovaOutcome::Fitted(m) = &mut structural.outcome else {
            panic!()
        };
        m.effects[0].gg.as_mut().unwrap().test.df2.value = Some(2.);
        assert!(structural.validate(&JobControl::default()).is_err());
    }
}
