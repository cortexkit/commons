#![cfg(feature = "sqlite")]

use cortexkit_store::{
    acquire_store_lease, open_sqlite, open_sqlite_with, open_sqlite_with_removals, Isolation,
    RemovalRecord, RemovalsHandler, SqliteOpenOptions, SqliteStore, StorageBackend,
    StorageDescriptor, StoreError,
};
use rusqlite::{Connection, OpenFlags};
use std::{
    path::{Path, PathBuf},
    process::{Command, Output},
    sync::{
        atomic::{AtomicU64, Ordering},
        Arc, Barrier,
    },
    thread,
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
            "cortexkit-store-owner-{}-{}-{}",
            std::process::id(),
            SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap()
                .as_nanos(),
            SEQ.fetch_add(1, Ordering::Relaxed),
        ));
        let descriptor = StorageDescriptor {
            module_id: "owner-tests".into(),
            storage_namespace: "main".into(),
            isolation: Isolation::Module,
            backend: StorageBackend::Sqlite {
                path: root.join("store.db").to_string_lossy().into_owned(),
            },
        };
        Self { root, descriptor }
    }

    fn other_namespace(&self) -> StorageDescriptor {
        let mut descriptor = self.descriptor.clone();
        descriptor.storage_namespace = "other".into();
        descriptor
    }

    fn path(&self) -> PathBuf {
        match &self.descriptor.backend {
            StorageBackend::Sqlite { path } => PathBuf::from(path),
            _ => unreachable!(),
        }
    }
}

impl Drop for TestDb {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.root);
    }
}

fn mismatch(
    result: Result<SqliteStore, StoreError>,
    recorded: &StorageDescriptor,
    requested: &StorageDescriptor,
) {
    let error = match result {
        Err(error) => error,
        Ok(_) => panic!("a different database owner was admitted"),
    };
    let message = error.to_string();
    match error {
        StoreError::OwnershipMismatch {
            recorded_module_id,
            recorded_storage_namespace,
            requested_module_id,
            requested_storage_namespace,
        } => {
            assert_eq!(recorded_module_id, recorded.module_id);
            assert_eq!(recorded_storage_namespace, recorded.storage_namespace);
            assert_eq!(requested_module_id, requested.module_id);
            assert_eq!(requested_storage_namespace, requested.storage_namespace);
        }
        other => panic!("expected OwnershipMismatch, got {other:?}"),
    }
    assert_eq!(
        message,
        format!(
            "database ownership mismatch: recorded owner ({:?}, {:?}), requested owner ({:?}, {:?})",
            recorded.module_id, recorded.storage_namespace,
            requested.module_id, requested.storage_namespace,
        ),
    );
}

fn owner(conn: &Connection) -> (String, String) {
    conn.query_row(
        "SELECT module_id, storage_namespace FROM cortexkit_owner WHERE id = 1",
        [],
        |row| Ok((row.get(0)?, row.get(1)?)),
    )
    .unwrap()
}

fn entries(path: &Path) -> Vec<PathBuf> {
    let mut paths: Vec<_> = std::fs::read_dir(path)
        .unwrap()
        .map(|entry| entry.unwrap().path())
        .collect();
    paths.sort();
    paths
}

const CHILD_DATABASE: &str = "CORTEXKIT_STORE_OWNER_TEST_CHILD_DATABASE";

fn run_child_open_if_requested(expect_lease_error: bool) -> bool {
    let Some(path) = std::env::var_os(CHILD_DATABASE) else {
        return false;
    };
    let descriptor = StorageDescriptor {
        module_id: "owner-tests".into(),
        storage_namespace: "main".into(),
        isolation: Isolation::Module,
        backend: StorageBackend::Sqlite {
            path: path.to_string_lossy().into_owned(),
        },
    };
    let result = open_sqlite_with(
        &descriptor,
        SqliteOpenOptions {
            busy_timeout: Duration::ZERO,
            ..SqliteOpenOptions::default()
        },
    );
    if expect_lease_error {
        match result {
            Err(StoreError::Lease(_)) => {}
            Err(error) => panic!("expected lease refusal, got {error:?}"),
            Ok(_) => panic!("a second matching writer was admitted"),
        }
    } else {
        let writer = result.unwrap();
        writer
            .with_conn(|conn| {
                assert_eq!(
                    owner(conn),
                    (descriptor.module_id, descriptor.storage_namespace)
                );
                Ok(())
            })
            .unwrap();
    }
    true
}

