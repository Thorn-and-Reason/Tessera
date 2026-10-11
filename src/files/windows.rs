//! Parent handles deny delete sharing, pinning the checked path while Win32 name-based APIs run.
use super::FileIdentity;
use crate::{
    error::{AppError, AppResult},
    operation::JobControl,
};
use std::{
    ffi::OsStr,
    fs::{File, OpenOptions},
    io,
    os::windows::{
        ffi::OsStrExt,
        fs::{MetadataExt, OpenOptionsExt},
        io::AsRawHandle,
    },
    path::{Component, Path, PathBuf},
    thread,
    time::Duration,
};
use windows_sys::Win32::{
    Foundation::{LocalFree, ERROR_LOCK_VIOLATION, HANDLE},
    Security::{
        Authorization::ConvertStringSecurityDescriptorToSecurityDescriptorW, SECURITY_ATTRIBUTES,
    },
    Storage::FileSystem::{
        CreateDirectoryW, FileIdInfo, GetDriveTypeW, GetFileInformationByHandleEx,
        GetVolumeInformationByHandleW, LockFileEx, MoveFileExW, ReplaceFileW,
        FILE_ATTRIBUTE_REPARSE_POINT, FILE_FLAG_BACKUP_SEMANTICS, FILE_FLAG_OPEN_REPARSE_POINT,
        FILE_ID_INFO, FILE_SHARE_READ, FILE_SHARE_WRITE, LOCKFILE_EXCLUSIVE_LOCK,
        LOCKFILE_FAIL_IMMEDIATELY, MOVEFILE_WRITE_THROUGH,
    },
    System::IO::OVERLAPPED,
};
pub(super) const DISPLACED: &str = "displaced";
pub(crate) struct Directory {
    file: File,
    _ancestors: Vec<File>,
    pub path: PathBuf,
}
fn wide(path: &Path) -> AppResult<Vec<u16>> {
    let mut value: Vec<_> = path.as_os_str().encode_wide().collect();
    if value.contains(&0) {
        return Err(AppError::invalid("path", "File names cannot contain NUL."));
    }
    value.push(0);
    Ok(value)
}
fn safe(file: &File, directory: bool) -> AppResult<()> {
    let m = file.metadata()?;
    if m.file_attributes() & FILE_ATTRIBUTE_REPARSE_POINT != 0
        || m.is_dir() != directory
        || (!directory && !m.is_file())
    {
        return Err(AppError::invalid(
            "path",
            "Choose a regular file in a folder without reparse points.",
        ));
    }
    Ok(())
}
fn handle(file: &File) -> HANDLE {
    file.as_raw_handle().cast()
}
impl Directory {
    pub(crate) fn heap_bytes(&self) -> usize {
        self.path.capacity() + self._ancestors.capacity() * std::mem::size_of::<File>()
    }
    pub fn open(path: &Path) -> AppResult<Self> {
        let path = if path.is_absolute() {
            path.to_owned()
        } else {
            std::env::current_dir()?.join(path)
        };
        let mut prefix = PathBuf::new();
        let mut ancestors = Vec::new();
        for component in path.components() {
            if matches!(component, Component::ParentDir) {
                return Err(AppError::invalid(
                    "path",
                    "Use a path without parent-directory components.",
                ));
            }
            prefix.push(component.as_os_str());
            if matches!(component, Component::Prefix(_) | Component::CurDir) {
                continue;
            }
            let file = OpenOptions::new()
                .read(true)
                .share_mode(FILE_SHARE_READ | FILE_SHARE_WRITE)
                .custom_flags(FILE_FLAG_BACKUP_SEMANTICS | FILE_FLAG_OPEN_REPARSE_POINT)
                .open(&prefix)?;
            safe(&file, true)?;
            ancestors.push(file);
        }
        let file = ancestors
            .pop()
            .ok_or_else(|| AppError::invalid("path", "Choose a destination folder."))?;
        Ok(Self {
            file,
            _ancestors: ancestors,
            path,
        })
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
        // Ancestor handles deny delete sharing. Verify the stored name rather than
        // assuming that this protection establishes a usable returned location.
        self.verify_path(&self.path)?;
        Ok(self.path.clone())
    }
    pub fn create_dir(&self, leaf: &OsStr) -> AppResult<Self> {
        let path = wide(&self.path.join(leaf))?;
        // Protected owner-rights DACL: no inherited ACE can grant access to another user.
        let sddl: Vec<u16> = "D:P(A;OICI;FA;;;OW)\0".encode_utf16().collect();
        let mut descriptor = std::ptr::null_mut();
        // SAFETY: NUL-terminated input, writable descriptor output, SDDL revision 1.
        if unsafe {
            ConvertStringSecurityDescriptorToSecurityDescriptorW(
                sddl.as_ptr(),
                1,
                &mut descriptor,
                std::ptr::null_mut(),
            )
        } == 0
        {
            return Err(io::Error::last_os_error().into());
        }
        let attributes = SECURITY_ATTRIBUTES {
            nLength: std::mem::size_of::<SECURITY_ATTRIBUTES>() as u32,
            lpSecurityDescriptor: descriptor,
            bInheritHandle: 0,
        };
        // SAFETY: valid security descriptor and path remain live throughout the synchronous call.
        let result = unsafe { CreateDirectoryW(path.as_ptr(), &attributes) };
        let error = io::Error::last_os_error();
        // SAFETY: ConvertStringSecurityDescriptorToSecurityDescriptorW allocated this descriptor with LocalAlloc.
        unsafe {
            LocalFree(descriptor);
        }
        if result == 0 {
            return Err(error.into());
        }
        Self::open(&self.path.join(leaf))
    }
    pub fn read(&self, leaf: &OsStr) -> AppResult<File> {
        let file = OpenOptions::new()
            .read(true)
            .share_mode(
                FILE_SHARE_READ
                    | FILE_SHARE_WRITE
                    | windows_sys::Win32::Storage::FileSystem::FILE_SHARE_DELETE,
            )
            .custom_flags(FILE_FLAG_OPEN_REPARSE_POINT)
            .open(self.path.join(leaf))?;
        safe(&file, false)?;
        Ok(file)
    }
    pub fn create(&self, leaf: &OsStr) -> AppResult<File> {
        let file = OpenOptions::new()
            .read(true)
            .write(true)
            .create_new(true)
            .custom_flags(FILE_FLAG_OPEN_REPARSE_POINT)
            .open(self.path.join(leaf))?;
        safe(&file, false)?;
        Ok(file)
    }
    pub fn lock(&self, control: &JobControl) -> AppResult<File> {
        let file = OpenOptions::new()
            .read(true)
            .write(true)
            .create(true)
            .truncate(false)
            .share_mode(FILE_SHARE_READ | FILE_SHARE_WRITE)
            .custom_flags(FILE_FLAG_OPEN_REPARSE_POINT)
            .open(self.path.join(".tessera-publication.lock"))?;
        safe(&file, false)?;
        loop {
            control.checkpoint(0, None)?;
            // SAFETY: initialized synchronous OVERLAPPED and valid open handle, one-byte exclusive range.
            let mut overlapped: OVERLAPPED = unsafe { std::mem::zeroed() };
            if unsafe {
                LockFileEx(
                    handle(&file),
                    LOCKFILE_EXCLUSIVE_LOCK | LOCKFILE_FAIL_IMMEDIATELY,
                    0,
                    1,
                    0,
                    &mut overlapped,
                )
            } != 0
            {
                return Ok(file);
            }
            let error = io::Error::last_os_error();
            if error.raw_os_error() != Some(ERROR_LOCK_VIOLATION as i32) {
                return Err(error.into());
            }
            thread::sleep(Duration::from_millis(10));
        }
    }
    pub fn remove(&self, leaf: &OsStr, directory: bool) -> AppResult<()> {
        if directory {
            std::fs::remove_dir(self.path.join(leaf))?;
        } else {
            std::fs::remove_file(self.path.join(leaf))?;
        }
        Ok(())
    }
    pub fn sync(&self) -> AppResult<()> {
        Err(AppError::UnsupportedPublication("Windows directory persistence has not been verified; atomic visibility does not guarantee power-loss durability.".into()))
    }
}
pub(super) fn identity(file: &File) -> AppResult<FileIdentity> {
    let mut info = FILE_ID_INFO::default();
    // SAFETY: writable correctly sized output and live handle. Preserve all 128 ID bits on ReFS.
    if unsafe {
        GetFileInformationByHandleEx(
            handle(file),
            FileIdInfo,
            (&mut info as *mut FILE_ID_INFO).cast(),
            std::mem::size_of::<FILE_ID_INFO>() as u32,
        )
    } == 0
    {
        return Err(io::Error::last_os_error().into());
    }
    Ok(FileIdentity {
        volume: info.VolumeSerialNumber,
        file: u128::from_le_bytes(info.FileId.Identifier),
    })
}
pub(super) fn probe(tx: &Directory) -> AppResult<()> {
    let root: PathBuf = tx.path.components().take(2).collect();
    let root = wide(&root)?;
    // SAFETY: NUL-terminated checked root. Values 2/3/6 are removable/fixed/RAM local drives.
    if !matches!(unsafe { GetDriveTypeW(root.as_ptr()) }, 2 | 3 | 6) {
        return Err(AppError::UnsupportedPublication(
            "Safe publication requires a local volume. Choose Save as in a local folder.".into(),
        ));
    }
    let mut filesystem = [0_u16; 32];
    // SAFETY: writable fixed output array, live directory handle; unused outputs are null.
    if unsafe {
        GetVolumeInformationByHandleW(
            handle(&tx.file),
            std::ptr::null_mut(),
            0,
            std::ptr::null_mut(),
            std::ptr::null_mut(),
            std::ptr::null_mut(),
            filesystem.as_mut_ptr(),
            filesystem.len() as u32,
        )
    } == 0
    {
        return Err(io::Error::last_os_error().into());
    }
    let end = filesystem
        .iter()
        .position(|n| *n == 0)
        .unwrap_or(filesystem.len());
    if !matches!(
        String::from_utf16_lossy(&filesystem[..end]).as_str(),
        "NTFS" | "ReFS"
    ) {
        return Err(AppError::UnsupportedPublication("Safe replacement requires a supported local filesystem. Choose Save as on NTFS or ReFS.".into()));
    }
    Ok(())
}
pub(super) fn publish(
    parent: &Directory,
    tx: &Directory,
    leaf: &OsStr,
    replace: bool,
) -> AppResult<()> {
    let dest = wide(&parent.path.join(leaf))?;
    let source = wide(&tx.path.join("prepared"))?;
    let result = if replace {
        let backup = wide(&tx.path.join(DISPLACED))?;
        // SAFETY: NUL-terminated absolute paths remain live. The backup is absent in our exclusive directory.
        // Flags are zero: REPLACEFILE_WRITE_THROUGH is unsupported. Caller reconciles partial failures.
        unsafe {
            ReplaceFileW(
                dest.as_ptr(),
                source.as_ptr(),
                backup.as_ptr(),
                0,
                std::ptr::null(),
                std::ptr::null(),
            )
        }
    } else {
        // SAFETY: both paths remain live. No REPLACE_EXISTING flag: an existing export is never overwritten.
        unsafe { MoveFileExW(source.as_ptr(), dest.as_ptr(), MOVEFILE_WRITE_THROUGH) }
    };
    if result == 0 {
        Err(io::Error::last_os_error().into())
    } else {
        Ok(())
    }
}
