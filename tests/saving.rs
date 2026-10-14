mod common;
use std::{
    fs,
    path::PathBuf,
    sync::atomic::{AtomicU64, Ordering},
};
use tessera::{
    data::Cell,
    error::AppError,
    files::{read_project, save_project, stamp, Publication},
    operation::JobControl,
    project::Project,
};
static NEXT: AtomicU64 = AtomicU64::new(0);
struct Temp(PathBuf);
impl Temp {
    fn new() -> Self {
        let p = std::env::temp_dir().canonicalize().unwrap().join(format!(
            "tessera-save-test-{}-{}",
            std::process::id(),
            NEXT.fetch_add(1, Ordering::Relaxed)
        ));
        fs::create_dir(&p).unwrap();
        Self(p)
    }
}
impl Drop for Temp {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}
fn project() -> Project {
    Project::new("Saving test", common::sample_dataset()).unwrap()
}
#[test]
fn save_reopen_and_cleanup_durable_displaced_version() {
    let temp = Temp::new();
    let path = temp.0.join("project.tessera");
    let p = project();
    let control = JobControl::default();
    let first = save_project(&path, &p, None, &control).unwrap();
    assert_eq!(first.publication, Publication::Committed);
    #[cfg(windows)]
    let prior = fs::read(&path).unwrap();
    let (opened, expected) = read_project(&path, &control).unwrap();
    assert_eq!(opened.title(), p.title());
    let edited = p
        .dataset()
        .edit_cell(tessera::data::ColumnId(2), 0, Cell::Number(99.0))
        .unwrap();
    let next = p.with_dataset(edited).unwrap();
    let saved = save_project(&path, &next, Some(&expected), &control).unwrap();
    assert_eq!(saved.publication, Publication::Committed);
    assert_eq!(saved.committed_stamp, Some(stamp(&path).unwrap()));
    #[cfg(unix)]
    {
        assert!(saved.recovery.is_none());
        assert!(fs::read_dir(&temp.0)
            .unwrap()
            .all(|entry| !entry.unwrap().path().is_dir()));
    }
    #[cfg(windows)]
    assert_eq!(
        fs::read(saved.recovery.unwrap().displaced.unwrap()).unwrap(),
        prior
    );
    assert_eq!(
        read_project(&path, &control)
            .unwrap()
            .0
            .dataset()
            .cell(tessera::data::ColumnId(2), 0)
            .unwrap(),
        Cell::Number(99.0)
    );
}
#[test]
fn external_change_before_or_during_commit_preserves_both() {
    let t = Temp::new();
    let path = t.0.join("p.tessera");
    let p = project();
    let c = JobControl::default();
    save_project(&path, &p, None, &c).unwrap();
    let expected = stamp(&path).unwrap();
    fs::write(&path, b"external complete file").unwrap();
    assert!(matches!(
        save_project(&path, &p, Some(&expected), &c),
        Err(AppError::Conflict(_))
    ));
    assert_eq!(fs::read(&path).unwrap(), b"external complete file");
}
#[test]
fn cancel_and_failure_leave_complete_files() {
    let t = Temp::new();
    let path = t.0.join("p.tessera");
    let p = project();
    let c = JobControl::default();
    c.cancel();
    assert!(matches!(
        save_project(&path, &p, None, &c),
        Err(AppError::Cancelled)
    ));
    assert!(!path.exists());
    assert!(save_project(&t.0.join("missing/p"), &p, None, &JobControl::default()).is_err());
}
#[test]
fn existing_save_as_target_and_disappearance_never_clobber() {
    let t = Temp::new();
    let path = t.0.join("p.tessera");
    fs::write(&path, b"unrelated").unwrap();
    let p = project();
    let c = JobControl::default();
    assert!(matches!(
        save_project(&path, &p, None, &c),
        Err(AppError::Conflict(_))
    ));
    assert_eq!(fs::read(&path).unwrap(), b"unrelated");
    let expected = stamp(&path).unwrap();
    fs::remove_file(&path).unwrap();
    assert!(matches!(
        save_project(&path, &p, Some(&expected), &c),
        Err(AppError::Conflict(_))
    ));
    assert!(!path.exists());
}
#[cfg(unix)]
#[test]
fn symlinks_are_rejected_in_destination_and_ancestors() {
    let t = Temp::new();
    let target = t.0.join("real");
    fs::write(&target, b"keep").unwrap();
    let link = t.0.join("link");
    std::os::unix::fs::symlink(&target, &link).unwrap();
    assert!(stamp(&link).is_err());
    assert!(save_project(&link, &project(), None, &JobControl::default()).is_err());
    let dir = t.0.join("dir");
    std::os::unix::fs::symlink(&t.0, &dir).unwrap();
    assert!(save_project(&dir.join("new"), &project(), None, &JobControl::default()).is_err());
    assert_eq!(fs::read(target).unwrap(), b"keep");
}

