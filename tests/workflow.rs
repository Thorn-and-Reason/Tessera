#![cfg(feature = "desktop")]
use std::{
    fs,
    path::PathBuf,
    sync::{
        atomic::{AtomicU64, Ordering},
        Arc,
    },
    time::{Duration, Instant},
};
use tessera::{
    analysis::{DescribeRequest, MissingPolicy, RowFilter, Statistic},
    data::{Cell, ColumnId},
    desktop::{DesktopState, PendingAction, UiAction, UnsavedChoice},
    files::{read_project, stamp},
    import::{preview, Delimiter},
    jobs::{JobOperation, JobRunner},
    operation::JobControl,
};
static NEXT: AtomicU64 = AtomicU64::new(0);
struct Temp(PathBuf);
impl Temp {
    fn new() -> Self {
        let path = std::env::temp_dir().canonicalize().unwrap().join(format!(
            "tessera-workflow-{}-{}",
            std::process::id(),
            NEXT.fetch_add(1, Ordering::Relaxed)
        ));
        fs::create_dir(&path).unwrap();
        Self(path)
    }
}
impl Drop for Temp {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}
fn finish(state: &mut DesktopState, jobs: &mut JobRunner) {
    let deadline = Instant::now() + Duration::from_secs(20);
    while jobs.is_running() {
        state.poll(jobs);
        assert!(
            Instant::now() < deadline,
            "native state worker did not finish"
        );
        std::thread::yield_now();
    }
}
fn run(state: &mut DesktopState, jobs: &mut JobRunner, operation: JobOperation) {
    state.start_operation(jobs, operation).unwrap();
    finish(state, jobs);
    assert!(state.error().is_none(), "{:?}", state.error());
}
fn request() -> DescribeRequest {
    DescribeRequest {
        columns: vec![ColumnId(2)],
        statistics: vec![
            Statistic::Mean,
            Statistic::SampleStdDev,
            Statistic::Minimum,
            Statistic::Maximum,
        ],
        missing_policy: MissingPolicy::PerVariable,
        filter: RowFilter::default(),
        frequency_weight: None,
    }
}
fn imported() -> DesktopState {
    let input = Arc::new(
        preview(
            Arc::from(&b"Name,Total score\nP01,17\nP02,22\nP03,25\nP04,20\nP05,25\nP06,31\n"[..]),
            Delimiter::Comma,
            &JobControl::default(),
        )
        .unwrap(),
    );
    let choices = input.default_choices().unwrap();
    let mut state = DesktopState::new().unwrap();
    run(
        &mut state,
        &mut JobRunner::default(),
        JobOperation::CommitImport {
            preview: input,
            choices,
        },
    );
    state
}

#[test]
fn first_build_workflow() {
    let temp = Temp::new();
    let mut state = imported();
    let mut jobs = JobRunner::default();
    state = state
        .apply_action(UiAction::EditCell {
            column: ColumnId(1),
            row: 0,
            value: Cell::Text("Siân <&>".into()),
        })
        .unwrap();
    let before_run = Arc::clone(state.project());
    run(
        &mut state,
        &mut jobs,
        JobOperation::Analyze {
            project: before_run,
            request: request().into(),
        },
    );
    let tessera::analysis::CompletedAnalysis::Describe(result) =
        state.project().runs()[0].analysis.clone()
    else {
        panic!("expected descriptives");
    };
    assert_eq!(result.rows[0].valid_n, 6);
    assert_eq!(
        result.rows[0].estimates[&Statistic::Mean].value,
        Some(23.333333333333332)
    );
    let sd = result.rows[0].estimates[&Statistic::SampleStdDev]
        .value
        .unwrap();
    assert!(sd.to_bits().abs_diff(4.844240566555986_f64.to_bits()) <= 2);
    assert_eq!(
        result.rows[0].estimates[&Statistic::Minimum].value,
        Some(17.0)
    );
    assert_eq!(
        result.rows[0].estimates[&Statistic::Maximum].value,
        Some(31.0)
    );
    let path = temp.0.join("analysis.tessera");
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
    run(
        &mut state,
        &mut jobs,
        JobOperation::Open { path: path.clone() },
    );
    assert_eq!(
        state.project().dataset().cell(ColumnId(1), 0).unwrap(),
        Cell::Text("Siân <&>".into())
    );
    assert_eq!(
        state.project().runs()[0].analysis,
        tessera::analysis::CompletedAnalysis::Describe(result.clone())
    );
    for (name, delimiter) in [("data.csv", Delimiter::Comma), ("data.tsv", Delimiter::Tab)] {
        let destination = temp.0.join(name);
        let project = Arc::clone(state.project());
        run(
            &mut state,
            &mut jobs,
            JobOperation::ExportData {
                project,
                path: destination.clone(),
                delimiter,
                acknowledge_formula_risk: false,
            },
        );
        let exported = fs::read_to_string(&destination).unwrap();
        assert!(exported.contains("Siân <&>"));
        assert!(exported.contains("31"));
        let project = Arc::clone(state.project());
        state
            .start_operation(
                &mut jobs,
                JobOperation::ExportData {
                    project,
                    path: destination.clone(),
                    delimiter,
                    acknowledge_formula_risk: false,
                },
            )
            .unwrap();
        finish(&mut state, &mut jobs);
        assert_eq!(fs::read_to_string(&destination).unwrap(), exported);
    }
    let report = temp.0.join("report.html");
    let project = Arc::clone(state.project());
    let run_id = project.runs()[0].id;
    run(
        &mut state,
        &mut jobs,
        JobOperation::ExportReport {
            project,
            path: report.clone(),
            runs: vec![run_id],
        },
    );
    let html = fs::read_to_string(report).unwrap();
    assert!(html.contains("23.333333333333332"));
    assert!(html.contains("Total score"));
    assert_eq!(
        read_project(&path, &JobControl::default())
            .unwrap()
            .0
            .runs()[0]
            .analysis,
        tessera::analysis::CompletedAnalysis::Describe(result.clone())
    );
}

