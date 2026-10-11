use super::*;

#[test]
fn conversion_preserves_precision_null_and_empty_text() {
    let number = Column::numeric(
        ColumnId(1),
        "N",
        vec![Some(1.2345678901234567), Some(-0.0), None],
    )
    .unwrap();
    let text = number.convert(ColumnKind::Text).unwrap();
    let back = text.convert(ColumnKind::Number).unwrap();
    assert_eq!(back.cell(0).unwrap(), Cell::Number(1.2345678901234567));
    assert_eq!(back.cell(1).unwrap(), Cell::Number(-0.0));
    assert_eq!(back.cell(2).unwrap(), Cell::Missing);
    let ds = Dataset::new(vec![Column::text(
        ColumnId(1),
        "T",
        vec![Some("".into()), None, Some("oops".into())],
    )
    .unwrap()])
    .unwrap();
    assert_eq!(ds.cell(ColumnId(1), 0).unwrap(), Cell::Text("".into()));
    assert_eq!(ds.cell(ColumnId(1), 1).unwrap(), Cell::Missing);
    assert!(ds
        .column(ColumnId(1))
        .unwrap()
        .convert(ColumnKind::Number)
        .is_err());
    assert_eq!(ds.revision(), RevisionId(1));
    let metadata = ColumnMetadata {
        display_decimals: 2,
        ..number.metadata().clone()
    };
    let changed = number.with_metadata(metadata).unwrap();
    assert_eq!(changed.cell(0).unwrap(), Cell::Number(1.2345678901234567));
    let original = Dataset::new(vec![number]).unwrap();
    assert_eq!(
        original.replace_column(changed).unwrap().revision(),
        RevisionId(2)
    );
}

#[test]
fn conversion_preserves_neutral_metadata_or_rejects_incompatible_ranges() {
    let number = Column::numeric(ColumnId(1), "N", vec![Some(-99.0), Some(5.0)]).unwrap();
    let number = number
        .with_metadata(ColumnMetadata {
            missing_rules: MissingRules::Number {
                values: vec![-99.0],
                ranges: vec![],
            },
            value_labels: vec![ValueLabel {
                value: Cell::Number(5.0),
                label: "Five".into(),
            }],
            ..number.metadata().clone()
        })
        .unwrap();
    let text = number.convert(ColumnKind::Text).unwrap();
    assert!(text.is_missing(0).unwrap());
    assert_eq!(
        text.metadata().value_labels[0].value,
        Cell::Text("5".into())
    );
    let back = text.convert(ColumnKind::Number).unwrap();
    assert_eq!(
        back.metadata().missing_rules,
        number.metadata().missing_rules
    );
    let ranged = number
        .with_metadata(ColumnMetadata {
            missing_rules: MissingRules::Number {
                values: vec![],
                ranges: vec![(1.0, 9.0)],
            },
            ..number.metadata().clone()
        })
        .unwrap();
    assert!(ranged.convert(ColumnKind::Text).is_err());
}

#[test]
fn conversions_refuse_signed_zero_missingness_changes_in_both_directions() {
    for (number, token) in [(-0.0, 0.0), (0.0, -0.0)] {
        let column = Column::numeric(ColumnId(1), "N", vec![Some(number)]).unwrap();
        let column = column
            .with_metadata(ColumnMetadata {
                missing_rules: MissingRules::Number {
                    values: vec![token],
                    ranges: vec![],
                },
                ..column.metadata().clone()
            })
            .unwrap();
        let ds = Dataset::new(vec![column]).unwrap();
        assert!(ds.column(ColumnId(1)).unwrap().is_missing(0).unwrap());
        assert!(ds
            .column(ColumnId(1))
            .unwrap()
            .convert(ColumnKind::Text)
            .is_err());
        assert_eq!(ds.revision(), RevisionId(1));
    }
    for (observed, token) in [("-0", "0"), ("0", "-0")] {
        let column = Column::text(ColumnId(1), "T", vec![Some(observed.into())]).unwrap();
        let column = column
            .with_metadata(ColumnMetadata {
                missing_rules: MissingRules::Text(vec![token.into()]),
                ..column.metadata().clone()
            })
            .unwrap();
        assert!(!column.is_missing(0).unwrap());
        assert!(column.convert(ColumnKind::Number).is_err());
    }
}

