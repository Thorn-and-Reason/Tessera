use tessera::{
    data::Dataset,
    operation::JobControl,
    project::Project,
    project_io::{decode, encode},
};
#[test]
fn current_header_is_fixed_and_old_or_future_headers_are_rejected() {
    let project = Project::new("Current", Dataset::new(vec![]).unwrap()).unwrap();
    let mut bytes = Vec::new();
    encode(&project, &mut bytes, &JobControl::default()).unwrap();
    let current: serde_json::Value = serde_json::from_slice(&bytes).unwrap();
    assert_eq!(current["format"], "tessera-project");
    assert_eq!(current["version"], 3);
    assert!(decode(bytes.as_slice(), &JobControl::default()).is_ok());
    for (format, version) in [
        ("foreign-project", 1),
        ("foreign-project", 6),
        ("tessera-project", 0),
        ("tessera-project", 1),
        ("tessera-project", 2),
        ("tessera-project", 4),
        ("tessera-project", 999),
    ] {
        let mut invalid = current.clone();
        invalid["format"] = format.into();
        invalid["version"] = version.into();
        assert!(decode(
            serde_json::to_vec(&invalid).unwrap().as_slice(),
            &JobControl::default()
        )
        .is_err());
    }
}
#[test]
fn current_examples_preserve_tagged_captures_and_reject_untagged_describe() {
    for input in [
        include_bytes!("../examples/sample.tessera").as_slice(),
        include_bytes!("../examples/mixed-analysis.tessera").as_slice(),
        include_bytes!("../examples/exploration.tessera").as_slice(),
        include_bytes!("../examples/regression-analysis.tessera").as_slice(),
    ] {
        let project = decode(input, &JobControl::default()).unwrap();
        let mut output = Vec::new();
        encode(&project, &mut output, &JobControl::default()).unwrap();
        let saved: serde_json::Value = serde_json::from_slice(&output).unwrap();
        let reopened = decode(output.as_slice(), &JobControl::default()).unwrap();
        assert_eq!(reopened.runs(), project.runs());
        assert_eq!(reopened.provenance(), project.provenance());
        assert_eq!(reopened.dataset().revision(), project.dataset().revision());
        assert_eq!(
            reopened.dataset().row_count(),
            project.dataset().row_count()
        );
        for column in project.dataset().columns() {
            for row in 0..project.dataset().row_count() {
                assert_eq!(
                    reopened.dataset().cell(column.id(), row).unwrap(),
                    project.dataset().cell(column.id(), row).unwrap()
                );
            }
        }
        let mut repeated = Vec::new();
        encode(&reopened, &mut repeated, &JobControl::default()).unwrap();
        assert_eq!(output, repeated);
        assert_eq!(saved["format"], "tessera-project");
        assert_eq!(saved["version"], 3);
        for run in saved["runs"].as_array().unwrap() {
            assert!(run.get("operation").is_none());
            assert_eq!(run["analysis"].as_object().unwrap().len(), 1);
        }
        let mut untagged = saved.clone();
        if let Some(run) = untagged["runs"]
            .as_array_mut()
            .unwrap()
            .iter_mut()
            .find(|run| run["analysis"].get("describe").is_some())
        {
            run["operation"] = "describe".into();
            run["analysis"] = run["analysis"]["describe"].clone();
            assert!(decode(
                serde_json::to_vec(&untagged).unwrap().as_slice(),
                &JobControl::default()
            )
            .is_err());
        }
    }
}
