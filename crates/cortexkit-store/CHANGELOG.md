# Changelog

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
