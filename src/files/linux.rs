//! Directory-relative operations keep destination and recovery names on pinned directories.
use super::FileIdentity;
use crate::{
    error::{AppError, AppResult},
    operation::JobControl,
};
use std::{
    ffi::{CString, OsStr},
    fs::File,
    io,
    os::{
        fd::{AsRawFd, FromRawFd},
        unix::{ffi::OsStrExt, fs::MetadataExt},
    },
    path::{Component, Path, PathBuf},
    thread,
    time::Duration,
};

pub(super) const DISPLACED: &str = "prepared";
pub(crate) struct Directory {
    file: File,
    pub path: PathBuf,
}
fn name(value: &OsStr) -> AppResult<CString> {
    if value.is_empty() || value.as_bytes().contains(&b'/') || value == "." || value == ".." {
        return Err(AppError::invalid("path", "Choose a plain file name."));
    }
    CString::new(value.as_bytes())
        .map_err(|_| AppError::invalid("path", "File names cannot contain NUL."))
}
fn opened(fd: libc::c_int) -> AppResult<File> {
    if fd < 0 {
        return Err(io::Error::last_os_error().into());
    }
    // SAFETY: successful open/openat returned a new, uniquely owned descriptor.
    Ok(unsafe { File::from_raw_fd(fd) })
}
impl Directory {
    pub(crate) fn heap_bytes(&self) -> usize {
        self.path.capacity()
    }
    pub(crate) fn metadata(&self) -> AppResult<std::fs::Metadata> {
        Ok(self.file.metadata()?)
    }
    pub fn open(path: &Path) -> AppResult<Self> {
        let path = if path.is_absolute() {
            path.to_owned()
        } else {
            std::env::current_dir()?.join(path)
        };
        if path.components().any(|part| {
            !matches!(
                part,
                Component::RootDir | Component::CurDir | Component::Normal(_)
            )
        }) {
            return Err(AppError::invalid(
                "path",
                "Use an absolute path without parent-directory components.",
            ));
        }
        // O_PATH permits traversal-only ancestors without reading their contents.
        // Each openat pins one directory and rejects a symlink in that component.
        // SAFETY: static NUL-terminated root and directory-only flags.
        let mut directory = opened(unsafe {
            libc::open(
                c"/".as_ptr(),
                libc::O_PATH | libc::O_DIRECTORY | libc::O_CLOEXEC,
            )
        })?;
        for component in path.components() {
            if let Component::Normal(leaf) = component {
                let leaf = name(leaf)?;
                // SAFETY: one component and a live parent descriptor; symlinks are forbidden.
                directory = opened(unsafe {
                    libc::openat(
                        directory.as_raw_fd(),
                        leaf.as_ptr(),
                        libc::O_PATH | libc::O_DIRECTORY | libc::O_NOFOLLOW | libc::O_CLOEXEC,
                    )
                })?;
            }
        }
        // Only the selected parent needs a readable descriptor for fsync.
        // SAFETY: a live pinned directory and the literal current-directory component.
        let file = opened(unsafe {
            libc::openat(
                directory.as_raw_fd(),
                c".".as_ptr(),
                libc::O_RDONLY | libc::O_DIRECTORY | libc::O_NOFOLLOW | libc::O_CLOEXEC,
            )
        })?;
        Ok(Self { file, path })
    }
    pub fn verify_path(&self, path: &Path) -> AppResult<()> {
        let named = Self::open(path)?;
        if identity(&named.file)? != identity(&self.file)? {
            return Err(AppError::Conflict(
                "The folder moved or was replaced.".into(),
            ));
        }
        Ok(())
    }
    pub fn current_path(&self) -> AppResult<PathBuf> {
        // Linux exposes the descriptor's current location through procfs. Reopening
        // and checking identity rejects deleted, replaced or symlinked locations.
        let path = std::fs::read_link(format!("/proc/self/fd/{}", self.file.as_raw_fd()))?;
        self.verify_path(&path)?;
        Ok(path)
    }
    pub fn create_dir(&self, leaf: &OsStr) -> AppResult<Self> {
        let name = name(leaf)?;
        // SAFETY: pinned parent and one NUL-terminated component. Mode limits recovery access to this user.
        if unsafe { libc::mkdirat(self.file.as_raw_fd(), name.as_ptr(), 0o700) } != 0 {
            return Err(io::Error::last_os_error().into());
        }
        // SAFETY: opens the exclusively created child without following substitutions by symlinks.
        let file = opened(unsafe {
            libc::openat(
                self.file.as_raw_fd(),
                name.as_ptr(),
                libc::O_RDONLY | libc::O_DIRECTORY | libc::O_NOFOLLOW | libc::O_CLOEXEC,
            )
        })?;
        let metadata = file.metadata()?;
        // SAFETY: geteuid has no arguments or memory preconditions.
        if metadata.uid() != unsafe { libc::geteuid() } || metadata.mode() & 0o077 != 0 {
            return Err(AppError::UnsupportedPublication(
                "The recovery directory is not private. Choose Save as in a private folder.".into(),
            ));
        }
        Ok(Self {
            file,
            path: self.path.join(leaf),
        })
    }
    fn file(&self, leaf: &OsStr, flags: i32) -> AppResult<File> {
        let leaf = name(leaf)?;
        // SAFETY: descriptor and CString live through openat; O_NOFOLLOW rejects final symlinks.
        let file = opened(unsafe {
            libc::openat(
                self.file.as_raw_fd(),
                leaf.as_ptr(),
                flags | libc::O_NOFOLLOW | libc::O_CLOEXEC | libc::O_NONBLOCK,
                0o600,
            )
        })?;
        if !file.metadata()?.is_file() {
            return Err(AppError::invalid("path", "Choose a regular file."));
        }
        Ok(file)
    }
    pub fn read(&self, leaf: &OsStr) -> AppResult<File> {
        self.file(leaf, libc::O_RDONLY)
    }
    pub fn create(&self, leaf: &OsStr) -> AppResult<File> {
        self.file(leaf, libc::O_RDWR | libc::O_CREAT | libc::O_EXCL)
    }
    pub fn lock(&self, control: &JobControl) -> AppResult<File> {
        let file = self.file(
            OsStr::new(".tessera-publication.lock"),
            libc::O_RDWR | libc::O_CREAT,
        )?;
        let metadata = file.metadata()?;
        // SAFETY: geteuid is a value-only query.
        if metadata.nlink() != 1
            || metadata.uid() != unsafe { libc::geteuid() }
            || metadata.mode() & 0o022 != 0
        {
            return Err(AppError::UnsupportedPublication(
                "The publication lock is unsafe. Choose Save as in a private folder.".into(),
            ));
        }
        loop {
            control.checkpoint(0, None)?;
            // SAFETY: valid owned descriptor; lock is released by close/process exit.
            if unsafe { libc::flock(file.as_raw_fd(), libc::LOCK_EX | libc::LOCK_NB) } == 0 {
                return Ok(file);
            }
            let error = io::Error::last_os_error();
            if error.kind() != io::ErrorKind::WouldBlock {
                return Err(error.into());
            }
            thread::sleep(Duration::from_millis(10));
        }
    }
    pub fn remove(&self, leaf: &OsStr, directory: bool) -> AppResult<()> {
        let leaf = name(leaf)?;
        // SAFETY: one component relative to a pinned directory; never recursively follows paths.
        if unsafe {
            libc::unlinkat(
                self.file.as_raw_fd(),
                leaf.as_ptr(),
                if directory { libc::AT_REMOVEDIR } else { 0 },
            )
        } != 0
        {
            return Err(io::Error::last_os_error().into());
        }
        Ok(())
    }
    pub fn sync(&self) -> AppResult<()> {
        self.file.sync_all().map_err(Into::into)
    }
}
pub(super) fn identity(file: &File) -> AppResult<FileIdentity> {
    let metadata = file.metadata()?;
    Ok(FileIdentity {
        volume: metadata.dev(),
        file: u128::from(metadata.ino()),
    })
}
fn rename(
    from: &Directory,
    source: &OsStr,
    to: &Directory,
    dest: &OsStr,
    flags: u32,
) -> AppResult<()> {
    let source = name(source)?;
    let dest = name(dest)?;
    // SAFETY: both descriptors remain open, and both strings are single NUL-terminated components.
    if unsafe {
        libc::renameat2(
            from.file.as_raw_fd(),
            source.as_ptr(),
            to.file.as_raw_fd(),
            dest.as_ptr(),
            flags,
        )
    } == 0
    {
        return Ok(());
    }
    Err(publication_error(io::Error::last_os_error()))
}
fn publication_error(error: io::Error) -> AppError {
    if matches!(
        error.raw_os_error(),
        Some(libc::ENOTSUP | libc::EINVAL | libc::EXDEV | libc::ENOSYS)
    ) {
        AppError::UnsupportedPublication("This filesystem cannot safely replace projects. Choose Save as on a supported local volume.".into())
    } else {
        error.into()
    }
}
pub(super) fn probe(tx: &Directory) -> AppResult<()> {
    tx.create(OsStr::new("probe-a"))?;
    tx.create(OsStr::new("probe-b"))?;
    rename(
        tx,
        OsStr::new("probe-a"),
        tx,
        OsStr::new("probe-c"),
        libc::RENAME_NOREPLACE,
    )?;
    rename(
        tx,
        OsStr::new("probe-c"),
        tx,
        OsStr::new("probe-b"),
        libc::RENAME_EXCHANGE,
    )?;
    tx.remove(OsStr::new("probe-c"), false)?;
    tx.remove(OsStr::new("probe-b"), false)
}
pub(super) fn publish(
    parent: &Directory,
    tx: &Directory,
    leaf: &OsStr,
    replace: bool,
) -> AppResult<()> {
    rename(
        tx,
        OsStr::new("prepared"),
        parent,
        leaf,
        if replace {
            libc::RENAME_EXCHANGE
        } else {
            libc::RENAME_NOREPLACE
        },
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn unsupported_atomic_primitives_fail_closed() {
        for code in [libc::ENOTSUP, libc::EINVAL, libc::EXDEV, libc::ENOSYS] {
            assert!(matches!(
                publication_error(io::Error::from_raw_os_error(code)),
                AppError::UnsupportedPublication(_)
            ));
        }
        assert!(matches!(
            publication_error(io::Error::from_raw_os_error(libc::EEXIST)),
            AppError::Io(_)
        ));
    }

    #[test]
    fn publication_names_cannot_escape_pinned_directory() {
        for leaf in ["", ".", "..", "a/b", "nul\0"] {
            assert!(name(OsStr::new(leaf)).is_err(), "{leaf:?}");
        }
    }
}
