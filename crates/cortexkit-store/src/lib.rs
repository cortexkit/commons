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
//!
//! File-backed SQLite databases also record their `(module_id, storage_namespace)`
//! owner, checked right after the lease is taken. A different owner is refused
//! even through a path alias. Read-only operations do not claim or check
//! ownership; in-memory and URI databases are exempt. Intentionally changing
//! ownership requires clearing the row in `cortexkit_owner` while all writers
//! are stopped; there is no API for it.
//! A first claim attempts a best-effort WAL checkpoint. If readers delay it, the
//! writer still opens, but hard-link aliases may not see the owner until a later
//! checkpoint copies the row into the main file. Matching-owner opens never
//! checkpoint ownership.
//!
//! A module that deletes data keeps a removals journal beside its database
//! (`<store dir>/removals/journal`) and opens with
//! [`open_sqlite_with_removals`], which replays every journaled removal the
//! database has not applied before it serves, so an older copy of the database
//! can't bring removed data back. It removes through [`SqliteStore::remove`].
//! A store that has adopted the journal refuses a plain [`open_sqlite`].

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

/// What one migration run found and did.
///
/// Returned instead of `Ok(())` because two different runs used to look
/// identical from the caller's side: a run with nothing left to apply and a
/// run that skipped everything because the store's recorded version is AHEAD
/// of the binary's chain (a binary rollback, or a module that reset its
/// migration numbering). The second one is a store the binary may not be able
/// to serve, and a module that cannot serve a newer store must refuse to start
/// on it; a module whose migrations are additive may proceed. The migrator
/// does not choose between those — refusing here would turn every binary
/// rollback into a bricked store — it reports, and the caller decides.
///
/// The fleet integration gate cannot catch the store-ahead case at all: every
/// rig opens a fresh data tree, so no module there ever meets a store newer
/// than its binary. This value is the only place the condition is visible.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct MigrationOutcome {
    /// Migrations applied by this run.
    pub applied: u32,
    /// The namespace's recorded version after the run.
    pub recorded: u32,
    /// The highest version in the chain the binary carries (0 for an empty chain).
    pub chain_max: u32,
}

impl MigrationOutcome {
    /// The store was written by a binary carrying a longer chain than this one.
    pub fn store_ahead(&self) -> bool {
        self.recorded > self.chain_max
    }
}

