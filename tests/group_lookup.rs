use tessera::{analysis::*, data::*, operation::JobControl};
fn text(values: Vec<Option<String>>) -> Dataset {
    Dataset::new(vec![Column::text(ColumnId(1), "group", values).unwrap()]).unwrap()
}
fn request(query: &str) -> GroupLookupRequest {
    GroupLookupRequest {
        column: ColumnId(1),
        role_epoch: 7,
        query: query.into(),
        selected: [None, None],
    }
}
fn lookup(d: &Dataset, r: &GroupLookupRequest) -> GroupLookupResult {
    group_values_with_budget(d, r, &JobControl::default(), &[], r.heap_bytes().unwrap()).unwrap()
}
#[test]
fn search_finds_category_beyond_first_page() {
    let d = text((0..300).map(|i| Some(format!("category {i}"))).collect());
    assert!(lookup(&d, &request("")).more);
    let result = lookup(&d, &request("category 299"));
    assert_eq!(result.choices.len(), 1);
    assert_eq!(result.choices[0].value, Cell::Text("category 299".into()));
}
#[test]
fn discovery_ignores_filter_weight_and_outcome() {
    let d = Dataset::new(vec![
        Column::text(
            ColumnId(1),
            "group",
            vec![Some("a".into()), Some("b".into())],
        )
        .unwrap(),
        Column::numeric(ColumnId(2), "weight", vec![Some(-1.), None]).unwrap(),
        Column::numeric(ColumnId(3), "outcome", vec![None, None]).unwrap(),
    ])
    .unwrap();
    assert_eq!(lookup(&d, &request("")).choices.len(), 2);
}
#[test]
fn labels_do_not_identify_groups() {
    let c = Column::text(
        ColumnId(1),
        "group",
        vec![Some("a".into()), Some("b".into())],
    )
    .unwrap();
    let mut m = c.metadata().clone();
    m.value_labels = vec![
        ValueLabel {
            value: Cell::Text("a".into()),
            label: "Same".into(),
        },
        ValueLabel {
            value: Cell::Text("b".into()),
            label: "Same".into(),
        },
    ];
    let d = Dataset::new(vec![c.with_metadata(m).unwrap()]).unwrap();
    let result = lookup(&d, &request("same"));
    assert_eq!(result.choices.len(), 2);
    assert_ne!(result.choices[0].label, result.choices[1].label);
}
#[test]
fn query_independent_selected_presence() {
    let d = text(vec![Some("a".into()), Some("b".into()), None]);
    let mut r = request("absent");
    r.selected = [Some(Cell::Text("b".into())), Some(Cell::Text("c".into()))];
    assert_eq!(lookup(&d, &r).selected_present, [true, false]);
}
#[test]
fn signed_zero_unicode_empty_text_and_whitespace() {
    let d = text(vec![
        Some("".into()),
        Some(" ".into()),
        Some("Σ".into()),
        Some("ς".into()),
        None,
    ]);
    assert_eq!(lookup(&d, &request("")).choices.len(), 4);
    assert_eq!(lookup(&d, &request("Σ")).choices.len(), 1);
    let d = Dataset::new(vec![Column::numeric(
        ColumnId(1),
        "g",
        vec![Some(0.), Some(-0.)],
    )
    .unwrap()])
    .unwrap();
    assert_eq!(lookup(&d, &request("")).choices.len(), 1);
}
#[test]
fn large_text_budget_and_inner_cancel() {
    let d = text(vec![Some("a".repeat(1024 * 1024 + 1))]);
    assert!(group_values_with_budget(&d, &request(""), &JobControl::default(), &[], 0).is_err());
    let d = text(vec![Some("x".into())]);
    assert!(group_values_with_budget(
        &d,
        &request(""),
        &JobControl::default(),
        &[],
        MAX_OWNED_BYTES
    )
    .is_err());
    let c = JobControl::default();
    c.cancel();
    assert!(group_values_with_budget(&d, &request(""), &c, &[], 0).is_err());
}

#[test]
fn source_missing_rules_and_selected_types_are_respected() {
    let c = Column::text(
        ColumnId(1),
        "group",
        vec![Some("".into()), Some(" ".into()), None],
    )
    .unwrap();
    let mut m = c.metadata().clone();
    m.missing_rules = MissingRules::Text(vec!["".into()]);
    let d = Dataset::new(vec![c.with_metadata(m).unwrap()]).unwrap();
    let mut r = request("");
    r.selected = [Some(Cell::Text("".into())), Some(Cell::Text(" ".into()))];
    let result = lookup(&d, &r);
    assert_eq!(result.selected_present, [false, true]);
    assert_eq!(result.choices.len(), 1);
    for invalid in [Cell::Missing, Cell::Number(0.), Cell::Number(f64::NAN)] {
        r.selected = [Some(invalid), None];
        assert!(group_values_with_budget(
            &d,
            &r,
            &JobControl::default(),
            &[],
            r.heap_bytes().unwrap()
        )
        .is_err());
    }
    r = request(&"a".repeat(8193));
    assert!(
        group_values_with_budget(&d, &r, &JobControl::default(), &[], r.heap_bytes().unwrap())
            .is_err()
    );
    r = request("");
    r.column = ColumnId(99);
    assert!(group_values_with_budget(&d, &r, &JobControl::default(), &[], 0).is_err());
}
#[test]
fn output_is_bounded_and_raw_search_is_untruncated() {
    let first = format!("{}needle", "a".repeat(400_000));
    let second = format!("{}needle", "b".repeat(400_000));
    let d = text(vec![Some(first.clone()), Some(second)]);
    let r = request("needle");
    let result = lookup(&d, &r);
    assert_eq!(result.choices.len(), 1);
    assert!(result.more);
    assert_eq!(result.choices[0].value, Cell::Text(first));
    assert!(result.owned_bytes().unwrap() <= 1024 * 1024);
    assert_eq!(result.role_epoch, r.role_epoch);
    assert_eq!(result.query, r.query);
}
#[test]
fn long_source_search_observes_worker_cancellation() {
    let d = text(vec![Some("a".repeat(16 * 1024 * 1024))]);
    let r = request("aaaaab");
    let control = JobControl::default();
    let cancel = control.clone();
    let thread = std::thread::spawn(move || {
        std::thread::sleep(std::time::Duration::from_millis(2));
        cancel.cancel();
    });
    let result = group_values_with_budget(&d, &r, &control, &[], r.heap_bytes().unwrap());
    thread.join().unwrap();
    assert!(matches!(result, Err(tessera::error::AppError::Cancelled)));
}
#[test]
fn numeric_roundtrip_search_and_signed_zero_selected_identity() {
    let d = Dataset::new(vec![Column::numeric(
        ColumnId(1),
        "group",
        vec![Some(-0.), Some(0.), Some(1e-300)],
    )
    .unwrap()])
    .unwrap();
    let mut r = request("");
    r.selected = [Some(Cell::Number(0.)), Some(Cell::Number(-0.))];
    let result = lookup(&d, &r);
    assert_eq!(result.selected_present, [true, true]);
    assert_eq!(result.choices.len(), 2);
    r = request(&1e-300f64.to_string());
    assert_eq!(lookup(&d, &r).choices[0].value, Cell::Number(1e-300));
}
