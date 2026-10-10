#![cfg(feature = "sqlite")]

use cortexkit_store::{
    open_sqlite, open_sqlite_with, Isolation, SqliteOpenOptions, SqliteStore, StorageBackend,
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

#[test]
fn another_namespace_is_refused_before_lease_and_first_writer_keeps_writing() {
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
    assert_eq!(
        entries(&db.root),
        before,
        "a refused open must not create a lease"
    );
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
    assert!(entries(&aliases).iter().all(|path| path == &alias));
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

/// Lease files in the working directory. An in-memory or URI store's path has no
/// parent directory, so its lease lands there.
fn working_dir_leases() -> std::collections::BTreeSet<PathBuf> {
    std::fs::read_dir(".")
        .map(|dir| {
            dir.flatten()
                .map(|entry| entry.path())
                .filter(|path| path.extension().is_some_and(|ext| ext == "lease"))
                .collect()
        })
        .unwrap_or_default()
}

/// Removes the working-directory leases a test created, even when it panics, so
/// running the suite leaves no stray files in the crate folder.
struct RemoveNewLeases(std::collections::BTreeSet<PathBuf>);

impl Drop for RemoveNewLeases {
    fn drop(&mut self) {
        for lease in working_dir_leases().difference(&self.0) {
            let _ = std::fs::remove_file(lease);
        }
    }
}

#[test]
fn memory_and_uri_databases_do_not_claim_or_enforce_ownership() {
    let _leases = RemoveNewLeases(working_dir_leases());
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
