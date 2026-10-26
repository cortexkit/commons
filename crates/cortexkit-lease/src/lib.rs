//! The durable single-writer lease for CortexKit modules.
//!
//! A module that owns a database must never have two live writers on the same
//! logical store. A writer acquires this lease and FAILS if a live writer already
//! holds it.
//!
//! Two layers:
//! - **Liveness** comes from an OS advisory lock (the file impl). The kernel
//!   releases it on process death, so a crashed holder's lease is reclaimable for
//!   free with no stale-PID bookkeeping.
//! - **Fencing** comes from a persisted, monotonically increasing `epoch`. Every
//!   durable write carries the holder's epoch; a write fenced by a stale epoch
//!   (from a superseded writer) is rejected. The OS lock alone is enough for a
//!   single local process, but a distributed/cloud backend cannot rely on a kernel
//!   lock, so the epoch is the portable fence: the cloud variant does a
//!   compare-and-set on the expected epoch in the cloud store's write path.
//!
//! Everything is behind the [`LeaseStore`] trait returning a boxed [`LeaseHandle`]
//! so a file-based lease and a future cloud lease are interchangeable without the
//! caller naming a concrete type.
//!
//! ## Key namespacing
//!
//! A [`LeaseKey`] is `(module_id, backend, scope_key)`. The `module_id` and
//! `backend` are part of the key so two different modules sharing one lease
//! directory can never collide on the same `scope_key` (e.g. two modules both
//! using session id "abc" get distinct locks). This is a deliberate requirement:
//! the shared lease root is shared across all modules.

use std::{
    fs::{File, OpenOptions},
    io::{self, Read, Seek, SeekFrom, Write},
    path::{Path, PathBuf},
};

use fs2::FileExt;

#[cfg(windows)]
mod windows;

/// Native Windows ACL observations and fixtures for tests only.
///
/// Available on Windows with the opt-in `test-support` feature. Enable the
/// feature only on a dev-dependency; the fixture helpers deliberately grant
/// broad access and must not be used to configure production permissions.
#[cfg(all(windows, feature = "test-support"))]
pub use windows::test_support;

/// Why a file or directory could not be verified as private to the current user.
#[derive(Debug)]
#[non_exhaustive]
pub enum NotPrivate {
    /// The entry could not be inspected.
    Io(io::Error),
    /// The owner is not the current user (a SID on Windows, a uid on Unix).
    Owner { principal: String },
    /// The Windows DACL is not protected from parent permission changes.
    Unprotected,
    /// Another principal has access (a SID on Windows, a group or everyone on Unix).
    Access { principal: String },
    /// The entry is a symlink or Windows reparse point.
    Link,
    /// An unfamiliar Windows ACE cannot safely be interpreted.
    UnsupportedAce { ace_type: u8 },
}

impl std::fmt::Display for NotPrivate {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Io(error) => write!(f, "could not inspect permissions: {error}"),
            Self::Owner { principal } => write!(f, "owner is another principal: {principal}"),
            Self::Unprotected => f.write_str(
                "permissions are inherited from the parent folder, so a change there would apply here",
            ),
            Self::Access { principal } => {
                write!(f, "access granted to another principal: {principal}")
            }
            Self::Link => f.write_str("symlink or reparse point is not private"),
            Self::UnsupportedAce { ace_type } => write!(
                f,
                "unrecognized ACE type {ace_type}; refusing to assume it is private"
            ),
        }
    }
}

impl std::error::Error for NotPrivate {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Self::Io(error) => Some(error),
            _ => None,
        }
    }
}

impl From<io::Error> for NotPrivate {
    fn from(error: io::Error) -> Self {
        Self::Io(error)
    }
}

/// Checks that the entry's owner is the current user and no other principal is
/// granted access. This inspects the entry itself, not its ancestor directories,
/// and refuses symlinks and Windows reparse points rather than following them.
///
/// On Unix the owner must be the effective uid and no group/other mode bits may
/// be set. Unix mode bits are not inherited, so no inheritance-protection check
/// is needed. On Windows the owner must be TokenUser, the DACL must be protected
/// (`SE_DACL_PROTECTED`) and non-null, and every access-allowed ACE must grant
/// access only to that user. An unprotected DACL is refused even if it currently
/// grants access only to that user, because a parent's permission changes could
/// propagate into the entry. Deny ACEs cannot grant access and are ignored;
/// unfamiliar ACE types are refused.
/// Inspection errors, including a missing path, are returned as [`NotPrivate::Io`].
pub fn verify_private(path: &Path) -> Result<(), NotPrivate> {
    #[cfg(unix)]
    {
        use std::os::unix::fs::MetadataExt;
        let metadata = std::fs::symlink_metadata(path)?;
        if metadata.file_type().is_symlink() {
            return Err(NotPrivate::Link);
        }
        // SAFETY: geteuid has no arguments and returns the process's effective uid.
        if metadata.uid() != unsafe { libc::geteuid() } {
            return Err(NotPrivate::Owner {
                principal: format!("uid {}", metadata.uid()),
            });
        }
        if metadata.mode() & 0o070 != 0 {
            return Err(NotPrivate::Access {
                principal: format!("group (gid {})", metadata.gid()),
            });
        }
        if metadata.mode() & 0o007 != 0 {
            return Err(NotPrivate::Access {
                principal: "everyone (other)".into(),
            });
        }
        Ok(())
    }
    #[cfg(windows)]
    {
        windows::verify_private(path)
    }
    #[cfg(not(any(unix, windows)))]
    {
        let _ = path;
        Err(io::Error::new(
            io::ErrorKind::Unsupported,
            "permission verification is unsupported",
        )
        .into())
    }
}

