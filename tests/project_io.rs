mod common;
use std::io::Read;
use tessera::{
    analysis::{describe, DescribeRequest, MissingPolicy, RowFilter, Statistic},
    data::{
        Cell, Column, ColumnId, ColumnMetadata, Dataset, MissingRules, ValueLabel, MAX_OWNED_BYTES,
    },
    operation::JobControl,
    project::Project,
    project_io::{decode, decode_with_budget, encode, encode_with_budget, MAX_PROJECT_BYTES},
};
fn bytes(project: &Project) -> Vec<u8> {
    let mut bytes = Vec::new();
    encode(project, &mut bytes, &JobControl::default()).unwrap();
    bytes
}
fn read(bytes: &[u8]) -> tessera::error::AppResult<Project> {
    decode(bytes, &JobControl::default())
}
fn with_run() -> Project {
    let dataset = common::sample_dataset();
    let request = DescribeRequest {
        columns: vec![ColumnId(3), ColumnId(4)],
        statistics: vec![Statistic::Mean, Statistic::SampleStdDev, Statistic::Minimum],
        missing_policy: MissingPolicy::PerVariable,
        filter: RowFilter::default(),
        frequency_weight: None,
    };
    let run = describe(&dataset, &request, &JobControl::default()).unwrap();
    Project::new("Six rows", dataset)
        .unwrap()
        .with_run(run)
        .unwrap()
}
#[test]
fn project_roundtrip_preserves_values_and_history() {
    let original = with_run();
    let reopened = read(&bytes(&original)).unwrap();
    assert_eq!(reopened.runs(), original.runs());
    assert_eq!(reopened.next_run_id(), original.next_run_id());
    let id = ColumnId(9007199254740993);
    let text = Column::text(
        id,
        "Cymraeg",
        vec![None, Some("".into()), Some("ŵ 海 😀".into())],
    )
    .unwrap();
    let text = text
        .with_metadata(ColumnMetadata {
            missing_rules: MissingRules::Text(vec!["NA".into()]),
            value_labels: vec![ValueLabel {
                value: Cell::Text("".into()),
                label: "Empty".into(),
            }],
            description: "Preserved".into(),
            display_decimals: 16,
            ..text.metadata().clone()
        })
        .unwrap();
    let numeric = Column::numeric(
        ColumnId(2),
        "Extreme",
        vec![Some(f64::MAX), Some(f64::from_bits(1)), Some(-0.0)],
    )
    .unwrap();
    let original = Project::new("Unicode", Dataset::new(vec![text, numeric]).unwrap()).unwrap();
    let reopened = read(&bytes(&original)).unwrap();
    assert_eq!(reopened.dataset().cell(id, 0).unwrap(), Cell::Missing);
    assert_eq!(
        reopened.dataset().cell(id, 1).unwrap(),
        Cell::Text("".into())
    );
    assert_eq!(
        reopened.dataset().column(id).unwrap().metadata(),
        original.dataset().column(id).unwrap().metadata()
    );
    for row in 0..3 {
        let Cell::Number(value) = reopened.dataset().cell(ColumnId(2), row).unwrap() else {
            panic!()
        };
        let Cell::Number(expected) = original.dataset().cell(ColumnId(2), row).unwrap() else {
            panic!()
        };
        assert_eq!(value.to_bits(), expected.to_bits());
    }
}
#[test]
fn malformed_or_oversized_project_is_rejected() {
    for fixture in [
        r#"{"format":"foreign-project","version":999}"#,
        r#"{"format":"foreign-project","version":1,"surprise":true}"#,
        "[[]]",
        "{}",
    ] {
        assert!(read(fixture.as_bytes()).is_err());
    }
    let original = with_run();
    let encoded = String::from_utf8(bytes(&original)).unwrap();
    for bad in [
        encoded.replace("\"describe\":", "\"script\":"),
        encoded.replace("\"next_run_id\":\"2\"", "\"next_run_id\":\"1\""),
        encoded.replacen("\"id\":\"2\"", "\"id\":\"1\"", 1),
        encoded.replacen("\"rows\":6", "\"rows\":7", 1),
        encoded.replacen("\"version\":3", "\"version\":3,\"version\":3", 1),
    ] {
        assert!(read(bad.as_bytes()).is_err(), "bad fixture accepted");
    }
    let reader = std::io::repeat(b' ').take(MAX_PROJECT_BYTES as u64 + 1);
    assert!(decode(reader, &JobControl::default()).is_err());
}
#[test]
fn project_loading_has_no_external_effects() {
    let project = with_run();
    let encoded = String::from_utf8(bytes(&project)).unwrap();
    let fixture = encoded.replacen("\"provenance\":[]", "\"provenance\":[\"file:///tmp/never-created-tessera\",\"https://invalid.example/\",\"sh -c touch /tmp/never-created-tessera\"]", 1);
    let reopened = read(fixture.as_bytes()).unwrap();
    assert_eq!(reopened.runs(), project.runs());
    assert_eq!(
        reopened.provenance(),
        &[
            "file:///tmp/never-created-tessera",
            "https://invalid.example/",
            "sh -c touch /tmp/never-created-tessera"
        ]
    );
    assert_eq!(
        read(&bytes(&reopened)).unwrap().provenance(),
        reopened.provenance()
    );
    assert!(!std::path::Path::new("/tmp/never-created-tessera").exists());
    assert!(read(b"broken").is_err());
    assert_eq!(project.runs().len(), 1);
}
#[test]
fn historical_types_survive_current_type_change() {
    let original = with_run();
    let column = original
        .dataset()
        .column(ColumnId(3))
        .unwrap()
        .convert(tessera::data::ColumnKind::Text)
        .unwrap();
    let dataset = original.dataset().replace_column(column).unwrap();
    let changed = original.with_dataset(dataset).unwrap();
    assert_eq!(read(&bytes(&changed)).unwrap().runs(), changed.runs());
}
#[test]
fn retained_budget_and_cancellation_are_enforced() {
    let project = with_run();
    let encoded = bytes(&project);
    assert!(decode_with_budget(
        &encoded[..],
        &JobControl::default(),
        &[project.dataset()],
        MAX_OWNED_BYTES
    )
    .is_err());
    assert!(decode_with_budget(
        &encoded[..],
        &JobControl::default(),
        &[],
        MAX_OWNED_BYTES - encoded.len() * 3
    )
    .is_err());
    assert!(encode_with_budget(
        &project,
        std::io::sink(),
        &JobControl::default(),
        &[],
        MAX_OWNED_BYTES
    )
    .is_err());
    let control = JobControl::default();
    control.cancel();
    assert!(matches!(
        decode(&encoded[..], &control),
        Err(tessera::error::AppError::Cancelled)
    ));
    assert!(matches!(
        encode(&project, std::io::sink(), &control),
        Err(tessera::error::AppError::Cancelled)
    ));
}

