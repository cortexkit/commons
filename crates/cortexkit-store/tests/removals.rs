#![cfg(feature = "sqlite")]

//! The removals journal from outside the crate: an older copy of `store.db`
//! must never serve data a newer journal removed, and every way the journal
//! and the database can disagree must refuse to serve.

use cortexkit_store::{
    acquire_store_lease, open_sqlite, open_sqlite_with_removals, Isolation, Removal,
    RemovalJournal, RemovalRecord, RemovalsHandler, SqliteOpenOptions, SqliteStore, StorageBackend,
    StorageDescriptor, StoreError,
};
use rusqlite::{Connection, Transaction};
use std::{
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
            "cortexkit-store-removals-{}-{}-{}",
            std::process::id(),
            SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap()
                .as_nanos(),
            SEQ.fetch_add(1, Ordering::Relaxed),
        ));
        let descriptor = StorageDescriptor {
            module_id: "removals-tests".into(),
            storage_namespace: "default".into(),
            isolation: Isolation::Module,
            backend: StorageBackend::Sqlite {
                path: root.join("store.db").to_string_lossy().into_owned(),
            },
        };
        Self { root, descriptor }
    }

    fn path(&self) -> PathBuf {
        self.root.join("store.db")
    }

    fn journal(&self) -> PathBuf {
        self.root.join("removals").join("journal")
    }

    fn journal_text(&self) -> String {
        std::fs::read_to_string(self.journal()).unwrap()
    }

    fn write_journal(&self, text: &str) {
        std::fs::write(self.journal(), text).unwrap();
    }

    /// Save a closed store's database (the backup an older generation holds).
    fn save_db(&self, name: &str) -> PathBuf {
        assert_no_sidecars(&self.path());
        let saved = self.root.join(name);
        std::fs::copy(self.path(), &saved).unwrap();
        saved
    }

    /// Put a saved database back as the store's, as a restore would.
    fn restore_db(&self, saved: &Path) {
        assert_no_sidecars(&self.path());
        std::fs::copy(saved, self.path()).unwrap();
    }

    fn open(&self) -> Result<SqliteStore, StoreError> {
        open_sqlite_with_removals(&self.descriptor, SqliteOpenOptions::default(), &Items)
    }
}

impl Drop for TestDb {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.root);
    }
}

/// A closed store's `-wal` and `-shm` are gone, so its main file holds every
/// commit and copying that one file copies the whole database.
fn assert_no_sidecars(db: &Path) {
    for suffix in ["-wal", "-shm"] {
        let sidecar = PathBuf::from(format!("{}{suffix}", db.display()));
        assert!(
            !sidecar.exists(),
            "{} still exists: the store is not closed",
            sidecar.display()
        );
    }
}

/// The module's replay handler for these tests: removing an item deletes its
/// row and leaves a tombstone. Replaying is idempotent (`OR IGNORE`, and
/// deleting a missing row deletes nothing).
struct Items;

fn erase_items(tx: &Transaction<'_>, record: &RemovalRecord) -> rusqlite::Result<()> {
    for id in &record.ids {
        tx.execute("DELETE FROM items WHERE id = ?1", [id])?;
        tx.execute(
            "INSERT OR IGNORE INTO tombstones (kind, id) VALUES (?1, ?2)",
            [&record.kind, id],
        )?;
    }
    Ok(())
}

impl RemovalsHandler for Items {
    fn replay(&self, tx: &Transaction<'_>, record: &RemovalRecord) -> rusqlite::Result<()> {
        erase_items(tx, record)
    }
}

/// A module that removed items before adopting the journal: it backfills one
/// removal per tombstone it already holds.
struct ItemsWithBackfill;

impl RemovalsHandler for ItemsWithBackfill {
    fn replay(&self, tx: &Transaction<'_>, record: &RemovalRecord) -> rusqlite::Result<()> {
        erase_items(tx, record)
    }

    fn backfill(&self, conn: &Connection) -> rusqlite::Result<Vec<Removal>> {
        let mut statement = conn.prepare("SELECT kind, id FROM tombstones ORDER BY id")?;
        let rows = statement.query_map([], |row| {
            Ok(Removal {
                kind: row.get(0)?,
                ids: vec![row.get(1)?],
            })
        })?;
        rows.collect()
    }
}

