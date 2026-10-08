//! Native Windows ACL observations and deliberately broad fixtures for tests only.
//!
//! Available through `cortexkit_lease::test_support` with the opt-in
//! `test-support` feature on Windows. Enable it only on dev-dependencies.
//! Expectations come from the process token and ACLs read back from Windows,
//! not from the permission writer. Helpers panic on failed Win32 calls or failed
//! assertions; they are not production permission-management APIs.

use std::{
    mem::{size_of, zeroed},
    os::windows::ffi::OsStrExt,
    path::Path,
    ptr::{null, null_mut},
};
use windows_sys::Win32::{
    Foundation::{CloseHandle, LocalFree},
    Security::{
        AclSizeInformation,
        Authorization::{
            ConvertStringSecurityDescriptorToSecurityDescriptorW, GetNamedSecurityInfoW,
            SetNamedSecurityInfoW, SDDL_REVISION_1, SE_FILE_OBJECT,
        },
        GetAce, GetAclInformation, GetLengthSid, GetSecurityDescriptorControl,
        GetSecurityDescriptorDacl, GetTokenInformation, TokenUser, ACCESS_ALLOWED_ACE, ACE_HEADER,
        ACL, ACL_SIZE_INFORMATION, CONTAINER_INHERIT_ACE, DACL_SECURITY_INFORMATION, INHERITED_ACE,
        OBJECT_INHERIT_ACE, PROTECTED_DACL_SECURITY_INFORMATION, PSECURITY_DESCRIPTOR, PSID,
        SE_DACL_PROTECTED, TOKEN_QUERY, TOKEN_USER,
    },
    Storage::FileSystem::FILE_ALL_ACCESS,
    System::Threading::{GetCurrentProcess, OpenProcessToken},
};

struct Descriptor(PSECURITY_DESCRIPTOR);

impl Drop for Descriptor {
    fn drop(&mut self) {
        // SAFETY: The descriptor was allocated by an API that requires LocalFree.
        unsafe { LocalFree(self.0) };
    }
}

fn wide(path: &Path) -> Vec<u16> {
    path.as_os_str().encode_wide().chain(Some(0)).collect()
}

/// Returns the current process token user's SID as its native binary bytes.
///
/// Panics if the token or its user information cannot be read.
pub fn current_user_sid() -> Vec<u8> {
    let mut token = null_mut();
    // SAFETY: token is writable and the current process pseudo handle is valid.
    assert_ne!(
        unsafe { OpenProcessToken(GetCurrentProcess(), TOKEN_QUERY, &mut token) },
        0
    );
    let mut bytes = 0;
    // SAFETY: A null buffer queries the needed size; token is live.
    unsafe { GetTokenInformation(token, TokenUser, null_mut(), 0, &mut bytes) };
    assert!(bytes as usize >= size_of::<TOKEN_USER>());
    let mut data = vec![0usize; (bytes as usize).div_ceil(size_of::<usize>())];
    // SAFETY: data is aligned and large enough for TOKEN_USER and its SID.
    let result = unsafe {
        GetTokenInformation(
            token,
            TokenUser,
            data.as_mut_ptr().cast(),
            bytes,
            &mut bytes,
        )
    };
    // SAFETY: token was opened successfully and is no longer needed.
    unsafe { CloseHandle(token) };
    assert_ne!(result, 0);
    // SAFETY: The successful GetTokenInformation(TokenUser) call above
    // initialized a TOKEN_USER and its SID inside data. Copy the SID bytes while
    // data is still alive.
    unsafe {
        let sid = (*(data.as_ptr().cast::<TOKEN_USER>())).User.Sid;
        std::slice::from_raw_parts(sid.cast::<u8>(), GetLengthSid(sid) as usize).to_vec()
    }
}

