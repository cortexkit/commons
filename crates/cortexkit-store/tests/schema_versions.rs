#![cfg(feature = "sqlite")]

use cortexkit_store::{
    acquire_store_lease, open_sqlite, read_schema_versions, Isolation, Migration, SqliteStore,
    StorageBackend, StorageDescriptor, StoreError,
};
use rusqlite::Connection;
use std::{
    collections::BTreeMap,
    fs,
    io::{BufRead, BufReader, Write},
    path::{Path, PathBuf},
    process::{Command, Stdio},
    sync::atomic::{AtomicU64, Ordering},
    time::{Duration, SystemTime, UNIX_EPOCH},
};

struct TestDb {
    root: PathBuf,
    descriptor: StorageDescriptor,
}

impl TestDb {
    fn new() -> Self {
        static SEQ: AtomicU64 = AtomicU64::new(0);
        let root = std::env::temp_dir().join(format!(
            "cortexkit-schema-versions-{}-{}-{}",
            std::process::id(),
            SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap()
                .as_nanos(),
            SEQ.fetch_add(1, Ordering::Relaxed),
        ));
        fs::create_dir(&root).unwrap();
        let name = if cfg!(windows) {
            "store #%.db"
        } else {
            "store ?#%.db"
        };
        Self {
            descriptor: descriptor(&root.join(name)),
            root,
        }
    }

    fn path(&self) -> &Path {
        match &self.descriptor.backend {
            StorageBackend::Sqlite { path } => Path::new(path),
            _ => unreachable!(),
        }
    }

    fn sidecar(&self, suffix: &str) -> PathBuf {
        PathBuf::from(format!("{}{suffix}", self.path().display()))
    }

    fn migrated(&self) -> SqliteStore {
        let store = open_sqlite(&self.descriptor).unwrap();
        migrate(&store);
        store
    }
}

impl Drop for TestDb {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.root);
    }
}

fn descriptor(path: &Path) -> StorageDescriptor {
    StorageDescriptor {
        module_id: "schema-version-tests".into(),
        storage_namespace: "main".into(),
        isolation: Isolation::Module,
        backend: StorageBackend::Sqlite {
            path: path.to_str().unwrap().into(),
        },
    }
}

fn migrate(store: &SqliteStore) {
    store
        .migrate(
            "alpha",
            &[
                Migration {
                    version: 1,
                    statements: "CREATE TABLE items (id INTEGER);",
                },
                Migration {
                    version: 3,
                    statements: "INSERT INTO items VALUES (3);",
                },
            ],
        )
        .unwrap();
    store
        .migrate(
            "beta",
            &[Migration {
                version: 2,
                statements: "SELECT 1;",
            }],
        )
        .unwrap();
}

fn expected() -> BTreeMap<String, u64> {
    BTreeMap::from([("alpha".into(), 3), ("beta".into(), 2)])
}

#[derive(Debug, PartialEq, Eq)]
struct FileState {
    /// `None` when another process holds a byte-range lock on the file, which
    /// only Windows enforces against reads; size and time are compared then.
    bytes: Option<Vec<u8>>,
    len: u64,
    modified: Option<SystemTime>,
}

/// Windows `ERROR_LOCK_VIOLATION`: a region of the file is locked by another
/// process, here the store holder's lease or database lock.
const LOCK_VIOLATION: i32 = 33;

fn read_unless_locked(path: &Path) -> Option<Vec<u8>> {
    match fs::read(path) {
        Ok(bytes) => Some(bytes),
        Err(error) if cfg!(windows) && error.raw_os_error() == Some(LOCK_VIOLATION) => None,
        Err(error) => panic!("reading {}: {error}", path.display()),
    }
}

