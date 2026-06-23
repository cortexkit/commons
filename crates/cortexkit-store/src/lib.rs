//! Backend mechanics for CortexKit module storage: open a database from a
//! [`StorageDescriptor`], guard it with the single-writer lease, and apply
//! versioned migrations once.
//!
//! A module does NOT reinvent persistence. It receives a resolved descriptor
//! (from subc), hands it here with its ordered migrations, and gets back a
//! lease-guarded, migrated connection it runs its own domain queries against. The
//! module owns only its domain: its store trait, its schema/seed (as migrations),
//! and its queries.
//!
//! Backends are feature-gated and additive: `sqlite` today, a `postgres` feature
//! next, a `cloud` feature later. The descriptor's backend set
//! ([`cortexkit_store_types::StorageBackend`]) is open the same way, so adding a
//! backend is additive and module code never branches on which backend it got.
//!
//! The single-writer lease ([`cortexkit_lease`]) is keyed by
//! `(module_id, backend, storage_namespace)` so two modules sharing one lease root
//! never collide, and the persisted epoch is available as the fence token a
//! distributed/cloud backend's writes would compare-and-set on.

pub use cortexkit_store_types::{
    postgres_database_name, sqlite_store_path, Isolation, StorageBackend, StorageDescriptor,
};

use cortexkit_lease::{LeaseError, LeaseKey, LeaseStore};

/// An ordered schema migration: DDL and/or seed `INSERT`s applied exactly once.
///
/// Migrations are applied in ascending `version` order; each runs once and is
/// recorded, so seed data (for example a module's initial fact rows) belongs in a
/// migration body and is inserted exactly once on first creation, never re-run on
/// later opens.
#[derive(Debug, Clone, Copy)]
pub struct Migration {
    /// Strictly increasing version. Applied when greater than the recorded max.
    pub version: u32,
    /// SQL executed as a batch (multiple statements allowed): DDL and/or seed rows.
    pub statements: &'static str,
}

#[derive(Debug)]
pub enum StoreError {
    /// A live writer already holds this module's store.
    Lease(LeaseError),
    /// The descriptor asked for a backend this build was not compiled with.
    UnsupportedBackend(String),
    /// A migration or schema-version operation failed.
    Migration(String),
    /// A backend (database driver) operation failed.
    Backend(String),
    /// An io failure preparing the store location.
    Io(std::io::Error),
}

impl std::fmt::Display for StoreError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            StoreError::Lease(e) => write!(f, "storage lease: {e}"),
            StoreError::UnsupportedBackend(b) => write!(
                f,
                "storage backend '{b}' is not supported by this build (missing feature)"
            ),
            StoreError::Migration(m) => write!(f, "migration: {m}"),
            StoreError::Backend(m) => write!(f, "storage backend: {m}"),
            StoreError::Io(e) => write!(f, "storage io: {e}"),
        }
    }
}

impl std::error::Error for StoreError {}

/// The lease key for a descriptor's store: namespaced by module + backend so two
/// modules sharing a lease root cannot collide on the same namespace.
fn lease_key(descriptor: &StorageDescriptor) -> LeaseKey {
    LeaseKey::new(
        &descriptor.module_id,
        descriptor.backend.label(),
        &descriptor.storage_namespace,
    )
}

#[cfg(feature = "sqlite")]
mod sqlite_backend {
    use super::*;
    use std::{
        path::{Path, PathBuf},
        sync::Mutex,
        time::{Duration, SystemTime, UNIX_EPOCH},
    };

    use cortexkit_lease::{FileLeaseStore, LeaseHandle};
    use rusqlite::Connection;

    /// A lease-guarded, migrated sqlite store. Holds the single-writer lease for
    /// its lifetime and serializes connection access behind a mutex (sqlite is
    /// single-connection here; the module runs its domain queries via
    /// [`SqliteStore::with_conn`]).
    pub struct SqliteStore {
        conn: Mutex<Connection>,
        epoch: u64,
        // The held lease releases on drop; kept alive for the store's lifetime.
        _lease: Box<dyn LeaseHandle>,
    }

