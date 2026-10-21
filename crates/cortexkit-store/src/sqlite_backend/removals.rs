//! The removals journal: evidence of each removal, kept outside `store.db`.
//!
//! A module's tombstones live inside its database, so an older copy of that
//! database (a backup restore, a hand-placed export, a new machine) brings
//! deleted data back. The journal records every removal in its own file,
//! `<store dir>/removals/journal`, and every open replays the records the
//! database has not applied yet before it serves. The store never tries to
//! detect a restore: every open reads and checks the whole journal, then
//! compares its newest sequence with the database's `applied_removal_seq`;
//! when nothing is new, that comparison is the only database work.
//!
//! The file is append-only JSON lines. The first line is a header,
//! `{"format":1,"owner":{"module_id":…,"storage_namespace":…}}`; each later
//! line is one record, `{"seq":…,"kind":…,"ids":[…],"at_ms":…,"crc":…}`.
//! `seq` starts at 1 and grows by exactly 1. `crc` is the CRC-32C, as eight
//! lowercase hex digits, of the record's line without its `crc` field (the
//! compact JSON `{"seq":…,"kind":…,"ids":[…],"at_ms":…}` in that key order).
//! Records are never compacted: an older generation of the database can come
//! back at any time, so no record is ever safe to drop.

use super::*;
use std::{fs::OpenOptions, io::Write};

/// The journal's directory beside the database. A directory of its own, so a
/// module can declare it as its own backup entry.
const DIR_NAME: &str = "removals";
const FILE_NAME: &str = "journal";
/// Where a new journal (or a rewritten one) is written before it is renamed
/// into place, so the journal file is never seen half-written.
const TEMP_NAME: &str = "journal.tmp";
/// The only header `format` this build reads and writes.
const FORMAT: u64 = 1;

/// One journaled removal, as passed to the `erase` closure of
/// [`SqliteStore::remove`] and to [`RemovalsHandler::replay`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RemovalRecord {
    /// Position in the journal: 1 for the first record, then exactly one more
    /// for each record after it.
    pub seq: u64,
    /// The module's word for what was removed ("session", "transcript"). The
    /// store never interprets it.
    pub kind: String,
    /// The removed ids. One record may carry many (a retention sweep).
    pub ids: Vec<String>,
    /// When the record was written, in milliseconds since the Unix epoch.
    pub at_ms: u64,
}

/// A removal the database already holds as a tombstone, returned by
/// [`RemovalsHandler::backfill`] when a module adopts the journal.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Removal {
    /// As [`RemovalRecord::kind`].
    pub kind: String,
    /// As [`RemovalRecord::ids`].
    pub ids: Vec<String>,
}

/// How a module applies journaled removals to its database. Passed to
/// [`open_sqlite_with_removals`] and [`StoreLease::open_scratch_with_removals`].
pub trait RemovalsHandler {
    /// Erase `record`'s ids from the database, exactly as the `erase` closure
    /// given to [`SqliteStore::remove`] did when the record was written.
    ///
    /// The contract:
    /// - **Database only.** Touch nothing but `tx`. Cleanup outside the database
    ///   (files, private host directories) goes through the module's own
    ///   receipts and retries, never through this method: a replay can run on a
    ///   scratch copy that is later thrown away, and runs again on every older
    ///   copy of the database that comes back.
    /// - **Idempotent.** The same record may be replayed onto a database that
    ///   already lacks some or all of its ids (a backfilled tombstone, a copy
    ///   restored twice). Erasing what is already gone must succeed.
    ///
    /// The store sets `applied_removal_seq` to `record.seq` in the same
    /// transaction and commits it. Returning an error rolls both back and the
    /// open fails, so the store never serves with the record unapplied.
    fn replay(&self, tx: &Transaction<'_>, record: &RemovalRecord) -> rusqlite::Result<()>;

    /// The removals the database already holds as tombstones, one entry per
    /// tombstone. Called once, by the open that creates the journal (the
    /// module's adoption), before it serves. The default is none, which is
    /// right for a module that never removed data before adopting.
    ///
    /// A module that removed data before adopting the journal has that
    /// evidence only inside its database, so an older copy of the database
    /// would otherwise bring those items back. Each returned entry becomes one
    /// journal record, written in the same step that creates the journal; the
    /// open then replays them like any other record, so [`Self::replay`] must
    /// accept a record whose ids are already gone.
    fn backfill(&self, conn: &Connection) -> rusqlite::Result<Vec<Removal>> {
        let _ = conn;
        Ok(Vec::new())
    }
}

/// Operations on a store's removals journal that run outside an open store.
#[derive(Debug)]
pub struct RemovalJournal {
    _private: (),
}

impl RemovalJournal {
    /// Where `descriptor`'s journal lives: `<store dir>/removals/journal`.
    /// The module declares `<store dir>/removals/` as its own portable backup
    /// entry. Refused for an in-memory database or a SQLite URI, which have no
    /// journal.
    pub fn path(descriptor: &StorageDescriptor) -> Result<PathBuf, StoreError> {
        let path = sqlite_path(descriptor)?;
        require_file_store(&path)?;
        Ok(journal_file(&path))
    }

