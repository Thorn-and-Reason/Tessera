// Published recovery checkpoints require verified directory durability (Unix only).
#![cfg(unix)]

use std::{
    fs,
    path::PathBuf,
    sync::atomic::{AtomicU64, Ordering},
};
use tessera::{data::Dataset, operation::JobControl, project::Project, recovery::RecoveryStore};
struct Temp(PathBuf);
impl Temp {
    fn new() -> Self {
        static NEXT: AtomicU64 = AtomicU64::new(0);
        let parent = std::env::temp_dir().canonicalize().unwrap();
        Self(parent.join(format!(
            "tessera-recovery-test-{}-{}",
            std::process::id(),
            NEXT.fetch_add(1, Ordering::Relaxed)
        )))
    }
}
impl Drop for Temp {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}
fn project(title: &str) -> Project {
    Project::new(title, Dataset::new(vec![]).unwrap()).unwrap()
}
#[test]
fn successful_checkpoint_replacement_and_retirement_leave_no_snapshot_backups() {
    let temp = Temp::new();
    let store = RecoveryStore::open_at(&temp.0).unwrap();
    for sequence in 1..=3 {
        let outcome = store
            .checkpoint(
                &project(&format!("Private version {sequence}")),
                sequence,
                &JobControl::default(),
            )
            .unwrap();
        assert!(outcome.recovery.is_none());
        assert!(fs::read_dir(store.path().parent().unwrap())
            .unwrap()
            .all(|entry| !entry.unwrap().path().is_dir()));
    }
    store.discard_owned(&JobControl::default()).unwrap();
    assert!(fs::read_dir(store.path().parent().unwrap())
        .unwrap()
        .all(|entry| !entry.unwrap().path().is_dir()));
    drop(store);
    let next = RecoveryStore::open_at(&temp.0).unwrap();
    assert!(next.list().unwrap().is_empty());
}
#[test]
fn explicit_discard_of_recovered_entry_leaves_no_snapshot_backups() {
    let temp = Temp::new();
    let first = RecoveryStore::open_at(&temp.0).unwrap();
    first
        .checkpoint(&project("Private snapshot"), 1, &JobControl::default())
        .unwrap();
    let session = first.path().parent().unwrap().to_owned();
    drop(first);
    let second = RecoveryStore::open_at(&temp.0).unwrap();
    let entries = second.list().unwrap();
    second
        .discard_entry(&entries[0], &JobControl::default(), &[], 0)
        .unwrap();
    assert!(fs::read_dir(session)
        .unwrap()
        .all(|entry| !entry.unwrap().path().is_dir()));
}
#[test]
fn live_sessions_are_hidden_and_crashed_sessions_recover() {
    let temp = Temp::new();
    let store = RecoveryStore::open_at(&temp.0).unwrap();
    store
        .checkpoint(&project("Unsaved"), 1, &JobControl::default())
        .unwrap();
    assert!(store.list().unwrap().is_empty());
    drop(store);
    let next = RecoveryStore::open_at(&temp.0).unwrap();
    let entries = next.list().unwrap();
    assert_eq!(entries.len(), 1);
    assert_eq!(
        next.recover(&entries[0], &JobControl::default())
            .unwrap()
            .title(),
        "Unsaved"
    );
    assert!(next.list().unwrap().is_empty());
}
#[test]
fn stale_and_cancelled_work_preserve_latest_checkpoint() {
    let temp = Temp::new();
    let store = RecoveryStore::open_at(&temp.0).unwrap();
    store
        .checkpoint(&project("Latest"), 2, &JobControl::default())
        .unwrap();
    assert!(store
        .checkpoint(&project("Old"), 1, &JobControl::default())
        .is_err());
    let cancelled = JobControl::default();
    cancelled.cancel();
    assert!(store
        .checkpoint(&project("Cancelled"), 3, &cancelled)
        .is_err());
    let path = store.path().to_owned();
    drop(store);
    let (saved, _) = tessera::files::read_project(&path, &JobControl::default()).unwrap();
    assert_eq!(saved.title(), "Latest");
}
#[test]
fn clean_discard_removes_only_own_checkpoint() {
    let temp = Temp::new();
    let first = RecoveryStore::open_at(&temp.0).unwrap();
    first
        .checkpoint(&project("Keep"), 1, &JobControl::default())
        .unwrap();
    let path = first.path().to_owned();
    drop(first);
    let second = RecoveryStore::open_at(&temp.0).unwrap();
    second
        .checkpoint(&project("Discard"), 1, &JobControl::default())
        .unwrap();
    second.discard_owned(&JobControl::default()).unwrap();
    assert_eq!(
        fs::read(second.path()).unwrap(),
        br#"{"format":"tessera-discarded","version":1}"#
    );
    assert!(path.exists());
    assert_eq!(second.list().unwrap().len(), 1);
}
#[cfg(unix)]
#[test]
fn public_or_symlinked_roots_are_rejected() {
    use std::os::unix::fs::{symlink, PermissionsExt};
    let temp = Temp::new();
    fs::create_dir(&temp.0).unwrap();
    fs::set_permissions(&temp.0, fs::Permissions::from_mode(0o755)).unwrap();
    assert!(RecoveryStore::open_at(&temp.0).is_err());
    let link = Temp::new();
    symlink(&temp.0, &link.0).unwrap();
    assert!(RecoveryStore::open_at(&link.0).is_err());
}
#[test]
fn replaced_checkpoint_is_not_discarded() {
    let temp = Temp::new();
    let store = RecoveryStore::open_at(&temp.0).unwrap();
    store
        .checkpoint(&project("Original"), 1, &JobControl::default())
        .unwrap();
    fs::write(store.path(), b"changed").unwrap();
    assert!(store.discard_owned(&JobControl::default()).is_err());
    assert_eq!(fs::read(store.path()).unwrap(), b"changed");
}
#[test]
fn concurrent_out_of_order_requests_keep_highest_sequence() {
    let temp = Temp::new();
    let store = RecoveryStore::open_at(&temp.0).unwrap();
    std::thread::scope(|scope| {
        for sequence in 1..=16 {
            let store = store.clone();
            scope.spawn(move || {
                let result = store.checkpoint(
                    &project(&format!("Version {sequence}")),
                    sequence,
                    &JobControl::default(),
                );
                assert!(
                    result.is_ok() || matches!(result, Err(tessera::error::AppError::Cancelled))
                );
            });
        }
    });
    let (saved, _) = tessera::files::read_project(store.path(), &JobControl::default()).unwrap();
    assert_eq!(saved.title(), "Version 16");
}
#[cfg(unix)]
#[test]
fn symlinked_checkpoint_is_rejected_without_reading_target() {
    use std::os::unix::fs::symlink;
    let temp = Temp::new();
    let source = Temp::new();
    fs::write(&source.0, b"private other file").unwrap();
    let store = RecoveryStore::open_at(&temp.0).unwrap();
    store
        .checkpoint(&project("Keep"), 1, &JobControl::default())
        .unwrap();
    let checkpoint = store.path().to_owned();
    drop(store);
    fs::remove_file(&checkpoint).unwrap();
    symlink(&source.0, &checkpoint).unwrap();
    let next = RecoveryStore::open_at(&temp.0).unwrap();
    assert!(next.list().is_err());
    assert_eq!(fs::read(&source.0).unwrap(), b"private other file");
}
#[test]
fn scan_limit_is_explicit_and_never_deletes_excess_files() {
    let temp = Temp::new();
    let store = RecoveryStore::open_at(&temp.0).unwrap();
    for i in 0..4096 {
        fs::write(temp.0.join(format!("unrelated-{i}")), b"keep").unwrap();
    }
    assert!(matches!(
        store.list(),
        Err(tessera::error::AppError::SizeLimit { limit: 4096 })
    ));
    assert_eq!(fs::read(temp.0.join("unrelated-4095")).unwrap(), b"keep");
}
#[test]
fn recovery_entry_cannot_be_redirected_to_another_session() {
    let temp = Temp::new();
    let first = RecoveryStore::open_at(&temp.0).unwrap();
    first
        .checkpoint(&project("Original"), 1, &JobControl::default())
        .unwrap();
    drop(first);
    let active = RecoveryStore::open_at(&temp.0).unwrap();
    active
        .checkpoint(&project("Live"), 1, &JobControl::default())
        .unwrap();
    let mut entry = active.list().unwrap().remove(0);
    entry.path = active.path().to_owned();
    assert!(active.recover(&entry, &JobControl::default()).is_err());
}
#[test]
fn retired_checkpoint_can_be_replaced_by_a_later_edit() {
    let temp = Temp::new();
    let store = RecoveryStore::open_at(&temp.0).unwrap();
    store
        .checkpoint(&project("Saved"), 1, &JobControl::default())
        .unwrap();
    store.discard_owned(&JobControl::default()).unwrap();
    store
        .checkpoint(&project("New unsaved edit"), 2, &JobControl::default())
        .unwrap();
    drop(store);
    let next = RecoveryStore::open_at(&temp.0).unwrap();
    let entries = next.list().unwrap();
    assert_eq!(entries.len(), 1);
    assert_eq!(
        next.recover(&entries[0], &JobControl::default())
            .unwrap()
            .title(),
        "New unsaved edit"
    );
}
#[test]
fn malformed_checkpoint_is_reported_instead_of_treated_as_discarded() {
    let temp = Temp::new();
    let store = RecoveryStore::open_at(&temp.0).unwrap();
    store
        .checkpoint(&project("Unsaved"), 1, &JobControl::default())
        .unwrap();
    fs::write(
        store.path(),
        br#"{"format":"tessera-discarded","version":2}"#,
    )
    .unwrap();
    drop(store);
    let next = RecoveryStore::open_at(&temp.0).unwrap();
    let entries = next.list().unwrap();
    assert_eq!(entries.len(), 1);
    assert!(next.recover(&entries[0], &JobControl::default()).is_err());
    assert!(entries[0].path.exists());
}
#[test]
fn explicitly_discarded_crash_entry_is_retired_without_touching_other_sessions() {
    let temp = Temp::new();
    let old = RecoveryStore::open_at(&temp.0).unwrap();
    old.checkpoint(&project("Discard old"), 1, &JobControl::default())
        .unwrap();
    drop(old);
    let current = RecoveryStore::open_at(&temp.0).unwrap();
    current
        .checkpoint(&project("Keep current"), 1, &JobControl::default())
        .unwrap();
    let entry = current.list().unwrap().remove(0);
    current
        .discard_entry(&entry, &JobControl::default(), &[], 0)
        .unwrap();
    drop(entry);
    assert!(current.list().unwrap().is_empty());
    assert_eq!(
        tessera::files::read_project(current.path(), &JobControl::default())
            .unwrap()
            .0
            .title(),
        "Keep current"
    );
}
#[test]
fn changed_discovered_entry_is_preserved_and_cannot_be_recovered_as_original() {
    let temp = Temp::new();
    let old = RecoveryStore::open_at(&temp.0).unwrap();
    old.checkpoint(&project("Original"), 1, &JobControl::default())
        .unwrap();
    drop(old);
    let current = RecoveryStore::open_at(&temp.0).unwrap();
    let entry = current.list().unwrap().remove(0);
    let stamp = tessera::files::stamp(&entry.path).unwrap();
    tessera::files::save_project(
        &entry.path,
        &project("Changed"),
        Some(&stamp),
        &JobControl::default(),
    )
    .unwrap();
    assert!(current.recover(&entry, &JobControl::default()).is_err());
    assert!(current
        .discard_entry(&entry, &JobControl::default(), &[], 0)
        .is_err());
    assert_eq!(
        tessera::files::read_project(&entry.path, &JobControl::default())
            .unwrap()
            .0
            .title(),
        "Changed"
    );
}
#[test]
fn retirement_budget_failure_preserves_recoverable_data() {
    let temp = Temp::new();
    let store = RecoveryStore::open_at(&temp.0).unwrap();
    store
        .checkpoint(&project("Keep"), 1, &JobControl::default())
        .unwrap();
    assert!(matches!(
        store.discard_owned_with_budget(&JobControl::default(), &[], usize::MAX),
        Err(tessera::error::AppError::SizeLimit { .. })
    ));
    assert_eq!(
        tessera::files::read_project(store.path(), &JobControl::default())
            .unwrap()
            .0
            .title(),
        "Keep"
    );
}
#[test]
fn cancelled_discovery_releases_all_claimed_entries() {
    let temp = Temp::new();
    let store = RecoveryStore::open_at(&temp.0).unwrap();
    store
        .checkpoint(&project("Keep"), 1, &JobControl::default())
        .unwrap();
    drop(store);
    let current = RecoveryStore::open_at(&temp.0).unwrap();
    let cancel = JobControl::default();
    cancel.cancel();
    assert!(matches!(
        current.list_with_control(&cancel),
        Err(tessera::error::AppError::Cancelled)
    ));
    assert_eq!(current.list().unwrap().len(), 1);
}
#[test]
fn changed_retired_marker_is_preserved_and_reported() {
    let temp = Temp::new();
    let store = RecoveryStore::open_at(&temp.0).unwrap();
    store
        .checkpoint(&project("Saved"), 1, &JobControl::default())
        .unwrap();
    store.discard_owned(&JobControl::default()).unwrap();
    std::fs::write(store.path(), b"external change after retirement").unwrap();
    assert!(store.discard_owned(&JobControl::default()).is_err());
    assert_eq!(
        std::fs::read(store.path()).unwrap(),
        b"external change after retirement"
    );
}