    impl SqliteStore {
        /// The fence epoch of the held lease (strictly greater than any superseded
        /// writer's). A distributed backend would carry this on every write.
        pub fn epoch(&self) -> u64 {
            self.epoch
        }

        /// Run a closure against the connection under the store mutex. The module's
        /// domain trait implementation calls this for every query/transaction.
        pub fn with_conn<T>(
            &self,
            f: impl FnOnce(&Connection) -> rusqlite::Result<T>,
        ) -> Result<T, StoreError> {
            let guard = self.conn.lock().unwrap_or_else(|p| p.into_inner());
            f(&guard).map_err(|e| StoreError::Backend(e.to_string()))
        }

        /// Apply a `namespace`'s migration chain to this store's database, once.
        ///
        /// Applied migrations are tracked per `(namespace, version)`, so a module
        /// that owns several domains in one database registers an INDEPENDENT chain
        /// per domain (`migrate("work_graph", ..)`, `migrate("hires", ..)`): each
        /// domain's history is separate, and adding a domain later never re-runs or
        /// entangles another's migrations. A single-domain module just calls this
        /// once. Idempotent: only un-applied versions in this namespace run.
        pub fn migrate(&self, namespace: &str, migrations: &[Migration]) -> Result<(), StoreError> {
            let mut guard = self.conn.lock().unwrap_or_else(|p| p.into_inner());
            run_migrations(&mut guard, namespace, migrations)
        }
    }

    /// Open a module's sqlite store from its descriptor: acquire the single-writer
    /// lease and open the file with durable pragmas. Migrations are applied
    /// separately via [`SqliteStore::migrate`], so a multi-domain module can apply
    /// several independent per-domain chains into its one database.
    ///
    /// The lease is acquired BEFORE the file is opened, so a second live writer is
    /// rejected (`StoreError::Lease`) rather than corrupting a shared file.
    /// `lease_dir` is where lease files live (a directory the module/daemon owns;
    /// keys inside it are namespaced per module + backend).
    pub fn open_sqlite(
        descriptor: &StorageDescriptor,
        lease_dir: impl Into<PathBuf>,
    ) -> Result<SqliteStore, StoreError> {
        let path = match &descriptor.backend {
            StorageBackend::Sqlite { path } => path.clone(),
            other => return Err(StoreError::UnsupportedBackend(other.label().to_string())),
        };

        // Acquire the single-writer lease first.
        let lease = FileLeaseStore::new(lease_dir.into())
            .acquire(&lease_key(descriptor))
            .map_err(StoreError::Lease)?;
        let epoch = lease.epoch();

        if let Some(parent) = Path::new(&path).parent() {
            std::fs::create_dir_all(parent).map_err(StoreError::Io)?;
        }

        let conn = Connection::open(&path).map_err(|e| StoreError::Backend(e.to_string()))?;
        // Durability + concurrency pragmas: WAL for concurrent readers, a busy
        // timeout so a transient lock waits rather than erroring, foreign keys on.
        conn.pragma_update(None, "journal_mode", "WAL")
            .map_err(|e| StoreError::Backend(e.to_string()))?;
        conn.busy_timeout(Duration::from_secs(5))
            .map_err(|e| StoreError::Backend(e.to_string()))?;
        conn.pragma_update(None, "foreign_keys", "ON")
            .map_err(|e| StoreError::Backend(e.to_string()))?;

        Ok(SqliteStore {
            conn: Mutex::new(conn),
            epoch,
            _lease: lease,
        })
    }