/// Removes an entry's explicit Windows DACL and restores inheritance from its
/// parent, leaving its owner unchanged. With `recursive`, directories and all
/// descendants are reset; reparse points are reset as links, never traversed.
/// Callers must not concurrently replace entries in the tree while it is reset.
///
/// On Unix this is a no-op: Unix has no permission inheritance, and the caller
/// applies captured mode bits itself. No modes are widened or otherwise changed.
/// A missing path is still an error on every OS.
pub fn reset_to_inherited(path: &Path, recursive: bool) -> io::Result<()> {
    #[cfg(windows)]
    {
        windows::reset_to_inherited(path, recursive)
    }
    #[cfg(not(windows))]
    {
        let _ = recursive;
        std::fs::symlink_metadata(path)?;
        Ok(())
    }
}

/// Creates a new read/write file with owner-only permissions from the instant
/// it exists. An existing entry (including a symlink) is refused, never opened.
/// On Unix the mode is 0600 narrowed by the umask; on Windows a protected DACL
/// grants only the current user access and that user is explicitly the owner.
pub fn create_private_file(path: &Path) -> io::Result<File> {
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        OpenOptions::new()
            .read(true)
            .write(true)
            .create_new(true)
            .mode(0o600)
            .custom_flags(libc::O_NOFOLLOW)
            .open(path)
    }
    #[cfg(windows)]
    {
        windows::create_private_file(path)
    }
    #[cfg(not(any(unix, windows)))]
    {
        let _ = path;
        Err(io::Error::new(
            io::ErrorKind::Unsupported,
            "private file creation is unsupported",
        ))
    }
}

/// Tightens one existing directory to owner-only permissions without changing
/// any existing children. Symlinks and Windows reparse points are refused.
///
/// On Windows the directory gets a protected owner-only DACL and the current
/// user as owner. New children inherit its user-only ACE, but existing children
/// keep their current ACLs, including inherited ACEs that no longer match the
/// parent. This call does not make the directory's contents private; callers
/// use it when they mean exactly that. On Unix only the open directory's mode
/// is changed to 0700, never its children's modes.
pub fn protect_dir_only(path: &Path) -> io::Result<()> {
    #[cfg(unix)]
    {
        use std::os::unix::fs::{OpenOptionsExt, PermissionsExt};
        let directory = OpenOptions::new()
            .read(true)
            .custom_flags(libc::O_DIRECTORY | libc::O_NOFOLLOW)
            .open(path)?;
        directory.set_permissions(std::fs::Permissions::from_mode(0o700))
    }
    #[cfg(windows)]
    {
        windows::protect_dir_only(path)
    }
    #[cfg(not(any(unix, windows)))]
    {
        let _ = path;
        Err(io::Error::new(
            io::ErrorKind::Unsupported,
            "directory protection is unsupported",
        ))
    }
}

/// Force owner-only permissions on a file this process owns the lifecycle of.
///
/// Files created through `File::create` or `OpenOptions::create` get their mode
/// from the process umask, which on a default system means `0644` — readable by
/// every other account and, more to the point, by every other process running
/// as this user. That is the exposure that actually exists on a
/// single-account machine: every module, every worker, every tool, plus
/// anything that copies the tree (a backup, a restore, an `install` into a
/// shared location, a container bind-mount).
///
/// Applied on OPEN rather than only at creation, because a file that already
/// exists carries whatever mode it was given — by an older build, a copy, or a
/// restore. A creation-time-only fix protects exactly the installations with no
/// history and leaves the ones that matter permissive forever.
///
/// A path that is not a regular file is REFUSED rather than adjusted. Following
/// a symlink here would chmod a file the caller never named, which is a
/// privilege-escalation primitive wearing a hardening step's clothes.
///
/// A missing file is not an error: callers pass optional sidecars (a WAL that
/// exists only while the journal is active) and the absence of a file is
/// nothing to protect.
pub fn protect_file(path: &std::path::Path) -> std::io::Result<()> {
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let metadata = match std::fs::symlink_metadata(path) {
            Ok(metadata) => metadata,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(()),
            Err(error) => return Err(error),
        };
        if !metadata.is_file() {
            return Err(std::io::Error::new(
                std::io::ErrorKind::InvalidInput,
                format!(
                    "{} is not a regular file; refusing to change its permissions",
                    path.display()
                ),
            ));
        }
        if metadata.permissions().mode() & 0o777 != 0o600 {
            std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o600))?;
        }
    }
    #[cfg(windows)]
    windows::protect_file(path)?;
    #[cfg(not(any(unix, windows)))]
    let _ = path;
    Ok(())
}

/// Creates a module's data directory owner-only (0700) and tightens it if it
/// already exists with group or world access. `cortexkit-store` applies it to
/// every store's directory on open; a module that cannot open its store through
/// `cortexkit-store` calls it itself, beside `protect_file` for the files.
///
/// The files are 0600 (`protect_file`), but a directory created under the
/// default umask is 0755, and so is every missing parent `create_dir_all`
/// makes. Another account can then list the directory and read any file a
/// module leaves in it with a loose mode (backups, catalogs, logs), even though
/// the database itself is protected. Call it on every open, like
/// `protect_file`, because a directory made by an older build keeps its mode.
///
/// Only `dir` itself is tightened when it already exists; the directories
/// above it belong to whoever created them. A `dir` that is a symlink is left
/// alone rather than followed, for the same reason `protect_file` refuses
/// symlinks.
pub fn create_private_dir(dir: &std::path::Path) -> std::io::Result<()> {
    #[cfg(unix)]
    {
        use std::os::unix::fs::{DirBuilderExt, PermissionsExt};
        std::fs::DirBuilder::new()
            .recursive(true)
            .mode(0o700)
            .create(dir)?;
        // Narrowing an existing directory is a second layer: the files inside
        // are already 0600. So a failure here (a directory owned by another
        // account, a read-only mount) is reported on stderr, which the
        // supervisor captures, and the caller carries on. Creation above is
        // different: without the directory there is nothing to open.
        let metadata = std::fs::symlink_metadata(dir)?;
        if metadata.is_dir() && metadata.permissions().mode() & 0o077 != 0 {
            let mode = metadata.permissions().mode() & 0o700;
            if let Err(error) = std::fs::set_permissions(dir, std::fs::Permissions::from_mode(mode))
            {
                eprintln!(
                    "cortexkit-lease: {} is group or world accessible and could not be made owner-only: {error}",
                    dir.display()
                );
            }
        }
        Ok(())
    }
    #[cfg(windows)]
    {
        windows::create_private_dir(dir)
    }
    #[cfg(not(any(unix, windows)))]
    {
        std::fs::create_dir_all(dir)
    }
}