const SCHEMA: &str = "CREATE TABLE items (id TEXT PRIMARY KEY);
     CREATE TABLE tombstones (kind TEXT NOT NULL, id TEXT NOT NULL, PRIMARY KEY (kind, id));
     INSERT INTO items VALUES ('a'), ('b'), ('c'), ('d');";

fn seed(store: &SqliteStore) {
    store.with_conn(|conn| conn.execute_batch(SCHEMA)).unwrap();
}

fn items(store: &SqliteStore) -> Vec<String> {
    store
        .with_read(|conn| {
            let mut statement = conn.prepare("SELECT id FROM items ORDER BY id")?;
            let rows = statement.query_map([], |row| row.get(0))?;
            rows.collect()
        })
        .unwrap()
}

fn remove(store: &SqliteStore, ids: &[&str]) -> RemovalRecord {
    store
        .remove("item", ids.iter().copied(), erase_items)
        .unwrap()
}

/// A store opened with the journal, seeded with items a..d, then closed.
fn seeded(db: &TestDb) {
    let store = db.open().unwrap();
    seed(&store);
}

fn expect_unreadable(result: Result<SqliteStore, StoreError>, what: &str) {
    match result {
        Err(StoreError::RemovalsUnreadable { detail }) => {
            assert!(
                detail.contains(what),
                "{detail:?} does not mention {what:?}"
            )
        }
        Err(error) => panic!("expected RemovalsUnreadable ({what}), got {error:?}"),
        Ok(_) => panic!("expected RemovalsUnreadable ({what}), but the store opened"),
    }
}

fn lines(text: &str) -> Vec<&str> {
    text.split_inclusive('\n').collect()
}

/// The promise the journal exists for: an older `store.db` (a backup taken
/// before the removals) opened beside the newest journal serves without the
/// removed ids, and has applied every record when the open returns.
#[test]
fn an_older_database_with_the_newest_journal_serves_without_the_removed_ids() {
    let db = TestDb::new();
    seeded(&db);
    let older = db.save_db("older.db");

    {
        let store = db.open().unwrap();
        assert_eq!(remove(&store, &["a", "b"]).seq, 1);
        assert_eq!(remove(&store, &["c"]).seq, 2);
        assert_eq!(items(&store), ["d"]);
    }

    db.restore_db(&older);
    let store = db.open().unwrap();
    assert_eq!(
        items(&store),
        ["d"],
        "the older database served removed ids"
    );
    assert_eq!(store.applied_removal_seq().unwrap(), 2);
    assert_eq!(remove(&store, &["d"]).seq, 3, "the next record follows on");
}

/// A crash between the two write steps: the record is in the journal, but its
/// erase never committed (here the erase fails). The store refuses further
/// removals, and the next open replays the record.
#[test]
fn a_record_journaled_without_its_erase_is_replayed_on_the_next_open() {
    let db = TestDb::new();
    seeded(&db);
    {
        let store = db.open().unwrap();
        let failed = store.remove("item", ["a"], |_, _| {
            Err(rusqlite::Error::InvalidParameterName("crash".into()))
        });
        assert!(matches!(failed, Err(StoreError::Backend(_))), "{failed:?}");
        assert_eq!(items(&store), ["a", "b", "c", "d"], "the erase rolled back");
        assert_eq!(store.applied_removal_seq().unwrap(), 0);
        assert!(
            store.remove("item", ["b"], erase_items).is_err(),
            "a later removal was admitted past an unapplied record"
        );
    }
    assert_eq!(lines(&db.journal_text()).len(), 2, "header and record 1");

    let store = db.open().unwrap();
    assert_eq!(items(&store), ["b", "c", "d"]);
    assert_eq!(store.applied_removal_seq().unwrap(), 1);
}

/// The write order, observed from inside the erase: by the time `erase` runs,
/// its record is already in the journal file.
#[test]
fn the_erase_runs_only_after_its_record_is_in_the_journal() {
    let db = TestDb::new();
    seeded(&db);
    let store = db.open().unwrap();
    let journal = db.journal();
    store
        .remove("item", ["a"], |tx, record| {
            let text = std::fs::read_to_string(&journal).unwrap();
            let last = lines(&text).last().copied().unwrap_or_default();
            assert!(
                last.starts_with(&format!("{{\"seq\":{},", record.seq)) && last.ends_with('\n'),
                "the erase ran before its record was journaled: {last:?}"
            );
            erase_items(tx, record)
        })
        .unwrap();
}

