use std::{
    ffi::c_void,
    io,
    mem::{offset_of, size_of},
    os::windows::ffi::OsStrExt,
    path::Path,
    ptr,
};
use windows_sys::Win32::{
    Foundation::{LocalFree, ERROR_SUCCESS},
    Security::{
        Authorization::{
            ConvertStringSecurityDescriptorToSecurityDescriptorW, GetNamedSecurityInfoW,
            SE_FILE_OBJECT,
        },
        EqualSid, GetAce, GetSecurityDescriptorControl, GetSecurityDescriptorDacl, IsValidAcl,
        IsValidSecurityDescriptor, IsValidSid, ACCESS_ALLOWED_ACE, ACE_HEADER, ACL,
        DACL_SECURITY_INFORMATION, PSECURITY_DESCRIPTOR, SECURITY_ATTRIBUTES, SE_DACL_PROTECTED,
    },
    Storage::FileSystem::CreateDirectoryW,
};
struct LocalAllocation(PSECURITY_DESCRIPTOR);
impl Drop for LocalAllocation {
    fn drop(&mut self) {
        // SAFETY: both security APIs return LocalAlloc-owned descriptors.
        unsafe {
            LocalFree(self.0);
        }
    }
}
fn privacy_error() -> io::Error {
    io::Error::new(
        io::ErrorKind::PermissionDenied,
        "The filesystem did not retain the protected owner-only workbook directory permissions.",
    )
}
fn wide(path: &Path) -> io::Result<Vec<u16>> {
    let mut path: Vec<u16> = path.as_os_str().encode_wide().collect();
    if path.contains(&0) {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            "Directory path contains a NUL character.",
        ));
    }
    path.push(0);
    Ok(path)
}
fn descriptor_from_sddl(sddl: &str) -> io::Result<LocalAllocation> {
    let sddl: Vec<u16> = sddl.encode_utf16().chain(std::iter::once(0)).collect();
    let mut descriptor = ptr::null_mut();
    // SAFETY: terminated input and writable output remain live for this synchronous call.
    if unsafe {
        ConvertStringSecurityDescriptorToSecurityDescriptorW(
            sddl.as_ptr(),
            1,
            &mut descriptor,
            ptr::null_mut(),
        )
    } == 0
    {
        return Err(io::Error::last_os_error());
    }
    Ok(LocalAllocation(descriptor))
}
fn owner_descriptor() -> io::Result<LocalAllocation> {
    descriptor_from_sddl("D:P(A;OICI;FA;;;OW)")
}
fn single_ace(descriptor: PSECURITY_DESCRIPTOR) -> io::Result<*const ACCESS_ALLOWED_ACE> {
    if descriptor.is_null() {
        return Err(privacy_error());
    }
    let mut control = 0;
    let mut revision = 0;
    let mut present = 0;
    let mut defaulted = 0;
    let mut dacl: *mut ACL = ptr::null_mut();
    let mut ace: *mut c_void = ptr::null_mut();
    // SAFETY: descriptor comes from a native security API and all outputs are valid.
    unsafe {
        if IsValidSecurityDescriptor(descriptor) == 0
            || GetSecurityDescriptorControl(descriptor, &mut control, &mut revision) == 0
            || control & SE_DACL_PROTECTED == 0
            || GetSecurityDescriptorDacl(descriptor, &mut present, &mut dacl, &mut defaulted) == 0
            || present == 0
            || defaulted != 0
            || dacl.is_null()
            || IsValidAcl(dacl) == 0
            || (*dacl).AceCount != 1
            || GetAce(dacl, 0, &mut ace) == 0
            || ace.is_null()
        {
            return Err(privacy_error());
        }
        let header = ace.cast::<ACE_HEADER>();
        if usize::from((*header).AceSize) < offset_of!(ACCESS_ALLOWED_ACE, SidStart) + 8 {
            return Err(privacy_error());
        }
        Ok(ace.cast::<ACCESS_ALLOWED_ACE>())
    }
}
unsafe fn valid_ace_sid(ace: *const ACCESS_ALLOWED_ACE) -> io::Result<*mut c_void> {
    // SAFETY: single_ace validated native ACL storage and space for the SID header.
    let sid = ptr::addr_of!((*ace).SidStart).cast::<u8>();
    let bytes = 8 + usize::from(*sid.add(1)) * 4;
    if bytes > usize::from((*ace).Header.AceSize) - offset_of!(ACCESS_ALLOWED_ACE, SidStart)
        || IsValidSid(sid.cast_mut().cast()) == 0
    {
        return Err(privacy_error());
    }
    Ok(sid.cast_mut().cast())
}
fn verify_descriptor(
    actual: PSECURITY_DESCRIPTOR,
    expected: PSECURITY_DESCRIPTOR,
) -> io::Result<()> {
    let actual = single_ace(actual)?;
    let expected = single_ace(expected)?;
    // SAFETY: native ACL checks establish storage; the expected ACE is the fixed allow/owner-rights SDDL.
    unsafe {
        if (*actual).Header.AceType != (*expected).Header.AceType
            || (*actual).Header.AceFlags != (*expected).Header.AceFlags
            || (*actual).Mask != (*expected).Mask
            || EqualSid(valid_ace_sid(actual)?, valid_ace_sid(expected)?) == 0
        {
            return Err(privacy_error());
        }
    }
    Ok(())
}
fn verify_directory(path: &[u16], expected: PSECURITY_DESCRIPTOR) -> io::Result<()> {
    let mut descriptor = ptr::null_mut();
    // SAFETY: terminated path and descriptor output remain valid; other optional outputs are unused.
    let error = unsafe {
        GetNamedSecurityInfoW(
            path.as_ptr(),
            SE_FILE_OBJECT,
            DACL_SECURITY_INFORMATION,
            ptr::null_mut(),
            ptr::null_mut(),
            ptr::null_mut(),
            ptr::null_mut(),
            &mut descriptor,
        )
    };
    if error != ERROR_SUCCESS {
        return Err(io::Error::from_raw_os_error(error as i32));
    }
    let descriptor = LocalAllocation(descriptor);
    verify_descriptor(descriptor.0, expected)
}
pub(super) fn create(path: &Path) -> io::Result<()> {
    let encoded = wide(path)?;
    let descriptor = owner_descriptor()?;
    let attributes = SECURITY_ATTRIBUTES {
        nLength: size_of::<SECURITY_ATTRIBUTES>() as u32,
        lpSecurityDescriptor: descriptor.0,
        bInheritHandle: 0,
    };
    // SAFETY: terminated path, security attributes and descriptor are live for the call.
    if unsafe { CreateDirectoryW(encoded.as_ptr(), &attributes) } == 0 {
        return Err(io::Error::last_os_error());
    }
    if let Err(error) = verify_directory(&encoded, descriptor.0) {
        // No workbook has been written; remove only the newly-created empty directory.
        let _ = std::fs::remove_dir(path);
        return Err(error);
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::{
        fs,
        path::PathBuf,
        time::{SystemTime, UNIX_EPOCH},
    };
    struct TestDirectory(PathBuf);
    impl Drop for TestDirectory {
        fn drop(&mut self) {
            let _ = fs::remove_file(self.0.join("keep"));
            let _ = fs::remove_dir(&self.0);
        }
    }
    fn target() -> TestDirectory {
        TestDirectory(std::env::temp_dir().join(format!(
                "tessera-acl-test-{}-{}",
                std::process::id(),
                SystemTime::now()
                    .duration_since(UNIX_EPOCH)
                    .unwrap()
                    .as_nanos()
            )))
    }
    #[test]
    fn created_directory_native_dacl_is_protected_owner_only_and_inheritable() {
        let path = target();
        create(&path.0).unwrap();
        let descriptor = owner_descriptor().unwrap();
        verify_directory(&wide(&path.0).unwrap(), descriptor.0).unwrap();
    }
    #[test]
    fn existing_directory_and_contents_are_preserved() {
        let path = target();
        create(&path.0).unwrap();
        fs::write(path.0.join("keep"), b"original").unwrap();
        assert_eq!(
            create(&path.0).unwrap_err().kind(),
            std::io::ErrorKind::AlreadyExists
        );
        assert_eq!(fs::read(path.0.join("keep")).unwrap(), b"original");
    }
    #[test]
    fn invalid_path_fails_before_creation() {
        use std::os::windows::ffi::OsStringExt;
        let path = PathBuf::from(std::ffi::OsString::from_wide(&[
            b'x' as u16,
            0,
            b'y' as u16,
        ]));
        assert_eq!(
            create(&path).unwrap_err().kind(),
            std::io::ErrorKind::InvalidInput
        );
    }
    #[test]
    fn native_descriptor_check_rejects_unprotected_or_public_dacls() {
        let expected = owner_descriptor().unwrap();
        for sddl in [
            "D:(A;OICI;FA;;;OW)",
            "D:P(A;OICI;FA;;;WD)",
            "D:P(A;OICI;FR;;;OW)",
            "D:P(A;;FA;;;OW)",
            "D:P(A;OICI;FA;;;OW)(A;OICI;FR;;;WD)",
        ] {
            let descriptor = descriptor_from_sddl(sddl).unwrap();
            assert!(
                verify_descriptor(descriptor.0, expected.0).is_err(),
                "{sddl}"
            );
        }
    }
}