    /// Rewrite the journal header's owner from `(previous_module_id,
    /// previous_storage_namespace)` to the owner of `lease`'s descriptor,
    /// keeping every record as it is.
    ///
    /// The journal header names the store's `(module_id, storage_namespace)`,
    /// and an open refuses a journal naming another owner
    /// ([`StoreError::RemovalsOwnerMismatch`]). A module that legitimately
    /// changes its owner value would therefore refuse its own journal; this is
    /// its migration. The steps:
    /// 1. stop every process that writes the store, under the old owner too
    ///    (its lease key differs, so the new owner's lease does not exclude it);
    /// 2. clear the database's `cortexkit_owner` row, as for any ownership
    ///    change, so the next open claims it for the new owner;
    /// 3. take [`acquire_store_lease`] for the NEW descriptor and call this;
    /// 4. drop the lease and open the store as usual.
    ///
    /// The new journal is written to a temporary file, synced, and renamed over
    /// the old one, so a crash leaves either the old journal or the new one. A
    /// torn final line is left out, as an open would drop it. A journal that
    /// already names the new owner is left untouched, so running the migration
    /// again succeeds. A header naming neither owner is refused with
    /// [`StoreError::RemovalsOwnerMismatch`]; a missing or damaged journal with
    /// [`StoreError::RemovalsUnreadable`].
    pub fn rewrite_owner(
        lease: &StoreLease,
        previous_module_id: &str,
        previous_storage_namespace: &str,
    ) -> Result<(), StoreError> {
        require_file_store(&lease.live_path)?;
        let journal = journal_file(&lease.live_path);
        let bytes = std::fs::read(&journal).map_err(|e| unreadable(&journal, e.to_string()))?;
        let parsed = parse_journal(&journal, &bytes)?;
        let new = &lease.descriptor;
        if parsed.owner == (new.module_id.clone(), new.storage_namespace.clone()) {
            return Ok(());
        }
        check_journal_owner(parsed.owner, previous_module_id, previous_storage_namespace)?;
        let mut contents = header_line(&new.module_id, &new.storage_namespace).into_bytes();
        contents.extend_from_slice(&bytes[parsed.header_len..parsed.valid_len]);
        write_journal_atomically(&journal_dir(&lease.live_path), &contents)
    }
}

/// Open a module's SQLite store that keeps a removals journal: everything
/// [`open_sqlite_with`] does, then replay the journal before serving.
///
/// The open:
/// 1. takes the lease, opens the database and claims or checks its owner, as
///    [`open_sqlite_with`] does;
/// 2. reads `<store dir>/removals/journal` and checks that its header names
///    this store's owner;
/// 3. replays, in order, every record newer than the database's
///    `applied_removal_seq`, each through `handler` in its own transaction
///    that also advances that sequence;
/// 4. serves.
///
/// When the journal does not exist and the database has applied no removal,
/// this open is the module's adoption: it creates `removals/` (mode 0700) and
/// the journal (mode 0600), with one record per tombstone that
/// [`RemovalsHandler::backfill`] returns, and replays those records before
/// serving. From then on a plain [`open_sqlite`] of the store is refused with
/// [`StoreError::RemovalsHandlerRequired`].
///
/// A torn final line (a crash while a record was being appended) is truncated
/// away: its erase never ran, because [`SqliteStore::remove`] erases only after
/// the record is synced. Refusals, all before serving:
/// - [`StoreError::RemovalsUnreadable`]: the journal can't be read, or a
///   record before its final line is damaged, or its sequence has a gap or a
///   repeat;
/// - [`StoreError::RemovalsBehindStore`]: the journal ends before the
///   database's applied sequence (the journal is from an older generation);
/// - [`StoreError::RemovalsMissing`]: the database has applied removals but
///   the journal is gone;
/// - [`StoreError::RemovalsOwnerMismatch`]: the journal's header names another
///   owner.
///
/// Refused with [`StoreError::Backend`] for an in-memory database or a SQLite
/// URI, which have no directory to keep a journal in.
pub fn open_sqlite_with_removals(
    descriptor: &StorageDescriptor,
    options: SqliteOpenOptions,
    handler: &dyn RemovalsHandler,
) -> Result<SqliteStore, StoreError> {
    // All three checks run before anything touches the disk.
    let path = sqlite_path(descriptor)?;
    check_options(&options)?;
    require_file_store(&path)?;

    let (path, lease) = take_store_lease(descriptor)?;
    let mut store = open_under_lease(
        path.clone(),
        descriptor,
        options,
        Arc::from(lease),
        AppliedRemovals::Allow,
    )?;
    let journal_seq = replay_journal(&store, &path, descriptor, handler, IfMissing::Create)?;
    store.removals = Some(Mutex::new(JournalWriter {
        path: journal_file(&path),
        next_seq: journal_seq + 1,
        poisoned: None,
    }));
    Ok(store)
}

impl StoreLease {
    /// [`StoreLease::open_scratch`] for a store that keeps a removals journal:
    /// open the scratch copy, then replay the LIVE store directory's journal
    /// onto it before returning, so a copy placed as the live store never holds
    /// rows the journal removed, even briefly. The journal beside the scratch
    /// file, if any, plays no part.
    ///
    /// The checks and replay are those of [`open_sqlite_with_removals`], run
    /// against the scratch database: a copy whose applied sequence is ahead of
    /// the live journal is refused with [`StoreError::RemovalsBehindStore`],
    /// and one that has applied removals while the live journal is missing
    /// with [`StoreError::RemovalsMissing`], so neither is placed. A torn final
    /// line of the live journal is truncated away, as the module's own open
    /// would do.
    ///
    /// When the live store has no journal and the copy has applied no removal,
    /// there is nothing to replay and no journal is created: the module adopts
    /// the journal on its own next open, with its own backfill.
    ///
    /// The returned store cannot [`SqliteStore::remove`]: deciding removals is
    /// the module's job, not a restore tool's.
    pub fn open_scratch_with_removals(
        &self,
        path: impl AsRef<Path>,
        options: SqliteOpenOptions,
        handler: &dyn RemovalsHandler,
    ) -> Result<SqliteStore, StoreError> {
        let path = self.scratch_path(path.as_ref(), &options)?;
        let store = self.open_scratch_at(path, options, AppliedRemovals::Allow)?;
        replay_journal(
            &store,
            &self.live_path,
            &self.descriptor,
            handler,
            IfMissing::Accept,
        )?;
        Ok(store)
    }
}

impl SqliteStore {
    /// Remove data with evidence that survives an older copy of the database:
    /// journal a record of `kind` and `ids`, then run `erase` to delete them.
    ///
    /// 1. The record is appended to the journal and the file is synced.
    /// 2. `erase(&tx, &record)` runs in an immediate transaction that also sets
    ///    `applied_removal_seq` to `record.seq`, and that transaction commits.
    ///
    /// `erase` should delete exactly what the module's
    /// [`RemovalsHandler::replay`] would for the same record; usually it calls
    /// the handler. Returns the record on success.
    ///
    /// If step 1 fails, nothing was erased. If step 2 fails (or `erase` returns
    /// an error), the record is already durable: the removal has been decided,
    /// and the next open replays it. In either case this store refuses further
    /// removals until it is reopened, so a record whose erase did not commit
    /// can't be skipped by a later one.
    ///
    /// Refused with [`StoreError::RemovalsHandlerRequired`] on a store not opened
    /// with [`open_sqlite_with_removals`], including every scratch store.
    pub fn remove<I>(
        &self,
        kind: &str,
        ids: I,
        erase: impl FnOnce(&Transaction<'_>, &RemovalRecord) -> rusqlite::Result<()>,
    ) -> Result<RemovalRecord, StoreError>
    where
        I: IntoIterator,
        I::Item: Into<String>,
    {
        let journal = self
            .removals
            .as_ref()
            .ok_or(StoreError::RemovalsHandlerRequired)?;
        let backend = |e: rusqlite::Error| StoreError::Backend(e.to_string());
        let mut conn = self.conn.lock().unwrap_or_else(|p| p.into_inner());
        let mut journal = journal.lock().unwrap_or_else(|p| p.into_inner());
        if let Some(why) = &journal.poisoned {
            return Err(StoreError::Backend(format!(
                "removals are refused until the store is reopened: {why}"
            )));
        }

        // Taken before the append only so the check below and the erase see the
        // same database state; no erase runs until the record is durable.
        let tx = conn
            .transaction_with_behavior(TransactionBehavior::Immediate)
            .map_err(backend)?;
        let applied = read_applied_seq(&tx).map_err(backend)?;
        if applied + 1 != journal.next_seq {
            return Err(StoreError::Backend(format!(
                "the database has applied removals up to {applied}, but the journal's next \
                 record is {}; reopen the store so it replays the journal",
                journal.next_seq
            )));
        }
        let record = RemovalRecord {
            seq: journal.next_seq,
            kind: kind.to_owned(),
            ids: ids.into_iter().map(Into::into).collect(),
            at_ms: now_ms(),
        };

        // Step 1: the record is synced to the journal BEFORE its erase runs.
        // This order is what makes an open's handling of a torn final line safe:
        // the open drops that line without replaying it, which is right only
        // because no erase can have run for a record that never finished
        // syncing. Erasing first would let a crash leave data erased with no
        // evidence, and an older copy of the database would bring it back.
        if let Err(error) = append_record(&journal.path, &record) {
            // A partly written line may now end the file. Refusing further
            // appends keeps it the final line, which the next open drops.
            journal.poisoned = Some(format!("appending record {} failed: {error}", record.seq));
            return Err(error);
        }
        journal.next_seq += 1;

        // Step 2: erase and record the applied sequence in one transaction.
        let erased = erase(&tx, &record)
            .and_then(|()| set_applied_seq(&tx, record.seq))
            .and_then(|()| tx.commit());
        if let Err(error) = erased {
            journal.poisoned = Some(format!(
                "record {} is journaled but its erase did not commit; reopening replays it",
                record.seq
            ));
            return Err(StoreError::Backend(format!(
                "erasing removal record {}: {error}",
                record.seq
            )));
        }
        Ok(record)
    }

    /// The sequence of the last journaled removal this database has applied;
    /// 0 when it has applied none.
    pub fn applied_removal_seq(&self) -> Result<u64, StoreError> {
        self.with_conn(read_applied_seq)
    }
}

/// The append side of an open store's journal: where it is, the sequence the
/// next record takes, and why removals are refused, once they are.
pub(super) struct JournalWriter {
    path: PathBuf,
    next_seq: u64,
    poisoned: Option<String>,
}

/// What [`replay_journal`] does when the journal file does not exist and the
/// database has applied no removal.
enum IfMissing {
    /// Create it, with the handler's backfill: the module is adopting it.
    Create,
    /// Leave it missing: a scratch copy is being prepared, and the module
    /// adopts the journal itself on its next open.
    Accept,
}

/// Bring `store`'s database up to the journal beside `journal_db_path` (the
/// database path whose directory holds `removals/`), checking it against
/// `owner`. Returns the journal's last sequence (0 for an empty or absent
/// journal).
fn replay_journal(
    store: &SqliteStore,
    journal_db_path: &str,
    owner: &StorageDescriptor,
    handler: &dyn RemovalsHandler,
    if_missing: IfMissing,
) -> Result<u64, StoreError> {
    let backend = |e: rusqlite::Error| StoreError::Backend(e.to_string());
    let dir = journal_dir(journal_db_path);
    let journal = journal_file(journal_db_path);
    let applied = store.with_conn(read_applied_seq)?;

    let records = match read_journal(&journal)? {
        Some(parsed) => {
            // The owner is the store's `(module_id, storage_namespace)` and holds
            // no machine id. The subc daemon gives every SQLite storage
            // descriptor the namespace "default" (`daemon_config.rs` in
            // `subc-daemon`), so a database and journal restored together onto a
            // new machine still name the same owner, and a correct journal is
            // never refused here.
            check_journal_owner(parsed.owner, &owner.module_id, &owner.storage_namespace)?;
            let journal_seq = parsed.records.last().map_or(0, |r| r.seq);
            // Checked against the records that stay, so a final line whose
            // erase committed (only possible when the journal is older than the
            // database) is refused rather than dropped as torn.
            if journal_seq < applied {
                return Err(StoreError::RemovalsBehindStore {
                    journal_seq,
                    store_seq: applied,
                });
            }
            if let Some(why) = &parsed.torn_tail {
                eprintln!(
                    "warning: removals journal {} ended in a torn record ({why}); dropping it, \
                     since its erase never ran",
                    journal.display()
                );
                truncate_journal(&journal, parsed.valid_len)?;
            }
            parsed.records
        }
        None if applied > 0 => return Err(StoreError::RemovalsMissing { store_seq: applied }),
        None => match if_missing {
            IfMissing::Accept => return Ok(0),
            IfMissing::Create => {
                let backfill = store.with_conn(|conn| handler.backfill(conn))?;
                create_journal(&dir, owner, backfill)?
            }
        },
    };
    // Reapplied on every open, as for the database: a restored or copied journal
    // carries whatever modes it arrived with.
    cortexkit_lease::create_private_dir(&dir).map_err(StoreError::Io)?;
    protect_file(&journal).map_err(StoreError::Io)?;

    for record in records.iter().filter(|r| r.seq > applied) {
        let mut conn = store.conn.lock().unwrap_or_else(|p| p.into_inner());
        let tx = conn
            .transaction_with_behavior(TransactionBehavior::Immediate)
            .map_err(backend)?;
        handler.replay(&tx, record).map_err(|e| {
            StoreError::Backend(format!("replaying removal record {}: {e}", record.seq))
        })?;
        set_applied_seq(&tx, record.seq).map_err(backend)?;
        tx.commit().map_err(backend)?;
    }
    Ok(records.last().map_or(0, |r| r.seq))
}

/// Refuse a store whose database path has a `removals/` directory beside it
/// with [`StoreError::RemovalsHandlerRequired`]. In-memory and URI databases
/// have no such directory.
pub(super) fn refuse_if_journal_dir(db_path: &str) -> Result<(), StoreError> {
    if is_memory_or_uri(db_path) {
        return Ok(());
    }
    match std::fs::symlink_metadata(journal_dir(db_path)) {
        Ok(_) => Err(StoreError::RemovalsHandlerRequired),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(()),
        // Anything else may hide a journal, so it refuses too.
        Err(error) => Err(StoreError::Io(error)),
    }
}

fn require_file_store(db_path: &str) -> Result<(), StoreError> {
    if is_memory_or_uri(db_path) {
        return Err(StoreError::Backend(format!(
            "{db_path:?} is an in-memory database or a SQLite URI; a removals journal \
             needs a file-backed store"
        )));
    }
    Ok(())
}

fn journal_dir(db_path: &str) -> PathBuf {
    parent_dir(db_path).join(DIR_NAME)
}

fn journal_file(db_path: &str) -> PathBuf {
    journal_dir(db_path).join(FILE_NAME)
}

fn unreadable(path: &Path, what: impl std::fmt::Display) -> StoreError {
    StoreError::RemovalsUnreadable {
        detail: format!("{}: {what}", path.display()),
    }
}

/// The database's applied removal sequence; 0 when the table that holds it
/// was never created.
pub(super) fn read_applied_seq(conn: &Connection) -> rusqlite::Result<u64> {
    let exists: bool = conn.query_row(
        "SELECT EXISTS(SELECT 1 FROM sqlite_schema
         WHERE type = 'table' AND name = 'cortexkit_removals')",
        [],
        |row| row.get(0),
    )?;
    if !exists {
        return Ok(0);
    }
    let seq: Option<i64> = conn
        .query_row(
            "SELECT applied_removal_seq FROM cortexkit_removals WHERE id = 1",
            [],
            |row| row.get(0),
        )
        .optional()?;
    Ok(seq.map_or(0, |seq| seq.max(0) as u64))
}

fn set_applied_seq(tx: &Transaction<'_>, seq: u64) -> rusqlite::Result<()> {
    let seq =
        i64::try_from(seq).map_err(|_| rusqlite::Error::IntegralValueOutOfRange(0, i64::MAX))?;
    tx.execute_batch(
        "CREATE TABLE IF NOT EXISTS cortexkit_removals (
             id INTEGER PRIMARY KEY CHECK (id = 1),
             applied_removal_seq INTEGER NOT NULL
         )",
    )?;
    tx.execute(
        "INSERT INTO cortexkit_removals (id, applied_removal_seq) VALUES (1, ?1)
         ON CONFLICT(id) DO UPDATE SET applied_removal_seq = excluded.applied_removal_seq",
        rusqlite::params![seq],
    )?;
    Ok(())
}