// File names include directories. The shared-memory file may change during a
// SQLite read, so compare only its presence, not its contents.
fn folder_state(root: &Path) -> BTreeMap<PathBuf, Option<FileState>> {
    fn visit(root: &Path, dir: &Path, state: &mut BTreeMap<PathBuf, Option<FileState>>) {
        for entry in fs::read_dir(dir).unwrap() {
            let path = entry.unwrap().path();
            let metadata = fs::metadata(&path).unwrap();
            let contents = if metadata.is_file() && !path.to_string_lossy().ends_with("-shm") {
                Some(FileState {
                    bytes: read_unless_locked(&path),
                    len: metadata.len(),
                    modified: metadata.modified().ok(),
                })
            } else {
                None
            };
            state.insert(path.strip_prefix(root).unwrap().to_path_buf(), contents);
            if metadata.is_dir() {
                visit(root, &path, state);
            }
        }
    }
    let mut state = BTreeMap::new();
    visit(root, root, &mut state);
    state
}

fn unchanged_read(db: &TestDb) -> Result<Option<BTreeMap<String, u64>>, StoreError> {
    let before = folder_state(&db.root);
    let result = read_schema_versions(&db.descriptor);
    let after = folder_state(&db.root);
    assert_eq!(
        after.keys().collect::<Vec<_>>(),
        before.keys().collect::<Vec<_>>()
    );
    assert_eq!(after, before);
    result
}

fn assert_unavailable(result: Result<Option<BTreeMap<String, u64>>, StoreError>) {
    assert!(
        matches!(result, Err(StoreError::SchemaVersionsUnavailable { .. })),
        "{result:?}"
    );
}

#[test]
fn schema_versions_reads_namespace_maxima_from_migrations_and_wal() {
    let db = TestDb::new();
    let store = db.migrated();
    assert!(db.sidecar("-wal").is_file());
    assert!(db.sidecar("-shm").is_file());
    assert_eq!(unchanged_read(&db).unwrap(), Some(expected()));
    store
        .with_read(|conn| {
            let fence_exists: bool = conn.query_row(
                "SELECT EXISTS(SELECT 1 FROM sqlite_schema WHERE name = 'cortexkit_fence')",
                [],
                |row| row.get(0),
            )?;
            assert!(!fence_exists);
            Ok(())
        })
        .unwrap();
}

#[test]
fn schema_versions_missing_file_and_parent_create_nothing() {
    let mut db = TestDb::new();
    assert_eq!(unchanged_read(&db).unwrap(), None);
    db.descriptor = descriptor(&db.root.join("missing-parent/store.db"));
    assert_eq!(unchanged_read(&db).unwrap(), None);
}

#[test]
fn schema_versions_without_table_returns_empty_without_creating_files() {
    let db = TestDb::new();
    drop(Connection::open(db.path()).unwrap());
    assert_eq!(unchanged_read(&db).unwrap(), Some(BTreeMap::new()));
    let conn = Connection::open(db.path()).unwrap();
    conn.execute_batch("CREATE TABLE unrelated (id INTEGER);")
        .unwrap();
    drop(conn);
    assert_eq!(unchanged_read(&db).unwrap(), Some(BTreeMap::new()));
}

#[test]
fn schema_versions_reads_checkpointed_versions_with_an_empty_wal() {
    let db = TestDb::new();
    let store = db.migrated();
    store
        .with_conn(|conn| conn.execute_batch("PRAGMA wal_checkpoint(TRUNCATE);"))
        .unwrap();
    assert_eq!(fs::metadata(db.sidecar("-wal")).unwrap().len(), 0);
    assert_eq!(unchanged_read(&db).unwrap(), Some(expected()));
}

#[test]
fn schema_versions_cleanly_closed_wal_store_never_creates_shared_memory() {
    let db = TestDb::new();
    drop(db.migrated());
    assert!(!db.sidecar("-wal").exists());
    assert!(!db.sidecar("-shm").exists());
    let error = unchanged_read(&db).unwrap_err();
    assert!(matches!(
        error,
        StoreError::SchemaVersionsUnavailable { .. }
    ));
    assert!(error.to_string().contains("no shared memory"), "{error}");
}

#[test]
fn schema_versions_wal_without_shared_memory_is_an_error_without_creation() {
    let source = TestDb::new();
    let _writer = source.migrated();
    let copy = TestDb::new();
    fs::copy(source.path(), copy.path()).unwrap();
    fs::copy(source.sidecar("-wal"), copy.sidecar("-wal")).unwrap();
    assert!(!copy.sidecar("-shm").exists());
    assert_unavailable(unchanged_read(&copy));
}

