use crate::metadata::{MetadataDecoder, WorkbookInfo, MAX_METADATA};
use crate::protocol::{validate_range, Cell, Decoder, MAX_OUTPUT};
use sha2::{Digest, Sha256};
use std::{
    fs::{self, File, OpenOptions},
    io::{Read, Write},
    path::{Path, PathBuf},
    process::{Child, Command, Stdio},
    sync::{
        atomic::{AtomicBool, AtomicU64, Ordering},
        mpsc, Arc,
    },
    thread,
    time::{Duration, Instant, SystemTime, UNIX_EPOCH},
};
const MAX_INPUT: usize = 128 * 1024 * 1024;
const MAX_OWNED: usize = 256 * 1024 * 1024;
static SNAPSHOT_NONCE: AtomicU64 = AtomicU64::new(0);
#[cfg(windows)]
#[path = "private_directory_windows.rs"]
mod private_directory_windows;
pub struct Config {
    pub worker: PathBuf,
    pub source: PathBuf,
    pub range: [u32; 4],
    pub worksheet_index: u32,
    pub quota: usize,
    pub retained_bytes: usize,
    pub timeout: Duration,
    pub cancel: Arc<AtomicBool>,
    /// Test adversaries only; never expose this field in product UI.
    pub probe: Option<String>,
}
#[derive(Debug)]
struct Snapshot {
    path: PathBuf,
    identity: [u8; 32],
    source_bytes: usize,
}
#[derive(Debug, Clone)]
pub struct WorkbookSnapshot(Arc<Snapshot>);
impl WorkbookSnapshot {
    pub fn source_identity(&self) -> [u8; 32] {
        self.0.identity
    }
    pub fn owned_bytes(&self) -> usize {
        self.0.source_bytes
            + self.0.path.capacity()
            + std::mem::size_of::<Snapshot>()
            + 2 * std::mem::size_of::<usize>()
    }
    pub fn prepare(
        source: &Path,
        retained_bytes: usize,
        cancel: &AtomicBool,
    ) -> Result<Self, String> {
        let metadata = fs::metadata(source).map_err(|e| e.to_string())?;
        if !metadata.is_file() || metadata.len() > MAX_INPUT as u64 {
            return Err("invalid or oversized source".into());
        }
        let length = metadata.len() as usize;
        if retained_bytes
            .checked_add(length * 2 + 128 * 1024)
            .is_none_or(|n| n > MAX_OWNED)
        {
            return Err("aggregate ownership budget exceeded".into());
        }
        snapshot(source, length, cancel).map(|owner| Self(Arc::new(owner)))
    }
}
impl Drop for Snapshot {
    fn drop(&mut self) {
        #[cfg(windows)]
        if let Ok(metadata) = fs::metadata(self.path.join("source.xlsx")) {
            let mut permissions = metadata.permissions();
            // Windows READONLY clearing permits cleanup without changing the protected DACL.
            #[allow(clippy::permissions_set_readonly_false)]
            permissions.set_readonly(false);
            let _ = fs::set_permissions(self.path.join("source.xlsx"), permissions);
        }
        let _ = fs::remove_file(self.path.join("source.xlsx"));
        let _ = fs::remove_dir(&self.path);
    }
}
struct OwnedChild(Child);
impl Drop for OwnedChild {
    fn drop(&mut self) {
        let _ = self.0.kill();
        let _ = self.0.wait();
    }
}
#[cfg(windows)]
fn create_private_directory(path: &Path) -> std::io::Result<()> {
    private_directory_windows::create(path)
}
#[cfg(not(windows))]
fn create_private_directory(path: &Path) -> std::io::Result<()> {
    let builder = fs::DirBuilder::new();
    #[cfg(unix)]
    let builder = {
        use std::os::unix::fs::DirBuilderExt;
        let mut builder = builder;
        builder.mode(0o700);
        builder
    };
    builder.create(path)
}
fn reserve_snapshot_directory(
    stamp: u128,
    mut create: impl FnMut(&Path) -> std::io::Result<()>,
) -> Result<PathBuf, String> {
    for _ in 0..8 {
        let nonce = SNAPSHOT_NONCE
            .fetch_update(Ordering::Relaxed, Ordering::Relaxed, |n| n.checked_add(1))
            .map_err(|_| "snapshot identity exhausted")?;
        let path = std::env::temp_dir().join(format!(
            "excel-proof-{}-{stamp}-{nonce}",
            std::process::id()
        ));
        match create(&path) {
            Ok(()) => return Ok(path),
            Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => continue,
            Err(error) => return Err(error.to_string()),
        }
    }
    Err("Could not reserve a unique workbook snapshot directory.".into())
}
fn snapshot(source: &Path, length: usize, cancel: &AtomicBool) -> Result<Snapshot, String> {
    let mut input = File::open(source).map_err(|e| e.to_string())?;
    let mut bytes = Vec::with_capacity(length);
    let mut buffer = [0u8; 32768];
    loop {
        if cancel.load(Ordering::SeqCst) {
            return Err("cancelled".into());
        }
        let n = input.read(&mut buffer).map_err(|e| e.to_string())?;
        if n == 0 {
            break;
        }
        if bytes.len() + n > length {
            return Err("source changed during snapshot".into());
        }
        bytes.extend_from_slice(&buffer[..n]);
    }
    if bytes.len() != length {
        return Err("source changed during snapshot".into());
    }
    let stamp = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_err(|e| e.to_string())?
        .as_nanos();
    let path = reserve_snapshot_directory(stamp, create_private_directory)?;
    let identity = Sha256::digest(&bytes).into();
    let owned = Snapshot {
        path,
        identity,
        source_bytes: length,
    };
    let mut file = OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(owned.path.join("source.xlsx"))
        .map_err(|e| e.to_string())?;
    file.write_all(&bytes).map_err(|e| e.to_string())?;
    let mut permissions = file.metadata().map_err(|e| e.to_string())?.permissions();
    permissions.set_readonly(true);
    file.set_permissions(permissions)
        .map_err(|e| e.to_string())?;
    Ok(owned)
}
pub fn run(config: Config) -> Result<Vec<Cell>, String> {
    validate_range(config.range).map_err(str::to_owned)?;
    let retained = config
        .retained_bytes
        .checked_add(config.quota)
        .and_then(|n| n.checked_add(MAX_OUTPUT * 2 + 128 * 1024))
        .ok_or("aggregate ownership budget exceeded")?;
    let snapshot = WorkbookSnapshot::prepare(&config.source, retained, &config.cancel)?;
    run_snapshot(&snapshot, &config)
}
pub fn run_snapshot(snapshot: &WorkbookSnapshot, config: &Config) -> Result<Vec<Cell>, String> {
    validate_range(config.range).map_err(str::to_owned)?;
    execute(
        snapshot,
        config,
        || {
            Decoder::for_source(
                config.range,
                snapshot.source_identity(),
                config.worksheet_index,
            )
        },
        "--protocol",
        MAX_OUTPUT,
    )
}
pub fn inspect(snapshot: &WorkbookSnapshot, config: &Config) -> Result<WorkbookInfo, String> {
    execute(
        snapshot,
        config,
        || Ok(MetadataDecoder::new(snapshot.source_identity())),
        "--inspect",
        MAX_METADATA,
    )
}
trait Response {
    type Output;
    fn feed(&mut self, bytes: &[u8]) -> Result<(), &'static str>;
    fn finish(self) -> Result<Self::Output, &'static str>;
}
impl Response for Decoder {
    type Output = Vec<Cell>;
    fn feed(&mut self, bytes: &[u8]) -> Result<(), &'static str> {
        self.feed(bytes)
    }
    fn finish(self) -> Result<Self::Output, &'static str> {
        self.finish()
    }
}
impl Response for MetadataDecoder {
    type Output = WorkbookInfo;
    fn feed(&mut self, bytes: &[u8]) -> Result<(), &'static str> {
        self.feed(bytes)
    }
    fn finish(self) -> Result<Self::Output, &'static str> {
        self.finish()
    }
}
fn execute<D: Response>(
    snapshot: &WorkbookSnapshot,
    config: &Config,
    create_decoder: impl FnOnce() -> Result<D, &'static str>,
    operation: &str,
    output_budget: usize,
) -> Result<D::Output, String> {
    if config.cancel.load(Ordering::SeqCst) {
        return Err("cancelled".into());
    }
    let admitted = config
        .retained_bytes
        .checked_add(snapshot.owned_bytes())
        .and_then(|n| n.checked_add(config.quota))
        .and_then(|n| n.checked_add(output_budget * 2 + 128 * 1024));
    if admitted.is_none_or(|n| n > MAX_OWNED) {
        return Err("aggregate ownership budget exceeded".into());
    }
    if config.quota < 65536 || config.timeout.is_zero() || config.timeout > Duration::from_secs(60)
    {
        return Err("invalid worker limits".into());
    }
    let mut decoder = create_decoder().map_err(str::to_owned)?;
    let mut command = Command::new(&config.worker);
    if let Some(probe) = &config.probe {
        command.arg("--probe").arg(probe);
    } else {
        command
            .arg(operation)
            .arg(snapshot.0.path.join("source.xlsx"))
            .arg(config.quota.to_string());
        if operation == "--protocol" {
            command.arg(config.worksheet_index.to_string());
            for coordinate in config.range {
                command.arg(coordinate.to_string());
            }
        }
    }
    let mut child = OwnedChild(
        command
            .stdin(Stdio::null())
            .stdout(Stdio::piped())
            .stderr(Stdio::null())
            .spawn()
            .map_err(|e| e.to_string())?,
    );
    let mut stdout = child.0.stdout.take().ok_or("worker stdout missing")?;
    let (sender, receiver) = mpsc::sync_channel::<Result<Vec<u8>, String>>(4);
    let reader = thread::spawn(move || {
        let mut buffer = [0u8; 4096];
        loop {
            match stdout.read(&mut buffer) {
                Ok(0) => break,
                Ok(n) => {
                    if sender.send(Ok(buffer[..n].to_vec())).is_err() {
                        break;
                    }
                }
                Err(e) => {
                    let _ = sender.send(Err(e.to_string()));
                    break;
                }
            }
        }
    });
    let deadline = Instant::now() + config.timeout;
    let result = (|| {
        loop {
            if config.cancel.load(Ordering::SeqCst) {
                return Err("cancelled".into());
            }
            if Instant::now() >= deadline {
                return Err("worker timed out".into());
            }
            match receiver.recv_timeout(Duration::from_millis(10)) {
                Ok(bytes) => decoder.feed(&bytes?).map_err(str::to_owned)?,
                Err(mpsc::RecvTimeoutError::Timeout) => continue,
                Err(mpsc::RecvTimeoutError::Disconnected) => break,
            }
        }
        loop {
            if config.cancel.load(Ordering::SeqCst) {
                return Err("cancelled".into());
            }
            if Instant::now() >= deadline {
                return Err("worker timed out".into());
            }
            match child.0.try_wait().map_err(|e| e.to_string())? {
                Some(status) if status.success() => return decoder.finish().map_err(str::to_owned),
                Some(_) => return Err("worker failed; no partial output accepted".into()),
                None => thread::sleep(Duration::from_millis(5)),
            }
        }
    })();
    // Drop the queue receiver before joining: a flood writer may be blocked on send.
    drop(receiver);
    drop(child);
    reader
        .join()
        .map_err(|_| "output reader panicked".to_owned())?;
    result
}