fn now_ms() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_millis() as u64)
        .unwrap_or(0)
}

// ---- On-disk format -------------------------------------------------------

impl RemovalRecord {
    /// The record's compact JSON without its `crc` field, in the fixed key
    /// order `seq`, `kind`, `ids`, `at_ms`: the bytes the crc covers.
    fn unsigned_json(&self) -> String {
        format!(
            "{{\"seq\":{},\"kind\":{},\"ids\":{},\"at_ms\":{}}}",
            self.seq,
            serde_json::Value::from(self.kind.as_str()),
            serde_json::Value::from(self.ids.clone()),
            self.at_ms
        )
    }

    /// The record's journal line, `crc` last, ending in a line feed.
    fn to_line(&self) -> String {
        let unsigned = self.unsigned_json();
        let crc = crc32c(unsigned.as_bytes());
        // Replace the closing brace with the crc field and close it again.
        let open = &unsigned[..unsigned.len() - 1];
        format!("{open},\"crc\":\"{crc:08x}\"}}\n")
    }
}

fn header_line(module_id: &str, storage_namespace: &str) -> String {
    let header = serde_json::json!({
        "format": FORMAT,
        "owner": { "module_id": module_id, "storage_namespace": storage_namespace },
    });
    format!("{header}\n")
}

/// The header's `(module_id, storage_namespace)` owner.
fn parse_header(line: &[u8]) -> Result<(String, String), String> {
    let value: serde_json::Value =
        serde_json::from_slice(line).map_err(|e| format!("header is not JSON: {e}"))?;
    match value.get("format").and_then(serde_json::Value::as_u64) {
        Some(FORMAT) => {}
        Some(other) => return Err(format!("header format {other} is not supported")),
        None => return Err("header has no format".into()),
    }
    let owner = value.get("owner").ok_or("header has no owner")?;
    let field = |name: &str| {
        owner
            .get(name)
            .and_then(serde_json::Value::as_str)
            .map(str::to_owned)
            .ok_or_else(|| format!("header owner has no {name}"))
    };
    Ok((field("module_id")?, field("storage_namespace")?))
}

