mod common;
use std::{
    sync::Arc,
    time::{Duration, Instant},
};
use tessera::{
    analysis::*,
    data::{Cell, ColumnId, MAX_OWNED_BYTES},
    error::AppError,
    jobs::{JobEvent, JobOperation, JobOutput, JobRunner, JobTask},
    operation::{JobControl, JobToken},
    project::Project,
    project_io::{decode, encode},
};
fn request() -> ReliabilityRequest {
    ReliabilityRequest {
        items: vec![ColumnId(3), ColumnId(4)],
        filter: RowFilter::default(),
        frequency_weight: None,
        reversals: vec![],
    }
}
fn completed(project: &Project, c: &JobControl) -> CompletedAnalysis {
    let r = AnalysisRequest::Reliability(request());
    analyze_with_budget(
        project.dataset(),
        &r,
        c,
        &[],
        std::mem::size_of::<AnalysisRequest>() + r.heap_bytes().unwrap(),
    )
    .unwrap()
}
#[test]
fn dispatch_history_and_passive_open_preserve_capture() {
    let c = JobControl::default();
    let original = Project::new("Reliability", common::sample_dataset()).unwrap();
    let result = completed(&original, &c);
    let CompletedAnalysis::Reliability(run) = &result else {
        panic!("wrong result")
    };
    assert_eq!(run.request, request());
    run.validate_controlled(&c).unwrap();
    let saved = original
        .with_run_with_budget_controlled(result.clone(), &[], 0, &c)
        .unwrap();
    assert!(original.runs().is_empty());
    let changed = saved
        .with_dataset(
            saved
                .dataset()
                .edit_cell(ColumnId(3), 0, Cell::Number(-900.))
                .unwrap(),
        )
        .unwrap();
    assert!(Arc::ptr_eq(&saved.runs()[0], &changed.runs()[0]));
    let mut bytes = Vec::new();
    encode(&changed, &mut bytes, &c).unwrap();
    let value: serde_json::Value = serde_json::from_slice(&bytes).unwrap();
    assert_eq!(value["version"], 3);
    let reopened = decode(bytes.as_slice(), &c).unwrap();
    assert_eq!(reopened.runs()[0].analysis, result);
    assert_eq!(reopened.dataset().revision(), changed.dataset().revision());
}
#[test]
fn project_supplied_cancellation_and_stale_completion_leave_history_unchanged() {
    let p = Project::new("History", common::sample_dataset()).unwrap();
    let c = JobControl::default();
    let a = completed(&p, &c);
    let saved = p.with_run(a.clone()).unwrap();
    let before = Arc::clone(&saved.runs()[0]);
    c.cancel();
    assert!(matches!(
        saved.with_run_with_budget_controlled(a.clone(), &[], MAX_OWNED_BYTES, &c),
        Err(AppError::Cancelled)
    ));
    assert!(matches!(
        Project::from_parts_controlled(
            "",
            saved.dataset().clone(),
            saved.runs().to_vec(),
            saved.next_run_id(),
            &c
        ),
        Err(AppError::Cancelled)
    ));
    assert!(matches!(
        a.validate_controlled(&c),
        Err(AppError::Cancelled)
    ));
    let changed = saved
        .with_dataset(
            saved
                .dataset()
                .edit_cell(ColumnId(3), 0, Cell::Number(900.))
                .unwrap(),
        )
        .unwrap();
    assert!(matches!(changed.with_run(a), Err(AppError::Conflict(_))));
    assert!(Arc::ptr_eq(&before, &saved.runs()[0]));
    assert_eq!(saved.runs().len(), 1);
}
#[test]
fn real_worker_routes_reliability_with_retained_project_owners() {
    let p = Arc::new(Project::new("Worker", common::sample_dataset()).unwrap());
    let token = JobToken {
        document: 1,
        revision: p.dataset().revision(),
        generation: 1,
    };
    let mut runner = JobRunner::default();
    runner
        .start(
            JobTask::new(JobOperation::Analyze {
                project: Arc::clone(&p),
                request: AnalysisRequest::Reliability(request()),
            }),
            token,
        )
        .unwrap();
    let until = Instant::now() + Duration::from_secs(15);
    let mut result = None;
    loop {
        for event in runner.poll() {
            if let JobEvent::Completed { result: output, .. } = event {
                let JobOutput::Analyzed(a) = output.unwrap() else {
                    panic!("wrong worker output")
                };
                result = Some(a);
            }
        }
        if !runner.is_running() {
            break;
        }
        assert!(Instant::now() < until);
        std::thread::sleep(Duration::from_millis(2));
    }
    let a = result.expect("completion");
    assert!(matches!(a, CompletedAnalysis::Reliability(_)));
    assert_eq!(p.with_run(a).unwrap().runs().len(), 1);
    assert!(p.runs().is_empty());
}
#[test]
fn central_current_validation_and_capacity_admission() {
    let p = Project::new("Admission", common::sample_dataset()).unwrap();
    let r = AnalysisRequest::Reliability(request());
    let c = JobControl::default();
    r.validate_current_controlled(p.dataset(), &[], r.heap_bytes().unwrap(), &c)
        .unwrap();
    let mut invalid = request();
    invalid.items[0] = ColumnId(1);
    assert!(AnalysisRequest::Reliability(invalid)
        .validate_current_controlled(p.dataset(), &[], 0, &c)
        .is_err());
    assert!(analyze_with_budget(p.dataset(), &r, &c, &[], MAX_OWNED_BYTES).is_err());
    c.cancel();
    assert!(matches!(
        r.validate_current_controlled(p.dataset(), &[], MAX_OWNED_BYTES, &c),
        Err(AppError::Cancelled)
    ));
}
#[test]
fn project_admits_simultaneous_retained_history_output_and_scratch() {
    let c = JobControl::default();
    let p = Project::new("Budget", common::sample_dataset()).unwrap();
    let a = completed(&p, &c);
    let p = p.with_run(a.clone()).unwrap();
    let overhead =
        p.owned_bytes().unwrap() + a.owned_bytes().unwrap() + a.validation_scratch_bytes().unwrap();
    assert!(p
        .with_run_with_budget_controlled(a, &[], MAX_OWNED_BYTES - overhead / 2, &c)
        .is_err());
    assert_eq!(p.runs().len(), 1);
}