/// Stamp the database's fence epoch through a raw connection, as a newer
/// writer's fenced write would.
fn stamp_fence(db: &Path, epoch: u64) {
    let conn = Connection::open(db).unwrap();
    conn.execute_batch(
        "CREATE TABLE IF NOT EXISTS cortexkit_fence (
             id INTEGER PRIMARY KEY CHECK (id = 0), epoch INTEGER NOT NULL)",
    )
    .unwrap();
    conn.execute(
        "INSERT INTO cortexkit_fence (id, epoch) VALUES (0, ?1)
         ON CONFLICT(id) DO UPDATE SET epoch = excluded.epoch",
        [epoch as i64],
    )
    .unwrap();
}

/// The database's fence epoch, read through a raw connection.
fn stored_fence(db: &Path) -> u64 {
    let conn = Connection::open(db).unwrap();
    conn.query_row(
        "SELECT epoch FROM cortexkit_fence WHERE id = 0",
        [],
        |row| row.get::<_, i64>(0),
    )
    .unwrap() as u64
}

/// A superseded writer: a newer writer has stamped a fence epoch above this
/// store's lease epoch. Its removal is refused with `Fenced` BEFORE anything
/// reaches the journal, so the journal's bytes are unchanged and the next open
/// has nothing to replay: no removal that no current writer decided.
#[test]
fn a_superseded_writer_is_refused_before_its_record_reaches_the_journal() {
    let db = TestDb::new();
    seeded(&db);
    let journal_before;
    {
        let store = db.open().unwrap();
        let epoch = store.epoch();
        stamp_fence(&db.path(), epoch + 1);
        journal_before = std::fs::read(db.journal()).unwrap();

        match store.remove("item", ["a"], erase_items) {
            Err(StoreError::Fenced {
                holder_epoch,
                db_epoch,
            }) => assert_eq!((holder_epoch, db_epoch), (epoch, epoch + 1)),
            other => panic!("expected Fenced, got {other:?}"),
        }
        assert_eq!(
            std::fs::read(db.journal()).unwrap(),
            journal_before,
            "a refused removal changed the journal"
        );
        assert_eq!(items(&store), ["a", "b", "c", "d"]);
        assert_eq!(store.applied_removal_seq().unwrap(), 0);
    }

    let store = db.open().unwrap();
    assert_eq!(std::fs::read(db.journal()).unwrap(), journal_before);
    assert_eq!(
        store.applied_removal_seq().unwrap(),
        0,
        "a record was replayed"
    );
    assert_eq!(items(&store), ["a", "b", "c", "d"]);
}

/// The fence refuses only a store whose epoch is BELOW the database's: one
/// holding exactly the stamped epoch still removes, and its record is applied.
#[test]
fn a_writer_at_the_fence_epoch_still_removes() {
    let db = TestDb::new();
    seeded(&db);
    let store = db.open().unwrap();
    let epoch = store.epoch();
    stamp_fence(&db.path(), epoch);

    assert_eq!(remove(&store, &["a"]).seq, 1);
    assert_eq!(items(&store), ["b", "c", "d"]);
    assert_eq!(store.applied_removal_seq().unwrap(), 1);
    assert_eq!(lines(&db.journal_text()).len(), 2, "header and record 1");
    assert_eq!(stored_fence(&db.path()), epoch);
}

/// A crash while a record was being appended leaves a torn final line. Its
/// erase never ran, so the open drops it (truncating the file) and serves, and
/// the next record takes its seq.
#[test]
fn a_torn_final_line_is_dropped_and_the_store_serves() {
    let db = TestDb::new();
    seeded(&db);
    {
        let store = db.open().unwrap();
        remove(&store, &["a"]);
    }
    let intact = db.journal_text();
    for torn in [
        "{\"seq\":2,\"kind\":\"it",
        "{\"seq\":2,\"kind\":\"item\",\"ids\":[\"b\"],\"at_ms\":1,\"crc\":\"00000000\"}\n",
    ] {
        db.write_journal(&format!("{intact}{torn}"));
        let store = db.open().expect("a torn final line refused the open");
        assert_eq!(items(&store), ["b", "c", "d"]);
        assert_eq!(
            db.journal_text(),
            intact,
            "the torn line was not truncated away"
        );
        drop(store);
    }

    let store = db.open().unwrap();
    assert_eq!(remove(&store, &["b"]).seq, 2);
}

