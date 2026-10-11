use tessera::data::{Cell, Column, ColumnId, Dataset, RevisionId, MAX_OWNED_BYTES};
fn data() -> Dataset {
    Dataset::new(vec![
        Column::numeric(ColumnId(1), "Number", vec![Some(1.0); 3]).unwrap(),
        Column::text(ColumnId(2), "Text", vec![None; 3]).unwrap(),
    ])
    .unwrap()
}
#[test]
fn paste_is_typed_atomic_and_has_one_revision() {
    let old = data();
    let next = old
        .paste_tsv_with_budget(ColumnId(1), 0, "2\thello\r\n\t", &[&old], 0)
        .unwrap();
    assert_eq!(next.revision(), RevisionId(2));
    assert_eq!(next.cell(ColumnId(1), 0).unwrap(), Cell::Number(2.0));
    assert_eq!(next.cell(ColumnId(1), 1).unwrap(), Cell::Missing);
    assert_eq!(
        next.cell(ColumnId(2), 1).unwrap(),
        Cell::Text(String::new())
    );
    assert_eq!(old.cell(ColumnId(1), 0).unwrap(), Cell::Number(1.0));
    for text in ["2\tok\ninvalid\tbad", "2\tok\n3", "2\tok\textra"] {
        assert!(old
            .paste_tsv_with_budget(ColumnId(1), 0, text, &[], 0)
            .is_err());
        assert_eq!(old.revision(), RevisionId(1));
    }
}
#[test]
fn quoted_multiline_fields_reuse_import_rules() {
    let next = data()
        .paste_tsv_with_budget(ColumnId(2), 0, "\"a\t\"\"b\"\"\r\nc\"\nplain", &[], 0)
        .unwrap();
    assert_eq!(
        next.cell(ColumnId(2), 0).unwrap(),
        Cell::Text("a\t\"b\"\r\nc".into())
    );
}
#[test]
fn paste_limits_and_identity_fail_without_changes() {
    let old = data();
    assert!(old
        .paste_tsv_with_budget(ColumnId(1), 2, "2\n3", &[], 0)
        .is_err());
    assert!(old
        .paste_tsv_with_budget(ColumnId(2), 0, &"x".repeat(512 * 1024 + 1), &[], 0)
        .is_err());
    assert!(old
        .paste_tsv_with_budget(ColumnId(1), 0, "2", &[], MAX_OWNED_BYTES)
        .is_err());
    let exhausted = Dataset::from_revision(old.columns().to_vec(), RevisionId(u64::MAX)).unwrap();
    assert!(exhausted
        .paste_tsv_with_budget(ColumnId(1), 0, "2", &[], 0)
        .is_err());
    let many = Dataset::new(vec![
        Column::numeric(ColumnId(1), "N", vec![None; 4097]).unwrap()
    ])
    .unwrap();
    assert!(many
        .paste_tsv_with_budget(ColumnId(1), 0, &vec!["1"; 4097].join("\n"), &[], 0)
        .is_err());
}

#[test]
fn malformed_quotes_and_nonfinite_numbers_leave_source_unchanged() {
    let old = data();
    for text in [
        "2\t\"unclosed",
        "2\tbare\"quote",
        "2\t\"closed\"extra",
        "NaN\tname",
        "inf\tname",
    ] {
        assert!(
            old.paste_tsv_with_budget(ColumnId(1), 0, text, &[], 0)
                .is_err(),
            "accepted {text:?}"
        );
        assert_eq!(old.revision(), RevisionId(1));
        assert_eq!(old.cell(ColumnId(1), 0).unwrap(), Cell::Number(1.0));
        assert_eq!(old.cell(ColumnId(2), 0).unwrap(), Cell::Missing);
    }
}

#[test]
fn exact_cell_limit_is_accepted_in_one_revision() {
    let old = Dataset::new(vec![
        Column::numeric(ColumnId(1), "N", vec![None; 4096]).unwrap()
    ])
    .unwrap();
    let next = old
        .paste_tsv_with_budget(ColumnId(1), 0, &vec!["1"; 4096].join("\n"), &[&old], 0)
        .unwrap();
    assert_eq!(next.revision(), RevisionId(2));
    assert_eq!(next.cell(ColumnId(1), 4095).unwrap(), Cell::Number(1.0));
    assert_eq!(old.cell(ColumnId(1), 4095).unwrap(), Cell::Missing);
}