/// One record line, with its crc checked. Any field missing, of the wrong type
/// or extra, or a crc that does not match, is an error.
fn parse_record(line: &[u8]) -> Result<RemovalRecord, String> {
    let value: serde_json::Value =
        serde_json::from_slice(line).map_err(|e| format!("not JSON: {e}"))?;
    let object = value.as_object().ok_or("not a JSON object")?;
    if object.len() != 5 {
        return Err(format!("{} fields where 5 were expected", object.len()));
    }
    let number = |name: &str| {
        object
            .get(name)
            .and_then(serde_json::Value::as_u64)
            .ok_or_else(|| format!("no unsigned {name}"))
    };
    let text = |name: &str| {
        object
            .get(name)
            .and_then(serde_json::Value::as_str)
            .ok_or_else(|| format!("no string {name}"))
    };
    let ids = object
        .get("ids")
        .and_then(serde_json::Value::as_array)
        .ok_or("no ids array")?
        .iter()
        .map(|id| {
            id.as_str()
                .map(str::to_owned)
                .ok_or("an id is not a string")
        })
        .collect::<Result<Vec<_>, _>>()?;
    let record = RemovalRecord {
        seq: number("seq")?,
        kind: text("kind")?.to_owned(),
        ids,
        at_ms: number("at_ms")?,
    };
    let crc = text("crc")?;
    let expected = format!("{:08x}", crc32c(record.unsigned_json().as_bytes()));
    if crc != expected {
        return Err(format!("crc {crc} does not match the record's {expected}"));
    }
    Ok(record)
}

