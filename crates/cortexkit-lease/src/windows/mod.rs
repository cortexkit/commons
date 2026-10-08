//! Owner-only Windows DACLs. All handles are opened without following a final
//! reparse point, and new directories receive their security at creation.

use std::{
    io,
    mem::{size_of, zeroed},
    os::windows::{ffi::OsStrExt, fs::MetadataExt},
    path::Path,
    ptr::{null, null_mut},
};

use windows_sys::Win32::{
    Foundation::{
        CloseHandle, ERROR_ALREADY_EXISTS, ERROR_PATH_NOT_FOUND, HANDLE, INVALID_HANDLE_VALUE,
    },
    Security::{
        AddAccessAllowedAceEx,
        Authorization::{SetSecurityInfo, SE_FILE_OBJECT},
        GetLengthSid, GetTokenInformation, InitializeAcl, InitializeSecurityDescriptor,
        SetSecurityDescriptorControl, SetSecurityDescriptorDacl, TokenUser, ACCESS_ALLOWED_ACE,
        ACL, ACL_REVISION, CONTAINER_INHERIT_ACE, DACL_SECURITY_INFORMATION, OBJECT_INHERIT_ACE,
        PROTECTED_DACL_SECURITY_INFORMATION, PSID, SECURITY_ATTRIBUTES, SECURITY_DESCRIPTOR,
        SE_DACL_PROTECTED, TOKEN_QUERY, TOKEN_USER,
    },
    Storage::FileSystem::{
        CreateDirectoryW, CreateFileW, GetFileInformationByHandle, GetFileType,
        BY_HANDLE_FILE_INFORMATION, FILE_ALL_ACCESS, FILE_ATTRIBUTE_DIRECTORY,
        FILE_ATTRIBUTE_REPARSE_POINT, FILE_FLAG_BACKUP_SEMANTICS, FILE_FLAG_OPEN_REPARSE_POINT,
        FILE_SHARE_DELETE, FILE_SHARE_READ, FILE_SHARE_WRITE, FILE_TYPE_DISK, OPEN_EXISTING,
        READ_CONTROL, WRITE_DAC,
    },
    System::Threading::{GetCurrentProcess, OpenProcessToken},
};

struct Handle(HANDLE);

impl Drop for Handle {
    fn drop(&mut self) {
        // SAFETY: Handle owns a valid handle returned by a successful Win32 call.
        unsafe { CloseHandle(self.0) };
    }
}

fn wide(path: &Path) -> io::Result<Vec<u16>> {
    let mut value: Vec<u16> = path.as_os_str().encode_wide().collect();
    if value.contains(&0) {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            "path contains NUL",
        ));
    }
    value.push(0);
    Ok(value)
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
}

fn open_no_follow(path: &Path) -> io::Result<(Handle, u32)> {
    let name = wide(path)?;
    // SAFETY: name is NUL-terminated; no security attributes or template are used.
    // OPEN_REPARSE_POINT opens the final link itself, not its target.
    let raw = unsafe {
        CreateFileW(
            name.as_ptr(),
            READ_CONTROL | WRITE_DAC,
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

fn apply(handle: &Handle, directory: bool) -> io::Result<()> {
    let mut acl = OwnerAcl::new(directory)?;
    // Windows can bypass directory traversal checks. SetSecurityInfo also
    // replaces inherited ACEs on existing descendants, so their own ACLs lose
    // broad access when the directory is narrowed. Protected child DACLs stay intact.
    // SAFETY: handle is live and acl contains a valid DACL. SetSecurityInfo copies
    // the DACL, leaving owner/group/SACL unchanged. Protection disables parent ACEs.
    let error = unsafe {
        SetSecurityInfo(
            handle.0,
            SE_FILE_OBJECT,
            DACL_SECURITY_INFORMATION | PROTECTED_DACL_SECURITY_INFORMATION,
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
    let (handle, attributes) = match open_no_follow(path) {
        Ok(opened) => opened,
        Err(error) if error.kind() == io::ErrorKind::NotFound => return Ok(()),
        Err(error) => return Err(error),
    };
    if attributes & (FILE_ATTRIBUTE_DIRECTORY | FILE_ATTRIBUTE_REPARSE_POINT) != 0 {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            "not a regular file",
        ));
    }
    apply(&handle, false)
}

pub(super) fn create_private_dir(dir: &Path) -> io::Result<()> {
    create_all(dir)?;
    let metadata = std::fs::symlink_metadata(dir)?;
    if metadata.file_attributes() & FILE_ATTRIBUTE_REPARSE_POINT != 0 {
        return Ok(());
    }
    // Existing directories may belong to another account. As on Unix, failure
    // to tighten them is diagnostic only; failure to create them is an error.
    let result = open_no_follow(dir).and_then(|(handle, attributes)| {
        if attributes & FILE_ATTRIBUTE_REPARSE_POINT != 0 {
            return Ok(());
        }
        if attributes & FILE_ATTRIBUTE_DIRECTORY == 0 {
            return Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                "not a directory",
            ));
        }
        apply(&handle, true)
    });
    if let Err(error) = result {
        eprintln!(
            "cortexkit-lease: {} could not be made owner-only: {error}",
            dir.display()
        );
    }
    Ok(())
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
    // SAFETY: SECURITY_DESCRIPTOR is a plain Win32 structure initialized below.
    let mut descriptor: SECURITY_DESCRIPTOR = unsafe { zeroed() };
    let descriptor_ptr = (&mut descriptor as *mut SECURITY_DESCRIPTOR).cast();
    // SAFETY: descriptor is aligned and writable; acl remains live until after
    // CreateDirectoryW copies the descriptor. Revision 1 is SECURITY_DESCRIPTOR_REVISION.
    if unsafe {
        InitializeSecurityDescriptor(descriptor_ptr, 1) == 0
            || SetSecurityDescriptorDacl(descriptor_ptr, 1, acl.as_mut_ptr(), 0) == 0
            || SetSecurityDescriptorControl(descriptor_ptr, SE_DACL_PROTECTED, SE_DACL_PROTECTED)
                == 0
    } {
        return Err(io::Error::last_os_error());
    }
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
