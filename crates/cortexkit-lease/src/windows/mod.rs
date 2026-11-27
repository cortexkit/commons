//! Owner-only Windows DACLs. All handles are opened without following a final
//! reparse point, and new directories receive their security at creation.

use std::{
    fs::{File, OpenOptions},
    io,
    mem::{offset_of, size_of, zeroed},
    os::windows::{
        fs::{MetadataExt, OpenOptionsExt},
        io::{AsRawHandle, FromRawHandle},
    },
    path::Path,
    ptr::{copy_nonoverlapping, null, null_mut},
};

use windows_sys::Win32::{
    Foundation::{
        CloseHandle, LocalFree, ERROR_ALREADY_EXISTS, ERROR_PATH_NOT_FOUND, GENERIC_READ,
        GENERIC_WRITE, HANDLE, INVALID_HANDLE_VALUE,
    },
    Security::{
        AddAccessAllowedAceEx,
        Authorization::{ConvertSidToStringSidW, GetSecurityInfo, SetSecurityInfo, SE_FILE_OBJECT},
        DeleteAce, EqualSid, GetAce, GetLengthSid, GetSecurityDescriptorControl,
        GetTokenInformation, InitializeAcl, InitializeSecurityDescriptor, IsValidSid,
        IsWellKnownSid, SetKernelObjectSecurity, SetSecurityDescriptorControl,
        SetSecurityDescriptorDacl, SetSecurityDescriptorOwner, TokenUser, WinCreatorOwnerRightsSid,
        ACCESS_ALLOWED_ACE, ACCESS_ALLOWED_OBJECT_ACE, ACE_HEADER,
        ACE_INHERITED_OBJECT_TYPE_PRESENT, ACE_OBJECT_TYPE_PRESENT, ACL, ACL_REVISION,
        CONTAINER_INHERIT_ACE, DACL_SECURITY_INFORMATION, INHERITED_ACE, INHERIT_ONLY_ACE,
        OBJECT_INHERIT_ACE, OWNER_SECURITY_INFORMATION, PROTECTED_DACL_SECURITY_INFORMATION,
        PSECURITY_DESCRIPTOR, PSID, SECURITY_ATTRIBUTES, SECURITY_DESCRIPTOR, SE_DACL_PROTECTED,
        TOKEN_QUERY, TOKEN_USER, UNPROTECTED_DACL_SECURITY_INFORMATION,
    },
    Storage::FileSystem::{
        CreateDirectoryW, CreateFileW, FileDispositionInfoEx, FileIdInfo, FlushFileBuffers,
        GetFileInformationByHandle, GetFileInformationByHandleEx, GetFileType, MoveFileExW,
        ReOpenFile, SetFileInformationByHandle, BY_HANDLE_FILE_INFORMATION, CREATE_NEW, DELETE,
        FILE_ALL_ACCESS, FILE_ATTRIBUTE_DIRECTORY, FILE_ATTRIBUTE_REPARSE_POINT,
        FILE_DISPOSITION_FLAG_DELETE, FILE_DISPOSITION_FLAG_POSIX_SEMANTICS,
        FILE_DISPOSITION_INFO_EX, FILE_FLAG_BACKUP_SEMANTICS, FILE_FLAG_OPEN_REPARSE_POINT,
        FILE_ID_INFO, FILE_READ_ATTRIBUTES, FILE_SHARE_DELETE, FILE_SHARE_READ, FILE_SHARE_WRITE,
        FILE_TYPE_DISK, MOVEFILE_REPLACE_EXISTING, MOVEFILE_WRITE_THROUGH, OPEN_EXISTING,
        READ_CONTROL, WRITE_DAC, WRITE_OWNER,
    },
    System::Threading::{GetCurrentProcess, OpenProcessToken},
};

use crate::{windows_path::wide, NotPrivate};

struct Handle(HANDLE);

impl Drop for Handle {
    fn drop(&mut self) {
        // SAFETY: Handle owns a valid handle returned by a successful Win32 call.
        unsafe { CloseHandle(self.0) };
    }
}

// A waiter that opened the file before locking must not prevent its removal.
pub(super) const LEASE_FILE_SHARE: u32 = FILE_SHARE_READ | FILE_SHARE_WRITE | FILE_SHARE_DELETE;

pub(super) fn lease_path_matches(file: &File, path: &Path) -> io::Result<bool> {
    let entry = OpenOptions::new()
        .access_mode(FILE_READ_ATTRIBUTES)
        .share_mode(LEASE_FILE_SHARE)
        .custom_flags(FILE_FLAG_OPEN_REPARSE_POINT)
        .open(path)?;
    let locked = file_id(file)?;
    let entry = file_id(&entry)?;
    Ok(locked.VolumeSerialNumber == entry.VolumeSerialNumber
        && locked.FileId.Identifier == entry.FileId.Identifier)
}