// A separate process exposes warnings without redirecting the parallel test
// runner's global stderr. The parent-held snapshot prevents a FULL checkpoint
// from completing until the child exits, so checkpoint warnings are observable
// for first claims and must be absent for already-recorded owners.
fn child_open(test_name: &str, db: &TestDb) -> Output {
    Command::new(std::env::current_exe().unwrap())
        .args(["--exact", test_name, "--nocapture"])
        .env(CHILD_DATABASE, db.path())
        .output()
        .unwrap()
}

fn assert_child_passed(output: &Output) {
    assert!(
        output.status.success(),
        "child test failed: stdout={} stderr={}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(
        stdout.contains("running 1 test") && stdout.contains("1 passed; 0 failed"),
        "the named child test must actually run: {stdout}"
    );
}

fn writer_with_points(db: &TestDb) -> SqliteStore {
    let writer = open_sqlite(&db.descriptor).unwrap();
    writer
        .with_conn(|conn| {
            conn.execute_batch(
                "CREATE TABLE points (value INTEGER); INSERT INTO points VALUES (5);
         PRAGMA wal_checkpoint(TRUNCATE)",
            )
        })
        .unwrap();
    writer
}

/// An opener under another namespace derives its own lease key, so it takes a
/// lease of its own before the owner check refuses it. It must release that
/// lease before returning, and leave only that lease file behind: no sidecar or
/// other file appears, and the first writer keeps writing.
#[test]
fn another_namespace_is_refused_by_the_owner_check_and_releases_its_lease() {
    let db = TestDb::new();
    let writer = open_sqlite(&db.descriptor).unwrap();
    writer
        .with_conn(|conn| {
            conn.execute_batch("CREATE TABLE points (value INTEGER); INSERT INTO points VALUES (1)")
        })
        .unwrap();
    let before = entries(&db.root);
    let requested = db.other_namespace();
    mismatch(open_sqlite(&requested), &db.descriptor, &requested);
    let added: Vec<_> = entries(&db.root)
        .into_iter()
        .filter(|path| !before.contains(path))
        .collect();
    assert!(
        added.len() == 1 && added[0].extension().is_some_and(|ext| ext == "lease"),
        "a refused open may leave only its own lease file behind: {added:?}"
    );
    // The refused opener's lease is free again, while the owner's is not.
    let released =
        acquire_store_lease(&requested).expect("the refused opener's lease was released");
    drop(released);
    assert!(matches!(
        acquire_store_lease(&db.descriptor),
        Err(StoreError::Lease(_))
    ));
    writer
        .with_conn(|conn| conn.execute("INSERT INTO points VALUES (2)", []))
        .unwrap();
    assert_eq!(
        writer
            .with_read(
                |tx| tx.query_row("SELECT sum(value) FROM points", [], |row| row
                    .get::<_, i64>(0))
            )
            .unwrap(),
        3
    );
    writer
        .with_conn(|conn| {
            assert_eq!(owner(conn), ("owner-tests".into(), "main".into()));
            Ok(())
        })
        .unwrap();
}

#[test]
fn another_module_is_refused_even_with_the_same_namespace() {
    let db = TestDb::new();
    let _writer = open_sqlite(&db.descriptor).unwrap();
    let mut requested = db.descriptor.clone();
    requested.module_id = "other-module".into();
    mismatch(open_sqlite(&requested), &db.descriptor, &requested);
}

