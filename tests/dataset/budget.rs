use super::*;

#[test]
fn dataset_budget_and_identity_validation() {
    assert!(Column::numeric(ColumnId(0), "A", vec![]).is_err());
    for name in ["", "  ", "bad\nname", "bad\u{7f}name"] {
        assert!(Column::text(ColumnId(1), name, vec![]).is_err());
    }
    let a = Column::numeric(ColumnId(1), "  Cymraeg â spaces  ", vec![Some(1.0)]).unwrap();
    assert_eq!(a.name(), "  Cymraeg â spaces  ");
    assert!(Dataset::new(vec![a.clone(), a.clone()]).is_err());
    assert!(Dataset::new(vec![
        a.clone(),
        Column::text(ColumnId(2), a.name(), vec![None]).unwrap()
    ])
    .is_err());
    assert!(Dataset::new(vec![a, Column::text(ColumnId(2), "B", vec![]).unwrap()]).is_err());
    let ds = sample_dataset();
    Budget::check_datasets(&[&ds, &ds.clone()], 0).unwrap();
    assert!(Budget::check_datasets(&[&ds], 256 * 1024 * 1024).is_err());
    assert!(Budget::check_datasets(&[&ds], usize::MAX).is_err());
    assert!(Column::numeric(ColumnId(1), "oversized", vec![None; MAX_CELLS + 1]).is_err());
    let big = Column::numeric(ColumnId(1), "Full", vec![None; MAX_CELLS]).unwrap();
    let full = Dataset::new(vec![big]).unwrap();
    assert!(full.append_row().is_err());
    assert!(full
        .append_column(Column::numeric(ColumnId(2), "Other", vec![None; MAX_CELLS]).unwrap())
        .is_err());
}

#[test]
fn metadata_growth_counts_retained_source_before_allocation() {
    let column = Column::text(ColumnId(1), "T", vec![Some("x".repeat(128 * 1024 * 1024))]).unwrap();
    let metadata = ColumnMetadata {
        description: "d".repeat(128 * 1024 * 1024),
        ..column.metadata().clone()
    };
    assert!(matches!(
        column.with_metadata(metadata),
        Err(AppError::SizeLimit { .. })
    ));
    assert_eq!(column.len(), 1);
}

#[test]
fn aggregate_budget_deduplicates_shared_chunks_but_counts_new_metadata() {
    let ds = Dataset::new(vec![Column::text(
        ColumnId(1),
        "T",
        vec![Some("abc".into()), None],
    )
    .unwrap()])
    .unwrap();
    let clone = ds.clone();
    let single = Budget::dataset_bytes(&[&ds], 0).unwrap();
    assert_eq!(Budget::dataset_bytes(&[&ds, &ds], 0).unwrap(), single);
    let shared = Budget::dataset_bytes(&[&ds, &clone], 0).unwrap();
    assert!(shared > single);
    assert!(shared < single * 2);
    assert!(ds
        .column(ColumnId(1))
        .unwrap()
        .convert(ColumnKind::Text)
        .is_ok());
    let altered = ds
        .replace_column(
            ds.column(ColumnId(1))
                .unwrap()
                .with_metadata(ColumnMetadata {
                    description: "Longer description".into(),
                    ..ds.column(ColumnId(1)).unwrap().metadata().clone()
                })
                .unwrap(),
        )
        .unwrap();
    assert!(Budget::dataset_bytes(&[&ds, &altered], 0).unwrap() > shared);
    // Temporary deduplication storage also needs headroom at the owned-byte ceiling.
    Budget::check_datasets(&[&ds], 256 * 1024 * 1024 - single - 1024).unwrap();
    assert!(Budget::check_datasets(&[&ds], 256 * 1024 * 1024 - single).is_err());
    assert!(Budget::check_datasets(&[&ds], 256 * 1024 * 1024 - single + 1).is_err());
}

#[test]
fn budget_refuses_insufficient_bookkeeping_headroom_before_allocation() {
    use tessera::data::MAX_OWNED_BYTES;
    let dataset = Dataset::new(vec![]).unwrap();
    let dataset_bytes = std::mem::size_of::<Dataset>();
    let pointer_bytes = std::mem::size_of::<*const ()>();
    for headroom in [
        0,
        1,
        dataset_bytes - 1,
        dataset_bytes,
        dataset_bytes + pointer_bytes - 1,
    ] {
        let (result, allocated) = budget_allocation_observer::measure(|| {
            Budget::dataset_bytes(&[&dataset], MAX_OWNED_BYTES - headroom)
        });
        assert!(matches!(result, Err(AppError::SizeLimit { .. })));
        assert_eq!(
            allocated, 0,
            "headroom {headroom} allocated {allocated} bytes before refusal"
        );
    }
}