/// Why a store operation failed.
///
/// Non-exhaustive: a later release may add a variant without a breaking
/// version bump, so a match needs a catch-all arm.
#[derive(Debug)]
#[non_exhaustive]
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
    /// The database belongs to a different module or storage namespace.
    /// No application writes were performed. The opener had already taken the
    /// lease for its own `(module_id, storage_namespace)` (a different lease from
    /// the recorded owner's) and released it before returning this error.
    OwnershipMismatch {
        recorded_module_id: String,
        recorded_storage_namespace: String,
        requested_module_id: String,
        requested_storage_namespace: String,
    },
    /// A fenced (epoch-checked) write was rejected because the database has already
    /// been claimed by a newer writer. `db_epoch` (the epoch stamped in the
    /// database) is greater than `holder_epoch` (this store's lease epoch), so this
    /// writer has been superseded — for example a draining old instance attempting a
    /// late write after a replacement took the lease. The write was not applied.
    Fenced { holder_epoch: u64, db_epoch: u64 },
    /// The store's removals journal (`<store dir>/removals/journal`) cannot be
    /// read, or holds a damaged record before its final line, a sequence gap or a
    /// repeated sequence. The store refuses to serve rather than risk serving data
    /// a lost record had removed. `detail` names the file and the problem.
    RemovalsUnreadable { detail: String },
    /// The journal's newest record (`journal_seq`) is older than the last removal
    /// the database has applied (`store_seq`): the journal came from an older
    /// generation than the database.
    RemovalsBehindStore { journal_seq: u64, store_seq: u64 },
    /// The database has applied removals up to `store_seq`, but the removals
    /// journal is missing.
    RemovalsMissing { store_seq: u64 },
    /// The journal's header names a different `(module_id, storage_namespace)`
    /// owner from the store being opened.
    RemovalsOwnerMismatch {
        journal_module_id: String,
        journal_storage_namespace: String,
        expected_module_id: String,
        expected_storage_namespace: String,
    },
    /// The store has adopted the removals journal (its database has applied a
    /// removal, or a `removals/` directory sits beside it), so it must be opened
    /// with the module's replay handler. Nothing was served.
    RemovalsHandlerRequired,
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
            StoreError::OwnershipMismatch {
                recorded_module_id,
                recorded_storage_namespace,
                requested_module_id,
                requested_storage_namespace,
            } => write!(
                f,
                "database ownership mismatch: recorded owner ({recorded_module_id:?}, \
                 {recorded_storage_namespace:?}), requested owner ({requested_module_id:?}, \
                 {requested_storage_namespace:?})"
            ),
            StoreError::Fenced {
                holder_epoch,
                db_epoch,
            } => write!(
                f,
                "fenced write rejected: this writer holds epoch {holder_epoch} but the \
                 database was claimed by a newer writer at epoch {db_epoch}"
            ),
            StoreError::RemovalsUnreadable { detail } => write!(
                f,
                "removals journal unreadable ({detail}); the store refuses to serve: restore \
                 the newest removals journal from Engram"
            ),
            StoreError::RemovalsBehindStore {
                journal_seq,
                store_seq,
            } => write!(
                f,
                "removals journal ends at record {journal_seq} but the store has applied \
                 removals up to {store_seq}: restore the removals journal from the same or a \
                 newer generation than the store"
            ),
            StoreError::RemovalsMissing { store_seq } => write!(
                f,
                "removals journal missing, but the store has applied removals up to \
                 {store_seq}: restore the newest removals journal from Engram"
            ),
            StoreError::RemovalsOwnerMismatch {
                journal_module_id,
                journal_storage_namespace,
                expected_module_id,
                expected_storage_namespace,
            } => write!(
                f,
                "removals journal belongs to ({journal_module_id:?}, \
                 {journal_storage_namespace:?}), not ({expected_module_id:?}, \
                 {expected_storage_namespace:?}): restore this store's own removals journal, \
                 or, if the module changed its owner on purpose, run \
                 RemovalJournal::rewrite_owner under acquire_store_lease"
            ),
            StoreError::RemovalsHandlerRequired => write!(
                f,
                "this store uses a removals journal: open it with open_sqlite_with_removals \
                 (or StoreLease::open_scratch_with_removals) and the module's replay handler"
            ),
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
        sync::{Arc, Condvar, Mutex},
        time::{Duration, Instant, SystemTime, UNIX_EPOCH},
    };

    use cortexkit_lease::{protect_file, FileLeaseStore, LeaseHandle};
    use rusqlite::{Connection, OpenFlags, OptionalExtension, Transaction, TransactionBehavior};

    mod removals;
    pub use removals::{
        open_sqlite_with_removals, Removal, RemovalJournal, RemovalRecord, RemovalsHandler,
    };

    /// Decides whether `open_sqlite` creates and narrows the store's directory:
    /// only when the store's file sits in a directory of its own.
    ///
    /// Not for an in-memory database or a SQLite URI, whose "parent" is not a
    /// directory on disk, and not for a bare file name or a path with no
    /// parent: their directory is the process's working directory, which
    /// belongs to whoever started the process, so changing its mode would
    /// reach far beyond the store.
    pub(crate) fn owns_store_dir(path: &str, parent: &Path) -> bool {
        let no_own_dir = parent.as_os_str().is_empty() || parent == Path::new(".");
        !(is_memory_or_uri(path) || no_own_dir)
    }

    /// True for an in-memory database or a SQLite URI. Neither names a file
    /// on disk under this exact string, so there is no file or directory to
    /// create or protect. On Windows such a string is not even a valid file
    /// name (`:` and `?`), so treating it as a path fails outright.
    pub(crate) fn is_memory_or_uri(path: &str) -> bool {
        path == ":memory:" || path.is_empty() || path.starts_with("file:")
    }

    /// Options for [`open_sqlite_with`].
    #[derive(Debug, Clone, Copy)]
    pub struct SqliteOpenOptions {
        /// Maximum simultaneous pooled readers. Must be at least one; defaults to 4.
        pub read_pool_size: usize,
        /// SQLite lock timeout on both writer and readers, and the maximum wait
        /// for a pooled reader. Defaults to 5 seconds.
        pub busy_timeout: Duration,
    }

    impl Default for SqliteOpenOptions {
        fn default() -> Self {
            Self {
                read_pool_size: 4,
                busy_timeout: Duration::from_secs(5),
            }
        }
    }

    #[derive(Default)]
    struct ReaderState {
        idle: Vec<Connection>,
        // Includes connections being opened, so concurrent lazy opens stay bounded.
        total: usize,
    }

    struct ReadPool {
        path: String,
        options: SqliteOpenOptions,
        state: Mutex<ReaderState>,
        available: Condvar,
    }

    impl ReadPool {
        fn new(path: String, options: SqliteOpenOptions) -> Self {
            Self {
                path,
                options,
                state: Mutex::new(ReaderState::default()),
                available: Condvar::new(),
            }
        }

        fn acquire(&self) -> Result<PooledReader<'_>, StoreError> {
            let started = Instant::now();
            let mut state = self.state.lock().unwrap_or_else(|p| p.into_inner());
            loop {
                if let Some(conn) = state.idle.pop() {
                    return Ok(PooledReader {
                        pool: self,
                        conn: Some(conn),
                    });
                }
                if state.total < self.options.read_pool_size {
                    state.total += 1;
                    drop(state);
                    let opened = (|| {
                        let conn = Connection::open_with_flags(
                            &self.path,
                            OpenFlags::SQLITE_OPEN_READ_ONLY | OpenFlags::SQLITE_OPEN_URI,
                        )?;
                        conn.busy_timeout(self.options.busy_timeout)?;
                        conn.pragma_update(None, "query_only", true)?;
                        Ok::<_, rusqlite::Error>(conn)
                    })();
                    return match opened {
                        Ok(conn) => Ok(PooledReader {
                            pool: self,
                            conn: Some(conn),
                        }),
                        Err(e) => {
                            let mut state = self.state.lock().unwrap_or_else(|p| p.into_inner());
                            state.total -= 1;
                            self.available.notify_one();
                            Err(StoreError::Backend(format!("sqlite read pool open: {e}")))
                        }
                    };
                }
                let remaining = self.options.busy_timeout.saturating_sub(started.elapsed());
                if remaining.is_zero() {
                    return Err(StoreError::Backend(format!(
                        "sqlite read pool exhausted ({} readers): timed out after {:?}",
                        self.options.read_pool_size, self.options.busy_timeout
                    )));
                }
                (state, _) = self
                    .available
                    .wait_timeout(state, remaining)
                    .unwrap_or_else(|p| p.into_inner());
            }
        }
    }

    struct PooledReader<'a> {
        pool: &'a ReadPool,
        conn: Option<Connection>,
    }

    impl Drop for PooledReader<'_> {
        fn drop(&mut self) {
            let mut state = self.pool.state.lock().unwrap_or_else(|p| p.into_inner());
            let conn = self.conn.take().expect("pooled reader owns a connection");
            // Do not reuse a connection if a failed rollback left a transaction open.
            if conn.is_autocommit() {
                state.idle.push(conn);
            } else {
                state.total -= 1;
            }
            self.pool.available.notify_one();
        }
    }

    // In-memory reads share the writer. Restore its query_only setting even if
    // the read closure panics, so later with_conn writes retain their behavior.
    struct QueryOnlyGuard<'a> {
        conn: &'a Connection,
        previous: bool,
    }

    impl Drop for QueryOnlyGuard<'_> {
        fn drop(&mut self) {
            let _ = self.conn.pragma_update(None, "query_only", self.previous);
        }
    }

    fn read_transaction<T>(
        conn: &Connection,
        f: impl FnOnce(&Connection) -> rusqlite::Result<T>,
    ) -> Result<T, StoreError> {
        let tx = Transaction::new_unchecked(conn, TransactionBehavior::Deferred)
            .map_err(|e| StoreError::Backend(e.to_string()))?;
        let out = f(&tx).map_err(|e| StoreError::Backend(e.to_string()))?;
        tx.commit()
            .map_err(|e| StoreError::Backend(e.to_string()))?;
        Ok(out)
    }

    /// A lease-guarded sqlite store. Holds the single-writer lease for its lifetime,
    /// serializes writer access via [`SqliteStore::with_conn`], and offers opt-in
    /// pooled read-only snapshots via [`SqliteStore::with_read`].
    pub struct SqliteStore {
        // Read-only connections must close before the writer so SQLite can remove
        // its WAL and shared-memory files when the final connection closes.
        readers: Option<ReadPool>,
        conn: Mutex<Connection>,
        epoch: u64,
        // Where [`SqliteStore::remove`] appends, set only by
        // `open_sqlite_with_removals`. Locked after `conn`, never before.
        removals: Option<Mutex<removals::JournalWriter>>,
        // The held lease releases on drop; kept last so it remains held through
        // the writer's checkpoint and close. Shared because a scratch store opened
        // through `StoreLease::open_scratch` keeps the maintenance lease held until
        // both it and the `StoreLease` are gone.
        _lease: Arc<dyn LeaseHandle>,
    }

    impl Drop for SqliteStore {
        fn drop(&mut self) {
            // A checked-out reader borrows the store, so only idle connections can
            // remain here. Close them before checkpointing through the writer.
            drop(self.readers.take());

            let conn = self
                .conn
                .get_mut()
                .unwrap_or_else(std::sync::PoisonError::into_inner);
            // This is best-effort: a busy or failed checkpoint must never panic
            // during destruction. Closing the writer still gives SQLite its normal
            // opportunity to checkpoint and remove the sidecar files.
            let _ = conn.query_row("PRAGMA wal_checkpoint(TRUNCATE)", [], |row| {
                row.get::<_, i64>(0)
            });
        }
    }

    impl SqliteStore {
        /// The fence epoch of the held lease (strictly greater than any superseded
        /// writer's). A distributed backend would carry this on every write.
        ///
        /// Also greater than the fence epoch the database carried when it was
        /// opened: an open that finds a database stamped at or above the lease's
        /// epoch (one restored or copied without its lease files) raises the
        /// lease to one above it first (saturating at `u64::MAX`). See
        /// [`open_sqlite`].
        pub fn epoch(&self) -> u64 {
            self.epoch
        }

        /// Construct a store over an already-open connection at a chosen epoch,
        /// WITHOUT acquiring the lease. Test-only: it exists to reconstruct the
        /// two-handles-one-database state of a lease handover (an old draining writer
        /// and its replacement at different epochs), which the live lease's OS lock
        /// would otherwise make impossible to set up. Not for production — real opens
        /// go through `open_sqlite`, which holds the single-writer lease.
        #[cfg(test)]
        pub(crate) fn for_test(conn: Connection, epoch: u64) -> Self {
            #[derive(Debug)]
            struct NoLease(cortexkit_lease::LeaseKey);
            impl LeaseHandle for NoLease {
                fn epoch(&self) -> u64 {
                    0
                }
                fn key(&self) -> &cortexkit_lease::LeaseKey {
                    &self.0
                }
            }
            SqliteStore {
                readers: None,
                conn: Mutex::new(conn),
                epoch,
                removals: None,
                _lease: Arc::new(NoLease(cortexkit_lease::LeaseKey::new(
                    "test", "sqlite", "test",
                ))),
            }
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

        /// Run a closure in one read-only, DEFERRED transaction: all its statements
        /// see one consistent snapshot. Success commits; errors or panics roll back.
        ///
        /// File stores use a bounded pool, opened lazily on first use after the
        /// writer exists (and any migrations the caller has already run). If all
        /// readers are busy, this waits up to the configured busy timeout, then
        /// returns an error naming the read pool; it never falls back to the writer.
        /// Writes through a reader fail. Reads neither take the epoch fence nor
        /// create its table or run migrations.
        ///
        /// Readers see only committed data. To read your own uncommitted writes,
        /// use [`Self::with_conn`] inside that write transaction instead. A read
        /// started after a commit returns sees that commit. Long reads keep old WAL
        /// frames alive and hold back checkpoints, so keep read closures short.
        ///
        /// For `:memory:` and `file::memory:` descriptors (including URI query
        /// parameters), a separate connection cannot see the same private database.
        /// These reads instead hold the existing writer mutex, still using a
        /// read-only transaction. They cannot run beside a writer on that store.
        pub fn with_read<T>(
            &self,
            f: impl FnOnce(&Connection) -> rusqlite::Result<T>,
        ) -> Result<T, StoreError> {
            if let Some(pool) = &self.readers {
                let reader = pool.acquire()?;
                read_transaction(reader.conn.as_ref().expect("pooled connection"), f)
            } else {
                let conn = self.conn.lock().unwrap_or_else(|p| p.into_inner());
                let previous = conn
                    .pragma_query_value(None, "query_only", |row| row.get(0))
                    .map_err(|e| StoreError::Backend(e.to_string()))?;
                conn.pragma_update(None, "query_only", true)
                    .map_err(|e| StoreError::Backend(e.to_string()))?;
                let _query_only = QueryOnlyGuard {
                    conn: &conn,
                    previous,
                };
                read_transaction(&conn, f)
            }
        }

        /// Run a closure inside an epoch-FENCED write transaction: the write is
        /// rejected ([`StoreError::Fenced`]) if a newer writer has taken over the
        /// database, otherwise it commits atomically.
        ///
        /// Use this instead of [`with_conn`](SqliteStore::with_conn) for a durable write that must NOT be
        /// applied by a superseded writer — for example a credential vault's
        /// token-rotation commit, where a draining old instance must never clobber a
        /// fresh token its replacement already wrote. The OS advisory lock already
        /// prevents two *live* lease holders, but during a lease handover an old
        /// instance can briefly still hold an open connection after releasing the
        /// lock; the persisted epoch fence rejects its late writes at the database
        /// layer (the lock alone cannot).
        ///
        /// Mechanism: an IMMEDIATE transaction reads the database's stored fence
        /// epoch and, if it is greater than this store's lease epoch, rejects without
        /// applying `f` (a newer writer owns the database). Otherwise it claims the
        /// database for this epoch and runs `f`, committing atomically. Returning an
        /// error from `f` rolls the transaction back.
        ///
        /// Additive and opt-in: the fence table is created lazily on the first fenced
        /// write, so existing [`with_conn`](SqliteStore::with_conn)-only stores have a byte-identical
        /// database and are unaffected. A store mixes both paths freely.
        pub fn with_conn_fenced<T>(
            &self,
            f: impl FnOnce(&rusqlite::Transaction) -> rusqlite::Result<T>,
        ) -> Result<T, StoreError> {
            let mut guard = self.conn.lock().unwrap_or_else(|p| p.into_inner());
            let tx = guard
                .transaction_with_behavior(rusqlite::TransactionBehavior::Immediate)
                .map_err(|e| StoreError::Backend(e.to_string()))?;

            // On a refusal the transaction rolls back on drop, without applying `f`.
            claim_fence(&tx, self.epoch)?;

            let out = f(&tx).map_err(|e| StoreError::Backend(e.to_string()))?;
            tx.commit()
                .map_err(|e| StoreError::Backend(e.to_string()))?;
            Ok(out)
        }

        /// Apply a `namespace`'s migration chain to this store's database, once.
        ///
        /// Applied migrations are tracked per `(namespace, version)`, so a module
        /// that owns several domains in one database registers an INDEPENDENT chain
        /// per domain (`migrate("work_graph", ..)`, `migrate("hires", ..)`): each
        /// domain's history is separate, and adding a domain later never re-runs or
        /// entangles another's migrations. A single-domain module just calls this
        /// once. Idempotent: only un-applied versions in this namespace run.
        ///
        /// Read [`MigrationOutcome::store_ahead`] on the result: a module that
        /// cannot serve a store newer than its chain refuses startup on it,
        /// naming both versions.
        pub fn migrate(
            &self,
            namespace: &str,
            migrations: &[Migration],
        ) -> Result<MigrationOutcome, StoreError> {
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
    /// rejected (`StoreError::Lease`) without SQLite ever opening the database: the
    /// database file and its `-wal` and `-shm` siblings are not read, created or
    /// written by the refused opener.
    ///
    /// The recorded owner is checked right after the lease is taken. An opener
    /// with a different `(module_id, storage_namespace)` derives a different lease
    /// key, so its own lease succeeds; the owner check then refuses it
    /// (`StoreError::OwnershipMismatch`) and its lease is released before the
    /// error returns. That opener's lease file is left behind in the store's
    /// directory (created on first use, epoch bumped on later ones), as every
    /// lease file is; it is never deleted, since removing a lock file other
    /// processes may have open would let two of them hold "the" lock at once.
    ///
    /// The lease lives next to the database file (its parent directory), DERIVED
    /// from the descriptor's path rather than passed in. This makes the
    /// one-lease-per-database invariant structural: two distinct database paths get
    /// distinct leases (correct isolation), and the same database path gets one
    /// lease (the single-writer guarantee). A caller cannot accidentally point a
    /// shared lease directory at several distinct databases (which would falsely
    /// make them contend) or split one database across lease directories (which
    /// would break single-writer). [`acquire_store_lease`] takes this same lease
    /// for maintenance without opening the database.
    ///
    /// The database's stored fence epoch is read at the same point as the owner.
    /// When it is at or above the lease's epoch, as for a `store.db` restored or
    /// copied to a directory without its `*.lease` files, the lease's persisted
    /// epoch is raised to one above it before the store is returned, so fenced
    /// writes succeed and later opens continue above it. An epoch is never
    /// lowered. If the lease cannot be raised, the open is refused with
    /// [`StoreError::Lease`]. A database that has never taken a fenced write is
    /// not written to by this check.
    pub fn open_sqlite(descriptor: &StorageDescriptor) -> Result<SqliteStore, StoreError> {
        open_sqlite_with(descriptor, SqliteOpenOptions::default())
    }

    /// Like [`open_sqlite`], with a configurable read pool size and busy timeout.
    /// Reader connections are opened only by [`SqliteStore::with_read`], never by
    /// this function or by migrations. A zero pool size is rejected.
    pub fn open_sqlite_with(
        descriptor: &StorageDescriptor,
        options: SqliteOpenOptions,
    ) -> Result<SqliteStore, StoreError> {
        // Both checks run before anything touches the disk, so a descriptor or
        // option this build cannot serve never creates a directory or a lease.
        sqlite_path(descriptor)?;
        check_options(&options)?;

        let (path, lease) = take_store_lease(descriptor)?;
        // A store that has adopted the removals journal must replay it before
        // serving, which needs the module's handler. The directory check runs
        // before SQLite opens, so a forgotten handler never creates or touches
        // the database; `open_under_lease` then checks the database itself.
        removals::refuse_if_journal_dir(&path)?;
        open_under_lease(
            path,
            descriptor,
            options,
            Arc::from(lease),
            AppliedRemovals::Refuse,
        )
    }

    /// Whether [`open_under_lease`] refuses a database that has applied a
    /// journaled removal: plain opens do, opens that replay the journal don't.
    #[derive(Debug, Clone, Copy)]
    enum AppliedRemovals {
        Refuse,
        Allow,
    }

    /// The SQLite file path a descriptor names, or `UnsupportedBackend` for a
    /// descriptor of any other backend.
    fn sqlite_path(descriptor: &StorageDescriptor) -> Result<String, StoreError> {
        match &descriptor.backend {
            StorageBackend::Sqlite { path } => Ok(path.clone()),
            other => Err(StoreError::UnsupportedBackend(other.label().to_string())),
        }
    }

    /// The directory a SQLite file lives in; `.` for a path with no parent.
    fn parent_dir(path: &str) -> PathBuf {
        Path::new(path)
            .parent()
            .map(Path::to_path_buf)
            .unwrap_or_else(|| PathBuf::from("."))
    }

    fn check_options(options: &SqliteOpenOptions) -> Result<(), StoreError> {
        if options.read_pool_size == 0 {
            return Err(StoreError::Backend(
                "sqlite read pool size must be at least one".into(),
            ));
        }
        Ok(())
    }

    /// Create the directory a store file lives in. When the file has a directory
    /// of its own, it is created (or narrowed) owner-only; see [`owns_store_dir`].
    fn prepare_store_dir(path: &str, parent: &Path) -> Result<(), StoreError> {
        if owns_store_dir(path, parent) {
            cortexkit_lease::create_private_dir(parent).map_err(StoreError::Io)
        } else {
            std::fs::create_dir_all(parent).map_err(StoreError::Io)
        }
    }

    /// Prepare a store's directory and take its single-writer lease, without
    /// opening the database. Returns the database path with the held lease.
    ///
    /// This is the only place the lease's location and key are derived from a
    /// descriptor. [`open_sqlite_with`] and [`acquire_store_lease`] both go
    /// through it, so a maintenance tool and the module itself always contend on
    /// the same lease for the same database.
    fn take_store_lease(
        descriptor: &StorageDescriptor,
    ) -> Result<(String, Box<dyn LeaseHandle>), StoreError> {
        let path = sqlite_path(descriptor)?;
        let parent = parent_dir(&path);
        prepare_store_dir(&path, &parent)?;
        let lease = FileLeaseStore::new(&parent)
            .acquire(&lease_key(descriptor))
            .map_err(StoreError::Lease)?;
        Ok((path, lease))
    }

    /// Open the SQLite file at `path` for a caller that already holds `lease`:
    /// claim or check `owner` as the file's recorded owner, apply the durable
    /// pragmas and owner-only file modes, and return the store holding `lease`.
    ///
    /// On any error the connection closes before `lease` is dropped (locals drop
    /// before parameters), so a refused opener never holds the lease while its
    /// connection is still open.
    ///
    /// With [`AppliedRemovals::Refuse`], a database that has applied a journaled
    /// removal is refused with [`StoreError::RemovalsHandlerRequired`]. That
    /// check reads the database right after the owner check, before WAL is
    /// enabled, so it leaves the connection in the same state the owner check
    /// alone would.
    fn open_under_lease(
        path: String,
        owner: &StorageDescriptor,
        options: SqliteOpenOptions,
        lease: Arc<dyn LeaseHandle>,
        applied_removals: AppliedRemovals,
    ) -> Result<SqliteStore, StoreError> {
        let mut conn = Connection::open(&path).map_err(|e| StoreError::Backend(e.to_string()))?;
        conn.busy_timeout(options.busy_timeout)
            .map_err(|e| StoreError::Backend(e.to_string()))?;
        if !is_memory_or_uri(&path) {
            claim_owner(&mut conn, owner)?;
            if let AppliedRemovals::Refuse = applied_removals {
                // Catches a database restored without its `removals/`
                // directory, which the directory check cannot see.
                let applied = removals::read_applied_seq(&conn)
                    .map_err(|e| StoreError::Backend(e.to_string()))?;
                if applied > 0 {
                    return Err(StoreError::RemovalsHandlerRequired);
                }
            }
        }
        // Beside the owner check and before WAL is enabled, so an open refused
        // here leaves a rollback-journal database in its own journal mode.
        let epoch = raise_lease_past_fence(&conn, &path, lease.as_ref())?;

        // WAL permits concurrent readers; foreign keys protect referential integrity.
        enable_wal(&conn, options.busy_timeout).map_err(|e| StoreError::Backend(e.to_string()))?;
        conn.pragma_update(None, "foreign_keys", "ON")
            .map_err(|e| StoreError::Backend(e.to_string()))?;

        // Owner-only, decided here rather than left to the caller's umask.
        //
        // SQLite sets no mode of its own, so without this the shipped default is
        // world-readable `0644` — measured across every module store on a real
        // deployment. A crate that already decides WAL mode, busy timeout and
        // foreign keys has taken responsibility for how this file behaves on
        // disk; leaving permissions to callers means the decision is made by the
        // ambient umask, which is to say not made at all.
        //
        // The WAL and SHM siblings are the half that gets missed. Recently
        // committed rows live in the WAL until a checkpoint, so protecting only
        // the database file leaves the NEWEST data permissive while the database
        // itself reads as correct — and on real hosts the WAL is routinely
        // larger than the database.
        //
        // Ordering: after the pragma that ENABLES WAL, so the sibling files
        // exist to be protected on a first open rather than being created
        // unprotected immediately afterwards.
        //
        // A group-readable store is not a configuration this crate supports: it
        // hands out an exclusive single-writer lease, so an out-of-band reader
        // is already outside the contract. That need is a read replica or an
        // export operation, not a looser file mode.
        if !is_memory_or_uri(&path) {
            for suffix in ["", "-wal", "-shm"] {
                protect_file(Path::new(&format!("{path}{suffix}")))
                    .map_err(|e| StoreError::Backend(e.to_string()))?;
            }
        }

        Ok(SqliteStore {
            readers: if path.is_empty()
                || path == ":memory:"
                || path == "file::memory:"
                || path.starts_with("file::memory:?")
            {
                None
            } else {
                Some(ReadPool::new(path, options))
            },
            conn: Mutex::new(conn),
            epoch,
            removals: None,
            _lease: lease,
        })
    }

    /// The epoch a store opened over `conn` writes with: the lease's epoch,
    /// raised first to one above the database's stored fence epoch when the
    /// database already carries an epoch at or above the lease's.
    ///
    /// On one machine the lease epoch only grows, so it is always above any
    /// epoch it stamped into the database. A database that arrives WITHOUT its
    /// lease files (a backup restored onto a new machine, a copy placed by hand,
    /// a deleted lease file) meets a fresh lease starting at 1 while the
    /// database still carries the old fence epoch; without the raise, every
    /// fenced write would be refused until that many restarts had passed.
    ///
    /// Refuses the open with [`StoreError::Lease`] when a raise is needed and the
    /// lease cannot do it, rather than serving a store whose fenced writes would
    /// all fail. A database with no fence table reads as epoch 0 and is not
    /// written to.
    fn raise_lease_past_fence(
        conn: &Connection,
        path: &str,
        lease: &dyn LeaseHandle,
    ) -> Result<u64, StoreError> {
        let fence = read_fence_epoch(conn).map_err(|e| StoreError::Backend(e.to_string()))?;
        let epoch = lease.epoch();
        if fence < epoch {
            return Ok(epoch);
        }
        // Raising is safe. The caller holds the EXCLUSIVE lease, so no other
        // live writer holds any epoch of this lease, and the raised value is
        // persisted before the store serves, so the next holder starts above it.
        // Fencing still does its job: a lingering older writer on this machine
        // carries an epoch at or below the stored fence epoch, which is below the
        // raised one, so its fenced writes stay refused once this store stamps
        // the database. A raised epoch can only fence more writers, never fewer.
        lease
            .raise_epoch(fence.saturating_add(1))
            .map_err(|error| match error {
                LeaseError::Io(io) => StoreError::Lease(LeaseError::Io(std::io::Error::new(
                    io.kind(),
                    format!(
                        "database {path:?} carries fence epoch {fence}, at or above this \
                         lease's epoch {epoch}, and the lease could not be raised past it: {io}"
                    ),
                ))),
                other => StoreError::Lease(other),
            })
    }

    /// The epoch fence check of every fenced write ([`SqliteStore::with_conn_fenced`]
    /// and [`SqliteStore::remove`]), run inside the write's own IMMEDIATE
    /// transaction `tx` by a store holding lease epoch `epoch`.
    ///
    /// Creates the fence table if it is absent and reads the database's fence
    /// epoch. When that is above `epoch`, a newer writer owns the database: the
    /// write is refused with [`StoreError::Fenced`] and nothing is written (the
    /// caller drops `tx`, which rolls back). Otherwise, when `epoch` is higher,
    /// it is stamped as the database's fence epoch, which fences every older
    /// writer once `tx` commits. The IMMEDIATE transaction holds SQLite's write
    /// lock from this check to the commit, so no other process can stamp a
    /// higher epoch in between.
    fn claim_fence(tx: &Transaction<'_>, epoch: u64) -> Result<(), StoreError> {
        tx.execute_batch(
            "CREATE TABLE IF NOT EXISTS cortexkit_fence (\
                 id INTEGER PRIMARY KEY CHECK (id = 0), \
                 epoch INTEGER NOT NULL)",
        )
        .map_err(|e| StoreError::Backend(e.to_string()))?;

        // COALESCE over the (at most one) fence row yields 0 when unclaimed.
        let db_epoch: u64 = tx
            .query_row(
                "SELECT COALESCE((SELECT epoch FROM cortexkit_fence WHERE id = 0), 0)",
                [],
                |r| r.get::<_, i64>(0).map(|v| v as u64),
            )
            .map_err(|e| StoreError::Backend(e.to_string()))?;

        if db_epoch > epoch {
            return Err(StoreError::Fenced {
                holder_epoch: epoch,
                db_epoch,
            });
        }
        if epoch > db_epoch {
            tx.execute(
                "INSERT INTO cortexkit_fence (id, epoch) VALUES (0, ?1) \
                 ON CONFLICT(id) DO UPDATE SET epoch = excluded.epoch",
                rusqlite::params![epoch as i64],
            )
            .map_err(|e| StoreError::Backend(e.to_string()))?;
        }
        Ok(())
    }

    /// The epoch stamped by the last fenced write, or 0 when the database has
    /// never taken one. Reads only: the fence table is not created here.
    fn read_fence_epoch(conn: &Connection) -> rusqlite::Result<u64> {
        let exists: bool = conn.query_row(
            "SELECT EXISTS(SELECT 1 FROM sqlite_schema
             WHERE type = 'table' AND name = 'cortexkit_fence')",
            [],
            |row| row.get(0),
        )?;
        if !exists {
            return Ok(0);
        }
        conn.query_row(
            "SELECT COALESCE((SELECT epoch FROM cortexkit_fence WHERE id = 0), 0)",
            [],
            |row| row.get::<_, i64>(0).map(|v| v as u64),
        )
    }

    /// A store's single-writer lease, held WITHOUT the store's database open.
    ///
    /// For maintenance that must keep the module's own writer out while it works
    /// on the store from outside, for example restoring a backup: while this guard
    /// is held, [`open_sqlite`] for the same descriptor is refused with
    /// [`StoreError::Lease`], exactly as if a live writer held the store.
    ///
    /// Dropping the guard releases the lease, unless a scratch store opened
    /// through [`StoreLease::open_scratch`] is still alive: each scratch store
    /// shares the lease and keeps it held until it is dropped too. That way a
    /// scratch connection that is still open can never be writing to a file the
    /// module has meanwhile reopened as its live store.
    #[derive(Debug)]
    pub struct StoreLease {
        descriptor: StorageDescriptor,
        live_path: String,
        lease: Arc<dyn LeaseHandle>,
    }

    /// Take a store's single-writer lease without opening its database.
    ///
    /// The lease is the one [`open_sqlite`] takes for `descriptor`: same
    /// directory (the database file's parent), same key, derived by the same
    /// code. Like `open_sqlite`, it first creates the store's directory if it is
    /// missing (owner-only when the store has a directory of its own). It does
    /// not open SQLite, does not read or write `cortexkit_owner`, and does not
    /// create or touch the database file or its `-wal` and `-shm` siblings.
    ///
    /// A successful acquire, like every lease acquire, creates the lease file if
    /// it is missing and increments the epoch persisted in it; that new epoch is
    /// [`StoreLease::epoch`].
    ///
    /// A refused acquire (`StoreError::Lease`, because a live writer or another
    /// `StoreLease` holds the store) writes nothing to the lease file: the epoch
    /// is only incremented after the OS lock is won. It creates no file either,
    /// because a held lease's file and directory already exist. Its only possible
    /// change is to permissions, which are reapplied on every attempt before the
    /// lock is tried:
    /// - on Unix, the lease file is reset to mode 0600 and the store's directory
    ///   narrowed to owner-only only if something loosened them since the holder
    ///   took the lease; that changes their ctime, not their contents or mtime;
    /// - on Windows, the lease file's owner-only access list is rewritten on every
    ///   attempt, which updates its change time but not its contents or last-write
    ///   time.
    ///
    /// Only SQLite descriptors are supported; any other backend is refused with
    /// [`StoreError::UnsupportedBackend`] before anything touches the disk.
    pub fn acquire_store_lease(descriptor: &StorageDescriptor) -> Result<StoreLease, StoreError> {
        let (live_path, lease) = take_store_lease(descriptor)?;
        Ok(StoreLease {
            descriptor: descriptor.clone(),
            live_path,
            lease: Arc::from(lease),
        })
    }

    impl StoreLease {
        /// The fence epoch this acquire persisted: strictly greater than any
        /// earlier holder's, live writers included. A scratch store opened
        /// through [`StoreLease::open_scratch`] over a database stamped at or
        /// above it raises it, and this then reports the raised value.
        pub fn epoch(&self) -> u64 {
            self.lease.epoch()
        }

        /// Open a SQLite database at `path`, a different file from the live store,
        /// for preparing a replacement while the live store is held: for example a
        /// temporary copy of a backup that a restore tool migrates and verifies
        /// before moving it into place.
        ///
        /// The database gets the same treatment as [`open_sqlite_with`] gives a live
        /// store: busy timeout, WAL, foreign keys, owner-only file modes (and an
        /// owner-only directory when it has one of its own), and a read pool sized
        /// by `options`. Its owner is claimed or checked against this lease's
        /// descriptor, so a copy recorded for another module or namespace is refused
        /// with [`StoreError::OwnershipMismatch`] before it can be placed where this
        /// store's module would then refuse to open it.
        ///
        /// It takes no second lease: the returned store shares this one and carries
        /// its epoch, and the lease stays held until both are dropped. It never opens
        /// the live store's database, so nothing is claimed or written there. Like
        /// [`open_sqlite`], it raises the shared lease's epoch past the scratch
        /// database's fence epoch when that is at or above it, so fenced writes on
        /// a copy from another machine succeed.
        ///
        /// Refused with [`StoreError::Backend`] when `path` is not valid UTF-8, is an
        /// in-memory database or a SQLite URI, or names the live store's database
        /// (the same path, or the same file reached through a symlink, or on Unix
        /// through a hard link). Drop the scratch store before moving its file into
        /// place, so that its connection closes; dropping a store attempts to
        /// checkpoint its WAL into the main file and SQLite removes the `-wal` and
        /// `-shm` when the last connection closes cleanly.
        ///
        /// Refused with [`StoreError::RemovalsHandlerRequired`] when the live store
        /// has a `removals/` directory or the copy has applied a removal: such a
        /// copy must go through [`StoreLease::open_scratch_with_removals`], which
        /// replays the live journal onto it.
        pub fn open_scratch(
            &self,
            path: impl AsRef<Path>,
            options: SqliteOpenOptions,
        ) -> Result<SqliteStore, StoreError> {
            let path = self.scratch_path(path.as_ref(), &options)?;
            // The live store's journal decides, as in `open_sqlite_with`: a copy
            // prepared without the handler would be placed with resurrected rows.
            removals::refuse_if_journal_dir(&self.live_path)?;
            self.open_scratch_at(path, options, AppliedRemovals::Refuse)
        }

        /// Validates a scratch path for [`StoreLease::open_scratch`] and
        /// [`StoreLease::open_scratch_with_removals`]: valid UTF-8, a file path,
        /// not the live store's database. Also checks `options`.
        fn scratch_path(
            &self,
            path: &Path,
            options: &SqliteOpenOptions,
        ) -> Result<String, StoreError> {
            let path = path.to_str().map(str::to_owned).ok_or_else(|| {
                StoreError::Backend(format!(
                    "scratch database path {} is not valid UTF-8",
                    path.display()
                ))
            })?;
            check_options(options)?;
            if is_memory_or_uri(&path) {
                return Err(StoreError::Backend(format!(
                    "scratch database {path:?} must be a file path, not an in-memory \
                     database or a SQLite URI"
                )));
            }
            if names_same_file(&path, &self.live_path) {
                return Err(StoreError::Backend(format!(
                    "scratch database {path:?} is the live store's database {:?}",
                    self.live_path
                )));
            }
            Ok(path)
        }

        /// Opens a validated scratch path under this lease.
        fn open_scratch_at(
            &self,
            path: String,
            options: SqliteOpenOptions,
            applied_removals: AppliedRemovals,
        ) -> Result<SqliteStore, StoreError> {
            prepare_store_dir(&path, &parent_dir(&path))?;
            open_under_lease(
                path,
                &self.descriptor,
                options,
                Arc::clone(&self.lease),
                applied_removals,
            )
        }
    }

    /// Whether two SQLite paths name the same file: the same string, the same
    /// resolved location (through symlinks), or on Unix the same inode (through a
    /// hard link). A path that does not exist yet is compared by its resolved
    /// directory and file name.
    fn names_same_file(a: &str, b: &str) -> bool {
        if a == b {
            return true;
        }
        let (a, b) = (Path::new(a), Path::new(b));
        #[cfg(unix)]
        if let (Ok(a), Ok(b)) = (std::fs::metadata(a), std::fs::metadata(b)) {
            use std::os::unix::fs::MetadataExt;
            return (a.dev(), a.ino()) == (b.dev(), b.ino());
        }
        fn resolve(path: &Path) -> Option<PathBuf> {
            if let Ok(resolved) = std::fs::canonicalize(path) {
                return Some(resolved);
            }
            let dir = match path.parent() {
                Some(dir) if !dir.as_os_str().is_empty() => dir,
                _ => Path::new("."),
            };
            Some(std::fs::canonicalize(dir).ok()?.join(path.file_name()?))
        }
        matches!((resolve(a), resolve(b)), (Some(a), Some(b)) if a == b)
    }

    /// Switch to WAL, retrying while another connection briefly holds a lock.
    ///
    /// Leaving rollback-journal mode needs an exclusive lock, and when another
    /// connection holds a write lock SQLite reports busy at once instead of
    /// running the busy handler, to avoid a deadlock. That happens on a fresh
    /// database when a second opener under another owner is still inside its
    /// claim transaction, about to be refused: without a retry, the opener that
    /// rightly won the claim would fail to start. Retries stop at the caller's
    /// busy timeout.
    fn enable_wal(conn: &Connection, busy_timeout: Duration) -> rusqlite::Result<()> {
        let started = Instant::now();
        loop {
            match conn.pragma_update(None, "journal_mode", "WAL") {
                Err(rusqlite::Error::SqliteFailure(error, _))
                    if matches!(
                        error.code,
                        rusqlite::ErrorCode::DatabaseBusy | rusqlite::ErrorCode::DatabaseLocked
                    ) && started.elapsed() < busy_timeout =>
                {
                    std::thread::sleep(Duration::from_millis(5));
                }
                result => return result,
            }
        }
    }

    /// The recorded `(module_id, storage_namespace)` owner, or `None` when the
    /// database has never been claimed.
    fn read_owner(conn: &Connection) -> rusqlite::Result<Option<(String, String)>> {
        let exists: bool = conn.query_row(
            "SELECT EXISTS(SELECT 1 FROM sqlite_schema
             WHERE type = 'table' AND name = 'cortexkit_owner')",
            [],
            |row| row.get(0),
        )?;
        if !exists {
            return Ok(None);
        }
        conn.query_row(
            "SELECT module_id, storage_namespace FROM cortexkit_owner WHERE id = 1",
            [],
            |row| Ok((row.get(0)?, row.get(1)?)),
        )
        .optional()
    }

    fn check_owner(
        (module_id, storage_namespace): (String, String),
        descriptor: &StorageDescriptor,
    ) -> Result<(), StoreError> {
        if module_id != descriptor.module_id || storage_namespace != descriptor.storage_namespace {
            return Err(StoreError::OwnershipMismatch {
                recorded_module_id: module_id,
                recorded_storage_namespace: storage_namespace,
                requested_module_id: descriptor.module_id.clone(),
                requested_storage_namespace: descriptor.storage_namespace.clone(),
            });
        }
        Ok(())
    }

    fn claim_owner(
        conn: &mut Connection,
        descriptor: &StorageDescriptor,
    ) -> Result<(), StoreError> {
        let backend = |e: rusqlite::Error| StoreError::Backend(e.to_string());
        // Almost every open finds an owner already recorded. Check that with a plain
        // read first, so it never waits for the write lock. An opener under another
        // namespace holds a lease of its own and gets this far even while the owner
        // is writing; behind a busy writer it would otherwise time out as a backend
        // error instead of reporting the ownership mismatch.
        if let Some(recorded) = read_owner(conn).map_err(backend)? {
            return check_owner(recorded, descriptor);
        }
        // No owner yet. Serialize the first claim with every other writer open, not
        // just openers whose descriptors happen to share a lease key, and look again
        // inside the transaction, since another opener may have claimed it meanwhile.
        // IMMEDIATE takes SQLite's write lock when the transaction begins, so the
        // second look and the insert are one atomic step: of two openers under
        // different namespaces racing on an unowned database (each holding its own
        // lease), the second to get the lock sees the first one's row and is
        // refused. The single-row primary key would reject a second insert anyway.
        let tx = conn
            .transaction_with_behavior(TransactionBehavior::Immediate)
            .map_err(backend)?;
        if let Some(recorded) = read_owner(&tx).map_err(backend)? {
            check_owner(recorded, descriptor)?;
            return tx.commit().map_err(backend);
        } else {
            tx.execute_batch(
                "CREATE TABLE IF NOT EXISTS cortexkit_owner (
                id INTEGER PRIMARY KEY CHECK (id = 1),
                module_id TEXT NOT NULL,
                storage_namespace TEXT NOT NULL
             );",
            )
            .map_err(backend)?;
            tx.execute(
                "INSERT INTO cortexkit_owner (id, module_id, storage_namespace) VALUES (1, ?1, ?2)",
                rusqlite::params![descriptor.module_id, descriptor.storage_namespace],
            )
            .map_err(backend)?;
        }
        tx.commit().map_err(backend)?;

        // Publish a first claim in the main file when possible, since a hard-link
        // alias has a different WAL file name. A pinned read snapshot can delay
        // this checkpoint; it must not prevent the owning writer from starting.
        let checkpoint: rusqlite::Result<(i64, i64, i64)> =
            conn.query_row("PRAGMA wal_checkpoint(FULL)", [], |row| {
                Ok((row.get(0)?, row.get(1)?, row.get(2)?))
            });
        match checkpoint {
            Ok((0, frames, checkpointed)) if frames == checkpointed => {}
            Ok((busy, frames, checkpointed)) => eprintln!(
                "warning: SQLite owner ({:?}, {:?}) was recorded, but its first checkpoint \
                 was busy or incomplete (busy={busy}, frames={frames}, checkpointed={checkpointed}); \
                 continuing writer open, hard-link aliases may not see ownership until the next checkpoint",
                descriptor.module_id, descriptor.storage_namespace,
            ),
            Err(error) => eprintln!(
                "warning: SQLite owner ({:?}, {:?}) was recorded, but its first checkpoint \
                 failed ({error}); continuing writer open, hard-link aliases may not see ownership \
                 until the next checkpoint",
                descriptor.module_id, descriptor.storage_namespace,
            ),
        }
        Ok(())
    }

    /// Apply un-applied migrations for one `namespace` in ascending version order,
    /// each in its own transaction together with its version record, so a migration
    /// and the record that it ran commit atomically (a crash mid-migration leaves
    /// it un-recorded and it re-runs cleanly next open).
    ///
    /// Applied migrations are keyed by `(namespace, version)`, so independent
    /// domain chains in one database never collide or re-run each other.
    ///
    /// Deliberate policy: a binary OLDER than the store it opens is not
    /// refused here — migrations at or below the recorded version are skipped
    /// and the run returns `Ok`. Refusing would turn every binary rollback
    /// into a bricked store, and rollbacks are a supported recovery path. What
    /// the run does NOT do is hide that it skipped: the returned
    /// [`MigrationOutcome`] reports the recorded version against the chain's
    /// highest, and the caller — who knows whether its migrations are
    /// additive — decides whether to serve. The supervisor-side comparison
    /// of a module's manifest-declared `store_schema_version` against the
    /// store is still consumed by nothing; this value is the module-side
    /// half of that job.
    fn run_migrations(
        conn: &mut Connection,
        namespace: &str,
        migrations: &[Migration],
    ) -> Result<MigrationOutcome, StoreError> {
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
        let chain_max = ordered.last().map_or(0, |m| m.version);
        let mut applied = 0u32;

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
            applied += 1;
        }
        Ok(MigrationOutcome {
            applied,
            recorded: current.max(chain_max),
            chain_max,
        })
    }

    fn now_unix() -> i64 {
        SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map(|d| d.as_secs() as i64)
            .unwrap_or(0)
    }

    #[cfg(test)]
    mod ownership_read_tests {
        use super::*;

        #[test]
        fn readonly_handles_with_another_identity_never_claim_or_refuse_ownership() {
            struct Cleanup(PathBuf);
            impl Drop for Cleanup {
                fn drop(&mut self) {
                    let _ = std::fs::remove_dir_all(&self.0);
                }
            }
            let root = std::env::temp_dir().join(format!(
                "cortexkit-store-read-owner-{}-{}",
                std::process::id(),
                SystemTime::now()
                    .duration_since(UNIX_EPOCH)
                    .unwrap()
                    .as_nanos()
            ));
            let _cleanup = Cleanup(root.clone());
            std::fs::create_dir_all(&root).unwrap();
            let path = root.join("store.db").to_string_lossy().into_owned();
            let legacy = Connection::open(&path).unwrap();
            legacy
                .execute_batch("CREATE TABLE points (value INTEGER); INSERT INTO points VALUES (9)")
                .unwrap();
            drop(legacy);

            let descriptor = StorageDescriptor {
                module_id: "owner-module".into(),
                storage_namespace: "owner-space".into(),
                isolation: Isolation::Module,
                backend: StorageBackend::Sqlite { path: path.clone() },
            };
            let mut readers = Vec::new();
            for pooled in [false, true] {
                let conn =
                    Connection::open_with_flags(&path, OpenFlags::SQLITE_OPEN_READ_ONLY).unwrap();
                let mut reader = SqliteStore::for_test(conn, 0);
                assert_ne!(reader._lease.key(), &lease_key(&descriptor));
                if pooled {
                    reader.readers =
                        Some(ReadPool::new(path.clone(), SqliteOpenOptions::default()));
                }
                assert_eq!(
                    reader
                        .with_read(|tx| tx.query_row(
                            "SELECT count(*) FROM sqlite_schema WHERE name = 'cortexkit_owner'",
                            [],
                            |row| row.get::<_, i64>(0)
                        ))
                        .unwrap(),
                    0
                );
                readers.push(reader);
            }
            let writer = open_sqlite(&descriptor).unwrap();
            for reader in &readers {
                assert_eq!(
                    reader
                        .with_read(|tx| tx
                            .query_row("SELECT value FROM points", [], |row| row.get::<_, i64>(0)))
                        .unwrap(),
                    9
                );
                assert_eq!(
                    reader
                        .with_read(|tx| tx.query_row(
                            "SELECT module_id, storage_namespace FROM cortexkit_owner WHERE id = 1",
                            [],
                            |row| Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?))
                        ))
                        .unwrap(),
                    (
                        descriptor.module_id.clone(),
                        descriptor.storage_namespace.clone()
                    )
                );
            }
            drop(writer);
            drop(readers);
        }
    }

    /// A `store.db` that arrives without the lease files it was written under,
    /// on another machine or after the lease files were lost, must open and take
    /// fenced writes at once, not after as many restarts as its stored fence
    /// epoch. The persisted lease epoch is observed by opening again or through
    /// `acquire_store_lease`, never by reading a `*.lease` file, which Windows
    /// refuses while the lease is locked.
    #[cfg(test)]
    mod fence_raise_tests {
        use super::*;

        const KV: &[Migration] = &[Migration {
            version: 1,
            statements: "CREATE TABLE kv (k TEXT PRIMARY KEY, v TEXT NOT NULL);",
        }];

        /// A fresh, unique directory, removed when the guard drops.
        struct TempDir(PathBuf);

        impl TempDir {
            fn new(label: &str) -> Self {
                use std::sync::atomic::{AtomicU64, Ordering};
                static NEXT: AtomicU64 = AtomicU64::new(0);
                let dir = std::env::temp_dir().join(format!(
                    "cortexkit-store-fence-{label}-{}-{}-{}",
                    std::process::id(),
                    SystemTime::now()
                        .duration_since(UNIX_EPOCH)
                        .unwrap()
                        .as_nanos(),
                    NEXT.fetch_add(1, Ordering::Relaxed)
                ));
                std::fs::create_dir_all(&dir).unwrap();
                Self(dir)
            }

            fn db(&self, name: &str) -> PathBuf {
                self.0.join(name)
            }
        }

        impl Drop for TempDir {
            fn drop(&mut self) {
                let _ = std::fs::remove_dir_all(&self.0);
            }
        }

        fn descriptor(db: &Path) -> StorageDescriptor {
            StorageDescriptor {
                module_id: "fence-module".into(),
                storage_namespace: "main".into(),
                isolation: Isolation::Module,
                backend: StorageBackend::Sqlite {
                    path: db.to_string_lossy().into_owned(),
                },
            }
        }

        fn fenced_put(store: &SqliteStore, key: &str) -> Result<(), StoreError> {
            store.with_conn_fenced(|tx| {
                tx.execute(
                    "INSERT INTO kv (k, v) VALUES (?1, 'v') \
                     ON CONFLICT(k) DO UPDATE SET v = excluded.v",
                    [key],
                )
                .map(|_| ())
            })
        }

        /// The fence epoch stored in a closed database file.
        fn stored_fence(db: &Path) -> u64 {
            read_fence_epoch(&Connection::open(db).unwrap()).unwrap()
        }

        fn lease_files(dir: &Path) -> Vec<PathBuf> {
            std::fs::read_dir(dir)
                .unwrap()
                .map(|entry| entry.unwrap().path())
                .filter(|path| path.extension().is_some_and(|ext| ext == "lease"))
                .collect()
        }

        /// Write a store the normal way, opening it `opens` times with a fenced
        /// write each time, so its stored fence epoch ends at `opens`. Returns
        /// the closed database's path.
        fn store_fenced_at(dir: &TempDir, opens: u64) -> PathBuf {
            let db = dir.db("store.db");
            for open in 1..=opens {
                let store = open_sqlite(&descriptor(&db)).unwrap();
                assert_eq!(store.epoch(), open);
                store.migrate("kv", KV).unwrap();
                fenced_put(&store, "old").unwrap();
            }
            assert_eq!(stored_fence(&db), opens);
            db
        }

        /// Copy only the database file into a fresh directory with no lease
        /// files, as a backup restored onto another machine arrives.
        fn copy_without_leases(from: &Path, to: &TempDir, name: &str) -> PathBuf {
            let copy = to.db(name);
            std::fs::copy(from, &copy).unwrap();
            assert!(lease_files(&to.0).is_empty());
            copy
        }

        /// The promise: a copy carrying fence epoch 3 opens at epoch 4, takes
        /// fenced writes, and the raised epoch is persisted, so the next
        /// acquisition is at 5. Without the raise, the first fenced write is
        /// refused with `Fenced { holder_epoch: 1, db_epoch: 3 }`.
        #[test]
        fn a_store_copied_without_its_lease_files_opens_and_writes() {
            let old = TempDir::new("old-machine");
            let original = store_fenced_at(&old, 3);
            let new = TempDir::new("new-machine");
            let copy = copy_without_leases(&original, &new, "store.db");

            let store = open_sqlite(&descriptor(&copy)).unwrap();
            match fenced_put(&store, "new") {
                Ok(()) => {}
                Err(error) => panic!("a fenced write on the copy was refused: {error:?}"),
            }
            assert_eq!(store.epoch(), 4, "one above the copy's fence epoch");
            drop(store);
            assert_eq!(stored_fence(&copy), 4);

            // Reopening alone cannot prove persistence: it would raise again
            // from the stored fence epoch. Taking the lease without opening the
            // database shows what the lease file holds.
            let next = acquire_store_lease(&descriptor(&copy)).unwrap();
            assert_eq!(next.epoch(), 5, "the raised epoch was persisted");
            drop(next);
            let reopened = open_sqlite(&descriptor(&copy)).unwrap();
            assert_eq!(reopened.epoch(), 6);
            fenced_put(&reopened, "again").unwrap();
        }

        /// Raising does not weaken the fence: once the raised store has
        /// written, a writer still holding the old machine's last epoch, or the
        /// epoch the fresh lease started at, is refused.
        #[test]
        fn after_a_raise_an_older_writer_is_still_fenced() {
            let old = TempDir::new("old-writer");
            let original = store_fenced_at(&old, 3);
            let new = TempDir::new("raised");
            let copy = copy_without_leases(&original, &new, "store.db");

            let store = open_sqlite(&descriptor(&copy)).unwrap();
            assert_eq!(store.epoch(), 4);
            fenced_put(&store, "new").unwrap();

            for stale_epoch in [3, 1] {
                let stale = SqliteStore::for_test(Connection::open(&copy).unwrap(), stale_epoch);
                match fenced_put(&stale, "new") {
                    Err(StoreError::Fenced {
                        holder_epoch,
                        db_epoch,
                    }) => assert_eq!((holder_epoch, db_epoch), (stale_epoch, 4)),
                    other => {
                        panic!("a writer at epoch {stale_epoch} must be fenced, got {other:?}")
                    }
                }
            }
        }

        /// A scratch store shares its `StoreLease`'s lease, so a scratch copy
        /// carrying a higher fence epoch raises that shared lease: the scratch
        /// takes fenced writes, the guard reports the raised epoch, and the
        /// module's next open of the live store continues above it.
        #[test]
        fn a_scratch_copy_with_a_higher_fence_epoch_takes_fenced_writes() {
            let old = TempDir::new("scratch-source");
            let original = store_fenced_at(&old, 3);
            let new = TempDir::new("scratch");
            let live = descriptor(&new.db("store.db"));
            let copy = copy_without_leases(&original, &new, "restore.db");

            let lease = acquire_store_lease(&live).unwrap();
            assert_eq!(lease.epoch(), 1);
            let scratch = lease
                .open_scratch(&copy, SqliteOpenOptions::default())
                .unwrap();
            assert_eq!(scratch.epoch(), 4);
            assert_eq!(lease.epoch(), 4, "the shared lease was raised");
            match fenced_put(&scratch, "restored") {
                Ok(()) => {}
                Err(error) => panic!("a fenced write on the scratch copy was refused: {error}"),
            }
            drop(scratch);
            drop(lease);
            assert_eq!(stored_fence(&copy), 4);

            let next = acquire_store_lease(&live).unwrap();
            assert_eq!(next.epoch(), 5, "the raise was persisted");
            drop(next);
            let reopened = open_sqlite(&live).unwrap();
            assert_eq!(reopened.epoch(), 6);
        }

        /// Same machine, lease files lost: the reopened store takes fenced
        /// writes, and the raise is persisted by the open itself. Persistence is
        /// checked through `acquire_store_lease` after the store closes, which
        /// bumps the lease file's epoch without opening the database.
        #[test]
        fn a_store_whose_lease_files_were_deleted_reopens_and_writes() {
            let dir = TempDir::new("lost-lease");
            let db = store_fenced_at(&dir, 3);
            let leases = lease_files(&dir.0);
            assert!(!leases.is_empty());
            for lease in leases {
                std::fs::remove_file(lease).unwrap();
            }

            let store = open_sqlite(&descriptor(&db)).unwrap();
            assert_eq!(store.epoch(), 4);
            match fenced_put(&store, "after-loss") {
                Ok(()) => {}
                Err(error) => panic!("a fenced write after losing the lease was refused: {error}"),
            }
            drop(store);

            let next = acquire_store_lease(&descriptor(&db)).unwrap();
            assert_eq!(next.epoch(), 5, "the open persisted its raised epoch");
            drop(next);
            let reopened = open_sqlite(&descriptor(&db)).unwrap();
            assert!(reopened.epoch() > 5);
            fenced_put(&reopened, "again").unwrap();
        }

        /// A lease implementation that cannot raise its epoch makes the open
        /// fail with a clear error instead of returning a store whose fenced
        /// writes would all be refused. The same lease opens a database that
        /// needs no raise.
        #[test]
        fn a_lease_that_cannot_raise_refuses_the_open() {
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
            let fixed = || -> Arc<dyn LeaseHandle> {
                Arc::new(FixedLease(LeaseKey::new("fence-module", "sqlite", "main")))
            };

            let old = TempDir::new("fixed-source");
            let original = store_fenced_at(&old, 3);
            let new = TempDir::new("fixed");
            let copy = copy_without_leases(&original, &new, "store.db");
            let path = copy.to_string_lossy().into_owned();
            match open_under_lease(
                path,
                &descriptor(&copy),
                SqliteOpenOptions::default(),
                fixed(),
                AppliedRemovals::Refuse,
            ) {
                Err(StoreError::Lease(LeaseError::Io(error))) => {
                    assert_eq!(error.kind(), std::io::ErrorKind::Unsupported);
                    let message = error.to_string();
                    assert!(message.contains("fence epoch 3"), "{message}");
                }
                Err(other) => panic!("expected an unsupported lease error, got {other}"),
                Ok(_) => panic!("a lease that cannot raise must refuse the open"),
            }

            let fresh = new.db("fresh.db");
            let store = open_under_lease(
                fresh.to_string_lossy().into_owned(),
                &descriptor(&fresh),
                SqliteOpenOptions::default(),
                fixed(),
                AppliedRemovals::Refuse,
            )
            .unwrap();
            assert_eq!(store.epoch(), 1);
        }
    }
}

