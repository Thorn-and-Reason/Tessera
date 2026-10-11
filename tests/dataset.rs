#[path = "dataset/budget.rs"]
mod budget_tests;
mod common;
#[path = "dataset/conversion.rs"]
mod conversion_tests;
use common::sample_dataset;
use std::sync::{atomic::AtomicBool, mpsc, Arc};
use tessera::data::{
    Budget, Cell, Column, ColumnId, ColumnKind, ColumnMetadata, Dataset, Measurement, MissingRules,
    RevisionId, ValueLabel, MAX_CELLS,
};
use tessera::error::AppError;
use tessera::operation::{JobControl, Progress, RunId};

#[test]
fn edits_preserve_source_and_unaffected_chunks() {
    let before = sample_dataset();
    let after = before
        .edit_cell(ColumnId(6), 0, Cell::Number(18.0))
        .unwrap();
    assert_eq!(before.cell(ColumnId(6), 0).unwrap(), Cell::Number(17.0));
    assert_eq!(after.cell(ColumnId(6), 0).unwrap(), Cell::Number(18.0));
    assert_ne!(before.revision(), after.revision());
    assert_eq!(
        before.cell(ColumnId(2), 0).unwrap(),
        after.cell(ColumnId(2), 0).unwrap()
    );
    let appended = after.append_row().unwrap();
    assert_eq!(appended.row_count(), 7);
    for column in appended.columns() {
        assert_eq!(appended.cell(column.id(), 6).unwrap(), Cell::Missing);
    }
    let added = appended
        .append_column(Column::numeric(ColumnId(7), "Added", vec![None; 7]).unwrap())
        .unwrap();
    assert_eq!(added.column_count(), 7);
    assert_eq!(after.row_count(), 6);
}

#[test]
fn invalid_edits_are_transactional() {
    let before = sample_dataset();
    for value in [
        Cell::Number(f64::NAN),
        Cell::Number(f64::INFINITY),
        Cell::Text("18".into()),
    ] {
        assert!(before.edit_cell(ColumnId(6), 0, value).is_err());
    }
    assert!(before.edit_cell(ColumnId(0), 0, Cell::Missing).is_err());
    assert!(before.edit_cell(ColumnId(6), 6, Cell::Missing).is_err());
    assert!(before
        .append_column(Column::numeric(ColumnId(7), "Added", vec![None; 5]).unwrap())
        .is_err());
    assert!(before
        .replace_column(Column::numeric(ColumnId(99), "Unknown", vec![None; 6]).unwrap())
        .is_err());
    assert_eq!(before.revision(), RevisionId(1));
    assert_eq!(before.cell(ColumnId(6), 0).unwrap(), Cell::Number(17.0));
}

#[test]
fn metadata_missing_rules_and_labels_are_typed_and_finite() {
    let column = Column::numeric(ColumnId(1), "A", vec![Some(-99.0), Some(5.0), None]).unwrap();
    let metadata = ColumnMetadata {
        name: "Renamed â".into(),
        description: "Description".into(),
        measurement: Measurement::Scale,
        missing_rules: MissingRules::Number {
            values: vec![-99.0],
            ranges: vec![(10.0, 20.0)],
        },
        value_labels: vec![ValueLabel {
            value: Cell::Number(5.0),
            label: "Five".into(),
        }],
        display_decimals: 16,
    };
    let renamed = column.with_metadata(metadata.clone()).unwrap();
    assert!(renamed.is_missing(0).unwrap());
    assert!(!renamed.is_missing(1).unwrap());
    assert!(renamed.is_missing(2).unwrap());
    assert_eq!(renamed.id(), ColumnId(1));
    for rules in [
        MissingRules::Number {
            values: vec![f64::NAN],
            ranges: vec![],
        },
        MissingRules::Number {
            values: vec![],
            ranges: vec![(2.0, 1.0)],
        },
        MissingRules::Text(vec!["x".into()]),
    ] {
        assert!(column
            .with_metadata(ColumnMetadata {
                missing_rules: rules,
                ..metadata.clone()
            })
            .is_err());
    }
    assert!(column
        .with_metadata(ColumnMetadata {
            display_decimals: 17,
            ..metadata.clone()
        })
        .is_err());
    assert!(column
        .with_metadata(ColumnMetadata {
            value_labels: vec![ValueLabel {
                value: Cell::Number(f64::INFINITY),
                label: "Bad".into()
            }],
            ..metadata
        })
        .is_err());
    let text = Column::text(
        ColumnId(2),
        "T",
        vec![Some("missing".into()), Some("".into())],
    )
    .unwrap();
    let marked = text
        .with_metadata(ColumnMetadata {
            missing_rules: MissingRules::Text(vec!["missing".into()]),
            ..text.metadata().clone()
        })
        .unwrap();
    assert!(marked.is_missing(0).unwrap());
    assert!(!marked.is_missing(1).unwrap());
}

