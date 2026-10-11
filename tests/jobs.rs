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
    data::{Cell, Column, ColumnId, ColumnKind, Dataset, RevisionId, MAX_OWNED_BYTES},
    error::AppError,
    files::{read_project, Publication},
    import::Delimiter,
    jobs::{may_publish, JobEvent, JobOperation, JobOutput, JobRunner, JobTask},
    operation::{JobControl, JobToken},
    project::Project,
};
static NEXT: AtomicU64 = AtomicU64::new(0);
struct Temp(PathBuf);
impl Temp {
    fn new() -> Self {
        let path = std::env::temp_dir().canonicalize().unwrap().join(format!(
            "tessera-jobs-{}-{}",
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
fn project() -> Arc<Project> {
    Arc::new(
        Project::new(
            "Jobs",
            Dataset::new(vec![Column::numeric(
                ColumnId(1),
                "Score",
                vec![Some(1.0), Some(3.0)],
            )
            .unwrap()])
            .unwrap(),
        )
        .unwrap(),
    )
}
fn token() -> JobToken {
    JobToken {
        document: 1,
        revision: RevisionId(1),
        generation: 1,
    }
}
fn finish(runner: &mut JobRunner) -> JobEvent {
    let deadline = Instant::now() + Duration::from_secs(10);
    loop {
        for event in runner.poll() {
            if matches!(event, JobEvent::Completed { .. }) {
                return event;
            }
        }
        assert!(Instant::now() < deadline, "worker did not finish");
        std::thread::yield_now();
    }
}
fn output(event: JobEvent) -> JobOutput {
    match event {
        JobEvent::Completed { result, .. } => result.unwrap(),
        _ => panic!("missing completion"),
    }
}
#[test]
fn cancelled_or_superseded_results_are_not_published() {
    assert!(may_publish(token(), 1, RevisionId(1), 1));
    assert!(!may_publish(token(), 1, RevisionId(1), 2));
    assert!(!may_publish(token(), 1, RevisionId(2), 1));
    let cancelled = JobEvent::Completed {
        token: token(),
        cancelled: true,
        result: Err(AppError::Cancelled),
    };
    assert!(!cancelled.may_publish(1, RevisionId(1), 1));
}
#[test]
fn same_revision_in_new_document_is_distinct() {
    assert!(!may_publish(token(), 2, RevisionId(1), 1));
}
#[test]
fn saved_old_revision_does_not_clear_new_dirty_state() {
    let temp = Temp::new();
    let path = temp.0.join("save.tessera");
    let p = project();
    let mut runner = JobRunner::default();
    runner
        .start(
            JobTask::new(JobOperation::Save {
                project: p.clone(),
                path: path.clone(),
                expected: None,
            }),
            token(),
        )
        .unwrap();
    let edited = p
        .with_dataset(
            p.dataset()
                .edit_cell(ColumnId(1), 0, Cell::Number(9.0))
                .unwrap(),
        )
        .unwrap();
    let event = finish(&mut runner);
    assert!(!event.confirms_current_save(1, edited.dataset().revision(), 1));
    assert!(event.confirms_current_save(1, RevisionId(1), 1));
    assert!(event.published_file().is_some());
    for publication in [Publication::Conflict, Publication::Uncertain] {
        let mut outcome = event.published_file().unwrap().clone();
        outcome.publication = publication;
        let reconciled = JobEvent::Completed {
            token: token(),
            cancelled: false,
            result: Ok(JobOutput::Saved(outcome)),
        };
        assert!(reconciled.published_file().is_some());
        assert!(!reconciled.confirms_current_save(1, RevisionId(1), 1));
    }
    let mut unstamped = event.published_file().unwrap().clone();
    unstamped.committed_stamp = None;
    let unconfirmed = JobEvent::Completed {
        token: token(),
        cancelled: false,
        result: Ok(JobOutput::Saved(unstamped)),
    };
    assert!(unconfirmed.published_file().is_none());
    assert!(!unconfirmed.confirms_current_save(1, RevisionId(1), 1));
    assert!(path.exists());
    assert_eq!(
        read_project(&path, &JobControl::default())
            .unwrap()
            .0
            .dataset()
            .cell(ColumnId(1), 0)
            .unwrap(),
        Cell::Number(1.0)
    );
}
#[test]
fn worker_failure_remains_visible() {
    let temp = Temp::new();
    let mut runner = JobRunner::default();
    runner
        .start(
            JobTask::new(JobOperation::Open {
                path: temp.0.join("absent.tessera"),
            }),
            token(),
        )
        .unwrap();
    assert!(matches!(
        finish(&mut runner),
        JobEvent::Completed {
            result: Err(AppError::Io(_)),
            ..
        }
    ));
    assert!(!runner.is_running());
    assert!(runner.poll().is_empty());
}
#[test]
fn import_describe_convert_save_open_and_exports_use_the_worker() {
    let temp = Temp::new();
    let mut runner = JobRunner::default();
    for delimiter in [Delimiter::Comma, Delimiter::Tab] {
        let path = temp.0.join("source.txt");
        fs::write(&path, b"Score\n1\n3\n").unwrap();
        runner
            .start(
                JobTask::new(JobOperation::PreviewPath { path, delimiter }),
                token(),
            )
            .unwrap();
        let JobOutput::Preview(preview) = output(finish(&mut runner)) else {
            panic!()
        };
        let choices = preview.default_choices().unwrap();
        runner
            .start(
                JobTask::new(JobOperation::CommitImport { preview, choices }),
                token(),
            )
            .unwrap();
        let JobOutput::Imported(dataset) = output(finish(&mut runner)) else {
            panic!()
        };
        assert_eq!(dataset.row_count(), 2);
    }
    let p = project();
    let request = DescribeRequest {
        columns: vec![ColumnId(1)],
        statistics: vec![Statistic::Mean],
        missing_policy: MissingPolicy::PerVariable,
        filter: RowFilter::default(),
        frequency_weight: None,
    };
    runner
        .start(
            JobTask::new(JobOperation::Analyze {
                project: p.clone(),
                request: request.into(),
            }),
            token(),
        )
        .unwrap();
    let JobOutput::Analyzed(analysis) = output(finish(&mut runner)) else {
        panic!()
    };
    assert_eq!(
        {
            let tessera::analysis::CompletedAnalysis::Describe(a) = &analysis else {
                panic!("expected descriptives")
            };
            &a.rows[0]
        }
        .estimates[&Statistic::Mean]
            .value,
        Some(2.0)
    );
    let p = Arc::new(p.with_run(analysis).unwrap());
    runner
        .start(
            JobTask::new(JobOperation::ConvertColumn {
                project: p.clone(),
                id: ColumnId(1),
                kind: ColumnKind::Text,
            }),
            token(),
        )
        .unwrap();
    let JobOutput::Converted(dataset) = output(finish(&mut runner)) else {
        panic!()
    };
    assert_eq!(
        dataset.cell(ColumnId(1), 0).unwrap(),
        Cell::Text("1".into())
    );
    let path = temp.0.join("project.tessera");
    runner
        .start(
            JobTask::new(JobOperation::Save {
                project: p.clone(),
                path: path.clone(),
                expected: None,
            }),
            token(),
        )
        .unwrap();
    let JobOutput::Saved(saved) = output(finish(&mut runner)) else {
        panic!()
    };
    assert_eq!(saved.publication, Publication::Committed);
    runner
        .start(JobTask::new(JobOperation::Open { path }), token())
        .unwrap();
    let JobOutput::Opened {
        project: reopened, ..
    } = output(finish(&mut runner))
    else {
        panic!()
    };
    assert_eq!(reopened.runs().len(), 1);
    runner
        .start(
            JobTask::new(JobOperation::ExportData {
                project: p.clone(),
                path: temp.0.join("data.csv"),
                delimiter: Delimiter::Comma,
            }),
            token(),
        )
        .unwrap();
    assert!(finish(&mut runner).published_file().is_some());
    runner
        .start(
            JobTask::new(JobOperation::ExportReport {
                project: p.clone(),
                path: temp.0.join("report.html"),
                runs: vec![p.runs()[0].id],
            }),
            token(),
        )
        .unwrap();
    let event = finish(&mut runner);
    assert!(event.published_file().is_some());
    assert!(!event.confirms_current_save(1, RevisionId(1), 1));
}
#[test]
fn pending_work_budget_blocks_path_loading_and_conversion() {
    let temp = Temp::new();
    let path = temp.0.join("source.csv");
    fs::write(&path, b"Score\n1\n").unwrap();
    for operation in [
        JobOperation::PreviewPath {
            path,
            delimiter: Delimiter::Comma,
        },
        JobOperation::ConvertColumn {
            project: project(),
            id: ColumnId(1),
            kind: ColumnKind::Text,
        },
    ] {
        let mut task = JobTask::new(operation);
        task.retained.extra_bytes = MAX_OWNED_BYTES;
        let mut runner = JobRunner::default();
        assert!(matches!(
            runner.start(task, token()),
            Err(AppError::SizeLimit { .. })
        ));
        assert!(!runner.is_running());
    }
}

#[test]
fn repeated_project_snapshots_are_not_double_charged_by_file_jobs() {
    use tessera::data::Budget;
    let temp = Temp::new();
    let p = Arc::new(
        project()
            .with_provenance(vec!["source".repeat(200_000)])
            .unwrap(),
    );
    let live = Budget::dataset_bytes(&[p.dataset()], p.owned_bytes().unwrap()).unwrap();
    for operation in [
        JobOperation::Save {
            project: p.clone(),
            path: temp.0.join("dedup.tessera"),
            expected: None,
        },
        JobOperation::ExportReport {
            project: p.clone(),
            path: temp.0.join("dedup.html"),
            runs: vec![],
        },
    ] {
        // Save also owns the prepared input and decoded provenance while validating.
        // Four MiB covers that work, but not another 1.2 MiB source-project charge.
        let headroom = if matches!(operation, JobOperation::Save { .. }) {
            4 * 1024 * 1024
        } else {
            512 * 1024
        };
        let mut task = JobTask::new(operation);
        task.retained.projects = vec![p.clone(), p.clone()];
        task.retained.extra_bytes = MAX_OWNED_BYTES - live - headroom;
        let mut runner = JobRunner::default();
        runner.start(task, token()).unwrap();
        let event = finish(&mut runner);
        assert!(event.published_file().is_some(), "{event:?}");
    }
}

#[test]
fn oversized_import_path_is_refused_before_loading() {
    let temp = Temp::new();
    let path = temp.0.join("oversize.csv");
    fs::File::create(&path)
        .unwrap()
        .set_len(tessera::import::MAX_INPUT_BYTES as u64 + 1)
        .unwrap();
    let mut runner = JobRunner::default();
    runner
        .start(
            JobTask::new(JobOperation::PreviewPath {
                path,
                delimiter: Delimiter::Comma,
            }),
            token(),
        )
        .unwrap();
    assert!(matches!(
        finish(&mut runner),
        JobEvent::Completed {
            result: Err(AppError::SizeLimit { .. }),
            ..
        }
    ));
}

#[test]
fn task_token_must_match_the_owned_project_revision() {
    let mut runner = JobRunner::default();
    let wrong = JobToken {
        revision: RevisionId(2),
        ..token()
    };
    assert!(matches!(
        runner.start(
            JobTask::new(JobOperation::ConvertColumn {
                project: project(),
                id: ColumnId(1),
                kind: ColumnKind::Text
            }),
            wrong
        ),
        Err(AppError::Conflict(_))
    ));
    assert!(!runner.is_running());
}
