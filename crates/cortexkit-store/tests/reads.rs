#![cfg(feature = "sqlite")]

use cortexkit_store::{
    open_sqlite, open_sqlite_with, Isolation, Migration, SqliteOpenOptions, SqliteStore,
    StorageBackend, StorageDescriptor,
};
use rusqlite::{Connection, Transaction, TransactionBehavior};
use std::{
    path::PathBuf,
    sync::{
        atomic::{AtomicBool, AtomicU64, Ordering},
        mpsc, Arc, Barrier,
    },
    thread,
    time::{Duration, Instant, SystemTime, UNIX_EPOCH},
};

struct TestDb {
    root: PathBuf,
    descriptor: StorageDescriptor,
}

impl TestDb {
    fn new() -> Self {
        static SEQ: AtomicU64 = AtomicU64::new(0);
        let root = std::env::temp_dir().join(format!(
            "cortexkit-store-reads-{}-{}-{}",
            std::process::id(),
            SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap()
                .as_nanos(),
            SEQ.fetch_add(1, Ordering::Relaxed)
        ));
        let descriptor = StorageDescriptor {
            module_id: "read-tests".into(),
            storage_namespace: "main".into(),
            isolation: Isolation::Module,
            backend: StorageBackend::Sqlite {
                path: root.join("store.db").to_string_lossy().into_owned(),
            },
        };
        Self { root, descriptor }
    }

    fn open(&self, options: SqliteOpenOptions) -> Arc<SqliteStore> {
        let store = open_sqlite_with(&self.descriptor, options).unwrap();
        store.migrate("points", SCHEMA).unwrap();
        Arc::new(store)
    }
}

impl Drop for TestDb {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.root);
    }
}

const SCHEMA: &[Migration] = &[Migration {
    version: 1,
    statements: "CREATE TABLE points (id INTEGER PRIMARY KEY, value INTEGER NOT NULL);
                 INSERT INTO points VALUES (1, 10);",
}];

fn point(conn: &Connection) -> rusqlite::Result<i64> {
    conn.query_row("SELECT value FROM points WHERE id = 1", [], |row| {
        row.get(0)
    })
}

#[test]
fn read_runs_beside_uncommitted_writer() {
    let db = TestDb::new();
    let store = db.open(SqliteOpenOptions::default());
    let entered = Arc::new(Barrier::new(2));
    let (release_tx, release_rx) = mpsc::channel();
    let writer = {
        let store = Arc::clone(&store);
        let entered = Arc::clone(&entered);
        thread::spawn(move || {
            store
                .with_conn(|conn| {
                    let tx = Transaction::new_unchecked(conn, TransactionBehavior::Immediate)?;
                    tx.execute("UPDATE points SET value = 20 WHERE id = 1", [])?;
                    entered.wait();
                    release_rx.recv().unwrap();
                    tx.commit()
                })
                .unwrap();
        })
    };
    entered.wait();
    let (read_tx, read_rx) = mpsc::channel();
    let reader = {
        let store = Arc::clone(&store);
        thread::spawn(move || read_tx.send(store.with_read(point)).unwrap())
    };
    // The deadline bounds failure rather than ordering the threads. Always release
    // the writer before asserting, so a mutex regression fails instead of hanging.
    let before_commit = read_rx.recv_timeout(Duration::from_secs(2));
    release_tx.send(()).unwrap();
    writer.join().unwrap();
    reader.join().unwrap();
    assert_eq!(
        before_commit
            .expect("with_read blocked on the writer mutex")
            .unwrap(),
        10
    );
    assert_eq!(store.with_read(point).unwrap(), 20);
}

#[test]
fn new_read_sees_completed_commit() {
    let db = TestDb::new();
    let store = db.open(SqliteOpenOptions::default());
    assert_eq!(store.with_read(point).unwrap(), 10);
    store
        .with_conn(|conn| {
            let tx = Transaction::new_unchecked(conn, TransactionBehavior::Immediate)?;
            tx.execute("UPDATE points SET value = 20 WHERE id = 1", [])?;
            tx.commit()
        })
        .unwrap();
    assert_eq!(store.with_read(point).unwrap(), 20);
}

