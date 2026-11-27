use std::ffi::{OsStr, OsString};
use std::fs::{File, OpenOptions};
use std::io;
use std::mem::{offset_of, size_of, size_of_val};
use std::os::windows::ffi::{OsStrExt, OsStringExt};
use std::os::windows::fs::OpenOptionsExt;
use std::os::windows::io::{AsRawHandle, FromRawHandle};
use std::path::Path;
use std::ptr::{null, null_mut};

use windows_sys::Wdk::Foundation::OBJECT_ATTRIBUTES;
use windows_sys::Wdk::Storage::FileSystem::{
    NtCreateFile, FILE_OPEN_REPARSE_POINT, FILE_SYNCHRONOUS_IO_NONALERT,
    NTCREATEFILE_CREATE_DISPOSITION, NTCREATEFILE_CREATE_OPTIONS,
};
use windows_sys::Win32::Foundation::{RtlNtStatusToDosError, ERROR_NO_MORE_FILES, UNICODE_STRING};
use windows_sys::Win32::Storage::FileSystem::{
    FileAttributeTagInfo, FileIdBothDirectoryInfo, FileIdBothDirectoryRestartInfo,
    GetFileInformationByHandleEx, GetFileType, FILE_ACCESS_RIGHTS, FILE_ATTRIBUTE_DIRECTORY,
    FILE_ATTRIBUTE_NORMAL, FILE_ATTRIBUTE_REPARSE_POINT, FILE_ATTRIBUTE_TAG_INFO,
    FILE_FLAG_BACKUP_SEMANTICS, FILE_FLAG_OPEN_REPARSE_POINT, FILE_ID_BOTH_DIR_INFO,
    FILE_SHARE_DELETE, FILE_SHARE_READ, FILE_SHARE_WRITE, FILE_TYPE_DISK, SYNCHRONIZE,
};
use windows_sys::Win32::System::Ioctl::FSCTL_GET_REPARSE_POINT;
use windows_sys::Win32::System::SystemServices::{
    IO_REPARSE_TAG_MOUNT_POINT, IO_REPARSE_TAG_SYMLINK,
};
use windows_sys::Win32::System::IO::{DeviceIoControl, IO_STATUS_BLOCK};

#[derive(Clone, Copy, PartialEq, Eq)]
pub(super) enum Kind {
    Directory,
    File,
    Symlink,
    Junction,
    Unsupported,
}

pub(super) fn root(path: &Path) -> io::Result<File> {
    let dir = OpenOptions::new()
        .read(true)
        .custom_flags(FILE_FLAG_BACKUP_SEMANTICS | FILE_FLAG_OPEN_REPARSE_POINT)
        .open(path)?;
    if kind(&dir)? != Kind::Directory {
        return Err(io::ErrorKind::NotADirectory.into());
    }
    Ok(dir)
}

pub(super) fn relative(
    parent: &File,
    name: &OsStr,
    access: FILE_ACCESS_RIGHTS,
    disposition: NTCREATEFILE_CREATE_DISPOSITION,
    options: NTCREATEFILE_CREATE_OPTIONS,
) -> io::Result<File> {
    let mut name: Vec<u16> = name.encode_wide().collect();
    if name.is_empty()
        || name == [46]
        || name == [46, 46]
        || name.iter().any(|&unit| matches!(unit, 0 | 47 | 58 | 92))
    {
        return Err(io::ErrorKind::InvalidInput.into());
    }
    let bytes: u16 = size_of_val(name.as_slice())
        .try_into()
        .map_err(|_| io::ErrorKind::InvalidInput)?;
    let mut name = UNICODE_STRING {
        Length: bytes,
        MaximumLength: bytes,
        Buffer: name.as_mut_ptr(),
    };
    let attributes = OBJECT_ATTRIBUTES {
        Length: size_of::<OBJECT_ATTRIBUTES>() as u32,
        RootDirectory: parent.as_raw_handle(),
        ObjectName: &mut name,
        Attributes: 0x40, // OBJ_CASE_INSENSITIVE, as in an ordinary Win32 open.
        SecurityDescriptor: null_mut(),
        SecurityQualityOfService: null_mut(),
    };
    let mut status = IO_STATUS_BLOCK::default();
    let mut handle = null_mut();
    // SAFETY: attributes names exactly one component relative to a live parent.
    // The name, attributes and output buffers live across the synchronous call.
    // FILE_OPEN_REPARSE_POINT opens the entry itself, never its destination.
    let result = unsafe {
        NtCreateFile(
            &mut handle,
            access | SYNCHRONIZE,
            &attributes,
            &mut status,
            null(),
            FILE_ATTRIBUTE_NORMAL,
            FILE_SHARE_READ | FILE_SHARE_WRITE | FILE_SHARE_DELETE,
            disposition,
            options | FILE_OPEN_REPARSE_POINT | FILE_SYNCHRONOUS_IO_NONALERT,
            null(),
            0,
        )
    };
    if result < 0 {
        // SAFETY: the converter accepts any NTSTATUS value and has no pointers.
        return Err(io::Error::from_raw_os_error(
            unsafe { RtlNtStatusToDosError(result) } as i32,
        ));
    }
    // SAFETY: a successful native open returns a new owned file handle.
    Ok(unsafe { File::from_raw_handle(handle) })
}