fn altered(project: &Project, pointer: &str, replacement: serde_json::Value) -> Vec<u8> {
    let mut value: serde_json::Value = serde_json::from_slice(&bytes(project)).unwrap();
    *value.pointer_mut(pointer).unwrap() = replacement;
    serde_json::to_vec(&value).unwrap()
}
#[test]
fn invalid_cells_metadata_snapshots_and_results_are_rejected() {
    use serde_json::json;
    let project = with_run();
    for (path, bad) in [
        ("/dataset/columns/0/values/0", json!(false)),
        ("/dataset/columns/1/values/0", json!("17")),
        ("/dataset/columns/0/id", json!("01")),
        ("/dataset/revision", json!(0)),
        ("/dataset/columns/0/metadata/display_decimals", json!(17)),
        (
            "/dataset/columns/1/metadata/missing_rules",
            json!({"text":["NA"]}),
        ),
        (
            "/dataset/columns/1/metadata/missing_rules",
            json!({"number":{"values":[],"ranges":[[2,1]]}}),
        ),
        (
            "/dataset/columns/1/metadata/value_labels",
            json!([{"value":1,"label":"One"},{"value":1,"label":"Again"}]),
        ),
        ("/dataset/columns/0/metadata/name", json!("Age")),
        (
            "/runs/0/analysis/describe/request/columns",
            json!(["3", "3"]),
        ),
        ("/runs/0/analysis/describe/request/columns", json!(["99"])),
        (
            "/runs/0/analysis/describe/request/statistics",
            json!(["median"]),
        ),
        ("/runs/0/analysis/describe/columns/0/kind", json!("text")),
        ("/runs/0/analysis/describe/revision", json!("2")),
        (
            "/runs/0/analysis/describe/rows/0/valid_n",
            json!(9007199254740993u64),
        ),
        (
            "/runs/0/analysis/describe/rows/0/estimates/mean",
            json!({"value":null,"reason":null}),
        ),
        (
            "/runs/0/analysis/describe/plots/0/points/0/frequency",
            json!(0),
        ),
        ("/runs/0/analysis/describe/plots/0/reduced", json!(true)),
        ("/runs/0/analysis/describe/rows/0/missing_n", json!(1)),
        (
            "/runs/0/analysis/describe/rows/0/complete_case_excluded_n",
            json!(1),
        ),
        (
            "/runs/0/analysis/describe/diagnostics/complete_case_rows",
            json!(1),
        ),
        (
            "/runs/0/analysis/describe/diagnostics/missing_weight_rows",
            json!(1),
        ),
        (
            "/runs/0/analysis/describe/diagnostics/filtered_rows",
            json!(2_000_001),
        ),
        ("/runs/0/analysis/describe/plots/0/source_rows", json!(7)),
        (
            "/runs/0/analysis/describe/plots/0/points/0/frequency",
            json!(2),
        ),
        (
            "/runs/0/analysis/describe/columns/1/name",
            json!("First item"),
        ),
        (
            "/dataset/columns/0/metadata",
            json!({"nested":{"deeper":[]}}),
        ),
        ("/provenance", json!([{"path":"file:///tmp/x"}])),
    ] {
        assert!(
            read(&altered(&project, path, bad)).is_err(),
            "accepted {path}"
        );
    }
    let encoded = String::from_utf8(bytes(&project)).unwrap();
    assert!(read(
        encoded
            .replacen("\"mean\":{", "\"mean\":{\"value\":1,", 1)
            .as_bytes()
    )
    .is_err());
    assert!(read(
        encoded
            .replacen("\"metadata\":{", "\"metadata\":{\"extra\":true,", 1)
            .as_bytes()
    )
    .is_err());
    assert!(read(
        encoded
            .replacen("\"values\":[17.0", "\"values\":[1e400", 1)
            .as_bytes()
    )
    .is_err());
    assert!(read(
        encoded
            .replacen("\"title\":", "\"extra\":[] ,\"title\":", 1)
            .as_bytes()
    )
    .is_err());
}
#[test]
fn filters_weights_rules_and_historical_missingness_roundtrip() {
    use tessera::{
        analysis::{NumericComparison, RowPredicate},
        data::Measurement,
    };
    let dataset = common::sample_dataset();
    let column = dataset.column(ColumnId(3)).unwrap();
    let column = column
        .with_metadata(ColumnMetadata {
            measurement: Measurement::Ordinal,
            missing_rules: MissingRules::Number {
                values: vec![8.0],
                ranges: vec![(9.0, 10.0)],
            },
            value_labels: vec![ValueLabel {
                value: Cell::Number(12.0),
                label: "Twelve".into(),
            }],
            ..column.metadata().clone()
        })
        .unwrap();
    let dataset = dataset.replace_column(column).unwrap();
    let request = DescribeRequest {
        columns: vec![ColumnId(3), ColumnId(4)],
        statistics: vec![
            Statistic::Mean,
            Statistic::Variance,
            Statistic::Maximum,
            Statistic::Sum,
            Statistic::Range,
            Statistic::StdErrorMean,
        ],
        missing_policy: MissingPolicy::CompleteCase,
        filter: RowFilter {
            predicates: vec![
                RowPredicate::Number {
                    column: ColumnId(2),
                    comparison: NumericComparison::GreaterOrEqual,
                    value: 18.0,
                },
                RowPredicate::Text {
                    column: ColumnId(5),
                    value: "Control".into(),
                    equal: true,
                },
                RowPredicate::Missing {
                    column: ColumnId(4),
                    missing: false,
                },
            ],
        },
        frequency_weight: Some(ColumnId(2)),
    };
    let run = describe(&dataset, &request, &JobControl::default()).unwrap();
    let project = Project::new("Weighted history", dataset)
        .unwrap()
        .with_run(run)
        .unwrap();
    let column = project.dataset().column(ColumnId(3)).unwrap();
    let current = column
        .with_metadata(ColumnMetadata {
            missing_rules: MissingRules::None,
            ..column.metadata().clone()
        })
        .unwrap();
    let changed = project
        .with_dataset(project.dataset().replace_column(current).unwrap())
        .unwrap();
    let reopened = read(&bytes(&changed)).unwrap();
    assert_eq!(reopened.runs(), changed.runs());
    assert_eq!(
        reopened.dataset().column(ColumnId(3)).unwrap().metadata(),
        changed.dataset().column(ColumnId(3)).unwrap().metadata()
    );
    let mut request = request;
    for comparison in [
        NumericComparison::Equal,
        NumericComparison::NotEqual,
        NumericComparison::Less,
        NumericComparison::LessOrEqual,
        NumericComparison::Greater,
    ] {
        request.filter.predicates = vec![tessera::analysis::RowPredicate::Number {
            column: ColumnId(2),
            comparison,
            value: 18.0,
        }];
        let run = describe(reopened.dataset(), &request, &JobControl::default()).unwrap();
        let next = reopened.with_run(run).unwrap();
        assert_eq!(read(&bytes(&next)).unwrap().runs(), next.runs());
    }
}
#[test]
fn empty_datasets_unavailable_estimates_and_large_plot_roundtrip() {
    let empty = Project::new("Empty", Dataset::new(vec![]).unwrap()).unwrap();
    assert_eq!(read(&bytes(&empty)).unwrap().dataset().row_count(), 0);
    for len in [0, 1, 2001] {
        let dataset = Dataset::new(vec![Column::numeric(
            ColumnId(1),
            "Value",
            vec![Some(1.0); len],
        )
        .unwrap()])
        .unwrap();
        let request = DescribeRequest {
            columns: vec![ColumnId(1)],
            statistics: vec![Statistic::Mean, Statistic::SampleStdDev],
            missing_policy: MissingPolicy::PerVariable,
            filter: RowFilter::default(),
            frequency_weight: None,
        };
        let run = describe(&dataset, &request, &JobControl::default()).unwrap();
        let project = Project::new("Plot", dataset)
            .unwrap()
            .with_run(run)
            .unwrap();
        assert_eq!(read(&bytes(&project)).unwrap().runs(), project.runs());
    }
}
#[test]
fn depth_dimensions_and_mid_decode_budget_are_bounded() {
    let project = with_run();
    let mut encoded = serde_json::from_slice::<serde_json::Value>(&bytes(&project)).unwrap();
    encoded["unexpected"] =
        serde_json::from_str(&format!("{}0{}", "[".repeat(65), "]".repeat(65))).unwrap();
    assert!(read(&serde_json::to_vec(&encoded).unwrap()).is_err());
    assert!(read(&altered(
        &project,
        "/dataset/rows",
        serde_json::json!(2_000_001)
    ))
    .is_err());
    let mut encoded = bytes(&project);
    encoded.extend_from_slice(b" {}");
    assert!(read(&encoded).is_err());
    let large = "a".repeat(20_000);
    let fixture = altered(
        &project,
        "/dataset/columns/0/metadata/description",
        serde_json::json!(large),
    );
    // Permit input growth (old + doubled allocation), but leave less than the
    // owned description needs during the typed pass. Temporary record keys do
    // not form retained map entries and must not be needed to trigger refusal.
    let headroom = fixture.len().next_power_of_two() * 3 / 2;
    let (sender, receiver) = std::sync::mpsc::channel();
    let control = JobControl::new(
        std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false)),
        Some(sender),
    );
    assert!(matches!(
        decode_with_budget(&fixture[..], &control, &[], MAX_OWNED_BYTES - headroom),
        Err(tessera::error::AppError::SizeLimit { .. })
    ));
    assert!(
        receiver
            .try_iter()
            .filter(|p| p.done == fixture.len())
            .count()
            >= 3,
        "input reading and both JSON passes must begin before owned-payload refusal"
    );
    let encoded = bytes(&project);
    assert!(read(
        std::str::from_utf8(&encoded)
            .unwrap()
            .replacen("\"values\":[17.0", "\"values\":[-0", 1)
            .as_bytes()
    )
    .is_ok());
}
#[test]
fn io_failures_and_mid_operation_cancellation_propagate() {
    use tessera::error::AppError;
    struct Fails;
    impl Read for Fails {
        fn read(&mut self, _: &mut [u8]) -> std::io::Result<usize> {
            Err(std::io::Error::other("failure"))
        }
    }
    impl std::io::Write for Fails {
        fn write(&mut self, _: &[u8]) -> std::io::Result<usize> {
            Err(std::io::Error::other("failure"))
        }
        fn flush(&mut self) -> std::io::Result<()> {
            Ok(())
        }
    }
    assert!(matches!(
        decode(Fails, &JobControl::default()),
        Err(AppError::Io(_))
    ));
    assert!(matches!(
        encode(&with_run(), Fails, &JobControl::default()),
        Err(AppError::Io(_))
    ));
    struct Cancels<'a>(&'a JobControl);
    impl std::io::Write for Cancels<'_> {
        fn write(&mut self, bytes: &[u8]) -> std::io::Result<usize> {
            self.0.cancel();
            Ok(bytes.len())
        }
        fn flush(&mut self) -> std::io::Result<()> {
            Ok(())
        }
    }
    let control = JobControl::default();
    assert!(matches!(
        encode(&with_run(), Cancels(&control), &control),
        Err(AppError::Cancelled)
    ));
}

