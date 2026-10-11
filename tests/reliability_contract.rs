use tessera::{
    analysis::{reliability::*, Estimate, RunColumnInfo, RunDiagnostics},
    data::{ColumnId, ColumnKind, MissingRules, RevisionId},
    error::AppError,
    operation::JobControl,
};
type Mutation = Box<dyn Fn(&mut ReliabilityRun)>;
fn available(v: f64) -> Estimate {
    Estimate {
        value: Some(v),
        reason: None,
    }
}
fn unavailable() -> Estimate {
    Estimate {
        value: None,
        reason: Some("Undefined".into()),
    }
}
fn run() -> ReliabilityRun {
    let items = [1, 2, 3]
        .map(|id| ReliabilityItem {
            column: ColumnId(id),
            mean: available(2.),
            sample_sd: available(1.),
            corrected_item_total: available(1.),
            raw_alpha_if_removed: available(1.),
            standardized_alpha_if_removed: available(1.),
            energy: ReliabilityEnergyState::Positive,
            remaining_energy: ReliabilityEnergyState::Positive,
            remaining_standardized_energy: ReliabilityEnergyState::Positive,
        })
        .to_vec();
    ReliabilityRun {
        revision: RevisionId(1),
        request: ReliabilityRequest {
            items: vec![ColumnId(1), ColumnId(2), ColumnId(3)],
            filter: Default::default(),
            frequency_weight: None,
            reversals: vec![],
        },
        columns: [1, 2, 3]
            .map(|id| RunColumnInfo {
                id: ColumnId(id),
                name: format!("Item {id}"),
                kind: ColumnKind::Number,
                description: String::new(),
                missing_rules: MissingRules::None,
                value_labels: vec![],
            })
            .to_vec(),
        source_rows: 3,
        diagnostics: RunDiagnostics::default(),
        complete_rows: 3,
        frequency: 3,
        incomplete_case_frequency: 0,
        items,
        raw_alpha: available(1.),
        standardized_alpha: available(1.),
        total_energy: ReliabilityEnergyState::Positive,
        standardized_total_energy: ReliabilityEnergyState::Positive,
        method: ReliabilityMethod::ExactCenteredIntervalV1,
    }
}
#[test]
fn valid_capture_accepts_arbitrary_metadata_order_and_conservative_refusal() {
    let mut r = run();
    r.columns.reverse();
    r.items[0].raw_alpha_if_removed = unavailable();
    r.validate_controlled(&JobControl::default()).unwrap();
}
#[test]
fn mutations_of_identity_counts_estimates_and_dependencies_are_rejected() {
    let mutations: Vec<Mutation> = vec![
        Box::new(|r| r.columns[1].id = r.columns[0].id),
        Box::new(|r| {
            r.columns.pop();
        }),
        Box::new(|r| r.items.swap(0, 1)),
        Box::new(|r| r.frequency = 2),
        Box::new(|r| r.source_rows = 4),
        Box::new(|r| r.items[0].mean = available(f64::NAN)),
        Box::new(|r| r.items[0].energy = ReliabilityEnergyState::ExactZero),
        Box::new(|r| r.total_energy = ReliabilityEnergyState::Unresolved),
        Box::new(|r| r.items[0].mean = unavailable()),
        Box::new(|r| r.items[0].sample_sd = available(-1.)),
        Box::new(|r| r.raw_alpha = available(1.1)),
        Box::new(|r| r.items[0].corrected_item_total = available(-1.1)),
        Box::new(|r| r.items[0].raw_alpha_if_removed.reason = Some("x".repeat(1025))),
    ];
    for (i, mutation) in mutations.iter().enumerate() {
        let mut r = run();
        mutation(&mut r);
        assert!(
            r.validate_controlled(&JobControl::default()).is_err(),
            "mutation {i}"
        );
    }
}
#[test]
fn supplied_cancellation_wins_over_malformed_capture() {
    let mut r = run();
    r.columns.clear();
    let c = JobControl::default();
    c.cancel();
    assert!(matches!(
        r.validate_controlled(&c),
        Err(AppError::Cancelled)
    ));
}
#[test]
fn spare_capacity_is_owned_and_over_budget_capture_is_refused() {
    let mut r = run();
    let before = r.owned_bytes().unwrap();
    r.items.reserve_exact(30);
    assert!(r.owned_bytes().unwrap() > before);
}