#[test]
fn retained_work_budget_blocks_saves_and_opens_before_destination_changes() {
    use tessera::{
        data::MAX_OWNED_BYTES,
        files::{read_project_with_budget, save_project_with_budget},
    };
    let t = Temp::new();
    let path = t.0.join("p.tessera");
    let project = project();
    let c = JobControl::default();
    assert!(matches!(
        save_project_with_budget(&path, &project, None, &c, &[], MAX_OWNED_BYTES),
        Err(AppError::SizeLimit { .. })
    ));
    assert!(!path.exists());
    save_project(&path, &project, None, &c).unwrap();
    let bytes = fs::read(&path).unwrap();
    assert!(matches!(
        read_project_with_budget(&path, &c, &[], MAX_OWNED_BYTES),
        Err(AppError::SizeLimit { .. })
    ));
    assert_eq!(fs::read(&path).unwrap(), bytes);
}

#[test]
fn final_save_reopen_feasibility_preserves_destination_on_budget_failure() {
    use tessera::data::{Column, ColumnId, Dataset};
    let temp = Temp::new();
    let path = temp.0.join("project.tessera");
    let control = JobControl::default();
    let small = project();
    save_project(&path, &small, None, &control).unwrap();
    let prior = fs::read(&path).unwrap();
    let expected = stamp(&path).unwrap();
    let large = Project::new(
        "Large",
        Dataset::new(vec![Column::text(
            ColumnId(1),
            "Text",
            vec![Some("a".repeat(80 * 1024 * 1024))],
        )
        .unwrap()])
        .unwrap(),
    )
    .unwrap();
    assert!(matches!(
        save_project(&path, &large, Some(&expected), &control),
        Err(AppError::SizeLimit { .. })
    ));
    assert_eq!(fs::read(&path).unwrap(), prior);
    assert_eq!(stamp(&path).unwrap(), expected);
    assert_eq!(
        large
            .dataset()
            .column(ColumnId(1))
            .unwrap()
            .text_value(0)
            .unwrap()
            .unwrap()
            .len(),
        80 * 1024 * 1024
    );
    let absent = temp.0.join("new.tessera");
    assert!(save_project(&absent, &large, None, &control).is_err());
    assert!(!absent.exists());
}

#[test]
fn final_accepted_large_text_saves_reopen_and_escaped_limit_preserves_external_file() {
    use tessera::data::{Column, ColumnId, Dataset};
    let temp = Temp::new();
    let path = temp.0.join("text.tessera");
    let control = JobControl::default();
    for (unit, count) in [("a", 50 * 1024 * 1024), ("\"\n\\", 8 * 1024 * 1024)] {
        let source = Project::new(
            "Large text",
            Dataset::new(vec![Column::text(
                ColumnId(1),
                "Text",
                vec![Some(unit.repeat(count))],
            )
            .unwrap()])
            .unwrap(),
        )
        .unwrap();
        let saved = tessera::files::save_project_with_budget(
            &path,
            &source,
            None,
            &control,
            &[],
            4 * 1024 * 1024,
        )
        .unwrap();
        assert_eq!(saved.publication, Publication::Committed);
        let reopened = read_project(&path, &control).unwrap().0;
        assert_eq!(
            reopened
                .dataset()
                .column(ColumnId(1))
                .unwrap()
                .text_value(0)
                .unwrap(),
            source
                .dataset()
                .column(ColumnId(1))
                .unwrap()
                .text_value(0)
                .unwrap()
        );
        fs::remove_file(&path).unwrap();
    }
    fs::write(&path, b"external version").unwrap();
    let expected = stamp(&path).unwrap();
    let escaped = Project::new(
        "Escaped",
        Dataset::new(vec![Column::text(
            ColumnId(1),
            "Text",
            vec![Some("\"".repeat(50 * 1024 * 1024))],
        )
        .unwrap()])
        .unwrap(),
    )
    .unwrap();
    assert!(matches!(
        save_project(&path, &escaped, Some(&expected), &control),
        Err(AppError::SizeLimit { .. })
    ));
    assert_eq!(fs::read(&path).unwrap(), b"external version");
}