#[test]
fn schema_versions_preserves_removals_lease_epoch_and_fence() {
    let db = TestDb::new();
    let store = db.migrated();
    let epoch = store.epoch();
    store
        .with_conn_fenced(|conn| conn.execute_batch("INSERT INTO items VALUES (7);"))
        .unwrap();
    fs::create_dir(db.root.join("removals")).unwrap();
    fs::write(
        db.root.join("removals/journal"),
        b"unreadable journal must not be replayed\n",
    )
    .unwrap();
    assert_eq!(unchanged_read(&db).unwrap(), Some(expected()));
    assert_eq!(store.epoch(), epoch);
    let fence: u64 = store
        .with_read(|conn| {
            conn.query_row(
                "SELECT epoch FROM cortexkit_fence WHERE id = 0",
                [],
                |row| row.get(0),
            )
        })
        .unwrap();
    assert_eq!(fence, epoch);
    drop(store);
    let lease = acquire_store_lease(&db.descriptor).unwrap();
    assert_eq!(lease.epoch(), epoch + 1);
}

#[test]
fn schema_versions_unreadable_and_invalid_data_are_errors_without_changes() {
    for sql in [
        "CREATE TABLE cortexkit_schema_version (wrong_column INTEGER);",
        "CREATE TABLE cortexkit_schema_version (namespace, version); \
         INSERT INTO cortexkit_schema_version VALUES ('alpha', -1);",
        "CREATE TABLE cortexkit_schema_version (namespace, version); \
         INSERT INTO cortexkit_schema_version VALUES ('alpha', 'not a version');",
        "CREATE TABLE cortexkit_schema_version (namespace, version); \
         INSERT INTO cortexkit_schema_version VALUES (NULL, 1);",
        "CREATE TABLE cortexkit_schema_version (namespace, version); \
         INSERT INTO cortexkit_schema_version VALUES ('alpha', NULL);",
    ] {
        let db = TestDb::new();
        Connection::open(db.path())
            .unwrap()
            .execute_batch(sql)
            .unwrap();
        assert_unavailable(unchanged_read(&db));
    }
    let db = TestDb::new();
    fs::write(db.path(), b"not a SQLite database").unwrap();
    assert_unavailable(unchanged_read(&db));
    fs::remove_file(db.path()).unwrap();
    fs::create_dir(db.path()).unwrap();
    assert_unavailable(unchanged_read(&db));
}

#[test]
fn schema_versions_unusable_shared_memory_is_an_error_without_changes() {
    let source = TestDb::new();
    let _writer = source.migrated();
    let copy = TestDb::new();
    fs::copy(source.path(), copy.path()).unwrap();
    fs::copy(source.sidecar("-wal"), copy.sidecar("-wal")).unwrap();
    fs::create_dir(copy.sidecar("-shm")).unwrap();
    assert_unavailable(unchanged_read(&copy));
    fs::remove_dir(copy.sidecar("-shm")).unwrap();
    fs::write(copy.sidecar("-shm"), b"truncated shared memory").unwrap();
    assert_unavailable(unchanged_read(&copy));
    fs::remove_file(copy.sidecar("-wal")).unwrap();
    assert_unavailable(unchanged_read(&copy));
}

#[test]
fn schema_versions_truncated_database_does_not_discard_existing_wal() {
    let source = TestDb::new();
    let _writer = source.migrated();
    let copy = TestDb::new();
    for contents in [&b""[..], &b"SQLite format 3\0"[..]] {
        fs::write(copy.path(), contents).unwrap();
        fs::copy(source.sidecar("-wal"), copy.sidecar("-wal")).unwrap();
        assert_unavailable(unchanged_read(&copy));
    }
}