fn file_id(file: &File) -> io::Result<FILE_ID_INFO> {
    // SAFETY: FILE_ID_INFO is a plain Win32 output structure. The file and
    // writable buffer of the requested size are live throughout the call.
    let mut id: FILE_ID_INFO = unsafe { zeroed() };
    if unsafe {
        GetFileInformationByHandleEx(
            file.as_raw_handle(),
            FileIdInfo,
            (&mut id as *mut FILE_ID_INFO).cast(),
            size_of::<FILE_ID_INFO>() as u32,
        )
    } == 0
    {
        return Err(io::Error::last_os_error());
    }
    Ok(id)
}

pub(super) fn remove_locked_lease(file: &File) -> io::Result<()> {
    // SAFETY: `file` owns a live handle. Reopening it targets the locked file,
    // rather than looking up a name again, and only needs DELETE access.
    let delete = unsafe { ReOpenFile(file.as_raw_handle(), DELETE, LEASE_FILE_SHARE, 0) };
    if delete == INVALID_HANDLE_VALUE {
        return Err(io::Error::last_os_error());
    }
    let delete = Handle(delete);
    let disposition = FILE_DISPOSITION_INFO_EX {
        Flags: FILE_DISPOSITION_FLAG_DELETE | FILE_DISPOSITION_FLAG_POSIX_SEMANTICS,
    };
    // SAFETY: The delete handle and correctly sized disposition are live.
    if unsafe {
        SetFileInformationByHandle(
            delete.0,
            FileDispositionInfoEx,
            (&disposition as *const FILE_DISPOSITION_INFO_EX).cast(),
            size_of::<FILE_DISPOSITION_INFO_EX>() as u32,
        )
    } == 0
    {
        return Err(io::Error::last_os_error());
    }
    // Closing the POSIX delete handle unlinks the name even if waiters still
    // have it open. Use a separate delete handle so unlink precedes unlock.
    drop(delete);
    Ok(())
}

pub(super) fn durable_replace(temp: &Path, dest: &Path) -> io::Result<()> {
    let file = std::fs::OpenOptions::new().write(true).open(temp)?;
    // SAFETY: The file is open for write access and remains alive through the
    // flush call; FlushFileBuffers does not retain the handle.
    if unsafe { FlushFileBuffers(file.as_raw_handle() as HANDLE) } == 0 {
        return Err(io::Error::last_os_error());
    }
    drop(file);

    let temp = wide(temp)?;
    let dest = wide(dest)?;
    // SAFETY: Both paths are NUL-terminated UTF-16 strings valid for this call.
    if unsafe {
        MoveFileExW(
            temp.as_ptr(),
            dest.as_ptr(),
            MOVEFILE_REPLACE_EXISTING | MOVEFILE_WRITE_THROUGH,
        )
    } == 0
    {
        return Err(io::Error::last_os_error());
    }
    Ok(())
}

pub(super) fn sync_dir(dir: &Path) -> io::Result<()> {
    let name = wide(dir)?;
    // SAFETY: `name` is NUL-terminated; the returned handle is checked before
    // it is wrapped in Handle, which closes it on every exit path.
    let handle = unsafe {
        CreateFileW(
            name.as_ptr(),
            GENERIC_WRITE,
            FILE_SHARE_READ | FILE_SHARE_WRITE | FILE_SHARE_DELETE,
            null(),
            OPEN_EXISTING,
            FILE_FLAG_BACKUP_SEMANTICS,
            null_mut(),
        )
    };
    if handle == INVALID_HANDLE_VALUE {
        return Err(io::Error::last_os_error());
    }
    let handle = Handle(handle);
    // SAFETY: The handle is open for write access on the requested directory.
    if unsafe { FlushFileBuffers(handle.0) } == 0 {
        return Err(io::Error::last_os_error());
    }
    Ok(())
}

struct User(Vec<usize>);

impl User {
    fn current() -> io::Result<Self> {
        let mut token = null_mut();
        // SAFETY: The pseudo process handle is valid and token is a writable output.
        if unsafe { OpenProcessToken(GetCurrentProcess(), TOKEN_QUERY, &mut token) } == 0 {
            return Err(io::Error::last_os_error());
        }
        let token = Handle(token);
        let mut bytes = 0;
        // SAFETY: A null buffer with length zero requests the required buffer size.
        unsafe { GetTokenInformation(token.0, TokenUser, null_mut(), 0, &mut bytes) };
        if (bytes as usize) < size_of::<TOKEN_USER>() {
            return Err(io::Error::last_os_error());
        }
        let mut user = Self(vec![0; (bytes as usize).div_ceil(size_of::<usize>())]);
        // SAFETY: The buffer is pointer-aligned, has at least bytes bytes, and stays
        // alive with the embedded SID for as long as User is used.
        if unsafe {
            GetTokenInformation(
                token.0,
                TokenUser,
                user.0.as_mut_ptr().cast(),
                bytes,
                &mut bytes,
            )
        } == 0
        {
            return Err(io::Error::last_os_error());
        }
        Ok(user)
    }

    fn sid(&self) -> PSID {
        // SAFETY: current initialized an aligned TOKEN_USER and its embedded SID.
        unsafe { (*(self.0.as_ptr().cast::<TOKEN_USER>())).User.Sid }
    }
}

struct OwnerAcl(Vec<u32>);