/// A journal file, checked.
struct ParsedJournal {
    owner: (String, String),
    /// Bytes of the header line, line feed included.
    header_len: usize,
    /// Every complete, valid record, in order; their seqs are 1, 2, 3, ….
    records: Vec<RemovalRecord>,
    /// Bytes of the header and those records: where a torn final line starts.
    valid_len: usize,
    /// Why the final line was judged torn, when it was.
    torn_tail: Option<String>,
}

/// Check a journal's bytes without changing the file.
///
/// The final line is torn, and left for the caller to drop, when it has no
/// line feed (an append cut short) or does not parse or match its crc (an
/// append whose bytes never reached the disk). Any such line before the final
/// one is corruption. A record that parses and matches its crc but breaks the
/// sequence (a gap or a repeat) is corruption wherever it is: a torn write
/// can't produce one.
fn parse_journal(path: &Path, bytes: &[u8]) -> Result<ParsedJournal, StoreError> {
    let header_end = bytes
        .iter()
        .position(|&b| b == b'\n')
        .ok_or_else(|| unreadable(path, "no complete header line"))?;
    let owner = parse_header(&bytes[..header_end]).map_err(|why| unreadable(path, why))?;

    let mut records: Vec<RemovalRecord> = Vec::new();
    let mut pos = header_end + 1;
    let mut torn_tail = None;
    while pos < bytes.len() {
        let rest = &bytes[pos..];
        let Some(line_len) = rest.iter().position(|&b| b == b'\n') else {
            torn_tail = Some("the final line has no line end".to_owned());
            break;
        };
        let next = pos + line_len + 1;
        let expected = records.len() as u64 + 1;
        match parse_record(&rest[..line_len]) {
            Ok(record) if record.seq == expected => records.push(record),
            Ok(record) => {
                let problem = if record.seq > expected {
                    "a sequence gap"
                } else {
                    "a repeated sequence"
                };
                return Err(unreadable(
                    path,
                    format!(
                        "{problem}: the record at byte {pos} has seq {} where {expected} was \
                         expected",
                        record.seq
                    ),
                ));
            }
            Err(why) if next == bytes.len() => {
                torn_tail = Some(why);
                break;
            }
            Err(why) => {
                return Err(unreadable(
                    path,
                    format!("the record at byte {pos} is damaged: {why}"),
                ))
            }
        }
        pos = next;
    }
    Ok(ParsedJournal {
        owner,
        header_len: header_end + 1,
        records,
        valid_len: pos,
        torn_tail,
    })
}