#[test]
fn documented_sample_decodes_and_schema_is_a_complete_v1_object() {
    let sample = read(include_bytes!("../examples/sample.tessera")).unwrap();
    assert_eq!(sample.dataset().row_count(), 6);
    assert_eq!(sample.dataset().column_count(), 6);
    assert_eq!(
        sample.dataset().cell(ColumnId(6), 5).unwrap(),
        Cell::Number(31.0)
    );
    assert_eq!(sample.provenance(), &["Synthetic six-row teaching data."]);
    let reopened = read(&bytes(&sample)).unwrap();
    assert_eq!(reopened.provenance(), sample.provenance());
    let schema: serde_json::Value =
        serde_json::from_str(include_str!("../docs/project.schema.json")).unwrap();
    assert_eq!(schema["additionalProperties"], false);
    assert_eq!(schema["required"].as_array().unwrap().len(), 9);
    assert_eq!(
        schema["properties"]["version"]["const"],
        serde_json::json!(3)
    );
}

#[test]
fn every_declared_scalar_enum_rejects_object_forms() {
    use serde_json::json;
    let project = with_run();
    for (path, name) in [
        ("/format", "tessera-project"),
        ("/dataset/columns/1/kind", "number"),
        ("/dataset/columns/0/kind", "text"),
        ("/dataset/columns/1/metadata/measurement", "scale"),
        ("/dataset/columns/0/metadata/measurement", "nominal"),
        ("/dataset/columns/1/metadata/measurement", "unknown"),
        ("/dataset/columns/1/metadata/measurement", "ordinal"),
        ("/dataset/columns/1/metadata/missing_rules", "none"),
        (
            "/runs/0/analysis/describe/request/missing_policy",
            "per_variable",
        ),
        (
            "/runs/0/analysis/describe/request/missing_policy",
            "complete_case",
        ),
        ("/runs/0/analysis/describe/columns/0/kind", "number"),
        ("/runs/0/analysis/describe/columns/0/missing_rules", "none"),
    ] {
        assert!(
            read(&altered(&project, path, json!({ name: null }))).is_err(),
            "accepted object enum at {path}: {name}"
        );
    }
    let mut fixture: serde_json::Value = serde_json::from_slice(&bytes(&project)).unwrap();
    for name in [
        "mean",
        "sample_std_dev",
        "variance",
        "minimum",
        "maximum",
        "sum",
        "range",
        "std_error_mean",
    ] {
        fixture["runs"][0]["analysis"]["describe"]["request"]["statistics"] = json!([name]);
        for row in fixture["runs"][0]["analysis"]["describe"]["rows"]
            .as_array_mut()
            .unwrap()
        {
            row["estimates"] = json!({name:{"value":1.0,"reason":null}});
        }
        assert!(
            read(&serde_json::to_vec(&fixture).unwrap()).is_ok(),
            "rejected string statistic: {name}"
        );
        fixture["runs"][0]["analysis"]["describe"]["request"]["statistics"] = json!([{name:null}]);
        assert!(
            read(&serde_json::to_vec(&fixture).unwrap()).is_err(),
            "accepted object statistic: {name}"
        );
    }
    let mut fixture: serde_json::Value = serde_json::from_slice(&bytes(&project)).unwrap();
    for name in [
        "equal",
        "not_equal",
        "less",
        "less_or_equal",
        "greater",
        "greater_or_equal",
    ] {
        fixture["runs"][0]["analysis"]["describe"]["request"]["filter"]["predicates"] =
            json!([{"number":{"column":"3", "comparison":{name:null},"value":8}}]);
        assert!(
            read(&serde_json::to_vec(&fixture).unwrap()).is_err(),
            "accepted comparison object: {name}"
        );
    }
}
#[test]
fn estimates_require_both_nullable_keys() {
    let project = with_run();
    let encoded = bytes(&project);
    let mut value: serde_json::Value = serde_json::from_slice(&encoded).unwrap();
    let available = value
        .pointer_mut("/runs/0/analysis/describe/rows/0/estimates/mean")
        .unwrap();
    available.as_object_mut().unwrap().remove("reason");
    assert!(
        read(&serde_json::to_vec(&value).unwrap()).is_err(),
        "accepted absent reason"
    );
    let mut value: serde_json::Value = serde_json::from_slice(&encoded).unwrap();
    *value
        .pointer_mut("/runs/0/analysis/describe/rows/0/estimates/mean")
        .unwrap() = serde_json::json!({"reason":"Unavailable"});
    assert!(
        read(&serde_json::to_vec(&value).unwrap()).is_err(),
        "accepted absent value"
    );
    *value
        .pointer_mut("/runs/0/analysis/describe/rows/0/estimates/mean")
        .unwrap() = serde_json::json!({"value":null,"reason":"Unavailable"});
    assert!(
        read(&serde_json::to_vec(&value).unwrap()).is_ok(),
        "rejected explicit null value"
    );
    assert!(read(&encoded).is_ok(), "rejected explicit null reason");
}