#[test]
fn hard_link_under_another_namespace_is_refused() {
    let db = TestDb::new();
    let writer = open_sqlite(&db.descriptor).unwrap();
    let aliases = db.root.join("aliases");
    std::fs::create_dir(&aliases).unwrap();
    let alias = aliases.join("hard.db");
    std::fs::hard_link(db.path(), &alias).unwrap();
    let mut requested = db.other_namespace();
    requested.backend = StorageBackend::Sqlite {
        path: alias.to_string_lossy().into_owned(),
    };
    mismatch(open_sqlite(&requested), &db.descriptor, &requested);
    // The refused opener took (and released) its own lease beside the alias;
    // nothing else, in particular no `-wal` or `-shm` for the alias, remains.
    assert!(entries(&aliases)
        .iter()
        .all(|path| path == &alias || path.extension().is_some_and(|ext| ext == "lease")));
    drop(writer);
}

#[cfg(unix)]
#[test]
fn symlink_under_another_namespace_is_refused() {
    let db = TestDb::new();
    let _writer = open_sqlite(&db.descriptor).unwrap();
    let alias = db.root.join("symlink.db");
    std::os::unix::fs::symlink(db.path(), &alias).unwrap();
    let mut requested = db.other_namespace();
    requested.backend = StorageBackend::Sqlite {
        path: alias.to_string_lossy().into_owned(),
    };
    mismatch(open_sqlite(&requested), &db.descriptor, &requested);
}

#[test]
fn racing_first_writers_under_different_namespaces_have_exactly_one_owner() {
    for _ in 0..8 {
        let db = TestDb::new();
        let descriptors = [db.descriptor.clone(), db.other_namespace()];
        let start = Arc::new(Barrier::new(3));
        let handles: Vec<_> = descriptors
            .iter()
            .cloned()
            .map(|descriptor| {
                let start = start.clone();
                thread::spawn(move || {
                    start.wait();
                    open_sqlite(&descriptor)
                })
            })
            .collect();
        start.wait();
        let mut results: Vec<_> = handles
            .into_iter()
            .map(|handle| handle.join().unwrap())
            .collect();
        assert_eq!(
            results.iter().filter(|result| result.is_ok()).count(),
            1,
            "exactly one first writer must be admitted"
        );
        let winner = results.iter().position(|result| result.is_ok()).unwrap();
        let loser = 1 - winner;
        mismatch(
            results.remove(loser),
            &descriptors[winner],
            &descriptors[loser],
        );
        let writer = results.pop().unwrap().ok().unwrap();
        writer
            .with_conn(|conn| {
                assert_eq!(
                    owner(conn),
                    (
                        descriptors[winner].module_id.clone(),
                        descriptors[winner].storage_namespace.clone()
                    )
                );
                conn.execute_batch(
                    "CREATE TABLE winner (value INTEGER); INSERT INTO winner VALUES (1)",
                )
            })
            .unwrap();
    }
}

#[test]
fn legacy_wal_database_is_claimed_reopens_and_refuses_a_hard_link_alias() {
    let db = TestDb::new();
    std::fs::create_dir_all(&db.root).unwrap();
    let legacy = Connection::open(db.path()).unwrap();
    legacy.pragma_update(None, "journal_mode", "WAL").unwrap();
    legacy.pragma_update(None, "wal_autocheckpoint", 0).unwrap();
    legacy
        .execute_batch("CREATE TABLE points (value INTEGER); INSERT INTO points VALUES (7)")
        .unwrap();
    assert_eq!(
        legacy
            .query_row(
                "SELECT count(*) FROM sqlite_schema WHERE name = 'cortexkit_owner'",
                [],
                |row| row.get::<_, i64>(0)
            )
            .unwrap(),
        0
    );

    let writer = open_sqlite(&db.descriptor).unwrap();
    assert_eq!(
        owner(&legacy),
        (
            db.descriptor.module_id.clone(),
            db.descriptor.storage_namespace.clone()
        )
    );
    let alias = db.root.join("legacy-hard.db");
    std::fs::hard_link(db.path(), &alias).unwrap();
    let mut requested = db.other_namespace();
    requested.backend = StorageBackend::Sqlite {
        path: alias.to_string_lossy().into_owned(),
    };
    mismatch(open_sqlite(&requested), &db.descriptor, &requested);
    drop(writer);
    let reopened = open_sqlite(&db.descriptor).unwrap();
    assert_eq!(reopened.with_read(|tx| tx.query_row("SELECT value FROM points", [], |row| row.get::<_, i64>(0))).unwrap(), 7);
    drop(reopened);
    drop(legacy);
}

