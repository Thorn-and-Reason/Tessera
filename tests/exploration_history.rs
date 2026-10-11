use tessera::{
    analysis::*,
    data::*,
    operation::{JobControl, RunId},
    project::Project,
    project_io::{decode, encode},
};
fn mixed() -> Project {
    let c = JobControl::default();
    let mut p = decode(
        &include_bytes!("../examples/mixed-analysis.tessera")[..],
        &c,
    )
    .unwrap();
    for r in requests() {
        let a = analyze_with_budget(
            p.dataset(),
            &r,
            &c,
            &[],
            p.owned_bytes().unwrap() + r.heap_bytes().unwrap(),
        )
        .unwrap();
        p = p.with_run(a).unwrap();
    }
    p
}
fn requests() -> Vec<AnalysisRequest> {
    vec![
        AnalysisRequest::Frequencies(FrequenciesRequest {
            columns: vec![ColumnId(1), ColumnId(3)],
            filter: RowFilter::default(),
            frequency_weight: None,
        }),
        AnalysisRequest::Histogram(HistogramRequest {
            variable: ColumnId(1),
            bins: HistogramBins::Count(3),
            filter: RowFilter::default(),
            frequency_weight: None,
        }),
        AnalysisRequest::Scatter(ScatterRequest {
            x: ColumnId(1),
            y: ColumnId(2),
            filter: RowFilter::default(),
            frequency_weight: None,
        }),
        AnalysisRequest::Boxplot(BoxplotRequest {
            outcome: ColumnId(1),
            group: Some(ColumnId(3)),
            filter: RowFilter::default(),
            frequency_weight: None,
        }),
    ]
}
fn encoded(p: &Project) -> String {
    let mut bytes = vec![];
    encode(p, &mut bytes, &JobControl::default()).unwrap();
    String::from_utf8(bytes).unwrap()
}
#[test]
fn whole_graph_v3_passive_roundtrip_rerun_and_current_type_refusal() {
    let p = mixed();
    assert_eq!(p.runs().len(), 9);
    let wire = encoded(&p);
    assert!(wire.contains("\"version\":3"));
    let reopened = decode(wire.as_bytes(), &JobControl::default()).unwrap();
    assert_eq!(p.runs(), reopened.runs());
    assert_eq!(wire, encoded(&reopened));
    let edited = p
        .dataset()
        .convert_column_with_budget(
            ColumnId(1),
            ColumnKind::Text,
            &JobControl::default(),
            &[],
            p.owned_bytes().unwrap(),
        )
        .unwrap();
    let edited = p.with_dataset(edited).unwrap();
    assert_eq!(edited.runs(), p.runs());
    let reopened = decode(encoded(&edited).as_bytes(), &JobControl::default()).unwrap();
    assert_eq!(reopened.runs(), p.runs());
    for run in &reopened.runs()[6..] {
        assert!(run
            .analysis
            .request()
            .clone_owned()
            .validate_current(reopened.dataset(), &[], reopened.owned_bytes().unwrap())
            .is_err());
    }
    let request = p.runs()[8].analysis.request().clone_owned();
    let result = analyze_with_budget(
        p.dataset(),
        &request,
        &JobControl::default(),
        &[],
        p.owned_bytes().unwrap(),
    )
    .unwrap();
    let rerun = p.with_run(result).unwrap();
    assert_eq!(&rerun.runs()[..9], p.runs());
    assert_eq!(rerun.runs()[9].analysis, p.runs()[8].analysis);
}
#[test]
fn strict_versions_tags_fields_and_semantic_payloads() {
    let wire = encoded(&mixed());
    let late = wire.replacen("\"version\":3,", "", 1).replacen(
        "\"provenance\":",
        "\"version\":3,\"provenance\":",
        1,
    );
    assert!(decode(late.as_bytes(), &JobControl::default()).is_ok());
    for bad in [
        wire.replace("\"version\":3", "\"version\":4"),
        wire.replace("\"version\":3", "\"version\":0"),
        wire.replace("\"version\":3", "\"version\":3,\"version\":3"),
        wire.replace("\"source_rows\":", "\"unknown\":1,\"source_rows\":"),
        wire.replace("\"reduced\":false", "\"reduced\":false,\"reduced\":false"),
    ] {
        assert!(decode(bad.as_bytes(), &JobControl::default()).is_err());
    }
    let json: serde_json::Value = serde_json::from_str(&wire).unwrap();
    for (run, path, value) in [
        (
            5,
            "/frequencies/tables/0/categories/0/frequency",
            serde_json::json!(0),
        ),
        (6, "/histogram/bins/0/right", serde_json::json!(-999)),
        (7, "/scatter/points/0/x", serde_json::json!(1e200)),
        (8, "/boxplot/groups/0/q1/value", serde_json::json!(1e200)),
    ] {
        let mut bad = json.clone();
        *bad["runs"][run]["analysis"].pointer_mut(path).unwrap() = value;
        assert!(decode(
            &serde_json::to_vec(&bad).unwrap()[..],
            &JobControl::default()
        )
        .is_err());
    }
}
#[test]
fn search_requires_a_stored_frequency_run_and_requested_table() {
    let p = mixed();
    let request = FrequencySearchRequest {
        run: RunId(6),
        column: ColumnId(1),
        query: String::new(),
        offset: 0,
    };
    assert!(p
        .frequency_rows_with_budget(&request, &JobControl::default(), &[], 0)
        .is_ok());
    for run in [RunId(1), RunId(7), RunId(999)] {
        assert!(p
            .frequency_rows_with_budget(
                &FrequencySearchRequest {
                    run,
                    ..request.clone()
                },
                &JobControl::default(),
                &[],
                0
            )
            .is_err());
    }
    assert!(p
        .frequency_rows_with_budget(
            &FrequencySearchRequest {
                column: ColumnId(2),
                ..request
            },
            &JobControl::default(),
            &[],
            0
        )
        .is_err());
}
#[test]
fn report_preserves_nonordered_whiskers_and_unavailable_subnormal_whiskers() {
    for (case, values) in [
        ("ordinary", vec![0., 1., 2., 100.]),
        ("subnormal", vec![-f64::from_bits(1), f64::from_bits(1)]),
    ] {
        let data = Dataset::new(vec![Column::numeric(
            ColumnId(1),
            "Value <script>",
            values.into_iter().map(Some).collect(),
        )
        .unwrap()])
        .unwrap();
        let r = AnalysisRequest::Boxplot(BoxplotRequest {
            outcome: ColumnId(1),
            group: None,
            filter: RowFilter::default(),
            frequency_weight: None,
        });
        let result = analyze_with_budget(&data, &r, &JobControl::default(), &[], 4096).unwrap();
        let p = Project::new("Saved <&> output", data)
            .unwrap()
            .with_run(result)
            .unwrap();
        let p = decode(encoded(&p).as_bytes(), &JobControl::default()).unwrap();
        let CompletedAnalysis::Boxplot(v) = &p.runs()[0].analysis else {
            panic!()
        };
        if case == "ordinary" {
            assert_eq!(v.groups[0].q3.value, Some(26.5));
            assert_eq!(v.groups[0].whiskers[1].value, Some(2.));
        } else {
            assert!(v.groups[0]
                .whiskers
                .iter()
                .all(|e| e.value.is_none() && e.reason.is_some()));
        }
        let path = std::env::temp_dir()
            .canonicalize()
            .unwrap()
            .join(format!("tessera-v3-{case}-{}.html", std::process::id()));
        let _ = std::fs::remove_file(&path);
        tessera::export::export_report(&path, &p, &[RunId(1)], &JobControl::default()).unwrap();
        let html = std::fs::read_to_string(&path).unwrap();
        assert!(html.contains("&lt;script&gt;"));
        assert!(html.contains("<svg") && html.contains("<title") && html.contains("<table"));
        if case == "ordinary" {
            assert!(html.contains("26.5"));
        } else {
            assert!(html.contains(v.groups[0].whiskers[0].reason.as_ref().unwrap()));
        }
        std::fs::remove_file(path).unwrap();
    }
}
#[cfg(feature = "desktop")]
#[test]
fn rendered_captured_tables_and_checked_incomplete_forms_are_usable() {
    use eframe::egui;
    use tessera::{
        desktop::{
            draft::{AnalysisDraft, Procedure},
            render, DesktopState, UiAction, Workspace,
        },
        jobs::JobRunner,
    };
    let mut state = DesktopState::from_project(mixed()).unwrap();
    assert_eq!(Procedure::ALL.len(), 12);
    let ctx = egui::Context::default();
    ctx.enable_accesskit();
    tessera::desktop::configure_context(&ctx);
    let mut jobs = JobRunner::default();
    for i in 5..9 {
        let run = state.project().runs()[i].clone();
        let request = state.checked_run_request(run.id).unwrap();
        let draft = AnalysisDraft::from_request(request.clone());
        assert!(draft.matches_saved(run.analysis.request()));
        assert_eq!(
            draft.build_request(state.project().dataset()).unwrap(),
            request
        );
        let empty = draft.for_procedure(draft.procedure());
        assert!(empty.build_request(state.project().dataset()).is_err());
        state = state.apply_action(UiAction::SetDraft(draft)).unwrap();
        for workspace in [Workspace::Analyses, Workspace::Results] {
            state = state.apply_action(UiAction::SelectRun(run.id)).unwrap();
            state = state.apply_action(UiAction::Navigate(workspace)).unwrap();
            let input = egui::RawInput {
                screen_rect: Some(egui::Rect::from_min_size(
                    egui::Pos2::ZERO,
                    egui::vec2(1200., 850.),
                )),
                ..Default::default()
            };
            let mut output = ctx.run_ui(input, |ui| render(ui, &mut state, &mut jobs));
            output.textures_delta.clear();
            assert!(!output.shapes.is_empty());
            let labels = output
                .platform_output
                .accesskit_update
                .as_ref()
                .unwrap()
                .nodes
                .iter()
                .filter_map(|(_, n)| n.label().or_else(|| n.value()))
                .collect::<Vec<_>>();
            let expected = if workspace == Workspace::Analyses {
                ["Variables", "Variable", "X variable", "Outcome"][i - 5]
            } else {
                ["Frequencies", "Histogram", "Scatterplot", "Boxplot"][i - 5]
            };
            assert!(
                labels.iter().any(|s| s.contains(expected)),
                "missing {expected}: {labels:?}"
            );
            output.textures_delta.clear();
            assert!(state.error().is_none());
        }
    }
}