/// A final line whose erase DID commit is never the one dropped: when the
/// journal's final record is damaged but the database has already applied it,
/// dropping it would lose evidence the database relied on, so the open refuses
/// and leaves the file as it was.
#[test]
fn a_damaged_final_record_whose_erase_committed_is_refused_not_dropped() {
    let db = TestDb::new();
    seeded(&db);
    {
        let store = db.open().unwrap();
        remove(&store, &["a"]);
        remove(&store, &["b"]);
    }
    let text = db.journal_text();
    let damaged = text.replacen("[\"b\"]", "[\"x\"]", 1);
    assert_ne!(damaged, text);
    for journal in [damaged.clone(), text.trim_end_matches('\n').to_owned()] {
        db.write_journal(&journal);
        match db.open() {
            Err(StoreError::RemovalsBehindStore {
                journal_seq: 1,
                store_seq: 2,
            }) => {}
            other => panic!(
                "expected RemovalsBehindStore 1 < 2, got {:?}",
                other.map(|_| ())
            ),
        }
        assert_eq!(
            db.journal_text(),
            journal,
            "the refused open changed the journal"
        );
    }
}

/// Damage anywhere but the final line, and any break in the sequence, is
/// corruption: the store refuses to serve.
#[test]
fn a_damaged_middle_record_a_sequence_gap_and_a_repeated_seq_each_refuse() {
    let db = TestDb::new();
    seeded(&db);
    {
        let store = db.open().unwrap();
        remove(&store, &["a"]);
        remove(&store, &["b"]);
        remove(&store, &["c"]);
    }
    let text = db.journal_text();
    let original = lines(&text);
    assert_eq!(original.len(), 4, "header and three records");

    let damaged_middle = text.replacen("[\"b\"]", "[\"x\"]", 1);
    let gap = [original[0], original[1], original[3]].concat();
    let repeat = [&original[..], &original[3..]].concat().concat();
    for (journal, what) in [
        (damaged_middle, "damaged"),
        (gap, "sequence gap"),
        (repeat, "repeated sequence"),
    ] {
        db.write_journal(&journal);
        expect_unreadable(db.open(), what);
    }
}

/// The journal comes from an older generation than the database.
#[test]
fn a_journal_behind_the_database_refuses_with_the_remedy() {
    let db = TestDb::new();
    seeded(&db);
    let older_journal = {
        let store = db.open().unwrap();
        remove(&store, &["a"]);
        let older = db.journal_text();
        remove(&store, &["b"]);
        older
    };
    db.write_journal(&older_journal);
    match db.open() {
        Err(error @ StoreError::RemovalsBehindStore { .. }) => {
            assert!(matches!(
                error,
                StoreError::RemovalsBehindStore {
                    journal_seq: 1,
                    store_seq: 2
                }
            ));
            assert!(error.to_string().contains(
                "restore the removals journal from the same or a newer generation than the store"
            ));
        }
        other => panic!("expected RemovalsBehindStore, got {:?}", other.map(|_| ())),
    }
}

/// The database has applied removals but the journal is gone: deleted, or a
/// restore that brought back the database alone.
#[test]
fn a_missing_journal_refuses_once_the_database_has_applied_removals() {
    let db = TestDb::new();
    seeded(&db);
    {
        let store = db.open().unwrap();
        remove(&store, &["a"]);
    }
    for remove_dir in [false, true] {
        if remove_dir {
            std::fs::remove_dir_all(db.root.join("removals")).unwrap();
        } else {
            std::fs::remove_file(db.journal()).unwrap();
        }
        match db.open() {
            Err(error @ StoreError::RemovalsMissing { store_seq: 1 }) => {
                assert!(error
                    .to_string()
                    .contains("restore the newest removals journal"))
            }
            other => panic!("expected RemovalsMissing, got {:?}", other.map(|_| ())),
        }
    }
}