/// The journal at `path`, checked, or `None` when the file does not exist.
fn read_journal(path: &Path) -> Result<Option<ParsedJournal>, StoreError> {
    match std::fs::read(path) {
        Ok(bytes) => parse_journal(path, &bytes).map(Some),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(None),
        Err(error) => Err(unreadable(path, error)),
    }
}

fn check_journal_owner(
    (journal_module_id, journal_storage_namespace): (String, String),
    module_id: &str,
    storage_namespace: &str,
) -> Result<(), StoreError> {
    if journal_module_id != module_id || journal_storage_namespace != storage_namespace {
        return Err(StoreError::RemovalsOwnerMismatch {
            journal_module_id,
            journal_storage_namespace,
            expected_module_id: module_id.to_owned(),
            expected_storage_namespace: storage_namespace.to_owned(),
        });
    }
    Ok(())
}

// ---- File operations ------------------------------------------------------

/// Append one record and sync the file before returning.
fn append_record(path: &Path, record: &RemovalRecord) -> Result<(), StoreError> {
    let line = record.to_line();
    // Not `create`: the journal exists from the open that adopted it, and one
    // that has vanished since must not be silently started over at this seq.
    let mut file = OpenOptions::new()
        .append(true)
        .open(path)
        .map_err(StoreError::Io)?;
    file.write_all(line.as_bytes()).map_err(StoreError::Io)?;
    file.sync_all().map_err(StoreError::Io)
}

