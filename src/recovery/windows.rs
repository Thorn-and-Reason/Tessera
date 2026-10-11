use std::{ffi::c_void, io, mem::offset_of, os::windows::ffi::OsStrExt, path::Path, ptr};
use windows_sys::Win32::{
    Foundation::{LocalFree, ERROR_SUCCESS},
    Security::{
        Authorization::{
            ConvertStringSecurityDescriptorToSecurityDescriptorW, GetNamedSecurityInfoW,
            SE_FILE_OBJECT,
        },
        EqualSid, GetAce, GetSecurityDescriptorControl, GetSecurityDescriptorDacl, IsValidAcl,
        IsValidSecurityDescriptor, IsValidSid, ACCESS_ALLOWED_ACE, ACE_HEADER, ACL,
        DACL_SECURITY_INFORMATION, PSECURITY_DESCRIPTOR, SE_DACL_PROTECTED,
    },
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
        "The filesystem did not retain the protected owner-only recovery directory permissions.",
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
pub(super) fn verify(path: &Path) -> io::Result<()> {
    let expected = owner_descriptor()?;
    verify_directory(&wide(path)?, expected.0)
}