    /// Apply un-applied migrations for one `namespace` in ascending version order,
    /// each in its own transaction together with its version record, so a migration
    /// and the record that it ran commit atomically (a crash mid-migration leaves
    /// it un-recorded and it re-runs cleanly next open).
    ///
    /// Applied migrations are keyed by `(namespace, version)`, so independent
    /// domain chains in one database never collide or re-run each other.
    fn run_migrations(
        conn: &mut Connection,
        namespace: &str,
        migrations: &[Migration],
    ) -> Result<(), StoreError> {
        conn.execute_batch(
            "CREATE TABLE IF NOT EXISTS cortexkit_schema_version (\
                 namespace TEXT NOT NULL, \
                 version INTEGER NOT NULL, \
                 applied_at_unix INTEGER NOT NULL, \
                 PRIMARY KEY (namespace, version)\
             )",
        )
        .map_err(|e| StoreError::Migration(e.to_string()))?;

        let current: u32 = conn
            .query_row(
                "SELECT COALESCE(MAX(version), 0) FROM cortexkit_schema_version WHERE namespace = ?1",
                rusqlite::params![namespace],
                |r| r.get(0),
            )
            .map_err(|e| StoreError::Migration(e.to_string()))?;

        let mut ordered: Vec<&Migration> = migrations.iter().collect();
        ordered.sort_by_key(|m| m.version);

        for m in ordered {
            if m.version <= current {
                continue;
            }
            let tx = conn
                .transaction()
                .map_err(|e| StoreError::Migration(e.to_string()))?;
            tx.execute_batch(m.statements).map_err(|e| {
                StoreError::Migration(format!(
                    "namespace '{namespace}' migration {}: {e}",
                    m.version
                ))
            })?;
            tx.execute(
                "INSERT INTO cortexkit_schema_version (namespace, version, applied_at_unix) \
                 VALUES (?1, ?2, ?3)",
                rusqlite::params![namespace, m.version, now_unix()],
            )
            .map_err(|e| StoreError::Migration(e.to_string()))?;
            tx.commit()
                .map_err(|e| StoreError::Migration(e.to_string()))?;
        }
        Ok(())
    }

    fn now_unix() -> i64 {
        SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map(|d| d.as_secs() as i64)
            .unwrap_or(0)
    }
}

#[cfg(feature = "sqlite")]
pub use sqlite_backend::{open_sqlite, SqliteStore};

#[cfg(all(test, feature = "sqlite"))]
mod tests {
    use super::*;

    fn tmp() -> (PathBufs, StorageDescriptor) {
        use std::sync::atomic::{AtomicU64, Ordering};
        static SEQ: AtomicU64 = AtomicU64::new(0);
        // Per-call atomic counter (not a clock) guarantees a unique dir even when
        // tests run in parallel and the clock resolution is coarse, so two tests
        // never share a lease file.
        let root = std::env::temp_dir().join(format!(
            "cortexkit-store-{}-{}-{}",
            std::process::id(),
            now_nanos(),
            SEQ.fetch_add(1, Ordering::Relaxed)
        ));
        let db = root.join("store.db");
        let descriptor = StorageDescriptor {
            module_id: "test-module".into(),
            storage_namespace: "main".into(),
            isolation: Isolation::Module,
            backend: StorageBackend::Sqlite {
                path: db.to_string_lossy().into_owned(),
            },
        };
        (
            PathBufs {
                root: root.clone(),
                lease_dir: root.join("leases"),
            },
            descriptor,
        )
    }

    struct PathBufs {
        root: std::path::PathBuf,
        lease_dir: std::path::PathBuf,
    }

    fn now_nanos() -> u128 {
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos()
    }

    const M1: &[Migration] = &[Migration {
        version: 1,
        statements: "CREATE TABLE facts (id INTEGER PRIMARY KEY, name TEXT NOT NULL); \
                     INSERT INTO facts (id, name) VALUES (1, 'seed-a'), (2, 'seed-b');",
    }];

    #[test]
    fn open_runs_migrations_and_seeds_once() {
        let (p, d) = tmp();
        {
            let store = open_sqlite(&d, &p.lease_dir).expect("open");
            store.migrate("facts", M1).expect("migrate");
            let n: i64 = store
                .with_conn(|c| c.query_row("SELECT COUNT(*) FROM facts", [], |r| r.get(0)))
                .expect("count");
            assert_eq!(n, 2, "seed rows inserted");
            assert_eq!(store.epoch(), 1);
        }
        // Reopen: migration must NOT re-run (no duplicate seed rows), epoch bumps.
        {
            let store = open_sqlite(&d, &p.lease_dir).expect("reopen");
            store.migrate("facts", M1).expect("migrate again");
            let n: i64 = store
                .with_conn(|c| c.query_row("SELECT COUNT(*) FROM facts", [], |r| r.get(0)))
                .expect("count");
            assert_eq!(n, 2, "seed not re-inserted on reopen (run-once)");
            assert_eq!(store.epoch(), 2, "lease epoch is monotonic across opens");
        }
        let _ = std::fs::remove_dir_all(&p.root);
    }