impl OwnerAcl {
    fn new(directory: bool) -> io::Result<Self> {
        let user = User::current()?;
        // SAFETY: user owns a valid token SID for the duration of this call.
        let sid_bytes = unsafe { GetLengthSid(user.sid()) } as usize;
        let bytes =
            size_of::<ACL>() + size_of::<ACCESS_ALLOWED_ACE>() - size_of::<u32>() + sid_bytes;
        let mut acl = Self(vec![0; bytes.div_ceil(size_of::<u32>())]);
        let flags = if directory {
            OBJECT_INHERIT_ACE | CONTAINER_INHERIT_ACE
        } else {
            0
        };
        // SAFETY: The u32-aligned ACL buffer is large enough for its header and
        // one ACE. AddAccessAllowedAceEx copies the SID before user is dropped.
        if unsafe {
            InitializeAcl(
                acl.as_mut_ptr(),
                (acl.0.len() * size_of::<u32>()) as u32,
                ACL_REVISION,
            ) == 0
                || AddAccessAllowedAceEx(
                    acl.as_mut_ptr(),
                    ACL_REVISION,
                    flags,
                    FILE_ALL_ACCESS,
                    user.sid(),
                ) == 0
        } {
            return Err(io::Error::last_os_error());
        }
        Ok(acl)
    }

    fn as_mut_ptr(&mut self) -> *mut ACL {
        self.0.as_mut_ptr().cast()
    }

    fn append_directory_allow(&mut self, user: &User, used_bytes: usize) -> io::Result<()> {
        // SAFETY: user owns a valid token SID. The ACL header is initialized.
        let (sid_bytes, capacity) = unsafe {
            (
                GetLengthSid(user.sid()) as usize,
                (*self.as_mut_ptr()).AclSize,
            )
        };
        let bytes = used_bytes + offset_of!(ACCESS_ALLOWED_ACE, SidStart) + sid_bytes;
        let words = bytes.div_ceil(size_of::<u32>());
        let needed = u16::try_from(words * size_of::<u32>())
            .map_err(|_| io::Error::other("DACL is too large to add the owner's entry"))?;
        if needed > capacity {
            self.0.resize(words, 0);
            // SAFETY: The initialized ACL now has at least needed bytes of aligned storage.
            unsafe { (*self.as_mut_ptr()).AclSize = needed };
        }
        let acl = self.as_mut_ptr();
        // SAFETY: acl is initialized and has room for the new entry. The call
        // copies the live SID and appends the allow after the retained denies.
        if unsafe {
            AddAccessAllowedAceEx(
                acl,
                u32::from((*acl).AclRevision),
                OBJECT_INHERIT_ACE | CONTAINER_INHERIT_ACE,
                FILE_ALL_ACCESS,
                user.sid(),
            )
        } == 0
        {
            return Err(io::Error::last_os_error());
        }
        Ok(())
    }
}

fn private_descriptor(acl: &mut OwnerAcl, user: &User) -> io::Result<SECURITY_DESCRIPTOR> {
    // SAFETY: SECURITY_DESCRIPTOR is a plain Win32 structure initialized below.
    let mut descriptor: SECURITY_DESCRIPTOR = unsafe { zeroed() };
    let descriptor_ptr = (&mut descriptor as *mut SECURITY_DESCRIPTOR).cast();
    // SAFETY: descriptor is aligned and writable. Its pointers refer to acl/user,
    // which the caller keeps alive through creation. Revision 1 is the Win32
    // SECURITY_DESCRIPTOR_REVISION. Neither setter retains descriptor_ptr.
    if unsafe {
        InitializeSecurityDescriptor(descriptor_ptr, 1) == 0
            || SetSecurityDescriptorDacl(descriptor_ptr, 1, acl.as_mut_ptr(), 0) == 0
            || SetSecurityDescriptorOwner(descriptor_ptr, user.sid(), 0) == 0
            || SetSecurityDescriptorControl(descriptor_ptr, SE_DACL_PROTECTED, SE_DACL_PROTECTED)
                == 0
    } {
        return Err(io::Error::last_os_error());
    }
    Ok(descriptor)
}

pub(super) fn create_private_file(path: &Path) -> io::Result<std::fs::File> {
    let name = wide(path)?;
    let user = User::current()?;
    let mut acl = OwnerAcl::new(false)?;
    let mut descriptor = private_descriptor(&mut acl, &user)?;
    let attributes = SECURITY_ATTRIBUTES {
        nLength: size_of::<SECURITY_ATTRIBUTES>() as u32,
        lpSecurityDescriptor: (&mut descriptor as *mut SECURITY_DESCRIPTOR).cast(),
        bInheritHandle: 0,
    };
    // SAFETY: name and all descriptor/ACL/SID storage are live for this call.
    // CREATE_NEW refuses existing entries, and OPEN_REPARSE_POINT never follows
    // a final link. The protected DACL and owner are applied atomically at creation.
    let raw = unsafe {
        CreateFileW(
            name.as_ptr(),
            GENERIC_READ | GENERIC_WRITE,
            FILE_SHARE_READ | FILE_SHARE_WRITE | FILE_SHARE_DELETE,
            &attributes,
            CREATE_NEW,
            FILE_FLAG_OPEN_REPARSE_POINT,
            null_mut(),
        )
    };
    if raw == INVALID_HANDLE_VALUE {
        return Err(io::Error::last_os_error());
    }
    // SAFETY: CreateFileW returned a valid owned handle; File takes sole ownership.
    Ok(unsafe { std::fs::File::from_raw_handle(raw) })
}

