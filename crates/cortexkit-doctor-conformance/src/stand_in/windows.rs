use std::ffi::OsString;
use std::fs::File;
use std::io::{self, Write};
use std::mem::size_of_val;
use std::os::windows::ffi::{OsStrExt, OsStringExt};
use std::os::windows::io::AsRawHandle;
use std::path::Path;

use windows_sys::Wdk::Storage::FileSystem::{
    FILE_DIRECTORY_FILE, FILE_NON_DIRECTORY_FILE, FILE_OPEN, FILE_OPEN_IF,
};
use windows_sys::Win32::Storage::FileSystem::{
    FileDispositionInfo, FileIdInfo, GetFileInformationByHandleEx, LockFileEx,
    SetFileInformationByHandle, UnlockFileEx, DELETE, FILE_DISPOSITION_INFO, FILE_GENERIC_READ,
    FILE_GENERIC_WRITE, FILE_ID_INFO, FILE_READ_ATTRIBUTES, LOCKFILE_EXCLUSIVE_LOCK,
    LOCKFILE_FAIL_IMMEDIATELY,
};
use windows_sys::Win32::System::IO::{OVERLAPPED, OVERLAPPED_0, OVERLAPPED_0_0};

use crate::windows_fs::{self as win, Kind};
use crate::Liveness;

const UNSAFE_PATH: &str = "unsafe_liveness_path";
const SETUP_FAILED: &str = "liveness_setup_failed";

#[derive(Debug)]
pub(super) struct StandIn {
    file: File,
    pid_marker: Option<(File, OsString)>,
}

impl StandIn {
    pub(super) fn start(data: &Path, liveness: &Liveness) -> Result<Self, &'static str> {
        let (path, pid_file) = match liveness {
            Liveness::PidFile(path) => (path, true),
            Liveness::FlockFile(path) => (path, false),
        };
        // Keep the raw separators so a trailing slash or dot cannot silently
        // change a directory spelling into a marker file.
        let path: Vec<_> = path.as_os_str().encode_wide().collect();
        let separator = |unit| matches!(unit, 47 | 92);
        if path.first().is_none_or(|&unit| separator(unit))
            || path.iter().any(|&unit| matches!(unit, 0 | 58))
        {
            return Err(UNSAFE_PATH);
        }
        let mut parts = path.split(|&unit| separator(unit));
        let name = parts
            .next_back()
            .filter(|name| !name.is_empty() && *name != [46] && *name != [46, 46])
            .map(OsString::from_wide)
            .ok_or(UNSAFE_PATH)?;
        let mut parent = win::root(data).map_err(|_| UNSAFE_PATH)?;
        for part in parts.filter(|part| !part.is_empty() && *part != [46]) {
            parent = win::relative(
                &parent,
                &OsString::from_wide(part),
                FILE_GENERIC_READ,
                FILE_OPEN,
                FILE_DIRECTORY_FILE,
            )
            .map_err(|_| UNSAFE_PATH)?;
            if win::kind(&parent).ok() != Some(Kind::Directory) {
                return Err(UNSAFE_PATH);
            }
        }
        let access = FILE_GENERIC_READ | FILE_GENERIC_WRITE | if pid_file { DELETE } else { 0 };
        let file = win::relative(
            &parent,
            &name,
            access,
            FILE_OPEN_IF,
            FILE_NON_DIRECTORY_FILE,
        )
        .map_err(|_| UNSAFE_PATH)?;
        if win::kind(&file).ok() != Some(Kind::File) {
            return Err(UNSAFE_PATH);
        }
        let mut stand_in = Self {
            file,
            pid_marker: pid_file.then_some((parent, name)),
        };
        if pid_file {
            stand_in.file.set_len(0).map_err(|_| SETUP_FAILED)?;
            writeln!(stand_in.file, "{}", std::process::id()).map_err(|_| SETUP_FAILED)?;
        } else {
            lock(&stand_in.file).map_err(|_| SETUP_FAILED)?;
        }
        Ok(stand_in)
    }
}

fn lock(file: &File) -> io::Result<()> {
    let mut range = lock_range();
    // SAFETY: the file is synchronous and both the file and range outlive the call.
    if unsafe {
        LockFileEx(
            file.as_raw_handle(),
            LOCKFILE_EXCLUSIVE_LOCK | LOCKFILE_FAIL_IMMEDIATELY,
            0,
            1,
            0,
            &mut range,
        )
    } == 0
    {
        return Err(io::Error::last_os_error());
    }
    Ok(())
}