    #[test]
    fn second_live_writer_is_rejected() {
        let (p, d) = tmp();
        let _held = open_sqlite(&d, &p.lease_dir).expect("first open");
        match open_sqlite(&d, &p.lease_dir) {
            Err(StoreError::Lease(_)) => {}
            Err(e) => panic!("expected Lease(Held), got {e}"),
            Ok(_) => panic!("expected Lease(Held), got a second open"),
        }
        let _ = std::fs::remove_dir_all(&p.root);
    }

    #[test]
    fn later_migration_applies_on_top_of_earlier() {
        let (p, d) = tmp();
        {
            let s = open_sqlite(&d, &p.lease_dir).expect("v1");
            s.migrate("facts", M1).expect("v1 migrate");
        }
        const M2: &[Migration] = &[
            Migration {
                version: 1,
                statements: "CREATE TABLE facts (id INTEGER PRIMARY KEY, name TEXT NOT NULL);",
            },
            Migration {
                version: 2,
                statements: "ALTER TABLE facts ADD COLUMN weight REAL NOT NULL DEFAULT 0;",
            },
        ];
        let store = open_sqlite(&d, &p.lease_dir).expect("v2");
        store.migrate("facts", M2).expect("v2 migrate");
        // The v2 column exists and v1 did not re-run (table already present).
        let ok: i64 = store
            .with_conn(|c| {
                c.query_row("SELECT COUNT(*) FROM facts WHERE weight = 0", [], |r| {
                    r.get(0)
                })
            })
            .expect("weight column queryable");
        assert_eq!(ok, 2);
        let _ = std::fs::remove_dir_all(&p.root);
    }

    #[test]
    fn independent_namespace_chains_in_one_database() {
        // A multi-domain module (one database, several domains) registers an
        // independent migration chain per domain. Each chain's history is separate:
        // adding the second domain later does not re-run or entangle the first.
        let (p, d) = tmp();
        const WORK_GRAPH: &[Migration] = &[Migration {
            version: 1,
            statements: "CREATE TABLE wg_nodes (id INTEGER PRIMARY KEY);",
        }];
        const HIRES: &[Migration] = &[Migration {
            version: 1,
            statements: "CREATE TABLE hires (id INTEGER PRIMARY KEY);",
        }];
        let store = open_sqlite(&d, &p.lease_dir).expect("open");
        // Both domains use version 1, but distinct namespaces -> both run.
        store.migrate("work_graph", WORK_GRAPH).expect("work_graph");
        store.migrate("hires", HIRES).expect("hires");
        // Re-applying is a no-op per namespace; same version across namespaces did
        // not collide (both tables exist).
        store
            .migrate("work_graph", WORK_GRAPH)
            .expect("work_graph again");
        let tables: i64 = store
            .with_conn(|c| {
                c.query_row(
                    "SELECT COUNT(*) FROM sqlite_master WHERE type='table' AND name IN ('wg_nodes','hires')",
                    [],
                    |r| r.get(0),
                )
            })
            .expect("count tables");
        assert_eq!(
            tables, 2,
            "both domains' tables exist; version 1 did not collide across namespaces"
        );
        let _ = std::fs::remove_dir_all(&p.root);
    }

    #[test]
    fn unsupported_backend_is_rejected() {
        let d = StorageDescriptor {
            module_id: "m".into(),
            storage_namespace: "n".into(),
            isolation: Isolation::Module,
            backend: StorageBackend::Postgres {
                dsn: "postgres://x".into(),
                database: "y".into(),
            },
        };
        match open_sqlite(&d, std::env::temp_dir()) {
            Err(StoreError::UnsupportedBackend(b)) => assert_eq!(b, "postgres"),
            Err(e) => panic!("expected UnsupportedBackend, got {e}"),
            Ok(_) => panic!("expected UnsupportedBackend, got an open store"),
        }
    }
}
