//! Postgres backend mechanics for CortexKit module storage: open a per-module
//! postgres database from a [`StorageDescriptor`], guard it with a native
//! single-writer lease, and apply versioned migrations once.
//!
//! This is the parallel of the sqlite backend in `cortexkit-store`: same
//! descriptor in, same open/migrate/lease surface out, so a module's domain code
//! is unchanged when the central storage policy switches sqlite to postgres. The
//! module receives a `Postgres { dsn, database }` descriptor whose `dsn` is a
//! scoped, least-privilege runtime credential reaching only its own database; the
//! per-module database and role are provisioned out of band (this crate connects
//! and migrates, it does not `CREATE DATABASE`).
//!
//! ## Single-writer lease
//!
//! sqlite uses a file advisory lock for liveness; postgres cannot (a database
//! client holds no file). Instead this uses a postgres SESSION advisory lock
//! (`pg_advisory_lock`), which the server releases automatically when the
//! connection drops, giving the same crash-releases-the-lock liveness across
//! processes AND machines. The epoch fence is persisted in a small lease table in
//! the module's own database, bumped under the advisory lock, matching the
//! file-lease semantics so a distributed writer's durable writes can carry a
//! monotonic fence token.

use cortexkit_lease::{LeaseError, LeaseKey};
use cortexkit_store_types::{StorageBackend, StorageDescriptor};
use postgres::{Client, NoTls};

pub use cortexkit_store_types::{Isolation, StorageBackend as Backend};

/// An ordered schema migration: DDL and/or seed `INSERT`s applied exactly once,
/// in ascending `version` order, tracked per `(namespace, version)`.
#[derive(Debug, Clone, Copy)]
pub struct Migration {
    pub version: u32,
    pub statements: &'static str,
}

#[derive(Debug)]
pub enum StoreError {
    Lease(LeaseError),
    UnsupportedBackend(String),
    Migration(String),
    Backend(String),
}

impl std::fmt::Display for StoreError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            StoreError::Lease(e) => write!(f, "storage lease: {e}"),
            StoreError::UnsupportedBackend(b) => write!(
                f,
                "storage backend '{b}' is not a postgres descriptor for this backend"
            ),
            StoreError::Migration(m) => write!(f, "migration: {m}"),
            StoreError::Backend(m) => write!(f, "storage backend: {m}"),
        }
    }
}

impl std::error::Error for StoreError {}

/// A 64-bit advisory-lock key derived from the namespaced lease identity. postgres
/// `pg_advisory_lock` takes a bigint; we hash the `(module_id, backend, namespace)`
/// identity into one so distinct modules/namespaces map to distinct locks.
fn advisory_key(key: &LeaseKey) -> i64 {
    // FNV-1a 64-bit over the same identity the file lease uses, reinterpreted as a
    // signed bigint for postgres' advisory-lock API.
    let identity = format!(
        "{}\u{1f}{}\u{1f}{}",
        key.module_id, key.backend, key.scope_key
    );
    let mut h: u64 = 0xcbf2_9ce4_8422_2325;
    for b in identity.as_bytes() {
        h ^= *b as u64;
        h = h.wrapping_mul(0x0000_0100_0000_01b3);
    }
    h as i64
}

fn lease_key(descriptor: &StorageDescriptor) -> LeaseKey {
    LeaseKey::new(
        &descriptor.module_id,
        descriptor.backend.label(),
        &descriptor.storage_namespace,
    )
}

/// A lease-guarded, migrated postgres store. Holds a session advisory lock on its
/// connection for the store's lifetime (released when the connection drops, e.g.
/// on crash). The module runs its domain queries via [`PostgresStore::with_client`].
pub struct PostgresStore {
    client: std::sync::Mutex<Client>,
    epoch: i64,
    lease_key: i64,
}

impl PostgresStore {
    /// The fence epoch of the held lease (strictly greater than any superseded
    /// writer's), available for a distributed write-path compare-and-set.
    pub fn epoch(&self) -> i64 {
        self.epoch
    }

    /// Run a closure against the postgres client under the store mutex. The
    /// module's domain trait implementation calls this for every query/transaction.
    pub fn with_client<T>(
        &self,
        f: impl FnOnce(&mut Client) -> Result<T, postgres::Error>,
    ) -> Result<T, StoreError> {
        let mut guard = self.client.lock().unwrap_or_else(|p| p.into_inner());
        f(&mut guard).map_err(|e| StoreError::Backend(e.to_string()))
    }

    /// Apply a `namespace`'s migration chain to this database, once. Applied
    /// migrations are tracked per `(namespace, version)`, so a multi-domain module
    /// registers an independent chain per domain.
    pub fn migrate(&self, namespace: &str, migrations: &[Migration]) -> Result<(), StoreError> {
        let mut guard = self.client.lock().unwrap_or_else(|p| p.into_inner());
        run_migrations(&mut guard, namespace, migrations)
    }
}

impl Drop for PostgresStore {
    fn drop(&mut self) {
        // Best-effort explicit unlock; the server also releases the session lock
        // when the connection closes on drop.
        if let Ok(mut guard) = self.client.lock() {
            let _ = guard.execute("SELECT pg_advisory_unlock($1)", &[&self.lease_key]);
        }
    }
}