fn open_no_follow(path: &Path, access: u32) -> io::Result<(Handle, u32)> {
    let name = wide(path)?;
    // SAFETY: name is NUL-terminated; no security attributes or template are used.
    // OPEN_REPARSE_POINT opens the final link itself, not its target.
    let raw = unsafe {
        CreateFileW(
            name.as_ptr(),
            access,
            FILE_SHARE_READ | FILE_SHARE_WRITE | FILE_SHARE_DELETE,
            null(),
            OPEN_EXISTING,
            FILE_FLAG_OPEN_REPARSE_POINT | FILE_FLAG_BACKUP_SEMANTICS,
            null_mut(),
        )
    };
    if raw == INVALID_HANDLE_VALUE {
        return Err(io::Error::last_os_error());
    }
    let handle = Handle(raw);
    // SAFETY: BY_HANDLE_FILE_INFORMATION is a plain Win32 output structure.
    let mut info: BY_HANDLE_FILE_INFORMATION = unsafe { zeroed() };
    // SAFETY: handle is live and info is a correctly sized writable output.
    if unsafe { GetFileInformationByHandle(handle.0, &mut info) } == 0 {
        return Err(io::Error::last_os_error());
    }
    // SAFETY: handle is live; GetFileType takes no output pointers.
    if unsafe { GetFileType(handle.0) } != FILE_TYPE_DISK {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            "not a disk file",
        ));
    }
    Ok((handle, info.dwFileAttributes))
}

struct LocalAllocation(*mut std::ffi::c_void);

impl Drop for LocalAllocation {
    fn drop(&mut self) {
        // SAFETY: The pointer is an allocation returned by a successful Win32
        // security API, each of which specifies LocalFree for its release.
        unsafe { LocalFree(self.0) };
    }
}

struct Security {
    _descriptor: LocalAllocation,
    owner: PSID,
    dacl: *mut ACL,
}

impl Security {
    fn read(handle: &Handle) -> io::Result<Self> {
        let mut owner = null_mut();
        let mut dacl = null_mut();
        let mut descriptor: PSECURITY_DESCRIPTOR = null_mut();
        // SAFETY: handle has READ_CONTROL and outputs are writable. The owner
        // and DACL point into the returned descriptor, retained by Self.
        let error = unsafe {
            GetSecurityInfo(
                handle.0,
                SE_FILE_OBJECT,
                OWNER_SECURITY_INFORMATION | DACL_SECURITY_INFORMATION,
                &mut owner,
                null_mut(),
                &mut dacl,
                null_mut(),
                &mut descriptor,
            )
        };
        if error != 0 {
            return Err(io::Error::from_raw_os_error(error as i32));
        }
        Ok(Self {
            _descriptor: LocalAllocation(descriptor),
            owner,
            dacl,
        })
    }

    fn dacl_is_protected(&self) -> io::Result<bool> {
        let mut control = 0;
        let mut revision = 0;
        // SAFETY: GetSecurityInfo returned the live descriptor and both outputs are writable.
        if unsafe { GetSecurityDescriptorControl(self._descriptor.0, &mut control, &mut revision) }
            == 0
        {
            return Err(io::Error::last_os_error());
        }
        Ok(control & SE_DACL_PROTECTED != 0)
    }
}

struct Protection {
    handle: Handle,
    attributes: u32,
    user: User,
    change_owner: bool,
}

impl Protection {
    fn open(path: &Path) -> io::Result<Self> {
        let (handle, attributes) = open_no_follow(path, READ_CONTROL | WRITE_DAC)?;
        let user = User::current()?;
        let security = Security::read(&handle)?;
        // Owners implicitly have READ_CONTROL and WRITE_DAC, but not WRITE_OWNER.
        // Request the latter only when an elevated token or another creator left
        // an owner different from TokenUser.
        // SAFETY: Both SIDs belong to live Win32 structures; a missing owner is
        // checked before EqualSid is called.
        let change_owner =
            security.owner.is_null() || unsafe { EqualSid(security.owner, user.sid()) } == 0;
        let (handle, attributes) = if change_owner {
            open_no_follow(path, READ_CONTROL | WRITE_DAC | WRITE_OWNER)?
        } else {
            (handle, attributes)
        };
        Ok(Self {
            handle,
            attributes,
            user,
            change_owner,
        })
    }

    fn information(&self) -> u32 {
        DACL_SECURITY_INFORMATION
            | PROTECTED_DACL_SECURITY_INFORMATION
            | if self.change_owner {
                OWNER_SECURITY_INFORMATION
            } else {
                0
            }
    }

    fn owner(&self) -> PSID {
        if self.change_owner {
            self.user.sid()
        } else {
            null_mut()
        }
    }
}

