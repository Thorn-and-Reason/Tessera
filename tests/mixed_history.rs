use tessera::{
    analysis::*,
    data::*,
    operation::JobControl,
    project::Project,
    project_io::{decode, encode},
};
fn dataset() -> Dataset {
    Dataset::new(vec![
        Column::numeric(
            ColumnId(1),
            "First",
            vec![Some(1.), Some(2.), Some(4.), Some(5.)],
        )
        .unwrap(),
        Column::numeric(
            ColumnId(2),
            "Second",
            vec![Some(2.), Some(1.), Some(3.), Some(7.)],
        )
        .unwrap(),
        Column::text(
            ColumnId(3),
            "Group",
            vec![
                Some("<A>".into()),
                Some("<A>".into()),
                Some("B&".into()),
                Some("B&".into()),
            ],
        )
        .unwrap(),
    ])
    .unwrap()
}
fn requests() -> Vec<AnalysisRequest> {
    vec![
        DescribeRequest {
            columns: vec![ColumnId(1)],
            statistics: vec![Statistic::Mean],
            missing_policy: MissingPolicy::PerVariable,
            filter: RowFilter::default(),
            frequency_weight: None,
        }
        .into(),
        AnalysisRequest::Pearson(PearsonRequest {
            columns: vec![ColumnId(1), ColumnId(2)],
            missing_policy: CorrelationMissingPolicy::PairwiseComplete,
            filter: RowFilter::default(),
            frequency_weight: None,
        }),
        AnalysisRequest::OneSampleT(OneSampleTRequest {
            variable: ColumnId(1),
            reference_mean: 2.,
            confidence: ConfidenceLevel::P90,
            filter: RowFilter::default(),
            frequency_weight: None,
        }),
        AnalysisRequest::PairedT(PairedTRequest {
            first: ColumnId(1),
            second: ColumnId(2),
            confidence: ConfidenceLevel::P99,
            filter: RowFilter::default(),
            frequency_weight: None,
        }),
        AnalysisRequest::IndependentT(IndependentTRequest {
            outcome: ColumnId(1),
            group: ColumnId(3),
            groups: [Cell::Text("<A>".into()), Cell::Text("B&".into())],
            method: IndependentMethod::Welch,
            confidence: ConfidenceLevel::P95,
            filter: RowFilter::default(),
            frequency_weight: None,
        }),
    ]
}
fn encoded(project: &Project) -> String {
    let mut out = vec![];
    encode(project, &mut out, &JobControl::default()).unwrap();
    String::from_utf8(out).unwrap()
}
fn mixed() -> Project {
    let mut p = Project::new("Mixed", dataset()).unwrap();
    for request in requests() {
        let result = analyze_with_budget(
            p.dataset(),
            &request,
            &JobControl::default(),
            &[],
            request.heap_bytes().unwrap(),
        )
        .unwrap();
        p = p.with_run(result).unwrap();
    }
    p
}
#[test]
fn descriptions_and_mixed_tagged_runs_roundtrip() {
    let p = mixed();
    let first = Project::from_parts(
        "Mixed",
        dataset(),
        vec![p.runs()[0].clone()],
        tessera::operation::RunId(2),
    )
    .unwrap();
    assert!(encoded(&first).contains("\"version\":3"));
    let wire = encoded(&p);
    assert!(wire.contains("\"version\":3"));
    let opened = decode(wire.as_bytes(), &JobControl::default()).unwrap();
    assert_eq!(opened.runs(), p.runs());
    assert_eq!(encoded(&opened), wire);
}
#[test]
fn historical_snapshots_survive_type_change() {
    let p = mixed();
    let d = p
        .dataset()
        .convert_column_with_budget(
            ColumnId(1),
            ColumnKind::Text,
            &JobControl::default(),
            &[],
            p.owned_bytes().unwrap(),
        )
        .unwrap();
    let p = p.with_dataset(d).unwrap();
    let opened = decode(encoded(&p).as_bytes(), &JobControl::default()).unwrap();
    assert_eq!(opened.runs(), p.runs());
    for run in opened.runs() {
        assert!(run
            .analysis
            .request()
            .clone_owned()
            .validate_current(opened.dataset(), &[], opened.owned_bytes().unwrap())
            .is_err());
    }
}
#[test]
fn header_order_duplicates_and_tag_disagreement() {
    let wire = encoded(&mixed());
    let late = wire.replacen("\"version\":3,", "", 1).replacen(
        "\"provenance\":[]",
        "\"provenance\":[],\"version\":3",
        1,
    );
    assert!(decode(late.as_bytes(), &JobControl::default()).is_ok());
    for bad in [
        wire.replace("\"version\":3", "\"version\":0"),
        wire.replace("\"version\":3", "\"version\":3,\"version\":3"),
        wire.replacen("\"pearson\":{", "\"unknown\":{", 1),
        wire.replace(",\"frequency_weight\":null", ""),
    ] {
        assert!(decode(bad.as_bytes(), &JobControl::default()).is_err());
    }
}
#[test]
fn new_tags_fail_before_unbounded_formatting() {
    let wire = encoded(&mixed());
    let bad = wire.replacen(
        "\"pearson\":{",
        &format!("\"{}\":{{", "x".repeat(100000)),
        1,
    );
    let err = decode(bad.as_bytes(), &JobControl::default()).unwrap_err();
    assert!(err.to_string().len() < 256);
}