fn source() -> tessera::data::Dataset {
    use tessera::data::{Column, Dataset};
    Dataset::new(
        (1..=3)
            .map(|id| {
                Column::numeric(
                    ColumnId(id),
                    &format!("Original {id}"),
                    vec![Some(0.), Some(1.), Some(2.)],
                )
                .unwrap()
            })
            .collect(),
    )
    .unwrap()
}
#[test]
fn real_capture_retains_original_labels_bounds_and_passive_facts() {
    use tessera::data::{Column, Dataset};
    let d = source();
    let mut request = run().request;
    request.reversals = vec![ReliabilityReverse {
        column: ColumnId(1),
        lower: 0.,
        upper: 2.,
    }];
    let extra = std::mem::size_of::<ReliabilityRequest>() + request.heap_bytes().unwrap();
    let captured =
        reliability_with_budget(&d, &request, &JobControl::default(), &[], extra).unwrap();
    let changed = Dataset::new(vec![Column::text(
        ColumnId(1),
        "Renamed",
        vec![Some("changed".into()); 3],
    )
    .unwrap()])
    .unwrap();
    captured
        .validate_controlled(&JobControl::default())
        .unwrap();
    assert!(captured
        .request
        .validate_current(&changed, &[], 0, &JobControl::default())
        .is_err());
    assert_eq!(captured.columns[0].name, "Original 1");
    assert_eq!(captured.request.reversals, request.reversals);
    assert_eq!(captured.items[0].mean.value, Some(1.));
    assert_eq!(d.column(ColumnId(1)).unwrap().metadata().name, "Original 1");
}
#[test]
fn empty_single_and_replicated_captures_use_frozen_states() {
    use tessera::data::{Column, Dataset};
    for (rows, n) in [(0, 0), (1, 1), (1, 4)] {
        let d = Dataset::new(vec![
            Column::numeric(ColumnId(1), "a", vec![Some(1.); rows]).unwrap(),
            Column::numeric(ColumnId(2), "b", vec![Some(2.); rows]).unwrap(),
            Column::numeric(ColumnId(3), "w", vec![Some(n as f64); rows]).unwrap(),
        ])
        .unwrap();
        let mut r = run().request;
        r.items.truncate(2);
        r.frequency_weight = Some(ColumnId(3));
        let extra = std::mem::size_of::<ReliabilityRequest>() + r.heap_bytes().unwrap();
        let capture = reliability_with_budget(&d, &r, &JobControl::default(), &[], extra).unwrap();
        capture.validate_controlled(&JobControl::default()).unwrap();
        assert_eq!(
            capture.total_energy,
            if n < 2 {
                ReliabilityEnergyState::Unavailable
            } else {
                ReliabilityEnergyState::ExactZero
            }
        );
        let mut bad = capture.clone();
        bad.items[0].sample_sd = available(1.);
        assert!(bad.validate_controlled(&JobControl::default()).is_err());
    }
}
#[test]
fn unresolved_and_constant_prerequisites_are_distinct() {
    let mut r = run();
    r.items[0].energy = ReliabilityEnergyState::Unresolved;
    r.items[0].sample_sd = unavailable();
    r.items[0].corrected_item_total = unavailable();
    r.standardized_total_energy = ReliabilityEnergyState::Unresolved;
    r.standardized_alpha = unavailable();
    r.raw_alpha = unavailable();
    for i in 1..3 {
        r.items[i].remaining_standardized_energy = ReliabilityEnergyState::Unresolved;
        r.items[i].raw_alpha_if_removed = unavailable();
        r.items[i].standardized_alpha_if_removed = unavailable();
    }
    r.validate_controlled(&JobControl::default()).unwrap();
    let mut bad = r.clone();
    bad.standardized_total_energy = ReliabilityEnergyState::Unavailable;
    assert!(bad.validate_controlled(&JobControl::default()).is_err());
    r.items[0].energy = ReliabilityEnergyState::ExactZero;
    r.items[0].sample_sd = available(0.);
    r.standardized_total_energy = ReliabilityEnergyState::Unavailable;
    for i in 1..3 {
        r.items[i].remaining_standardized_energy = ReliabilityEnergyState::Unavailable;
    }
    r.validate_controlled(&JobControl::default()).unwrap();
}
#[test]
fn metadata_kinds_roles_rules_label_duplicates_and_oversized_reason_fail() {
    use tessera::data::{Cell, ValueLabel};
    let mutations: Vec<Mutation> = vec![
        Box::new(|r| r.columns[0].kind = ColumnKind::Text),
        Box::new(|r| r.columns[0].name = " \n".into()),
        Box::new(|r| {
            r.columns[0].missing_rules = MissingRules::Number {
                values: vec![f64::INFINITY],
                ranges: vec![],
            }
        }),
        Box::new(|r| {
            r.columns[0].value_labels = vec![
                ValueLabel {
                    value: Cell::Number(0.),
                    label: "a".into(),
                },
                ValueLabel {
                    value: Cell::Number(-0.),
                    label: "b".into(),
                },
            ]
        }),
        Box::new(|r| {
            r.items[0].corrected_item_total = Estimate {
                value: None,
                reason: Some("x".repeat(1025)),
            }
        }),
        Box::new(|r| {
            r.columns.push(RunColumnInfo {
                id: ColumnId(99),
                ..r.columns[0].clone()
            })
        }),
    ];
    for mutation in mutations {
        let mut r = run();
        mutation(&mut r);
        assert!(r.validate_controlled(&JobControl::default()).is_err());
    }
}

#[test]
fn passive_capture_does_not_authenticate_numerical_truth() {
    let mut r = run();
    r.items[0].mean = available(12345.);
    r.raw_alpha = available(-999.);
    r.validate_controlled(&JobControl::default()).unwrap();
}

#[test]
fn real_captures_accept_constant_cancellation_and_extreme_scale_publications() {
    use tessera::data::{Column, Dataset};
    let tiny = f64::from_bits(1);
    for rows in [
        vec![vec![0., 0., 0.], vec![0., 1., 2.], vec![0., 2., 4.]],
        vec![vec![-f64::MAX, -f64::MAX], vec![f64::MAX, f64::MAX]],
        vec![vec![0., tiny], vec![tiny, 0.]],
        vec![vec![-1., 1., 0.], vec![0., 0., 1.], vec![1., -1., 2.]],
    ] {
        let k = rows[0].len();
        let d = Dataset::new(
            (0..k)
                .map(|i| {
                    Column::numeric(
                        ColumnId(i as u64 + 1),
                        &format!("item {i}"),
                        rows.iter().map(|r| Some(r[i])).collect(),
                    )
                    .unwrap()
                })
                .collect(),
        )
        .unwrap();
        let r = ReliabilityRequest {
            items: (1..=k as u64).map(ColumnId).collect(),
            filter: Default::default(),
            frequency_weight: None,
            reversals: vec![],
        };
        let extra = std::mem::size_of::<ReliabilityRequest>() + r.heap_bytes().unwrap();
        let capture = reliability_with_budget(&d, &r, &JobControl::default(), &[], extra).unwrap();
        capture.validate_controlled(&JobControl::default()).unwrap();
    }
}