fn principal(sid: PSID) -> io::Result<String> {
    let mut text = null_mut();
    // SAFETY: The SID belongs to a live token or security descriptor. The call
    // allocates a NUL-terminated SID string, released with LocalFree below.
    if unsafe { ConvertSidToStringSidW(sid, &mut text) } == 0 {
        return Err(io::Error::last_os_error());
    }
    let _text = LocalAllocation(text.cast());
    // SAFETY: ConvertSidToStringSidW returned a valid NUL-terminated UTF-16 string.
    let value = unsafe {
        let mut len = 0;
        while *text.add(len) != 0 {
            len += 1;
        }
        String::from_utf16_lossy(std::slice::from_raw_parts(text, len))
    };
    // Well-known account aliases are cheap and cannot trigger a domain lookup.
    // LookupAccountSidW can contact domain controllers even with a null system
    // name, so unfamiliar SIDs are reported without attempting name resolution.
    let account = match value.as_str() {
        "S-1-1-0" => Some("Everyone"),
        "S-1-5-11" => Some("NT AUTHORITY\\Authenticated Users"),
        "S-1-5-18" => Some("NT AUTHORITY\\SYSTEM"),
        "S-1-5-19" => Some("NT AUTHORITY\\LOCAL SERVICE"),
        "S-1-5-20" => Some("NT AUTHORITY\\NETWORK SERVICE"),
        "S-1-5-32-544" => Some("BUILTIN\\Administrators"),
        "S-1-5-32-545" => Some("BUILTIN\\Users"),
        "S-1-5-32-546" => Some("BUILTIN\\Guests"),
        _ => None,
    };
    Ok(match account {
        Some(account) => format!("{value} ({account})"),
        None => value,
    })
}

pub(super) fn verify_private(path: &Path) -> Result<(), NotPrivate> {
    let (handle, attributes) = open_no_follow(path, READ_CONTROL)?;
    if attributes & FILE_ATTRIBUTE_REPARSE_POINT != 0 {
        return Err(NotPrivate::Link);
    }
    let user = User::current()?;
    let security = Security::read(&handle)?;
    if security.owner.is_null() {
        return Err(NotPrivate::Owner {
            principal: "unknown (missing owner SID)".into(),
        });
    }
    // SAFETY: Both SIDs belong to live, initialized Win32 security structures.
    if unsafe { EqualSid(security.owner, user.sid()) } == 0 {
        return Err(NotPrivate::Owner {
            principal: principal(security.owner)?,
        });
    }
    if !security.dacl_is_protected()? {
        return Err(NotPrivate::Unprotected);
    }
    if security.dacl.is_null() {
        return Err(NotPrivate::Access {
            principal: "S-1-1-0 (Everyone; null DACL)".into(),
        });
    }
    // SAFETY: GetSecurityInfo returned a valid ACL in the live descriptor.
    let count = unsafe { (*security.dacl).AceCount };
    for index in 0..count {
        let mut ace = null_mut();
        // SAFETY: The ACL is live and index is below its AceCount.
        if unsafe { GetAce(security.dacl, index as u32, &mut ace) } == 0 {
            return Err(io::Error::last_os_error().into());
        }
        // SAFETY: Every ACE returned by GetAce starts with an ACE_HEADER.
        let kind = unsafe { (*ace.cast::<ACE_HEADER>()).AceType };
        match kind {
            0 => {
                // SAFETY: Type 0 is ACCESS_ALLOWED_ACE. Its embedded SID remains
                // live with the descriptor until after comparison and formatting.
                let (mask, sid) = unsafe {
                    let allowed = &*ace.cast::<ACCESS_ALLOWED_ACE>();
                    (
                        allowed.Mask,
                        (&allowed.SidStart as *const u32).cast_mut().cast(),
                    )
                };
                // SAFETY: Both pointers refer to valid SIDs in live storage.
                if mask != 0 && unsafe { EqualSid(sid, user.sid()) } == 0 {
                    return Err(NotPrivate::Access {
                        principal: principal(sid)?,
                    });
                }
            }
            // ACCESS_DENIED, ACCESS_DENIED_OBJECT, ACCESS_DENIED_CALLBACK,
            // ACCESS_DENIED_CALLBACK_OBJECT: none can grant access.
            1 | 6 | 10 | 12 => {}
            ace_type => return Err(NotPrivate::UnsupportedAce { ace_type }),
        }
    }
    Ok(())
}

