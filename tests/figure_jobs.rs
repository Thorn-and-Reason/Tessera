use std::{
    sync::Arc,
    time::{Duration, Instant},
};
use tessera::{
    data::{Dataset, RevisionId},
    export::figures::FigureFormat,
    figures::{FigureKey, FigureKind},
    jobs::{JobEvent, JobOperation, JobRunner, JobTask},
    operation::JobToken,
    project::Project,
};

#[test]
fn missing_figure_job_returns_error_without_creating_destination() {
    let path =
        std::env::temp_dir().join(format!("tessera-missing-figure-{}.svg", std::process::id()));
    let project = Arc::new(Project::new("Figure job", Dataset::new(Vec::new()).unwrap()).unwrap());
    let mut runner = JobRunner::default();
    runner
        .start(
            JobTask::new(JobOperation::ExportFigure {
                project,
                path: path.clone(),
                key: FigureKey {
                    run: u64::MAX,
                    kind: FigureKind::Histogram,
                    variant: 0,
                },
                format: FigureFormat::Svg,
            }),
            JobToken {
                document: 1,
                revision: RevisionId(1),
                generation: 1,
            },
        )
        .unwrap();
    let deadline = Instant::now() + Duration::from_secs(10);
    loop {
        if let Some(JobEvent::Completed { result, .. }) = runner
            .poll()
            .into_iter()
            .find(|event| matches!(event, JobEvent::Completed { .. }))
        {
            assert!(result.is_err());
            assert!(!path.exists());
            break;
        }
        assert!(Instant::now() < deadline, "figure export did not finish");
        std::thread::yield_now();
    }
}

fn batch_project() -> Arc<Project> {
    use tessera::{
        analysis::*,
        data::{Column, ColumnId},
        operation::JobControl,
    };
    let data = Dataset::new(vec![Column::numeric(
        ColumnId(1),
        "Score",
        vec![Some(1.), Some(2.), Some(3.)],
    )
    .unwrap()])
    .unwrap();
    let result = histogram_with_budget(
        &data,
        &HistogramRequest {
            variable: ColumnId(1),
            bins: HistogramBins::Count(2),
            filter: RowFilter::default(),
            frequency_weight: None,
        },
        &JobControl::default(),
        &[],
        0,
    )
    .unwrap();
    Arc::new(
        Project::new("Batch", data)
            .unwrap()
            .with_run(CompletedAnalysis::Histogram(result))
            .unwrap(),
    )
}
fn batch_result(
    directory: &std::path::Path,
    keys: Vec<FigureKey>,
) -> tessera::error::AppResult<tessera::jobs::JobOutput> {
    let mut runner = JobRunner::default();
    runner
        .start(
            JobTask::new(JobOperation::ExportFigures {
                project: batch_project(),
                directory: directory.into(),
                keys,
                format: FigureFormat::Svg,
            }),
            JobToken {
                document: 1,
                revision: RevisionId(1),
                generation: 1,
            },
        )
        .unwrap();
    let deadline = Instant::now() + Duration::from_secs(10);
    loop {
        for event in runner.poll() {
            if let JobEvent::Completed { result, .. } = event {
                return result;
            }
        }
        assert!(Instant::now() < deadline);
        std::thread::yield_now();
    }
}
#[test]
fn batch_exports_stable_file_and_preserves_existing_conflict() {
    use tessera::{files::Publication, jobs::JobOutput};
    let directory = std::env::temp_dir()
        .canonicalize()
        .unwrap()
        .join(format!("tessera-batch-{}", std::process::id()));
    std::fs::create_dir_all(&directory).unwrap();
    let key = FigureKey {
        run: 1,
        kind: FigureKind::Histogram,
        variant: 0,
    };
    let path = directory.join("run-1-histogram-0.svg");
    let _ = std::fs::remove_file(&path);
    let JobOutput::FiguresExported { outcomes } = batch_result(&directory, vec![key]).unwrap()
    else {
        panic!("missing outcomes")
    };
    assert_eq!(outcomes.len(), 1);
    assert_eq!(outcomes[0].publication, Publication::Committed);
    let original = std::fs::read(&path).unwrap();
    assert!(matches!(
        batch_result(&directory, vec![key]),
        Err(tessera::error::AppError::Conflict(_))
    ));
    assert_eq!(std::fs::read(&path).unwrap(), original);
    std::fs::remove_dir_all(directory).unwrap();
}