#[cfg(feature = "sqlite")]
pub use sqlite_backend::{
    acquire_store_lease, open_sqlite, open_sqlite_with, open_sqlite_with_removals, Removal,
    RemovalJournal, RemovalRecord, RemovalsHandler, SqliteOpenOptions, SqliteStore, StoreLease,
};

#[cfg(all(test, feature = "sqlite"))]
mod tests {
    use super::*;

    /// The database file AND its WAL sibling are owner-only on disk after
    /// `open_sqlite`, including a database that already exists permissively.
    ///
    /// Two properties, asserted separately because they fail independently.
    /// SQLite sets no mode, so without this the shipped default is `0644`.
    /// And the WAL is the half that gets missed: recently committed rows live
    /// there until a checkpoint, so protecting only the database leaves the
    /// NEWEST data readable while the database file itself looks correct.
    ///
    /// Asserts the mode ON DISK rather than that `open_sqlite` returned Ok — a
    /// hardening step that silently did nothing would still return Ok.
    ///
    /// The scenario is REOPENING an already-deployed store, which is the only
    /// shape in which the WAL assertion means anything. A first open cannot
    /// exercise it: SQLite creates a fresh WAL inheriting the database's mode,
    /// so by then the database is already `0600` and the WAL follows for free.
    /// A permissive WAL only exists because a PREVIOUS process wrote one under
    /// the old umask — exactly the state every deployed machine is in.
    ///
    /// Mutation-proved, both suffixes independently: dropping `""` fails the
    /// database assertion, dropping `"-wal"` fails the WAL assertion, and
    /// neither is carried by the other. An earlier version of this test used a
    /// first open and the `"-wal"` mutation SURVIVED it — the assertion was
    /// there, it just could not fail.
    #[cfg(unix)]
    #[test]
    fn reopening_a_permissive_store_protects_the_database_and_its_wal() {
        use std::os::unix::fs::PermissionsExt;

        let (root, descriptor) = tmp();
        let StorageBackend::Sqlite { path } = &descriptor.backend else {
            panic!("sqlite descriptor");
        };
        let path = std::path::PathBuf::from(path);
        let wal = std::path::PathBuf::from(format!("{}-wal", path.display()));

        // First open: create a real database, then close it.
        {
            let store = open_sqlite(&descriptor).expect("first open");
            store
                .migrate(
                    "perm",
                    &[Migration {
                        version: 1,
                        statements: "CREATE TABLE t (k TEXT);",
                    }],
                )
                .expect("migrate");
        }

        // A clean close checkpoints and REMOVES the WAL, so one has to be put
        // back deliberately. That is not artificial: a WAL surviving on disk is
        // precisely what an unclean shutdown leaves behind, and it is the only
        // state in which a permissive WAL can be waiting at open time.
        std::fs::write(&wal, b"").expect("leave a WAL behind");

        // Reproduce the deployed state: both files permissive, as every store
        // created before this hardening actually is on disk.
        for file in [&path, &wal] {
            std::fs::set_permissions(file, std::fs::Permissions::from_mode(0o644))
                .expect("set permissive mode");
        }

        let store = open_sqlite(&descriptor).expect("reopen");

        let mode = |p: &std::path::Path| {
            std::fs::metadata(p)
                .unwrap_or_else(|error| panic!("stat {}: {error}", p.display()))
                .permissions()
                .mode()
                & 0o777
        };
        assert_eq!(
            mode(&path),
            0o600,
            "the database stayed group/world readable on reopen"
        );
        assert_eq!(
            mode(&wal),
            0o600,
            "the WAL stayed group/world readable while the database looked correct"
        );

        drop(store);
        let _ = std::fs::remove_dir_all(&root);
    }