#[test]
fn budget_bookkeeping_fits_reserved_space_and_deduplicates_dataset_aliases() {
    use tessera::data::MAX_OWNED_BYTES;
    let dataset = Dataset::new(vec![]).unwrap();
    let pointer_bytes = std::mem::size_of::<*const ()>();
    let extra = MAX_OWNED_BYTES - std::mem::size_of::<Dataset>() - pointer_bytes;
    let (result, allocated) = budget_allocation_observer::measure(|| {
        Budget::dataset_bytes(&[&dataset, &dataset, &dataset], extra)
    });
    assert_eq!(result.unwrap(), MAX_OWNED_BYTES - pointer_bytes);
    assert!(
        allocated <= pointer_bytes,
        "deduplication allocated {allocated} bytes with {pointer_bytes} reserved"
    );
}

#[test]
fn budget_shared_snapshots_charge_unique_chunks_and_structures_exactly() {
    use tessera::data::CHUNK_SIZE;
    let before = Dataset::new(vec![Column::numeric(
        ColumnId(1),
        "N",
        vec![Some(1.0); CHUNK_SIZE * 2 + 1],
    )
    .unwrap()])
    .unwrap();
    let clone = before.clone();
    let after = before.edit_cell(ColumnId(1), 0, Cell::Number(2.0)).unwrap();
    let single = Budget::dataset_bytes(&[&before], 0).unwrap();
    let clone_structure = std::mem::size_of::<Dataset>()
        + std::mem::size_of::<Column>()
        + 3 * std::mem::size_of::<Arc<[Option<f64>]>>();
    let shared = Budget::dataset_bytes(&[&before, &clone], 0).unwrap();
    assert_eq!(shared, single + clone_structure);
    let changed_chunk =
        2 * std::mem::size_of::<usize>() + CHUNK_SIZE * std::mem::size_of::<Option<f64>>();
    let edited = Budget::dataset_bytes(&[&before, &after], 0).unwrap();
    assert_eq!(edited, shared + changed_chunk);
    assert_eq!(
        Budget::dataset_bytes(&[&before, &before, &after, &after], 0).unwrap(),
        edited
    );
}

#[test]
fn budget_refuses_pointer_vector_reallocation_before_overlap_exceeds_headroom() {
    use tessera::data::MAX_OWNED_BYTES;
    let first = Dataset::new(vec![]).unwrap();
    let second = Dataset::new(vec![]).unwrap();
    let pointer_bytes = std::mem::size_of::<*const ()>();
    let extra = MAX_OWNED_BYTES - 2 * std::mem::size_of::<Dataset>() - 3 * pointer_bytes + 1;
    let (result, allocated) =
        budget_allocation_observer::measure(|| Budget::dataset_bytes(&[&first, &second], extra));
    assert!(matches!(result, Err(AppError::SizeLimit { .. })));
    assert_eq!(
        allocated, 0,
        "pointer vector allocated before complete overlap guard: {allocated} bytes"
    );
}

#[test]
fn budget_aware_edits_preserve_revisions_values_and_metadata() {
    let source = sample_dataset();
    let retained = source.clone();
    let after = source
        .edit_cell_with_budget(
            ColumnId(6),
            0,
            Cell::Number(18.0),
            &[&source, &retained],
            1024,
        )
        .unwrap();
    assert_eq!(after.revision(), RevisionId(2));
    assert_eq!(source.cell(ColumnId(6), 0).unwrap(), Cell::Number(17.0));
    assert_eq!(after.cell(ColumnId(6), 0).unwrap(), Cell::Number(18.0));
    assert_eq!(
        after.column(ColumnId(6)).unwrap().metadata(),
        source.column(ColumnId(6)).unwrap().metadata()
    );
    let appended = after
        .append_row_with_budget(&[&source, &retained], 1024)
        .unwrap();
    assert_eq!(appended.cell(ColumnId(1), 6).unwrap(), Cell::Missing);
    let added = appended
        .append_column_with_budget(
            Column::numeric(ColumnId(7), "New", vec![None; 7]).unwrap(),
            &[&source],
            1024,
        )
        .unwrap();
    assert_eq!(added.column_count(), 7);
    let replacement = Column::numeric(ColumnId(7), "Renamed", vec![Some(3.0); 7]).unwrap();
    let replaced = added
        .replace_column_with_budget(replacement, &[&source, &retained], 1024)
        .unwrap();
    assert_eq!(replaced.cell(ColumnId(7), 0).unwrap(), Cell::Number(3.0));
    let metadata = ColumnMetadata {
        description: "Updated".into(),
        display_decimals: 5,
        ..source.column(ColumnId(6)).unwrap().metadata().clone()
    };
    let updated = source
        .with_column_metadata_with_budget(ColumnId(6), metadata, &[&retained], 1024)
        .unwrap();
    assert_eq!(updated.revision(), RevisionId(2));
    assert_eq!(
        updated.column(ColumnId(6)).unwrap().metadata().description,
        "Updated"
    );
    assert_eq!(
        source.column(ColumnId(6)).unwrap().metadata().description,
        "Sum of the two scale items"
    );
}