// Test-only allocator observer; measurement is local to the calling test thread.
struct ObservedAllocator;
thread_local! {
    static OBSERVE_ALLOCATIONS: std::cell::Cell<bool> = const { std::cell::Cell::new(false) };
    static ALLOCATED_BYTES: std::cell::Cell<usize> = const { std::cell::Cell::new(0) };
}
// SAFETY: Every allocation/deallocation is forwarded to System with its original layout.
unsafe impl std::alloc::GlobalAlloc for ObservedAllocator {
    unsafe fn alloc(&self, layout: std::alloc::Layout) -> *mut u8 {
        let _ = OBSERVE_ALLOCATIONS.try_with(|active| {
            if active.get() {
                let _ = ALLOCATED_BYTES
                    .try_with(|bytes| bytes.set(bytes.get().saturating_add(layout.size())));
            }
        });
        // SAFETY: The caller supplied the valid allocation layout required by GlobalAlloc.
        unsafe { std::alloc::System.alloc(layout) }
    }
    unsafe fn dealloc(&self, ptr: *mut u8, layout: std::alloc::Layout) {
        // SAFETY: The pointer/layout are forwarded unchanged to their originating allocator.
        unsafe { std::alloc::System.dealloc(ptr, layout) }
    }
}
#[global_allocator]
static ALLOCATOR: ObservedAllocator = ObservedAllocator;
fn allocations<T>(f: impl FnOnce() -> T) -> (T, usize) {
    ALLOCATED_BYTES.with(|bytes| bytes.set(0));
    OBSERVE_ALLOCATIONS.with(|active| active.set(true));
    let value = f();
    OBSERVE_ALLOCATIONS.with(|active| active.set(false));
    (value, ALLOCATED_BYTES.with(std::cell::Cell::get))
}
#[test]
fn decode_retained_budget_rejection_precedes_accounting_allocations() {
    use tessera::error::AppError;
    let retained = Dataset::new(vec![]).unwrap();
    let control = JobControl::default();
    let (result, bytes) =
        allocations(|| decode_with_budget(&b""[..], &control, &[&retained], MAX_OWNED_BYTES - 1));
    assert!(matches!(result, Err(AppError::SizeLimit { .. })));
    assert_eq!(bytes, 0, "decode allocated before retained-work rejection");
}
#[test]
fn encode_retained_budget_rejection_precedes_accounting_allocations() {
    use tessera::error::AppError;
    let retained = Dataset::new(vec![]).unwrap();
    let control = JobControl::default();
    let project = Project::new("Empty", Dataset::new(vec![]).unwrap()).unwrap();
    // Leave exactly one byte after the encoder's project, validation and reference reserves.
    let extra = MAX_OWNED_BYTES
        - 1
        - project.owned_bytes().unwrap()
        - std::mem::size_of::<Project>()
        - 4096
        - 2 * std::mem::size_of::<&Dataset>();
    let (result, bytes) = allocations(|| {
        encode_with_budget(&project, std::io::sink(), &control, &[&retained], extra)
    });
    assert!(matches!(result, Err(AppError::SizeLimit { .. })));
    assert_eq!(bytes, 0, "encode allocated before retained-work rejection");
}