    #[cfg(unix)]
    fn dir_mode(dir: &std::path::Path) -> u32 {
        use std::os::unix::fs::PermissionsExt;
        std::fs::symlink_metadata(dir)
            .unwrap_or_else(|error| panic!("stat {}: {error}", dir.display()))
            .permissions()
            .mode()
            & 0o777
    }

    #[cfg(unix)]
    fn descriptor_at(db: &std::path::Path) -> StorageDescriptor {
        StorageDescriptor {
            module_id: "test-module".into(),
            storage_namespace: "main".into(),
            isolation: Isolation::Module,
            backend: StorageBackend::Sqlite {
                path: db.to_string_lossy().into_owned(),
            },
        }
    }

    /// Every directory the first open creates is owner-only, not only the
    /// store's own: a missing `data/module` pair made by `create_dir_all` would
    /// leave `data` at 0755.
    #[cfg(unix)]
    #[test]
    fn a_fresh_store_creates_its_directories_owner_only() {
        let (root, _) = tmp();
        let module_dir = root.join("data").join("module");
        let store = open_sqlite(&descriptor_at(&module_dir.join("store.db"))).expect("open");
        assert_eq!(
            dir_mode(&root.join("data")),
            0o700,
            "a created parent was left group/world accessible"
        );
        assert_eq!(
            dir_mode(&module_dir),
            0o700,
            "the store directory was created group/world accessible"
        );
        drop(store);
        let _ = std::fs::remove_dir_all(&root);
    }