#[test]
fn schema_versions_refuses_uris_memory_and_unsupported_backends_without_changes() {
    let mut db = TestDb::new();
    for path in [
        ":memory:",
        "file::memory:?cache=shared",
        "file:missing.db?immutable=1",
    ] {
        db.descriptor = descriptor(Path::new(path));
        assert_unavailable(unchanged_read(&db));
    }
    db.descriptor.backend = StorageBackend::Postgres {
        dsn: "unused".into(),
        database: "unused".into(),
    };
    assert!(matches!(
        unchanged_read(&db),
        Err(StoreError::UnsupportedBackend(_))
    ));
}

#[test]
fn schema_versions_lock_contention_is_an_error_without_changes() {
    let db = TestDb::new();
    let conn = Connection::open(db.path()).unwrap();
    conn.execute_batch(
        "CREATE TABLE unrelated (id); BEGIN EXCLUSIVE; INSERT INTO unrelated VALUES (1);",
    )
    .unwrap();
    assert_unavailable(unchanged_read(&db));
    conn.execute_batch("ROLLBACK;").unwrap();
}

#[test]
#[ignore = "subprocess helper for schema_versions_reads_while_another_process_holds_write_and_lease"]
fn schema_versions_writer_process() {
    let path = std::env::var("CORTEXKIT_SCHEMA_VERSIONS_WRITER_PATH").unwrap();
    let store = open_sqlite(&descriptor(Path::new(&path))).unwrap();
    migrate(&store);
    store
        .with_conn(|conn| {
            conn.busy_timeout(Duration::ZERO)?;
            conn.execute_batch(
                "BEGIN IMMEDIATE; \
             INSERT INTO cortexkit_schema_version VALUES ('alpha', 9, 0);",
            )?;
            println!("SCHEMA_WRITER_READY");
            std::io::stdout().flush().unwrap();
            let mut input = String::new();
            std::io::stdin().read_line(&mut input).unwrap();
            assert_eq!(input.trim(), "commit");
            conn.execute_batch("COMMIT;")?;
            let version: u64 = conn.query_row(
                "SELECT MAX(version) FROM cortexkit_schema_version WHERE namespace = 'alpha'",
                [],
                |row| row.get(0),
            )?;
            assert_eq!(version, 9);
            Ok(())
        })
        .unwrap();
    println!("SCHEMA_WRITER_COMMITTED");
}

#[test]
fn schema_versions_reads_while_another_process_holds_write_and_lease() {
    let db = TestDb::new();
    let mut child = Command::new(std::env::current_exe().unwrap())
        .args([
            "--exact",
            "schema_versions_writer_process",
            "--ignored",
            "--nocapture",
        ])
        .env("CORTEXKIT_SCHEMA_VERSIONS_WRITER_PATH", db.path())
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::inherit())
        .spawn()
        .unwrap();
    let mut output = BufReader::new(child.stdout.take().unwrap());
    loop {
        let mut line = String::new();
        assert_ne!(
            output.read_line(&mut line).unwrap(),
            0,
            "writer exited before ready"
        );
        if line.trim() == "SCHEMA_WRITER_READY" {
            break;
        }
    }
    let lease_held = matches!(
        acquire_store_lease(&db.descriptor),
        Err(StoreError::Lease(_))
    );
    let before = folder_state(&db.root);
    let result = read_schema_versions(&db.descriptor);
    let after = folder_state(&db.root);
    child.stdin.take().unwrap().write_all(b"commit\n").unwrap();
    let mut committed = false;
    for line in output.lines() {
        committed |= line.unwrap().trim() == "SCHEMA_WRITER_COMMITTED";
    }
    let status = child.wait().unwrap();
    assert!(lease_held);
    assert_eq!(result.unwrap(), Some(expected()));
    assert_eq!(after, before);
    assert!(status.success(), "writer failed to commit: {status}");
    assert!(committed);
    let conn = Connection::open(db.path()).unwrap();
    let version: u64 = conn
        .query_row(
            "SELECT MAX(version) FROM cortexkit_schema_version WHERE namespace = 'alpha'",
            [],
            |row| row.get(0),
        )
        .unwrap();
    assert_eq!(version, 9);
}