pub(super) fn reset_to_inherited(path: &Path, recursive: bool) -> io::Result<()> {
    let (handle, attributes) = open_no_follow(path, READ_CONTROL | WRITE_DAC)?;
    let mut acl = OwnerAcl(vec![0; size_of::<ACL>().div_ceil(size_of::<u32>())]);
    // SAFETY: acl has enough aligned storage for an empty ACL header.
    if unsafe { InitializeAcl(acl.as_mut_ptr(), size_of::<ACL>() as u32, ACL_REVISION) } == 0 {
        return Err(io::Error::last_os_error());
    }
    // SAFETY: handle refers to the entry itself, even for reparse points. The
    // empty (not null) ACL removes explicit ACEs and unprotection restores parent
    // inheritance. Owner/group/SACL are not requested and remain unchanged.
    let error = unsafe {
        SetSecurityInfo(
            handle.0,
            SE_FILE_OBJECT,
            DACL_SECURITY_INFORMATION | UNPROTECTED_DACL_SECURITY_INFORMATION,
            null_mut(),
            null_mut(),
            acl.as_mut_ptr(),
            null(),
        )
    };
    if error != 0 {
        return Err(io::Error::from_raw_os_error(error as i32));
    }
    if recursive
        && attributes & FILE_ATTRIBUTE_DIRECTORY != 0
        && attributes & FILE_ATTRIBUTE_REPARSE_POINT == 0
    {
        for entry in std::fs::read_dir(path)? {
            reset_to_inherited(&entry?.path(), true)?;
        }
    }
    Ok(())
}

fn apply_file_protection(protection: &Protection) -> io::Result<()> {
    let mut acl = OwnerAcl::new(false)?;
    // SAFETY: The handle has WRITE_DAC and, only when needed, WRITE_OWNER.
    // ACL/SID storage stays live through the call. Group/SACL are unchanged,
    // and an already-correct owner is not requested. Protection disables parent ACEs.
    let error = unsafe {
        SetSecurityInfo(
            protection.handle.0,
            SE_FILE_OBJECT,
            protection.information(),
            protection.owner(),
            null_mut(),
            acl.as_mut_ptr(),
            null(),
        )
    };
    if error != 0 {
        return Err(io::Error::from_raw_os_error(error as i32));
    }
    Ok(())
}

pub(super) fn protect_dir_only(path: &Path) -> io::Result<()> {
    let protection = Protection::open(path)?;
    if protection.attributes & FILE_ATTRIBUTE_DIRECTORY == 0
        || protection.attributes & FILE_ATTRIBUTE_REPARSE_POINT != 0
    {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            "not a directory or is a reparse point",
        ));
    }
    let mut acl = OwnerAcl::new(true)?;
    let mut descriptor = private_descriptor(&mut acl, &protection.user)?;
    // SetSecurityInfo propagates inherited ACE changes to existing descendants.
    // The handle-based kernel setter changes only this object's descriptor.
    // SAFETY: The handle always has WRITE_DAC; WRITE_OWNER is added only when
    // ownership differs.
    // Descriptor/ACL/SID storage is live. The owner field is ignored unless an
    // ownership change is requested; only the protected DACL is always requested.
    if unsafe {
        SetKernelObjectSecurity(
            protection.handle.0,
            protection.information(),
            (&mut descriptor as *mut SECURITY_DESCRIPTOR).cast(),
        )
    } == 0
    {
        return Err(io::Error::last_os_error());
    }
    Ok(())
}

pub(super) fn protect_file(path: &Path) -> io::Result<()> {
    let metadata = match std::fs::symlink_metadata(path) {
        Ok(metadata) => metadata,
        Err(error) if error.kind() == io::ErrorKind::NotFound => return Ok(()),
        Err(error) => return Err(error),
    };
    if !metadata.is_file() || metadata.file_attributes() & FILE_ATTRIBUTE_REPARSE_POINT != 0 {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            format!(
                "{} is not a regular file; refusing to change its permissions",
                path.display()
            ),
        ));
    }
    let protection = match Protection::open(path) {
        Ok(opened) => opened,
        Err(error) if error.kind() == io::ErrorKind::NotFound => return Ok(()),
        Err(error) => return Err(error),
    };
    if protection.attributes & (FILE_ATTRIBUTE_DIRECTORY | FILE_ATTRIBUTE_REPARSE_POINT) != 0 {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            "not a regular file",
        ));
    }
    apply_file_protection(&protection)
}

pub(super) fn create_private_dir(dir: &Path) -> io::Result<()> {
    create_all(dir)?;
    let metadata = std::fs::symlink_metadata(dir)?;
    if metadata.file_attributes() & FILE_ATTRIBUTE_REPARSE_POINT != 0 {
        return Ok(());
    }
    // Existing directories may belong to another account. As on Unix, failure
    // to tighten them is diagnostic only; failure to create them is an error.
    if let Err(error) = narrow_existing_dir(dir) {
        eprintln!(
            "cortexkit-lease: {} could not be made owner-only: {error}",
            dir.display()
        );
    }
    Ok(())
}

