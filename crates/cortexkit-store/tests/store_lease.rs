#![cfg(feature = "sqlite")]

//! The single-writer lease as seen from outside SQLite: a refused opener must
//! not touch the live database, and the maintenance lease (`StoreLease`) must
//! guard the store exactly like a live writer does.

use cortexkit_store::{
    acquire_store_lease, open_sqlite, Isolation, Migration, SqliteOpenOptions, SqliteStore,
    StorageBackend, StorageDescriptor, StoreError,
};
use std::{
    collections::BTreeMap,
    path::{Path, PathBuf},
    sync::atomic::{AtomicU64, Ordering},
    time::{SystemTime, UNIX_EPOCH},
};

struct TestDb {
    root: PathBuf,
    descriptor: StorageDescriptor,
}

impl TestDb {
    fn new() -> Self {
        static SEQ: AtomicU64 = AtomicU64::new(0);
        let root = std::env::temp_dir().join(format!(
            "cortexkit-store-lease-{}-{}-{}",
            std::process::id(),
            SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap()
                .as_nanos(),
            SEQ.fetch_add(1, Ordering::Relaxed),
        ));
        let descriptor = StorageDescriptor {
            module_id: "lease-tests".into(),
            storage_namespace: "main".into(),
            isolation: Isolation::Module,
            backend: StorageBackend::Sqlite {
                path: root.join("store.db").to_string_lossy().into_owned(),
            },
        };
        Self { root, descriptor }
    }

    fn path(&self) -> PathBuf {
        match &self.descriptor.backend {
            StorageBackend::Sqlite { path } => PathBuf::from(path),
            _ => unreachable!(),
        }
    }

    fn sidecar(&self, suffix: &str) -> PathBuf {
        PathBuf::from(format!("{}{suffix}", self.path().display()))
    }
}

impl Drop for TestDb {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.root);
    }
}

/// One file's observable state: length, modification time and full contents.
type FileState = (u64, SystemTime, Vec<u8>);

/// What a refused opener must not change in a store's directory: the
/// directory's own modification time (which moves when a file is created or
/// removed) and the length, mtime and bytes of every file in it, lease files
/// included, so a bumped epoch shows up as changed contents. Permissions and
/// change times are left out: a refused lease acquire may reapply owner-only
/// permissions, which changes no contents.
#[derive(Debug, PartialEq)]
struct DirState {
    dir_mtime: SystemTime,
    files: BTreeMap<PathBuf, FileState>,
}

fn dir_state(dir: &Path) -> DirState {
    let files = std::fs::read_dir(dir)
        .unwrap()
        .map(|entry| {
            let path = entry.unwrap().path();
            let metadata = std::fs::metadata(&path).unwrap();
            let state = (
                metadata.len(),
                metadata.modified().unwrap(),
                std::fs::read(&path).unwrap(),
            );
            (path, state)
        })
        .collect();
    DirState {
        dir_mtime: std::fs::metadata(dir).unwrap().modified().unwrap(),
        files,
    }
}

fn lease_files(dir: &Path) -> Vec<PathBuf> {
    let mut leases: Vec<_> = std::fs::read_dir(dir)
        .unwrap()
        .map(|entry| entry.unwrap().path())
        .filter(|path| path.extension().is_some_and(|ext| ext == "lease"))
        .collect();
    leases.sort();
    leases
}

fn expect_lease_error<T>(result: Result<T, StoreError>, what: &str) {
    match result {
        Err(StoreError::Lease(_)) => {}
        Err(error) => panic!("{what}: expected a lease refusal, got {error:?}"),
        Ok(_) => panic!("{what}: expected a lease refusal, but it was admitted"),
    }
}

fn value(store: &SqliteStore) -> i64 {
    store
        .with_conn(|conn| conn.query_row("SELECT value FROM points", [], |row| row.get(0)))
        .unwrap()
}

/// A live writer whose `-wal` holds a committed frame no reader has seen yet,
/// with `-wal` and `-shm` both on disk.
///
/// Leaving the WAL unread matters: a connection that so much as reads this
/// database records a read mark for that frame in the `-shm`, so a second
/// opener that opened SQLite before learning the store was held would change
/// the `-shm` bytes even though it only read. The `-shm` is memory-mapped, so
/// that change leaves its mtime alone: comparing bytes is what catches it.
fn writer_with_unread_wal(db: &TestDb) -> SqliteStore {
    let writer = open_sqlite(&db.descriptor).unwrap();
    writer
        .with_conn(|conn| {
            conn.execute_batch(
                "CREATE TABLE points (value INTEGER); INSERT INTO points VALUES (1);
                 PRAGMA wal_checkpoint(TRUNCATE);
                 UPDATE points SET value = 2;",
            )
        })
        .unwrap();
    for suffix in ["-wal", "-shm"] {
        assert!(
            db.sidecar(suffix).is_file(),
            "the live store has no {suffix}, so this test could not see it change"
        );
    }
    writer
}