#[test]
fn conversion_refuses_text_missing_token_and_label_collisions() {
    let column = Column::text(ColumnId(1), "T", vec![Some("01".into()), Some("1".into())]).unwrap();
    let marked = column
        .with_metadata(ColumnMetadata {
            missing_rules: MissingRules::Text(vec!["01".into()]),
            ..column.metadata().clone()
        })
        .unwrap();
    assert!(marked.is_missing(0).unwrap());
    assert!(!marked.is_missing(1).unwrap());
    assert!(marked.convert(ColumnKind::Number).is_err());
    let labelled = column
        .with_metadata(ColumnMetadata {
            value_labels: vec![ValueLabel {
                value: Cell::Text("01".into()),
                label: "First".into(),
            }],
            ..column.metadata().clone()
        })
        .unwrap();
    assert!(labelled.convert(ColumnKind::Number).is_err());
    let number = Column::numeric(ColumnId(2), "N", vec![Some(-0.0)]).unwrap();
    let labelled = number
        .with_metadata(ColumnMetadata {
            value_labels: vec![ValueLabel {
                value: Cell::Number(0.0),
                label: "Zero".into(),
            }],
            ..number.metadata().clone()
        })
        .unwrap();
    assert!(labelled.convert(ColumnKind::Text).is_err());
}

#[test]
fn numeric_conversion_refuses_lossy_integer_lexemes_with_cell_location() {
    for text in [
        "9007199254740993",
        "-9007199254740993",
        "9007199254740993.0",
        "-9007199254740993.0",
        "9.007199254740993e15",
        "-9.007199254740993e15",
        "1e23",
    ] {
        let ds = Dataset::new(vec![Column::text(
            ColumnId(4),
            "Identifier",
            vec![Some("1".into()), Some(text.into())],
        )
        .unwrap()])
        .unwrap();
        let error = ds
            .column(ColumnId(4))
            .unwrap()
            .convert(ColumnKind::Number)
            .unwrap_err();
        assert!(
            matches!(error, AppError::InvalidInput { ref location, .. } if location.contains("column 4") && location.contains("row 2")),
            "{error}"
        );
        assert_eq!(ds.revision(), RevisionId(1));
        assert_eq!(ds.cell(ColumnId(4), 1).unwrap(), Cell::Text(text.into()));
    }
    for (text, expected) in [
        ("9007199254740992", 9007199254740992.0),
        ("-9007199254740992", -9007199254740992.0),
        ("9007199254740994", 9007199254740994.0),
        ("-9007199254740994", -9007199254740994.0),
        ("0.125", 0.125),
        ("12.50", 12.5),
        (
            "3.40282366920938463463374607431768211456e38",
            340282366920938463463374607431768211456.0,
        ),
    ] {
        let column = Column::text(ColumnId(1), "T", vec![Some(text.into())]).unwrap();
        assert_eq!(
            column.convert(ColumnKind::Number).unwrap().cell(0).unwrap(),
            Cell::Number(expected)
        );
    }
}

#[test]
fn conversion_counts_retained_snapshots_before_planning_or_building() {
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
    let owned = Budget::dataset_bytes(&[&source, &retained], 0).unwrap();
    let extra = MAX_OWNED_BYTES - owned - 8192;
    let (result, allocated) = budget_allocation_observer::measure(|| {
        source.convert_column_with_budget(
            ColumnId(1),
            ColumnKind::Text,
            &JobControl::default(),
            &[&retained],
            extra,
        )
    });
    assert!(matches!(result, Err(AppError::SizeLimit { .. })));
    assert!(
        allocated < 4096,
        "allocated conversion work before retained guard: {allocated}"
    );
    assert_eq!(source.revision(), RevisionId(1));
    assert_eq!(
        source.column(ColumnId(1)).unwrap().kind(),
        ColumnKind::Number
    );
}

#[test]
fn controlled_conversion_preserves_metadata_missingness_and_noop_revision() {
    let column = Column::numeric(ColumnId(1), "N", vec![Some(1.0), Some(-99.0), None]).unwrap();
    let column = column
        .with_metadata(ColumnMetadata {
            missing_rules: MissingRules::Number {
                values: vec![-99.0],
                ranges: vec![],
            },
            value_labels: vec![ValueLabel {
                value: Cell::Number(1.0),
                label: "One".into(),
            }],
            description: "Meaning".into(),
            ..column.metadata().clone()
        })
        .unwrap();
    let source = Dataset::new(vec![column]).unwrap();
    let converted = source
        .convert_column_with_budget(
            ColumnId(1),
            ColumnKind::Text,
            &JobControl::default(),
            &[&source],
            0,
        )
        .unwrap();
    assert_eq!(converted.revision(), RevisionId(2));
    assert_eq!(
        converted
            .column(ColumnId(1))
            .unwrap()
            .metadata()
            .description,
        "Meaning"
    );
    assert_eq!(
        converted
            .column(ColumnId(1))
            .unwrap()
            .metadata()
            .value_labels[0]
            .value,
        Cell::Text("1".into())
    );
    for row in 0..3 {
        assert_eq!(
            source.column(ColumnId(1)).unwrap().is_missing(row).unwrap(),
            converted
                .column(ColumnId(1))
                .unwrap()
                .is_missing(row)
                .unwrap()
        );
    }
    let noop = source
        .convert_column_with_budget(
            ColumnId(1),
            ColumnKind::Number,
            &JobControl::default(),
            &[],
            0,
        )
        .unwrap();
    assert_eq!(noop.revision(), source.revision());
    let control = JobControl::default();
    control.cancel();
    assert!(matches!(
        source.convert_column_with_budget(ColumnId(1), ColumnKind::Text, &control, &[], 0),
        Err(AppError::Cancelled)
    ));
    assert_eq!(source.revision(), RevisionId(1));
}

