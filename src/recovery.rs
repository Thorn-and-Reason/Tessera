//! Private crash checkpoints never change the identity of the user's saved project.
use crate::{
    data::Dataset,
    error::{AppError, AppResult},
    files::{self, native::Directory, FileStamp, Publication, SaveOutcome},
    operation::JobControl,
    project::Project,
};
use std::{
    ffi::OsStr,
    fs::File,
    path::{Component, Path, PathBuf},
    sync::{
        atomic::{AtomicU64, Ordering},
        Arc, Mutex,
    },
    time::{SystemTime, UNIX_EPOCH},
};
#[cfg(windows)]
mod windows;
const MAX_SCAN: usize = 4096;
const MAX_ENTRIES: usize = 128;
const MAX_LIST_BYTES: usize = 512 * 1024;
/// Retained entries plus streaming digest and native path traversal work.
pub const DISCOVERY_WORK_BYTES: usize = MAX_LIST_BYTES + 256 * 1024;
const CHECKPOINT: &str = "checkpoint.tessera";
const RETIRED: &[u8] = br#"{"format":"tessera-discarded","version":1}"#;
const LOCK: &str = ".session.lock";
static NEXT: AtomicU64 = AtomicU64::new(0);
#[derive(Clone)]
pub struct RecoveryStore(Arc<StoreInner>);
struct StoreInner {
    root: Directory,
    session: Directory,
    _lock: File,
    path: PathBuf,
    state: Mutex<CheckpointState>,
}
#[derive(Default)]
struct CheckpointState {
    sequence: u64,
    stamp: Option<FileStamp>,
    retired: bool,
    uncertain: bool,
}
impl CheckpointState {
    fn observe(&mut self, outcome: &SaveOutcome, retired: bool) {
        if let Some(stamp) = &outcome.committed_stamp {
            self.stamp = Some(stamp.clone());
        }
        self.uncertain =
            outcome.publication != Publication::Committed || outcome.committed_stamp.is_none();
        self.retired = retired && !self.uncertain;
    }
}
#[derive(Clone)]
pub struct RecoveryEntry {
    pub path: PathBuf,
    pub modified: Option<SystemTime>,
    owner: Arc<EntryOwner>,
}
struct EntryOwner {
    directory: Directory,
    _lock: File,
    stamp: FileStamp,
}
fn private(directory: &Directory) -> AppResult<()> {
    directory.verify_path(&directory.path)?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::MetadataExt;
        let m = directory.metadata()?;
        // SAFETY: geteuid has no arguments or memory requirements.
        if m.uid() != unsafe { libc::geteuid() } || m.mode() & 0o077 != 0 {
            return Err(AppError::invalid(
                "Recovery",
                "The recovery folder must be private to your account.",
            ));
        }
    }
    #[cfg(windows)]
    windows::verify(&directory.path)?;
    Ok(())
}
fn directory(path: &Path) -> AppResult<Directory> {
    if !path.is_absolute()
        || path.components().any(|c| {
            !matches!(
                c,
                Component::RootDir | Component::Prefix(_) | Component::Normal(_)
            )
        })
    {
        return Err(AppError::invalid(
            "Recovery",
            "Use an absolute recovery folder without parent components.",
        ));
    }
    match Directory::open(path) {
        Ok(dir) => Ok(dir),
        Err(AppError::Io(e)) if e.kind() == std::io::ErrorKind::NotFound => {
            let parent = path
                .parent()
                .ok_or_else(|| AppError::invalid("Recovery", "Recovery parent is unavailable."))?;
            let parent = directory(parent)?;
            let leaf = path
                .file_name()
                .ok_or_else(|| AppError::invalid("Recovery", "Recovery folder is unavailable."))?;
            match parent.create_dir(leaf) {
                Ok(dir) => {
                    parent.sync()?;
                    Ok(dir)
                }
                Err(AppError::Io(e)) if e.kind() == std::io::ErrorKind::AlreadyExists => {
                    Directory::open(path)
                }
                Err(e) => Err(e),
            }
        }
        Err(e) => Err(e),
    }
}
fn variable(name: &str) -> AppResult<PathBuf> {
    let path = std::env::var_os(name)
        .map(PathBuf::from)
        .filter(|p| p.is_absolute())
        .ok_or_else(|| {
            AppError::invalid(
                "Recovery",
                format!("A valid {name} folder is required for crash recovery."),
            )
        })?;
    Ok(path)
}
fn native_path() -> AppResult<PathBuf> {
    #[cfg(target_os = "macos")]
    {
        Ok(variable("HOME")?.join("Library/Application Support/Tessera/Recovery"))
    }
    #[cfg(target_os = "linux")]
    {
        let base = match std::env::var_os("XDG_STATE_HOME") {
            Some(v) if !v.is_empty() => {
                let p = PathBuf::from(v);
                if !p.is_absolute() {
                    return Err(AppError::invalid(
                        "Recovery",
                        "XDG_STATE_HOME must be absolute.",
                    ));
                }
                p
            }
            _ => variable("HOME")?.join(".local/state"),
        };
        Ok(base.join("tessera/recovery"))
    }
    #[cfg(target_os = "windows")]
    {
        Ok(variable("LOCALAPPDATA")?.join("Tessera/Recovery"))
    }
}
fn conflict() -> AppError {
    AppError::Conflict("The crash checkpoint changed. It has been preserved.".into())
}
impl RecoveryStore {
    pub fn open_native() -> AppResult<Self> {
        Self::open_at(&native_path()?)
    }
    pub fn open_at(path: &Path) -> AppResult<Self> {
        let root = directory(path)?;
        private(&root)?;
        for _ in 0..8 {
            let nonce = NEXT
                .fetch_update(Ordering::Relaxed, Ordering::Relaxed, |v| v.checked_add(1))
                .map_err(|_| AppError::IdentityExhausted)?;
            let time = SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .map_err(|_| AppError::invalid("Recovery", "The system clock is unavailable."))?
                .as_nanos();
            let name = format!("session-{time:x}-{:x}-{nonce:x}", std::process::id());
            let session = match root.create_dir(OsStr::new(&name)) {
                Ok(d) => d,
                Err(AppError::Io(e)) if e.kind() == std::io::ErrorKind::AlreadyExists => continue,
                Err(e) => return Err(e),
            };
            private(&session)?;
            root.sync()?;
            drop(session.create(OsStr::new(LOCK))?);
            let lock = session.read(OsStr::new(LOCK))?;
            lock.try_lock().map_err(std::io::Error::from)?;
            let path = session.path.join(CHECKPOINT);
            return Ok(Self(Arc::new(StoreInner {
                root,
                session,
                _lock: lock,
                path,
                state: Mutex::new(CheckpointState::default()),
            })));
        }
        Err(AppError::Conflict(
            "Could not create a unique recovery session.".into(),
        ))
    }
    pub fn path(&self) -> &Path {
        &self.0.path
    }
    fn state(&self, control: &JobControl) -> AppResult<std::sync::MutexGuard<'_, CheckpointState>> {
        loop {
            control.checkpoint(0, None)?;
            match self.0.state.try_lock() {
                Ok(state) => return Ok(state),
                Err(std::sync::TryLockError::Poisoned(_)) => return Err(conflict()),
                Err(std::sync::TryLockError::WouldBlock) => {
                    std::thread::sleep(std::time::Duration::from_millis(10))
                }
            }
        }
    }
    pub fn same_owner(&self, other: &Self) -> bool {
        Arc::ptr_eq(&self.0, &other.0)
    }
    pub fn owned_bytes(&self) -> usize {
        std::mem::size_of::<StoreInner>()
            + 2 * std::mem::size_of::<usize>()
            + self.0.root.heap_bytes()
            + self.0.session.heap_bytes()
            + self.0.path.capacity()
    }
    pub fn checkpoint(
        &self,
        project: &Project,
        sequence: u64,
        control: &JobControl,
    ) -> AppResult<SaveOutcome> {
        self.checkpoint_with_budget(project, sequence, control, &[], 0)
    }
    pub fn checkpoint_with_budget(
        &self,
        project: &Project,
        sequence: u64,
        control: &JobControl,
        retained: &[&Dataset],
        extra_bytes: usize,
    ) -> AppResult<SaveOutcome> {
        let mut state = self.state(control)?;
        if state.uncertain {
            return Err(conflict());
        }
        if sequence <= state.sequence {
            return Err(AppError::Cancelled);
        }
        state.sequence = sequence;
        private(&self.0.session)?;
        control.checkpoint(0, None)?;
        let outcome = files::save_project_with_budget(
            &self.0.path,
            project,
            state.stamp.as_ref(),
            control,
            retained,
            extra_bytes,
        )?;
        state.observe(&outcome, false);
        Ok(outcome)
    }
    pub fn list(&self) -> AppResult<Vec<RecoveryEntry>> {
        self.list_with_control(&JobControl::default())
    }
    pub fn list_with_control(&self, control: &JobControl) -> AppResult<Vec<RecoveryEntry>> {
        private(&self.0.root)?;
        let mut entries = Vec::new();
        let mut bytes = MAX_ENTRIES * std::mem::size_of::<RecoveryEntry>();
        for (count, item) in std::fs::read_dir(&self.0.root.path)?.enumerate() {
            control.checkpoint(count, None)?;
            if count >= MAX_SCAN {
                return Err(AppError::SizeLimit { limit: MAX_SCAN });
            }
            let item = item?;
            let name = item.file_name();
            let Some(name) = name.to_str() else { continue };
            if !name.starts_with("session-")
                || !name[8..]
                    .bytes()
                    .all(|b| b.is_ascii_hexdigit() || b == b'-')
            {
                continue;
            }
            if !item.file_type()?.is_dir() {
                continue;
            }
            let directory = Directory::open(&item.path())?;
            private(&directory)?;
            let lock = match directory.read(OsStr::new(LOCK)) {
                Ok(f) => f,
                Err(AppError::Io(e)) if e.kind() == std::io::ErrorKind::NotFound => continue,
                Err(e) => return Err(e),
            };
            match lock.try_lock() {
                Ok(()) => {}
                Err(std::fs::TryLockError::WouldBlock) => continue,
                Err(std::fs::TryLockError::Error(e)) => return Err(e.into()),
            }
            let path = directory.path.join(CHECKPOINT);
            let mut checkpoint = match directory.read(OsStr::new(CHECKPOINT)) {
                Ok(f) => f,
                Err(AppError::Io(e)) if e.kind() == std::io::ErrorKind::NotFound => continue,
                Err(e) => return Err(e),
            };
            let metadata = checkpoint.metadata()?;
            if metadata.len() > crate::project_io::MAX_PROJECT_BYTES as u64 {
                return Err(AppError::SizeLimit {
                    limit: crate::project_io::MAX_PROJECT_BYTES,
                });
            }
            if metadata.len() == RETIRED.len() as u64 {
                use sha2::{Digest, Sha256};
                use std::io::Read;
                let mut marker = [0u8; RETIRED.len()];
                checkpoint.read_exact(&mut marker)?;
                if marker == RETIRED
                    && files::stamp_with_control(&path, control)?.digest
                        == <[u8; 32]>::from(Sha256::digest(RETIRED))
                {
                    continue;
                }
            }
            if entries.len() >= MAX_ENTRIES {
                return Err(AppError::SizeLimit { limit: MAX_ENTRIES });
            }
            bytes = bytes
                .checked_add(path.capacity())
                .and_then(|b| b.checked_add(directory.heap_bytes()))
                .and_then(|b| {
                    b.checked_add(
                        std::mem::size_of::<EntryOwner>() + 2 * std::mem::size_of::<usize>(),
                    )
                })
                .ok_or(AppError::SizeLimit {
                    limit: MAX_LIST_BYTES,
                })?;
            if bytes > MAX_LIST_BYTES {
                return Err(AppError::SizeLimit {
                    limit: MAX_LIST_BYTES,
                });
            }
            let stamp = files::stamp_with_control(&path, control)?;
            entries.push(RecoveryEntry {
                path,
                modified: metadata.modified().ok(),
                owner: Arc::new(EntryOwner {
                    directory,
                    _lock: lock,
                    stamp,
                }),
            });
        }
        self.0.root.verify_path(&self.0.root.path)?;
        entries.sort_by(|a, b| {
            b.modified
                .cmp(&a.modified)
                .then_with(|| a.path.cmp(&b.path))
        });
        Ok(entries)
    }
    pub fn recover(&self, entry: &RecoveryEntry, control: &JobControl) -> AppResult<Project> {
        self.recover_with_budget(entry, control, &[], 0)
    }
    pub fn recover_with_budget(
        &self,
        entry: &RecoveryEntry,
        control: &JobControl,
        retained: &[&Dataset],
        extra_bytes: usize,
    ) -> AppResult<Project> {
        if entry.path != entry.owner.directory.path.join(CHECKPOINT)
            || entry.path.parent().and_then(Path::parent) != Some(self.0.root.path.as_path())
        {
            return Err(conflict());
        }
        private(&entry.owner.directory)?;
        let (project, stamp) =
            files::read_project_with_budget(&entry.path, control, retained, extra_bytes)?;
        if stamp != entry.owner.stamp {
            return Err(conflict());
        }
        Ok(project)
    }
    pub fn discard_entry(
        &self,
        entry: &RecoveryEntry,
        control: &JobControl,
        retained: &[&Dataset],
        extra_bytes: usize,
    ) -> AppResult<()> {
        if entry.path != entry.owner.directory.path.join(CHECKPOINT)
            || entry.path.parent().and_then(Path::parent) != Some(self.0.root.path.as_path())
        {
            return Err(conflict());
        }
        private(&entry.owner.directory)?;
        files::work_budget(retained, extra_bytes)?;
        let outcome = files::publish(
            &entry.path,
            None,
            Some(&entry.owner.stamp),
            control,
            |writer| {
                writer.write_all(RETIRED)?;
                Ok(())
            },
        )?;
        if outcome.publication != Publication::Committed || outcome.committed_stamp.is_none() {
            return Err(conflict());
        }
        Ok(())
    }
    pub fn discard_owned(&self, control: &JobControl) -> AppResult<()> {
        self.discard_owned_with_budget(control, &[], 0)
    }
    pub fn discard_owned_with_budget(
        &self,
        control: &JobControl,
        retained: &[&Dataset],
        extra_bytes: usize,
    ) -> AppResult<()> {
        files::work_budget(retained, extra_bytes)?;
        let mut state = self.state(control)?;
        control.checkpoint(0, None)?;
        private(&self.0.session)?;
        let Some(expected) = state.stamp.as_ref() else {
            return if state.uncertain {
                Err(conflict())
            } else {
                Ok(())
            };
        };
        if state.retired {
            return if files::stamp_with_control(&self.0.path, control)? == *expected {
                Ok(())
            } else {
                Err(conflict())
            };
        }
        let outcome = files::publish(&self.0.path, None, Some(expected), control, |writer| {
            writer.write_all(RETIRED)?;
            Ok(())
        })?;
        state.observe(&outcome, true);
        if state.uncertain {
            return Err(conflict());
        }
        Ok(())
    }
}