#[cfg(test)]
mod snapshot_tests {
    use super::*;
    use std::collections::HashSet;
    #[test]
    fn fixed_clock_concurrent_snapshots_never_alias() {
        let threads: Vec<_> = (0..32)
            .map(|_| {
                thread::spawn(|| {
                    let path = reserve_snapshot_directory(123, create_private_directory).unwrap();
                    Snapshot {
                        path,
                        identity: [7; 32],
                        source_bytes: 0,
                    }
                })
            })
            .collect();
        let owners: Vec<_> = threads.into_iter().map(|t| t.join().unwrap()).collect();
        let paths: HashSet<_> = owners.iter().map(|s| s.path.clone()).collect();
        assert_eq!(paths.len(), owners.len());
        for owner in &owners {
            assert_eq!(owner.identity, [7; 32]);
            #[cfg(unix)]
            {
                use std::os::unix::fs::PermissionsExt;
                assert_eq!(
                    fs::metadata(&owner.path).unwrap().permissions().mode() & 0o777,
                    0o700
                );
            }
        }
        drop(owners);
        assert!(paths.iter().all(|path| !path.exists()));
    }
    #[test]
    fn exclusive_creation_retries_preserve_existing_directory() {
        let mut existing = None;
        let mut attempts = 0;
        let path = reserve_snapshot_directory(456, |path| {
            attempts += 1;
            if attempts == 1 {
                create_private_directory(path)?;
                fs::write(path.join("keep"), b"original")?;
                existing = Some(path.to_owned());
                Err(std::io::Error::from(std::io::ErrorKind::AlreadyExists))
            } else {
                create_private_directory(path)
            }
        })
        .unwrap();
        assert_eq!(attempts, 2);
        let existing = existing.unwrap();
        assert_ne!(path, existing);
        assert_eq!(fs::read(existing.join("keep")).unwrap(), b"original");
        fs::remove_dir(path).unwrap();
        fs::remove_file(existing.join("keep")).unwrap();
        fs::remove_dir(existing).unwrap();
    }
    #[test]
    fn repeated_collisions_are_bounded() {
        let mut attempts = 0;
        assert!(reserve_snapshot_directory(789, |_| {
            attempts += 1;
            Err(std::io::Error::from(std::io::ErrorKind::AlreadyExists))
        })
        .is_err());
        assert_eq!(attempts, 8);
    }
}