fn lock_range() -> OVERLAPPED {
    // A byte lock blocks ordinary reads. Reserve a byte past the contents so
    // the snapshot can still hash the whole file while the stand-in is live.
    OVERLAPPED {
        Anonymous: OVERLAPPED_0 {
            Anonymous: OVERLAPPED_0_0 {
                Offset: 0xffff_fffe,
                OffsetHigh: 0x7fff_ffff,
            },
        },
        ..OVERLAPPED::default()
    }
}

fn identity(file: &File) -> io::Result<(u64, [u8; 16])> {
    let mut id = FILE_ID_INFO::default();
    // SAFETY: id has the size and layout required by FileIdInfo.
    let ok = unsafe {
        GetFileInformationByHandleEx(
            file.as_raw_handle(),
            FileIdInfo,
            (&mut id as *mut FILE_ID_INFO).cast(),
            size_of_val(&id) as u32,
        )
    };
    if ok == 0 {
        return Err(io::Error::last_os_error());
    }
    Ok((id.VolumeSerialNumber, id.FileId.Identifier))
}

impl Drop for StandIn {
    fn drop(&mut self) {
        let Some((parent, name)) = &self.pid_marker else {
            let mut range = lock_range();
            // SAFETY: file and range are live and identify the exact byte
            // reserved by the stand-in. Closing the file is the fallback.
            unsafe { UnlockFileEx(self.file.as_raw_handle(), 0, 1, 0, &mut range) };
            return;
        };
        let current = win::relative(parent, name, FILE_READ_ATTRIBUTES, FILE_OPEN, 0)
            .and_then(|current| identity(&current));
        if !identity(&self.file)
            .is_ok_and(|original| current.is_ok_and(|current| current == original))
        {
            return;
        }
        let disposition = FILE_DISPOSITION_INFO { DeleteFile: true };
        // SAFETY: disposition has the required layout, and only the original
        // marker's handle can be deleted, not a replacement at its old path.
        unsafe {
            SetFileInformationByHandle(
                self.file.as_raw_handle(),
                FileDispositionInfo,
                (&disposition as *const FILE_DISPOSITION_INFO).cast(),
                size_of_val(&disposition) as u32,
            )
        };
    }
}

#[cfg(test)]
#[path = "../../tests/support/junction.rs"]
mod junction;

#[cfg(test)]
mod tests {
    use std::fs::{self, OpenOptions};

    use super::*;
    use windows_sys::Win32::Foundation::ERROR_LOCK_VIOLATION;

    #[test]
    fn kit_core_windows_stand_in_rejects_reparse_points_without_writing() {
        let data = tempfile::tempdir().unwrap();
        let other = tempfile::tempdir().unwrap();
        let marker = other.path().join("marker");
        fs::write(&marker, "untouched").unwrap();
        let regular = data.path().join("regular");
        fs::write(&regular, "untouched").unwrap();
        junction::junction(other.path(), &data.path().join("via-dir")).unwrap();
        for path in [
            "via-dir/marker",
            "via-dir",
            "../marker",
            r"\marker",
            "regular/",
            "regular/.",
            "regular:stream",
        ] {
            for liveness in [
                Liveness::PidFile(path.into()),
                Liveness::FlockFile(path.into()),
            ] {
                assert_eq!(
                    StandIn::start(data.path(), &liveness).unwrap_err(),
                    UNSAFE_PATH,
                    "{path}"
                );
            }
        }
        assert_eq!(fs::read(&marker).unwrap(), b"untouched");
        assert_eq!(fs::read(&regular).unwrap(), b"untouched");
    }

    #[test]
    fn kit_core_windows_stand_in_publishes_pid_and_exclusively_locks_reserved_byte() {
        let data = tempfile::tempdir().unwrap();
        fs::create_dir(data.path().join("run")).unwrap();
        let pid = data.path().join("run/doctor.pid");
        fs::write(&pid, "old marker longer than a pid").unwrap();
        let stand_in =
            StandIn::start(data.path(), &Liveness::PidFile("run/doctor.pid".into())).unwrap();
        assert_eq!(
            fs::read_to_string(&pid).unwrap(),
            format!("{}\n", std::process::id())
        );
        drop(stand_in);
        assert!(!pid.exists());

        let path = data.path().join("run/doctor.lock");
        fs::write(&path, "actual file contents").unwrap();
        let stand_in =
            StandIn::start(data.path(), &Liveness::FlockFile("run/doctor.lock".into())).unwrap();
        assert_eq!(fs::read(&path).unwrap(), b"actual file contents");
        let probe = OpenOptions::new()
            .read(true)
            .write(true)
            .open(&path)
            .unwrap();
        assert_eq!(
            lock(&probe).unwrap_err().raw_os_error(),
            Some(ERROR_LOCK_VIOLATION as i32)
        );
        drop(stand_in);
        lock(&probe).unwrap();
    }
}