#[test]
fn budget_aware_mutations_refuse_before_output_allocation() {
    use tessera::data::MAX_OWNED_BYTES;
    let source = sample_dataset();
    let extra = MAX_OWNED_BYTES - 1;
    let (edit, bytes) = budget_allocation_observer::measure(|| {
        source.edit_cell_with_budget(ColumnId(6), 0, Cell::Number(18.0), &[], extra)
    });
    assert!(matches!(edit, Err(AppError::SizeLimit { .. })));
    assert_eq!(bytes, 0);
    let (row, bytes) =
        budget_allocation_observer::measure(|| source.append_row_with_budget(&[], extra));
    assert!(matches!(row, Err(AppError::SizeLimit { .. })));
    assert_eq!(bytes, 0);
    let column = Column::numeric(ColumnId(7), "New", vec![None; 6]).unwrap();
    let (append, bytes) = budget_allocation_observer::measure(|| {
        source.append_column_with_budget(column, &[], extra)
    });
    assert!(matches!(append, Err(AppError::SizeLimit { .. })));
    assert_eq!(bytes, 0);
    let column = Column::numeric(ColumnId(6), "Total score", vec![None; 6]).unwrap();
    let (replace, bytes) = budget_allocation_observer::measure(|| {
        source.replace_column_with_budget(column, &[], extra)
    });
    assert!(matches!(replace, Err(AppError::SizeLimit { .. })));
    assert_eq!(bytes, 0);
    let metadata = ColumnMetadata {
        description: "Updated".into(),
        ..source.column(ColumnId(6)).unwrap().metadata().clone()
    };
    let (metadata, bytes) = budget_allocation_observer::measure(|| {
        source.with_column_metadata_with_budget(ColumnId(6), metadata, &[], extra)
    });
    assert!(matches!(metadata, Err(AppError::SizeLimit { .. })));
    assert_eq!(bytes, 0);
    assert_eq!(source.revision(), RevisionId(1));
}

#[test]
fn every_budget_aware_edit_counts_retained_work_before_output_growth() {
    use tessera::data::MAX_OWNED_BYTES;
    let source = Dataset::new(vec![Column::numeric(
        ColumnId(1),
        "N",
        vec![Some(1.0); 10_000],
    )
    .unwrap()])
    .unwrap();
    let retained = Dataset::new(vec![Column::text(
        ColumnId(2),
        "Retained",
        vec![Some("x".repeat(1024 * 1024))],
    )
    .unwrap()])
    .unwrap();
    let extra = MAX_OWNED_BYTES - Budget::dataset_bytes(&[&source], 0).unwrap() - 256 * 1024;
    let (edit, bytes) = budget_allocation_observer::measure(|| {
        source.edit_cell_with_budget(ColumnId(1), 0, Cell::Number(2.0), &[&retained], extra)
    });
    assert!(matches!(edit, Err(AppError::SizeLimit { .. })));
    assert!(bytes < 4096);
    let (row, bytes) =
        budget_allocation_observer::measure(|| source.append_row_with_budget(&[&retained], extra));
    assert!(matches!(row, Err(AppError::SizeLimit { .. })));
    assert!(bytes < 4096);
    let added = Column::numeric(ColumnId(3), "Added", vec![None; 10_000]).unwrap();
    let (column, bytes) = budget_allocation_observer::measure(|| {
        source.append_column_with_budget(added, &[&retained], extra)
    });
    assert!(matches!(column, Err(AppError::SizeLimit { .. })));
    assert!(bytes < 4096);
    let replacement = Column::numeric(ColumnId(1), "N", vec![Some(2.0); 10_000]).unwrap();
    let (replaced, bytes) = budget_allocation_observer::measure(|| {
        source.replace_column_with_budget(replacement, &[&retained], extra)
    });
    assert!(matches!(replaced, Err(AppError::SizeLimit { .. })));
    assert!(bytes < 4096);
    let metadata = ColumnMetadata {
        description: "Updated".into(),
        ..source.column(ColumnId(1)).unwrap().metadata().clone()
    };
    let (updated, bytes) = budget_allocation_observer::measure(|| {
        source.with_column_metadata_with_budget(ColumnId(1), metadata, &[&retained], extra)
    });
    assert!(matches!(updated, Err(AppError::SizeLimit { .. })));
    assert!(bytes < 4096);
    // A cell edit fits the same headroom when the separately retained dataset is absent.
    assert!(source
        .edit_cell_with_budget(ColumnId(1), 0, Cell::Number(2.0), &[], extra)
        .is_ok());
    assert_eq!(source.revision(), RevisionId(1));
}