#[test]
fn checked_identity_exhaustion_and_persisted_revisions() {
    assert!(RevisionId(0).next().is_err());
    assert!(RevisionId(u64::MAX).next().is_err());
    assert!(ColumnId(u64::MAX).next().is_err());
    assert!(RunId(u64::MAX).next().is_err());
    assert_eq!(RunId(1).next().unwrap(), RunId(2));
    let ds =
        Dataset::from_revision(sample_dataset().columns().to_vec(), RevisionId(u64::MAX)).unwrap();
    assert!(ds.edit_cell(ColumnId(6), 0, Cell::Number(18.0)).is_err());
    assert_eq!(ds.cell(ColumnId(6), 0).unwrap(), Cell::Number(17.0));
    assert!(Dataset::from_revision(vec![], RevisionId(0)).is_err());
    let ds = Dataset::new(vec![
        Column::numeric(ColumnId(u64::MAX), "last", vec![]).unwrap()
    ])
    .unwrap();
    assert!(ds.next_column_id().is_err());
}

#[test]
fn cancellation_and_progress_use_shared_control() {
    let (sender, receiver) = mpsc::channel();
    let cancelled = Arc::new(AtomicBool::new(false));
    let control = JobControl::new(cancelled, Some(sender));
    control.checkpoint(2, Some(6)).unwrap();
    assert_eq!(
        receiver.recv().unwrap(),
        Progress {
            done: 2,
            total: Some(6)
        }
    );
    control.cancel();
    assert!(matches!(
        control.checkpoint(3, Some(6)),
        Err(AppError::Cancelled)
    ));
    assert!(receiver.try_recv().is_err());
    JobControl::default().checkpoint(0, None).unwrap();
}

#[test]
fn edits_at_chunk_boundaries_and_text_rows_preserve_source() {
    let ds = Dataset::new(vec![Column::text(
        ColumnId(1),
        "T",
        vec![Some("old".into()); 4096],
    )
    .unwrap()])
    .unwrap();
    let appended = ds.append_row().unwrap();
    assert_eq!(appended.cell(ColumnId(1), 4096).unwrap(), Cell::Missing);
    let edited = appended
        .edit_cell(ColumnId(1), 4096, Cell::Text("".into()))
        .unwrap();
    assert_eq!(
        edited.cell(ColumnId(1), 4096).unwrap(),
        Cell::Text("".into())
    );
    assert_eq!(appended.cell(ColumnId(1), 4096).unwrap(), Cell::Missing);
    let deleted = edited.edit_cell(ColumnId(1), 0, Cell::Missing).unwrap();
    assert_eq!(deleted.cell(ColumnId(1), 0).unwrap(), Cell::Missing);
    assert_eq!(ds.cell(ColumnId(1), 0).unwrap(), Cell::Text("old".into()));
    assert!(Dataset::new(vec![]).unwrap().append_row().is_err());
    let first = Dataset::new(vec![])
        .unwrap()
        .append_column(Column::numeric(ColumnId(1), "N", vec![Some(2.0)]).unwrap())
        .unwrap();
    assert_eq!(first.row_count(), 1);
    assert_eq!(first.next_column_id().unwrap(), ColumnId(2));
    assert!(first
        .append_column(Column::numeric(ColumnId(1), "Duplicate", vec![None]).unwrap())
        .is_err());
    assert!(first
        .append_column(Column::numeric(ColumnId(2), "N", vec![None]).unwrap())
        .is_err());
    let ds = sample_dataset();
    assert!(ds
        .replace_column(
            ds.column(ColumnId(6))
                .unwrap()
                .with_metadata(ColumnMetadata {
                    name: "Age".into(),
                    ..ds.column(ColumnId(6)).unwrap().metadata().clone()
                })
                .unwrap()
        )
        .is_err());
}

#[test]
fn approved_fixture_has_continuous_and_category_metadata() {
    let ds = sample_dataset();
    for column in ds.columns() {
        assert_eq!(column.metadata().display_decimals, 0);
        assert_eq!(
            column.metadata().measurement,
            match column.kind() {
                ColumnKind::Number => Measurement::Scale,
                ColumnKind::Text => Measurement::Nominal,
            }
        );
        assert_eq!(
            column.metadata().description,
            if column.id() == ColumnId(6) {
                "Sum of the two scale items"
            } else {
                ""
            }
        );
        assert_eq!(column.metadata().missing_rules, MissingRules::None);
        assert!(column.metadata().value_labels.is_empty());
    }
}

#[test]
fn generated_numeric_text_roundtrips_even_for_large_float_integers() {
    for number in [
        1e23,
        -1e23,
        1e300,
        -1e300,
        f64::MAX,
        -f64::MAX,
        0.0,
        -0.0,
        f64::MIN_POSITIVE,
        1.23456789,
    ] {
        let column = Column::numeric(ColumnId(1), "N", vec![Some(number)]).unwrap();
        let text = column.convert(ColumnKind::Text).unwrap();
        let roundtrip = text.convert(ColumnKind::Number).unwrap();
        let Cell::Number(recovered) = roundtrip.cell(0).unwrap() else {
            panic!("numeric cell expected")
        };
        assert_eq!(recovered.to_bits(), number.to_bits());
    }
}

