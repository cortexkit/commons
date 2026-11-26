use std::ffi::{OsStr, OsString};
use std::fs::File;
use std::io::Write;
use std::os::unix::ffi::OsStrExt;
use std::path::Path;

use rustix::fd::OwnedFd;
use rustix::fs::{self, AtFlags, FileType, FlockOperation, Mode, OFlags};
use rustix::io::Errno;

use crate::scratch::{DIRECTORY_FLAGS, ENTRY_FLAGS};
use crate::Liveness;

const UNSAFE_PATH: &str = "unsafe_liveness_path";
const SETUP_FAILED: &str = "liveness_setup_failed";

#[derive(Debug)]
pub(super) struct StandIn {
    file: File,
    pid_marker: Option<(OwnedFd, OsString)>,
}

impl StandIn {
    pub(super) fn start(data: &Path, liveness: &Liveness) -> Result<Self, &'static str> {
        let (path, pid_file) = match liveness {
            Liveness::PidFile(path) => (path, true),
            Liveness::FlockFile(path) => (path, false),
        };
        if path.is_absolute() {
            return Err(UNSAFE_PATH);
        }
        // Keep trailing slashes and dots: `marker/` and `marker/.` name
        // directories, not the regular file `marker`.
        let mut parts = path.as_os_str().as_bytes().split(|&byte| byte == b'/');
        let name = parts
            .next_back()
            .filter(|part| !matches!(*part, b"" | b"." | b".."))
            .map(OsStr::from_bytes)
            .ok_or(UNSAFE_PATH)?;
        let mut parent = fs::open(data, DIRECTORY_FLAGS, Mode::empty()).map_err(|_| UNSAFE_PATH)?;
        for part in parts.filter(|part| !part.is_empty()) {
            if part == b".." {
                return Err(UNSAFE_PATH);
            }
            parent = fs::openat(
                &parent,
                OsStr::from_bytes(part),
                DIRECTORY_FLAGS,
                Mode::empty(),
            )
            .map_err(|_| UNSAFE_PATH)?;
        }
        let file = open_marker(&parent, name)?;
        let mut stand_in = Self {
            file,
            pid_marker: pid_file.then(|| (parent, name.to_owned())),
        };
        if pid_file {
            stand_in.file.set_len(0).map_err(|_| SETUP_FAILED)?;
            writeln!(stand_in.file, "{}", std::process::id()).map_err(|_| SETUP_FAILED)?;
        } else {
            fs::flock(&stand_in.file, FlockOperation::NonBlockingLockExclusive)
                .map_err(|_| SETUP_FAILED)?;
        }
        Ok(stand_in)
    }
}

fn open_marker(parent: &OwnedFd, name: &OsStr) -> Result<File, &'static str> {
    let flags = ENTRY_FLAGS | OFlags::RDWR;
    let fd = match fs::statat(parent, name, AtFlags::SYMLINK_NOFOLLOW) {
        Ok(stat) if FileType::from_raw_mode(stat.st_mode) == FileType::RegularFile => {
            fs::openat(parent, name, flags, Mode::empty())
        }
        Err(Errno::NOENT) => fs::openat(
            parent,
            name,
            flags | OFlags::CREATE | OFlags::EXCL,
            Mode::RUSR | Mode::WUSR,
        ),
        _ => return Err(UNSAFE_PATH),
    }
    .map_err(|_| UNSAFE_PATH)?;
    let stat = fs::fstat(&fd).map_err(|_| UNSAFE_PATH)?;
    if FileType::from_raw_mode(stat.st_mode) != FileType::RegularFile {
        return Err(UNSAFE_PATH);
    }
    Ok(File::from(fd))
}

impl Drop for StandIn {
    fn drop(&mut self) {
        if let Some((parent, name)) = &self.pid_marker {
            let matches_open_file = fs::statat(parent, name, AtFlags::SYMLINK_NOFOLLOW)
                .and_then(|path| {
                    fs::fstat(&self.file)
                        .map(|open| (path.st_dev, path.st_ino) == (open.st_dev, open.st_ino))
                })
                .unwrap_or(false);
            // A module may have replaced the marker; remove only our own file.
            if matches_open_file {
                let _ = fs::unlinkat(parent, name, AtFlags::empty());
            }
        }
    }
}

#[cfg(test)]
mod tests;