/// Run only this test in a child so changing its current directory cannot race
/// the other tests. The parent lists the directory even if the child fails.
fn in_temporary_working_directory(test: &str, expect_empty: bool) -> bool {
    const CHILD: &str = "CORTEXKIT_STORE_MEMORY_CHILD";
    if std::env::var(CHILD).as_deref() == Ok(test) {
        return true;
    }
    let db = TestDb::new();
    std::fs::create_dir(&db.root).unwrap();
    let output = Command::new(std::env::current_exe().unwrap())
        .args(["--exact", test, "--nocapture"])
        .env(CHILD, test)
        .current_dir(&db.root)
        .output()
        .unwrap();
    let remaining = entries(&db.root);
    eprintln!("{test}: temporary current directory contains {remaining:?}");
    assert_child_passed(&output);
    if expect_empty {
        assert!(remaining.is_empty(), "created files: {remaining:?}");
    }
    false
}

fn memory_descriptor(path: &str) -> StorageDescriptor {
    StorageDescriptor {
        module_id: "memory-tests".into(),
        storage_namespace: path.into(),
        isolation: Isolation::Module,
        backend: StorageBackend::Sqlite { path: path.into() },
    }
}

#[test]
fn in_memory_operations_create_no_files() {
    if !in_temporary_working_directory("in_memory_operations_create_no_files", true) {
        return;
    }
    struct NoRemovals;
    impl RemovalsHandler for NoRemovals {
        fn replay(&self, _: &rusqlite::Transaction<'_>, _: &RemovalRecord) -> rusqlite::Result<()> {
            panic!("memory stores cannot adopt a disk journal")
        }
    }
    let absolute_memory_uri = format!(
        "file:{}/absolute/missing/memory?mode=memory",
        std::env::current_dir()
            .unwrap()
            .display()
            .to_string()
            .replace('\\', "/")
    );
    for path in [
        "",
        ":memory:",
        "file::memory:",
        "file::memory:?cache=shared",
        "file:private?mode=memory",
        "file:shared?mode=memory&cache=shared",
        "file:missing/parent/private?mode=memory",
        "file:%3Amemory%3A",
        "file:encoded?%6dode=mem%6fry",
        "file::memory:%00ignored",
        "file:encoded?mode=memory%00ignored",
        "file:",
        &absolute_memory_uri,
    ] {
        let descriptor = memory_descriptor(path);
        assert!(matches!(
            open_sqlite_with_removals(&descriptor, SqliteOpenOptions::default(), &NoRemovals),
            Err(StoreError::Backend(_))
        ));
        let store = open_sqlite(&descriptor).unwrap();
        store
            .with_conn(|conn| {
                conn.execute_batch(
                    "CREATE TABLE items (id INTEGER PRIMARY KEY); INSERT INTO items VALUES (1)",
                )
            })
            .unwrap();
        store
            .with_conn_fenced(|tx| {
                tx.execute("INSERT INTO items VALUES (2)", [])?;
                Ok(())
            })
            .unwrap();
        let epoch: u64 = store
            .with_conn(|conn| {
                conn.query_row(
                    "SELECT epoch FROM cortexkit_fence WHERE id = 0",
                    [],
                    |row| row.get(0),
                )
            })
            .unwrap();
        assert_eq!(epoch, store.epoch());
        assert!(matches!(
            store.remove("items", ["1"], |_, _| panic!(
                "no journal: erase must not run"
            )),
            Err(StoreError::RemovalsHandlerRequired)
        ));
        store
            .with_conn_fenced(|tx| {
                assert_eq!(tx.execute("DELETE FROM items WHERE id = 1", [])?, 1);
                Ok(())
            })
            .unwrap();
        let count: u64 = store
            .with_conn(|conn| conn.query_row("SELECT count(*) FROM items", [], |row| row.get(0)))
            .unwrap();
        assert_eq!(count, 1);
        drop(store);
        let lease = acquire_store_lease(&descriptor).unwrap();
        assert!(lease.epoch() > 0);
        drop(lease);
    }
    assert!(entries(Path::new(".")).is_empty());
}