/// A second opener for the same owner is refused by the lease before SQLite
/// opens the database, so the database, its `-wal` and `-shm`, the lease file
/// and the directory are byte-for-byte and mtime-for-mtime unchanged.
#[test]
fn a_same_owner_opener_refused_by_the_lease_leaves_the_store_files_unchanged() {
    let db = TestDb::new();
    let writer = writer_with_unread_wal(&db);
    let before = dir_state(&db.root);

    expect_lease_error(open_sqlite(&db.descriptor), "second same-owner open");

    assert_eq!(
        dir_state(&db.root),
        before,
        "a refused opener changed the live store's files"
    );
    writer
        .with_conn(|conn| conn.execute("UPDATE points SET value = 3", []))
        .unwrap();
    assert_eq!(value(&writer), 3, "the live writer keeps writing");
}

/// While a live store holds the lease, the maintenance lease is refused, and the
/// refusal creates no file and leaves the persisted epoch (the lease file's
/// contents) unchanged; it may only reapply owner-only permissions. The next
/// successful acquire proves the epoch was not bumped, by coming out exactly
/// one above the live writer's epoch.
#[test]
fn acquire_store_lease_is_refused_while_a_store_is_open_and_changes_nothing() {
    let db = TestDb::new();
    let writer = writer_with_unread_wal(&db);
    let writer_epoch = writer.epoch();
    let before = dir_state(&db.root);

    expect_lease_error(acquire_store_lease(&db.descriptor), "maintenance lease");

    assert_eq!(
        dir_state(&db.root),
        before,
        "a refused maintenance lease changed the store's directory"
    );
    drop(writer);
    let lease = acquire_store_lease(&db.descriptor).expect("free once the writer is gone");
    assert_eq!(
        lease.epoch(),
        writer_epoch + 1,
        "the refused attempt bumped the persisted epoch"
    );
}

/// The maintenance lease takes the store's lease and nothing else: on a fresh
/// store it creates the directory and one lease file, and no database file.
/// While it is held, the module cannot open its store; dropping it lets the
/// module open at a later epoch.
#[test]
fn open_sqlite_is_refused_while_a_store_lease_is_held_and_dropping_it_releases() {
    let db = TestDb::new();
    let lease = acquire_store_lease(&db.descriptor).expect("acquire on a fresh store");
    let files: Vec<_> = dir_state(&db.root).files.into_keys().collect();
    assert_eq!(
        files,
        lease_files(&db.root),
        "the maintenance lease created something besides its lease file"
    );
    assert_eq!(files.len(), 1);

    expect_lease_error(
        open_sqlite(&db.descriptor),
        "open under a maintenance lease",
    );
    assert!(
        !db.path().exists(),
        "a refused open created the database while the store was held"
    );

    let lease_epoch = lease.epoch();
    drop(lease);
    let store = open_sqlite(&db.descriptor).expect("open once the maintenance lease is dropped");
    assert!(store.epoch() > lease_epoch);
}

/// Taking the maintenance lease on an existing, closed store leaves its database
/// exactly as it was: SQLite is not opened, so no owner row is written and no
/// `-wal` or `-shm` is created.
#[test]
fn acquire_store_lease_does_not_open_an_existing_database() {
    let db = TestDb::new();
    drop(open_sqlite(&db.descriptor).unwrap());
    let before = std::fs::read(db.path()).unwrap();
    let modified = std::fs::metadata(db.path()).unwrap().modified().unwrap();

    let lease = acquire_store_lease(&db.descriptor).unwrap();

    assert_eq!(std::fs::read(db.path()).unwrap(), before);
    assert_eq!(
        std::fs::metadata(db.path()).unwrap().modified().unwrap(),
        modified
    );
    for suffix in ["-wal", "-shm"] {
        assert!(
            !db.sidecar(suffix).exists(),
            "taking the maintenance lease opened the database and created {suffix}"
        );
    }
    drop(lease);
}

#[test]
fn acquire_store_lease_refuses_a_non_sqlite_descriptor() {
    let db = TestDb::new();
    let mut descriptor = db.descriptor.clone();
    descriptor.backend = StorageBackend::Postgres {
        dsn: "postgres://x".into(),
        database: "y".into(),
    };
    match acquire_store_lease(&descriptor) {
        Err(StoreError::UnsupportedBackend(backend)) => assert_eq!(backend, "postgres"),
        other => panic!("expected UnsupportedBackend, got {other:?}"),
    }
}

const POINTS_V2: &[Migration] = &[
    Migration {
        version: 1,
        statements: "CREATE TABLE points (value INTEGER); INSERT INTO points VALUES (1);",
    },
    Migration {
        version: 2,
        statements: "UPDATE points SET value = 2;",
    },
];