#[test]
fn text_values_borrow_original_unicode_storage_and_distinguish_null_empty() {
    let unicode = "Cymraeg: ŵ, â; 日本語 🐑".to_owned();
    let original_pointer = unicode.as_ptr();
    let column = Column::text(
        ColumnId(1),
        "T",
        vec![Some(unicode), None, Some(String::new())],
    )
    .unwrap();
    let borrowed = column.text_value(0).unwrap().unwrap();
    assert_eq!(borrowed, "Cymraeg: ŵ, â; 日本語 🐑");
    assert_eq!(borrowed.as_ptr(), original_pointer);
    assert_eq!(
        column.text_value(0).unwrap().unwrap().as_ptr(),
        borrowed.as_ptr()
    );
    assert_eq!(column.text_value(1).unwrap(), None);
    assert_eq!(column.text_value(2).unwrap(), Some(""));
    assert!(matches!(
        column.text_value(3),
        Err(AppError::InvalidInput { .. })
    ));
    assert!(matches!(
        column.text_value(usize::MAX),
        Err(AppError::InvalidInput { .. })
    ));
    let numeric = Column::numeric(ColumnId(2), "N", vec![Some(1.0), None]).unwrap();
    assert!(matches!(
        numeric.text_value(0),
        Err(AppError::InvalidInput { .. })
    ));
    assert!(matches!(
        numeric.text_value(1),
        Err(AppError::InvalidInput { .. })
    ));
}

mod budget_allocation_observer {
    use std::alloc::{GlobalAlloc, Layout, System};
    struct Observer;
    #[global_allocator]
    static ALLOCATOR: Observer = Observer;
    std::thread_local! {
        static ACTIVE: std::cell::Cell<bool> = const { std::cell::Cell::new(false) };
        static ALLOCATED: std::cell::Cell<usize> = const { std::cell::Cell::new(0) };
        static LARGEST: std::cell::Cell<usize> = const { std::cell::Cell::new(0) };
        static CANCEL_SIZE: std::cell::Cell<usize> = const { std::cell::Cell::new(0) };
        static CANCEL_MATCHES: std::cell::Cell<usize> = const { std::cell::Cell::new(0) };
        static CANCEL_FLAG: std::cell::RefCell<Option<std::sync::Arc<std::sync::atomic::AtomicBool>>> = const { std::cell::RefCell::new(None) };
    }
    // System retains responsibility for memory; tests observe allocations on their own thread.
    unsafe impl GlobalAlloc for Observer {
        unsafe fn alloc(&self, layout: Layout) -> *mut u8 {
            record(layout.size());
            unsafe { System.alloc(layout) }
        }
        unsafe fn dealloc(&self, pointer: *mut u8, layout: Layout) {
            unsafe { System.dealloc(pointer, layout) }
        }
        unsafe fn realloc(&self, pointer: *mut u8, layout: Layout, size: usize) -> *mut u8 {
            record(size);
            unsafe { System.realloc(pointer, layout, size) }
        }
    }
    fn record(bytes: usize) {
        let _ = CANCEL_SIZE.try_with(|size| {
            if size.get() != 0 && size.get() == bytes {
                let _ = CANCEL_MATCHES.try_with(|matched| matched.set(matched.get() + 1));
                let _ = CANCEL_FLAG.try_with(|flag| {
                    if let Some(flag) = flag.borrow().as_ref() {
                        flag.store(true, std::sync::atomic::Ordering::Release);
                    }
                });
            }
        });

        let _ = ACTIVE.try_with(|active| {
            if active.get() {
                let _ = LARGEST.try_with(|largest| largest.set(largest.get().max(bytes)));
                let _ = ALLOCATED
                    .try_with(|allocated| allocated.set(allocated.get().saturating_add(bytes)));
            }
        });
    }
    pub fn cancel_on_allocation<T>(
        size: usize,
        flag: std::sync::Arc<std::sync::atomic::AtomicBool>,
        action: impl FnOnce() -> T,
    ) -> (T, usize) {
        CANCEL_SIZE.with(|value| value.set(size));
        CANCEL_MATCHES.with(|matched| matched.set(0));
        CANCEL_FLAG.with(|stored| *stored.borrow_mut() = Some(flag));
        let result = action();
        CANCEL_SIZE.with(|value| value.set(0));
        CANCEL_FLAG.with(|stored| *stored.borrow_mut() = None);
        (result, CANCEL_MATCHES.with(std::cell::Cell::get))
    }
    pub fn measure_largest<T>(action: impl FnOnce() -> T) -> (T, usize) {
        let (result, _) = measure(action);
        (result, LARGEST.with(std::cell::Cell::get))
    }
    pub fn measure<T>(action: impl FnOnce() -> T) -> (T, usize) {
        ALLOCATED.with(|allocated| allocated.set(0));
        LARGEST.with(|largest| largest.set(0));
        ACTIVE.with(|active| active.set(true));
        let result = action();
        ACTIVE.with(|active| active.set(false));
        (result, ALLOCATED.with(std::cell::Cell::get))
    }
}