#[test]
fn private_in_memory_stores_with_the_same_descriptor_both_open() {
    if !in_temporary_working_directory(
        "private_in_memory_stores_with_the_same_descriptor_both_open",
        true,
    ) {
        return;
    }
    for path in [
        "",
        ":memory:",
        "file::memory:",
        "file:private?mode=memory",
        "file::memory:?cache=private",
        "file:private-last?mode=memory&cache=shared&cache=private",
    ] {
        let descriptor = memory_descriptor(path);
        let first = open_sqlite(&descriptor).unwrap();
        first
            .with_conn(|conn| conn.execute_batch("CREATE TABLE first_only (v INTEGER)"))
            .unwrap();
        let second = open_sqlite(&descriptor).expect("private databases must not contend");
        second
            .with_conn_fenced(|tx| tx.execute_batch("CREATE TABLE second_only (v INTEGER)"))
            .unwrap();
        first
            .with_conn(|conn| {
                assert_eq!(
                    conn.query_row(
                        "SELECT count(*) FROM sqlite_schema WHERE name = 'second_only'",
                        [],
                        |row| row.get::<_, i64>(0)
                    )?,
                    0
                );
                Ok(())
            })
            .unwrap();
    }
}

#[test]
fn shared_in_memory_stores_keep_an_exclusive_in_process_lease() {
    if !in_temporary_working_directory(
        "shared_in_memory_stores_keep_an_exclusive_in_process_lease",
        true,
    ) {
        return;
    }
    for path in [
        "file::memory:?cache=shared",
        "file:shared?mode=memory&cache=shared",
        "file:shared-encoded?mode=memory&%63ache=sha%72ed",
        "file:shared-last?mode=memory&cache=private&cache=shared",
    ] {
        let descriptor = memory_descriptor(path);
        // Keep SQLite's shared database alive across the store handover.
        let keeper = Connection::open(path).unwrap();
        let first = open_sqlite(&descriptor).unwrap();
        first
            .with_conn_fenced(|tx| {
                tx.execute_batch("CREATE TABLE kept (v INTEGER); INSERT INTO kept VALUES (7)")
            })
            .unwrap();
        let epoch = first.epoch();
        assert!(matches!(
            open_sqlite(&descriptor),
            Err(StoreError::Lease(_))
        ));
        assert!(matches!(
            acquire_store_lease(&descriptor),
            Err(StoreError::Lease(_))
        ));
        drop(first);
        let restored_epoch = epoch + 1_000_000;
        keeper
            .execute("UPDATE cortexkit_fence SET epoch = ?1", [restored_epoch])
            .unwrap();
        let second = open_sqlite(&descriptor).expect("drop releases the in-process lease");
        assert!(second.epoch() > restored_epoch);
        second
            .with_conn_fenced(|tx| {
                assert_eq!(tx.execute("DELETE FROM kept WHERE v = 7", [])?, 1);
                Ok(())
            })
            .unwrap();
        let mut replacement_descriptor = descriptor.clone();
        replacement_descriptor
            .storage_namespace
            .push_str("-replacement");
        let replacement = open_sqlite(&replacement_descriptor).unwrap();
        replacement
            .with_conn_fenced(|tx| tx.execute_batch("INSERT INTO kept VALUES (8)"))
            .unwrap();
        assert!(matches!(
            second.with_conn_fenced(|tx| tx.execute_batch("DELETE FROM kept")),
            Err(StoreError::Fenced { .. })
        ));
        drop(replacement);
        drop(second);
        drop(keeper);
    }
}

#[test]
fn disk_uris_keep_their_file_lease() {
    if !in_temporary_working_directory("disk_uris_keep_their_file_lease", false) {
        return;
    }
    for (path, filename) in [
        ("file:disk.db?mode=rwc", "disk.db"),
        ("file:fragment.db#?mode=memory", "fragment.db"),
        ("file:mode=memory.db", "mode=memory.db"),
        ("file:query.db?ignored=mode=memory", "query.db"),
        ("file:cache.db?cache=shared", "cache.db"),
    ] {
        let descriptor = memory_descriptor(path);
        let first = open_sqlite(&descriptor).unwrap();
        assert!(Path::new(filename).is_file());
        assert!(entries(Path::new("."))
            .iter()
            .any(|path| path.extension().is_some_and(|ext| ext == "lease")));
        assert!(matches!(
            open_sqlite(&descriptor),
            Err(StoreError::Lease(_))
        ));
        drop(first);
        open_sqlite(&descriptor).expect("disk lease is released on drop");
    }
}

