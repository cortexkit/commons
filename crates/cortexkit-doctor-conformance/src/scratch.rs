use std::collections::BTreeMap;
use std::ffi::{OsStr, OsString};
use std::fs::{self, File};
use std::io;
#[cfg(unix)]
use std::os::unix::ffi::OsStrExt;
use std::path::{Path, PathBuf};

use cortexkit_doctor::runner::Options;
#[cfg(unix)]
use rustix::fd::AsFd;
#[cfg(unix)]
use rustix::fs::{self as unix, AtFlags, Dir, FileType, Mode, OFlags};
#[cfg(unix)]
use rustix::io::Errno;
use sha2::{Digest, Sha256};

use crate::Adapter;

#[cfg(unix)]
pub(super) const ENTRY_FLAGS: OFlags = OFlags::NOFOLLOW
    .union(OFlags::NONBLOCK)
    .union(OFlags::CLOEXEC);
#[cfg(unix)]
pub(super) const DIRECTORY_FLAGS: OFlags =
    ENTRY_FLAGS.union(OFlags::RDONLY).union(OFlags::DIRECTORY);

pub(super) struct Scratch {
    _directory: tempfile::TempDir,
    root: PathBuf,
    data: PathBuf,
    config: PathBuf,
}

impl Scratch {
    pub(super) fn new(adapter: &mut impl Adapter) -> Result<Self, &'static str> {
        Self::seed(adapter).map_err(|_| "setup_failed")
    }

    fn seed(adapter: &mut impl Adapter) -> io::Result<Self> {
        let directory = tempfile::Builder::new().prefix("doctor-").tempdir()?;
        let root = directory.path().canonicalize()?;
        let data = root.join("data");
        let config = root.join("config");
        fs::create_dir(&data)?;
        fs::create_dir(&config)?;
        adapter.seed(&data, &config)?;
        if !data.symlink_metadata()?.is_dir() || !config.symlink_metadata()?.is_dir() {
            return Err(io::ErrorKind::NotADirectory.into());
        }
        Ok(Self {
            _directory: directory,
            root,
            data,
            config,
        })
    }

    pub(super) fn runner_options(&self) -> Options {
        Options::new()
            .with_data_home(&self.data)
            .with_config_home(&self.config)
    }

    pub(super) fn data_home(&self) -> &Path {
        &self.data
    }

    pub(super) fn snapshot(&self) -> Result<Snapshot, &'static str> {
        #[cfg(unix)]
        let root = unix::open(&self.root, DIRECTORY_FLAGS, Mode::empty())
            .map_err(|_| "snapshot_failed")?;
        #[cfg(windows)]
        let root = crate::windows_fs::root(&self.root).map_err(|_| "snapshot_failed")?;
        let mut snapshot = Snapshot::default();
        for home in ["data", "config"] {
            snapshot.record(&root, home.as_ref(), home.as_ref())?;
        }
        Ok(snapshot)
    }
}

#[derive(Debug, Default, PartialEq, Eq)]
pub(super) struct Snapshot(BTreeMap<PathBuf, Entry>);

#[derive(Debug, PartialEq, Eq)]
enum Entry {
    Directory,
    Symlink(OsString),
    #[cfg(windows)]
    Junction(OsString),
    File {
        size: u64,
        sha256: [u8; 32],
    },
}

impl Snapshot {
    #[cfg(unix)]
    fn record(&mut self, parent: impl AsFd, name: &OsStr, path: &Path) -> Result<(), &'static str> {
        let stat = match unix::statat(&parent, name, AtFlags::SYMLINK_NOFOLLOW) {
            Ok(stat) => stat,
            Err(Errno::NOENT) => return Ok(()),
            Err(_) => return Err("snapshot_failed"),
        };
        let entry = match FileType::from_raw_mode(stat.st_mode) {
            FileType::Directory => {
                let child = unix::openat(&parent, name, DIRECTORY_FLAGS, Mode::empty())
                    .map_err(|_| "snapshot_failed")?;
                let mut entries = Dir::read_from(&child).map_err(|_| "snapshot_failed")?;
                for entry in &mut entries {
                    let entry = entry.map_err(|_| "snapshot_failed")?;
                    let name = entry.file_name().to_bytes();
                    if name != b"." && name != b".." {
                        let name = OsStr::from_bytes(name);
                        self.record(&child, name, &path.join(name))?;
                    }
                }
                Entry::Directory
            }
            FileType::Symlink => {
                let target =
                    unix::readlinkat(&parent, name, Vec::new()).map_err(|_| "snapshot_failed")?;
                Entry::Symlink(OsStr::from_bytes(target.to_bytes()).to_owned())
            }
            FileType::RegularFile => regular_file(&parent, name)?,
            _ => return Err("unsupported_entry"),
        };
        self.0.insert(path.to_owned(), entry);
        Ok(())
    }

    #[cfg(windows)]
    fn record(&mut self, parent: &File, name: &OsStr, path: &Path) -> Result<(), &'static str> {
        use crate::windows_fs::{self as win, Kind};
        use windows_sys::Wdk::Storage::FileSystem::FILE_OPEN;
        use windows_sys::Win32::Storage::FileSystem::{FILE_READ_ATTRIBUTES, FILE_READ_DATA};

        let file = match win::relative(
            parent,
            name,
            FILE_READ_ATTRIBUTES | FILE_READ_DATA,
            FILE_OPEN,
            0,
        ) {
            Ok(file) => file,
            Err(err) if err.kind() == io::ErrorKind::NotFound => return Ok(()),
            Err(_) => return Err("snapshot_failed"),
        };
        let kind = win::kind(&file).map_err(|_| "snapshot_failed")?;
        let entry = match kind {
            Kind::Directory => {
                for name in win::children(&file).map_err(|_| "snapshot_failed")? {
                    self.record(&file, &name, &path.join(&name))?;
                }
                Entry::Directory
            }
            Kind::Symlink => {
                Entry::Symlink(win::link_target(&file, kind).map_err(|_| "snapshot_failed")?)
            }
            Kind::Junction => {
                Entry::Junction(win::link_target(&file, kind).map_err(|_| "snapshot_failed")?)
            }
            Kind::File => hash_file(file)?,
            Kind::Unsupported => return Err("unsupported_entry"),
        };
        self.0.insert(path.to_owned(), entry);
        Ok(())
    }
}

#[cfg(unix)]
fn regular_file(parent: impl AsFd, name: &OsStr) -> Result<Entry, &'static str> {
    let file = unix::openat(parent, name, ENTRY_FLAGS | OFlags::RDONLY, Mode::empty())
        .map_err(|_| "snapshot_failed")?;
    hash_file(File::from(file))
}

fn hash_file(mut file: File) -> Result<Entry, &'static str> {
    let metadata = file.metadata().map_err(|_| "snapshot_failed")?;
    if !metadata.is_file() {
        return Err("unsupported_entry");
    }
    let mut hasher = Sha256::new();
    io::copy(&mut file, &mut hasher).map_err(|_| "snapshot_failed")?;
    Ok(Entry::File {
        size: metadata.len(),
        sha256: hasher.finalize().into(),
    })
}
