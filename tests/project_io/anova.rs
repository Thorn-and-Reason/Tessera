use super::*;
#[test]
fn old_v5_header_rejects_passive_empty_history() {
    let p = Project::new("Empty", common::sample_dataset()).unwrap();
    let old = String::from_utf8(bytes(&p)).unwrap();
    let v5 = old.replacen("\"version\":3", "\"version\":5", 1);
    assert!(read(v5.as_bytes()).is_err());
}
#[path = "../common/anova.rs"]
mod fixture;
fn v5() -> serde_json::Value {
    serde_json::from_slice(&bytes(&fixture::captured(1))).unwrap()
}
#[test]
fn all_top_level_key_orders_enforce_v5_applicability() {
    let value = v5();
    let fields = value
        .as_object()
        .unwrap()
        .iter()
        .map(|(k, v)| (k.clone(), v.clone()))
        .collect::<Vec<_>>();
    for shift in 0..fields.len() {
        for reverse in [false, true] {
            let mut order = fields.clone();
            order.rotate_left(shift);
            if reverse {
                order.reverse();
            }
            for version in [0, 1, 2, 3, 4, 5, 6] {
                let json = format!(
                    "{{{}}}",
                    order
                        .iter()
                        .map(|(k, v)| format!(
                            "{}:{}",
                            serde_json::to_string(k).unwrap(),
                            if k == "version" {
                                version.to_string()
                            } else {
                                v.to_string()
                            }
                        ))
                        .collect::<Vec<_>>()
                        .join(",")
                );
                assert_eq!(read(json.as_bytes()).is_ok(), version == 3);
            }
        }
    }
}

#[test]
fn public_v5_decoder_rejects_header_shape_and_captured_corruption() {
    let baseline = v5();
    for version in [
        serde_json::json!(0),
        serde_json::json!(7),
        serde_json::json!("5"),
        serde_json::Value::Null,
    ] {
        let mut v = baseline.clone();
        v["version"] = version;
        assert!(read(&serde_json::to_vec(&v).unwrap()).is_err());
    }
    for key in ["version", "format"] {
        let mut v = baseline.clone();
        v.as_object_mut().unwrap().remove(key);
        assert!(read(&serde_json::to_vec(&v).unwrap()).is_err());
        let text = serde_json::to_string(&baseline).unwrap();
        let duplicate = format!(
            "{{\"{key}\":{},{}",
            serde_json::to_string(&baseline[key]).unwrap(),
            &text[1..]
        );
        assert!(read(duplicate.as_bytes()).is_err());
    }
    let mut v = baseline.clone();
    v["runs"][0]["analysis"]["anova"]["source_rows"] = serde_json::json!(0);
    assert!(read(&serde_json::to_vec(&v).unwrap()).is_err());
    let mut v = baseline.clone();
    v["runs"][0]["analysis"]["anova"]["request"]["all_pairs"][0]["unknown"] =
        serde_json::json!(true);
    assert!(read(&serde_json::to_vec(&v).unwrap()).is_err());
    let mut v = baseline.clone();
    v["runs"][0]["analysis"]["anova"]["extra"] = serde_json::json!(0);
    assert!(read(&serde_json::to_vec(&v).unwrap()).is_err());
    let text = serde_json::to_string(&baseline).unwrap();
    assert!(read(format!("{text} 0").as_bytes()).is_err());
    assert!(read(text[..text.len() - 1].as_bytes()).is_err());
}
#[test]
fn all_designs_real_public_v5_roundtrip_and_old_only_minimum_versions_stay_intact() {
    for design in 0..3 {
        let p = fixture::captured(design);
        let b = bytes(&p);
        assert_eq!(read(&b).unwrap().runs(), p.runs());
    }
    let p = with_run();
    assert_eq!(
        serde_json::from_slice::<serde_json::Value>(&bytes(&p)).unwrap()["version"],
        3
    );
}
#[test]
fn public_v5_rejects_recipe_mapping_families_and_model_inconsistencies() {
    let base = v5();
    let run = |payload: serde_json::Value| {
        let mut v = base.clone();
        v["runs"][0]["analysis"]["anova"] = payload;
        assert!(read(&serde_json::to_vec(&v).unwrap()).is_err());
    };
    let payload = base["runs"][0]["analysis"]["anova"].clone();
    for key in ["method", "columns", "diagnostics", "outcome", "request"] {
        let mut p = payload.clone();
        p.as_object_mut().unwrap().remove(key);
        run(p);
    }
    let mut p = payload.clone();
    p["request"]["planned_pairs"]
        .as_array_mut()
        .unwrap()
        .push(payload["request"]["planned_pairs"][0].clone());
    run(p);
    let mut p = payload.clone();
    let mut reversed = payload["request"]["planned_pairs"][0].clone();
    let first = reversed["first"].clone();
    reversed["first"] = reversed["second"].clone();
    reversed["second"] = first;
    p["request"]["planned_pairs"]
        .as_array_mut()
        .unwrap()
        .push(reversed);
    run(p);
    let mut p = payload.clone();
    p["request"]["all_pairs"]
        .as_array_mut()
        .unwrap()
        .push(payload["request"]["all_pairs"][0].clone());
    run(p);
    let mut p = payload.clone();
    p["families"][0]["m"] = serde_json::json!(999);
    run(p);
    let mut p = payload.clone();
    p["outcome"]["fitted"]["effects"][0]["test"]["df1"]["value"] = serde_json::json!(63);
    run(p);
    let repeated = fixture::captured(2);
    let mut v: serde_json::Value = serde_json::from_slice(&bytes(&repeated)).unwrap();
    v["runs"][0]["analysis"]["anova"]["request"]["frequency_weight"] = serde_json::json!("1");
    assert!(read(&serde_json::to_vec(&v).unwrap()).is_err());
    let mut v: serde_json::Value = serde_json::from_slice(&bytes(&repeated)).unwrap();
    v["runs"][0]["analysis"]["anova"]["request"]["design"]["repeated"]["measurements"][0] =
        serde_json::json!("6");
    assert!(read(&serde_json::to_vec(&v).unwrap()).is_err());
    let mut v: serde_json::Value = serde_json::from_slice(&bytes(&repeated)).unwrap();
    v["runs"][0]["analysis"]["anova"]["outcome"]["fitted"]["effects"][0]["gg"]["test"]["df2"]
        ["value"] = serde_json::json!(123);
    assert!(read(&serde_json::to_vec(&v).unwrap()).is_err());
}