#[test]
fn final_large_numeric_history_roundtrips_with_tiny_retained_workspace() {
    let columns = (2..10)
        .map(|id| {
            Column::numeric(
                ColumnId(id),
                &format!("Value {id}"),
                (0..250000).map(|row| Some((row % 101) as f64)).collect(),
            )
            .unwrap()
        })
        .collect();
    let dataset = Dataset::new(columns)
        .unwrap()
        .edit_cell(ColumnId(2), 0, Cell::Number(42.0))
        .unwrap();
    let mut project = Project::new("Large history", dataset).unwrap();
    for columns in [
        vec![ColumnId(2)],
        vec![ColumnId(2)],
        (2..10).map(ColumnId).collect(),
    ] {
        let request = DescribeRequest {
            columns,
            statistics: vec![Statistic::Mean, Statistic::SampleStdDev],
            missing_policy: MissingPolicy::PerVariable,
            filter: RowFilter::default(),
            frequency_weight: None,
        };
        let run = describe(project.dataset(), &request, &JobControl::default()).unwrap();
        project = project.with_run(run).unwrap();
    }
    let path = std::env::temp_dir().canonicalize().unwrap().join(format!(
        "tessera-final-numeric-{}.tessera",
        std::process::id()
    ));
    let _ = std::fs::remove_file(&path);
    let control = JobControl::default();
    let outcome = tessera::files::save_project_with_budget(
        &path,
        &project,
        None,
        &control,
        &[],
        4 * 1024 * 1024,
    )
    .unwrap();
    assert_eq!(outcome.publication, tessera::files::Publication::Committed);
    let tiny = with_run();
    let reopened = tessera::files::read_project_with_budget(
        &path,
        &control,
        &[tiny.dataset()],
        tiny.owned_bytes().unwrap() + 4 * 1024 * 1024,
    )
    .unwrap()
    .0;
    std::fs::remove_file(path).unwrap();
    assert_eq!(reopened.dataset().row_count(), 250000);
    assert_eq!(reopened.dataset().revision(), project.dataset().revision());
    assert_eq!(reopened.runs(), project.runs());
    assert_eq!(
        reopened.dataset().cell(ColumnId(2), 0).unwrap(),
        Cell::Number(42.0)
    );
}
#[test]
fn final_large_ascii_and_escaped_text_roundtrip() {
    for (unit, count) in [("a", 50 * 1024 * 1024), ("\"\n\\", 8 * 1024 * 1024)] {
        let text = unit.repeat(count);
        let project = Project::new(
            "Text",
            Dataset::new(vec![
                Column::text(ColumnId(1), "Text", vec![Some(text)]).unwrap()
            ])
            .unwrap(),
        )
        .unwrap();
        let encoded = bytes(&project);
        let reopened = read(&encoded).unwrap();
        assert_eq!(
            reopened
                .dataset()
                .column(ColumnId(1))
                .unwrap()
                .text_value(0)
                .unwrap(),
            project
                .dataset()
                .column(ColumnId(1))
                .unwrap()
                .text_value(0)
                .unwrap()
        );
    }
}

#[path = "project_io/errors.rs"]
mod error_allocation_tests;

#[path = "project_io/versioned.rs"]
mod versioned;

#[path = "project_io/regression.rs"]
mod regression;

#[path = "project_io/anova.rs"]
mod anova;

#[path = "project_io/reliability.rs"]
mod reliability;
