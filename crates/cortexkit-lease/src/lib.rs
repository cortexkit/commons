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
    io::{Read, Seek, SeekFrom, Write},
    path::PathBuf,
};

use fs2::FileExt;

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
    epoch: u64,
    /// Holds the OS advisory lock until dropped.
    file: File,
    key: LeaseKey,
}

impl LeaseHandle for FileLeaseHandle {
    fn epoch(&self) -> u64 {
        self.epoch
    }

    fn key(&self) -> &LeaseKey {
        &self.key
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
            epoch,
            file,
            key: key.clone(),
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
            epoch,
            file,
            key: key.clone(),
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

    fn tmp_store() -> (FileLeaseStore, PathBuf) {
        let dir = std::env::temp_dir().join(format!(
            "cortexkit-lease-{}-{}",
            std::process::id(),
            fnv1a_hex(&format!("{:?}", std::time::Instant::now()))
        ));
        (FileLeaseStore::new(&dir), dir)
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
            other => panic!("exclusive must stay Held until the last shared holder drops, got {other:?}"),
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
            let entry = entries
                .next()
                .expect("one lease file")
                .expect("dir entry");
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
            other => panic!("exclusive must be Held under cross-process shared lock, got {other:?}"),
        }
        // Shared, however, coexists with the child's shared lock.
        let s = store
            .acquire_shared(&k)
            .expect("shared coexists with cross-process shared holder");
        drop(s);

        child.wait().expect("child exit");
        let g = store
            .acquire(&k)
            .expect("exclusive after child released");
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
}
