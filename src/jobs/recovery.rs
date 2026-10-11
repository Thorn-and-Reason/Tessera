// Published recovery checkpoints require verified directory durability (Unix only).
#![cfg(unix)]

use super::*;
#[test]
fn retirement_cancelled_before_domain_work_preserves_checkpoint() {
    static NEXT: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
    let path = std::env::temp_dir().canonicalize().unwrap().join(format!(
        "tessera-retire-cancel-{}-{}",
        std::process::id(),
        NEXT.fetch_add(1, std::sync::atomic::Ordering::Relaxed)
    ));
    let store = RecoveryStore::open_at(&path).unwrap();
    let project = Project::new(
        "Cancelled retirement",
        Dataset::new(vec![crate::data::Column::numeric(
            ColumnId(1),
            "Value",
            vec![Some(1.0)],
        )
        .unwrap()])
        .unwrap(),
    )
    .unwrap();
    store
        .checkpoint(&project, 1, &JobControl::default())
        .unwrap();
    let before = std::fs::read(store.path()).unwrap();
    let task = JobTask::new(JobOperation::RetireCheckpoint {
        store: store.clone(),
    });
    let control = JobControl::default();
    control.cancel();
    assert!(matches!(task.execute(&control), Err(AppError::Cancelled)));
    assert_eq!(std::fs::read(store.path()).unwrap(), before);
    drop(task);
    drop(store);
    std::fs::remove_dir_all(&path).unwrap();
}