#[test]
fn failure_paths_preserve_active_data_results_and_external_files() {
    let temp = Temp::new();
    let mut state = imported();
    let mut jobs = JobRunner::default();
    let project = Arc::clone(state.project());
    run(
        &mut state,
        &mut jobs,
        JobOperation::Analyze {
            project,
            request: request().into(),
        },
    );
    let before = Arc::clone(state.project());
    let bad = temp.0.join("bad.csv");
    fs::write(&bad, b"Score\n\"private\"unexpected\n").unwrap();
    state
        .start_operation(
            &mut jobs,
            JobOperation::PreviewPath {
                path: bad,
                delimiter: Delimiter::Comma,
            },
        )
        .unwrap();
    finish(&mut state, &mut jobs);
    assert!(state.error().is_some());
    assert!(!state.error().unwrap().contains("private"));
    assert!(Arc::ptr_eq(state.project(), &before));
    let project = Arc::clone(state.project());
    state
        .start_operation(
            &mut jobs,
            JobOperation::Analyze {
                project,
                request: request().into(),
            },
        )
        .unwrap();
    jobs.cancel();
    finish(&mut state, &mut jobs);
    assert!(Arc::ptr_eq(state.project(), &before));
    let pending = state
        .apply_action(UiAction::Request(PendingAction::New))
        .unwrap();
    let cancelled = pending
        .apply_action(UiAction::ResolveUnsaved(UnsavedChoice::Cancel))
        .unwrap();
    assert!(Arc::ptr_eq(cancelled.project(), &before));
    let path = temp.0.join("project.tessera");
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
    let original_stamp = stamp(&path).unwrap();
    fs::write(&path, b"external version").unwrap();
    state = state
        .apply_action(UiAction::EditCell {
            column: ColumnId(2),
            row: 0,
            value: Cell::Number(19.0),
        })
        .unwrap();
    let edited = Arc::clone(state.project());
    state
        .start_operation(
            &mut jobs,
            JobOperation::Save {
                project: Arc::clone(&edited),
                path: path.clone(),
                expected: Some(original_stamp),
            },
        )
        .unwrap();
    finish(&mut state, &mut jobs);
    assert!(state.is_dirty());
    assert!(Arc::ptr_eq(state.project(), &edited));
    assert_eq!(fs::read(&path).unwrap(), b"external version");
    assert_eq!(
        descriptive(&state.project().runs()[0].analysis).rows[0].estimates[&Statistic::Mean].value,
        Some(23.333333333333332)
    );
}

fn descriptive(a: &tessera::analysis::CompletedAnalysis) -> &tessera::analysis::ComputedAnalysis {
    let tessera::analysis::CompletedAnalysis::Describe(v) = a else {
        panic!("expected descriptives")
    };
    v
}

mod common;