#[derive(Debug, PartialEq, Eq)]
/// An access-allowed ACE observed directly in a named filesystem object's DACL.
pub struct Ace {
    /// Native ACE type; `read_acl` rejects types other than ACCESS_ALLOWED (0).
    pub kind: u8,
    /// Native ACE flags, including object/container inheritance and inherited status.
    pub flags: u8,
    /// Native access mask, compared to FILE_ALL_ACCESS by `assert_owner_only`.
    pub mask: u32,
    /// Trustee SID in its native binary representation.
    pub sid: Vec<u8>,
}

#[derive(Debug, PartialEq, Eq)]
/// DACL observations copied from the Windows security descriptor.
pub struct Snapshot {
    /// Whether SE_DACL_PROTECTED prevents inheritance from the parent.
    pub protected: bool,
    /// Access entries in their native ACL order.
    pub aces: Vec<Ace>,
}

/// Reads a named filesystem object's DACL with GetNamedSecurityInfoW.
///
/// Panics on a failed Win32 call, a null DACL, or any non-ACCESS_ALLOWED ACE.
pub fn read_acl(path: &Path) -> Snapshot {
    let name = wide(path);
    let mut acl: *mut ACL = null_mut();
    let mut raw = null_mut();
    // SAFETY: name is terminated and both output pointers are writable. Windows
    // returns a descriptor owning the ACL, kept alive through all observations.
    assert_eq!(
        unsafe {
            GetNamedSecurityInfoW(
                name.as_ptr(),
                SE_FILE_OBJECT,
                DACL_SECURITY_INFORMATION,
                null_mut(),
                null_mut(),
                &mut acl,
                null_mut(),
                &mut raw,
            )
        },
        0,
        "read named DACL for {}",
        path.display()
    );
    let descriptor = Descriptor(raw);
    assert!(!acl.is_null(), "a null DACL grants everyone access");
    let mut control = 0;
    let mut revision = 0;
    // SAFETY: descriptor is live; control and revision are writable outputs.
    assert_ne!(
        unsafe { GetSecurityDescriptorControl(descriptor.0, &mut control, &mut revision) },
        0
    );
    // SAFETY: ACL_SIZE_INFORMATION is a plain output structure initialized below.
    let mut info: ACL_SIZE_INFORMATION = unsafe { zeroed() };
    // SAFETY: acl belongs to the live descriptor and info is correctly sized.
    assert_ne!(
        unsafe {
            GetAclInformation(
                acl,
                (&mut info as *mut ACL_SIZE_INFORMATION).cast(),
                size_of::<ACL_SIZE_INFORMATION>() as u32,
                AclSizeInformation,
            )
        },
        0
    );
    let mut aces = Vec::new();
    for index in 0..info.AceCount {
        let mut raw_ace = null_mut();
        // SAFETY: index is within the count obtained from this live ACL.
        assert_ne!(unsafe { GetAce(acl, index, &mut raw_ace) }, 0);
        // SAFETY: The test fixtures contain ACCESS_ALLOWED ACEs. Check their type
        // before reading the corresponding mask and variable-length SID.
        unsafe {
            let header = &*raw_ace.cast::<ACE_HEADER>();
            assert_eq!(header.AceType, 0, "expected ACCESS_ALLOWED_ACE_TYPE");
            let ace = &*raw_ace.cast::<ACCESS_ALLOWED_ACE>();
            let sid: PSID = std::ptr::addr_of!(ace.SidStart).cast_mut().cast();
            aces.push(Ace {
                kind: ace.Header.AceType,
                flags: ace.Header.AceFlags,
                mask: ace.Mask,
                sid: std::slice::from_raw_parts(sid.cast::<u8>(), GetLengthSid(sid) as usize)
                    .to_vec(),
            });
        }
    }
    Snapshot {
        protected: control & SE_DACL_PROTECTED != 0,
        aces,
    }
}