#[test]
fn prepared_mixed_save_readback_refusal_preserves_bytes() {
    use tessera::{
        error::AppError,
        files::{save_project_with_budget, stamp},
        project_io::encode_with_budget,
    };
    let p = mixed();
    let folder = std::env::temp_dir()
        .canonicalize()
        .unwrap()
        .join(format!("tessera-mixed-save-{}", std::process::id()));
    std::fs::create_dir_all(&folder).unwrap();
    let path = folder.join("mixed.tessera");
    std::fs::write(&path, b"prior complete version").unwrap();
    let expected = stamp(&path).unwrap();
    // Encoding fits with 128 KiB live headroom; a 256 KiB provenance string makes
    // complete prepared decode exceed it. The source already owns that string.
    let p = p.with_provenance(vec!["p".repeat(256 * 1024)]).unwrap();
    let baseline = Budget::dataset_bytes(&[p.dataset()], p.owned_bytes().unwrap()).unwrap();
    let file_work = 256 * 1024;
    let extra = MAX_OWNED_BYTES - baseline - file_work - 128 * 1024;
    encode_with_budget(
        &p,
        std::io::sink(),
        &JobControl::default(),
        &[],
        extra + file_work + std::mem::size_of::<&Dataset>(),
    )
    .unwrap();
    let encoded_length = encoded(&p).len();
    let (sender, receiver) = std::sync::mpsc::channel();
    let control = JobControl::new(
        std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false)),
        Some(sender),
    );
    assert!(matches!(
        save_project_with_budget(&path, &p, Some(&expected), &control, &[], extra),
        Err(AppError::SizeLimit { .. })
    ));
    let progress = receiver.try_iter().collect::<Vec<_>>();
    let completed = progress
        .iter()
        .position(|p| p.done == encoded_length && p.total == Some(encoded_length))
        .expect("prepared encoding completed before refusal");
    assert!(
        progress[completed + 1..]
            .iter()
            .any(|p| p.total.is_none() && p.done > 0 && p.done < encoded_length),
        "readback began before its input-growth refusal"
    );
    assert_eq!(std::fs::read(&path).unwrap(), b"prior complete version");
    assert!(std::fs::read_dir(&folder).unwrap().all(|entry| matches!(
        entry.unwrap().file_name().to_str(),
        Some("mixed.tessera" | ".tessera-publication.lock")
    )));
    std::fs::remove_dir_all(folder).unwrap();
}
#[test]
fn mixed_report_contains_each_method_and_escaped_groups() {
    let p = mixed();
    let path = std::env::temp_dir()
        .canonicalize()
        .unwrap()
        .join(format!("tessera-mixed-report-{}.html", std::process::id()));
    let _ = std::fs::remove_file(&path);
    let selected = p.runs().iter().map(|r| r.id).collect::<Vec<_>>();
    tessera::export::export_report(&path, &p, &selected, &JobControl::default()).unwrap();
    let report = std::fs::read_to_string(&path).unwrap();
    for phrase in [
        "Pearson correlation",
        "zero population correlation",
        "independent observations",
        "bivariate normal",
        "linear association",
        "unadjusted across pairs",
        "One-sample t-test",
        "Paired t-test",
        "Independent t-test",
        "Welch",
        "&lt;A&gt;",
        "B&amp;",
        "90%",
        "99%",
        "95%",
        "First group minus Second group",
        "SD of paired differences",
    ] {
        assert!(report.contains(phrase), "missing {phrase}");
    }
    assert!(!report.contains("<A>"));
    std::fs::remove_file(path).unwrap();
}