    /// A store directory an older build left at 0755 is tightened on open, and
    /// the directory above it, which the store does not own, is not touched.
    #[cfg(unix)]
    #[test]
    fn reopening_tightens_the_store_directory_but_not_its_parent() {
        use std::os::unix::fs::PermissionsExt;
        let (root, _) = tmp();
        let module_dir = root.join("module");
        std::fs::create_dir_all(&module_dir).expect("mkdir");
        for dir in [&root, &module_dir] {
            std::fs::set_permissions(dir, std::fs::Permissions::from_mode(0o755)).expect("chmod");
        }
        let store = open_sqlite(&descriptor_at(&module_dir.join("store.db"))).expect("open");
        assert_eq!(
            dir_mode(&module_dir),
            0o700,
            "an existing 0755 store directory stayed open"
        );
        assert_eq!(
            dir_mode(&root),
            0o755,
            "a directory above the store was changed"
        );
        drop(store);
        let _ = std::fs::remove_dir_all(&root);
    }

    /// A store directory whose mode can't be changed (here, one carrying the
    /// user-immutable flag, which refuses chmod) is reported, not treated as a
    /// failure: the files inside are owner-only regardless.
    #[cfg(target_os = "macos")]
    #[test]
    fn a_directory_that_cannot_be_narrowed_is_not_an_error() {
        use std::os::unix::fs::PermissionsExt;
        let (root, _) = tmp();
        let module_dir = root.join("module");
        std::fs::create_dir_all(&module_dir).expect("mkdir");
        std::fs::set_permissions(&module_dir, std::fs::Permissions::from_mode(0o755))
            .expect("chmod");
        let flag = |op: &str| {
            std::process::Command::new("chflags")
                .args([op, module_dir.to_str().expect("utf-8 path")])
                .status()
                .expect("chflags")
                .success()
        };
        assert!(flag("uchg"), "could not set the immutable flag");
        let result = cortexkit_lease::create_private_dir(&module_dir);
        let narrowed = dir_mode(&module_dir);
        assert!(flag("nouchg"), "could not clear the immutable flag");
        assert_eq!(
            narrowed, 0o755,
            "the immutable flag did not block the chmod, so this test proves nothing"
        );
        assert!(
            result.is_ok(),
            "a failed narrowing was returned as an error: {:?}",
            result.err()
        );
        let _ = std::fs::remove_dir_all(&root);
    }

