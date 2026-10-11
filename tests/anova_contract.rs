use tessera::{
    analysis::*,
    data::{Column, ColumnId, Dataset},
    operation::JobControl,
};
fn data() -> Dataset {
    Dataset::new(vec![
        Column::numeric(ColumnId(1), "y", vec![Some(1.)]).unwrap(),
        Column::text(ColumnId(2), "g", vec![Some("a".into())]).unwrap(),
    ])
    .unwrap()
}
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
#[test]
fn checked_request_budget_before_allocation() {
    let mut r = request();
    r.planned_pairs = Vec::with_capacity(MAX_REQUEST_BYTES);
    assert!(r
        .validate_current(&data(), &[], 0, &JobControl::default())
        .is_err());
}
#[test]
fn duplicate_all_pairs_scope_rejected() {
    let mut r = request();
    r.all_pairs = vec![
        AnovaFamilyRequest {
            factor: 0,
            scope: AnovaScope::Marginal
        };
        2
    ];
    assert!(r
        .validate_current(&data(), &[], 0, &JobControl::default())
        .is_err());
}
#[test]
fn declared_mapping_bijection() {
    let mut r = request();
    r.design = AnovaDesign::Repeated {
        subject: ColumnId(2),
        factors: vec![AnovaWithinFactor {
            name: "Time".into(),
            levels: vec!["a".into(), "b".into()],
        }],
        measurements: vec![ColumnId(1), ColumnId(1)],
    };
    assert!(r
        .validate_current(&data(), &[], 0, &JobControl::default())
        .is_err());
}
fn empty_result() -> ComputedAnova {
    ComputedAnova {
        revision: tessera::data::RevisionId(1),
        request: request(),
        columns: vec![
            RunColumnInfo {
                id: ColumnId(1),
                name: "y".into(),
                kind: tessera::data::ColumnKind::Number,
                description: String::new(),
                missing_rules: tessera::data::MissingRules::None,
                value_labels: vec![],
            },
            RunColumnInfo {
                id: ColumnId(2),
                name: "g".into(),
                kind: tessera::data::ColumnKind::Text,
                description: String::new(),
                missing_rules: tessera::data::MissingRules::None,
                value_labels: vec![],
            },
        ],
        source_rows: 0,
        diagnostics: RunDiagnostics::default(),
        fitted_rows: 0,
        frequency: 0,
        incomplete_case_frequency: 0,
        observed_measurements: 0,
        grid: AnovaGrid {
            factors: vec![AnovaFactorLevels {
                column: Some(ColumnId(2)),
                name: "g".into(),
                values: vec![],
            }],
            cells: vec![],
            complete: false,
        },
        method: AnovaMethod {
            numerical_version: 1,
            basis_version: 1,
        },
        outcome: AnovaOutcome::Unavailable(AnovaFailure {
            kind: AnovaFailureKind::EmptyCohort,
            reason: "No complete observations remain.".into(),
        }),
        families: vec![],
    }
}
#[test]
fn legitimate_empty_captured_result() {
    assert!(empty_result().validate(&JobControl::default()).is_ok());
}
#[test]
fn corrupt_source_partitions_rejected() {
    let mut r = empty_result();
    r.source_rows = 1;
    assert!(r.validate(&JobControl::default()).is_err());
}
#[test]
fn typed_labels_preserved() {
    let mut r = request();
    r.planned_pairs = vec![AnovaPairRequest {
        factor: 0,
        first: tessera::data::Cell::Text("é".into()),
        second: tessera::data::Cell::Text("e\u{301}".into()),
        scope: AnovaScope::Marginal,
    }];
    assert!(r
        .validate_current(&data(), &[], 0, &JobControl::default())
        .is_ok());
    r.planned_pairs.push(AnovaPairRequest {
        factor: 0,
        first: tessera::data::Cell::Text("e\u{301}".into()),
        second: tessera::data::Cell::Text("é".into()),
        scope: AnovaScope::Marginal,
    });
    assert!(r
        .validate_current(&data(), &[], 0, &JobControl::default())
        .is_err());
}
#[test]
fn request_filter_metadata_capacities() {
    let mut r = request();
    let value = String::with_capacity(MAX_REQUEST_BYTES + 1);
    r.filter.predicates.push(RowPredicate::Text {
        column: ColumnId(2),
        value,
        equal: true,
    });
    assert!(r.heap_bytes().unwrap() > MAX_REQUEST_BYTES);
    assert!(r
        .validate_current(&data(), &[], 0, &JobControl::default())
        .is_err());
    let mut result = empty_result();
    result.columns[0].description = String::with_capacity(100_000);
    assert!(result.owned_bytes().unwrap() >= 100_000);
}
#[test]
fn repeated_weights_and_blank_levels_rejected() {
    let d = Dataset::new(vec![
        Column::text(ColumnId(1), "id", vec![Some("s".into())]).unwrap(),
        Column::numeric(ColumnId(2), "a", vec![Some(1.)]).unwrap(),
        Column::numeric(ColumnId(3), "b", vec![Some(2.)]).unwrap(),
    ])
    .unwrap();
    let mut r = request();
    r.design = AnovaDesign::Repeated {
        subject: ColumnId(1),
        factors: vec![AnovaWithinFactor {
            name: "Time".into(),
            levels: vec!["\u{2003}".into(), "b".into()],
        }],
        measurements: vec![ColumnId(2), ColumnId(3)],
    };
    assert!(r
        .validate_current(&d, &[], 0, &JobControl::default())
        .is_err());
    if let AnovaDesign::Repeated { factors, .. } = &mut r.design {
        factors[0].levels[0] = "a".into();
    }
    assert!(r
        .validate_current(&d, &[], 0, &JobControl::default())
        .is_ok());
    r.frequency_weight = Some(ColumnId(2));
    assert!(r
        .validate_current(&d, &[], 0, &JobControl::default())
        .is_err());
}
fn e(value: f64) -> Estimate {
    Estimate {
        value: Some(value),
        reason: None,
    }
}
fn unavailable() -> Estimate {
    Estimate {
        value: None,
        reason: Some("Undefined for this model.".into()),
    }
}
fn positive(value: f64) -> AnovaEnergy {
    AnovaEnergy {
        state: AnovaQuantityState::Positive,
        value: e(value),
    }
}
fn fitted() -> ComputedAnova {
    let mut r = empty_result();
    r.source_rows = 4;
    r.fitted_rows = 4;
    r.frequency = 4;
    r.observed_measurements = 4;
    r.grid.factors[0].values = vec![
        tessera::data::Cell::Text("a".into()),
        tessera::data::Cell::Text("b".into()),
    ];
    r.grid.cells = vec![
        AnovaCell {
            physical_rows: 2,
            frequency: 2,
            mean: e(1.),
        },
        AnovaCell {
            physical_rows: 2,
            frequency: 2,
            mean: e(3.),
        },
    ];
    r.grid.complete = true;
    r.outcome = AnovaOutcome::Fitted(AnovaModelResult {
        effects: vec![AnovaEffect {
            factors: vec![0],
            dimension: 1,
            ss: positive(4.),
            ms: e(4.),
            error_ss: positive(2.),
            error_ms: e(1.),
            test: AnovaTest {
                df1: e(1.),
                df2: e(2.),
                f: e(4.),
                p: e(0.18350341907227397),
            },
            gg: None,
            eta_squared: e(2. / 3.),
            partial_eta_squared: e(2. / 3.),
            failure: None,
        }],
        error_ss: positive(2.),
        model_ss: positive(4.),
        total_ss: positive(6.),
        subject_ss: None,
        residual_df: 2,
        total_df: 3,
        balanced: true,
    });
    r
}
#[test]
fn valid_fitted_contract() {
    assert!(fitted().validate(&JobControl::default()).is_ok());
}
#[test]
fn corrupted_f_ms_eta_and_decomposition_rejected() {
    for field in 0..4 {
        let mut r = fitted();
        let AnovaOutcome::Fitted(m) = &mut r.outcome else {
            panic!()
        };
        match field {
            0 => m.effects[0].test.f = e(9.),
            1 => m.effects[0].ms = e(9.),
            2 => m.effects[0].eta_squared = e(0.1),
            _ => m.total_ss = positive(9.),
        };
        assert!(r.validate(&JobControl::default()).is_err(), "field{field}");
    }
}
#[test]
fn duplicate_captured_all_pairs_scope_rejected() {
    let mut r = fitted();
    r.request.all_pairs = vec![
        AnovaFamilyRequest {
            factor: 0,
            scope: AnovaScope::Marginal
        };
        2
    ];
    assert!(r.validate(&JobControl::default()).is_err());
}
#[test]
fn absent_planned_identities_preserve_membership() {
    let mut r = fitted();
    let p = AnovaPairRequest {
        factor: 0,
        first: tessera::data::Cell::Text("gone".into()),
        second: tessera::data::Cell::Text("b".into()),
        scope: AnovaScope::Marginal,
    };
    r.request.planned_pairs.push(p.clone());
    r.families.push(AnovaContrastFamily {
        adjustment_failure: Some(AnovaFailure {
            kind: AnovaFailureKind::IncompleteFamily,
            reason: "Holm requires every raw probability in the captured family.".into(),
        }),
        kind: AnovaFamilyKind::Planned,
        m: 1,
        members: vec![AnovaComparison {
            ordinal: 0,
            request: p,
            state: AnovaComparisonState::AbsentRequestedLevel(AnovaFailure {
                kind: AnovaFailureKind::AbsentRequestedLevel,
                reason: "Requested level is absent.".into(),
            }),
            estimate: unavailable(),
            standard_error: unavailable(),
            t: unavailable(),
            df: unavailable(),
            raw_p: unavailable(),
            adjusted_p: unavailable(),
            interval: [unavailable(), unavailable()],
            variance: AnovaQuantityState::Unavailable(AnovaFailureKind::AbsentRequestedLevel),
        }],
    });
    assert!(r.validate(&JobControl::default()).is_ok());
    r.families[0].m = 0;
    assert!(r.validate(&JobControl::default()).is_err());
}
fn factorial() -> ComputedAnova {
    let mut r = fitted();
    r.request.design = AnovaDesign::Factorial {
        outcome: ColumnId(1),
        factors: vec![ColumnId(2), ColumnId(3)],
    };
    let mut column = r.columns[1].clone();
    column.id = ColumnId(3);
    column.name = "h".into();
    r.columns.push(column);
    let mut factor = r.grid.factors[0].clone();
    factor.column = Some(ColumnId(3));
    factor.name = "h".into();
    r.grid.factors.push(factor);
    r.grid.cells = (0..4)
        .map(|i| AnovaCell {
            physical_rows: 2,
            frequency: 2,
            mean: e(1. + 2. * i as f64),
        })
        .collect();
    r.source_rows = 8;
    r.fitted_rows = 8;
    r.frequency = 8;
    r.observed_measurements = 8;
    let AnovaOutcome::Fitted(m) = &mut r.outcome else {
        panic!()
    };
    m.error_ss = positive(4.);
    m.model_ss = positive(40.);
    m.total_ss = positive(44.);
    m.residual_df = 4;
    m.total_df = 7;
    let template = m.effects[0].clone();
    m.effects = (0..3)
        .map(|i| {
            let ss = [32., 8., 0.][i];
            let mut effect = template.clone();
            effect.factors = match i {
                0 => vec![0],
                1 => vec![1],
                _ => vec![0, 1],
            };
            effect.ss = if ss == 0. {
                AnovaEnergy {
                    state: AnovaQuantityState::ExactZero,
                    value: e(0.),
                }
            } else {
                positive(ss)
            };
            effect.ms = e(ss);
            effect.error_ss = positive(4.);
            effect.error_ms = e(1.);
            effect.test = AnovaTest {
                df1: e(1.),
                df2: e(4.),
                f: e(ss),
                p: e(if ss == 0. { 1. } else { 0.1 }),
            };
            effect.eta_squared = e(ss / 44.);
            effect.partial_eta_squared = e(ss / (ss + 4.));
            effect
        })
        .collect();
    r
}
#[test]
fn all_pairs_absent_slice_preserves_target_members() {
    let mut r = factorial();
    let scope = AnovaScope::Fixed(vec![tessera::data::Cell::Text("gone".into())]);
    let family = AnovaFamilyRequest {
        factor: 0,
        scope: scope.clone(),
    };
    r.request.all_pairs.push(family.clone());
    let p = AnovaPairRequest {
        factor: 0,
        first: r.grid.factors[0].values[0].clone(),
        second: r.grid.factors[0].values[1].clone(),
        scope,
    };
    r.families = vec![AnovaContrastFamily {
        adjustment_failure: Some(AnovaFailure {
            kind: AnovaFailureKind::IncompleteFamily,
            reason: "Holm requires every raw probability in the captured family.".into(),
        }),
        kind: AnovaFamilyKind::AllPairs(family),
        m: 1,
        members: vec![AnovaComparison {
            ordinal: 0,
            request: p,
            state: AnovaComparisonState::AbsentRequestedLevel(AnovaFailure {
                kind: AnovaFailureKind::AbsentRequestedLevel,
                reason: "Requested fixed slice is absent.".into(),
            }),
            estimate: unavailable(),
            standard_error: unavailable(),
            t: unavailable(),
            df: unavailable(),
            raw_p: unavailable(),
            adjusted_p: unavailable(),
            interval: [unavailable(), unavailable()],
            variance: AnovaQuantityState::Unavailable(AnovaFailureKind::AbsentRequestedLevel),
        }],
    }];
    assert!(r.validate(&JobControl::default()).is_ok());
    r.families[0].members.clear();
    r.families[0].m = 0;
    assert!(r.validate(&JobControl::default()).is_err());
}
#[test]
fn exact_nested_request_capacity_boundary() {
    let mut r = request();
    r.planned_pairs.push(AnovaPairRequest {
        factor: 0,
        first: tessera::data::Cell::Text(String::new()),
        second: tessera::data::Cell::Text("b".into()),
        scope: AnovaScope::Marginal,
    });
    let current = r.heap_bytes().unwrap();
    r.planned_pairs[0].first =
        tessera::data::Cell::Text(String::with_capacity(MAX_REQUEST_BYTES - current));
    assert_eq!(r.heap_bytes().unwrap(), MAX_REQUEST_BYTES);
    assert!(r
        .validate_current(&data(), &[], 0, &JobControl::default())
        .is_ok());
    if let tessera::data::Cell::Text(s) = &mut r.planned_pairs[0].first {
        s.reserve_exact(s.capacity() + 1);
    }
    assert!(r
        .validate_current(&data(), &[], 0, &JobControl::default())
        .is_err());
}
#[test]
fn valid_unbalanced_one_way() {
    let mut r = fitted();
    r.source_rows = 5;
    r.fitted_rows = 5;
    r.frequency = 5;
    r.observed_measurements = 5;
    r.grid.cells[1].physical_rows = 3;
    r.grid.cells[1].frequency = 3;
    let AnovaOutcome::Fitted(m) = &mut r.outcome else {
        panic!()
    };
    m.balanced = false;
    m.residual_df = 3;
    m.total_df = 4;
    m.model_ss = positive(4.8);
    m.total_ss = positive(6.8);
    let e = &mut m.effects[0];
    e.ss = positive(4.8);
    e.ms = crate::e(4.8);
    e.error_ms = crate::e(2. / 3.);
    e.test.df2 = crate::e(3.);
    e.test.f = crate::e(7.2);
    e.eta_squared = crate::e(4.8 / 6.8);
    e.partial_eta_squared = crate::e(4.8 / 6.8);
    assert!(r.validate(&JobControl::default()).is_ok());
}
#[test]
fn holm_membership_and_interval_corruption() {
    let mut r = fitted();
    let p = AnovaPairRequest {
        factor: 0,
        first: r.grid.factors[0].values[0].clone(),
        second: r.grid.factors[0].values[1].clone(),
        scope: AnovaScope::Marginal,
    };
    r.request.planned_pairs.push(p.clone());
    r.families.push(AnovaContrastFamily {
        adjustment_failure: None,
        kind: AnovaFamilyKind::Planned,
        m: 1,
        members: vec![AnovaComparison {
            ordinal: 0,
            request: p,
            state: AnovaComparisonState::Available,
            estimate: e(-2.),
            standard_error: e(1.),
            t: e(-2.),
            df: e(2.),
            raw_p: e(0.18350341907227397),
            adjusted_p: e(0.18350341907227397),
            interval: [e(-5.), e(1.)],
            variance: AnovaQuantityState::Positive,
        }],
    });
    assert!(r.validate(&JobControl::default()).is_ok());
    let mut bad = r.clone();
    bad.families[0].members[0].adjusted_p = e(0.1);
    assert!(bad.validate(&JobControl::default()).is_err());
    let mut bad = r.clone();
    bad.families[0].members[0].interval = [e(-6.), e(1.)];
    assert!(bad.validate(&JobControl::default()).is_err());
    r.families[0].members[0].ordinal = 1;
    assert!(r.validate(&JobControl::default()).is_err());
}
fn publication_only_comparison() -> ComputedAnova {
    let mut r = fitted();
    let max = f64::MAX;
    r.source_rows = 6;
    r.fitted_rows = 6;
    r.frequency = 6;
    r.observed_measurements = 6;
    r.grid.factors[0].values = ["a", "b", "c", "d"]
        .map(|v| tessera::data::Cell::Text(v.into()))
        .to_vec();
    r.grid.cells = [(1, max), (1, -max), (2, 0.), (2, 0.)]
        .map(|(n, mean)| AnovaCell {
            physical_rows: n,
            frequency: n as u64,
            mean: e(mean),
        })
        .to_vec();
    let carried = || AnovaEnergy {
        state: AnovaQuantityState::Positive,
        value: Estimate {
            value: None,
            reason: Some("Positive original-unit energy exceeds Float64 range.".into()),
        },
    };
    let AnovaOutcome::Fitted(m) = &mut r.outcome else {
        panic!()
    };
    m.error_ss = carried();
    m.model_ss = carried();
    m.total_ss = carried();
    m.residual_df = 2;
    m.total_df = 5;
    m.balanced = false;
    let effect = &mut m.effects[0];
    effect.dimension = 3;
    effect.ss = carried();
    effect.error_ss = carried();
    effect.ms = unavailable();
    effect.error_ms = unavailable();
    effect.test = AnovaTest {
        df1: e(3.),
        df2: e(2.),
        f: e(1. / 3.),
        p: e(1. - (1f64 / 3.).powf(1.5)),
    };
    effect.eta_squared = e(1. / 3.);
    effect.partial_eta_squared = e(1. / 3.);
    let p = AnovaPairRequest {
        factor: 0,
        first: r.grid.factors[0].values[0].clone(),
        second: r.grid.factors[0].values[1].clone(),
        scope: AnovaScope::Marginal,
    };
    r.request.planned_pairs.push(p.clone());
    let raw = 1. - 1. / 3f64.sqrt();
    r.families = vec![AnovaContrastFamily {
        adjustment_failure: None,
        kind: AnovaFamilyKind::Planned,
        m: 1,
        members: vec![AnovaComparison {
            ordinal: 0,
            request: p,
            state: AnovaComparisonState::Available,
            estimate: Estimate {
                value: None,
                reason: Some("The positive pair difference exceeds Float64 range.".into()),
            },
            standard_error: Estimate {
                value: None,
                reason: Some("The positive pair standard error exceeds Float64 range.".into()),
            },
            t: e(1.),
            df: e(2.),
            raw_p: e(raw),
            adjusted_p: e(raw),
            interval: [unavailable(), unavailable()],
            variance: AnovaQuantityState::Positive,
        }],
    }];
    r
}
#[test]
fn realised_publication_failure_retains_valid_scaled_test() {
    let r = publication_only_comparison();
    assert!(r.validate(&JobControl::default()).is_ok());
    let mut bad = r.clone();
    bad.families[0].members[0].variance =
        AnovaQuantityState::Unavailable(AnovaFailureKind::Numerical);
    assert!(bad.validate(&JobControl::default()).is_err());
    let mut bad = r;
    bad.families[0].members[0].state = AnovaComparisonState::AbsentRequestedLevel(AnovaFailure {
        kind: AnovaFailureKind::AbsentRequestedLevel,
        reason: "Fabricated absence.".into(),
    });
    assert!(bad.validate(&JobControl::default()).is_err());
}
#[test]
fn unavailable_model_blocks_comparison_inference() {
    let mut r = publication_only_comparison();
    r.families[0].members[0].estimate = e(1.);
    r.outcome = AnovaOutcome::Unavailable(AnovaFailure {
        kind: AnovaFailureKind::Numerical,
        reason: "The underlying model calculation failed.".into(),
    });
    assert!(r.validate(&JobControl::default()).is_err());
}
#[test]
fn unavailable_comparison_blocks_inference_and_requires_incomplete_family() {
    for kind in [
        AnovaFailureKind::Numerical,
        AnovaFailureKind::PublicationRange,
    ] {
        let mut r = publication_only_comparison();
        r.families[0].members[0].state = AnovaComparisonState::Unavailable(AnovaFailure {
            kind,
            reason: "The comparison calculation is unavailable.".into(),
        });
        assert!(
            r.validate(&JobControl::default()).is_err(),
            "finite inference retained under {kind:?}"
        );
        let member = &mut r.families[0].members[0];
        member.t = unavailable();
        member.raw_p = unavailable();
        member.adjusted_p = unavailable();
        assert!(
            r.validate(&JobControl::default()).is_err(),
            "missing family failure for {kind:?}"
        );
        r.families[0].adjustment_failure = Some(AnovaFailure {
            kind: AnovaFailureKind::IncompleteFamily,
            reason: "Holm requires every raw probability in the captured family.".into(),
        });
        assert!(
            r.validate(&JobControl::default()).is_ok(),
            "captured comparison failure under {kind:?}"
        );
        let mut bad = r.clone();
        bad.families[0].members[0].standard_error = e(1.);
        assert!(
            bad.validate(&JobControl::default()).is_err(),
            "dependent SE retained under {kind:?}"
        );
    }
}
fn unpublishable_f_available_probability() -> ComputedAnova {
    // Finite data: group a=[2^550], b=[-1,1]. SSE=2, modelSS=(2/3)2^1100,
    // F=2^1100/3, df=(1,1), p=(2/pi)atan(sqrt(3)2^-550).
    let mut r = fitted();
    r.source_rows = 3;
    r.fitted_rows = 3;
    r.frequency = 3;
    r.observed_measurements = 3;
    r.grid.cells[0] = AnovaCell {
        physical_rows: 1,
        frequency: 1,
        mean: e(2f64.powi(550)),
    };
    r.grid.cells[1] = AnovaCell {
        physical_rows: 2,
        frequency: 2,
        mean: e(0.),
    };
    let carried = || AnovaEnergy {
        state: AnovaQuantityState::Positive,
        value: Estimate {
            value: None,
            reason: Some("The positive energy exceeds Float64 range.".into()),
        },
    };
    let AnovaOutcome::Fitted(m) = &mut r.outcome else {
        panic!()
    };
    m.model_ss = carried();
    m.total_ss = carried();
    m.residual_df = 1;
    m.total_df = 2;
    m.balanced = false;
    let effect = &mut m.effects[0];
    effect.ss = carried();
    effect.ms = unavailable();
    effect.error_ms = e(2.);
    effect.test.df2 = e(1.);
    effect.test.f = Estimate {
        value: None,
        reason: Some("The positive F statistic exceeds Float64 range.".into()),
    };
    effect.test.p = e((2. / std::f64::consts::PI) * (3f64.sqrt() * 2f64.powi(-550)).atan());
    effect.eta_squared = e(1.);
    effect.partial_eta_squared = e(1.);
    r
}
#[test]
fn unpublishable_f_retains_normal_probability() {
    assert!(unpublishable_f_available_probability()
        .validate(&JobControl::default())
        .is_ok());
}
#[test]
fn unpublishable_f_probability_requires_valid_df_and_energy() {
    for field in 0..4 {
        let mut r = unpublishable_f_available_probability();
        let AnovaOutcome::Fitted(m) = &mut r.outcome else {
            panic!()
        };
        match field {
            0 => m.effects[0].test.df1 = e(0.),
            1 => m.effects[0].test.df2 = e(0.),
            2 => m.effects[0].test.df1 = unavailable(),
            _ => m.effects[0].test.df2 = unavailable(),
        };
        assert!(
            r.validate(&JobControl::default()).is_err(),
            "df mutation{field}"
        );
    }
    let mut r = unpublishable_f_available_probability();
    let AnovaOutcome::Fitted(m) = &mut r.outcome else {
        panic!()
    };
    m.effects[0].ss.state = AnovaQuantityState::Unavailable(AnovaFailureKind::Numerical);
    m.effects[0].eta_squared = unavailable();
    m.effects[0].partial_eta_squared = unavailable();
    assert!(r.validate(&JobControl::default()).is_err());
    let mut r = unpublishable_f_available_probability();
    let AnovaOutcome::Fitted(m) = &mut r.outcome else {
        panic!()
    };
    m.error_ss.state = AnovaQuantityState::Unavailable(AnovaFailureKind::Numerical);
    m.error_ss.value = unavailable();
    m.effects[0].error_ss = m.error_ss.clone();
    m.effects[0].error_ms = unavailable();
    m.effects[0].partial_eta_squared = unavailable();
    assert!(r.validate(&JobControl::default()).is_err());
    for p in [0., f64::from_bits(1), 1. + f64::EPSILON] {
        let mut r = unpublishable_f_available_probability();
        let AnovaOutcome::Fitted(m) = &mut r.outcome else {
            panic!()
        };
        m.effects[0].test.p = e(p);
        assert!(r.validate(&JobControl::default()).is_err());
    }
}