#[test]
fn memory_and_uri_databases_do_not_claim_or_enforce_ownership() {
    if !in_temporary_working_directory(
        "memory_and_uri_databases_do_not_claim_or_enforce_ownership",
        true,
    ) {
        return;
    }
    let db = TestDb::new();
    for path in [
        "",
        ":memory:",
        "file::memory:",
        "file::memory:?cache=shared",
        "file:owner-exempt?mode=memory&cache=shared",
    ] {
        let mut descriptor = db.descriptor.clone();
        descriptor.storage_namespace = format!("memory-{}-{}", std::process::id(), path);
        descriptor.backend = StorageBackend::Sqlite { path: path.into() };
        let writer = open_sqlite(&descriptor).unwrap();
        writer.with_conn(|conn| {
            assert_eq!(conn.query_row("SELECT count(*) FROM sqlite_schema WHERE name = 'cortexkit_owner'", [], |row| row.get::<_, i64>(0)).unwrap(), 0);
            conn.execute_batch("CREATE TABLE cortexkit_owner (id INTEGER PRIMARY KEY, module_id TEXT, storage_namespace TEXT);
                                INSERT INTO cortexkit_owner VALUES (1, 'another-module', 'another-namespace')")
        }).unwrap();
        if path.contains("cache=shared") {
            let mut requested = descriptor.clone();
            requested.storage_namespace.push_str("-other");
            let other = open_sqlite(&requested).unwrap();
            other
                .with_conn(|conn| {
                    assert_eq!(
                        owner(conn),
                        ("another-module".into(), "another-namespace".into())
                    );
                    Ok(())
                })
                .unwrap();
        }
    }
}

#[test]
fn first_owner_claim_succeeds_with_a_read_snapshot_and_alias_is_refused_after_checkpoint() {
    if run_child_open_if_requested(false) {
        return;
    }
    let db = TestDb::new();
    std::fs::create_dir_all(&db.root).unwrap();
    let legacy = Connection::open(db.path()).unwrap();
    legacy.pragma_update(None, "journal_mode", "WAL").unwrap();
    legacy
        .execute_batch("CREATE TABLE points (value INTEGER); INSERT INTO points VALUES (5)")
        .unwrap();
    legacy
        .execute_batch("PRAGMA wal_checkpoint(TRUNCATE)")
        .unwrap();
    let snapshot = legacy.unchecked_transaction().unwrap();
    assert_eq!(
        snapshot
            .query_row("SELECT value FROM points", [], |row| row.get::<_, i64>(0))
            .unwrap(),
        5
    );
    let output = child_open(
        "first_owner_claim_succeeds_with_a_read_snapshot_and_alias_is_refused_after_checkpoint",
        &db,
    );
    assert_child_passed(&output);
    let warnings = String::from_utf8_lossy(&output.stderr);
    assert_eq!(
        warnings.lines().count(),
        1,
        "expected exactly one warning: {warnings}"
    );
    assert!(
        warnings.contains("first checkpoint was busy or incomplete"),
        "{warnings}"
    );
    let verification = Connection::open(db.path()).unwrap();
    assert_eq!(
        owner(&verification),
        (
            db.descriptor.module_id.clone(),
            db.descriptor.storage_namespace.clone()
        )
    );
    drop(verification);
    snapshot.commit().unwrap();
    let (busy, frames, checkpointed): (i64, i64, i64) = legacy
        .query_row("PRAGMA wal_checkpoint(FULL)", [], |row| {
            Ok((row.get(0)?, row.get(1)?, row.get(2)?))
        })
        .unwrap();
    assert_eq!(busy, 0);
    assert_eq!(frames, checkpointed);
    let alias = db.root.join("checkpointed-hard.db");
    std::fs::hard_link(db.path(), &alias).unwrap();
    let mut requested = db.other_namespace();
    requested.backend = StorageBackend::Sqlite {
        path: alias.to_string_lossy().into_owned(),
    };
    mismatch(open_sqlite(&requested), &db.descriptor, &requested);
    drop(legacy);
}

#[test]
fn recorded_owner_reopens_with_a_read_snapshot_without_checkpointing() {
    if run_child_open_if_requested(false) {
        return;
    }
    let db = TestDb::new();
    let writer = writer_with_points(&db);
    let reader = Connection::open_with_flags(db.path(), OpenFlags::SQLITE_OPEN_READ_ONLY).unwrap();
    let snapshot = reader.unchecked_transaction().unwrap();
    assert_eq!(
        snapshot
            .query_row("SELECT value FROM points", [], |row| row.get::<_, i64>(0))
            .unwrap(),
        5
    );
    writer
        .with_conn(|conn| conn.execute("UPDATE points SET value = 6", []))
        .unwrap();
    drop(writer);
    let output = child_open(
        "recorded_owner_reopens_with_a_read_snapshot_without_checkpointing",
        &db,
    );
    assert_child_passed(&output);
    assert!(
        output.stderr.is_empty(),
        "a recorded owner must not attempt a checkpoint: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    assert_eq!(
        snapshot
            .query_row("SELECT value FROM points", [], |row| row.get::<_, i64>(0))
            .unwrap(),
        5
    );
    snapshot.commit().unwrap();
    assert_eq!(
        reader
            .query_row("SELECT value FROM points", [], |row| row.get::<_, i64>(0))
            .unwrap(),
        6
    );
    assert_eq!(
        owner(&reader),
        (
            db.descriptor.module_id.clone(),
            db.descriptor.storage_namespace.clone()
        )
    );
}

#[test]
fn second_same_owner_process_with_a_read_snapshot_gets_lease_error() {
    if run_child_open_if_requested(true) {
        return;
    }
    let db = TestDb::new();
    let writer = writer_with_points(&db);
    let reader = Connection::open_with_flags(db.path(), OpenFlags::SQLITE_OPEN_READ_ONLY).unwrap();
    let snapshot = reader.unchecked_transaction().unwrap();
    assert_eq!(
        snapshot
            .query_row("SELECT value FROM points", [], |row| row.get::<_, i64>(0))
            .unwrap(),
        5
    );
    writer
        .with_conn(|conn| conn.execute("UPDATE points SET value = 6", []))
        .unwrap();
    let output = child_open(
        "second_same_owner_process_with_a_read_snapshot_gets_lease_error",
        &db,
    );
    assert_child_passed(&output);
    assert!(
        output.stderr.is_empty(),
        "a competing matching writer must not attempt a checkpoint: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    writer
        .with_conn(|conn| conn.execute("UPDATE points SET value = 7", []))
        .unwrap();
    snapshot.commit().unwrap();
    assert_eq!(
        reader
            .query_row("SELECT value FROM points", [], |row| row.get::<_, i64>(0))
            .unwrap(),
        7
    );
    drop(writer);
}

/// A matching owner's reopen reads the recorded owner without taking SQLite's
/// write lock. So while the live writer is in the middle of a write transaction,
/// a second process with the same descriptor still reaches the lease and gets the
/// lease refusal, rather than timing out on the lock as a backend error.
#[test]
fn second_same_owner_process_during_a_write_transaction_gets_lease_error() {
    if run_child_open_if_requested(true) {
        return;
    }
    let db = TestDb::new();
    let writer = writer_with_points(&db);
    writer
        .with_conn(|conn| {
            conn.execute_batch("BEGIN IMMEDIATE; UPDATE points SET value = 8")?;
            let output = child_open(
                "second_same_owner_process_during_a_write_transaction_gets_lease_error",
                &db,
            );
            assert_child_passed(&output);
            conn.execute_batch("COMMIT")
        })
        .unwrap();
    assert_eq!(
        writer
            .with_read(|tx| tx.query_row("SELECT value FROM points", [], |row| row.get::<_, i64>(0)))
            .unwrap(),
        8
    );
}