impl RecoveryEntry {
    pub fn same_owner(&self, other: &Self) -> bool {
        Arc::ptr_eq(&self.owner, &other.owner)
    }
    pub fn owned_bytes(&self) -> usize {
        std::mem::size_of::<EntryOwner>()
            + 2 * std::mem::size_of::<usize>()
            + std::mem::size_of::<Self>()
            + self.path.capacity()
            + self.owner.directory.heap_bytes()
    }
}
impl std::fmt::Debug for RecoveryStore {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("RecoveryStore")
            .field("path", &self.path())
            .finish_non_exhaustive()
    }
}
impl std::fmt::Debug for RecoveryEntry {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("RecoveryEntry")
            .field("path", &self.path)
            .field("modified", &self.modified)
            .finish_non_exhaustive()
    }
}

// Published recovery checkpoints require verified directory durability (Unix only).
#[cfg(all(test, unix))]
mod outcome_tests {
    use super::*;
    use crate::data::Dataset;
    struct Temp(PathBuf);
    impl Temp {
        fn new() -> Self {
            Self(std::env::temp_dir().canonicalize().unwrap().join(format!(
                "tessera-recovery-outcome-{}-{}",
                std::process::id(),
                NEXT.fetch_add(1, Ordering::Relaxed)
            )))
        }
    }
    impl Drop for Temp {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.0);
        }
    }
    fn project(title: &str) -> Project {
        Project::new(title, Dataset::new(vec![]).unwrap()).unwrap()
    }
    #[test]
    fn uncertain_published_first_checkpoint_remains_retirable_and_blocks_new_writes() {
        for publication in [Publication::Conflict, Publication::Uncertain] {
            let temp = Temp::new();
            let store = RecoveryStore::open_at(&temp.0).unwrap();
            let mut outcome = files::save_project(
                store.path(),
                &project("Sensitive unsaved"),
                None,
                &JobControl::default(),
            )
            .unwrap();
            outcome.publication = publication;
            store.0.state.lock().unwrap().observe(&outcome, false);
            assert!(store
                .checkpoint(&project("New write"), 1, &JobControl::default())
                .is_err());
            assert_eq!(
                files::read_project(store.path(), &JobControl::default())
                    .unwrap()
                    .0
                    .title(),
                "Sensitive unsaved"
            );
            store.discard_owned(&JobControl::default()).unwrap();
            assert_eq!(std::fs::read(store.path()).unwrap(), RETIRED);
            store
                .checkpoint(&project("After retirement"), 2, &JobControl::default())
                .unwrap();
        }
    }
    #[test]
    fn unverified_first_publication_never_reports_retired_or_overwrites_checkpoint() {
        let temp = Temp::new();
        let store = RecoveryStore::open_at(&temp.0).unwrap();
        let mut outcome = files::save_project(
            store.path(),
            &project("Keep uncertain"),
            None,
            &JobControl::default(),
        )
        .unwrap();
        outcome.publication = Publication::Uncertain;
        outcome.committed_stamp = None;
        store.0.state.lock().unwrap().observe(&outcome, false);
        assert!(store.discard_owned(&JobControl::default()).is_err());
        assert!(store
            .checkpoint(&project("New"), 1, &JobControl::default())
            .is_err());
        assert_eq!(
            files::read_project(store.path(), &JobControl::default())
                .unwrap()
                .0
                .title(),
            "Keep uncertain"
        );
    }
}