    /// An in-memory store opens: it has no directory, and the directory step
    /// must not try to inspect or change one.
    #[test]
    fn an_in_memory_store_opens() {
        // An in-memory store's lease lands in the working directory, because its
        // path has no parent, so remove the lease this test creates.
        let leases = || -> std::collections::BTreeSet<std::path::PathBuf> {
            std::fs::read_dir(".")
                .map(|dir| {
                    dir.flatten()
                        .map(|entry| entry.path())
                        .filter(|path| path.extension().is_some_and(|ext| ext == "lease"))
                        .collect()
                })
                .unwrap_or_default()
        };
        let before = leases();
        for path in [":memory:", "file::memory:", "file::memory:?cache=shared"] {
            let store = open_sqlite(&StorageDescriptor {
                module_id: "test-module".into(),
                storage_namespace: "memory".into(),
                isolation: Isolation::Module,
                backend: StorageBackend::Sqlite { path: path.into() },
            })
            .expect("in-memory open");
            store
                .with_conn(|conn| {
                    conn.execute_batch("CREATE TABLE memory_values (v INTEGER); INSERT INTO memory_values VALUES (10)")
                })
                .unwrap();
            let value: i64 = store
                .with_read(|conn| {
                    assert!(!conn.is_autocommit());
                    conn.query_row("SELECT v FROM memory_values", [], |row| row.get(0))
                })
                .unwrap();
            assert_eq!(value, 10, "{path}");
            assert!(store
                .with_read(|conn| conn.execute("UPDATE memory_values SET v = 99", []))
                .is_err());
            let value: i64 = store
                .with_read(|conn| {
                    conn.query_row("SELECT v FROM memory_values", [], |row| row.get(0))
                })
                .unwrap();
            assert_eq!(value, 10);
            store
                .with_conn(|conn| conn.execute("UPDATE memory_values SET v = 20", []))
                .expect("memory read restores writable writer");
        }
        for lease in leases().difference(&before) {
            let _ = std::fs::remove_file(lease);
        }
    }