/// Open a module's postgres store from its descriptor: connect with the scoped
/// DSN, acquire the native single-writer advisory lock, and bump the persisted
/// epoch. Migrations are applied separately via [`PostgresStore::migrate`].
///
/// A second live writer is rejected (`StoreError::Lease`) because the advisory
/// lock is already held by the first connection.
pub fn open_postgres(descriptor: &StorageDescriptor) -> Result<PostgresStore, StoreError> {
    let dsn = match &descriptor.backend {
        StorageBackend::Postgres { dsn, .. } => dsn.clone(),
        other => return Err(StoreError::UnsupportedBackend(other.label().to_string())),
    };

    let mut client =
        Client::connect(&dsn, NoTls).map_err(|e| StoreError::Backend(e.to_string()))?;

    let lease_id = advisory_key(&lease_key(descriptor));

    // Single-writer gate: try the session advisory lock without blocking. If a
    // live writer (another connection) holds it, reject as Held rather than block
    // forever.
    let acquired: bool = client
        .query_one("SELECT pg_try_advisory_lock($1)", &[&lease_id])
        .map_err(|e| StoreError::Backend(e.to_string()))?
        .get(0);
    if !acquired {
        return Err(StoreError::Lease(LeaseError::Held {
            key: lease_key(descriptor),
        }));
    }

    // We hold the lock: ensure the lease/epoch table and bump the epoch fence.
    let epoch = match bump_epoch(&mut client, lease_id) {
        Ok(epoch) => epoch,
        Err(e) => {
            let _ = client.execute("SELECT pg_advisory_unlock($1)", &[&lease_id]);
            return Err(e);
        }
    };

    Ok(PostgresStore {
        client: std::sync::Mutex::new(client),
        epoch,
        lease_key: lease_id,
    })
}

/// Persist + increment the monotonic epoch fence in the module's own database,
/// under the held advisory lock.
fn bump_epoch(client: &mut Client, lease_id: i64) -> Result<i64, StoreError> {
    client
        .batch_execute(
            "CREATE TABLE IF NOT EXISTS cortexkit_lease (\
                 lease_key BIGINT PRIMARY KEY, \
                 epoch BIGINT NOT NULL\
             )",
        )
        .map_err(|e| StoreError::Migration(e.to_string()))?;
    let row = client
        .query_one(
            "INSERT INTO cortexkit_lease (lease_key, epoch) VALUES ($1, 1) \
             ON CONFLICT (lease_key) DO UPDATE SET epoch = cortexkit_lease.epoch + 1 \
             RETURNING epoch",
            &[&lease_id],
        )
        .map_err(|e| StoreError::Migration(e.to_string()))?;
    Ok(row.get(0))
}

/// Apply un-applied migrations for one `namespace` in ascending version order,
/// each in its own transaction with its version record, so a crash mid-migration
/// leaves it un-recorded and it re-runs cleanly. Keyed by `(namespace, version)`.
fn run_migrations(
    client: &mut Client,
    namespace: &str,
    migrations: &[Migration],
) -> Result<(), StoreError> {
    client
        .batch_execute(
            "CREATE TABLE IF NOT EXISTS cortexkit_schema_version (\
                 namespace TEXT NOT NULL, \
                 version INTEGER NOT NULL, \
                 applied_at_unix BIGINT NOT NULL, \
                 PRIMARY KEY (namespace, version)\
             )",
        )
        .map_err(|e| StoreError::Migration(e.to_string()))?;

    let current: i32 = client
        .query_one(
            "SELECT COALESCE(MAX(version), 0) FROM cortexkit_schema_version WHERE namespace = $1",
            &[&namespace],
        )
        .map_err(|e| StoreError::Migration(e.to_string()))?
        .get(0);
    let current = current as u32;

    let mut ordered: Vec<&Migration> = migrations.iter().collect();
    ordered.sort_by_key(|m| m.version);

    for m in ordered {
        if m.version <= current {
            continue;
        }
        let mut tx = client
            .transaction()
            .map_err(|e| StoreError::Migration(e.to_string()))?;
        tx.batch_execute(m.statements).map_err(|e| {
            StoreError::Migration(format!(
                "namespace '{namespace}' migration {}: {e}",
                m.version
            ))
        })?;
        tx.execute(
            "INSERT INTO cortexkit_schema_version (namespace, version, applied_at_unix) \
             VALUES ($1, $2, $3)",
            &[&namespace, &(m.version as i32), &now_unix()],
        )
        .map_err(|e| StoreError::Migration(e.to_string()))?;
        tx.commit()
            .map_err(|e| StoreError::Migration(e.to_string()))?;
    }
    Ok(())
}

fn now_unix() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs() as i64)
        .unwrap_or(0)
}

