//! Recovery-aware publication. Atomic visibility is distinct from power-loss durability.
#[cfg(target_os = "macos")]
#[path = "macos.rs"]
pub(crate) mod native;
#[cfg(target_os = "windows")]
#[path = "windows.rs"]
pub(crate) mod native;
#[cfg(target_os = "linux")]
#[path = "linux.rs"]
pub(crate) mod native;
#[cfg(not(any(target_os = "macos", target_os = "windows", target_os = "linux")))]
compile_error!("Safe publication currently supports macOS, Windows and Linux only.");
#[cfg(test)]
mod tests;
use crate::{
    data::{Budget, Dataset, RevisionId, MAX_OWNED_BYTES},
    error::{AppError, AppResult},
    operation::JobControl,
    project::Project,
    project_io::{self, MAX_PROJECT_BYTES},
};
use native::Directory;
use sha2::{Digest, Sha256};
use std::{
    ffi::{OsStr, OsString},
    fs::File,
    io::{self, BufWriter, Read, Seek, SeekFrom, Write},
    path::{Path, PathBuf},
    sync::atomic::{AtomicU64, Ordering},
    time::{SystemTime, UNIX_EPOCH},
};
const BUFFER_BYTES: usize = 32 * 1024;
const FILE_WORK_BYTES: usize = 256 * 1024;
static NEXT: AtomicU64 = AtomicU64::new(0);
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct FileIdentity {
    pub volume: u64,
    pub file: u128,
}
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FileStamp {
    pub identity: FileIdentity,
    pub digest: [u8; 32],
    pub size: u64,
}
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Publication {
    Committed,
    /// A preflight or reconciliation conflict; committed_stamp records whether publication occurred.
    Conflict,
    /// Publication or current file locations could not be verified. Check committed_stamp:
    /// a present stamp still establishes publication. Surviving transaction files are kept.
    Uncertain,
}
#[derive(Debug, Clone)]
pub struct RecoveryPaths {
    pub directory: PathBuf,
    pub frozen_revision: Option<PathBuf>,
    pub displaced: Option<PathBuf>,
    pub prepared: Option<PathBuf>,
}
#[derive(Debug, Clone)]
pub struct SaveOutcome {
    pub publication: Publication,
    pub revision: Option<RevisionId>,
    /// Facts about the file installed by this operation, even if a later writer changed the destination.
    pub committed_stamp: Option<FileStamp>,
    pub destination: PathBuf,
    pub recovery: Option<RecoveryPaths>,
    pub warning: Option<String>,
}
fn split(path: &Path) -> AppResult<(Directory, OsString)> {
    let leaf = path
        .file_name()
        .filter(|name| !name.is_empty())
        .ok_or_else(|| AppError::invalid("path", "Choose a file name."))?;
    let parent = path
        .parent()
        .filter(|p| !p.as_os_str().is_empty())
        .unwrap_or_else(|| Path::new("."));
    if path.as_os_str().len() > 16 * 1024 {
        return Err(AppError::invalid("path", "The file path is too long."));
    }
    Ok((Directory::open(parent)?, leaf.to_owned()))
}
fn absent(error: &AppError) -> bool {
    matches!(error,AppError::Io(error) if error.kind()==io::ErrorKind::NotFound)
}
fn stamp_file(file: &mut File, control: &JobControl) -> AppResult<FileStamp> {
    stamp_file_with_limit(file, control, MAX_PROJECT_BYTES)
}
fn stamp_file_with_limit(
    file: &mut File,
    control: &JobControl,
    limit: usize,
) -> AppResult<FileStamp> {
    let before = file.metadata()?;
    if !before.is_file() {
        return Err(AppError::invalid("path", "Choose a regular file."));
    }
    if before.len() > limit as u64 {
        return Err(AppError::SizeLimit { limit });
    }
    let identity = native::identity(file)?;
    file.seek(SeekFrom::Start(0))?;
    let mut digest = Sha256::new();
    let mut size = 0_u64;
    let mut buffer = [0_u8; BUFFER_BYTES];
    loop {
        control.checkpoint(size as usize, Some(before.len() as usize))?;
        let count = file.read(&mut buffer)?;
        if count == 0 {
            break;
        }
        size += count as u64;
        if size > limit as u64 {
            return Err(AppError::SizeLimit { limit });
        }
        digest.update(&buffer[..count]);
    }
    let after = file.metadata()?;
    if size != before.len() || size != after.len() || before.modified()? != after.modified()? {
        return Err(AppError::Conflict(
            "The file changed while being read. Try again.".into(),
        ));
    }
    Ok(FileStamp {
        identity,
        digest: digest.finalize().into(),
        size,
    })
}
fn stamp_at(dir: &Directory, leaf: &OsStr, control: &JobControl) -> AppResult<FileStamp> {
    stamp_file(&mut dir.read(leaf)?, control)
}
fn optional_stamp(
    dir: &Directory,
    leaf: &OsStr,
    control: &JobControl,
) -> AppResult<Option<FileStamp>> {
    match stamp_at(dir, leaf, control) {
        Ok(stamp) => Ok(Some(stamp)),
        Err(error) if absent(&error) => Ok(None),
        Err(error) => Err(error),
    }
}
pub fn stamp(path: &Path) -> AppResult<FileStamp> {
    stamp_with_control(path, &JobControl::default())
}
pub(crate) fn stamp_with_limit(
    path: &Path,
    control: &JobControl,
    limit: usize,
) -> AppResult<FileStamp> {
    let (parent, leaf) = split(path)?;
    let stamp = stamp_file_with_limit(&mut parent.read(&leaf)?, control, limit)?;
    parent.verify_path(&parent.path)?;
    Ok(stamp)
}
pub(crate) fn stamp_with_control(path: &Path, control: &JobControl) -> AppResult<FileStamp> {
    let (parent, leaf) = split(path)?;
    let stamp = stamp_at(&parent, &leaf, control)?;
    parent.verify_path(&parent.path)?;
    Ok(stamp)
}
pub fn read_project(path: &Path, control: &JobControl) -> AppResult<(Project, FileStamp)> {
    read_project_with_budget(path, control, &[], 0)
}
/// Count retained datasets and non-dataset owned work, including titles/history, before opening.
pub fn read_project_with_budget(
    path: &Path,
    control: &JobControl,
    retained: &[&Dataset],
    extra_bytes: usize,
) -> AppResult<(Project, FileStamp)> {
    read_project_checked(path, control, retained, extra_bytes, || Ok(()))
}
fn read_project_checked(
    path: &Path,
    control: &JobControl,
    retained: &[&Dataset],
    extra_bytes: usize,
    after_decode: impl FnOnce() -> AppResult<()>,
) -> AppResult<(Project, FileStamp)> {
    let extra = work_budget(retained, extra_bytes)?;
    control.checkpoint(0, None)?;
    let (parent, leaf) = split(path)?;
    let mut file = parent.read(&leaf)?;
    let before = stamp_file(&mut file, control)?;
    file.seek(SeekFrom::Start(0))?;
    let mut reader = HashReader {
        file: &mut file,
        hash: Sha256::new(),
        size: 0,
    };
    let project = project_io::decode_with_budget(&mut reader, control, retained, extra)?;
    let read_digest: [u8; 32] = reader.hash.finalize().into();
    let read_size = reader.size;
    after_decode()?;
    let after = stamp_file(&mut file, control)?;
    let current = stamp_at(&parent, &leaf, control)?;
    if before != after
        || after != current
        || read_digest != before.digest
        || read_size != before.size
    {
        return Err(AppError::Conflict(
            "The project changed while opening. Open it again.".into(),
        ));
    }
    parent.verify_path(&parent.path).map_err(|_| AppError::Conflict(
        "The project folder moved or was replaced while opening. Open the project again from its current location.".into()
    ))?;
    Ok((project, after))
}
struct HashReader<'a> {
    file: &'a mut File,
    hash: Sha256,
    size: u64,
}
impl Read for HashReader<'_> {
    fn read(&mut self, buffer: &mut [u8]) -> io::Result<usize> {
        let n = self.file.read(buffer)?;
        self.hash.update(&buffer[..n]);
        self.size += n as u64;
        Ok(n)
    }
}
pub fn save_project(
    path: &Path,
    project: &Project,
    expected: Option<&FileStamp>,
    control: &JobControl,
) -> AppResult<SaveOutcome> {
    save_project_with_budget(path, project, expected, control, &[], 0)
}
pub fn save_project_with_budget(
    path: &Path,
    project: &Project,
    expected: Option<&FileStamp>,
    control: &JobControl,
    retained: &[&Dataset],
    extra_bytes: usize,
) -> AppResult<SaveOutcome> {
    let extra = work_budget(retained, extra_bytes)?;
    let refs = retained
        .len()
        .checked_add(1)
        .and_then(|n| n.checked_mul(std::mem::size_of::<&Dataset>()))
        .ok_or(AppError::SizeLimit {
            limit: MAX_OWNED_BYTES,
        })?;
    let live = extra
        .checked_add(project.owned_bytes()?)
        .and_then(|n| n.checked_add(refs))
        .ok_or(AppError::SizeLimit {
            limit: MAX_OWNED_BYTES,
        })?;
    Budget::check_datasets(retained, live)?;
    let datasets: Vec<_> = std::iter::once(project.dataset())
        .chain(retained.iter().copied())
        .collect();
    Budget::check_datasets(&datasets, live)?;
    publish_validated(
        path,
        Some(project.dataset().revision()),
        expected,
        control,
        |writer| project_io::encode_with_budget(project, writer, control, retained, extra + refs),
        |file| {
            // Validate the complete prepared file before publication using the same
            // decoder and actual live source ownership. Tight saves may be refused:
            // validation temporarily owns both projects, within the unchanged budget.
            project_io::decode_with_budget(file, control, &datasets, live).map(|_| ())
        },
        |_, _, _| Ok(()),
    )
}
pub(crate) fn work_budget(retained: &[&Dataset], extra: usize) -> AppResult<usize> {
    let extra = extra
        .checked_add(FILE_WORK_BYTES)
        .ok_or(AppError::SizeLimit {
            limit: MAX_OWNED_BYTES,
        })?;
    Budget::check_datasets(retained, extra)?;
    Ok(extra)
}
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Stage {
    Write,
    Flush,
    Precheck,
    Publish,
    NativeResult,
    Published,
    Cleanup,
    DirectorySync,
}
pub(crate) fn publish(
    path: &Path,
    revision: Option<RevisionId>,
    expected: Option<&FileStamp>,
    control: &JobControl,
    encode: impl FnOnce(&mut dyn Write) -> AppResult<()>,
) -> AppResult<SaveOutcome> {
    publish_with(path, revision, expected, control, encode, |_, _, _| Ok(()))
}
fn precheck(
    parent: &Directory,
    leaf: &OsStr,
    expected: Option<&FileStamp>,
    control: &JobControl,
) -> AppResult<()> {
    let actual = optional_stamp(parent, leaf, control)?;
    if actual.as_ref() != expected {
        return Err(AppError::Conflict("The destination changed or already exists. Review it, then choose Save as or confirm its current version.".into()));
    }
    Ok(())
}
fn transaction(parent: &Directory) -> AppResult<(Directory, OsString)> {
    for _ in 0..32 {
        let nanos = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap_or_default()
            .as_nanos();
        let leaf = OsString::from(format!(
            ".tessera-recovery-{}-{nanos:x}-{:x}",
            std::process::id(),
            NEXT.fetch_add(1, Ordering::Relaxed)
        ));
        match parent.create_dir(&leaf) {
            Ok(dir) => return Ok((dir, leaf)),
            Err(AppError::Io(error)) if error.kind() == io::ErrorKind::AlreadyExists => continue,
            Err(error) => return Err(error),
        }
    }
    Err(AppError::Io(io::Error::new(
        io::ErrorKind::AlreadyExists,
        "Could not reserve a recovery directory.",
    )))
}
fn freeze(tx: &Directory, control: &JobControl) -> AppResult<()> {
    let mut source = tx.read(OsStr::new("prepared"))?;
    let mut frozen = tx.create(OsStr::new("frozen"))?;
    let mut buffer = [0_u8; BUFFER_BYTES];
    let mut bytes = 0;
    loop {
        control.checkpoint(bytes, None)?;
        let n = source.read(&mut buffer)?;
        if n == 0 {
            break;
        }
        bytes += n;
        if bytes > MAX_PROJECT_BYTES {
            return Err(AppError::SizeLimit {
                limit: MAX_PROJECT_BYTES,
            });
        }
        frozen.write_all(&buffer[..n])?;
    }
    frozen.sync_all()?;
    Ok(())
}
fn recovery(tx: &Directory, replace: bool, staged: &FileStamp) -> RecoveryPaths {
    // An unreadable or substituted entry must still be reported as a surviving recovery path.
    let exists = |name: &str| match tx.read(OsStr::new(name)) {
        Err(error) if absent(&error) => None,
        _ => Some(tx.path.join(name)),
    };
    let prepared = exists("prepared");
    let is_our_prepared = tx
        .read(OsStr::new("prepared"))
        .and_then(|file| native::identity(&file))
        .is_ok_and(|identity| identity == staged.identity);
    let swapped = replace && native::DISPLACED == "prepared" && !is_our_prepared;
    RecoveryPaths {
        directory: tx.path.clone(),
        frozen_revision: exists("frozen"),
        displaced: if replace && (native::DISPLACED != "prepared" || swapped) {
            exists(native::DISPLACED)
        } else {
            None
        },
        prepared: if swapped { None } else { prepared },
    }
}
fn displaced_matches(tx: &Directory, expected: Option<&FileStamp>) -> bool {
    expected.is_none_or(|expected| {
        stamp_at(tx, OsStr::new(native::DISPLACED), &JobControl::default())
            .is_ok_and(|actual| &actual == expected)
    })
}
fn reconcile_paths(
    parent: &Directory,
    tx: &Directory,
    leaf: &OsStr,
    outcome: &mut SaveOutcome,
) -> bool {
    let destination = parent.current_path().map(|path| path.join(leaf));
    let recovery = tx.current_path();
    let verified = destination.is_ok() && recovery.is_ok();
    if let Ok(path) = destination {
        outcome.destination = path;
    }
    if let (Some(paths), Ok(directory)) = (outcome.recovery.as_mut(), recovery) {
        paths.frozen_revision = paths
            .frozen_revision
            .as_ref()
            .map(|_| directory.join("frozen"));
        paths.displaced = paths
            .displaced
            .as_ref()
            .map(|_| directory.join(native::DISPLACED));
        paths.prepared = paths.prepared.as_ref().map(|_| directory.join("prepared"));
        paths.directory = directory;
    }
    if !verified {
        outcome.publication = Publication::Uncertain;
        let prior = outcome.warning.take().unwrap_or_default();
        outcome.warning = Some(if prior.contains("last-known") {
            prior
        } else {
            format!("The current destination or recovery folder could not be located and verified. Reported paths are last-known locations. All surviving recovery files were retained; a committed stamp still records completed publication. {prior}")
        });
    }
    verified
}
fn warning(message: &str) -> Option<String> {
    Some(message.into())
}
fn publish_with(
    path: &Path,
    revision: Option<RevisionId>,
    expected: Option<&FileStamp>,
    control: &JobControl,
    encode: impl FnOnce(&mut dyn Write) -> AppResult<()>,
    hook: impl FnMut(Stage, &Path, &Path) -> AppResult<()>,
) -> AppResult<SaveOutcome> {
    publish_validated(path, revision, expected, control, encode, |_| Ok(()), hook)
}
fn publish_validated(
    path: &Path,
    revision: Option<RevisionId>,
    expected: Option<&FileStamp>,
    control: &JobControl,
    encode: impl FnOnce(&mut dyn Write) -> AppResult<()>,
    validate: impl FnOnce(&mut File) -> AppResult<()>,
    mut hook: impl FnMut(Stage, &Path, &Path) -> AppResult<()>,
) -> AppResult<SaveOutcome> {
    control.checkpoint(0, None)?;
    let (parent, leaf) = split(path)?;
    // ponytail: one lock per directory covers case/Unicode aliases; narrower native name locks if contention is measured.
    let lock = parent.lock(control)?;
    if expected.is_some_and(|stamp| {
        native::identity(&lock).is_ok_and(|identity| identity == stamp.identity)
    }) {
        return Err(AppError::invalid(
            "path",
            "The publication lock cannot be used as a project destination.",
        ));
    }
    precheck(&parent, &leaf, expected, control)?;
    let (tx, tx_leaf) = transaction(&parent)?;
    let destination = parent.path.join(&leaf);
    let result = (|| {
        native::probe(&tx)?;
        let file = tx.create(OsStr::new("prepared"))?;
        hook(Stage::Write, &destination, &tx.path)?;
        let mut writer = BoundedWriter {
            inner: BufWriter::with_capacity(BUFFER_BYTES, file),
            bytes: 0,
            error: None,
            control,
            next_checkpoint: 0,
        };
        let encoded = encode(&mut writer);
        if let Some(error) = writer.error.take() {
            return Err(error);
        }
        encoded?;
        hook(Stage::Flush, &destination, &tx.path)?;
        writer.flush()?;
        writer.inner.get_ref().sync_all()?;
        drop(writer);
        validate(&mut tx.read(OsStr::new("prepared"))?)?;
        freeze(&tx, control)?;
        let staged = stamp_at(&tx, OsStr::new("prepared"), control)?;
        let frozen = stamp_at(&tx, OsStr::new("frozen"), control)?;
        if staged.identity == frozen.identity
            || staged.digest != frozen.digest
            || staged.size != frozen.size
        {
            return Err(AppError::Conflict(
                "The independent recovery copy could not be verified.".into(),
            ));
        }
        hook(Stage::Precheck, &destination, &tx.path)?;
        if let Err(error) = precheck(&parent, &leaf, expected, control) {
            if matches!(error, AppError::Cancelled) {
                return Err(error);
            }
            let mut outcome = SaveOutcome { publication:Publication::Conflict,revision,committed_stamp:None,destination:destination.clone(),recovery:Some(recovery(&tx,false,&staged)),warning:warning("The destination changed before publication. The new revision is preserved in recovery.") };
            reconcile_paths(&parent, &tx, &leaf, &mut outcome);
            return Ok(outcome);
        }
        hook(Stage::Publish, &destination, &tx.path)?;
        control.checkpoint(0, None)?;
        let published = native::publish(&parent, &tx, &leaf, expected.is_some());
        // UnsupportedPublication is returned only before a native mutation is attempted or when
        // the atomic rename reports an unsupported flag/volume with no publication.
        if matches!(published, Err(AppError::UnsupportedPublication(_))) {
            return published.map(|_| unreachable!());
        }
        let published = published.and_then(|_| hook(Stage::NativeResult, &destination, &tx.path));
        // No cancellation checkpoints after this point: a completed file publication remains a fact.
        let observed = optional_stamp(&parent, &leaf, &JobControl::default());
        let committed = published.is_ok()
            || observed
                .as_ref()
                .is_ok_and(|stamp| stamp.as_ref() == Some(&staged));
        if let Err(error) = published {
            let publication = if !committed {
                Publication::Uncertain
            } else if displaced_matches(&tx, expected) {
                Publication::Committed
            } else {
                Publication::Conflict
            };
            let mut outcome = SaveOutcome { publication,revision,committed_stamp:committed.then_some(staged.clone()),destination:destination.clone(),recovery:Some(recovery(&tx,expected.is_some(),&staged)),warning:Some(format!("The native publication call reported an error. All surviving recovery files were retained for inspection. {error}")) };
            reconcile_paths(&parent, &tx, &leaf, &mut outcome);
            return Ok(outcome);
        }
        let post_hook = hook(Stage::Published, &destination, &tx.path);
        let final_stamp = stamp_at(&parent, &leaf, &JobControl::default());
        let matched = final_stamp.as_ref().is_ok_and(|actual| actual == &staged)
            && displaced_matches(&tx, expected);
        let mut outcome = SaveOutcome {
            publication: if matched {
                Publication::Committed
            } else {
                Publication::Conflict
            },
            revision,
            committed_stamp: Some(staged.clone()),
            destination: destination.clone(),
            recovery: Some(recovery(&tx, expected.is_some(), &staged)),
            warning: None,
        };
        if !reconcile_paths(&parent, &tx, &leaf, &mut outcome) {
            return Ok(outcome);
        }
        if !matched {
            outcome.warning=warning("Publication occurred, but another file version was detected. The frozen revision and displaced version are preserved. The current destination was left in place.");
            return Ok(outcome);
        }
        if post_hook.is_err() {
            outcome.warning = warning(
                "Publication completed. Recovery files were retained because a later step failed.",
            );
            return Ok(outcome);
        }
        let sync = hook(Stage::DirectorySync, &destination, &tx.path)
            .and_then(|_| tx.sync())
            .and_then(|_| parent.sync());
        if !reconcile_paths(&parent, &tx, &leaf, &mut outcome) {
            return Ok(outcome);
        }
        if sync.is_err() {
            outcome.warning=warning("Publication completed. Directory persistence could not be verified; power-loss durability is not guaranteed.");
        }
        let cleanup = hook(Stage::Cleanup, &destination, &tx.path);
        // Keep the independent revision until every preceding operation has left verified paths.
        if !reconcile_paths(&parent, &tx, &leaf, &mut outcome) {
            return Ok(outcome);
        }
        if cleanup
            .and_then(|_| tx.remove(OsStr::new("frozen"), false))
            .is_err()
        {
            outcome.warning=warning("Publication completed, but recovery cleanup failed. Keep the recovery folder until the saved file is checked.");
            return Ok(outcome);
        }
        outcome.recovery = expected.map(|_| recovery(&tx, true, &staged));
        reconcile_paths(&parent, &tx, &leaf, &mut outcome);
        Ok(outcome)
    })();
    if result.is_err() {
        // Only pre-publication failures reach Err. Remove only our known owned temporary names.
        for name in ["prepared", "frozen", "probe-a", "probe-b", "probe-c"] {
            let _ = tx.remove(OsStr::new(name), false);
        }
    }
    let remove_dir = result.as_ref().is_err()
        || result
            .as_ref()
            .is_ok_and(|outcome| outcome.recovery.is_none());
    let transaction_path = tx.current_path();
    let removable = parent
        .current_path()
        .and_then(|path| tx.verify_path(&path.join(&tx_leaf)));
    drop(tx);
    let cleanup = if remove_dir {
        removable.and_then(|_| parent.remove(&tx_leaf, true))
    } else {
        Ok(())
    };
    match (result, cleanup) {
        (Ok(mut outcome), Err(_)) => {
            outcome.recovery = Some(RecoveryPaths {
                directory: transaction_path.unwrap_or_else(|_| parent.path.join(tx_leaf)),
                frozen_revision: None,
                displaced: None,
                prepared: None,
            });
            outcome.warning = warning(
                "Publication completed, but the empty transaction folder could not be removed.",
            );
            Ok(outcome)
        }
        (result, _) => result,
    }
}
struct BoundedWriter<'a> {
    inner: BufWriter<File>,
    bytes: usize,
    error: Option<AppError>,
    control: &'a JobControl,
    next_checkpoint: usize,
}
impl Write for BoundedWriter<'_> {
    fn write(&mut self, bytes: &[u8]) -> io::Result<usize> {
        let checkpoint = if self.bytes >= self.next_checkpoint {
            self.next_checkpoint = self.bytes.saturating_add(BUFFER_BYTES);
            self.control.checkpoint(self.bytes, None)
        } else {
            Ok(())
        };
        let check = checkpoint.and_then(|_| {
            if self
                .bytes
                .checked_add(bytes.len())
                .is_none_or(|n| n > MAX_PROJECT_BYTES)
            {
                Err(AppError::SizeLimit {
                    limit: MAX_PROJECT_BYTES,
                })
            } else {
                Ok(())
            }
        });
        if let Err(error) = check {
            self.error = Some(error);
            return Err(io::Error::other("Output interrupted or too large."));
        }
        let count = self.inner.write(bytes)?;
        self.bytes += count;
        Ok(count)
    }
    fn flush(&mut self) -> io::Result<()> {
        self.inner.flush()
    }
}
