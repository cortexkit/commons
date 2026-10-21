# cortexkit-store

SQLite module storage: opening databases, applying versioned migrations, and holding a single-writer lease.

Add to `Cargo.toml`:

```toml
[dependencies]
cortexkit-store = "0.4.0"
```

The crate re-exports `StorageDescriptor`. See the [crate source](src/lib.rs) for opening and reading storage.

File-backed writer opens take the single-writer lease first, then record a `(module_id, storage_namespace)` owner in `cortexkit_owner`; a different owner is refused with `StoreError::OwnershipMismatch`. A second opener of a held store is refused with `StoreError::Lease` before SQLite opens the database. Read-only operations do not claim or check ownership; in-memory databases and SQLite URIs are exempt. An intentional ownership change requires stopping all writers and clearing the owner row; there is no transfer API.

Maintenance tools that must keep a module's writer out without opening its database (a backup restore, for example) call `acquire_store_lease(&descriptor)`. It takes the same lease `open_sqlite` takes and returns a `StoreLease` guard that releases it on drop. While it is held, `StoreLease::open_scratch(path, options)` opens a separate database file, such as a copy to be migrated and verified before it replaces the live one, with the store's usual pragmas and file modes and without a second lease.

A `store.db` moved to another machine (a backup restored, a copy placed by hand) opens and takes fenced writes without its `*.lease` files. Every writer open, and `open_scratch`, reads the epoch the database's last fenced write stamped and, when it is at or above the lease's epoch, raises the lease's persisted epoch to one above it before serving. Epochs are never lowered, so a lingering older writer is still fenced out.

A store's database must be reachable through exactly one path. The lease lives beside the database, so the same `store.db` opened through a second path (a hard link in another directory, or a second mount of the same disk) takes a different lease. Two writers can then both hold one, and the fence does not stop them. The store does not detect this layout.

## Removals journal

A module's tombstones live inside `store.db`, so an older copy of the database (an Engram restore, a hand-placed export, Time Machine, a new machine) would bring deleted data back. A module that deletes data keeps the evidence of each removal outside the database, in `<store dir>/removals/journal` (directory mode 0700, file mode 0600), and every open replays the records the database has not applied yet before it serves.

- Open with `open_sqlite_with_removals(&descriptor, options, &handler)`. `handler` implements `RemovalsHandler::replay(&Transaction, &RemovalRecord)`, which erases a record's ids. It must touch the database only and be idempotent; cleanup outside the database goes through the module's own receipts and retries.
- Remove with `store.remove(kind, ids, erase)`. The record is appended and synced first; `erase` then runs in the transaction that records the removal as applied. `kind` is the module's own word ("session", "transcript"); one record may carry many ids.
- Records are never compacted: an older generation can come back at any time.
- The open refuses to serve when the journal is damaged (`RemovalsUnreadable`), ends before the database's applied removals (`RemovalsBehindStore`), is missing while the database has applied removals (`RemovalsMissing`), or names another owner (`RemovalsOwnerMismatch`). A torn final line, left by a crash during an append, is dropped: its erase never ran.
- Once a store has a `removals/` directory or has applied a removal, a plain `open_sqlite` or `StoreLease::open_scratch` is refused with `RemovalsHandlerRequired`.
- A restore tool holds `acquire_store_lease` and opens its copy with `StoreLease::open_scratch_with_removals(path, options, &handler)`, which replays the live store's journal onto the copy before it returns, and refuses a copy that is ahead of that journal.

### Adopting the journal

1. **Backfill.** The first `open_sqlite_with_removals` of a store creates the journal. A module that removed data before adopting has that evidence only inside its database, so it implements `RemovalsHandler::backfill` to return one `Removal` per existing tombstone. They are written in the same step that creates the journal, and replayed before the store serves.
2. **Back it up.** Add `<store dir>/removals/` to the module's `engram-catalog.json` as its own portable filetree entry, with `"capture": "after_databases"` on that entry. Don't exclude it: a dead disk would lose it, and a restore onto a new machine would bring back every removed item. The field makes Engram capture it after every database entry of the generation, so within one generation the journal is never behind the store.
3. **Restore contract.** In the module's Engram restore contract (the table of what each backed-up path holds and how a restore treats it), the row for the journal says "evidence of removal: <module>/removals, captured, replayed on open". A restore installs the newest journal it has and never one older than the restored database. A journal restored together with its database from the same generation satisfies that.

**Adoption gate.** A module may declare `"capture": "after_databases"` at any time, but must not rely on the journal's restore guarantee until every machine that backs the module up runs an Engram release that honours the field. Engram releases from 2026-10-21 on honour it. An older Engram ignores the field and captures filetree entries before page-db entries (databases Engram captures page by page), so for such a store it would capture `removals/` first, and a restore of that generation could fail with `RemovalsBehindStore`.

### Changing the owner

The journal header records the store's `(module_id, storage_namespace)` owner. That value holds no machine id, so a restore onto a new machine matches. A module that changes its owner on purpose stops every writer, clears the database's `cortexkit_owner` row, takes `acquire_store_lease` for the new descriptor, and calls `RemovalJournal::rewrite_owner(&lease, previous_module_id, previous_storage_namespace)`, which rewrites the header and keeps every record.
