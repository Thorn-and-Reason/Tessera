use super::*;
use crate::{
    data::{Column, Dataset, MAX_OWNED_BYTES},
    error::AppError,
    operation::CHECKPOINT_HOOK,
};
fn capture() -> ReliabilityRun {
    let d = Dataset::new(vec![
        Column::numeric(ColumnId(1), "a", vec![Some(0.), Some(1.), Some(2.)]).unwrap(),
        Column::numeric(ColumnId(2), "b", vec![Some(1.), Some(2.), Some(3.)]).unwrap(),
    ])
    .unwrap();
    let r = ReliabilityRequest {
        items: vec![ColumnId(1), ColumnId(2)],
        filter: Default::default(),
        frequency_weight: None,
        reversals: vec![],
    };
    reliability_with_budget(
        &d,
        &r,
        &JobControl::default(),
        &[],
        std::mem::size_of::<ReliabilityRequest>() + r.heap_bytes().unwrap(),
    )
    .unwrap()
}
#[test]
fn captured_cancellation_and_capacity_admission_allocate_nothing() {
    let mut r = capture();
    let c = JobControl::default();
    c.cancel();
    let (result, largest) = crate::data::measure_test_allocations(|| r.validate_controlled(&c));
    assert!(matches!(result, Err(AppError::Cancelled)));
    assert_eq!(largest, 0);
    r.columns[0].description = String::with_capacity(MAX_OWNED_BYTES);
    let c = JobControl::default();
    let (result, largest) = crate::data::measure_test_allocations(|| r.validate_controlled(&c));
    assert!(matches!(result, Err(AppError::SizeLimit { .. })));
    assert_eq!(largest, 0);
}
#[test]
fn controlled_owned_and_snapshot_loops_cancel_inside_wide_metadata() {
    let mut r = capture();
    r.columns[0].description = "ŵ".repeat(9000);
    let c = JobControl::default();
    let cancel = c.clone();
    CHECKPOINT_HOOK.with(|h| {
        *h.borrow_mut() = Some(Box::new(move |p| {
            if p.done >= 4096 {
                cancel.cancel();
            }
        }))
    });
    let result = r.validate_controlled(&c);
    CHECKPOINT_HOOK.with(|h| *h.borrow_mut() = None);
    assert!(matches!(result, Err(AppError::Cancelled)));
    r.columns[0].description.clear();
    r.columns[0].missing_rules = crate::data::MissingRules::Text(vec![String::new(); 200]);
    let c = JobControl::default();
    let cancel = c.clone();
    CHECKPOINT_HOOK.with(|h| {
        *h.borrow_mut() = Some(Box::new(move |p| {
            if p.done >= 64 {
                cancel.cancel();
            }
        }))
    });
    let (result, largest) = crate::data::measure_test_allocations(|| r.validate_controlled(&c));
    CHECKPOINT_HOOK.with(|h| *h.borrow_mut() = None);
    assert!(matches!(result, Err(AppError::Cancelled)));
    assert_eq!(largest, 0);
}
#[test]
fn external_capture_budget_is_charged_before_cohort_allocation() {
    let d = Dataset::new(vec![
        Column::numeric(ColumnId(1), "a", vec![Some(1.)]).unwrap(),
        Column::numeric(ColumnId(2), "b", vec![Some(2.)]).unwrap(),
    ])
    .unwrap();
    let r = ReliabilityRequest {
        items: vec![ColumnId(1), ColumnId(2)],
        filter: Default::default(),
        frequency_weight: None,
        reversals: vec![],
    };
    let c = JobControl::default();
    let (result, largest) = crate::data::measure_test_allocations(|| {
        reliability_with_budget(&d, &r, &c, &[], MAX_OWNED_BYTES)
    });
    assert!(matches!(result, Err(AppError::SizeLimit { .. })));
    assert_eq!(largest, 0);
}
#[test]
fn capacities_and_validation_label_scratch_are_measured_separately() {
    use crate::data::{Cell, ValueLabel};
    let mut r = capture();
    let before = r.owned_bytes().unwrap();
    let old = r.columns[0].description.capacity();
    r.columns[0].description.reserve_exact(9000);
    assert_eq!(
        r.owned_bytes().unwrap() - before,
        r.columns[0].description.capacity() - old
    );
    let scratch = r.validation_scratch_bytes().unwrap();
    r.columns[0].value_labels = (0..100)
        .map(|i| ValueLabel {
            value: Cell::Number(i as f64),
            label: "label".into(),
        })
        .collect();
    assert_eq!(
        r.validation_scratch_bytes().unwrap() - scratch,
        200 * std::mem::size_of::<usize>()
    );
    r.validate_controlled(&JobControl::default()).unwrap();
}

#[test]
fn retained_sources_share_existing_budget_identity_rules_and_copies_overlap() {
    use crate::data::ColumnMetadata;
    fn source() -> Dataset {
        let col = Column::numeric(ColumnId(1), "a", vec![Some(0.), Some(1.), Some(2.)]).unwrap();
        let col = col
            .with_metadata(ColumnMetadata {
                description: String::with_capacity(100 * 1024 * 1024),
                ..col.metadata().clone()
            })
            .unwrap();
        Dataset::new(vec![
            col,
            Column::numeric(ColumnId(2), "b", vec![Some(1.), Some(2.), Some(3.)]).unwrap(),
        ])
        .unwrap()
    }
    let d = source();
    let shared = d.clone();
    let r = ReliabilityRequest {
        items: vec![ColumnId(1), ColumnId(2)],
        filter: Default::default(),
        frequency_weight: None,
        reversals: vec![],
    };
    let extra = std::mem::size_of::<ReliabilityRequest>() + r.heap_bytes().unwrap();
    let c = JobControl::default();
    let capture = reliability_with_budget(&d, &r, &c, &[&d, &d, &shared], extra).unwrap();
    capture.validate_controlled(&c).unwrap();
    assert_eq!(capture.items[0].mean.value, Some(1.));
    let distinct = source();
    let (result, largest) = crate::data::measure_test_allocations(|| {
        reliability_with_budget(&d, &r, &c, &[&distinct], extra)
    });
    assert!(matches!(result, Err(AppError::SizeLimit { .. })));
    assert!(
        largest < 16 * 1024,
        "capture refused before large copies: {largest}"
    );
}
