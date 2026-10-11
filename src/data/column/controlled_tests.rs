use super::*;
use crate::{error::AppError, operation::CHECKPOINT_HOOK};
fn ruled(column: Column, rules: MissingRules) -> Column {
    column
        .with_metadata(ColumnMetadata {
            missing_rules: rules,
            ..column.metadata().clone()
        })
        .unwrap()
}
fn cancelled_at(column: &Column, threshold: usize) {
    let control = JobControl::new(Arc::new(std::sync::atomic::AtomicBool::new(false)), None);
    let cancel = control.clone();
    CHECKPOINT_HOOK.with(|hook| {
        *hook.borrow_mut() = Some(Box::new(move |p| {
            if p.done >= threshold {
                cancel.cancel()
            }
        }))
    });
    let result = column.is_missing_controlled(0, &control);
    CHECKPOINT_HOOK.with(|hook| *hook.borrow_mut() = None);
    assert!(
        matches!(result, Err(AppError::Cancelled)),
        "inner lookup at {threshold}: {result:?}"
    );
}
#[test]
fn controlled_missing_checks_numeric_values_and_ranges_in_chunks() {
    let col = Column::numeric(ColumnId(1), "n", vec![Some(10000.)]).unwrap();
    cancelled_at(
        &ruled(
            col.clone(),
            MissingRules::Number {
                values: (0..200).map(|x| x as f64).collect(),
                ranges: vec![],
            },
        ),
        64,
    );
    cancelled_at(
        &ruled(
            col,
            MissingRules::Number {
                values: vec![],
                ranges: (0..200).map(|x| (x as f64, x as f64)).collect(),
            },
        ),
        64,
    );
}
#[test]
fn controlled_missing_checks_text_candidates_and_borrowed_utf8_bytes() {
    let col = Column::text(ColumnId(1), "t", vec![Some("outside".into())]).unwrap();
    cancelled_at(
        &ruled(
            col,
            MissingRules::Text((0..200).map(|i| i.to_string()).collect()),
        ),
        64,
    );
    let text = "ŵ🦀".repeat(10000);
    let col = Column::text(ColumnId(1), "t", vec![Some(text.clone())]).unwrap();
    cancelled_at(&ruled(col, MissingRules::Text(vec![text])), 4096);
}
#[test]
fn controlled_missing_preserves_uncontrolled_semantics_and_cancel_precedes_bad_row() {
    let numeric = ruled(
        Column::numeric(
            ColumnId(1),
            "n",
            vec![None, Some(-0.), Some(1.), Some(2.), Some(5.)],
        )
        .unwrap(),
        MissingRules::Number {
            values: vec![0.],
            ranges: vec![(1., 2.)],
        },
    );
    let text = ruled(
        Column::text(
            ColumnId(2),
            "t",
            vec![
                None,
                Some("".into()),
                Some("ŵ".into()),
                Some("w".into()),
                Some("ŵ ".into()),
            ],
        )
        .unwrap(),
        MissingRules::Text(vec!["".into(), "ŵ".into()]),
    );
    for column in [&numeric, &text] {
        for row in 0..5 {
            assert_eq!(
                column
                    .is_missing_controlled(row, &JobControl::default())
                    .unwrap(),
                column.is_missing(row).unwrap()
            );
        }
    }
    let control = JobControl::default();
    control.cancel();
    assert!(matches!(
        numeric.is_missing_controlled(usize::MAX, &control),
        Err(AppError::Cancelled)
    ));
    let (result, allocated) = crate::data::measure_test_allocations(|| numeric.is_missing(4));
    assert!(!result.unwrap());
    assert_eq!(allocated, 0);
}