#[test]
fn mixed_first_workflow() {
    use tessera::{analysis::*, project::Project};
    let temp = Temp::new();
    let mut state = DesktopState::from_project(
        Project::new("Mixed <&> workflow", common::sample_dataset()).unwrap(),
    )
    .unwrap();
    let mut jobs = JobRunner::default();
    let requests = vec![
        AnalysisRequest::Describe(DescribeRequest {
            columns: vec![ColumnId(6)],
            ..request()
        }),
        AnalysisRequest::Pearson(PearsonRequest {
            columns: vec![ColumnId(3), ColumnId(4)],
            missing_policy: CorrelationMissingPolicy::PairwiseComplete,
            filter: RowFilter::default(),
            frequency_weight: None,
        }),
        AnalysisRequest::OneSampleT(OneSampleTRequest {
            variable: ColumnId(6),
            reference_mean: 20.,
            confidence: ConfidenceLevel::P95,
            filter: RowFilter::default(),
            frequency_weight: None,
        }),
        AnalysisRequest::PairedT(PairedTRequest {
            first: ColumnId(3),
            second: ColumnId(4),
            confidence: ConfidenceLevel::P90,
            filter: RowFilter::default(),
            frequency_weight: None,
        }),
        AnalysisRequest::IndependentT(IndependentTRequest {
            outcome: ColumnId(6),
            group: ColumnId(5),
            groups: [
                Cell::Text("Control".into()),
                Cell::Text("Comparison".into()),
            ],
            method: IndependentMethod::Welch,
            confidence: ConfidenceLevel::P99,
            filter: RowFilter::default(),
            frequency_weight: None,
        }),
    ];
    for request in &requests {
        let project = Arc::clone(state.project());
        run(
            &mut state,
            &mut jobs,
            JobOperation::Analyze {
                project,
                request: request.clone(),
            },
        );
    }
    assert_eq!(state.project().runs().len(), 5);
    let CompletedAnalysis::Pearson(p) = &state.project().runs()[1].analysis else {
        panic!()
    };
    assert_eq!(p.rows[0].n, 6);
    assert!((p.rows[0].r.value.unwrap() - 0.9449821672291636).abs() < 2e-12);
    // Independent mpmath 1.3.0, 80-digit regularized incomplete-beta tails.
    assert!((p.rows[0].p.value.unwrap() - 0.004457174441763437).abs() < 2e-14);
    let CompletedAnalysis::OneSampleT(t) = &state.project().runs()[2].analysis else {
        panic!()
    };
    assert!((t.statistics.difference.value.unwrap() - 10. / 3.).abs() < 2e-12);
    assert!((t.statistics.p.value.unwrap() - 0.15270488809703775).abs() < 2e-13);
    let CompletedAnalysis::PairedT(t) = &state.project().runs()[3].analysis else {
        panic!()
    };
    assert!((t.statistics.difference.value.unwrap() + 5. / 3.).abs() < 2e-12);
    assert!((t.statistics.p.value.unwrap() - 0.004104715980053322).abs() < 2e-14);
    let CompletedAnalysis::IndependentT(t) = &state.project().runs()[4].analysis else {
        panic!()
    };
    assert_eq!(t.groups[0].n, 3);
    assert_eq!(t.groups[1].n, 3);
    assert!((t.statistics.difference.value.unwrap() + 4.).abs() < 2e-12);
    assert!((t.statistics.p.value.unwrap() - 0.3726194626643288).abs() < 2e-13);
    assert!((t.statistics.standard_error.value.unwrap() - (140_f64 / 9.).sqrt()).abs() < 2e-12);
    assert!((t.statistics.df.value.unwrap() - 19600_f64 / 5341.).abs() < 2e-12);
    let before = state.project().runs().to_vec();
    state = state
        .apply_action(UiAction::EditCell {
            column: ColumnId(3),
            row: 0,
            value: Cell::Number(7.),
        })
        .unwrap();
    assert!(state.selected_run_is_stale());
    let mut metadata = state
        .project()
        .dataset()
        .column(ColumnId(6))
        .unwrap()
        .metadata()
        .clone();
    metadata.description = "Siân <&>".into();
    state = state
        .apply_action(UiAction::Metadata {
            column: ColumnId(6),
            metadata,
        })
        .unwrap();
    let path = temp.0.join("mixed.tessera");
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
        serde_json::from_str::<serde_json::Value>(&fs::read_to_string(&path).unwrap()).unwrap()
            ["version"],
        3
    );
    let mut reopened = DesktopState::new().unwrap();
    run(&mut reopened, &mut jobs, JobOperation::Open { path });
    assert_eq!(reopened.project().runs(), before);
    for (saved, request) in reopened.project().runs().iter().zip(&requests) {
        assert_eq!(saved.analysis.request(), request.borrowed());
    }
    for request in requests {
        let project = Arc::clone(reopened.project());
        run(
            &mut reopened,
            &mut jobs,
            JobOperation::Analyze { project, request },
        );
    }
    assert_eq!(reopened.project().runs().len(), 10);
    let report = temp.0.join("mixed.html");
    let project = Arc::clone(reopened.project());
    let runs = project.runs().iter().map(|r| r.id).collect();
    run(
        &mut reopened,
        &mut jobs,
        JobOperation::ExportReport {
            project,
            path: report.clone(),
            runs,
        },
    );
    let html = fs::read_to_string(report).unwrap();
    for label in [
        "Pearson",
        "One-sample",
        "Paired",
        "Independent",
        "Siân &lt;&amp;&gt;",
    ] {
        assert!(html.contains(label), "{label}");
    }
}

