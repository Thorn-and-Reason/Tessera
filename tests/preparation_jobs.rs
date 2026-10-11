use std::{
    sync::Arc,
    time::{Duration, Instant},
};
use tessera::{
    data::{Column, ColumnId, Dataset, RevisionId, MAX_OWNED_BYTES},
    error::AppError,
    jobs::{JobEvent, JobOperation, JobOutput, JobRunner, JobTask},
    operation::JobToken,
    preparation::Step,
    project::Project,
};
fn source() -> Arc<Project> {
    Arc::new(
        Project::new(
            "Prepare",
            Dataset::new(vec![Column::numeric(
                ColumnId(1),
                "Score",
                vec![Some(3.0), Some(1.0)],
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
fn finish(r: &mut JobRunner) -> JobEvent {
    let end = Instant::now() + Duration::from_secs(10);
    loop {
        for e in r.poll() {
            if matches!(e, JobEvent::Completed { .. }) {
                return e;
            }
        }
        assert!(Instant::now() < end);
        std::thread::yield_now();
    }
}
#[test]
fn preparation_job_is_atomic_and_token_gated() {
    let p = source();
    let mut task = JobTask::new(JobOperation::Prepare {
        project: p.clone(),
        step: Step::Sort {
            source: ColumnId(1),
            descending: false,
        },
    });
    task.retained.projects.push(p.clone());
    let mut r = JobRunner::default();
    r.start(task, token()).unwrap();
    let e = finish(&mut r);
    assert!(e.may_publish(1, RevisionId(1), 1));
    assert!(!e.may_publish(2, RevisionId(1), 1));
    let JobEvent::Completed {
        result: Ok(JobOutput::Prepared(output)),
        ..
    } = e
    else {
        panic!("preparation failed")
    };
    assert_eq!(
        output.dataset().cell(ColumnId(1), 0).unwrap(),
        tessera::data::Cell::Number(1.0)
    );
    assert_eq!(
        p.dataset().cell(ColumnId(1), 0).unwrap(),
        tessera::data::Cell::Number(3.0)
    );
    assert_eq!(p.dataset().revision(), RevisionId(1));
}
#[test]
fn preparation_work_limit_rejects_before_worker_starts() {
    let mut t = JobTask::new(JobOperation::Prepare {
        project: source(),
        step: Step::Sort {
            source: ColumnId(1),
            descending: false,
        },
    });
    t.retained.extra_bytes = MAX_OWNED_BYTES;
    let mut r = JobRunner::default();
    assert!(matches!(
        r.start(t, token()),
        Err(AppError::SizeLimit { .. })
    ));
    assert!(!r.is_running());
}
#[test]
fn cancellation_prevents_prepared_result_publication() {
    let p = source();
    let mut r = JobRunner::default();
    r.start(
        JobTask::new(JobOperation::Prepare {
            project: p.clone(),
            step: Step::Sort {
                source: ColumnId(1),
                descending: false,
            },
        }),
        token(),
    )
    .unwrap();
    r.cancel();
    let event = finish(&mut r);
    assert!(!event.may_publish(1, RevisionId(1), 1));
    assert_eq!(
        p.dataset().cell(ColumnId(1), 0).unwrap(),
        tessera::data::Cell::Number(3.0)
    );
}