#[test]
fn undisplayable_t_keeps_its_representable_log_tail_on_reopen() {
    let data = Dataset::new(vec![Column::numeric(
        ColumnId(1),
        "Value",
        vec![Some(0.01), Some(0.03)],
    )
    .unwrap()])
    .unwrap();
    let request = AnalysisRequest::OneSampleT(OneSampleTRequest {
        variable: ColumnId(1),
        reference_mean: 1e308,
        confidence: ConfidenceLevel::P95,
        filter: RowFilter::default(),
        frequency_weight: None,
    });
    let result = analyze_with_budget(
        &data,
        &request,
        &JobControl::default(),
        &[],
        request.heap_bytes().unwrap(),
    )
    .unwrap();
    let CompletedAnalysis::OneSampleT(r) = &result else {
        panic!()
    };
    assert!(r.statistics.t.value.is_none());
    assert!(r.statistics.p.value.unwrap() > 0.);
    let p = Project::new("Log tail", data)
        .unwrap()
        .with_run(result)
        .unwrap();
    let opened = decode(encoded(&p).as_bytes(), &JobControl::default()).unwrap();
    assert_eq!(opened.runs(), p.runs());
    #[cfg(feature = "desktop")]
    {
        let draft = tessera::desktop::draft::AnalysisDraft::from_request(request.clone());
        assert!(draft.matches_saved(request.borrowed()));
        assert_eq!(draft.build_request(opened.dataset()).unwrap(), request);
    }
}
#[test]
fn absent_current_groups_keep_historical_choices_and_rerun_as_empty_groups() {
    let p = mixed();
    let mut d = p.dataset().clone();
    for row in 0..2 {
        d = d
            .edit_cell(ColumnId(3), row, Cell::Text("Other".into()))
            .unwrap();
    }
    let p = p.with_dataset(d).unwrap();
    let opened = decode(encoded(&p).as_bytes(), &JobControl::default()).unwrap();
    assert_eq!(opened.runs(), p.runs());
    let request = opened.runs()[4].analysis.request().clone_owned();
    let result = analyze_with_budget(
        opened.dataset(),
        &request,
        &JobControl::default(),
        &[],
        opened.owned_bytes().unwrap() + request.heap_bytes().unwrap(),
    )
    .unwrap();
    let CompletedAnalysis::IndependentT(v) = result else {
        panic!()
    };
    assert_eq!(
        v.request.groups,
        [Cell::Text("<A>".into()), Cell::Text("B&".into())]
    );
    assert_eq!(v.groups[0].n, 0);
    assert!(v.groups[0].mean.value.is_none());
    assert_eq!(v.exclusions.other_group_rows, 2);
}