#[test]
fn mixed_failures_preserve_history_and_destination() {
    use tessera::{analysis::*, project::Project};
    let temp = Temp::new();
    let mut state =
        DesktopState::from_project(Project::new("Mixed", common::sample_dataset()).unwrap())
            .unwrap();
    let mut jobs = JobRunner::default();
    let request = AnalysisRequest::Pearson(PearsonRequest {
        columns: vec![ColumnId(3), ColumnId(4)],
        missing_policy: CorrelationMissingPolicy::PairwiseComplete,
        filter: RowFilter::default(),
        frequency_weight: None,
    });
    let project = Arc::clone(state.project());
    run(
        &mut state,
        &mut jobs,
        JobOperation::Analyze {
            project,
            request: request.clone(),
        },
    );
    let before = Arc::clone(state.project());
    let bad_import = temp.0.join("malformed-mixed.csv");
    fs::write(&bad_import, b"Score\n\"private\"unexpected\n").unwrap();
    state
        .start_operation(
            &mut jobs,
            JobOperation::PreviewPath {
                path: bad_import,
                delimiter: Delimiter::Comma,
            },
        )
        .unwrap();
    finish(&mut state, &mut jobs);
    assert!(state.error().is_some());
    assert!(!state.error().unwrap().contains("private"));
    assert!(Arc::ptr_eq(state.project(), &before));
    let bad = temp.0.join("malformed-future.tessera");
    let mut value: serde_json::Value =
        serde_json::from_str(include_str!("../examples/mixed-analysis.tessera")).unwrap();
    value["version"] = serde_json::json!(4);
    fs::write(&bad, serde_json::to_vec(&value).unwrap()).unwrap();
    state
        .start_operation(&mut jobs, JobOperation::Open { path: bad })
        .unwrap();
    finish(&mut state, &mut jobs);
    assert!(state.error().is_some());
    assert!(Arc::ptr_eq(state.project(), &before));
    for operation in [
        JobOperation::Analyze {
            project: Arc::clone(&before),
            request,
        },
        JobOperation::GroupLookup {
            project: Arc::clone(&before),
            request: GroupLookupRequest {
                column: ColumnId(5),
                role_epoch: 0,
                query: "Control".into(),
                selected: [None, None],
            },
        },
    ] {
        state.start_operation(&mut jobs, operation).unwrap();
        jobs.cancel();
        finish(&mut state, &mut jobs);
        assert!(Arc::ptr_eq(state.project(), &before));
        assert!(state.group_lookup().is_none());
    }
    let path = temp.0.join("mixed.tessera");
    run(
        &mut state,
        &mut jobs,
        JobOperation::Save {
            project: Arc::clone(&before),
            path: path.clone(),
            expected: None,
        },
    );
    let expected = stamp(&path).unwrap();
    fs::write(&path, b"external mixed history").unwrap();
    state = state
        .apply_action(UiAction::EditCell {
            column: ColumnId(3),
            row: 0,
            value: Cell::Number(7.),
        })
        .unwrap();
    let edited = Arc::clone(state.project());
    state
        .start_operation(
            &mut jobs,
            JobOperation::Save {
                project: Arc::clone(&edited),
                path: path.clone(),
                expected: Some(expected),
            },
        )
        .unwrap();
    finish(&mut state, &mut jobs);
    assert!(state.error().is_some());
    assert!(state.is_dirty());
    assert!(Arc::ptr_eq(state.project(), &edited));
    assert_eq!(state.project().runs(), before.runs());
    assert_eq!(fs::read(path).unwrap(), b"external mixed history");
}

#[path = "workflow/exploration.rs"]
mod exploration;

#[path = "workflow/survey.rs"]
mod survey;
#[path = "workflow/survey_improvements.rs"]
mod survey_improvements;
