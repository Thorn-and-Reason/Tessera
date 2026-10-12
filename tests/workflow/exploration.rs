use super::*;
use tessera::analysis::*;
use tessera::desktop::draft::AnalysisDraft;

fn requests() -> Vec<AnalysisRequest> {
    vec![
        AnalysisRequest::Frequencies(FrequenciesRequest {
            group: None,
            columns: vec![ColumnId(1), ColumnId(2)],
            filter: RowFilter::default(),
            frequency_weight: None,
        }),
        AnalysisRequest::Histogram(HistogramRequest {
            variable: ColumnId(2),
            bins: HistogramBins::Count(3),
            filter: RowFilter::default(),
            frequency_weight: None,
        }),
        AnalysisRequest::Scatter(ScatterRequest {
            x: ColumnId(2),
            y: ColumnId(3),
            filter: RowFilter::default(),
            frequency_weight: None,
        }),
        AnalysisRequest::Boxplot(BoxplotRequest {
            outcome: ColumnId(2),
            group: Some(ColumnId(1)),
            filter: RowFilter::default(),
            frequency_weight: None,
        }),
    ]
}

#[test]
fn four_family_worker_save_reopen_rerun_and_selected_report() {
    let temp = Temp::new();
    let input = Arc::new(
        preview(
            Arc::from(&b"Group,X <&>,Y\nA,1,2\nA,2,4\n<script>,3,6\n<script>,4,8\n"[..]),
            Delimiter::Comma,
            &JobControl::default(),
        )
        .unwrap(),
    );
    let choices = input.default_choices().unwrap();
    let mut state = DesktopState::new().unwrap();
    let mut jobs = JobRunner::default();
    run(
        &mut state,
        &mut jobs,
        JobOperation::CommitImport {
            preview: input,
            choices,
        },
    );
    for request in requests() {
        let draft = AnalysisDraft::from_request(request.clone());
        assert_eq!(
            draft.build_request(state.project().dataset()).unwrap(),
            request
        );
        let project = Arc::clone(state.project());
        run(
            &mut state,
            &mut jobs,
            JobOperation::Analyze { project, request },
        );
    }
    let historical = state.project().runs().to_vec();
    let CompletedAnalysis::Frequencies(f) = &historical[0].analysis else {
        panic!()
    };
    assert_eq!(f.tables[0].valid_n, 4);
    assert_eq!(
        f.tables[0].categories[0].value,
        Cell::Text("<script>".into())
    );
    assert_eq!(f.tables[0].categories[0].valid_percent.value, Some(50.));
    let CompletedAnalysis::Histogram(h) = &historical[1].analysis else {
        panic!()
    };
    assert_eq!(
        h.bins.iter().map(|b| b.frequency).collect::<Vec<_>>(),
        [1, 1, 2]
    );
    let CompletedAnalysis::Scatter(s) = &historical[2].analysis else {
        panic!()
    };
    assert_eq!(s.ranges[1][1].value, Some(8.));
    assert_eq!(s.points[2].x, 3.);
    let CompletedAnalysis::Boxplot(b) = &historical[3].analysis else {
        panic!()
    };
    assert_eq!(b.groups[0].median.value, Some(3.5));
    state = state
        .apply_action(UiAction::EditCell {
            column: ColumnId(2),
            row: 0,
            value: Cell::Number(100.),
        })
        .unwrap();
    assert!(state.selected_run_is_stale());
    for old in &historical {
        let request = state.checked_run_request(old.id).unwrap();
        assert_eq!(request.borrowed(), old.analysis.request());
        let project = Arc::clone(state.project());
        run(
            &mut state,
            &mut jobs,
            JobOperation::Analyze { project, request },
        );
    }
    assert_eq!(state.project().runs().len(), 8);
    assert_eq!(&state.project().runs()[..4], historical);
    let saved = state.project().runs().to_vec();
    let path = temp.0.join("charts.tessera");
    let project = Arc::clone(state.project());
    run(
        &mut state,
        &mut jobs,
        JobOperation::Save {
            project,
            path: path.clone(),
            expected: None,
        },
    );
    assert!(!state.is_dirty());
    assert_eq!(
        serde_json::from_slice::<serde_json::Value>(&fs::read(&path).unwrap()).unwrap()["version"],
        3
    );
    let mut reopened = DesktopState::new().unwrap();
    run(
        &mut reopened,
        &mut jobs,
        JobOperation::Open { path: path.clone() },
    );
    assert_eq!(reopened.project().runs(), saved);
    let report = temp.0.join("selected.html");
    let project = Arc::clone(reopened.project());
    run(
        &mut reopened,
        &mut jobs,
        JobOperation::ExportReport {
            project,
            path: report.clone(),
            runs: historical.iter().map(|r| r.id).collect(),
        },
    );
    let html = fs::read_to_string(&report).unwrap();
    for text in [
        "Frequencies",
        "Histogram",
        "Scatterplot",
        "Boxplot",
        "X &lt;&amp;&gt;",
        "&lt;script&gt;",
    ] {
        assert!(html.contains(text), "missing {text}");
    }
    assert!(!html.contains("<script>"));
    for id in 1..=4 {
        assert!(html.contains(&format!("<h2 id=\"run-{id}\">")));
    }
    assert!(!html.contains("<h2 id=\"run-5\">"));
    assert_eq!(reopened.project().runs(), saved);
    let before = Arc::clone(reopened.project());
    for version in [0, 1, 2, 4] {
        let mut malformed: serde_json::Value =
            serde_json::from_slice(&fs::read(&path).unwrap()).unwrap();
        malformed["version"] = serde_json::json!(version);
        let bad_path = temp.0.join(format!("wrong-version-{version}.tessera"));
        fs::write(&bad_path, serde_json::to_vec(&malformed).unwrap()).unwrap();
        reopened
            .start_operation(&mut jobs, JobOperation::Open { path: bad_path })
            .unwrap();
        finish(&mut reopened, &mut jobs);
        assert!(reopened.error().is_some());
        assert!(Arc::ptr_eq(reopened.project(), &before));
    }
    for request in requests() {
        reopened
            .start_operation(
                &mut jobs,
                JobOperation::Analyze {
                    project: Arc::clone(&before),
                    request,
                },
            )
            .unwrap();
        jobs.cancel();
        finish(&mut reopened, &mut jobs);
        assert!(Arc::ptr_eq(reopened.project(), &before));
    }
    let cancelled_report = temp.0.join("cancelled.html");
    reopened
        .start_operation(
            &mut jobs,
            JobOperation::ExportReport {
                project: Arc::clone(&before),
                path: cancelled_report.clone(),
                runs: saved.iter().map(|r| r.id).collect(),
            },
        )
        .unwrap();
    jobs.cancel();
    finish(&mut reopened, &mut jobs);
    assert!(!cancelled_report.exists());
    assert!(Arc::ptr_eq(reopened.project(), &before));
    let expected = stamp(&path).unwrap();
    fs::write(&path, b"external edit").unwrap();
    reopened
        .start_operation(
            &mut jobs,
            JobOperation::Save {
                project: Arc::clone(&before),
                path: path.clone(),
                expected: Some(expected),
            },
        )
        .unwrap();
    finish(&mut reopened, &mut jobs);
    assert!(reopened.error().is_some());
    assert_eq!(fs::read(path).unwrap(), b"external edit");
    assert!(Arc::ptr_eq(reopened.project(), &before));
}

