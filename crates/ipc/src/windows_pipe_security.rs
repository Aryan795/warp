//! Restricts the DACL on the Windows named pipes created by [`crate::Server`] so that only the
//! creating user's own processes (elevated or not), `LocalSystem`, and
//! `BUILTIN\Administrators` can connect to them.
//!
//! ## Background
//! When a Windows named pipe is created without an explicit security descriptor,
//! [`CreateNamedPipeW`] applies a default DACL that grants full control to `LocalSystem`,
//! `Administrators`, and the *creator owner*, and read-only access to `Everyone`.
//!
//! The "creator owner" is derived from the *token* of the process that created the pipe, not
//! from the human user running it. For a UAC-elevated process, a token's default owner is often
//! the `BUILTIN\Administrators` group rather than the signed-in user's own SID. If Warp's
//! single-instance named pipe is created by an elevated process (for example, the installer's
//! post-install launch) and a later, non-elevated instance of the *same* user tries to connect
//! to forward a `warp://` deep link, the non-elevated token has `BUILTIN\Administrators` filtered
//! out (standard UAC token filtering) and so is left with only the read-only `Everyone` ACE.
//! Connecting for full-duplex I/O then fails with `ERROR_ACCESS_DENIED` (OS error 5). See
//! REV-1546.
//!
//! ## Threat model / chosen ACL
//! We replace the default DACL with one scoped to exactly what's needed to fix the bug, rather
//! than opening the pipe to all local users (a null DACL or an `Everyone`-writable SDDL would let
//! any local user on a shared/multi-user machine inject deep-link URIs -- including auth redirect
//! URIs -- into another user's running Warp instance):
//! - The *signed-in user's own SID*, read from the current process's primary token
//!   (`TokenUser`). This SID is identical for both an elevated and a non-elevated token
//!   belonging to the same login session, so granting access to it (rather than to whatever
//!   "owner" a particular token happens to have) is what actually fixes the elevation mismatch,
//!   while still keeping other local users out.
//! - `LocalSystem` and `BUILTIN\Administrators`, matching the access Windows' own default named
//!   pipe DACL already grants those principals, for consistency with standard OS behavior (e.g.
//!   admin tooling).
//!
//! The signed-in user is granted exactly the two access rights a duplex named-pipe client needs
//! (`GENERIC_READ | GENERIC_WRITE`); `SYSTEM`/`Administrators` are granted `GENERIC_ALL` to match
//! the OS default. `Everyone`/anonymous access is dropped entirely.
//!
//! [`CreateNamedPipeW`]: https://learn.microsoft.com/windows/win32/api/namedpipeapi/nf-namedpipeapi-createnamedpipew
use std::ffi::c_void;

use windows::Win32::Foundation::{HANDLE, HLOCAL, LocalFree};
use windows::Win32::Security::Authorization::{
    EXPLICIT_ACCESS_W, GRANT_ACCESS, NO_MULTIPLE_TRUSTEE, SE_KERNEL_OBJECT, SetEntriesInAclW,
    SetNamedSecurityInfoW, TRUSTEE_IS_SID, TRUSTEE_IS_UNKNOWN, TRUSTEE_W,
};
use windows::Win32::Security::{
    ACL, CopySid, CreateWellKnownSid, DACL_SECURITY_INFORMATION, GetLengthSid, GetTokenInformation,
    NO_INHERITANCE, PROTECTED_DACL_SECURITY_INFORMATION, PSID, TOKEN_QUERY, TOKEN_USER, TokenUser,
    WELL_KNOWN_SID_TYPE, WinBuiltinAdministratorsSid, WinLocalSystemSid,
};
use windows::Win32::System::Threading::{GetCurrentProcess, OpenProcessToken};
use windows::core::PCWSTR;

/// `GENERIC_READ | GENERIC_WRITE`: exactly the access rights a duplex named-pipe client needs.
const PIPE_CLIENT_ACCESS_MASK: u32 = 0x8000_0000 | 0x4000_0000;

/// `GENERIC_ALL`: matches the access Windows' own default named pipe DACL grants to
/// `LocalSystem`/`Administrators`.
const FULL_CONTROL_ACCESS_MASK: u32 = 0x1000_0000;

/// Restricts the DACL of the named pipe at `pipe_path` (of the form `\\.\pipe\<name>`, see
/// [`super::native::server::windows_named_pipe_path`]) so that only the current user (across
/// elevation levels), `LocalSystem`, and `BUILTIN\Administrators` can connect to it. See the
/// module docs for the full threat model.
///
/// This must be called after the pipe's first instance has been created (i.e. after the
/// listener is bound), since the DACL governs the pipe object shared by all instances of the
/// name, not just the specific instance handle used to set it.
pub(crate) fn restrict_named_pipe_to_current_user(pipe_path: &str) -> windows::core::Result<()> {
    let user_sid = current_user_sid()?;
    let system_sid = well_known_sid(WinLocalSystemSid)?;
    let admins_sid = well_known_sid(WinBuiltinAdministratorsSid)?;

    let entries = [
        explicit_access_entry(&user_sid, PIPE_CLIENT_ACCESS_MASK),
        explicit_access_entry(&system_sid, FULL_CONTROL_ACCESS_MASK),
        explicit_access_entry(&admins_sid, FULL_CONTROL_ACCESS_MASK),
    ];

    unsafe {
        let mut new_dacl: *mut ACL = std::ptr::null_mut();
        SetEntriesInAclW(Some(&entries), None, &mut new_dacl).ok()?;

        let result = set_pipe_dacl(pipe_path, new_dacl);

        if !new_dacl.is_null() {
            let _ = LocalFree(Some(HLOCAL(new_dacl as *mut c_void)));
        }
        result
    }
}

