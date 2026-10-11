use super::*;
use crate::data::Column;
fn data() -> Dataset {
    Dataset::new(
        (1..=3)
            .map(|i| {
                Column::numeric(ColumnId(i), &format!("Item {i}"), vec![Some(0.), Some(1.)])
                    .unwrap()
            })
            .collect(),
    )
    .unwrap()
}
fn request() -> ReliabilityRequest {
    ReliabilityRequest {
        items: vec![ColumnId(2), ColumnId(1)],
        reversals: vec![ReliabilityReverse {
            column: ColumnId(2),
            lower: -f64::MIN_POSITIVE,
            upper: f64::MAX,
        }],
        filter: RowFilter::default(),
        frequency_weight: None,
    }
}
#[test]
fn reliability_draft_keeps_order_exact_bounds_and_current_validation() {
    let r = request();
    let mut d = ReliabilityDraft::from_request(r.clone());
    assert!(d.matches_saved(&r));
    assert_eq!(d.build_request(&data()).unwrap(), r);
    assert!(
        ReliabilityDraft::conversion_bytes(&r).unwrap()
            >= d.reversals[0].lower.capacity()
                + d.reversals[0].upper.capacity()
                + size_of::<ReverseDraft>()
    );
    d.items.swap(0, 1);
    assert!(!d.matches_saved(&r));
    assert_eq!(
        d.build_request(&data()).unwrap().items,
        vec![ColumnId(1), ColumnId(2)]
    );
    for bad in ["", "abc", "NaN", "inf", "-inf"] {
        d.reversals[0].lower = bad.into();
        assert!(d.build_request(&data()).is_err());
    }
    d.reversals[0].lower = "1".into();
    d.reversals[0].upper = "1".into();
    assert!(d.build_request(&data()).is_err());
    d.reversals[0].upper = "0".into();
    assert!(d.build_request(&data()).is_err());
    d.reversals[0].upper = "2".into();
    d.reversals[0].column = ColumnId(3);
    assert!(d.build_request(&data()).is_err());
    d.reversals.clear();
    d.items = vec![ColumnId(1), ColumnId(1)];
    assert!(d.build_request(&data()).is_err());
    d.items = vec![ColumnId(1)];
    assert!(d.build_request(&data()).is_err());
}
#[test]
fn reliability_draft_preserves_typed_filter_frequency_and_refuses_changed_items() {
    let mut r = request();
    r.frequency_weight = Some(ColumnId(3));
    r.filter.predicates = vec![RowPredicate::Number {
        column: ColumnId(3),
        comparison: NumericComparison::Greater,
        value: 0.,
    }];
    let d = ReliabilityDraft::from_request(r.clone());
    assert_eq!(d.build_request(&data()).unwrap(), r);
    let changed = Dataset::new(vec![
        Column::text(
            ColumnId(1),
            "Changed",
            vec![Some("x".into()), Some("y".into())],
        )
        .unwrap(),
        data().column(ColumnId(2)).unwrap().clone(),
        data().column(ColumnId(3)).unwrap().clone(),
    ])
    .unwrap();
    assert!(d.matches_saved(&r));
    assert!(d.build_request(&changed).is_err());
}

#[test]
fn reliability_draft_accepts_64_numeric_items_and_refuses_65_or_duplicate_reversals() {
    let data = Dataset::new(
        (1..=65)
            .map(|i| {
                Column::numeric(ColumnId(i), &format!("Item {i}"), vec![Some(0.), Some(1.)])
                    .unwrap()
            })
            .collect(),
    )
    .unwrap();
    let mut draft = ReliabilityDraft {
        items: (1..=64).map(ColumnId).collect(),
        ..Default::default()
    };
    assert_eq!(draft.build_request(&data).unwrap().items.len(), 64);
    draft.items.push(ColumnId(65));
    assert!(draft.build_request(&data).is_err());
    draft.items.pop();
    draft.reversals = vec![
        ReverseDraft {
            column: ColumnId(1),
            lower: "0".into(),
            upper: "1".into()
        };
        2
    ];
    assert!(draft.build_request(&data).is_err());
}