#[test]
fn a_journal_naming_another_owner_refuses() {
    let db = TestDb::new();
    seeded(&db);
    let text = db.journal_text();
    db.write_journal(&text.replacen("removals-tests", "someone-else", 1));
    match db.open() {
        Err(StoreError::RemovalsOwnerMismatch {
            journal_module_id,
            expected_module_id,
            ..
        }) => {
            assert_eq!(journal_module_id, "someone-else");
            assert_eq!(expected_module_id, "removals-tests");
        }
        other => panic!(
            "expected RemovalsOwnerMismatch, got {:?}",
            other.map(|_| ())
        ),
    }
}

/// A store that adopted the journal can't be opened without its handler:
/// not while its `removals/` directory exists, and not when its database has
/// applied removals even with the directory gone.
#[test]
fn a_plain_open_of_an_adopted_store_refuses() {
    let db = TestDb::new();
    seeded(&db);
    let expect_required = |what: &str| match open_sqlite(&db.descriptor) {
        Err(StoreError::RemovalsHandlerRequired) => {}
        other => panic!(
            "{what}: expected RemovalsHandlerRequired, got {:?}",
            other.map(|_| ())
        ),
    };
    expect_required("journal directory, nothing applied");
    {
        let store = db.open().unwrap();
        remove(&store, &["a"]);
        assert!(matches!(
            open_sqlite(&db.descriptor),
            Err(StoreError::Lease(_))
        ));
    }
    expect_required("journal directory, removals applied");
    std::fs::remove_dir_all(db.root.join("removals")).unwrap();
    expect_required("removals applied, directory gone");

    // A store that never adopted the journal opens as before.
    let fresh = TestDb::new();
    let store = open_sqlite(&fresh.descriptor).unwrap();
    assert!(matches!(
        store.remove("item", ["a"], erase_items),
        Err(StoreError::RemovalsHandlerRequired)
    ));
}

/// Adoption creates the journal owner-only, beside the database.
#[test]
fn adoption_creates_an_owner_only_journal_with_its_header() {
    let db = TestDb::new();
    seeded(&db);
    assert_eq!(RemovalJournal::path(&db.descriptor).unwrap(), db.journal());
    assert_eq!(
        db.journal_text(),
        "{\"format\":1,\"owner\":{\"module_id\":\"removals-tests\",\"storage_namespace\":\"default\"}}\n"
    );
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let mode = |path: &Path| std::fs::metadata(path).unwrap().permissions().mode() & 0o777;
        assert_eq!(mode(&db.root.join("removals")), 0o700);
        assert_eq!(mode(&db.journal()), 0o600);
    }
}

/// A restore tool opens a copy of an older backup as a scratch store: the
/// live store's journal is replayed onto it before `open_scratch_with_removals`
/// returns, and a plain scratch open is refused.
#[test]
fn open_scratch_replays_the_live_journal_onto_an_older_copy() {
    let db = TestDb::new();
    seeded(&db);
    let backup = db.save_db("backup.db");
    {
        let store = db.open().unwrap();
        remove(&store, &["a", "b"]);
    }
    let scratch_path = db.root.join("restore").join("store.db");
    std::fs::create_dir_all(scratch_path.parent().unwrap()).unwrap();
    std::fs::copy(&backup, &scratch_path).unwrap();

    let lease = acquire_store_lease(&db.descriptor).unwrap();
    assert!(matches!(
        lease.open_scratch(&scratch_path, SqliteOpenOptions::default()),
        Err(StoreError::RemovalsHandlerRequired)
    ));
    let scratch = lease
        .open_scratch_with_removals(&scratch_path, SqliteOpenOptions::default(), &Items)
        .unwrap();
    assert_eq!(
        items(&scratch),
        ["c", "d"],
        "the scratch copy holds removed rows"
    );
    assert_eq!(scratch.applied_removal_seq().unwrap(), 1);
    assert!(
        !scratch_path.parent().unwrap().join("removals").exists(),
        "the scratch open made a journal beside the copy"
    );
    assert!(matches!(
        scratch.remove("item", ["c"], erase_items),
        Err(StoreError::RemovalsHandlerRequired)
    ));
}