/// Asserts the expected protected flag and exactly one current-token-user ACE
/// granting full control. Directory ACEs must inherit to files and directories;
/// unprotected descendants must carry an actually inherited ACE.
pub fn assert_owner_only(path: &Path, directory: bool, protected: bool) {
    let snapshot = read_acl(path);
    assert_eq!(
        snapshot.protected,
        protected,
        "DACL protected flag for {}",
        path.display()
    );
    assert_eq!(
        snapshot.aces.len(),
        1,
        "DACL must have exactly one access entry for {}",
        path.display()
    );
    let ace = &snapshot.aces[0];
    assert_eq!(ace.kind, 0, "only ACCESS_ALLOWED entries are permitted");
    assert_eq!(
        ace.sid,
        current_user_sid(),
        "ACE SID must equal the current process token user"
    );
    assert_eq!(ace.mask, FILE_ALL_ACCESS, "the user must have full control");
    if directory {
        assert_eq!(
            u32::from(ace.flags) & (OBJECT_INHERIT_ACE | CONTAINER_INHERIT_ACE),
            OBJECT_INHERIT_ACE | CONTAINER_INHERIT_ACE,
            "directory ACE must cover child files and directories"
        );
    }
    if !protected {
        assert_ne!(
            u32::from(ace.flags) & INHERITED_ACE,
            0,
            "the child ACE must actually be inherited"
        );
    }
}

/// Gives a test fixture a protected, inheritable Everyone full-control DACL.
///
/// This deliberately broadens access. Use only on disposable test paths.
/// Panics if Windows cannot apply or read back the fixture ACL.
pub fn grant_everyone(dir: &Path) {
    let name = wide(dir);
    let sddl: Vec<u16> = "D:P(A;OICI;FA;;;WD)"
        .encode_utf16()
        .chain(Some(0))
        .collect();
    let mut raw = null_mut();
    // SAFETY: sddl is terminated; raw receives a LocalFree-owned descriptor.
    assert_ne!(
        unsafe {
            ConvertStringSecurityDescriptorToSecurityDescriptorW(
                sddl.as_ptr(),
                SDDL_REVISION_1,
                &mut raw,
                null_mut(),
            )
        },
        0
    );
    let descriptor = Descriptor(raw);
    let mut present = 0;
    let mut defaulted = 0;
    let mut acl = null_mut();
    // SAFETY: descriptor owns the security descriptor parsed from the SDDL string
    // above and frees it only on drop, after this call; the output pointers are
    // writable locals.
    assert_ne!(
        unsafe { GetSecurityDescriptorDacl(descriptor.0, &mut present, &mut acl, &mut defaulted) },
        0
    );
    assert_ne!(present, 0);
    assert!(!acl.is_null());
    // SAFETY: name and the ACL inside descriptor stay alive for the call. The
    // ACL grants Everyone (WD) full control, inherited by files and folders
    // (OICI), so the test starts from a deliberately broad directory. It is set
    // with the raw Win32 call, not the code under test, so the precondition
    // does not depend on what is being tested.
    assert_eq!(
        unsafe {
            SetNamedSecurityInfoW(
                name.as_ptr(),
                SE_FILE_OBJECT,
                DACL_SECURITY_INFORMATION | PROTECTED_DACL_SECURITY_INFORMATION,
                null_mut(),
                null_mut(),
                acl,
                null(),
            )
        },
        0
    );
    let snapshot = read_acl(dir);
    assert_eq!(snapshot.aces.len(), 1);
    assert_ne!(
        snapshot.aces[0].sid,
        current_user_sid(),
        "broad fixture must not already be owner-only"
    );
}

/// Asserts that a fixture is unprotected and inherits an ACE for a non-user SID.
pub fn assert_broad_inherited(path: &Path) {
    let snapshot = read_acl(path);
    assert!(
        !snapshot.protected,
        "fixture must inherit the broad parent DACL"
    );
    assert!(
        snapshot
            .aces
            .iter()
            .any(|ace| ace.sid != current_user_sid() && u32::from(ace.flags) & INHERITED_ACE != 0),
        "fixture must include a non-user inherited ACE"
    );
}