#[test]
fn conversion_cancels_during_first_chunk_without_publishing_partial_revision() {
    use tessera::data::CHUNK_SIZE;
    let source = Dataset::new(vec![Column::numeric(
        ColumnId(1),
        "N",
        vec![Some(1.0); CHUNK_SIZE * 3],
    )
    .unwrap()])
    .unwrap();
    let flag = Arc::new(AtomicBool::new(false));
    let control = JobControl::new(Arc::clone(&flag), None);
    let (result, matched) = budget_allocation_observer::cancel_on_allocation(
        CHUNK_SIZE * std::mem::size_of::<Option<String>>(),
        flag,
        || source.convert_column_with_budget(ColumnId(1), ColumnKind::Text, &control, &[], 0),
    );
    assert!(matches!(result, Err(AppError::Cancelled)));
    assert_eq!(
        matched, 1,
        "conversion built another chunk after cancellation"
    );
    assert_eq!(source.revision(), RevisionId(1));
    assert_eq!(
        source.cell(ColumnId(1), CHUNK_SIZE * 3 - 1).unwrap(),
        Cell::Number(1.0)
    );
}

#[test]
fn controlled_conversion_reports_monotonic_planning_build_and_parity_progress() {
    let source = Dataset::new(vec![Column::numeric(
        ColumnId(1),
        "N",
        vec![Some(1.0); 512],
    )
    .unwrap()])
    .unwrap();
    let (sender, receiver) = mpsc::channel();
    let control = JobControl::new(Arc::new(AtomicBool::new(false)), Some(sender));
    source
        .convert_column_with_budget(ColumnId(1), ColumnKind::Text, &control, &[], 0)
        .unwrap();
    let progress: Vec<_> = receiver.try_iter().collect();
    assert!(progress.windows(2).all(|pair| pair[0].done <= pair[1].done));
    assert!(progress.iter().all(|progress| progress.total == Some(1536)));
    assert_eq!(progress.last().unwrap().done, 1536);
    assert!(progress.iter().any(|progress| progress.done == 256));
    assert!(progress.iter().any(|progress| progress.done == 768));
    assert!(progress.iter().any(|progress| progress.done == 1280));
}

#[test]
fn missing_token_vectors_match_planned_non_power_of_two_lengths() {
    for count in [19, 1003] {
        let source = dataset_with_missing_numbers(count);
        let converted = source
            .convert_column_with_budget(
                ColumnId(1),
                ColumnKind::Text,
                &JobControl::default(),
                &[],
                0,
            )
            .unwrap();
        let MissingRules::Text(tokens) = &converted
            .column(ColumnId(1))
            .unwrap()
            .metadata()
            .missing_rules
        else {
            panic!("text rules expected")
        };
        assert_eq!(tokens.len(), count);
        assert_eq!(tokens.capacity(), count);
        let numeric = converted
            .convert_column_with_budget(
                ColumnId(1),
                ColumnKind::Number,
                &JobControl::default(),
                &[],
                0,
            )
            .unwrap();
        let MissingRules::Number { values, .. } = &numeric
            .column(ColumnId(1))
            .unwrap()
            .metadata()
            .missing_rules
        else {
            panic!("numeric rules expected")
        };
        assert_eq!(values.capacity(), count);
        assert_eq!(values[0], 0.0);
        assert_eq!(values[count - 1], (count - 1) as f64);
    }
}