/// Flushes a completed temporary file, atomically replaces `dest`, then makes
/// the replacement durable where the local filesystem supports that barrier.
/// The temporary file must be in the same directory as `dest`; otherwise this
/// returns [`io::ErrorKind::InvalidInput`] before changing either path.
///
/// On Unix, the file is synced before rename and the containing directory is
/// synced afterward. On Windows, the file is flushed and `MoveFileExW` is
/// called with write-through enabled. On local filesystems that honor these
/// operations, a successful return requests durability across power loss. The
/// rename is atomic to concurrent observers, so a process crash does not expose
/// a partially copied destination; if the process crashes before this function
/// returns, the caller cannot know whether the old or new name is durable. This
/// is not a guarantee for remote filesystems, devices that ignore flushes, or
/// filesystems that do not provide the documented barriers.
pub fn durable_replace(temp: &Path, dest: &Path) -> io::Result<()> {
    let parent = require_same_parent_directory(temp, dest)?;

    #[cfg(unix)]
    {
        File::open(temp)?.sync_all()?;
        std::fs::rename(temp, dest)?;
        sync_dir(&parent)
    }
    #[cfg(windows)]
    {
        let _ = parent;
        windows::durable_replace(temp, dest)
    }
    #[cfg(not(any(unix, windows)))]
    {
        let _ = (temp, dest, parent);
        Err(io::Error::new(
            io::ErrorKind::Unsupported,
            "durable replacement is supported only on Unix and Windows",
        ))
    }
}

/// Flushes directory metadata after creating a name without replacing an
/// existing one.
///
/// On Unix this calls `fsync` on the directory. An `EINVAL` response is treated
/// as an unsupported filesystem operation, so `Ok(())` in that case does not
/// promise power-loss durability. On Windows this opens the directory for write
/// access and calls `FlushFileBuffers`; NTFS supports this operation, but other
/// filesystems may refuse it and their errors are returned. A successful
/// barrier on a local filesystem that honors it requests persistence across
/// power loss; it is not needed to preserve metadata across a process-only
/// crash and does not guarantee behavior for remote filesystems or devices that
/// ignore flushes.
pub fn sync_dir(dir: &Path) -> io::Result<()> {
    #[cfg(unix)]
    {
        match File::open(dir)?.sync_all() {
            // EINVAL means this filesystem refuses fsync on directories; unlike
            // other errors it is documented as an unsupported barrier.
            Err(error)
                if error.kind() == io::ErrorKind::InvalidInput
                    && error.raw_os_error().is_some() =>
            {
                Ok(())
            }
            result => result,
        }
    }
    #[cfg(windows)]
    {
        windows::sync_dir(dir)
    }
    #[cfg(not(any(unix, windows)))]
    {
        let _ = dir;
        Err(io::Error::new(
            io::ErrorKind::Unsupported,
            "directory syncing is supported only on Unix and Windows",
        ))
    }
}

fn require_same_parent_directory(temp: &Path, dest: &Path) -> io::Result<PathBuf> {
    fn parent_or_current(path: &Path) -> &Path {
        path.parent()
            .filter(|parent| !parent.as_os_str().is_empty())
            .unwrap_or_else(|| Path::new("."))
    }

    let temp_parent = std::fs::canonicalize(parent_or_current(temp))?;
    let dest_parent = std::fs::canonicalize(parent_or_current(dest))?;
    if temp_parent != dest_parent {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            "temporary file and destination must be in the same directory",
        ));
    }
    Ok(dest_parent)
}

/// Identifies the thing being single-writer-guarded, namespaced so distinct
/// modules cannot collide on a shared lease root.
///
/// `scope_key` is the module's own partition within its storage (for a
/// machine-global store it can be a fixed constant like `"global"`; for a
/// project-partitioned store it is the project/session key). `module_id` and
/// `backend` are always part of the derived lock identity.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LeaseKey {
    pub module_id: String,
    pub backend: String,
    pub scope_key: String,
}

impl LeaseKey {
    pub fn new(
        module_id: impl Into<String>,
        backend: impl Into<String>,
        scope_key: impl Into<String>,
    ) -> Self {
        Self {
            module_id: module_id.into(),
            backend: backend.into(),
            scope_key: scope_key.into(),
        }
    }

    /// The namespaced identity string the lock is derived from. Module and backend
    /// are included so the same `scope_key` under two modules maps to two locks.
    fn identity(&self) -> String {
        format!(
            "{}\u{1f}{}\u{1f}{}",
            self.module_id, self.backend, self.scope_key
        )
    }
}

/// A held single-writer lease. Dropping it releases the lease. The `epoch` is the
/// fence token a backend's durable writes carry so a superseded writer's writes
/// are rejected.
///
/// This is a trait (not a concrete struct) so a file-backed handle and a
/// cloud-backed handle are interchangeable. The file impl holds an OS lock for its
/// lifetime; a cloud impl would hold a lease record renewed against the cloud
/// store.
pub trait LeaseHandle: Send + Sync + std::fmt::Debug {
    /// The CAS fence token for this writer: strictly greater than any prior
    /// holder's. Durable writes carry it; a stale-epoch write must be rejected.
    fn epoch(&self) -> u64;

    /// The namespaced identity this lease was acquired for.
    fn key(&self) -> &LeaseKey;