#[test]
fn reader_rejects_writes_without_changing_database() {
    let db = TestDb::new();
    let store = db.open(SqliteOpenOptions::default());
    let result =
        store.with_read(|conn| conn.execute("UPDATE points SET value = 99 WHERE id = 1", []));
    assert!(result.is_err(), "reader unexpectedly accepted a write");
    assert_eq!(store.with_conn(point).unwrap(), 10);
    assert_eq!(store.with_read(point).unwrap(), 10);
}

#[test]
fn two_statements_share_one_snapshot() {
    let db = TestDb::new();
    let store = db.open(SqliteOpenOptions::default());
    let first_read = Arc::new(Barrier::new(2));
    let committed = Arc::new(Barrier::new(2));
    let writer = {
        let store = Arc::clone(&store);
        let first_read = Arc::clone(&first_read);
        let committed = Arc::clone(&committed);
        thread::spawn(move || {
            first_read.wait();
            store
                .with_conn(|conn| {
                    let tx = Transaction::new_unchecked(conn, TransactionBehavior::Immediate)?;
                    tx.execute("UPDATE points SET value = 20 WHERE id = 1", [])?;
                    tx.commit()
                })
                .unwrap();
            committed.wait();
        })
    };
    let (before, after) = store
        .with_read(|conn| {
            let before = point(conn)?;
            first_read.wait();
            committed.wait();
            Ok((before, point(conn)?))
        })
        .unwrap();
    writer.join().unwrap();
    assert_eq!(before, 10);
    assert_eq!(after, 10, "read snapshot changed between statements");
    assert_eq!(store.with_read(point).unwrap(), 20);
}

#[test]
fn exhausted_read_pool_waits_then_names_pool() {
    let db = TestDb::new();
    let timeout = Duration::from_millis(75);
    let store = db.open(SqliteOpenOptions {
        read_pool_size: 2,
        busy_timeout: timeout,
    });
    let entered = Arc::new(Barrier::new(3));
    let release = Arc::new(Barrier::new(3));
    let readers: Vec<_> = (0..2)
        .map(|_| {
            let store = Arc::clone(&store);
            let entered = Arc::clone(&entered);
            let release = Arc::clone(&release);
            thread::spawn(move || {
                store
                    .with_read(|conn| {
                        let value = point(conn)?;
                        entered.wait();
                        release.wait();
                        Ok(value)
                    })
                    .unwrap()
            })
        })
        .collect();
    entered.wait();
    let start = Instant::now();
    let result = store.with_read(point);
    let elapsed = start.elapsed();
    release.wait();
    for reader in readers {
        assert_eq!(reader.join().unwrap(), 10);
    }
    let error = result.expect_err("pool exceeded its configured limit");
    assert!(error.to_string().contains("read pool"), "{error}");
    assert!(elapsed >= timeout, "pool did not wait: {elapsed:?}");
    assert_eq!(store.with_read(point).unwrap(), 10);
}

#[test]
fn readers_see_migrations_and_do_not_create_fence() {
    let db = TestDb::new();
    let store = open_sqlite(&db.descriptor).unwrap();
    store.migrate("points", SCHEMA).unwrap();
    assert_eq!(store.with_read(point).unwrap(), 10);
    store
        .migrate(
            "points",
            &[Migration {
                version: 2,
                statements: "ALTER TABLE points ADD COLUMN label TEXT DEFAULT 'migrated';",
            }],
        )
        .unwrap();
    let label: String = store
        .with_read(|conn| {
            conn.query_row("SELECT label FROM points WHERE id = 1", [], |row| {
                row.get(0)
            })
        })
        .unwrap();
    assert_eq!(label, "migrated");
    let fences: i64 = store
        .with_read(|conn| {
            conn.query_row(
                "SELECT count(*) FROM sqlite_master WHERE name = 'cortexkit_fence'",
                [],
                |row| row.get(0),
            )
        })
        .unwrap();
    assert_eq!(fences, 0);
}

// Unix only: the test forces a reader open to fail by renaming the database
// while the writer holds it open, and Windows refuses to rename an open file.
#[cfg(unix)]
#[test]
fn failed_reader_open_is_lazy_and_retried() {
    let db = TestDb::new();
    let store = db.open(SqliteOpenOptions {
        read_pool_size: 1,
        ..SqliteOpenOptions::default()
    });
    let path = db.root.join("store.db");
    let moved = db.root.join("moved.db");
    std::fs::rename(&path, &moved).unwrap();
    let failed = store.with_read(point);
    std::fs::rename(&moved, &path).unwrap();
    assert!(
        failed.is_err(),
        "reader was opened eagerly or created a file"
    );
    assert_eq!(store.with_read(point).unwrap(), 10);
}