    /// Only a store with a directory of its own gets it narrowed; the working
    /// directory behind a bare file name, an in-memory database and a URI are
    /// never touched.
    #[test]
    fn only_a_real_store_directory_is_narrowed() {
        use sqlite_backend::owns_store_dir;
        use std::path::Path;
        let parent = |p: &str| {
            Path::new(p)
                .parent()
                .map(Path::to_path_buf)
                .unwrap_or_default()
        };
        assert!(!owns_store_dir(":memory:", &parent(":memory:")));
        assert!(!owns_store_dir("store.db", &parent("store.db")));
        assert!(!owns_store_dir("./store.db", &parent("./store.db")));
        assert!(!owns_store_dir(
            "file:/x/store.db?mode=rwc",
            &parent("file:/x/store.db?mode=rwc")
        ));
        assert!(owns_store_dir(
            "/data/module/store.db",
            &parent("/data/module/store.db")
        ));
        assert!(owns_store_dir(
            "data/module/store.db",
            &parent("data/module/store.db")
        ));
    }

    /// When the store directory is a symlink, the store still opens and the
    /// symlink's target keeps its mode: permissions are never set through a link.
    #[cfg(unix)]
    #[test]
    fn a_symlinked_store_directory_is_left_alone() {
        use std::os::unix::fs::PermissionsExt;
        let (root, _) = tmp();
        let target = root.join("target");
        std::fs::create_dir_all(&target).expect("mkdir");
        std::fs::set_permissions(&target, std::fs::Permissions::from_mode(0o755)).expect("chmod");
        let link = root.join("link");
        std::os::unix::fs::symlink(&target, &link).expect("symlink");
        let store =
            open_sqlite(&descriptor_at(&link.join("store.db"))).expect("open through a symlink");
        assert_eq!(dir_mode(&target), 0o755, "the symlink's target was changed");
        drop(store);
        let _ = std::fs::remove_dir_all(&root);
    }

    /// A unique temp root + a descriptor whose sqlite file lives under it. The
    /// lease is derived from the db path (its parent), so no separate lease dir.
    fn tmp() -> (std::path::PathBuf, StorageDescriptor) {
        use std::sync::atomic::{AtomicU64, Ordering};
        static SEQ: AtomicU64 = AtomicU64::new(0);
        // Per-call atomic counter (not a clock) guarantees a unique dir even when
        // tests run in parallel and the clock resolution is coarse.
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
        (root, descriptor)
    }

    fn now_nanos() -> u128 {
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos()
    }

    fn insert_shutdown_rows(store: &SqliteStore, names: &[&str]) {
        store
            .with_conn(|conn| {
                conn.execute_batch(
                    "CREATE TABLE IF NOT EXISTS shutdown_rows (name TEXT PRIMARY KEY);",
                )?;
                for name in names {
                    conn.execute("INSERT INTO shutdown_rows (name) VALUES (?1)", [name])?;
                }
                Ok(())
            })
            .expect("write shutdown rows");
    }

    fn shutdown_rows(conn: &rusqlite::Connection) -> rusqlite::Result<Vec<String>> {
        let mut statement = conn.prepare("SELECT name FROM shutdown_rows ORDER BY name")?;
        let rows = statement.query_map([], |row| row.get(0))?;
        rows.collect::<rusqlite::Result<Vec<_>>>()
    }

    fn assert_wal_sidecars_removed(path: &std::path::Path) {
        assert!(path.is_file(), "main database file remains at rest");
        for suffix in ["-wal", "-shm"] {
            let sidecar = std::path::PathBuf::from(format!("{}{suffix}", path.display()));
            assert!(
                !sidecar.exists(),
                "SQLite sidecar remained after store drop: {}",
                sidecar.display()
            );
        }
    }

    fn assert_rows_in_main_file(path: &std::path::Path, expected: &[&str]) {
        let conn = rusqlite::Connection::open(path).expect("reopen main database file");
        assert_eq!(
            shutdown_rows(&conn).expect("read rows from reopened main file"),
            expected,
            "reopened main database contains every committed row"
        );
    }