#[test]
fn prepared_v3_save_conflict_and_readback_refusal_preserve_destination() {
    use tessera::files::{save_project_with_budget, stamp, Publication};
    let p = mixed();
    let dir = std::env::temp_dir()
        .canonicalize()
        .unwrap()
        .join(format!("tessera-v3-save-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let path = dir.join("history.tessera");
    let _ = std::fs::remove_file(&path);
    assert_eq!(
        save_project_with_budget(&path, &p, None, &JobControl::default(), &[], 0)
            .unwrap()
            .publication,
        Publication::Committed
    );
    let original = std::fs::read(&path).unwrap();
    let expected = stamp(&path).unwrap();
    let large = p.with_provenance(vec!["p".repeat(256 * 1024)]).unwrap();
    let baseline = Budget::dataset_bytes(&[large.dataset()], large.owned_bytes().unwrap()).unwrap();
    let extra = MAX_OWNED_BYTES - baseline - 256 * 1024 - 128 * 1024;
    tessera::project_io::encode_with_budget(
        &large,
        std::io::sink(),
        &JobControl::default(),
        &[],
        extra + 256 * 1024 + std::mem::size_of::<&Dataset>(),
    )
    .unwrap();
    let length = encoded(&large).len();
    let (sender, receiver) = std::sync::mpsc::channel();
    let control = JobControl::new(
        std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false)),
        Some(sender),
    );
    assert!(matches!(
        save_project_with_budget(&path, &large, Some(&expected), &control, &[], extra),
        Err(tessera::error::AppError::SizeLimit { .. })
    ));
    let progress = receiver.try_iter().collect::<Vec<_>>();
    let completed = progress
        .iter()
        .position(|p| p.done == length && p.total == Some(length))
        .expect("prepared encoding completed");
    assert!(
        progress[completed + 1..]
            .iter()
            .any(|p| p.total.is_none() && p.done > 0 && p.done < length),
        "readback began before budget refusal"
    );
    assert_eq!(std::fs::read(&path).unwrap(), original);
    std::fs::write(&path, b"changed outside the application").unwrap();
    assert!(
        save_project_with_budget(&path, &p, Some(&expected), &JobControl::default(), &[], 0)
            .is_err()
    );
    assert_eq!(
        std::fs::read(&path).unwrap(),
        b"changed outside the application"
    );
    std::fs::remove_dir_all(dir).unwrap();
}
#[test]
fn complete_reports_escape_whitespace_labels_and_use_full_scatter_extrema() {
    let mut data = Dataset::new(vec![
        Column::numeric(
            ColumnId(1),
            "X <&>",
            (0..2001)
                .map(|i| Some(if i == 1999 { 1e100 } else { i as f64 }))
                .collect(),
        )
        .unwrap(),
        Column::numeric(
            ColumnId(2),
            "Y",
            (0..2001).map(|i| Some(i as f64)).collect(),
        )
        .unwrap(),
        Column::text(
            ColumnId(3),
            "Typed text",
            (0..2001)
                .map(|i| Some(["", " ", "A\t", "A\n", "<script> "][i % 5].into()))
                .collect(),
        )
        .unwrap(),
    ])
    .unwrap();
    let column = data.column(ColumnId(3)).unwrap();
    let meta = ColumnMetadata {
        value_labels: vec![ValueLabel {
            value: Cell::Text("<script> ".into()),
            label: "<svg onload=evil()>".into(),
        }],
        ..column.metadata().clone()
    };
    data = data
        .with_column_metadata_with_budget(ColumnId(3), meta, &[], 0)
        .unwrap();
    let mut p = Project::new("Title <&>", data).unwrap();
    for r in requests() {
        let result = analyze_with_budget(
            p.dataset(),
            &r,
            &JobControl::default(),
            &[],
            p.owned_bytes().unwrap() + r.heap_bytes().unwrap(),
        )
        .unwrap();
        p = p.with_run(result).unwrap();
    }
    let CompletedAnalysis::Scatter(v) = &p.runs()[2].analysis else {
        panic!()
    };
    assert!(!v.points.iter().any(|p| p.row == 1999));
    assert_eq!(v.ranges[0][1].value, Some(1e100));
    let path = std::env::temp_dir()
        .canonicalize()
        .unwrap()
        .join(format!("tessera-v3-report-{}.html", std::process::id()));
    let _ = std::fs::remove_file(&path);
    tessera::export::export_report(
        &path,
        &p,
        &[RunId(1), RunId(2), RunId(3), RunId(4)],
        &JobControl::default(),
    )
    .unwrap();
    let html = std::fs::read_to_string(&path).unwrap();
    for text in [
        "Title &lt;&amp;&gt;",
        "&quot;&quot;",
        "&quot; &quot;",
        "A\\t",
        "A\\n",
        "&lt;script&gt; ",
        "&lt;svg onload=evil()&gt;",
        "Complete frequency table",
        "Complete histogram bins",
        "Complete boxplot group summaries",
        "Showing 2000 of 2001",
    ] {
        assert!(html.contains(text), "missing {text}");
    }
    for run in p.runs() {
        for key in tessera::figures::figure_keys(run) {
            match tessera::figures::render::svg(
                run,
                &key,
                &p.figure_settings(&key),
                false,
                &JobControl::default(),
            ) {
                Ok(svg) => assert!(html.contains(&svg)),
                Err(tessera::error::AppError::InvalidInput { location, message })
                    if location == "figure" =>
                {
                    assert!(html.contains("Drawing unavailable"));
                    assert!(html.contains(&message));
                }
                Err(error) => panic!("Unexpected rendering error: {error}"),
            }
        }
    }
    assert!(!html.contains("<script>") && !html.contains("<svg onload"));
    let reopened = decode(encoded(&p).as_bytes(), &JobControl::default()).unwrap();
    assert_eq!(p.runs(), reopened.runs());
    std::fs::remove_file(path).unwrap();
}