/// Cut a torn final line off, and sync, so the next append starts on a clean
/// line.
fn truncate_journal(path: &Path, valid_len: usize) -> Result<(), StoreError> {
    let file = OpenOptions::new()
        .write(true)
        .open(path)
        .map_err(StoreError::Io)?;
    file.set_len(valid_len as u64).map_err(StoreError::Io)?;
    file.sync_all().map_err(StoreError::Io)
}

/// Create `dir` and its journal, holding the header and one record per
/// `backfill` entry. Returns those records.
fn create_journal(
    dir: &Path,
    owner: &StorageDescriptor,
    backfill: Vec<Removal>,
) -> Result<Vec<RemovalRecord>, StoreError> {
    let at_ms = now_ms();
    let records: Vec<RemovalRecord> = backfill
        .into_iter()
        .zip(1u64..)
        .map(|(removal, seq)| RemovalRecord {
            seq,
            kind: removal.kind,
            ids: removal.ids,
            at_ms,
        })
        .collect();
    let mut contents = header_line(&owner.module_id, &owner.storage_namespace);
    for record in &records {
        contents.push_str(&record.to_line());
    }

    cortexkit_lease::create_private_dir(dir).map_err(StoreError::Io)?;
    // The store directory now names `removals/`; make that name durable too.
    if let Some(store_dir) = dir.parent() {
        sync_dir(store_dir).map_err(StoreError::Io)?;
    }
    write_journal_atomically(dir, contents.as_bytes())?;
    Ok(records)
}