    /// Raise this held EXCLUSIVE lease's epoch to at least `at_least`,
    /// persisting the new value before returning it. An epoch is never lowered:
    /// when the current epoch is already `at_least` or higher, nothing is
    /// written and the current epoch is returned. On success [`Self::epoch`]
    /// reports the returned value, and the next exclusive acquisition of the same
    /// key starts above it.
    ///
    /// For a writer whose fence token is behind a token already stamped in the
    /// data it guards, for example a database restored onto a machine that lacks
    /// the lease state the database was written under. Raising only ever fences
    /// MORE writers, and the caller already holds the exclusive lease, so no
    /// other live writer can be holding the raised value.
    ///
    /// Shared handles refuse: they are not writers, and their epoch is for
    /// observability only.
    ///
    /// The default implementation returns [`LeaseError::Io`] with
    /// [`io::ErrorKind::Unsupported`]. It cannot do nothing and report success,
    /// because a caller that needs the raise would then keep a fence token that
    /// every fenced write rejects; an implementor that can persist its epoch
    /// should override it.
    fn raise_epoch(&self, at_least: u64) -> Result<u64, LeaseError> {
        Err(LeaseError::Io(io::Error::new(
            io::ErrorKind::Unsupported,
            format!(
                "lease for module '{}' (backend {}, scope '{}') cannot raise its epoch to {at_least}: \
                 this lease implementation does not support raising",
                self.key().module_id,
                self.key().backend,
                self.key().scope_key
            ),
        )))
    }
}

#[derive(Debug)]
pub enum LeaseError {
    /// A live writer already holds the lease for this key.
    Held {
        key: LeaseKey,
    },
    Io(std::io::Error),
}

impl std::fmt::Display for LeaseError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            LeaseError::Held { key } => write!(
                f,
                "storage for module '{}' (backend {}, scope '{}') is held by a live writer",
                key.module_id, key.backend, key.scope_key
            ),
            LeaseError::Io(e) => write!(f, "lease io: {e}"),
        }
    }
}

impl std::error::Error for LeaseError {}

/// Acquire the single durable-writer lease for a [`LeaseKey`].
pub trait LeaseStore: Send + Sync {
    /// Acquire the lease, or `Err(Held)` if a live writer holds it. The returned
    /// handle must outlive the writer; dropping it releases the lease.
    fn acquire(&self, key: &LeaseKey) -> Result<Box<dyn LeaseHandle>, LeaseError>;

    /// Acquire the lease in SHARED mode: any number of shared holders may
    /// coexist, but a shared holder blocks [`LeaseStore::acquire`] (exclusive)
    /// and an exclusive holder blocks shared acquisition.
    ///
    /// Use for reader-side protection of shared resources: e.g. a model-cache
    /// consumer takes a shared lease on a blob's digest while validating or
    /// mmap-ing it, so a GC (exclusive holder) can never delete the file out
    /// from under a live reader, while concurrent readers never serialize each
    /// other.
    ///
    /// Shared handles do NOT bump the fence epoch (they are not writers; the
    /// epoch fences durable writes). [`LeaseHandle::epoch`] on a shared handle
    /// returns the last persisted writer epoch at acquisition time, for
    /// observability only — never use it as a write fence.
    fn acquire_shared(&self, key: &LeaseKey) -> Result<Box<dyn LeaseHandle>, LeaseError>;
}

/// File-based lease store: one lock file per key under `base_dir`. The OS advisory
/// lock provides liveness (released on process death); a persisted epoch in the
/// same file provides the fence token.
pub struct FileLeaseStore {
    base_dir: PathBuf,
}

impl FileLeaseStore {
    pub fn new(base_dir: impl Into<PathBuf>) -> Self {
        Self {
            base_dir: base_dir.into(),
        }
    }

    /// Stable per-key lock-file path: a deterministic hash of the namespaced
    /// identity, so the same key always maps to the same lock file across
    /// processes and restarts, and distinct modules never collide.
    fn lease_path(&self, key: &LeaseKey) -> PathBuf {
        self.base_dir
            .join(format!("{}.lease", fnv1a_hex(&key.identity())))
    }
}

/// A file-backed held lease: holds the OS advisory lock (exclusive or shared)
/// for its lifetime.
#[derive(Debug)]
struct FileLeaseHandle {
    /// Behind a mutex because [`LeaseHandle::raise_epoch`] takes `&self` (the
    /// handle may be shared through an `Arc`); holding it across the file
    /// write keeps the in-memory and persisted values in step.
    epoch: std::sync::Mutex<u64>,
    /// Holds the OS advisory lock until dropped.
    file: File,
    key: LeaseKey,
    /// Whether the OS lock is exclusive. Only an exclusive holder may write the
    /// persisted epoch.
    exclusive: bool,
}

impl LeaseHandle for FileLeaseHandle {
    fn epoch(&self) -> u64 {
        *self.epoch.lock().unwrap_or_else(|p| p.into_inner())
    }

    fn key(&self) -> &LeaseKey {
        &self.key
    }

    fn raise_epoch(&self, at_least: u64) -> Result<u64, LeaseError> {
        if !self.exclusive {
            return Err(LeaseError::Io(io::Error::new(
                io::ErrorKind::PermissionDenied,
                "a shared lease cannot raise the writer epoch",
            )));
        }
        let mut epoch = self.epoch.lock().unwrap_or_else(|p| p.into_inner());
        if *epoch >= at_least {
            return Ok(*epoch);
        }
        write_epoch(&self.file, at_least).map_err(LeaseError::Io)?;
        *epoch = at_least;
        Ok(at_least)
    }
}

impl Drop for FileLeaseHandle {
    fn drop(&mut self) {
        let _ = self.file.unlock();
    }
}