fn narrow_existing_dir(dir: &Path) -> io::Result<()> {
    let (handle, attributes) = open_no_follow(dir, READ_CONTROL | WRITE_DAC)?;
    if attributes & FILE_ATTRIBUTE_REPARSE_POINT != 0 {
        return Ok(());
    }
    if attributes & FILE_ATTRIBUTE_DIRECTORY == 0 {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            "not a directory",
        ));
    }
    let user = User::current()?;
    let security = Security::read(&handle)?;
    let information = DACL_SECURITY_INFORMATION | PROTECTED_DACL_SECURITY_INFORMATION;
    if security.dacl.is_null() {
        // A null DACL grants everyone full control. Limit the new grant to the
        // user, without propagating a new inheritable ACE to existing children.
        let mut acl = OwnerAcl::new(true)?;
        let mut descriptor = private_descriptor(&mut acl, &user)?;
        // SAFETY: The handle has WRITE_DAC and the descriptor keeps referring to
        // live ACL/SID storage. No ownership change is requested.
        if unsafe {
            SetKernelObjectSecurity(
                handle.0,
                information,
                (&mut descriptor as *mut SECURITY_DESCRIPTOR).cast(),
            )
        } == 0
        {
            return Err(io::Error::last_os_error());
        }
        return Ok(());
    }

    let (mut acl, changed) = narrow_dacl(&security, &user)?;
    if !changed && security.dacl_is_protected()? {
        return Ok(());
    }
    // Apply protection with the filtered ACEs and leave the owner unchanged.
    // The file security setter also updates inherited access on existing,
    // unprotected descendants.
    // SAFETY: The handle has WRITE_DAC and the filtered ACL buffer is live.
    let error = unsafe {
        SetSecurityInfo(
            handle.0,
            SE_FILE_OBJECT,
            information,
            null_mut(),
            null_mut(),
            acl.as_mut_ptr(),
            null(),
        )
    };
    if error != 0 {
        return Err(io::Error::from_raw_os_error(error as i32));
    }
    Ok(())
}

fn narrow_dacl(security: &Security, user: &User) -> io::Result<(OwnerAcl, bool)> {
    // SAFETY: The caller checked for a null DACL. The live descriptor owns the
    // OS-supplied ACL and its header gives the size of the full allocation.
    let (bytes, count) = unsafe {
        let acl = &*security.dacl;
        (usize::from(acl.AclSize), u32::from(acl.AceCount))
    };
    let mut acl = OwnerAcl(vec![0; bytes.div_ceil(size_of::<u32>())]);
    // SAFETY: The destination has at least AclSize bytes and does not alias the source.
    unsafe { copy_nonoverlapping(security.dacl.cast::<u8>(), acl.as_mut_ptr().cast(), bytes) };

    // Removing entries backwards preserves order and keeps the remaining
    // indices stable. Order matters when an allow and deny cover the same right.
    let mut changed = false;
    let mut has_own_grant = false;
    let mut used_bytes = size_of::<ACL>();
    for index in (0..count).rev() {
        let mut raw = null_mut();
        // SAFETY: The index still exists in the live ACL and raw is writable.
        if unsafe { GetAce(acl.as_mut_ptr(), index, &mut raw) } == 0 {
            return Err(io::Error::last_os_error());
        }
        let header = raw.cast::<ACE_HEADER>();
        // SAFETY: The ACE starts with a header and AceSize covers its bytes.
        let ace = unsafe { std::slice::from_raw_parts(raw.cast::<u8>(), (*header).AceSize.into()) };
        let entry = classify_directory_ace(ace, security.owner, user)?;
        if matches!(entry, DirectoryAce::OtherAllow) {
            // SAFETY: The index still exists in the writable ACL; ace is no longer borrowed.
            if unsafe { DeleteAce(acl.as_mut_ptr(), index) } == 0 {
                return Err(io::Error::last_os_error());
            }
            changed = true;
            continue;
        }
        used_bytes += ace.len();
        if matches!(entry, DirectoryAce::OwnAllow) && ace[1] & INHERIT_ONLY_ACE as u8 == 0 {
            // An entry for descendants only or with no rights does not grant
            // the user access to this directory.
            let mask = offset_of!(ACCESS_ALLOWED_ACE, Mask);
            has_own_grant |= ace[mask..mask + size_of::<u32>()].iter().any(|&b| b != 0);
        }
        if ace[1] & INHERITED_ACE as u8 != 0 {
            // SAFETY: The live ACL owns this writable header. Clearing the
            // inherited bit makes the ACE explicit without changing its scope.
            unsafe { (*header).AceFlags &= !(INHERITED_ACE as u8) };
            changed = true;
        }
    }
    if !has_own_grant {
        // SAFETY: Both non-null SIDs belong to live Win32 structures.
        if security.owner.is_null() || unsafe { EqualSid(security.owner, user.sid()) } == 0 {
            // A non-owner may rely on access through a group or logon session.
            // Leave the original DACL in place rather than strip that access.
            return Err(io::Error::new(
                io::ErrorKind::PermissionDenied,
                "no current-user grant on a directory owned by another principal; refusing to remove possible group access",
            ));
        }
        // The owner can set the DACL. Replace group-only access with a direct
        // grant to the owner, without allowing any other principal or removing denies.
        acl.append_directory_allow(user, used_bytes)?;
        changed = true;
    }
    Ok((acl, changed))
}

enum DirectoryAce {
    Deny,
    OwnAllow,
    OtherAllow,
}

