// Published recovery checkpoints require verified directory durability (Unix only).
#![cfg(unix)]

use std::{
    path::PathBuf,
    sync::{
        atomic::{AtomicU64, Ordering},
        Arc,
    },
    time::{Duration, Instant},
};
use tessera::{
    data::{Column, ColumnId, Dataset, RevisionId, MAX_OWNED_BYTES},
    error::AppError,
    jobs::{JobEvent, JobOperation, JobOutput, JobRunner, JobTask},
    operation::JobToken,
    project::Project,
    recovery::RecoveryStore,
};
static NEXT: AtomicU64 = AtomicU64::new(0);
struct Temp(PathBuf);
impl Temp {
    fn new() -> Self {
        let p = std::env::temp_dir().canonicalize().unwrap().join(format!(
            "tessera-recovery-jobs-{}-{}",
            std::process::id(),
            NEXT.fetch_add(1, Ordering::Relaxed)
        ));
        std::fs::create_dir(&p).unwrap();
        Self(p)
    }
}
impl Drop for Temp {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}
fn source() -> Arc<Project> {
    Arc::new(
        Project::new(
            "Recovery",
            Dataset::new(vec![
                Column::numeric(ColumnId(1), "Score", vec![Some(3.0)]).unwrap()
            ])
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
fn checkpoint_never_reports_a_user_save_and_budget_fails_before_writing() {
    let temp = Temp::new();
    let store = RecoveryStore::open_at(&temp.0.join("private")).unwrap();
    let p = source();
    let mut r = JobRunner::default();
    r.start(
        JobTask::new(JobOperation::Checkpoint {
            project: p.clone(),
            store: store.clone(),
            sequence: 1,
        }),
        token(),
    )
    .unwrap();
    let e = finish(&mut r);
    assert!(e.may_publish(1, RevisionId(1), 1));
    let JobEvent::Completed {
        result: Ok(out), ..
    } = e
    else {
        panic!("checkpoint failed")
    };
    assert!(out.file_outcome().is_none());
    assert!(matches!(out, JobOutput::Checkpoint(_)));
    assert!(store.path().exists());
    let before = std::fs::read(store.path()).unwrap();
    let mut t = JobTask::new(JobOperation::Checkpoint {
        project: p,
        store: store.clone(),
        sequence: 2,
    });
    t.retained.extra_bytes = MAX_OWNED_BYTES;
    assert!(matches!(
        r.start(t, token()),
        Err(AppError::SizeLimit { .. })
    ));
    assert_eq!(std::fs::read(store.path()).unwrap(), before);
}
#[test]
fn recovered_project_is_an_unsaved_token_gated_output() {
    let temp = Temp::new();
    let store = RecoveryStore::open_at(&temp.0.join("private")).unwrap();
    store.checkpoint(&source(), 1, &Default::default()).unwrap();
    drop(store);
    let store = RecoveryStore::open_at(&temp.0.join("private")).unwrap();
    let entry = store.list().unwrap().remove(0);
    let mut r = JobRunner::default();
    r.start(
        JobTask::new(JobOperation::Recover { store, entry }),
        token(),
    )
    .unwrap();
    let e = finish(&mut r);
    assert!(!e.may_publish(1, RevisionId(1), 2));
    let JobEvent::Completed {
        result: Ok(out), ..
    } = e
    else {
        panic!("recover failed")
    };
    assert!(out.file_outcome().is_none());
    let JobOutput::Recovered(p) = out else {
        panic!("wrong output")
    };
    assert_eq!(p.title(), "Recovery");
    assert_eq!(p.dataset().row_count(), 1);
}
#[test]
fn retire_checkpoint_is_a_background_non_save_outcome() {
    let temp = Temp::new();
    let store = RecoveryStore::open_at(&temp.0.join("private")).unwrap();
    store.checkpoint(&source(), 1, &Default::default()).unwrap();
    let path = store.path().to_path_buf();
    let mut r = JobRunner::default();
    r.start(
        JobTask::new(JobOperation::RetireCheckpoint {
            store: store.clone(),
        }),
        token(),
    )
    .unwrap();
    let e = finish(&mut r);
    let JobEvent::Completed {
        result: Ok(out), ..
    } = e
    else {
        panic!("retire failed")
    };
    assert!(out.file_outcome().is_none());
    assert!(matches!(out, JobOutput::RecoveryRetired));
    assert!(!String::from_utf8(std::fs::read(&path).unwrap())
        .unwrap()
        .contains("Recovery"));
    drop(store);
    let next = RecoveryStore::open_at(&temp.0.join("private")).unwrap();
    assert!(next.list().unwrap().is_empty());
}
#[test]
fn discard_recovery_removes_snapshot_from_discovery() {
    let temp = Temp::new();
    let store = RecoveryStore::open_at(&temp.0.join("private")).unwrap();
    store.checkpoint(&source(), 1, &Default::default()).unwrap();
    drop(store);
    let store = RecoveryStore::open_at(&temp.0.join("private")).unwrap();
    let entry = store.list().unwrap().remove(0);
    let mut r = JobRunner::default();
    r.start(
        JobTask::new(JobOperation::DiscardRecovery {
            store: store.clone(),
            entry,
        }),
        token(),
    )
    .unwrap();
    let e = finish(&mut r);
    let JobEvent::Completed {
        result: Ok(out), ..
    } = e
    else {
        panic!("discard failed")
    };
    assert!(out.file_outcome().is_none());
    assert!(matches!(out, JobOutput::RecoveryDiscarded));
    assert!(store.list().unwrap().is_empty());
}
#[test]
fn discovery_runs_in_worker_and_returns_only_abandoned_snapshots() {
    let temp = Temp::new();
    let store = RecoveryStore::open_at(&temp.0.join("private")).unwrap();
    store.checkpoint(&source(), 1, &Default::default()).unwrap();
    drop(store);
    let store = RecoveryStore::open_at(&temp.0.join("private")).unwrap();
    let mut r = JobRunner::default();
    r.start(
        JobTask::new(JobOperation::DiscoverRecovery { store }),
        token(),
    )
    .unwrap();
    let JobEvent::Completed {
        result: Ok(JobOutput::RecoveryEntries(entries)),
        ..
    } = finish(&mut r)
    else {
        panic!("discovery failed")
    };
    assert_eq!(entries.len(), 1);
}