impl LeaseStore for FileLeaseStore {
    fn acquire(&self, key: &LeaseKey) -> Result<Box<dyn LeaseHandle>, LeaseError> {
        std::fs::create_dir_all(&self.base_dir).map_err(LeaseError::Io)?;
        let path = self.lease_path(key);
        let mut file = OpenOptions::new()
            .read(true)
            .write(true)
            .create(true)
            .truncate(false)
            .open(&path)
            .map_err(LeaseError::Io)?;
        // The lease is the single-writer fence, so a world-WRITABLE lease file is
        // an integrity question rather than a privacy one: anything able to write
        // the persisted epoch can forge the fence token that readers use to
        // detect a stale writer.
        protect_file(&path).map_err(LeaseError::Io)?;

        // Liveness gate: a live holder still owns the lock, so the try-lock fails
        // with the OS "contended" error.
        match file.try_lock_exclusive() {
            Ok(()) => {}
            Err(e) if is_lock_contended(&e) => {
                return Err(LeaseError::Held { key: key.clone() });
            }
            Err(e) => return Err(LeaseError::Io(e)),
        }

        // We hold the lock: bump the persisted epoch (the CAS fence token).
        let epoch = bump_epoch(&mut file).map_err(|e| {
            let _ = file.unlock();
            LeaseError::Io(e)
        })?;

        Ok(Box::new(FileLeaseHandle {
            epoch: std::sync::Mutex::new(epoch),
            file,
            key: key.clone(),
            exclusive: true,
        }))
    }

    fn acquire_shared(&self, key: &LeaseKey) -> Result<Box<dyn LeaseHandle>, LeaseError> {
        std::fs::create_dir_all(&self.base_dir).map_err(LeaseError::Io)?;
        let path = self.lease_path(key);
        let mut file = OpenOptions::new()
            .read(true)
            .write(true)
            .create(true)
            .truncate(false)
            .open(&path)
            .map_err(LeaseError::Io)?;
        protect_file(&path).map_err(LeaseError::Io)?;

        // Shared liveness gate: only an exclusive holder contends; other shared
        // holders coexist. On unix this is flock(LOCK_SH|LOCK_NB); on Windows,
        // LockFileEx without LOCKFILE_EXCLUSIVE_LOCK (both via fs2).
        // Fully-qualified call: std >= 1.89 has an inherent File::try_lock_shared
        // (returning TryLockError) that would otherwise shadow the fs2 trait
        // method this crate's error handling is built around.
        match FileExt::try_lock_shared(&file) {
            Ok(()) => {}
            Err(e) if is_lock_contended(&e) => {
                return Err(LeaseError::Held { key: key.clone() });
            }
            Err(e) => return Err(LeaseError::Io(e)),
        }

        // Read-only peek at the persisted writer epoch: shared holders are not
        // writers, so the epoch is NOT bumped (it fences durable writes only).
        let epoch = read_epoch(&mut file).map_err(|e| {
            let _ = file.unlock();
            LeaseError::Io(e)
        })?;

        Ok(Box::new(FileLeaseHandle {
            epoch: std::sync::Mutex::new(epoch),
            file,
            key: key.clone(),
            exclusive: false,
        }))
    }
}

/// Whether a `try_lock_exclusive` error means "another live holder owns the lock"
/// (vs a real IO failure). The OS-level contended error differs by platform:
/// `EWOULDBLOCK` on unix, `ERROR_LOCK_VIOLATION` on Windows, and `ErrorKind` only
/// maps the unix one to `WouldBlock` (the Windows code lands in the catch-all
/// kind). Comparing against fs2's own `lock_contended_error()` by raw OS code is
/// exact on both platforms, so a genuinely-held lease is reported as `Held`, never
/// misread as `Io`.
fn is_lock_contended(e: &std::io::Error) -> bool {
    e.raw_os_error() == fs2::lock_contended_error().raw_os_error()
}

/// Read the persisted epoch without modifying it (0 if new/empty). Called while
/// holding a shared OS lock; must not write (concurrent shared holders read the
/// same file).
fn read_epoch(file: &mut File) -> std::io::Result<u64> {
    let mut buf = String::new();
    file.seek(SeekFrom::Start(0))?;
    file.read_to_string(&mut buf)?;
    Ok(buf.trim().parse().unwrap_or(0))
}

/// Read the persisted epoch (0 if new/empty), increment, write it back, return the
/// new value. Called while holding the OS lock.
fn bump_epoch(file: &mut File) -> std::io::Result<u64> {
    let mut buf = String::new();
    file.seek(SeekFrom::Start(0))?;
    file.read_to_string(&mut buf)?;
    let prev: u64 = buf.trim().parse().unwrap_or(0);
    let next = prev.saturating_add(1);
    file.set_len(0)?;
    file.seek(SeekFrom::Start(0))?;
    file.write_all(next.to_string().as_bytes())?;
    file.flush()?;
    Ok(next)
}

/// Persist `epoch` as the file's whole contents and sync it to disk. Called
/// only by an exclusive holder, through the handle that holds the OS lock (on
/// Windows the byte-range lock refuses writes through any other handle).
///
/// The new digits are written over the old ones before the file is cut to the
/// new length and synced, rather than truncating first, so the write never
/// starts by emptying the file (an empty file reads as epoch 0). A crash during
/// the write or before the sync can still leave partial contents.
fn write_epoch(mut file: &File, epoch: u64) -> io::Result<()> {
    let text = epoch.to_string();
    file.seek(SeekFrom::Start(0))?;
    file.write_all(text.as_bytes())?;
    file.set_len(text.len() as u64)?;
    file.sync_all()
}

/// FNV-1a 64-bit, hex: a dependency-free deterministic filename hash.
fn fnv1a_hex(s: &str) -> String {
    let mut h: u64 = 0xcbf2_9ce4_8422_2325;
    for b in s.as_bytes() {
        h ^= *b as u64;
        h = h.wrapping_mul(0x0000_0100_0000_01b3);
    }
    format!("{h:016x}")
}

#[cfg(test)]
mod tests {
    use super::*;

    fn key(scope: &str) -> LeaseKey {
        LeaseKey::new("test-module", "sqlite", scope)
    }

