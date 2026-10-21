# Changelog

## 0.3.3

- A `store.db` that arrives without its `*.lease` files (a backup restored onto
  another machine, a copy placed by hand, lease files deleted) now opens and
  takes fenced writes at once. Previously a fresh lease started at epoch 1
  while the database still carried the old fence epoch N, so every
  `with_conn_fenced` write was refused with `StoreError::Fenced` until N
  restarts had passed.
- `open_sqlite`, `open_sqlite_with` and `StoreLease::open_scratch` read the
  database's fence epoch beside the owner check, before WAL is enabled. When it
  is at or above the lease's epoch, the lease's persisted epoch is raised to
  one above it (through `LeaseHandle::raise_epoch`, written and synced under
  the held exclusive lock) and the store uses it. Epochs are never lowered, so
  an older writer is still fenced out. For `open_scratch` the raise applies to
  the shared lease, so `StoreLease::epoch` then reports the raised value.
- If a raise is needed and the lease implementation cannot raise, the open is
  refused with `StoreError::Lease` (an `Unsupported` I/O error naming the
  database and both epochs) instead of serving a store whose fenced writes
  would all fail.
- A database that has never taken a fenced write is not changed: the check
  only reads, and the fence table is still created by the first fenced write.
- Requires `cortexkit-lease` 0.1.4. A patch release: no public API changed.

## 0.3.2

- `open_sqlite` now takes the single-writer lease before it opens the
  database, as its documentation always said. Previously it opened SQLite and
  checked the recorded owner first, so a second opener of a held store read the
  live database (and could create or write its `-wal` and `-shm`) before being
  refused. A refused same-owner opener now never opens SQLite.
- An opener under a different `(module_id, storage_namespace)` derives a
  different lease key, so it now takes its own lease, is refused by the owner
  check with `StoreError::OwnershipMismatch`, and releases that lease before
  returning. Its lease file stays in the store's directory, as lease files
  always do; before this release such an opener created no lease file.
- Add `acquire_store_lease(&StorageDescriptor) -> Result<StoreLease, StoreError>`:
  takes the same lease as `open_sqlite` (same directory and key, derived by the
  same code) without opening SQLite, writing `cortexkit_owner` or touching the
  database files, so a maintenance tool such as a restore can keep the module's
  writer out. `StoreLease` releases on drop and exposes `epoch()`. A refused
  acquire never changes the persisted epoch or creates a file; it can only
  reapply owner-only permissions (see the function's documentation).
- Add `StoreLease::open_scratch(path, options)`: opens a different database
  file, such as a temporary copy being prepared to replace the live store, with
  the same pragmas and file modes, checking its owner against the lease's
  descriptor. It takes no second lease and never opens the live database; the
  lease stays held until the guard and every scratch store are dropped.
- A patch release: the new API is additive, and the reordering is a safety fix
  that existing `0.3` dependents should pick up with `cargo update`.
- Fix two documentation links to `with_conn` that did not resolve.

## 0.3.1

- Close pooled read-only connections before checkpointing and closing the SQLite
  writer. Previously, the writer closed first, leaving `-wal` and `-shm` beside
  the main database; restoring a saved `store.db` could then replay those newer
  WAL frames and silently undo the restore or mix database states.

## 0.3.0

- File-backed SQLite writer opens now claim a persistent
  `(module_id, storage_namespace)` owner in `cortexkit_owner` inside an immediate
  transaction, before taking a lease. Different owners are refused even through
  symlinks or hard links; databases without an owner are claimed on their next
  writer open, and matching owners can reopen normally.
- Checkpoint the owner row only on its first claim, on a best-effort basis. A
  busy, incomplete or failed checkpoint emits a warning without refusing startup;
  a hard-link alias may not see the new owner until the next checkpoint. Opens
  for an already-recorded owner do not checkpoint, preserving restart availability
  with active read snapshots and the lease error for competing matching writers.
- Add `StoreError::OwnershipMismatch`, reporting the recorded and requested
  owners. `StoreError` is not `#[non_exhaustive]`, so this is a breaking change
  for downstream exhaustive matches and requires the minor version bump.
- Read-only operations do not claim or check ownership. In-memory and SQLite URI
  databases remain exempt. Intentionally assigning a file to a new owner requires
  clearing its `cortexkit_owner` row with all writers stopped; no ownership-reset
  API is provided.
- An open whose owner is already recorded only reads it, without SQLite's write
  lock, so a second same-owner process reaches the lease refusal even while the
  live writer is mid-transaction. A first owner claim checkpoints the row into
  the main file on a best-effort basis: a busy checkpoint logs a warning instead
  of failing startup.
- On a fresh database, the opener that wins the owner claim retries its switch to
  WAL, within the busy timeout, while a refused opener finishes its transaction,
  so the rightful owner does not fail to start.

## 0.2.4

- On Windows, store files and the store's own directory are now owner-only:
  only the user running the program can open them, as with mode 0600 and
  0700 on Unix.
- In-memory databases and SQLite URIs are no longer treated as file paths
  when protecting files. They name no file on disk, and on Windows their
  `:` and `?` made the open fail.

## 0.2.3

- Add opt-in `SqliteStore::with_read` for committed, consistent read-only snapshots
  without waiting on the writer mutex for file-backed WAL stores. Readers are
  opened lazily in a bounded pool (default: four); exhaustion returns a pool error
  after the busy timeout. In-memory stores use the existing writer mutex instead.
- Add `open_sqlite_with` and `SqliteOpenOptions` to configure the reader limit and
  the shared writer/reader busy timeout. Existing writer, migration, lease, and
  fence behavior remains unchanged.
