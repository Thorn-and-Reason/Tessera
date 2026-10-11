use super::*;
#[test]
fn current_header_accepts_passive_empty_history() {
    let project = Project::new("Empty", common::sample_dataset()).unwrap();
    let mut fixture: serde_json::Value = serde_json::from_slice(&bytes(&project)).unwrap();
    fixture["version"] = serde_json::json!(3);
    assert!(read(&serde_json::to_vec(&fixture).unwrap()).is_ok());
}
fn captured() -> Project {
    use tessera::analysis::reliability::*;
    let d = common::sample_dataset();
    let request = ReliabilityRequest {
        items: vec![ColumnId(3), ColumnId(4), ColumnId(6)],
        filter: Default::default(),
        frequency_weight: None,
        reversals: vec![ReliabilityReverse {
            column: ColumnId(3),
            lower: 0.,
            upper: 20.,
        }],
    };
    let extra = std::mem::size_of::<ReliabilityRequest>() + request.heap_bytes().unwrap();
    let run = reliability_with_budget(&d, &request, &JobControl::default(), &[], extra).unwrap();
    Project::new("Reliability", d)
        .unwrap()
        .with_run(tessera::analysis::CompletedAnalysis::Reliability(run))
        .unwrap()
}
#[test]
fn v6_real_capture_roundtrip_strict_tags_and_old_headers() {
    let project = captured();
    let encoded = bytes(&project);
    let fixture: serde_json::Value = serde_json::from_slice(&encoded).unwrap();
    for end in [0, encoded.len() / 2, encoded.len() - 1] {
        assert!(read(&encoded[..end]).is_err());
    }
    assert_eq!(fixture["version"], 3);
    assert_eq!(read(&encoded).unwrap().runs(), project.runs());
    for old in [0, 1, 2, 4, 5, 6] {
        let mut value = fixture.clone();
        value["version"] = serde_json::json!(old);
        assert!(read(&serde_json::to_vec(&value).unwrap()).is_err());
    }
    for (field, bad) in [
        (
            "method",
            serde_json::json!({"exact_centered_interval_v1":null}),
        ),
        ("method", serde_json::json!("future")),
        ("total_energy", serde_json::json!({"positive":null})),
        ("standardized_total_energy", serde_json::json!("future")),
    ] {
        let mut value = fixture.clone();
        value["runs"][0]["analysis"]["reliability"][field] = bad;
        assert!(read(&serde_json::to_vec(&value).unwrap()).is_err());
    }
    for path in [
        "/runs/0/analysis/reliability",
        "/runs/0/analysis/reliability/request",
        "/runs/0/analysis/reliability/request/reversals/0",
        "/runs/0/analysis/reliability/items/0",
    ] {
        let mut value = fixture.clone();
        value
            .pointer_mut(path)
            .unwrap()
            .as_object_mut()
            .unwrap()
            .insert("extra".into(), serde_json::json!(true));
        assert!(read(&serde_json::to_vec(&value).unwrap()).is_err());
        let mut value = fixture.clone();
        let object = value.pointer_mut(path).unwrap().as_object_mut().unwrap();
        let key = object.keys().next().unwrap().clone();
        object.remove(&key);
        assert!(read(&serde_json::to_vec(&value).unwrap()).is_err());
        let mut value = fixture.clone();
        *value.pointer_mut(path).unwrap() = serde_json::json!([]);
        assert!(read(&serde_json::to_vec(&value).unwrap()).is_err());
    }
    let duplicate = String::from_utf8(encoded.clone()).unwrap().replace(
        "\"method\":",
        "\"method\":\"exact_centered_interval_v1\",\"method\":",
    );
    assert!(read(duplicate.as_bytes()).is_err());
    std::fs::create_dir_all(".build/reliability-task5").unwrap();
    std::fs::write(".build/reliability-task5/wire-current.tessera", &encoded).unwrap();
}
#[test]
fn current_header_validation_is_independent_of_field_order() {
    let text = String::from_utf8(bytes(&captured())).unwrap();
    let v: serde_json::Value = serde_json::from_str(&text).unwrap();
    let mut fields = v
        .as_object()
        .unwrap()
        .iter()
        .map(|(k, v)| (k.clone(), v.clone()))
        .collect::<Vec<_>>();
    fields.sort_by_key(|(key, _)| key == "version");
    for version in 1..=6 {
        let document = format!(
            "{{{}}}",
            fields
                .iter()
                .map(|(key, value)| format!(
                    "{}:{}",
                    serde_json::to_string(key).unwrap(),
                    if key == "version" {
                        version.to_string()
                    } else {
                        value.to_string()
                    }
                ))
                .collect::<Vec<_>>()
                .join(",")
        );
        assert_eq!(read(document.as_bytes()).is_ok(), version == 3);
    }
    let project = Project::new("reliability", common::sample_dataset()).unwrap();
    assert_eq!(
        serde_json::from_slice::<serde_json::Value>(&bytes(&project)).unwrap()["version"],
        3
    );
    assert!(read(&bytes(&project)).is_ok());
}
#[test]
fn cancelled_v6_codec_precedes_input_and_owner_admission() {
    let project = captured();
    let control = JobControl::default();
    control.cancel();
    assert!(matches!(
        decode_with_budget(b"invalid".as_slice(), &control, &[], MAX_OWNED_BYTES),
        Err(tessera::error::AppError::Cancelled)
    ));
    assert!(matches!(
        encode_with_budget(&project, Vec::new(), &control, &[], MAX_OWNED_BYTES),
        Err(tessera::error::AppError::Cancelled)
    ));
}
#[path = "../common/anova.rs"]
mod anova_fixture;
#[path = "../common/regression.rs"]
mod regression_fixture;
#[test]
fn mixed_histories_keep_fixed_current_version() {
    let reliability = captured();
    let regression = regression_fixture::captured();
    let anova = anova_fixture::captured(1);
    let version = |project: &Project| {
        serde_json::from_slice::<serde_json::Value>(&bytes(project)).unwrap()["version"]
            .as_u64()
            .unwrap()
    };
    assert_eq!(version(&regression), 3);
    assert_eq!(version(&anova), 3);
    let mixed = regression
        .with_run(anova.runs()[0].analysis.clone())
        .unwrap();
    assert_eq!(version(&mixed), 3);
    let mixed = mixed
        .with_run(reliability.runs()[0].analysis.clone())
        .unwrap();
    assert_eq!(version(&mixed), 3);
    let reopened = read(&bytes(&mixed)).unwrap();
    assert_eq!(reopened.runs(), mixed.runs());
    assert_eq!(version(&reopened), 3);
}