/// A restore tool's flow: hold the maintenance lease, open a copy of a backup
/// as a scratch store, migrate and verify it, then move it into place. The
/// scratch store shares the lease (same epoch, no second lease file), keeps the
/// lease held while it is open even after the guard is dropped, and never
/// touches the live database.
#[test]
fn open_scratch_prepares_a_replacement_under_the_held_lease() {
    let db = TestDb::new();
    {
        let live = open_sqlite(&db.descriptor).unwrap();
        live.migrate("points", &POINTS_V2[..1]).unwrap();
    }
    let scratch_path = db.root.join("restore.db");
    std::fs::copy(db.path(), &scratch_path).unwrap();
    let live_before = std::fs::read(db.path()).unwrap();

    let lease = acquire_store_lease(&db.descriptor).unwrap();
    let leases = lease_files(&db.root);
    let scratch = lease
        .open_scratch(&scratch_path, SqliteOpenOptions::default())
        .expect("open the scratch copy");
    assert_eq!(scratch.epoch(), lease.epoch());
    assert_eq!(
        lease_files(&db.root),
        leases,
        "the scratch open took a lease"
    );
    let outcome = scratch.migrate("points", POINTS_V2).unwrap();
    assert_eq!(outcome.applied, 1);
    assert_eq!(value(&scratch), 2);
    let journal_mode: String = scratch
        .with_conn(|conn| conn.query_row("PRAGMA journal_mode", [], |row| row.get(0)))
        .unwrap();
    assert_eq!(journal_mode, "wal");
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let mode = std::fs::metadata(&scratch_path)
            .unwrap()
            .permissions()
            .mode()
            & 0o777;
        assert_eq!(mode, 0o600, "the scratch database is not owner-only");
    }

    drop(lease);
    expect_lease_error(
        open_sqlite(&db.descriptor),
        "open while a scratch store is still alive",
    );
    assert_eq!(
        std::fs::read(db.path()).unwrap(),
        live_before,
        "preparing the scratch copy changed the live database"
    );

    // Dropping the scratch store closes its connection, which checkpoints its WAL
    // into the main file, so the file can be moved into place on its own; the
    // value read back after the move shows nothing was left behind in a WAL.
    drop(scratch);
    std::fs::rename(&scratch_path, db.path()).unwrap();
    let live = open_sqlite(&db.descriptor).expect("open the placed replacement");
    assert_eq!(value(&live), 2);
}

/// Links to the live database that a scratch open must also refuse: a
/// symlink and a hard link. Only Unix detects hard links (by inode), so
/// elsewhere there is nothing to add.
#[cfg(unix)]
fn links_to_live(db: &TestDb) -> Vec<PathBuf> {
    let symlink = db.root.join("alias.db");
    std::os::unix::fs::symlink(db.path(), &symlink).unwrap();
    let hard_link = db.root.join("hard.db");
    std::fs::hard_link(db.path(), &hard_link).unwrap();
    vec![symlink, hard_link]
}

#[cfg(not(unix))]
fn links_to_live(_db: &TestDb) -> Vec<PathBuf> {
    Vec::new()
}

#[test]
fn open_scratch_refuses_the_live_database_and_non_file_paths() {
    let db = TestDb::new();
    drop(open_sqlite(&db.descriptor).unwrap());
    let lease = acquire_store_lease(&db.descriptor).unwrap();
    let options = SqliteOpenOptions::default();
    let refused: Vec<PathBuf> = [
        db.path(),
        PathBuf::from(":memory:"),
        PathBuf::from(format!("file:{}", db.path().display())),
    ]
    .into_iter()
    .chain(links_to_live(&db))
    .collect();
    let live_before = std::fs::read(db.path()).unwrap();
    for path in refused {
        match lease.open_scratch(&path, options) {
            Err(StoreError::Backend(_)) => {}
            Err(error) => panic!(
                "{}: expected a Backend refusal, got {error:?}",
                path.display()
            ),
            Ok(_) => panic!(
                "{}: a scratch open of the live store was admitted",
                path.display()
            ),
        }
    }
    assert_eq!(std::fs::read(db.path()).unwrap(), live_before);
    for suffix in ["-wal", "-shm"] {
        assert!(!db.sidecar(suffix).exists(), "the live database was opened");
    }
}

/// A scratch copy recorded for another namespace is refused: placed as this
/// store, it would make the module refuse to open its own store.
#[test]
fn open_scratch_refuses_a_copy_owned_by_another_namespace() {
    let db = TestDb::new();
    let mut other = db.descriptor.clone();
    other.storage_namespace = "other".into();
    let scratch_path = db.root.join("foreign.db");
    other.backend = StorageBackend::Sqlite {
        path: scratch_path.to_string_lossy().into_owned(),
    };
    drop(open_sqlite(&other).unwrap());

    let lease = acquire_store_lease(&db.descriptor).unwrap();
    match lease.open_scratch(&scratch_path, SqliteOpenOptions::default()) {
        Err(StoreError::OwnershipMismatch {
            recorded_storage_namespace,
            requested_storage_namespace,
            ..
        }) => {
            assert_eq!(recorded_storage_namespace, "other");
            assert_eq!(requested_storage_namespace, "main");
        }
        other => panic!("expected OwnershipMismatch, got {:?}", other.map(|_| ())),
    }
}
