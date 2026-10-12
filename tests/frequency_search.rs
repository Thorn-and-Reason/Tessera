use tessera::{
    analysis::*,
    data::*,
    error::AppError,
    operation::{JobControl, RunId},
};
fn result() -> ComputedFrequencies {
    let column = Column::text(
        ColumnId(1),
        "T",
        (0..600)
            .map(|i| Some(format!("value{i:03}")))
            .chain([Some("ΟΣ ŵ".into())])
            .collect(),
    )
    .unwrap();
    let column = column
        .with_metadata(ColumnMetadata {
            value_labels: vec![
                ValueLabel {
                    value: Cell::Text("value599".into()),
                    label: "Shared ΕΛΛΗΝΙΚΆ".into(),
                },
                ValueLabel {
                    value: Cell::Text("value001".into()),
                    label: "Shared".into(),
                },
            ],
            ..column.metadata().clone()
        })
        .unwrap();
    let d = Dataset::new(vec![column]).unwrap();
    let r = FrequenciesRequest {
        group: None,
        columns: vec![ColumnId(1)],
        filter: RowFilter::default(),
        frequency_weight: None,
    };
    frequencies_with_budget(&d, &r, &JobControl::default(), &[], r.heap_bytes().unwrap()).unwrap()
}
fn search(result: &ComputedFrequencies, query: &str, offset: usize) -> FrequencySearchResult {
    let r = FrequencySearchRequest {
        run: RunId(1),
        column: ColumnId(1),
        query: query.into(),
        offset,
    };
    frequency_rows_with_budget(
        result,
        &r,
        &JobControl::default(),
        result.owned_bytes().unwrap() + r.heap_bytes().unwrap(),
    )
    .unwrap()
}
#[test]
fn complete_match_count_and_matching_ordinal_paging_beyond_256() {
    let result = result();
    let first = search(&result, "", 0);
    assert_eq!(
        (first.indices.len(), first.matched, first.next_offset),
        (256, 601, Some(256))
    );
    let second = search(&result, "", 256);
    assert_eq!(
        (second.indices.len(), second.matched, second.next_offset),
        (256, 601, Some(512))
    );
    assert_eq!(second.indices[0], 256);
    let last = search(&result, "", 512);
    assert_eq!(
        (last.indices.len(), last.matched, last.next_offset),
        (89, 601, None)
    );
    assert_eq!(search(&result, "value599", 0).indices, vec![599]);
    assert_eq!(
        search(&result, "value", 590).indices,
        (590..600).collect::<Vec<_>>()
    );
    assert!(search(&result, "absent", 0).indices.is_empty());
}
#[test]
fn unicode_exact_values_and_duplicate_captured_labels_remain_distinct() {
    let result = result();
    assert_eq!(search(&result, "ΟΣ Ŵ", 0).indices, vec![600]);
    assert_eq!(search(&result, "ελληνικά", 0).indices, vec![599]);
    let labels = search(&result, "SHARED", 0);
    assert_eq!(
        (labels.indices, labels.matched, labels.next_offset),
        (vec![1, 599], 2, None)
    );
    assert_eq!(search(&result, "SHARED", 1).indices, vec![599]);
    assert_eq!(search(&result, "SHARED", 1).request.query, "SHARED");
}
#[test]
fn invalid_inputs_budget_refusal_and_pre_cancelled_search_preserve_result() {
    let result = result();
    let before = result.clone();
    let valid = FrequencySearchRequest {
        run: RunId(1),
        column: ColumnId(1),
        query: String::new(),
        offset: 0,
    };
    for request in [
        FrequencySearchRequest {
            run: RunId(0),
            ..valid.clone()
        },
        FrequencySearchRequest {
            column: ColumnId(0),
            ..valid.clone()
        },
        FrequencySearchRequest {
            column: ColumnId(2),
            ..valid.clone()
        },
        FrequencySearchRequest {
            query: "q".repeat(8193),
            ..valid.clone()
        },
        FrequencySearchRequest {
            offset: usize::MAX,
            ..valid.clone()
        },
    ] {
        assert!(frequency_rows_with_budget(
            &result,
            &request,
            &JobControl::default(),
            result.owned_bytes().unwrap()
        )
        .is_err());
    }
    assert!(matches!(
        frequency_rows_with_budget(
            &result,
            &valid,
            &JobControl::default(),
            MAX_OWNED_BYTES - 100
        ),
        Err(AppError::SizeLimit { .. })
    ));
    let control = JobControl::default();
    control.cancel();
    assert!(matches!(
        frequency_rows_with_budget(&result, &valid, &control, result.owned_bytes().unwrap()),
        Err(AppError::Cancelled)
    ));
    assert_eq!(result, before);
    let mut malformed = result.clone();
    malformed.tables[0].categories[0].value_label_index = Some(usize::MAX);
    assert!(frequency_rows_with_budget(
        &malformed,
        &valid,
        &JobControl::default(),
        malformed.owned_bytes().unwrap()
    )
    .is_err());
}
#[test]
fn numeric_text_form_and_long_label_inner_cancellation() {
    let column = Column::numeric(
        ColumnId(1),
        "N",
        vec![Some(f64::MAX), Some(f64::from_bits(1)), Some(-0.)],
    )
    .unwrap();
    let column = column
        .with_metadata(ColumnMetadata {
            value_labels: vec![ValueLabel {
                value: Cell::Number(0.),
                label: "a".repeat(2_000_000),
            }],
            ..column.metadata().clone()
        })
        .unwrap();
    let d = Dataset::new(vec![column]).unwrap();
    let r = FrequenciesRequest {
        group: None,
        columns: vec![ColumnId(1)],
        filter: RowFilter::default(),
        frequency_weight: None,
    };
    let result =
        frequencies_with_budget(&d, &r, &JobControl::default(), &[], r.heap_bytes().unwrap())
            .unwrap();
    assert_eq!(search(&result, &f64::MAX.to_string(), 0).indices, vec![2]);
    assert_eq!(
        search(&result, &f64::from_bits(1).to_string(), 0).indices,
        vec![1]
    );
    let request = FrequencySearchRequest {
        run: RunId(1),
        column: ColumnId(1),
        query: "aaaaab".into(),
        offset: 0,
    };
    let (sender, receiver) = std::sync::mpsc::channel();
    let control = JobControl::new(
        std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false)),
        Some(sender),
    );
    let cancel = control.clone();
    let canceller = std::thread::spawn(move || {
        receiver.recv().unwrap();
        std::thread::sleep(std::time::Duration::from_millis(1));
        cancel.cancel();
    });
    assert!(matches!(
        frequency_rows_with_budget(
            &result,
            &request,
            &control,
            result.owned_bytes().unwrap() + request.heap_bytes().unwrap()
        ),
        Err(AppError::Cancelled)
    ));
    canceller.join().unwrap();
}