#[test]
fn expanded_mixed_history_protective_save_keeps_all_original_runs() {
    use tessera::{
        data::{Column, Dataset},
        project::Project,
        project_io,
    };
    let temp = Temp::new();
    let categories = ["", " ", "A", "A ", "A\t", "A\n", "<script>", "Cymraeg ŵ"];
    let rows = 12000;
    let mut columns = vec![
        Column::numeric(
            ColumnId(1),
            "Outcome <&> — long captured axis title",
            (0..rows).map(|i| Some((i % 101) as f64)).collect(),
        )
        .unwrap(),
        Column::numeric(
            ColumnId(2),
            "Second outcome — exact historical Y axis",
            (0..rows).map(|i| Some((i % 173) as f64 * 2.)).collect(),
        )
        .unwrap(),
        Column::text(
            ColumnId(3),
            "Exact category",
            (0..rows)
                .map(|i| Some(format!("Group {i:05} {} ", categories[i % 8])))
                .collect(),
        )
        .unwrap(),
    ];
    for i in 4..=64 {
        columns.push(
            Column::numeric(
                ColumnId(i),
                &format!("V{i}"),
                (0..rows)
                    .map(|r| Some(((r * i as usize) % 10007) as f64))
                    .collect(),
            )
            .unwrap(),
        );
    }
    let mut project = Project::new(
        "Native mixed-wide-categories <&>",
        Dataset::new(columns).unwrap(),
    )
    .unwrap();
    let requests = vec![
        AnalysisRequest::Describe(DescribeRequest {
            columns: vec![ColumnId(1)],
            statistics: vec![Statistic::Mean],
            missing_policy: MissingPolicy::PerVariable,
            filter: RowFilter::default(),
            frequency_weight: None,
        }),
        AnalysisRequest::Frequencies(FrequenciesRequest {
            group: None,
            columns: vec![ColumnId(3), ColumnId(1)],
            filter: RowFilter::default(),
            frequency_weight: None,
        }),
        AnalysisRequest::Histogram(HistogramRequest {
            variable: ColumnId(1),
            bins: HistogramBins::Count(7),
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
    ];
    for request in requests {
        let result = analyze_with_budget(
            project.dataset(),
            &request,
            &JobControl::default(),
            &[],
            project.owned_bytes().unwrap() + request.heap_bytes().unwrap(),
        )
        .unwrap();
        project = project.with_run(result).unwrap();
    }
    let path = temp.0.join("expanded.tessera");
    project_io::encode(
        &project,
        fs::File::create(&path).unwrap(),
        &JobControl::default(),
    )
    .unwrap();
    let serialized_bytes = fs::metadata(&path).unwrap().len();
    // Added captured means increase storage; this remains a large, bounded save fixture.
    assert!(
        (12_844_839..=tessera::project_io::MAX_PROJECT_BYTES as u64).contains(&serialized_bytes)
    );
    let old_runs = project.runs().to_vec();
    drop(project);
    let mut state = DesktopState::new().unwrap();
    let mut jobs = JobRunner::default();
    run(
        &mut state,
        &mut jobs,
        JobOperation::Open { path: path.clone() },
    );
    assert_eq!(
        state.project().dataset().row_count() * state.project().dataset().column_count(),
        768000
    );
    state = state
        .apply_action(UiAction::EditCell {
            column: ColumnId(1),
            row: 0,
            value: Cell::Number(1.),
        })
        .unwrap();
    state = state
        .apply_action(UiAction::Request(PendingAction::Close))
        .unwrap();
    state = state
        .apply_action(UiAction::ResolveUnsaved(UnsavedChoice::Save))
        .unwrap();
    let project = Arc::clone(state.project());
    let original = stamp(&path).unwrap();
    state
        .start_operation(
            &mut jobs,
            JobOperation::Save {
                project,
                path: path.clone(),
                expected: Some(original.clone()),
            },
        )
        .unwrap();
    finish(&mut state, &mut jobs);
    if state.error().is_some() {
        assert_eq!(
            stamp(&path).unwrap(),
            original,
            "refused Save must preserve destination"
        );
    }
    assert!(
        state.error().is_none(),
        "original expanded fixture must save: {:?}",
        state.error()
    );
    assert!(!state.is_dirty());
    // Windows verifies publication but retains the directory-durability warning.
    #[cfg(windows)]
    assert_eq!(state.dialog(), Some(tessera::desktop::Dialog::FileOutcome));
    #[cfg(unix)]
    assert!(state.dialog().is_none());
    assert_eq!(state.project().runs(), old_runs);
    drop(state);
    let mut reopened = DesktopState::new().unwrap();
    run(&mut reopened, &mut jobs, JobOperation::Open { path });
    assert_eq!(
        reopened.project().dataset().cell(ColumnId(1), 0).unwrap(),
        Cell::Number(1.)
    );
    assert_eq!(reopened.project().runs(), old_runs);
}