#[test]
fn empty_column_boundary_refuses_growth_before_constructor_allocation() {
    use tessera::data::{MAX_CELLS, MAX_OWNED_BYTES};
    let full = Dataset::new(vec![Column::numeric(
        ColumnId(1),
        "Full",
        vec![None; MAX_CELLS],
    )
    .unwrap()])
    .unwrap();
    let (result, allocated) = budget_allocation_observer::measure(|| {
        full.append_empty_column_with_budget("New", ColumnKind::Number, &[], 0)
    });
    assert!(matches!(result, Err(AppError::SizeLimit { .. })));
    assert_eq!(allocated, 0);
    let source = sample_dataset();
    for kind in [ColumnKind::Number, ColumnKind::Text] {
        let (result, allocated) = budget_allocation_observer::measure(|| {
            source.append_empty_column_with_budget("New", kind, &[], MAX_OWNED_BYTES - 1)
        });
        assert!(matches!(result, Err(AppError::SizeLimit { .. })));
        assert_eq!(allocated, 0);
    }
}

#[test]
fn empty_columns_get_next_stable_id_and_current_missing_rows() {
    let source = sample_dataset();
    for kind in [ColumnKind::Number, ColumnKind::Text] {
        let result = source
            .append_empty_column_with_budget("New â", kind, &[&source], 0)
            .unwrap();
        assert_eq!(result.revision(), RevisionId(2));
        assert_eq!(result.column(ColumnId(7)).unwrap().kind(), kind);
        assert_eq!(result.column(ColumnId(7)).unwrap().name(), "New â");
        for row in 0..6 {
            assert_eq!(result.cell(ColumnId(7), row).unwrap(), Cell::Missing);
        }
    }
    assert_eq!(source.column_count(), 6);
    assert_eq!(source.revision(), RevisionId(1));
    assert!(source
        .append_empty_column_with_budget("Age", ColumnKind::Text, &[], 0)
        .is_err());
    assert!(source
        .append_empty_column_with_budget("  ", ColumnKind::Text, &[], 0)
        .is_err());
    let empty = Dataset::new(vec![])
        .unwrap()
        .append_empty_column_with_budget("First", ColumnKind::Number, &[], 0)
        .unwrap();
    assert_eq!(empty.column(ColumnId(1)).unwrap().len(), 0);
    let exhausted = Dataset::new(vec![
        Column::numeric(ColumnId(u64::MAX), "Last", vec![]).unwrap()
    ])
    .unwrap();
    assert!(matches!(
        exhausted.append_empty_column_with_budget("New", ColumnKind::Number, &[], 0),
        Err(AppError::IdentityExhausted)
    ));
}

#[test]
fn complete_retained_baseline_precedes_any_pointer_table_growth() {
    use tessera::data::MAX_OWNED_BYTES;
    let seed = Dataset::new(vec![
        Column::numeric(ColumnId(1), "Small", vec![Some(1.0)]).unwrap()
    ])
    .unwrap();
    let small: Vec<_> = (0..2048).map(|_| seed.clone()).collect();
    let large = Dataset::new(vec![Column::numeric(
        ColumnId(2),
        "Large",
        vec![None; 2_000_000],
    )
    .unwrap()])
    .unwrap();
    let references: Vec<_> = small.iter().chain(std::iter::once(&large)).collect();
    let owned = Budget::dataset_bytes(&references, 0).unwrap();
    let extra = MAX_OWNED_BYTES - owned - 1024;
    let (result, largest) =
        budget_allocation_observer::measure_largest(|| Budget::dataset_bytes(&references, extra));
    assert!(matches!(result, Err(AppError::SizeLimit { .. })));
    assert_eq!(
        largest, 0,
        "pointer table allocated before complete baseline guard: {largest}"
    );
}

#[test]
fn exact_shared_snapshot_fallback_accepts_work_that_duplicate_upper_bound_rejects() {
    use tessera::data::MAX_OWNED_BYTES;
    let source = Dataset::new(vec![Column::numeric(
        ColumnId(1),
        "Shared",
        vec![None; 2_000_000],
    )
    .unwrap()])
    .unwrap();
    let clone = source.clone();
    let references = [&source, &clone];
    let owned = Budget::dataset_bytes(&references, 0).unwrap();
    let extra = MAX_OWNED_BYTES - owned - 16 * 1024;
    let (result, largest) =
        budget_allocation_observer::measure_largest(|| Budget::dataset_bytes(&references, extra));
    assert_eq!(result.unwrap(), MAX_OWNED_BYTES - 16 * 1024);
    assert!(largest <= 8192, "unexpected accounting buffer: {largest}");
    assert_eq!(
        Budget::dataset_bytes(&[&source, &source, &clone, &clone], 0).unwrap(),
        owned
    );
}