    /// A fresh directory per call. A counter makes it unique: tests run in
    /// parallel in one process, and two calls in the same clock tick would
    /// otherwise share a directory that one test deletes while another still
    /// uses it. On Windows that deleted directory stays pending deletion, so
    /// the other test's next open fails with "path not found".
    fn tmp_store() -> (FileLeaseStore, PathBuf) {
        static NEXT: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
        let dir = std::env::temp_dir().join(format!(
            "cortexkit-lease-{}-{}",
            std::process::id(),
            NEXT.fetch_add(1, std::sync::atomic::Ordering::Relaxed)
        ));
        (FileLeaseStore::new(&dir), dir)
    }

    /// An acquired lease file is owner-only on disk, including one that already
    /// exists with a permissive mode.
    ///
    /// The pre-existing half is the case that bites: every lease already on a
    /// deployed machine was created at the umask default, so correcting only at
    /// creation time would protect exactly the installations with no history.
    ///
    /// The lease is the single-writer FENCE, so the exposure here is integrity
    /// rather than privacy — anything able to write the persisted epoch can
    /// forge the token readers use to detect a stale writer.
    ///
    /// Mutation-proved: removing the `protect_file` call from `acquire` fails
    /// this on the pre-existing case.
    #[cfg(unix)]
    #[test]
    fn an_acquired_lease_file_is_owner_only() {
        use std::os::unix::fs::PermissionsExt;

        let (store, dir) = tmp_store();
        let k = key("perm");

        // Reproduce a lease left behind by an older build at the umask default.
        std::fs::create_dir_all(&dir).expect("create lease dir");
        let path = store.lease_path(&k);
        std::fs::write(&path, b"").expect("pre-create lease file");
        std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o644))
            .expect("set permissive mode");

        let guard = store.acquire(&k).expect("acquire");
        let mode = std::fs::metadata(&path)
            .expect("stat lease")
            .permissions()
            .mode()
            & 0o777;
        assert_eq!(
            mode, 0o600,
            "the lease file stayed group/world writable at {mode:o}"
        );

        drop(guard);
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// `protect_file` refuses a symlink rather than following it.
    ///
    /// Following one would change the mode of a file the caller never named,
    /// which is a privilege-escalation primitive wearing a hardening step's
    /// clothes. The assertion is that the TARGET's mode is unchanged, not
    /// merely that an error came back — an implementation could chmod the
    /// target and still return Err.
    #[cfg(unix)]
    #[test]
    fn protect_file_refuses_a_symlink_and_leaves_its_target_untouched() {
        use std::os::unix::fs::PermissionsExt;

        let dir = std::env::temp_dir().join(format!(
            "cortexkit-lease-symlink-{}-{}",
            std::process::id(),
            fnv1a_hex(&format!("{:?}", std::time::Instant::now()))
        ));
        std::fs::create_dir_all(&dir).expect("create dir");
        let target = dir.join("target");
        std::fs::write(&target, b"not mine").expect("write target");
        std::fs::set_permissions(&target, std::fs::Permissions::from_mode(0o644))
            .expect("set target mode");
        let link = dir.join("link");
        std::os::unix::fs::symlink(&target, &link).expect("symlink");

        assert!(
            protect_file(&link).is_err(),
            "a symlink must be refused rather than followed"
        );
        let mode = std::fs::metadata(&target)
            .expect("stat target")
            .permissions()
            .mode()
            & 0o777;
        assert_eq!(
            mode, 0o644,
            "the symlink target was chmod-ed through the link"
        );

        let _ = std::fs::remove_dir_all(&dir);
    }

    /// A path that does not exist is not an error: callers pass optional
    /// sidecars (a WAL that exists only while the journal is active), and the
    /// absence of a file is nothing to protect. Without this, a first open of a
    /// fresh database would fail on its missing WAL.
    #[test]
    fn protect_file_ignores_a_missing_path() {
        let missing = std::env::temp_dir().join(format!(
            "cortexkit-lease-absent-{}-{}",
            std::process::id(),
            fnv1a_hex(&format!("{:?}", std::time::Instant::now()))
        ));
        assert!(protect_file(&missing).is_ok());
    }

    /// A missing directory is created 0700 with its missing parents; an
    /// existing 0755 one is narrowed to 0700; a symlink to a directory is left
    /// alone, and so is its target.
    #[cfg(unix)]
    #[test]
    fn create_private_dir_creates_and_narrows_but_never_follows_a_symlink() {
        use std::os::unix::fs::PermissionsExt;
        let mode = |p: &std::path::Path| {
            std::fs::symlink_metadata(p)
                .expect("stat")
                .permissions()
                .mode()
                & 0o777
        };
        let root = std::env::temp_dir().join(format!(
            "cortexkit-lease-privdir-{}-{}",
            std::process::id(),
            fnv1a_hex(&format!("{:?}", std::time::Instant::now()))
        ));

        let fresh = root.join("a").join("store");
        create_private_dir(&fresh).expect("create");
        assert_eq!(
            mode(&fresh),
            0o700,
            "a created directory must be owner-only"
        );

        let wide = root.join("wide");
        std::fs::create_dir_all(&wide).expect("mkdir");
        std::fs::set_permissions(&wide, std::fs::Permissions::from_mode(0o755)).expect("chmod");
        create_private_dir(&wide).expect("narrow");
        assert_eq!(
            mode(&wide),
            0o700,
            "an existing wide directory must be narrowed"
        );

        let target = root.join("target");
        std::fs::create_dir_all(&target).expect("mkdir");
        std::fs::set_permissions(&target, std::fs::Permissions::from_mode(0o755)).expect("chmod");
        let link = root.join("link");
        std::os::unix::fs::symlink(&target, &link).expect("symlink");
        create_private_dir(&link).expect("a symlinked directory is not an error");
        assert_eq!(
            mode(&target),
            0o755,
            "a symlink's target must not be changed"
        );

        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn acquire_then_second_holder_is_rejected() {
        let (store, dir) = tmp_store();
        let k = key("alpha");

        let g1 = store.acquire(&k).expect("first acquire");
        match store.acquire(&k) {
            Err(LeaseError::Held { key }) => assert_eq!(key.scope_key, "alpha"),
            other => panic!("expected Held, got {other:?}"),
        }
        let e1 = g1.epoch();
        drop(g1);
        let g2 = store.acquire(&k).expect("re-acquire after release");
        assert!(g2.epoch() > e1, "epoch is monotonic across acquisitions");
        drop(g2);
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn distinct_scopes_do_not_conflict() {
        let (store, dir) = tmp_store();
        let a = store.acquire(&key("a")).expect("a");
        let b = store.acquire(&key("b")).expect("b - different scope");
        assert_eq!(a.epoch(), 1);
        assert_eq!(b.epoch(), 1);
        drop((a, b));
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn distinct_modules_do_not_conflict_on_same_scope() {
        // The Oracle's must-fix: two modules sharing one lease root must NOT
        // collide on the same scope_key. The module_id is part of the key.
        let (store, dir) = tmp_store();
        let a = store
            .acquire(&LeaseKey::new("module-a", "sqlite", "same-scope"))
            .expect("module-a");
        let b = store
            .acquire(&LeaseKey::new("module-b", "sqlite", "same-scope"))
            .expect("module-b - different module, same scope, must not conflict");
        drop((a, b));
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn distinct_backends_do_not_conflict_on_same_scope() {
        let (store, dir) = tmp_store();
        let a = store
            .acquire(&LeaseKey::new("m", "sqlite", "s"))
            .expect("sqlite");
        let b = store
            .acquire(&LeaseKey::new("m", "postgres", "s"))
            .expect("postgres - different backend, same scope");
        drop((a, b));
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn shared_holders_coexist_but_block_exclusive() {
        let (store, dir) = tmp_store();
        let k = key("shared");

        let s1 = store.acquire_shared(&k).expect("first shared");
        let s2 = store
            .acquire_shared(&k)
            .expect("second shared holder coexists");

        // A shared holder blocks the exclusive writer — this is the property
        // the model-cache GC relies on (never delete under a live reader).
        match store.acquire(&k) {
            Err(LeaseError::Held { key }) => assert_eq!(key.scope_key, "shared"),
            other => panic!("exclusive must be Held while shared holders live, got {other:?}"),
        }

        drop(s1);
        // Still one shared holder alive: exclusive must STILL be blocked.
        match store.acquire(&k) {
            Err(LeaseError::Held { .. }) => {}
            other => {
                panic!("exclusive must stay Held until the last shared holder drops, got {other:?}")
            }
        }

        drop(s2);
        let g = store
            .acquire(&k)
            .expect("exclusive after all shared holders released");
        drop(g);
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn exclusive_holder_blocks_shared() {
        let (store, dir) = tmp_store();
        let k = key("excl-blocks-shared");

        let g = store.acquire(&k).expect("exclusive");
        match store.acquire_shared(&k) {
            Err(LeaseError::Held { key }) => assert_eq!(key.scope_key, "excl-blocks-shared"),
            other => panic!("shared must be Held while exclusive holder lives, got {other:?}"),
        }
        drop(g);
        let s = store
            .acquire_shared(&k)
            .expect("shared after exclusive released");
        drop(s);
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn shared_acquisition_does_not_bump_the_write_epoch() {
        let (store, dir) = tmp_store();
        let k = key("epoch-neutral");

        let g = store.acquire(&k).expect("writer");
        assert_eq!(g.epoch(), 1);
        drop(g);

        // Shared holders observe the persisted epoch but never advance it.
        let s1 = store.acquire_shared(&k).expect("shared");
        assert_eq!(s1.epoch(), 1, "shared handle reports last writer epoch");
        drop(s1);
        let s2 = store.acquire_shared(&k).expect("shared again");
        assert_eq!(s2.epoch(), 1);
        drop(s2);

        let g2 = store.acquire(&k).expect("writer again");
        assert_eq!(
            g2.epoch(),
            2,
            "writer epoch continues from 1: shared holders did not consume epochs"
        );
        drop(g2);
        let _ = std::fs::remove_dir_all(dir);
    }

    // Unix-only: the child uses fcntl.flock. On Windows the same-process tests
    // above still exercise the real LockFileEx shared/exclusive semantics via
    // fs2, because LockFileEx locks are per-handle (two handles in one process
    // behave like two processes for contention purposes).
    #[cfg(unix)]
    #[test]
    fn shared_lease_across_processes_blocks_exclusive() {
        // Cross-PROCESS proof (not just same-process flock semantics): a child
        // process holds a shared lease while the parent tries exclusive.
        // flock/LockFileEx semantics are per-open-file-description, so the
        // same-process tests above could in principle pass with per-fd
        // semantics that differ across processes; this pins the real contract.
        let (store, dir) = tmp_store();
        let k = key("xproc");

        // Learn the exact lock file path by acquiring+releasing once (also
        // seeds the epoch file).
        let g = store.acquire(&k).expect("seed");
        drop(g);
        let lock_path = {
            let mut entries = std::fs::read_dir(&dir).expect("lease dir");
            let entry = entries.next().expect("one lease file").expect("dir entry");
            entry.path()
        };

        // Child: hold a SHARED flock on the lease file for 2 seconds.
        // `flock(1)` from util-linux is absent on macOS, so use a tiny python
        // child — python is available on every dev/CI platform we run.
        let mut child = std::process::Command::new("python3")
            .arg("-c")
            .arg(format!(
                "import fcntl,time\nf=open({lock_path:?},'r+')\nfcntl.flock(f,fcntl.LOCK_SH)\nprint('held',flush=True)\ntime.sleep(2)",
            ))
            .stdout(std::process::Stdio::piped())
            .spawn()
            .expect("spawn shared-holder child");

        // Wait until the child confirms it holds the shared lock.
        {
            use std::io::BufRead;
            let stdout = child.stdout.take().expect("child stdout");
            let mut line = String::new();
            std::io::BufReader::new(stdout)
                .read_line(&mut line)
                .expect("child readiness line");
            assert_eq!(line.trim(), "held");
        }

        // Parent: exclusive must be Held while the child's shared lock lives.
        match store.acquire(&k) {
            Err(LeaseError::Held { .. }) => {}
            other => {
                panic!("exclusive must be Held under cross-process shared lock, got {other:?}")
            }
        }
        // Shared, however, coexists with the child's shared lock.
        let s = store
            .acquire_shared(&k)
            .expect("shared coexists with cross-process shared holder");
        drop(s);

        child.wait().expect("child exit");
        let g = store.acquire(&k).expect("exclusive after child released");
        drop(g);
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn epoch_persists_across_store_instances() {
        let (store, dir) = tmp_store();
        let k = key("persist");
        let g = store.acquire(&k).expect("acquire");
        assert_eq!(g.epoch(), 1);
        drop(g);
        // A fresh store over the same dir continues the epoch (survives restart).
        let store2 = FileLeaseStore::new(&dir);
        let g2 = store2.acquire(&k).expect("re-acquire");
        assert_eq!(g2.epoch(), 2);
        drop(g2);
        let _ = std::fs::remove_dir_all(dir);
    }

    /// Raising goes through a shared `Arc` handle, is visible at once through
    /// `epoch()`, never lowers the epoch, and is persisted: the next exclusive
    /// acquisition continues above the raised value. The persisted value is
    /// observed by re-acquiring, not by reading the file, which Windows refuses
    /// while another handle holds the lock.
    #[test]
    fn raise_epoch_persists_and_never_lowers() {
        let (store, dir) = tmp_store();
        let k = key("raise");

        let lease: std::sync::Arc<dyn LeaseHandle> =
            std::sync::Arc::from(store.acquire(&k).expect("acquire"));
        assert_eq!(lease.epoch(), 1);
        let shared = std::sync::Arc::clone(&lease);
        assert_eq!(shared.raise_epoch(7).expect("raise"), 7);
        assert_eq!(lease.epoch(), 7, "every clone of the handle sees the raise");
        assert_eq!(
            lease
                .raise_epoch(3)
                .expect("a lower target is not an error"),
            7,
            "an epoch is never lowered"
        );
        assert_eq!(lease.epoch(), 7);
        drop((lease, shared));

        let next = FileLeaseStore::new(&dir).acquire(&k).expect("re-acquire");
        assert_eq!(next.epoch(), 8, "the raised epoch was persisted");
        drop(next);
        let _ = std::fs::remove_dir_all(dir);
    }

    /// A shared holder is not a writer, so it may not move the writer epoch.
    #[test]
    fn a_shared_lease_cannot_raise_the_epoch() {
        let (store, dir) = tmp_store();
        let k = key("raise-shared");

        let shared = store.acquire_shared(&k).expect("shared");
        match shared.raise_epoch(9) {
            Err(LeaseError::Io(error)) => {
                assert_eq!(error.kind(), io::ErrorKind::PermissionDenied)
            }
            other => panic!("a shared raise must be refused, got {other:?}"),
        }
        assert_eq!(shared.epoch(), 0);
        drop(shared);

        let writer = store.acquire(&k).expect("writer");
        assert_eq!(writer.epoch(), 1, "the refused raise persisted nothing");
        drop(writer);
        let _ = std::fs::remove_dir_all(dir);
    }

    /// An implementor that does not override `raise_epoch` reports it as
    /// unsupported instead of claiming a raise it did not make.
    #[test]
    fn the_default_raise_epoch_is_unsupported() {
        #[derive(Debug)]
        struct FixedLease(LeaseKey);
        impl LeaseHandle for FixedLease {
            fn epoch(&self) -> u64 {
                1
            }
            fn key(&self) -> &LeaseKey {
                &self.0
            }
        }

        match FixedLease(key("fixed")).raise_epoch(5) {
            Err(LeaseError::Io(error)) => assert_eq!(error.kind(), io::ErrorKind::Unsupported),
            other => panic!("expected an unsupported error, got {other:?}"),
        }
    }

    #[cfg(unix)]
    #[test]
    fn durable_replace_replaces_contents_and_removes_temp_file() {
        let (_, dir) = tmp_store();
        std::fs::create_dir_all(&dir).expect("create directory");
        let temp = dir.join("state.tmp");
        let dest = dir.join("state");
        std::fs::write(&temp, b"new contents").expect("write temporary file");
        std::fs::write(&dest, b"old contents").expect("write existing file");

        durable_replace(&temp, &dest).expect("replace durably");

        assert_eq!(
            std::fs::read(&dest).expect("read destination"),
            b"new contents"
        );
        assert!(!temp.exists(), "the temporary name must be removed");
        let _ = std::fs::remove_dir_all(dir);
    }

    #[cfg(unix)]
    #[test]
    fn durable_replace_refuses_different_directories() {
        let (_, dir) = tmp_store();
        let temp_dir = dir.join("temp");
        let dest_dir = dir.join("destination");
        std::fs::create_dir_all(&temp_dir).expect("create temporary directory");
        std::fs::create_dir_all(&dest_dir).expect("create destination directory");
        let temp = temp_dir.join("state.tmp");
        let dest = dest_dir.join("state");
        std::fs::write(&temp, b"new contents").expect("write temporary file");
        std::fs::write(&dest, b"old contents").expect("write existing file");

        let error = durable_replace(&temp, &dest).expect_err("different directories must fail");

        assert_eq!(error.kind(), io::ErrorKind::InvalidInput);
        assert_eq!(
            std::fs::read(&temp).expect("read temporary file"),
            b"new contents"
        );
        assert_eq!(
            std::fs::read(&dest).expect("read destination"),
            b"old contents"
        );
        let _ = std::fs::remove_dir_all(dir);
    }

    #[cfg(unix)]
    #[test]
    fn sync_dir_succeeds_on_a_real_directory() {
        let (_, dir) = tmp_store();
        std::fs::create_dir_all(&dir).expect("create directory");

        sync_dir(&dir).expect("sync directory");

        let _ = std::fs::remove_dir_all(dir);
    }
}