/// A scratch copy that has applied more removals than the live journal holds
/// is refused before it can be placed.
#[test]
fn open_scratch_refuses_a_copy_ahead_of_the_live_journal() {
    let db = TestDb::new();
    seeded(&db);
    let older_journal = {
        let store = db.open().unwrap();
        remove(&store, &["a"]);
        let older = db.journal_text();
        remove(&store, &["b"]);
        older
    };
    let newer = db.save_db("newer.db");
    db.write_journal(&older_journal);

    let lease = acquire_store_lease(&db.descriptor).unwrap();
    match lease.open_scratch_with_removals(&newer, SqliteOpenOptions::default(), &Items) {
        Err(StoreError::RemovalsBehindStore {
            journal_seq: 1,
            store_seq: 2,
        }) => {}
        other => panic!("expected RemovalsBehindStore, got {:?}", other.map(|_| ())),
    }
}

/// A module that changes its owner value rewrites the journal header under
/// the maintenance lease, keeping every record; without the rewrite it would
/// refuse its own journal.
#[test]
fn rewrite_owner_moves_the_journal_to_a_new_owner() {
    let db = TestDb::new();
    seeded(&db);
    {
        let store = db.open().unwrap();
        remove(&store, &["a"]);
    }
    let records_before: Vec<String> = lines(&db.journal_text())[1..]
        .iter()
        .map(|line| line.to_string())
        .collect();
    let mut renamed = db.descriptor.clone();
    renamed.storage_namespace = "renamed".into();
    // The ownership change itself: clear the database's owner row.
    Connection::open(db.path())
        .unwrap()
        .execute("DELETE FROM cortexkit_owner", [])
        .unwrap();
    let open_renamed = || open_sqlite_with_removals(&renamed, SqliteOpenOptions::default(), &Items);
    assert!(matches!(
        open_renamed(),
        Err(StoreError::RemovalsOwnerMismatch { .. })
    ));

    let lease = acquire_store_lease(&renamed).unwrap();
    assert!(matches!(
        RemovalJournal::rewrite_owner(&lease, "removals-tests", "not-the-old-namespace"),
        Err(StoreError::RemovalsOwnerMismatch { .. })
    ));
    RemovalJournal::rewrite_owner(&lease, "removals-tests", "default").unwrap();
    RemovalJournal::rewrite_owner(&lease, "removals-tests", "default")
        .expect("running the migration again is a no-op");
    drop(lease);

    let text = db.journal_text();
    assert!(lines(&text)[0].contains("\"storage_namespace\":\"renamed\""));
    assert_eq!(lines(&text)[1..], records_before[..]);
    let store = open_renamed().unwrap();
    assert_eq!(items(&store), ["b", "c", "d"]);
    assert_eq!(remove(&store, &["b"]).seq, 2);
}

/// A module that removed items before adopting the journal backfills one
/// record per tombstone when it adopts, so an older database from before
/// those removals still can't bring them back.
#[test]
fn adoption_backfills_one_record_per_existing_tombstone() {
    let db = TestDb::new();
    {
        let store = open_sqlite(&db.descriptor).unwrap();
        seed(&store);
    }
    let before_removals = db.save_db("before-removals.db");
    {
        // The module's removals before it adopted the journal.
        let store = open_sqlite(&db.descriptor).unwrap();
        store
            .with_conn(|conn| {
                conn.execute_batch(
                    "DELETE FROM items WHERE id IN ('a', 'b');
                     INSERT INTO tombstones VALUES ('item', 'a'), ('item', 'b');",
                )
            })
            .unwrap();
    }

    let open = || {
        open_sqlite_with_removals(
            &db.descriptor,
            SqliteOpenOptions::default(),
            &ItemsWithBackfill,
        )
    };
    {
        let store = open().unwrap();
        assert_eq!(store.applied_removal_seq().unwrap(), 2);
        assert_eq!(items(&store), ["c", "d"]);
    }
    let text = db.journal_text();
    let records = lines(&text);
    assert_eq!(records.len(), 3, "header and one record per tombstone");
    assert!(records[1].starts_with("{\"seq\":1,\"kind\":\"item\",\"ids\":[\"a\"],"));
    assert!(records[2].starts_with("{\"seq\":2,\"kind\":\"item\",\"ids\":[\"b\"],"));

    db.restore_db(&before_removals);
    let store = open().unwrap();
    assert_eq!(items(&store), ["c", "d"], "a backfilled removal came back");
}
