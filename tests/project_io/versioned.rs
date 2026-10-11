use super::*;
use serde_json::{json, Value};
fn fixture() -> Value {
    serde_json::from_slice(include_bytes!("../../examples/mixed-analysis.tessera")).unwrap()
}
fn accepts(v: &Value) -> bool {
    read(&serde_json::to_vec(v).unwrap()).is_ok()
}
#[test]
fn every_v2_record_is_map_only_and_nullable_fields_are_required() {
    let original = fixture();
    assert!(accepts(&original));
    let mut records = vec![
        (
            "",
            vec![
                "format",
                "version",
                "title",
                "dataset",
                "runs",
                "next_run_id",
                "provenance",
            ],
        ),
        ("/dataset", vec!["revision", "rows", "columns"]),
        (
            "/dataset/columns/0",
            vec!["id", "kind", "metadata", "values"],
        ),
        (
            "/dataset/columns/0/metadata",
            vec![
                "name",
                "description",
                "measurement",
                "missing_rules",
                "value_labels",
                "display_decimals",
            ],
        ),
    ];
    for (i, tag, request_fields, result_fields) in [
        (
            0,
            "describe",
            vec![
                "columns",
                "statistics",
                "missing_policy",
                "filter",
                "frequency_weight",
            ],
            vec![
                "revision",
                "request",
                "columns",
                "rows",
                "diagnostics",
                "plots",
            ],
        ),
        (
            1,
            "pearson",
            vec!["columns", "missing_policy", "filter", "frequency_weight"],
            vec!["revision", "request", "columns", "diagnostics", "rows"],
        ),
        (
            2,
            "one_sample_t",
            vec![
                "variable",
                "reference_mean",
                "confidence",
                "filter",
                "frequency_weight",
            ],
            vec![
                "revision",
                "request",
                "columns",
                "diagnostics",
                "sample",
                "missing_rows",
                "statistics",
            ],
        ),
        (
            3,
            "paired_t",
            vec![
                "first",
                "second",
                "confidence",
                "filter",
                "frequency_weight",
            ],
            vec![
                "revision",
                "request",
                "columns",
                "diagnostics",
                "first",
                "second",
                "difference_sd",
                "statistics",
            ],
        ),
        (
            4,
            "independent_t",
            vec![
                "outcome",
                "group",
                "groups",
                "method",
                "confidence",
                "filter",
                "frequency_weight",
            ],
            vec![
                "revision",
                "request",
                "columns",
                "diagnostics",
                "groups",
                "exclusions",
                "statistics",
            ],
        ),
    ] {
        let base = format!("/runs/{i}/analysis/{tag}");
        for (path, fields) in [
            (base.clone(), result_fields),
            (format!("{base}/request"), request_fields),
            (
                format!("{base}/diagnostics"),
                vec![
                    "filtered_rows",
                    "missing_weight_rows",
                    "zero_weight_rows",
                    "complete_case_rows",
                ],
            ),
            (
                format!("{base}/columns/0"),
                vec![
                    "id",
                    "name",
                    "kind",
                    "description",
                    "missing_rules",
                    "value_labels",
                ],
            ),
            (format!("{base}/request/filter"), vec!["predicates"]),
        ] {
            let mut v = original.clone();
            let a = fields
                .iter()
                .map(|k| v.pointer(&path).unwrap()[*k].clone())
                .collect();
            *v.pointer_mut(&path).unwrap() = Value::Array(a);
            assert!(!accepts(&v), "accepted record array {path}");
        }
        let mut v = original.clone();
        v.pointer_mut(&format!("{base}/request"))
            .unwrap()
            .as_object_mut()
            .unwrap()
            .remove("frequency_weight");
        assert!(!accepts(&v), "accepted omitted nullable weight {tag}");
        if i >= 2 {
            for path in [
                format!("{base}/statistics"),
                format!("{base}/statistics/difference"),
            ] {
                let keys = if path.ends_with("difference") {
                    vec!["value", "reason"]
                } else {
                    vec![
                        "difference",
                        "standard_error",
                        "t",
                        "df",
                        "p",
                        "interval",
                        "effect_size",
                    ]
                };
                let mut v = original.clone();
                let array = keys
                    .iter()
                    .map(|k| v.pointer(&path).unwrap()[*k].clone())
                    .collect();
                *v.pointer_mut(&path).unwrap() = Value::Array(array);
                assert!(!accepts(&v), "accepted {path}");
            }
            for key in ["value", "reason"] {
                let mut v = original.clone();
                v.pointer_mut(&format!("{base}/statistics/difference"))
                    .unwrap()
                    .as_object_mut()
                    .unwrap()
                    .remove(key);
                assert!(!accepts(&v), "accepted missing estimate {key}");
            }
            for malformed in [
                json!({"lower":0,"upper":1}),
                json!([]),
                json!([null]),
                json!([null, null, null]),
            ] {
                let mut v = original.clone();
                *v.pointer_mut(&format!("{base}/statistics/interval"))
                    .unwrap() = malformed;
                assert!(!accepts(&v));
            }
        }
    }
    for (path, fields) in records.drain(..) {
        let mut v = original.clone();
        let array = fields
            .iter()
            .map(|k| v.pointer(path).unwrap()[*k].clone())
            .collect();
        *v.pointer_mut(path).unwrap() = Value::Array(array);
        assert!(!accepts(&v), "accepted {path}");
    }
}
#[test]
fn external_tags_and_procedure_settings_are_strict() {
    let original = fixture();
    for (path, value) in [
        (
            "/runs/1/analysis/pearson/request/missing_policy",
            json!("complete_case"),
        ),
        (
            "/runs/2/analysis/one_sample_t/request/confidence",
            json!("95"),
        ),
        (
            "/runs/3/analysis/paired_t/request/confidence",
            json!({"95%":null}),
        ),
        (
            "/runs/4/analysis/independent_t/request/method",
            json!("student"),
        ),
        (
            "/runs/4/analysis/independent_t/request/groups",
            json!({"first":"<A>","second":"B&"}),
        ),
        (
            "/runs/4/analysis/independent_t/request/groups",
            json!([null, "B&"]),
        ),
        (
            "/runs/4/analysis/independent_t/request/groups",
            json!([1, 2]),
        ),
        (
            "/runs/4/analysis/independent_t/request/groups",
            json!(["<A>", "<A>"]),
        ),
        ("/runs/4/analysis/independent_t/request/outcome", json!("3")),
        (
            "/runs/1/analysis/pearson/rows/0/n",
            json!(9007199254740993u64),
        ),
        ("/runs/1/analysis/pearson/rows/0/r/value", json!(1.1)),
        ("/runs/1/analysis/pearson/rows/0/p/value", json!(-0.1)),
        (
            "/runs/2/analysis/one_sample_t/statistics/df/value",
            json!(-1),
        ),
        (
            "/runs/4/analysis/independent_t/groups/0/mean",
            json!({"value":1,"reason":"not available"}),
        ),
    ] {
        let mut v = original.clone();
        *v.pointer_mut(path).unwrap() = value;
        assert!(!accepts(&v), "accepted {path}");
    }
    for (i, tag) in [
        (1, "pearson"),
        (2, "one_sample_t"),
        (3, "paired_t"),
        (4, "independent_t"),
    ] {
        let wire = serde_json::to_string(&original).unwrap();
        let payload = serde_json::to_string(&original["runs"][i]["analysis"][tag]).unwrap();
        let prefix = format!("\"{tag}\":");
        let duplicate = wire.replacen(&prefix, &format!("{prefix}{payload},{prefix}"), 1);
        assert!(
            read(duplicate.as_bytes()).is_err(),
            "accepted duplicate {tag}"
        );
        for wrong in ["describe", "unsupported"] {
            let bad = wire.replacen(&prefix, &format!("\"{wrong}\":"), 1);
            assert!(read(bad.as_bytes()).is_err());
        }
    }
    let mut v = original.clone();
    v.as_object_mut().unwrap().remove("version");
    assert!(!accepts(&v));
    let mut v = original.clone();
    v["runs"][1]["operation"] = json!("describe");
    assert!(!accepts(&v));
}
#[test]
fn map_only_predicates_rules_labels_rows_and_points_agree_with_schema() {
    let p = with_run();
    let mut v: Value = serde_json::from_slice(&bytes(&p)).unwrap();
    v["runs"][0]["analysis"]["request"]["filter"]["predicates"] =
        json!([{"number":["2","equal",18]}]);
    assert!(!accepts(&v));
    let original = fixture();
    for path in [
        "/runs/2/analysis/one_sample_t/sample",
        "/runs/3/analysis/paired_t/first",
        "/runs/4/analysis/independent_t/groups/0",
    ] {
        let mut v = original.clone();
        let value = v.pointer(path).unwrap();
        let array = vec![
            value["n"].clone(),
            value["mean"].clone(),
            value["sample_sd"].clone(),
        ];
        *v.pointer_mut(path).unwrap() = json!(array);
        assert!(!accepts(&v));
    }
    let mut v = original.clone();
    v["runs"][4]["analysis"]["independent_t"]["exclusions"] = json!([0, 0, 0]);
    assert!(!accepts(&v));
    let mut v = original.clone();
    v["runs"][1]["analysis"]["pearson"]["rows"][0] =
        json!(["1","2",4,{"value":0.8,"reason":null},{"value":0.1,"reason":null},0,0]);
    assert!(!accepts(&v));
}