#[test]
fn complete_cases_pooled_numeric_groups_and_nullable_settings_roundtrip() {
    let mut requests = requests();
    if let AnalysisRequest::Pearson(q) = &mut requests[1] {
        q.missing_policy = CorrelationMissingPolicy::CompleteCases;
        q.frequency_weight = Some(ColumnId(1));
        q.filter.predicates.push(RowPredicate::Number {
            column: ColumnId(2),
            comparison: NumericComparison::Greater,
            value: 0.,
        });
    }
    if let AnalysisRequest::IndependentT(q) = &mut requests[4] {
        q.method = IndependentMethod::Pooled;
        q.group = ColumnId(2);
        q.groups = [Cell::Number(2.), Cell::Number(1.)];
        q.frequency_weight = Some(ColumnId(1));
    }
    let mut p = Project::new("Additional settings", dataset()).unwrap();
    for q in requests {
        let r = analyze_with_budget(
            p.dataset(),
            &q,
            &JobControl::default(),
            &[],
            p.owned_bytes().unwrap() + q.heap_bytes().unwrap(),
        )
        .unwrap();
        p = p.with_run(r).unwrap();
    }
    let wire = encoded(&p);
    assert!(wire.contains("\"method\":\"pooled\""));
    assert!(wire.contains("\"missing_policy\":\"complete_cases\""));
    assert_eq!(
        decode(wire.as_bytes(), &JobControl::default())
            .unwrap()
            .runs(),
        p.runs()
    );
}
#[test]
fn opening_describe_only_preserves_current_tagged_contract() {
    let p = mixed();
    let mut wire: serde_json::Value = serde_json::from_str(&encoded(&p)).unwrap();
    wire["runs"].as_array_mut().unwrap().truncate(1);
    wire["next_run_id"] = serde_json::json!("2");
    let opened = decode(
        &serde_json::to_vec(&wire).unwrap()[..],
        &JobControl::default(),
    )
    .unwrap();
    let expected = Project::from_parts(
        "Mixed",
        dataset(),
        vec![p.runs()[0].clone()],
        tessera::operation::RunId(2),
    )
    .unwrap();
    assert_eq!(encoded(&opened), encoded(&expected));
    assert!(encoded(&opened).contains("\"version\":3"));
}

#[test]
fn whitespace_groups_survive_report_presentation() {
    for groups in [["", " "], [" ", "  "], ["A", "A "], ["A\t", "A\n"]] {
        let data = Dataset::new(vec![
            Column::numeric(
                ColumnId(1),
                "Score",
                vec![Some(1.), Some(2.), Some(3.), Some(4.)],
            )
            .unwrap(),
            Column::text(
                ColumnId(2),
                "Group",
                vec![
                    Some(groups[0].into()),
                    Some(groups[0].into()),
                    Some(groups[1].into()),
                    Some(groups[1].into()),
                ],
            )
            .unwrap(),
        ])
        .unwrap();
        let request = AnalysisRequest::IndependentT(IndependentTRequest {
            outcome: ColumnId(1),
            group: ColumnId(2),
            groups: groups.map(|g| Cell::Text(g.into())),
            method: IndependentMethod::Welch,
            confidence: ConfidenceLevel::P95,
            filter: RowFilter::default(),
            frequency_weight: None,
        });
        let result =
            analyze_with_budget(&data, &request, &JobControl::default(), &[], 4096).unwrap();
        let p = Project::new("Whitespace", data)
            .unwrap()
            .with_run(result)
            .unwrap();
        let path = std::env::temp_dir()
            .canonicalize()
            .unwrap()
            .join(format!("tessera-whitespace-{}.html", std::process::id()));
        let _ = std::fs::remove_file(&path);
        tessera::export::export_report(&path, &p, &[p.runs()[0].id], &JobControl::default())
            .unwrap();
        let report = std::fs::read_to_string(&path).unwrap();
        assert!(report.contains("white-space:pre-wrap"));
        for group in groups {
            let quoted = format!("{group:?}").replace('"', "&quot;");
            assert!(report.contains(&quoted), "missing {quoted}");
        }
        std::fs::remove_file(path).unwrap();
    }
}
