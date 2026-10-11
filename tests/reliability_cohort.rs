use tessera::{
    analysis::{reliability::*, NumericComparison, RowFilter, RowPredicate},
    data::{Column, ColumnId, Dataset},
    error::AppError,
    operation::JobControl,
};
fn request() -> ReliabilityRequest {
    ReliabilityRequest {
        items: vec![ColumnId(1), ColumnId(2)],
        filter: RowFilter::default(),
        frequency_weight: None,
        reversals: vec![],
    }
}
fn data(a: Vec<Option<f64>>, b: Vec<Option<f64>>, w: Vec<Option<f64>>) -> Dataset {
    Dataset::new(vec![
        Column::numeric(ColumnId(1), "a", a).unwrap(),
        Column::numeric(ColumnId(2), "b", b).unwrap(),
        Column::numeric(ColumnId(3), "w", w).unwrap(),
    ])
    .unwrap()
}
#[test]
fn rejects_malformed_requests() {
    let r = request();
    for items in [
        vec![],
        vec![ColumnId(1)],
        vec![ColumnId(1), ColumnId(1)],
        vec![ColumnId(0), ColumnId(2)],
        (1..=65).map(ColumnId).collect(),
    ] {
        assert!(ReliabilityRequest { items, ..r.clone() }.admit().is_err());
    }
    for (id, l, u) in [
        (3, 0., 1.),
        (1, 1., 1.),
        (1, 2., 1.),
        (1, f64::NAN, 1.),
        (1, 0., f64::INFINITY),
    ] {
        assert!(ReliabilityRequest {
            reversals: vec![ReliabilityReverse {
                column: ColumnId(id),
                lower: l,
                upper: u
            }],
            ..r.clone()
        }
        .admit()
        .is_err());
    }
    let reverse = ReliabilityReverse {
        column: ColumnId(1),
        lower: 0.,
        upper: 1.,
    };
    assert!(ReliabilityRequest {
        reversals: vec![reverse.clone(), reverse],
        ..r
    }
    .admit()
    .is_err());
}
#[test]
fn counts_complete_rows_and_frequencies() {
    let d = data(
        vec![Some(1.), None, Some(4.)],
        vec![Some(2.), Some(3.), Some(5.)],
        vec![Some(2.), Some(3.), Some(0.)],
    );
    let r = ReliabilityRequest {
        frequency_weight: Some(ColumnId(3)),
        ..request()
    };
    let c = capture_cohort(&d, &r, &JobControl::default()).unwrap();
    assert_eq!(
        (c.complete_rows, c.frequency, c.incomplete_case_frequency),
        (1, 2, 3)
    );
    assert_eq!(c.diagnostics.complete_case_rows, 1);
    assert_eq!(c.diagnostics.zero_weight_rows, 1);
}
#[test]
fn validates_bounds_on_incomplete_base_rows() {
    let d = data(vec![Some(8.)], vec![None], vec![Some(1.)]);
    let mut r = request();
    r.reversals = vec![ReliabilityReverse {
        column: ColumnId(1),
        lower: 0.,
        upper: 5.,
    }];
    assert!(capture_cohort(&d, &r, &JobControl::default()).is_err());
}
#[test]
fn reversal_uses_exact_sum_and_preserves_source() {
    let d = data(vec![Some(f64::MAX)], vec![Some(1.)], vec![Some(1.)]);
    let mut r = request();
    r.reversals = vec![ReliabilityReverse {
        column: ColumnId(1),
        lower: f64::MAX / 2.,
        upper: f64::MAX,
    }];
    capture_cohort(&d, &r, &JobControl::default()).unwrap();
    assert_eq!(
        score_item(&d, &r, 0, 0).unwrap().to_bits(),
        (f64::MAX / 2.).to_bits()
    );
    assert_eq!(
        d.cell(ColumnId(1), 0).unwrap(),
        tessera::data::Cell::Number(f64::MAX)
    );
}
#[test]
fn weights_precede_missingness_but_follow_filter() {
    let d = data(vec![None], vec![None], vec![Some(-1.)]);
    let mut r = ReliabilityRequest {
        frequency_weight: Some(ColumnId(3)),
        ..request()
    };
    assert!(capture_cohort(&d, &r, &JobControl::default()).is_err());
    r.filter.predicates.push(RowPredicate::Number {
        column: ColumnId(3),
        comparison: NumericComparison::Greater,
        value: 0.,
    });
    assert_eq!(
        capture_cohort(&d, &r, &JobControl::default())
            .unwrap()
            .frequency,
        0
    );
}
#[test]
fn supplied_cancel_wins_before_bad_roles() {
    let d = data(vec![], vec![], vec![]);
    let mut r = request();
    r.items.clear();
    let control = JobControl::default();
    control.cancel();
    assert!(matches!(
        capture_cohort(&d, &r, &control),
        Err(AppError::Cancelled)
    ));
}
#[test]
fn current_roles_require_numeric_items_weights_and_typed_filters() {
    let d = Dataset::new(vec![
        Column::text(ColumnId(1), "text", vec![Some("1".into())]).unwrap(),
        Column::numeric(ColumnId(2), "number", vec![Some(2.)]).unwrap(),
    ])
    .unwrap();
    let r = request();
    assert!(r
        .validate_current(&d, &[], 0, &JobControl::default())
        .is_err());
    let d = data(vec![Some(1.)], vec![Some(2.)], vec![Some(1.)]);
    let mut r = request();
    r.items[1] = ColumnId(99);
    assert!(capture_cohort(&d, &r, &JobControl::default()).is_err());
    let mut r = request();
    r.filter.predicates.push(RowPredicate::Text {
        column: ColumnId(1),
        value: "1".into(),
        equal: true,
    });
    assert!(capture_cohort(&d, &r, &JobControl::default()).is_err());
    let d = Dataset::new(vec![
        Column::numeric(ColumnId(1), "a", vec![Some(1.)]).unwrap(),
        Column::numeric(ColumnId(2), "b", vec![Some(2.)]).unwrap(),
        Column::text(ColumnId(3), "weight", vec![Some("1".into())]).unwrap(),
    ])
    .unwrap();
    assert!(capture_cohort(
        &d,
        &ReliabilityRequest {
            frequency_weight: Some(ColumnId(3)),
            ..request()
        },
        &JobControl::default()
    )
    .is_err());
}
#[test]
fn base_frequency_ceiling_precedes_exclusion_of_incomplete_rows() {
    let d = data(
        vec![None, None],
        vec![None, None],
        vec![Some((1u64 << 53) as f64), Some(1.)],
    );
    assert!(capture_cohort(
        &d,
        &ReliabilityRequest {
            frequency_weight: Some(ColumnId(3)),
            ..request()
        },
        &JobControl::default()
    )
    .is_err());
}
#[test]
fn reversed_bounds_ignore_filtered_zero_and_missing_weight_rows() {
    let d = data(
        vec![Some(8.), Some(8.), Some(8.), Some(1.)],
        vec![Some(1.); 4],
        vec![Some(0.), None, Some(1.), Some(2.)],
    );
    let r = ReliabilityRequest {
        frequency_weight: Some(ColumnId(3)),
        filter: RowFilter {
            predicates: vec![RowPredicate::Number {
                column: ColumnId(3),
                comparison: NumericComparison::NotEqual,
                value: 1.,
            }],
        },
        reversals: vec![ReliabilityReverse {
            column: ColumnId(1),
            lower: 0.,
            upper: 5.,
        }],
        ..request()
    };
    let c = capture_cohort(&d, &r, &JobControl::default()).unwrap();
    assert_eq!((c.complete_rows, c.frequency), (1, 2));
    assert_eq!(c.diagnostics.zero_weight_rows, 1);
    // The numeric predicate itself excludes missing weight; retain it to exercise the weight exclusion separately.
    let d = data(
        vec![Some(8.), Some(8.), Some(1.)],
        vec![Some(1.); 3],
        vec![Some(0.), None, Some(2.)],
    );
    let r = ReliabilityRequest {
        filter: RowFilter::default(),
        ..r
    };
    let c = capture_cohort(&d, &r, &JobControl::default()).unwrap();
    assert_eq!(c.diagnostics.missing_weight_rows, 1);
    assert_eq!(c.frequency, 2);
}
#[test]
fn source_missing_rules_precede_reversal_and_metadata_remains_unchanged() {
    use tessera::data::{Cell, ColumnMetadata, MissingRules, ValueLabel};
    let col = Column::numeric(
        ColumnId(1),
        "original",
        vec![Some(-99.), Some(0.), Some(5.)],
    )
    .unwrap();
    let metadata = ColumnMetadata {
        missing_rules: MissingRules::Number {
            values: vec![-99.],
            ranges: vec![],
        },
        value_labels: vec![ValueLabel {
            value: Cell::Number(0.),
            label: "source lower".into(),
        }],
        ..col.metadata().clone()
    };
    let d = Dataset::new(vec![
        col.with_metadata(metadata.clone()).unwrap(),
        Column::numeric(ColumnId(2), "b", vec![Some(1.); 3]).unwrap(),
    ])
    .unwrap();
    let revision = d.revision();
    let r = ReliabilityRequest {
        reversals: vec![ReliabilityReverse {
            column: ColumnId(1),
            lower: 0.,
            upper: 5.,
        }],
        ..request()
    };
    let c = capture_cohort(&d, &r, &JobControl::default()).unwrap();
    assert_eq!(
        (c.complete_rows, c.frequency, c.incomplete_case_frequency),
        (2, 2, 1)
    );
    assert_eq!(score_item(&d, &r, 1, 0).unwrap(), 5.);
    assert_eq!(score_item(&d, &r, 2, 0).unwrap().to_bits(), 0);
    assert_eq!(d.revision(), revision);
    assert_eq!(d.column(ColumnId(1)).unwrap().metadata(), &metadata);
    assert_eq!(d.cell(ColumnId(1), 0).unwrap(), Cell::Number(-99.));
}
#[test]
fn zero_one_and_one_replicated_physical_row_have_separate_counts() {
    for (a, b, w, physical, frequency) in [
        (vec![], vec![], vec![], 0, 0),
        (vec![Some(1.)], vec![Some(2.)], vec![Some(1.)], 1, 1),
        (vec![Some(1.)], vec![Some(2.)], vec![Some(2.)], 1, 2),
    ] {
        let d = data(a, b, w);
        let c = capture_cohort(
            &d,
            &ReliabilityRequest {
                frequency_weight: Some(ColumnId(3)),
                ..request()
            },
            &JobControl::default(),
        )
        .unwrap();
        assert_eq!((c.complete_rows, c.frequency), (physical, frequency));
    }
}
#[test]
fn ordered_items_and_capture_preserve_exact_recipe_and_capacity_accounting() {
    let mut r = request();
    let before = r.heap_bytes().unwrap();
    r.items.reserve_exact(100);
    assert!(r.heap_bytes().unwrap() > before);
    r.items.reverse();
    r.reversals.push(ReliabilityReverse {
        column: ColumnId(1),
        lower: 0.,
        upper: 5.,
    });
    let saved = capture_request(&r, &JobControl::default()).unwrap();
    assert_eq!(saved, r);
    let d = data(vec![Some(0.)], vec![Some(2.)], vec![Some(1.)]);
    let c = capture_cohort(&d, &r, &JobControl::default()).unwrap();
    assert_eq!(c.complete_rows, 1);
    assert_eq!(score_item(&d, &r, 0, 0).unwrap(), 2.);
    assert_eq!(score_item(&d, &r, 0, 1).unwrap(), 5.);
    r.items[0] = ColumnId(3);
    assert_ne!(saved, r);
    assert!(score_item(&d, &saved, 0, usize::MAX).is_err());
    assert!(score_item(&d, &saved, usize::MAX, 0).is_err());
}
#[test]
fn public_score_leaf_rejects_bad_type_missing_value_and_nonfinite_bounds() {
    let d = data(vec![None], vec![Some(2.)], vec![Some(1.)]);
    assert!(score_item(&d, &request(), 0, 0).is_err());
    let d = data(vec![Some(1.)], vec![Some(2.)], vec![Some(1.)]);
    let r = ReliabilityRequest {
        reversals: vec![ReliabilityReverse {
            column: ColumnId(1),
            lower: f64::NAN,
            upper: 5.,
        }],
        ..request()
    };
    assert!(score_item(&d, &r, 0, 0).is_err());
    let d = Dataset::new(vec![Column::text(
        ColumnId(1),
        "text",
        vec![Some("1".into())],
    )
    .unwrap()])
    .unwrap();
    assert!(score_item(&d, &request(), 0, 0).is_err());
}
#[test]
fn quiet_control_forwarding_preserves_caller_progress_channel() {
    use std::sync::{atomic::AtomicBool, mpsc, Arc};
    let (send, recv) = mpsc::channel();
    let control = JobControl::new(Arc::new(AtomicBool::new(false)), Some(send));
    let d = data(vec![Some(1.)], vec![Some(2.)], vec![Some(1.)]);
    capture_cohort(&d, &request(), &control).unwrap();
    let updates: Vec<_> = recv.try_iter().collect();
    assert!(!updates.is_empty());
    assert!(updates.iter().all(|p| p.total.is_some()));
    control.cancel();
    assert!(matches!(
        capture_request(&request(), &control),
        Err(AppError::Cancelled)
    ));
}