#[test]
fn value_label_vectors_match_planned_non_power_of_two_lengths() {
    for count in [23, 1003] {
        let column = Column::numeric(ColumnId(1), "Labels", vec![]).unwrap();
        let column = column
            .with_metadata(ColumnMetadata {
                value_labels: (0..count)
                    .map(|value| ValueLabel {
                        value: Cell::Number(f64::from(value)),
                        label: "label".into(),
                    })
                    .collect(),
                ..column.metadata().clone()
            })
            .unwrap();
        let source = Dataset::new(vec![column]).unwrap();
        let converted = source
            .convert_column_with_budget(
                ColumnId(1),
                ColumnKind::Text,
                &JobControl::default(),
                &[],
                0,
            )
            .unwrap();
        let labels = &converted
            .column(ColumnId(1))
            .unwrap()
            .metadata()
            .value_labels;
        assert_eq!(labels.capacity(), count as usize);
        assert_eq!(labels[0].value, Cell::Text("0".into()));
        assert_eq!(labels[0].label, "label");
        let numeric = converted
            .convert_column_with_budget(
                ColumnId(1),
                ColumnKind::Number,
                &JobControl::default(),
                &[],
                0,
            )
            .unwrap();
        assert_eq!(
            numeric
                .column(ColumnId(1))
                .unwrap()
                .metadata()
                .value_labels
                .capacity(),
            count as usize
        );
        assert_eq!(
            numeric.column(ColumnId(1)).unwrap().metadata().value_labels,
            source.column(ColumnId(1)).unwrap().metadata().value_labels
        );
    }
}

#[test]
fn exact_token_vectors_fit_tight_retained_budget_without_reallocation() {
    use tessera::data::MAX_OWNED_BYTES;
    let count = 100_000;
    let source = dataset_with_missing_numbers(count);
    let extra = MAX_OWNED_BYTES - Budget::dataset_bytes(&[&source], 0).unwrap() - 3_500_000;
    let control = JobControl::default();
    let (converted, largest) = budget_allocation_observer::measure_largest(|| {
        source.convert_column_with_budget(ColumnId(1), ColumnKind::Text, &control, &[], extra)
    });
    let converted = converted.unwrap();
    assert!(
        largest <= count * std::mem::size_of::<String>(),
        "unplanned metadata vector allocation: {largest}"
    );
    let MissingRules::Text(tokens) = &converted
        .column(ColumnId(1))
        .unwrap()
        .metadata()
        .missing_rules
    else {
        panic!("text rules expected")
    };
    assert_eq!(tokens.capacity(), count);
    Budget::check_datasets(&[&source, &converted], extra).unwrap();
    assert_eq!(source.revision(), RevisionId(1));
    assert_eq!(converted.revision(), RevisionId(2));
}

#[test]
fn token_storage_refuses_before_vector_when_exact_plan_cannot_fit() {
    use tessera::data::MAX_OWNED_BYTES;
    let source = dataset_with_missing_numbers(100_000);
    let extra = MAX_OWNED_BYTES - Budget::dataset_bytes(&[&source], 0).unwrap() - 3_100_000;
    let control = JobControl::default();
    let (converted, largest) = budget_allocation_observer::measure_largest(|| {
        source.convert_column_with_budget(ColumnId(1), ColumnKind::Text, &control, &[], extra)
    });
    assert!(matches!(converted, Err(AppError::SizeLimit { .. })));
    assert!(
        largest < 4096,
        "allocated metadata before exact plan refusal: {largest}"
    );
    assert_eq!(source.revision(), RevisionId(1));
}

fn dataset_with_missing_numbers(count: usize) -> Dataset {
    let column = Column::numeric(ColumnId(1), "Tokens", vec![]).unwrap();
    let column = column
        .with_metadata(ColumnMetadata {
            missing_rules: MissingRules::Number {
                values: (0..count).map(|value| value as f64).collect(),
                ranges: vec![],
            },
            ..column.metadata().clone()
        })
        .unwrap();
    Dataset::new(vec![column]).unwrap()
}

#[test]
fn exact_label_vector_fits_tight_budget_without_capacity_growth() {
    use tessera::data::MAX_OWNED_BYTES;
    let count = 1003;
    let column = Column::numeric(ColumnId(1), "Labels", vec![]).unwrap();
    let column = column
        .with_metadata(ColumnMetadata {
            value_labels: (0..count)
                .map(|value| ValueLabel {
                    value: Cell::Number(value as f64),
                    label: "label".into(),
                })
                .collect(),
            ..column.metadata().clone()
        })
        .unwrap();
    let source = Dataset::new(vec![column]).unwrap();
    let extra = MAX_OWNED_BYTES - Budget::dataset_bytes(&[&source], 0).unwrap() - 200_000;
    let control = JobControl::default();
    let (converted, largest) = budget_allocation_observer::measure_largest(|| {
        source.convert_column_with_budget(ColumnId(1), ColumnKind::Text, &control, &[], extra)
    });
    let converted = converted.unwrap();
    assert!(
        largest <= count * std::mem::size_of::<ValueLabel>(),
        "unplanned label-vector allocation: {largest}"
    );
    assert_eq!(
        converted
            .column(ColumnId(1))
            .unwrap()
            .metadata()
            .value_labels
            .capacity(),
        count
    );
    Budget::check_datasets(&[&source, &converted], extra).unwrap();
    assert_eq!(source.revision(), RevisionId(1));
}