fn classify_directory_ace(ace: &[u8], owner: PSID, user: &User) -> io::Result<DirectoryAce> {
    let sid_start = match ace[0] {
        // All deny forms (ordinary, object, callback, and callback object).
        1 | 6 | 10 | 12 => return Ok(DirectoryAce::Deny),
        // Ordinary and callback allows have the same SID offset.
        0 | 9 => offset_of!(ACCESS_ALLOWED_ACE, SidStart),
        // Object and callback-object allows may omit either GUID before the SID.
        5 | 11 => {
            let start = offset_of!(ACCESS_ALLOWED_OBJECT_ACE, ObjectType);
            let flags = ace.get(start - 4..start).ok_or_else(invalid_ace_sid)?;
            let flags = u32::from_le_bytes([flags[0], flags[1], flags[2], flags[3]]);
            start
                + size_of::<windows_sys::core::GUID>()
                    * (flags & (ACE_OBJECT_TYPE_PRESENT | ACE_INHERITED_OBJECT_TYPE_PRESENT))
                        .count_ones() as usize
        }
        // Leave a DACL containing an unknown ACE untouched: guessing which
        // entries to remove could discard an access restriction.
        kind => {
            return Err(io::Error::other(format!(
                "unsupported DACL ACE type {kind}"
            )))
        }
    };
    let sid_bytes = ace.get(sid_start..).ok_or_else(invalid_ace_sid)?;
    let sub_authorities = *sid_bytes.get(1).ok_or_else(invalid_ace_sid)?;
    if sid_bytes.len() < 8 + 4 * usize::from(sub_authorities) {
        return Err(invalid_ace_sid());
    }
    let sid: PSID = sid_bytes.as_ptr().cast_mut().cast();
    // SAFETY: The ACE's full SID is in bounds; user and any non-null owner point
    // into live Win32 security structures. The SID APIs only read the buffers.
    unsafe {
        if IsValidSid(sid) == 0 {
            return Err(invalid_ace_sid());
        }
        if EqualSid(sid, user.sid()) != 0 {
            return Ok(DirectoryAce::OwnAllow);
        }
        if IsWellKnownSid(sid, WinCreatorOwnerRightsSid) != 0 {
            if !owner.is_null() && EqualSid(owner, user.sid()) != 0 {
                return Ok(DirectoryAce::OwnAllow);
            }
            // Leave the DACL untouched if OWNER RIGHTS refers to a different
            // owner: deleting it could give them implicit READ_CONTROL/WRITE_DAC.
            return Err(io::Error::other("OWNER RIGHTS ACE for a different owner"));
        }
    }
    // There is no special grant for SYSTEM, Administrators, or any other SID.
    Ok(DirectoryAce::OtherAllow)
}

fn invalid_ace_sid() -> io::Error {
    io::Error::new(io::ErrorKind::InvalidData, "invalid SID in DACL ACE")
}

fn create_all(dir: &Path) -> io::Result<()> {
    if dir.as_os_str().is_empty() {
        return Ok(());
    }
    match std::fs::symlink_metadata(dir) {
        Ok(metadata) if metadata.is_dir() || dir.is_dir() => return Ok(()),
        Ok(_) => {
            return Err(io::Error::new(
                io::ErrorKind::AlreadyExists,
                "not a directory",
            ));
        }
        Err(error) if error.kind() == io::ErrorKind::NotFound => {}
        Err(error) => return Err(error),
    }
    let name = wide(dir)?;
    let mut acl = OwnerAcl::new(true)?;
    let user = User::current()?;
    let mut descriptor = private_descriptor(&mut acl, &user)?;
    let descriptor_ptr = (&mut descriptor as *mut SECURITY_DESCRIPTOR).cast();
    let attributes = SECURITY_ATTRIBUTES {
        nLength: size_of::<SECURITY_ATTRIBUTES>() as u32,
        lpSecurityDescriptor: descriptor_ptr,
        bInheritHandle: 0,
    };
    // SAFETY: name is NUL-terminated and attributes references live descriptor/ACL
    // storage. The directory is private from the instant it becomes visible.
    if unsafe { CreateDirectoryW(name.as_ptr(), &attributes) } != 0 {
        return Ok(());
    }
    let error = io::Error::last_os_error();
    match error.raw_os_error().map(|code| code as u32) {
        Some(ERROR_ALREADY_EXISTS) => {
            let metadata = std::fs::symlink_metadata(dir)?;
            if metadata.is_dir()
                || (metadata.file_attributes() & FILE_ATTRIBUTE_REPARSE_POINT != 0 && dir.is_dir())
            {
                Ok(())
            } else {
                Err(error)
            }
        }
        Some(ERROR_PATH_NOT_FOUND) => {
            let parent = dir.parent().filter(|parent| *parent != dir).ok_or(error)?;
            create_all(parent)?;
            // SAFETY: The same live name and security attributes are used after
            // creating the parent. Concurrent creation is checked without chmod.
            if unsafe { CreateDirectoryW(name.as_ptr(), &attributes) } != 0 {
                return Ok(());
            }
            let error = io::Error::last_os_error();
            if error.raw_os_error() == Some(ERROR_ALREADY_EXISTS as i32) && dir.is_dir() {
                Ok(())
            } else {
                Err(error)
            }
        }
        _ => Err(error),
    }
}

#[cfg(any(test, feature = "test-support"))]
pub mod test_support;
#[cfg(test)]
mod tests;