#[test]
fn closure_error_and_panic_release_reader_transaction() {
    let db = TestDb::new();
    let store = db.open(SqliteOpenOptions {
        read_pool_size: 1,
        ..SqliteOpenOptions::default()
    });
    let failed = store.with_read(|conn| {
        assert!(!conn.is_autocommit());
        point(conn)?;
        Err::<(), _>(rusqlite::Error::InvalidQuery)
    });
    assert!(failed.is_err());
    let panicked = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        let _ = store.with_read::<()>(|conn| {
            assert_eq!(point(conn)?, 10);
            panic!("abort read closure");
        });
    }));
    assert!(panicked.is_err());
    store
        .with_conn(|conn| conn.execute("UPDATE points SET value = 20 WHERE id = 1", []))
        .unwrap();
    assert_eq!(store.with_read(point).unwrap(), 20);
}

#[test]
fn zero_read_pool_size_is_rejected() {
    let db = TestDb::new();
    let result = open_sqlite_with(
        &db.descriptor,
        SqliteOpenOptions {
            read_pool_size: 0,
            ..SqliteOpenOptions::default()
        },
    );
    assert!(matches!(result, Err(e) if e.to_string().contains("read pool")));
}

/// Run explicitly with `cargo test -p cortexkit-store --test reads
/// point_lookup_latency_under_commits -- --ignored --nocapture`.
#[test]
#[ignore = "measurement, not a timing gate"]
fn point_lookup_latency_under_commits() {
    let db = TestDb::new();
    let store = db.open(SqliteOpenOptions::default());
    // Warm the lazy pool before measuring individual lookups.
    store.with_read(point).unwrap();
    store
        .with_conn(|conn| conn.execute_batch("CREATE TABLE IF NOT EXISTS ballast (x BLOB)"))
        .unwrap();
    let running = Arc::new(AtomicBool::new(true));
    let entered = Arc::new(Barrier::new(2));
    let writer = {
        let store = Arc::clone(&store);
        let running = Arc::clone(&running);
        let entered = Arc::clone(&entered);
        thread::spawn(move || {
            let mut commits = 0;
            while running.load(Ordering::Relaxed) {
                store
                    .with_conn(|conn| {
                        let tx = Transaction::new_unchecked(conn, TransactionBehavior::Immediate)?;
                        tx.execute("UPDATE points SET value = value + 1 WHERE id = 1", [])?;
                        // A realistic write holds the writer for a while: a few hundred
                        // rows per commit, so a mutexed read can land behind it.
                        for _ in 0..300 {
                            tx.execute("INSERT INTO ballast (x) VALUES (zeroblob(256))", [])?;
                        }
                        if commits == 0 {
                            entered.wait();
                        }
                        tx.commit()
                    })
                    .unwrap();
                commits += 1;
            }
            commits
        })
    };
    entered.wait();
    let mut read_times = Vec::with_capacity(1000);
    let mut conn_times = Vec::with_capacity(1000);
    for _ in 0..1000 {
        let started = Instant::now();
        store.with_read(point).unwrap();
        read_times.push(started.elapsed());
        let started = Instant::now();
        store.with_conn(point).unwrap();
        conn_times.push(started.elapsed());
    }
    running.store(false, Ordering::Relaxed);
    let commits = writer.join().unwrap();
    read_times.sort_unstable();
    conn_times.sort_unstable();
    let read_median = (read_times[499] + read_times[500]) / 2;
    let conn_median = (conn_times[499] + conn_times[500]) / 2;
    // The pool's benefit is in the tail: a mutexed read waits whenever it lands
    // behind a commit, which the median hides.
    println!(
        "1000 point lookups per API, {commits} concurrent commits: \
         with_read p50 {read_median:?} p99 {:?} max {:?}; \
         with_conn p50 {conn_median:?} p99 {:?} max {:?}",
        read_times[989], read_times[999], conn_times[989], conn_times[999]
    );
}