struct FullReads<R>(R);
impl<R: Read> Read for FullReads<R> {
    fn read(&mut self, out: &mut [u8]) -> std::io::Result<usize> {
        let mut n = 0;
        while n < out.len() {
            let count = self.0.read(&mut out[n..])?;
            if count == 0 {
                break;
            }
            n += count;
        }
        Ok(n)
    }
}
#[test]
fn v2_huge_untrusted_fields_fail_before_error_or_numeric_scratch_growth() {
    let original = fixture();
    let mut cases = vec![];
    for path in [
        "/runs/1/analysis/pearson/request/missing_policy",
        "/runs/2/analysis/one_sample_t/request/confidence",
        "/runs/3/analysis/paired_t/request/first",
        "/runs/4/analysis/independent_t/request/method",
        "/runs/4/analysis/independent_t/request",
        "/runs/4/analysis/independent_t/exclusions/missing_group_rows",
        "/runs/1/analysis/pearson/rows/0/r/value",
        "/runs/2/analysis/one_sample_t/statistics/interval",
    ] {
        let mut v = original.clone();
        *v.pointer_mut(path).unwrap() = json!("__PAYLOAD__");
        cases.push((path, serde_json::to_string(&v).unwrap()));
    }
    let wire = serde_json::to_string(&original).unwrap();
    cases.push((
        "external procedure tag",
        wire.replacen("\"pearson\":", "\"__PAYLOAD__\":", 1),
    ));
    for (name, wire) in cases {
        let (prefix, suffix) = wire.split_once("__PAYLOAD__").unwrap();
        let reader = std::io::Cursor::new(prefix.as_bytes())
            .chain(std::io::repeat(b'x').take(30 * 1024 * 1024))
            .chain(std::io::Cursor::new(suffix.as_bytes()));
        let (result, allocated) = allocations(|| {
            decode_with_budget(
                FullReads(reader),
                &JobControl::default(),
                &[],
                MAX_OWNED_BYTES - 96 * 1024 * 1024,
            )
        });
        assert!(result.is_err(), "accepted {name}");
        assert!(
            allocated < 65 * 1024 * 1024,
            "unbounded error for {name}: {allocated}"
        );
        assert!(result.unwrap_err().to_string().len() < 256);
    }
    let mut v = original.clone();
    v["runs"][2]["analysis"]["one_sample_t"]["request"]["reference_mean"] = json!("__NUMBER__");
    let wire = serde_json::to_string(&v).unwrap();
    let (prefix, suffix) = wire.split_once("\"__NUMBER__\"").unwrap();
    let reader = std::io::Cursor::new(prefix.as_bytes())
        .chain(std::io::Cursor::new(b"0."))
        .chain(std::io::repeat(b'1').take(30 * 1024 * 1024))
        .chain(std::io::Cursor::new(suffix.as_bytes()));
    let (result, allocated) = allocations(|| {
        decode_with_budget(
            FullReads(reader),
            &JobControl::default(),
            &[],
            MAX_OWNED_BYTES - 56 * 1024 * 1024,
        )
    });
    assert!(matches!(
        result,
        Err(tessera::error::AppError::SizeLimit { .. })
    ));
    assert!(allocated < 65 * 1024 * 1024);
}
