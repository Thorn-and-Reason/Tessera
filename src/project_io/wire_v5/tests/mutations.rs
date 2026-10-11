use super::*;
fn rejected(v: &Value) {
    assert!(
        decode(&v.to_string(), MAX_OWNED_BYTES, &JobControl::default()).is_err(),
        "accepted corruption: {v}"
    )
}
#[test]
fn independent_scalar_count_grid_and_family_corruptions() {
    let p: Value = serde_json::from_str(&encode(&captures::capture(1, 1., false))).unwrap();
    for (path, value) in [
        ("/revision", json!("0")),
        ("/source_rows", json!(13)),
        ("/frequency", json!(13)),
        ("/fitted_rows", json!(11)),
        ("/observed_measurements", json!(25)),
        ("/incomplete_case_frequency", json!(1)),
        ("/method/numerical_version", json!(2)),
        ("/method/basis_version", json!(2)),
        ("/grid/complete", json!(false)),
        ("/grid/cells/0/frequency", json!(0)),
        ("/grid/factors/0/values/1", json!("a")),
        ("/columns/0/kind", json!("text")),
        ("/outcome/fitted/residual_df", json!(9)),
        ("/outcome/fitted/effects/0/dimension", json!(2)),
        ("/outcome/fitted/effects/0/factors", json!([1])),
        ("/outcome/fitted/effects/0/test/df1/value", json!(2)),
        ("/outcome/fitted/effects/0/test/p/value", json!(0)),
        ("/outcome/fitted/effects/0/test/p/value", json!(1e-320)),
        ("/outcome/fitted/effects/0/ss/state", json!("exact_zero")),
        (
            "/outcome/fitted/effects/0/ss/value/reason",
            json!("false reason"),
        ),
        ("/families/0/m", json!(0)),
        ("/families/0/members/0/ordinal", json!(1)),
        ("/families/0/members/0/request/second", json!("absent")),
        (
            "/families/0/members/0/request/scope",
            json!({"fixed":["x"]}),
        ),
        ("/families/0/members/0/adjusted_p/value", json!(0)),
        ("/families/0/members/0/interval", json!([])),
        (
            "/families/0/members/0/state",
            json!({"unavailable":{"kind":"numerical","reason":"failed"}}),
        ),
    ] {
        let mut bad = p.clone();
        *bad.pointer_mut(path).unwrap() = value;
        rejected(&bad);
    }
    let mut bad = p.clone();
    bad["request"]["all_pairs"] = json!([
        p["request"]["all_pairs"][0].clone(),
        p["request"]["all_pairs"][0].clone()
    ]);
    rejected(&bad);
    let mut bad = p.clone();
    let mut reverse = bad["request"]["planned_pairs"][0].clone();
    reverse["first"] = json!("b");
    reverse["second"] = json!("a");
    bad["request"]["planned_pairs"]
        .as_array_mut()
        .unwrap()
        .push(reverse);
    rejected(&bad);
    let mut bad = p.clone();
    bad["families"][1]["members"] = json!([]);
    rejected(&bad);
    let mut bad = p.clone();
    bad["families"][1]["members"][0]["request"]["first"] = json!("invented");
    rejected(&bad);
}
#[test]
fn repeated_mapping_metadata_covariance_and_linked_degrees_corruptions() {
    let p: Value = serde_json::from_str(&encode(&captures::capture(2, 1., false))).unwrap();
    for (path, value) in [
        ("/request/frequency_weight", json!("1")),
        ("/request/design/repeated/measurements", json!(["2", "3"])),
        (
            "/request/design/repeated/measurements",
            json!(["2", "2", "4"]),
        ),
        (
            "/request/design/repeated/measurements",
            json!(["1", "3", "4"]),
        ),
        ("/request/design/repeated/factors/0/name", json!("\u{2003}")),
        (
            "/request/design/repeated/factors/0/levels",
            json!(["a", "a", "c"]),
        ),
        (
            "/request/design/repeated/factors/0/levels",
            json!(["a", "b", "\u{2003}"]),
        ),
        ("/columns/1/kind", json!("text")),
        ("/observed_measurements", json!(14)),
        ("/grid/factors/0/column", json!("1")),
        ("/grid/factors/0/values", json!(["b", "a", "c"])),
        ("/outcome/fitted/effects/0/gg/epsilon/value", json!(1.1)),
        ("/outcome/fitted/effects/0/gg/epsilon/value", json!(0.49)),
        (
            "/outcome/fitted/effects/0/gg/covariance",
            json!("exact_zero"),
        ),
        ("/outcome/fitted/effects/0/gg/test/df2/value", json!(2)),
        ("/outcome/fitted/effects/0/gg/test/p/value", json!(0)),
        (
            "/outcome/fitted/effects/0/error_ss/state",
            json!({"unavailable":"numerical"}),
        ),
    ] {
        let mut bad = p.clone();
        *bad.pointer_mut(path).unwrap() = value;
        rejected(&bad);
    }
    let p: Value = serde_json::from_str(&encode(&captures::capture(3, 1., false))).unwrap();
    let mut bad = p.clone();
    bad["request"]["design"]["repeated"]["factors"][1]["name"] =
        bad["request"]["design"]["repeated"]["factors"][0]["name"].clone();
    rejected(&bad);
}
#[test]
fn absent_planned_identity_and_incomplete_family_are_preserved() {
    let r = captures::capture(1, 1., true);
    assert!(matches!(
        r.families[0].members[0].state,
        AnovaComparisonState::AbsentRequestedLevel(_)
    ));
    assert_eq!(
        decode(&encode(&r), MAX_OWNED_BYTES, &JobControl::default()).unwrap(),
        r
    );
    let p: Value = serde_json::from_str(&encode(&r)).unwrap();
    let mut bad = p.clone();
    bad["families"][0]["adjustment_failure"] = Value::Null;
    rejected(&bad);
    let mut bad = p.clone();
    bad["families"][0]["members"] = json!([]);
    rejected(&bad);
    let mut bad = p.clone();
    bad["families"][0]["members"][0]["adjusted_p"] = json!({"value":0.5,"reason":null});
    rejected(&bad);
}