pub(super) fn kind(file: &File) -> io::Result<Kind> {
    // SAFETY: file owns the handle throughout the call.
    if unsafe { GetFileType(file.as_raw_handle()) } != FILE_TYPE_DISK {
        return Ok(Kind::Unsupported);
    }
    let mut info = FILE_ATTRIBUTE_TAG_INFO::default();
    // SAFETY: info has the size and layout corresponding to FileAttributeTagInfo.
    let ok = unsafe {
        GetFileInformationByHandleEx(
            file.as_raw_handle(),
            FileAttributeTagInfo,
            (&mut info as *mut FILE_ATTRIBUTE_TAG_INFO).cast(),
            size_of_val(&info) as u32,
        )
    };
    if ok == 0 {
        return Err(io::Error::last_os_error());
    }
    Ok(if info.FileAttributes & FILE_ATTRIBUTE_REPARSE_POINT != 0 {
        match info.ReparseTag {
            IO_REPARSE_TAG_SYMLINK => Kind::Symlink,
            IO_REPARSE_TAG_MOUNT_POINT => Kind::Junction,
            _ => Kind::Unsupported,
        }
    } else if info.FileAttributes & FILE_ATTRIBUTE_DIRECTORY != 0 {
        Kind::Directory
    } else {
        Kind::File
    })
}

pub(super) fn children(directory: &File) -> io::Result<Vec<OsString>> {
    const NAME: usize = offset_of!(FILE_ID_BOTH_DIR_INFO, FileName);
    const NAME_LEN: usize = offset_of!(FILE_ID_BOTH_DIR_INFO, FileNameLength);
    let mut storage = vec![0u64; 4096];
    let mut class = FileIdBothDirectoryRestartInfo;
    let mut children = Vec::new();
    loop {
        let size = size_of_val(storage.as_slice());
        // SAFETY: the buffer is 8-byte aligned and holds size writable bytes;
        // the open directory remains live while enumeration advances.
        let ok = unsafe {
            GetFileInformationByHandleEx(
                directory.as_raw_handle(),
                class,
                storage.as_mut_ptr().cast(),
                size as u32,
            )
        };
        if ok == 0 {
            let error = io::Error::last_os_error();
            if error.raw_os_error() == Some(ERROR_NO_MORE_FILES as i32) {
                return Ok(children);
            }
            return Err(error);
        }
        class = FileIdBothDirectoryInfo;
        // SAFETY: the initialized u64 buffer can also be read as size bytes.
        let mut records =
            unsafe { std::slice::from_raw_parts(storage.as_ptr().cast::<u8>(), size) };
        loop {
            let bad = || io::Error::from(io::ErrorKind::InvalidData);
            let next = number(records, 0)? as usize;
            let end = if next == 0 { records.len() } else { next };
            let length = number(records, NAME_LEN)? as usize;
            let name = records
                .get(..end)
                .and_then(|record| record.get(NAME..NAME + length))
                .ok_or_else(bad)?;
            let name = wide(name)?;
            if name != "." && name != ".." {
                children.push(name);
            }
            if next == 0 {
                break;
            }
            records = &records[next..];
        }
    }
}

pub(super) fn link_target(file: &File, kind: Kind) -> io::Result<OsString> {
    let mut data = [0u8; 16 * 1024];
    let mut length = 0;
    // SAFETY: the live file refers to the reparse point itself, data is the
    // maximum-size output buffer, and the returned byte count is writable.
    let ok = unsafe {
        DeviceIoControl(
            file.as_raw_handle(),
            FSCTL_GET_REPARSE_POINT,
            null(),
            0,
            data.as_mut_ptr().cast(),
            data.len() as u32,
            &mut length,
            null_mut(),
        )
    };
    if ok == 0 {
        return Err(io::Error::last_os_error());
    }
    let invalid = || io::Error::from(io::ErrorKind::InvalidData);
    let data = data.get(..length as usize).ok_or_else(invalid)?;
    // Both formats locate the exact substitute name relative to PathBuffer;
    // the symlink layout has an extra flags DWORD before the buffer.
    let start = match kind {
        Kind::Symlink if number(data, 0)? == IO_REPARSE_TAG_SYMLINK => 20,
        Kind::Junction if number(data, 0)? == IO_REPARSE_TAG_MOUNT_POINT => 16,
        _ => return Err(invalid()),
    };
    let start = start + usize::from(half(data, 8)?);
    let end = start + usize::from(half(data, 10)?);
    wide(data.get(start..end).ok_or_else(invalid)?)
}

fn number(bytes: &[u8], offset: usize) -> io::Result<u32> {
    bytes
        .get(offset..offset + 4)
        .and_then(|bytes| bytes.try_into().ok())
        .map(u32::from_le_bytes)
        .ok_or_else(|| io::ErrorKind::InvalidData.into())
}

fn half(bytes: &[u8], offset: usize) -> io::Result<u16> {
    bytes
        .get(offset..offset + 2)
        .and_then(|bytes| bytes.try_into().ok())
        .map(u16::from_le_bytes)
        .ok_or_else(|| io::ErrorKind::InvalidData.into())
}

fn wide(bytes: &[u8]) -> io::Result<OsString> {
    let (chunks, rest) = bytes.as_chunks();
    if !rest.is_empty() {
        return Err(io::ErrorKind::InvalidData.into());
    }
    Ok(OsString::from_wide(
        &chunks
            .iter()
            .copied()
            .map(u16::from_le_bytes)
            .collect::<Vec<_>>(),
    ))
}
