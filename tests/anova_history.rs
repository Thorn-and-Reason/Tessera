mod common;
#[path = "common/anova.rs"]
mod fixture;
use std::{
    sync::Arc,
    time::{Duration, Instant},
};
use tessera::{
    analysis::*,
    data::{Cell, ColumnId, MAX_OWNED_BYTES},
    jobs::{JobEvent, JobOperation, JobOutput, JobRunner, JobTask},
    operation::{JobControl, JobToken},
    project::Project,
    project_io::{decode, encode},
};
#[test]
fn all_designs_dispatch_and_history_stay_frozen() {
    for design in 0..3 {
        let (d, r) = fixture::input(design);
        let c = JobControl::default();
        let direct = anova_with_budget(&d, &r, &c, &[&d], fixture::request_bytes(&r)).unwrap();
        direct.validate(&c).unwrap();
        assert!(matches!(direct.outcome, AnovaOutcome::Fitted(_)));
        let central = analyze_with_budget(
            &d,
            &AnalysisRequest::Anova(r.clone()),
            &c,
            &[],
            std::mem::size_of::<AnalysisRequest>() + r.heap_bytes().unwrap(),
        )
        .unwrap();
        assert_eq!(central, CompletedAnalysis::Anova(direct));
        let original = Project::new("ANOVA", d).unwrap();
        let saved = original.with_run(central.clone()).unwrap();
        assert!(original.runs().is_empty());
        let changed = saved
            .with_dataset(
                saved
                    .dataset()
                    .edit_cell(ColumnId(1), 0, Cell::Number(-900.))
                    .unwrap(),
            )
            .unwrap();
        assert!(Arc::ptr_eq(&saved.runs()[0], &changed.runs()[0]));
        let mut bytes = Vec::new();
        encode(&changed, &mut bytes, &c).unwrap();
        assert!(String::from_utf8_lossy(&bytes).contains("\"version\":3"));
        let reopened = decode(bytes.as_slice(), &c).unwrap();
        assert_eq!(reopened.runs()[0].analysis, central);
    }
}
#[test]
fn real_worker_produces_all_designs_without_mutating_source() {
    for design in 0..3 {
        let (d, r) = fixture::input(design);
        let project = Arc::new(Project::new("Worker", d).unwrap());
        let token = JobToken {
            document: 1,
            revision: project.dataset().revision(),
            generation: 1,
        };
        let mut runner = JobRunner::default();
        runner
            .start(
                JobTask::new(JobOperation::Analyze {
                    project: Arc::clone(&project),
                    request: AnalysisRequest::Anova(r),
                }),
                token,
            )
            .unwrap();
        let start = Instant::now();
        loop {
            for event in runner.poll() {
                if let JobEvent::Completed { result, .. } = event {
                    let JobOutput::Analyzed(result) = result.unwrap() else {
                        panic!("wrong output")
                    };
                    let newer = project.with_run(result).unwrap();
                    assert_eq!(newer.runs().len(), 1);
                    assert!(project.runs().is_empty());
                    break;
                }
            }
            if !runner.is_running() {
                break;
            }
            assert!(start.elapsed() < Duration::from_secs(15));
            std::thread::sleep(Duration::from_millis(2));
        }
    }
}
#[test]
fn direct_admission_and_precancellation_precede_copy() {
    let (d, r) = fixture::input(0);
    let c = JobControl::default();
    assert!(anova_with_budget(&d, &r, &c, &[], fixture::request_bytes(&r) - 1).is_err());
    assert!(anova_with_budget(&d, &r, &c, &[], MAX_OWNED_BYTES).is_err());
    c.cancel();
    assert!(matches!(
        anova_with_budget(&d, &r, &c, &[], 0),
        Err(tessera::error::AppError::Cancelled)
    ));
}
#[test]
fn fresh_process_helper() {
    let Some(dir) = std::env::var_os("TESSERA_ANOVA_PROCESS_DIR") else {
        return;
    };
    let dir = std::path::PathBuf::from(dir);
    let mode = std::env::var("TESSERA_ANOVA_PROCESS_MODE").unwrap();
    for design in 0..3 {
        let path = dir.join(format!("design-{design}.tessera"));
        let baseline = dir.join(format!("baseline-{design}.json"));
        if mode == "produce" {
            let p = fixture::captured(design);
            let mut bytes = Vec::new();
            encode(&p, &mut bytes, &JobControl::default()).unwrap();
            std::fs::write(&baseline, &bytes).unwrap();
            let d = p
                .dataset()
                .edit_cell(ColumnId(1), 0, Cell::Number(-777.))
                .unwrap();
            let old = d.column(ColumnId(2)).unwrap();
            let metadata = tessera::data::ColumnMetadata {
                name: "Current renamed factor".into(),
                description: "Current metadata".into(),
                value_labels: vec![tessera::data::ValueLabel {
                    value: Cell::Text(" A ".into()),
                    label: "Current label".into(),
                }],
                ..old.metadata().clone()
            };
            let changed = d
                .replace_column(old.with_metadata(metadata).unwrap())
                .unwrap();
            let changed = changed
                .replace_column(
                    tessera::data::Column::text(
                        ColumnId(if design == 2 { 5 } else { 1 }),
                        "Current text outcome",
                        (0..changed.row_count())
                            .map(|i| Some(format!("current-{i}")))
                            .collect(),
                    )
                    .unwrap(),
                )
                .unwrap();
            let p = p.with_dataset(changed).unwrap();
            tessera::files::save_project(&path, &p, None, &JobControl::default()).unwrap();
        } else {
            let (p, _) = tessera::files::read_project(&path, &JobControl::default()).unwrap();
            let expected = decode(
                std::fs::read(&baseline).unwrap().as_slice(),
                &JobControl::default(),
            )
            .unwrap();
            assert_eq!(p.runs(), expected.runs());
            assert_ne!(p.dataset().revision(), expected.dataset().revision());
            assert_eq!(
                p.dataset().column(ColumnId(2)).unwrap().name(),
                "Current renamed factor"
            );
            let request = p.runs()[0].analysis.request().clone_owned();
            assert!(request
                .validate_current(p.dataset(), &[], request.heap_bytes().unwrap())
                .is_err());
            let html = dir.join(format!("report-{design}.html"));
            tessera::export::export_report(
                &html,
                &p,
                &[tessera::operation::RunId(1)],
                &JobControl::default(),
            )
            .unwrap();
            let html = std::fs::read_to_string(html).unwrap();
            assert!(html.contains("Historical results"));
            assert!(!html.contains("Current renamed factor"));
            assert!(!html.contains("private-person-"));
        }
    }
}
#[test]
fn terminated_producer_then_fresh_consumer_preserves_captures() {
    let dir = fixture::Temp::new();
    for mode in ["produce", "consume"] {
        let status = std::process::Command::new(std::env::current_exe().unwrap())
            .args(["--exact", "fresh_process_helper", "--nocapture"])
            .env("TESSERA_ANOVA_PROCESS_DIR", &dir.0)
            .env("TESSERA_ANOVA_PROCESS_MODE", mode)
            .status()
            .unwrap();
        assert!(status.success(), "child {mode} failed");
    }
}