#[test]
fn typed_numeric_level_zero_and_top_level_unavailability_dependencies() {
    let d = Dataset::new(vec![
        Column::numeric(
            ColumnId(1),
            "Y",
            vec![Some(1.), Some(2.), Some(3.), Some(5.)],
        )
        .unwrap(),
        Column::numeric(
            ColumnId(2),
            "factor",
            vec![Some(0.), Some(0.), Some(1.), Some(1.)],
        )
        .unwrap(),
    ])
    .unwrap();
    let request = AnovaRequest {
        design: AnovaDesign::OneWay {
            outcome: ColumnId(1),
            factor: ColumnId(2),
        },
        confidence: ConfidenceLevel::P95,
        filter: RowFilter::default(),
        frequency_weight: None,
        planned_pairs: vec![AnovaPairRequest {
            factor: 0,
            first: Cell::Number(0.),
            second: Cell::Number(1.),
            scope: AnovaScope::Marginal,
        }],
        all_pairs: vec![],
    };
    let r = captures::assemble(&d, request);
    let text = encode(&r);
    assert_eq!(
        decode(&text, MAX_OWNED_BYTES, &JobControl::default()).unwrap(),
        r
    );
    let p: Value = serde_json::from_str(&text).unwrap();
    let mut bad = p.clone();
    bad["grid"]["factors"][0]["values"][0] = json!(-0.0);
    rejected(&bad);
    let mut bad = p.clone();
    bad["grid"]["factors"][0]["values"][0] = json!("0");
    rejected(&bad);
    let mut bad = p.clone();
    bad["outcome"] = json!({"unavailable":{"kind":"numerical","reason":"Unavailable model"}});
    rejected(&bad);
}