#[test]
fn metadata_and_value_edits_preserve_snapshots_and_fixed_header() {
    let p = mixed();
    let column = p.dataset().column(ColumnId(1)).unwrap();
    let metadata = ColumnMetadata {
        name: "Edited variable".into(),
        description: "Edited description".into(),
        ..column.metadata().clone()
    };
    let data = p
        .dataset()
        .with_column_metadata_with_budget(ColumnId(1), metadata, &[], p.owned_bytes().unwrap())
        .unwrap();
    let data = data
        .edit_cell_with_budget(
            ColumnId(1),
            0,
            Cell::Number(-999.),
            &[],
            p.owned_bytes().unwrap(),
        )
        .unwrap();
    let edited = p.with_dataset(data).unwrap();
    assert_eq!(edited.runs(), p.runs());
    let reopened = decode(encoded(&edited).as_bytes(), &JobControl::default()).unwrap();
    assert_eq!(reopened.runs(), p.runs());
    assert!(reopened
        .runs()
        .iter()
        .all(|r| r.analysis.revision() != reopened.dataset().revision()));
    for count in [1, 5] {
        let mut json: serde_json::Value = serde_json::from_str(&encoded(&p)).unwrap();
        json["runs"].as_array_mut().unwrap().truncate(count);
        let opened = decode(
            &serde_json::to_vec(&json).unwrap()[..],
            &JobControl::default(),
        )
        .unwrap();
        assert!(encoded(&opened).contains(if count == 1 {
            "\"version\":3"
        } else {
            "\"version\":3"
        }));
    }
}
#[cfg(feature = "desktop")]
#[test]
fn unavailable_observed_whiskers_remain_accessible_in_passive_native_output() {
    use eframe::egui;
    use tessera::{
        desktop::{render, DesktopState, UiAction, Workspace},
        jobs::JobRunner,
    };
    let data = Dataset::new(vec![Column::numeric(
        ColumnId(1),
        "Subnormal outcome",
        vec![Some(-f64::from_bits(1)), Some(f64::from_bits(1))],
    )
    .unwrap()])
    .unwrap();
    let r = AnalysisRequest::Boxplot(BoxplotRequest {
        outcome: ColumnId(1),
        group: None,
        filter: RowFilter::default(),
        frequency_weight: None,
    });
    let result = analyze_with_budget(&data, &r, &JobControl::default(), &[], 4096).unwrap();
    let p = Project::new("Subnormal", data)
        .unwrap()
        .with_run(result)
        .unwrap();
    let p = decode(encoded(&p).as_bytes(), &JobControl::default()).unwrap();
    let CompletedAnalysis::Boxplot(v) = &p.runs()[0].analysis else {
        panic!()
    };
    let reason = v.groups[0].whiskers[0].reason.clone().unwrap();
    let mut state = DesktopState::from_project(p)
        .unwrap()
        .apply_action(UiAction::SelectRun(RunId(1)))
        .unwrap()
        .apply_action(UiAction::Navigate(Workspace::Results))
        .unwrap();
    let ctx = egui::Context::default();
    ctx.enable_accesskit();
    tessera::desktop::configure_context(&ctx);
    let mut output = ctx.run_ui(
        egui::RawInput {
            screen_rect: Some(egui::Rect::from_min_size(
                egui::Pos2::ZERO,
                egui::vec2(4096., 1000.),
            )),
            ..Default::default()
        },
        |ui| render(ui, &mut state, &mut JobRunner::default()),
    );
    output.textures_delta.clear();
    let labels = output
        .platform_output
        .accesskit_update
        .unwrap()
        .nodes
        .into_iter()
        .filter_map(|(_, n)| n.label().or_else(|| n.value()).map(str::to_owned))
        .collect::<Vec<_>>();
    for metric in ["Lower observed whisker", "Upper observed whisker"] {
        assert!(
            labels
                .iter()
                .any(|s| s.contains(metric) && s.contains(&reason)),
            "missing {metric}: {labels:?}"
        );
    }
    let previous = state.project().runs().to_vec();
    let bad = encoded(state.project()).replace("\"version\":3", "\"version\":4");
    let error = decode(bad.as_bytes(), &JobControl::default()).unwrap_err();
    assert!(state
        .handle_event(tessera::jobs::JobEvent::Completed {
            token: state.token(),
            cancelled: false,
            result: Err(error)
        })
        .is_err());
    assert_eq!(state.project().runs(), previous);
}