    #[test]
    fn dropping_store_with_read_pool_removes_wal_and_shm() {
        let (root, descriptor) = tmp();
        let path = std::path::PathBuf::from(sqlite_path(&descriptor));
        let store = open_sqlite_with(&descriptor, SqliteOpenOptions::default()).expect("open");
        insert_shutdown_rows(&store, &["one", "two", "three"]);
        let rows = store
            .with_read(shutdown_rows)
            .expect("read all committed rows through the pool");
        assert_eq!(rows, ["one", "three", "two"]);

        drop(store);

        assert_wal_sidecars_removed(&path);
        assert_rows_in_main_file(&path, &["one", "three", "two"]);
        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn dropping_store_without_read_pool_removes_wal_and_shm() {
        let (root, descriptor) = tmp();
        let path = std::path::PathBuf::from(sqlite_path(&descriptor));
        std::fs::create_dir_all(&root).expect("create database directory");
        let conn = rusqlite::Connection::open(&path).expect("open file database");
        conn.pragma_update(None, "journal_mode", "WAL")
            .expect("enable WAL");
        let store = SqliteStore::for_test(conn, 1);
        insert_shutdown_rows(&store, &["alpha", "beta"]);

        drop(store);

        assert_wal_sidecars_removed(&path);
        assert_rows_in_main_file(&path, &["alpha", "beta"]);
        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn restoring_closed_database_does_not_replay_later_wal_rows() {
        let (root, descriptor) = tmp();
        let path = std::path::PathBuf::from(sqlite_path(&descriptor));
        let backup = root.join("backup.db");

        {
            let store = open_sqlite(&descriptor).expect("open initial database");
            insert_shutdown_rows(&store, &["backup"]);
            assert_eq!(
                store
                    .with_read(shutdown_rows)
                    .expect("read backup rows through the pool"),
                ["backup"]
            );
        }
        assert_wal_sidecars_removed(&path);
        std::fs::copy(&path, &backup).expect("copy self-contained database backup");

        {
            let store = open_sqlite(&descriptor).expect("reopen database");
            insert_shutdown_rows(&store, &["later"]);
            assert_eq!(
                store
                    .with_read(shutdown_rows)
                    .expect("read later rows through the pool"),
                ["backup", "later"]
            );
        }
        assert_wal_sidecars_removed(&path);

        std::fs::remove_file(&path).expect("remove later database before restore");
        std::fs::rename(&backup, &path).expect("restore saved main database file");
        let store = open_sqlite(&descriptor).expect("open restored database");
        let rows = store
            .with_conn(shutdown_rows)
            .expect("read restored database rows");
        assert_eq!(rows, ["backup"]);
        drop(store);

        assert_wal_sidecars_removed(&path);
        let _ = std::fs::remove_dir_all(&root);
    }

    const M1: &[Migration] = &[Migration {
        version: 1,
        statements: "CREATE TABLE facts (id INTEGER PRIMARY KEY, name TEXT NOT NULL); \
                     INSERT INTO facts (id, name) VALUES (1, 'seed-a'), (2, 'seed-b');",
    }];

    #[test]
    fn open_runs_migrations_and_seeds_once() {
        let (root, d) = tmp();
        {
            let store = open_sqlite(&d).expect("open");
            store.migrate("facts", M1).expect("migrate");
            let n: i64 = store
                .with_conn(|c| c.query_row("SELECT COUNT(*) FROM facts", [], |r| r.get(0)))
                .expect("count");
            assert_eq!(n, 2, "seed rows inserted");
            assert_eq!(store.epoch(), 1);
        }
        // Reopen: migration must NOT re-run (no duplicate seed rows), epoch bumps.
        {
            let store = open_sqlite(&d).expect("reopen");
            store.migrate("facts", M1).expect("migrate again");
            let n: i64 = store
                .with_conn(|c| c.query_row("SELECT COUNT(*) FROM facts", [], |r| r.get(0)))
                .expect("count");
            assert_eq!(n, 2, "seed not re-inserted on reopen (run-once)");
            assert_eq!(store.epoch(), 2, "lease epoch is monotonic across opens");
        }
        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn second_live_writer_is_rejected() {
        let (root, d) = tmp();
        let _held = open_sqlite(&d).expect("first open");
        match open_sqlite(&d) {
            Err(StoreError::Lease(_)) => {}
            Err(e) => panic!("expected Lease(Held), got {e}"),
            Ok(_) => panic!("expected Lease(Held), got a second open"),
        }
        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn distinct_databases_do_not_falsely_contend() {
        // The footgun the first consumer hit: two DISTINCT databases must not
        // contend on one lease. Because the lease is derived from the db path, two
        // descriptors with different paths get different leases and both open while
        // live, even with the same module_id/namespace.
        let (root_a, a) = tmp();
        let (root_b, b) = tmp();
        let held_a = open_sqlite(&a).expect("open a");
        let held_b = open_sqlite(&b).expect("open b - distinct db, must not contend with a");
        drop((held_a, held_b));
        let _ = std::fs::remove_dir_all(&root_a);
        let _ = std::fs::remove_dir_all(&root_b);
    }

    #[test]
    fn later_migration_applies_on_top_of_earlier() {
        let (root, d) = tmp();
        {
            let s = open_sqlite(&d).expect("v1");
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
        let store = open_sqlite(&d).expect("v2");
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
        let _ = std::fs::remove_dir_all(&root);
    }

    /// A binary whose chain is SHORTER than the store's recorded version (a
    /// rollback, or a module that reset its numbering) must not read as a
    /// no-op: the outcome says the store is ahead and by how much, and the
    /// caller decides whether it can serve. Before this value existed the two
    /// runs returned the same `Ok(())`, and a v1 store under a reset chain
    /// started a daemon that could answer nothing.
    #[test]
    fn store_ahead_of_chain_is_reported_not_hidden() {
        let (root, d) = tmp();
        {
            let s = open_sqlite(&d).expect("newer binary");
            const NEWER: &[Migration] = &[
                Migration {
                    version: 1,
                    statements: "CREATE TABLE facts (id INTEGER PRIMARY KEY, name TEXT NOT NULL);",
                },
                Migration {
                    version: 9,
                    statements: "CREATE TABLE later (id INTEGER PRIMARY KEY);",
                },
            ];
            let outcome = s.migrate("facts", NEWER).expect("newer migrate");
            assert_eq!(outcome.applied, 2);
            assert_eq!(outcome.recorded, 9);
            assert!(!outcome.store_ahead(), "a store at its chain is not ahead");
        }
        // The older (or reset) binary: chain max 1 against a store at 9.
        let store = open_sqlite(&d).expect("older binary");
        let outcome = store
            .migrate("facts", M1)
            .expect("older migrate is Ok by policy");
        assert_eq!(
            outcome,
            MigrationOutcome {
                applied: 0,
                recorded: 9,
                chain_max: 1
            }
        );
        assert!(
            outcome.store_ahead(),
            "the skip must be visible to the caller"
        );
        // And nothing was applied or re-applied: M1's seed rows are still exactly two.
        let n: i64 = store
            .with_conn(|c| c.query_row("SELECT COUNT(*) FROM facts", [], |r| r.get(0)))
            .expect("count");
        assert_eq!(
            n, 0,
            "the newer chain's table was created empty; the older chain seeded nothing"
        );
        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn independent_namespace_chains_in_one_database() {
        // A multi-domain module (one database, several domains) registers an
        // independent migration chain per domain. Each chain's history is separate:
        // adding the second domain later does not re-run or entangle the first.
        let (root, d) = tmp();
        const WORK_GRAPH: &[Migration] = &[Migration {
            version: 1,
            statements: "CREATE TABLE wg_nodes (id INTEGER PRIMARY KEY);",
        }];
        const HIRES: &[Migration] = &[Migration {
            version: 1,
            statements: "CREATE TABLE hires (id INTEGER PRIMARY KEY);",
        }];
        let store = open_sqlite(&d).expect("open");
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
        let _ = std::fs::remove_dir_all(&root);
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
        match open_sqlite(&d) {
            Err(StoreError::UnsupportedBackend(b)) => assert_eq!(b, "postgres"),
            Err(e) => panic!("expected UnsupportedBackend, got {e}"),
            Ok(_) => panic!("expected UnsupportedBackend, got an open store"),
        }
    }

    const FENCE_SCHEMA: &[Migration] = &[Migration {
        version: 1,
        statements: "CREATE TABLE kv (k TEXT PRIMARY KEY, v TEXT NOT NULL);",
    }];

    #[test]
    fn fenced_write_commits_and_persists() {
        let (root, d) = tmp();
        let store = open_sqlite(&d).expect("open");
        store.migrate("kv", FENCE_SCHEMA).expect("migrate");
        store
            .with_conn_fenced(|tx| {
                tx.execute("INSERT INTO kv (k, v) VALUES ('a', '1')", [])?;
                Ok(())
            })
            .expect("fenced write");
        let v: String = store
            .with_conn(|c| c.query_row("SELECT v FROM kv WHERE k = 'a'", [], |r| r.get(0)))
            .expect("read back");
        assert_eq!(v, "1");
        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn fenced_write_rolls_back_on_error() {
        let (root, d) = tmp();
        let store = open_sqlite(&d).expect("open");
        store.migrate("kv", FENCE_SCHEMA).expect("migrate");
        let r: Result<(), StoreError> = store.with_conn_fenced(|tx| {
            tx.execute("INSERT INTO kv (k, v) VALUES ('a', '1')", [])?;
            // Force the closure to fail AFTER a write: the transaction must roll back.
            tx.query_row("SELECT * FROM does_not_exist", [], |_| Ok(()))?;
            Ok(())
        });
        assert!(
            matches!(r, Err(StoreError::Backend(_))),
            "closure error surfaces"
        );
        let n: i64 = store
            .with_conn(|c| c.query_row("SELECT COUNT(*) FROM kv", [], |r| r.get(0)))
            .expect("count");
        assert_eq!(n, 0, "the failed fenced write rolled back");
        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn superseded_writer_is_fenced_out_after_handover() {
        // The lease-handover race the credential vault must survive: an OLD store
        // instance (lower lease epoch) that still holds an open connection after a
        // NEW instance took over must NOT clobber the database. The live OS lock
        // prevents two simultaneous LEASE holders, so this models the in-memory state
        // of the race directly: two handles on one db file at epochs 2 (replacement)
        // and 1 (draining old) — exactly what the persisted epoch fence must catch
        // and the lock alone cannot.
        let (root, d) = tmp();
        let path = sqlite_path(&d);

        // First, a real open migrates the schema (epoch 1, released on drop).
        open_sqlite(&d)
            .expect("seed schema")
            .migrate("kv", FENCE_SCHEMA)
            .expect("migrate");

        let new = SqliteStore::for_test(rusqlite::Connection::open(&path).unwrap(), 2);
        new.with_conn_fenced(|tx| {
            tx.execute("INSERT INTO kv (k, v) VALUES ('owner', 'new')", [])
                .map(|_| ())
        })
        .expect("replacement claims the db at epoch 2");

        let stale = SqliteStore::for_test(rusqlite::Connection::open(&path).unwrap(), 1);
        match stale.with_conn_fenced(|tx| {
            tx.execute("UPDATE kv SET v = 'clobbered' WHERE k = 'owner'", [])
                .map(|_| ())
        }) {
            Err(StoreError::Fenced {
                holder_epoch,
                db_epoch,
            }) => {
                assert_eq!(holder_epoch, 1);
                assert_eq!(db_epoch, 2);
            }
            other => panic!("expected Fenced, got {other:?}"),
        }

        let v: String = new
            .with_conn(|c| c.query_row("SELECT v FROM kv WHERE k = 'owner'", [], |r| r.get(0)))
            .expect("read");
        assert_eq!(v, "new", "stale writer was fenced out, no clobber");
        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn equal_epoch_writer_is_not_fenced() {
        // A writer at the SAME epoch the db is already claimed at (its own prior
        // fenced write) is allowed — the fence rejects only a STRICTLY newer claim.
        let (root, d) = tmp();
        let path = sqlite_path(&d);
        open_sqlite(&d)
            .expect("seed")
            .migrate("kv", FENCE_SCHEMA)
            .expect("migrate");
        let s = SqliteStore::for_test(rusqlite::Connection::open(&path).unwrap(), 5);
        s.with_conn_fenced(|tx| {
            tx.execute("INSERT INTO kv (k, v) VALUES ('a', '1')", [])
                .map(|_| ())
        })
        .expect("claims at 5");
        s.with_conn_fenced(|tx| {
            tx.execute("INSERT INTO kv (k, v) VALUES ('b', '2')", [])
                .map(|_| ())
        })
        .expect("same epoch 5 still writes");
        let _ = std::fs::remove_dir_all(&root);
    }

    fn sqlite_path(d: &StorageDescriptor) -> String {
        match &d.backend {
            StorageBackend::Sqlite { path } => path.clone(),
            _ => unreachable!(),
        }
    }
}