/// # Safety
/// `dacl` must be a valid, non-null pointer to an `ACL` for the duration of this call.
unsafe fn set_pipe_dacl(pipe_path: &str, dacl: *mut ACL) -> windows::core::Result<()> {
    let mut wide_path: Vec<u16> = pipe_path.encode_utf16().chain(std::iter::once(0)).collect();
    unsafe {
        SetNamedSecurityInfoW(
            PCWSTR(wide_path.as_mut_ptr()),
            SE_KERNEL_OBJECT,
            DACL_SECURITY_INFORMATION | PROTECTED_DACL_SECURITY_INFORMATION,
            None,
            None,
            Some(dacl as *const _),
            None,
        )
        .ok()
    }
}

/// Builds an `EXPLICIT_ACCESS_W` entry granting `access_mask` to `sid`, without inheritance
/// (named pipes have no child objects to inherit to).
///
/// # Safety
/// The returned value borrows `sid`; it must not outlive `sid`.
fn explicit_access_entry(sid: &[u8], access_mask: u32) -> EXPLICIT_ACCESS_W {
    EXPLICIT_ACCESS_W {
        grfAccessPermissions: access_mask,
        grfAccessMode: GRANT_ACCESS,
        grfInheritance: NO_INHERITANCE,
        Trustee: TRUSTEE_W {
            pMultipleTrustee: std::ptr::null_mut(),
            MultipleTrusteeOperation: NO_MULTIPLE_TRUSTEE,
            TrusteeForm: TRUSTEE_IS_SID,
            TrusteeType: TRUSTEE_IS_UNKNOWN,
            // Safety: callers (`restrict_named_pipe_to_current_user`) keep `sid`'s backing buffer
            // alive for as long as this entry is used, which is synchronously within the same
            // function via `SetEntriesInAclW`.
            ptstrName: windows::core::PWSTR(sid.as_ptr() as *mut u16),
        },
    }
}

/// Returns the SID bytes for the user identified by the calling process's primary token
/// (`TokenUser`). This is stable across elevation: an elevated and a non-elevated token for the
/// same login session report the same user SID here, even though their *token owner* can differ.
fn current_user_sid() -> windows::core::Result<Vec<u8>> {
    unsafe {
        let mut token = HANDLE::default();
        OpenProcessToken(GetCurrentProcess(), TOKEN_QUERY, &mut token)?;
        let token = ScopedHandle(token);

        let mut required_len: u32 = 0;
        // Expected to "fail" with ERROR_INSUFFICIENT_BUFFER; we only want the required size.
        let _ = GetTokenInformation(token.0, TokenUser, None, 0, &mut required_len);

        let mut buffer = vec![0u8; required_len as usize];
        GetTokenInformation(
            token.0,
            TokenUser,
            Some(buffer.as_mut_ptr() as *mut c_void),
            required_len,
            &mut required_len,
        )?;

        // Safety: `buffer` was sized and filled by `GetTokenInformation` above for `TokenUser`,
        // which is documented to return a `TOKEN_USER` struct.
        let token_user = &*(buffer.as_ptr() as *const TOKEN_USER);
        copy_sid(token_user.User.Sid)
    }
}

/// Returns the SID bytes for the given well-known SID type (e.g. `LocalSystem`,
/// `BUILTIN\Administrators`).
fn well_known_sid(sid_type: WELL_KNOWN_SID_TYPE) -> windows::core::Result<Vec<u8>> {
    unsafe {
        let mut size: u32 = 0;
        // Expected to "fail" because the buffer is too small; we only want the required size.
        let _ = CreateWellKnownSid(sid_type, None, None, &mut size);
        let mut buf = vec![0u8; size as usize];
        CreateWellKnownSid(
            sid_type,
            None,
            Some(PSID(buf.as_mut_ptr() as *mut c_void)),
            &mut size,
        )?;
        Ok(buf)
    }
}

/// # Safety
/// `sid` must be a valid `PSID` for the duration of this call.
unsafe fn copy_sid(sid: PSID) -> windows::core::Result<Vec<u8>> {
    unsafe {
        let sid_len = GetLengthSid(sid);
        let mut sid_bytes = vec![0u8; sid_len as usize];
        CopySid(sid_len, PSID(sid_bytes.as_mut_ptr() as *mut c_void), sid)?;
        Ok(sid_bytes)
    }
}

/// RAII wrapper that closes a `HANDLE` on drop.
struct ScopedHandle(HANDLE);

impl Drop for ScopedHandle {
    fn drop(&mut self) {
        unsafe {
            let _ = windows::Win32::Foundation::CloseHandle(self.0);
        }
    }
}