/// Live postgres tests. Require a reachable postgres and a DSN in
/// `CORTEXKIT_TEST_PG_DSN`; skipped (pass) when unset so the default `cargo test`
/// stays green without a database. CI provides a postgres service + the env var.
#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicU64, Ordering};

    fn test_dsn() -> Option<String> {
        match std::env::var("CORTEXKIT_TEST_PG_DSN") {
            Ok(dsn) => Some(dsn),
            // Anti-masking guard: the CI job that is SUPPOSED to run the live tests
            // sets CORTEXKIT_REQUIRE_PG. If that marker is present but the DSN is
            // not, the postgres service wiring is broken and these tests would
            // silently skip-pass (a false green) — fail loud instead. Locally
            // (marker unset) a missing DSN just skips.
            Err(_) => {
                assert!(
                    std::env::var("CORTEXKIT_REQUIRE_PG").is_err(),
                    "CORTEXKIT_REQUIRE_PG is set but CORTEXKIT_TEST_PG_DSN is missing: the live \
                     postgres tests must run in this job, not skip-pass"
                );
                None
            }
        }
    }

    fn descriptor(dsn: &str, namespace: &str) -> StorageDescriptor {
        StorageDescriptor {
            module_id: "test-module".into(),
            storage_namespace: namespace.into(),
            isolation: Isolation::Module,
            backend: StorageBackend::Postgres {
                dsn: dsn.into(),
                database: "test".into(),
            },
        }
    }

    // A unique namespace per test run so parallel tests + repeat runs against one
    // shared test database never collide on lease keys or migration tables.
    fn unique_ns(tag: &str) -> String {
        static SEQ: AtomicU64 = AtomicU64::new(0);
        let n = SEQ.fetch_add(1, Ordering::Relaxed);
        let t = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        format!("{tag}_{}_{t}_{n}", std::process::id())
    }

    const M1: &[Migration] = &[Migration {
        version: 1,
        statements:
            "CREATE TABLE IF NOT EXISTS facts_probe (id INT PRIMARY KEY, name TEXT NOT NULL);",
    }];

    #[test]
    fn open_migrate_and_single_writer() {
        let Some(dsn) = test_dsn() else {
            eprintln!("CORTEXKIT_TEST_PG_DSN unset; skipping live postgres test");
            return;
        };
        let ns = unique_ns("sw");
        let d = descriptor(&dsn, &ns);

        let store = open_postgres(&d).expect("open");
        store.migrate(&ns, M1).expect("migrate");
        assert!(store.epoch() >= 1);

        // A second live open on the same lease key is rejected while the first holds.
        match open_postgres(&d) {
            Err(StoreError::Lease(_)) => {}
            Err(e) => panic!("expected Lease(Held), got {e}"),
            Ok(_) => panic!("expected Lease(Held), got a second open"),
        }

        // Migration is run-once: a domain query proves the table exists, and the
        // schema-version row is present for this namespace.
        let applied: i64 = store
            .with_client(|c| {
                Ok(c.query_one(
                    "SELECT COUNT(*) FROM cortexkit_schema_version WHERE namespace = $1",
                    &[&ns],
                )?
                .get(0))
            })
            .expect("schema version query");
        assert_eq!(
            applied, 1,
            "exactly one migration recorded for the namespace"
        );

        drop(store);
        // After release, a re-open succeeds and the epoch advanced.
        let reopened = open_postgres(&d).expect("reopen after release");
        assert!(reopened.epoch() >= 2, "epoch is monotonic across opens");
    }

    #[test]
    fn independent_namespace_chains() {
        let Some(dsn) = test_dsn() else {
            return;
        };
        let ns = unique_ns("ns");
        let d = descriptor(&dsn, &ns);
        let store = open_postgres(&d).expect("open");
        const A: &[Migration] = &[Migration {
            version: 1,
            statements: "CREATE TABLE IF NOT EXISTS dom_a (id INT PRIMARY KEY);",
        }];
        const B: &[Migration] = &[Migration {
            version: 1,
            statements: "CREATE TABLE IF NOT EXISTS dom_b (id INT PRIMARY KEY);",
        }];
        store.migrate(&format!("{ns}_a"), A).expect("a");
        store
            .migrate(&format!("{ns}_b"), B)
            .expect("b - same version, distinct namespace");
        let count: i64 = store
            .with_client(|c| {
                Ok(c.query_one(
                    "SELECT COUNT(*) FROM cortexkit_schema_version WHERE namespace IN ($1, $2)",
                    &[&format!("{ns}_a"), &format!("{ns}_b")],
                )?
                .get(0))
            })
            .expect("count");
        assert_eq!(count, 2, "both namespace chains recorded independently");
    }

    #[test]
    fn sqlite_descriptor_is_rejected() {
        let d = StorageDescriptor {
            module_id: "m".into(),
            storage_namespace: "n".into(),
            isolation: Isolation::Module,
            backend: StorageBackend::Sqlite {
                path: "/tmp/x.db".into(),
            },
        };
        match open_postgres(&d) {
            Err(StoreError::UnsupportedBackend(b)) => assert_eq!(b, "sqlite"),
            Err(e) => panic!("expected UnsupportedBackend, got {e}"),
            Ok(_) => panic!("expected UnsupportedBackend"),
        }
    }
}
