#[path = "../common/regression.rs"]
mod fixture;
use super::*;
use tessera::analysis::{AnalysisRequest, CompletedAnalysis, PredictorRole};

#[test]
fn v4_roundtrip_refuses_downgrade_in_every_header_order() {
    let p = fixture::captured();
    let encoded = bytes(&p);
    assert_eq!(read(&encoded).unwrap().runs(), p.runs());
    for version in [0, 1, 2, 4] {
        let mut value: serde_json::Value = serde_json::from_slice(&encoded).unwrap();
        value["version"] = version.into();
        let regular = serde_json::to_vec(&value).unwrap();
        assert!(read(&regular).is_err());
        let object = value.as_object_mut().unwrap();
        object.remove("version");
        let tail = serde_json::to_string(&value).unwrap();
        let last = format!("{},\"version\":{version}}}", &tail[..tail.len() - 1]);
        let first = format!("{{\"version\":{version},{}", &tail[1..]);
        assert!(read(last.as_bytes()).is_err());
        assert!(read(first.as_bytes()).is_err());
    }
}
#[test]
fn unavailable_history_and_stale_current_types_roundtrip_without_rerun() {
    let p = fixture::captured();
    let mut r = fixture::request();
    let AnalysisRequest::Regression(ref mut r) = r else {
        unreachable!()
    };
    r.predictors[1].role = PredictorRole::Categorical {
        reference: Cell::Text("absent".into()),
    };
    let failed = tessera::analysis::analyze_with_budget(
        p.dataset(),
        &AnalysisRequest::Regression(r.clone()),
        &JobControl::default(),
        &[],
        p.owned_bytes().unwrap(),
    )
    .unwrap();
    assert!(
        matches!(&failed, CompletedAnalysis::Regression(v) if matches!(v.outcome, tessera::analysis::RegressionOutcome::Unavailable(_)))
    );
    let p = p.with_run(failed).unwrap();
    let data = p
        .dataset()
        .convert_column_with_budget(
            ColumnId(1),
            tessera::data::ColumnKind::Text,
            &JobControl::default(),
            &[],
            p.owned_bytes().unwrap(),
        )
        .unwrap();
    let p = p.with_dataset(data).unwrap();
    let reopened = read(&bytes(&p)).unwrap();
    assert_eq!(reopened.runs(), p.runs());
    assert!(reopened.runs()[0]
        .analysis
        .request()
        .clone_owned()
        .validate_current(reopened.dataset(), &[], 0)
        .is_err());
}
#[test]
fn mixed_histories_keep_fixed_current_header() {
    for (source, expected) in [
        (
            &include_bytes!("../../examples/mixed-analysis.tessera")[..],
            3,
        ),
        (&include_bytes!("../../examples/exploration.tessera")[..], 3),
    ] {
        let p = read(source).unwrap();
        let original: serde_json::Value = serde_json::from_slice(&bytes(&p)).unwrap();
        assert_eq!(original["version"], expected);
        let regression = fixture::captured().runs()[0].analysis.clone();
        let mixed = p.with_run(regression).unwrap();
        let value: serde_json::Value = serde_json::from_slice(&bytes(&mixed)).unwrap();
        assert_eq!(value["version"], 3);
        assert_eq!(read(&bytes(&mixed)).unwrap().runs(), mixed.runs());
    }
    let p = super::with_run();
    let value: serde_json::Value = serde_json::from_slice(&bytes(&p)).unwrap();
    assert_eq!(value["version"], 3);
}
#[test]
fn v4_structural_mutations_are_refused_at_public_boundary() {
    let good: serde_json::Value = serde_json::from_slice(&bytes(&fixture::captured())).unwrap();
    for field in ["request", "method", "outcome", "columns", "terms", "levels"] {
        let mut bad = good.clone();
        bad["runs"][0]["analysis"]["regression"]
            .as_object_mut()
            .unwrap()
            .remove(field);
        assert!(read(&serde_json::to_vec(&bad).unwrap()).is_err(), "{field}");
    }
    let mut bad = good.clone();
    bad["runs"][0]["analysis"]["regression"]["unknown"] = true.into();
    assert!(read(&serde_json::to_vec(&bad).unwrap()).is_err());
    let mut bad = good;
    bad["runs"][0]["analysis"]["regression"]["outcome"]["fitted"]["statistics"]["sse"]["value"] =
        (-1.).into();
    assert!(read(&serde_json::to_vec(&bad).unwrap()).is_err());
}