/// Write `contents` as `dir`'s journal: to a temporary file, synced, then
/// renamed over the journal, and the directory synced, so the journal is never
/// seen half-written and the new name survives power loss.
fn write_journal_atomically(dir: &Path, contents: &[u8]) -> Result<(), StoreError> {
    let temp = dir.join(TEMP_NAME);
    let mut options = OpenOptions::new();
    options.write(true).create(true).truncate(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.mode(0o600);
    }
    {
        let mut file = options.open(&temp).map_err(StoreError::Io)?;
        file.write_all(contents).map_err(StoreError::Io)?;
        file.sync_all().map_err(StoreError::Io)?;
    }
    // A temporary file left by an earlier crash keeps its old mode; narrow it.
    protect_file(&temp).map_err(StoreError::Io)?;
    std::fs::rename(&temp, dir.join(FILE_NAME)).map_err(StoreError::Io)?;
    sync_dir(dir).map_err(StoreError::Io)
}

/// Sync a directory, so a name just created in it survives power loss.
///
/// Unix only. EINVAL (reported as `InvalidInput`) means the filesystem does
/// not support syncing a directory, which is not an error here; the same rule
/// as `cortexkit_lease::sync_dir`.
#[cfg(unix)]
fn sync_dir(dir: &Path) -> std::io::Result<()> {
    match std::fs::File::open(dir)?.sync_all() {
        Err(error)
            if error.kind() == std::io::ErrorKind::InvalidInput
                && error.raw_os_error().is_some() =>
        {
            Ok(())
        }
        result => result,
    }
}

/// On Windows, `std` can't open a directory as a `File`, so there is no
/// directory sync to call. The guarantee holds anyway. NTFS records file creation and renames in
/// its own metadata log, and syncing the journal file itself (`sync_all`, which
/// is `FlushFileBuffers`) flushes that file's metadata as well as its data.
/// And if a power loss still lost the journal's name, the store would refuse
/// with [`StoreError::RemovalsMissing`] rather than serve removed data: the
/// erase that would make the journal matter commits only after this returns.
#[cfg(not(unix))]
fn sync_dir(dir: &Path) -> std::io::Result<()> {
    let _ = dir;
    Ok(())
}

/// CRC-32C (Castagnoli): the reflected polynomial 0x82F63B78, initial value
/// and final XOR all ones.
fn crc32c(bytes: &[u8]) -> u32 {
    const TABLE: [u32; 256] = {
        let mut table = [0u32; 256];
        let mut i = 0;
        while i < 256 {
            let mut crc = i as u32;
            let mut bit = 0;
            while bit < 8 {
                crc = if crc & 1 != 0 {
                    (crc >> 1) ^ 0x82F6_3B78
                } else {
                    crc >> 1
                };
                bit += 1;
            }
            table[i] = crc;
            i += 1;
        }
        table
    };
    let mut crc = !0u32;
    for &byte in bytes {
        crc = TABLE[((crc ^ u32::from(byte)) & 0xff) as usize] ^ (crc >> 8);
    }
    !crc
}

#[cfg(test)]
mod format_tests {
    use super::*;

    #[test]
    fn crc32c_matches_the_standard_check_value() {
        assert_eq!(crc32c(b"123456789"), 0xE306_9283);
        assert_eq!(crc32c(b""), 0);
    }

    #[test]
    fn a_record_line_round_trips_and_its_crc_covers_every_field() {
        let record = RemovalRecord {
            seq: 7,
            kind: "session \"quoted\"".into(),
            ids: vec!["a".into(), "b\nc".into()],
            at_ms: 1_700_000_000_000,
        };
        let line = record.to_line();
        assert!(line.ends_with("}\n") && line.matches('\n').count() == 1);
        assert!(line.starts_with("{\"seq\":7,\"kind\":"));
        assert_eq!(parse_record(line.trim_end().as_bytes()), Ok(record.clone()));

        let tampered = line.replace("\"seq\":7", "\"seq\":8");
        assert!(parse_record(tampered.trim_end().as_bytes())
            .unwrap_err()
            .contains("crc"));
    }

    #[test]
    fn the_header_round_trips() {
        let line = header_line("module", "default");
        assert_eq!(
            parse_header(line.trim_end().as_bytes()),
            Ok(("module".to_owned(), "default".to_owned()))
        );
        assert!(parse_header(br#"{"format":2,"owner":{}}"#)
            .unwrap_err()
            .contains("format 2"));
    }
}
