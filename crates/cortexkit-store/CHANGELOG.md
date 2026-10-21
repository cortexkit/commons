# Changelog

## 0.2.5

- Backport the fence-epoch recovery fix from cortexkit-store 0.3.3 to the 0.2
  release line. A `store.db` that arrives without its `*.lease` files (a backup
  restored onto another machine, a copy placed by hand, lease files deleted)
  now opens and takes fenced writes at once. Previously a fresh lease started
  at epoch 1 while the database still carried the old fence epoch N, so every
  `with_conn_fenced` write was refused with `StoreError::Fenced` until N
  restarts had passed.
- `open_sqlite` and `open_sqlite_with` read the database's fence epoch while
  holding the exclusive lease, before WAL is enabled. When it is at or above
  the lease's epoch, the lease's persisted epoch is raised to one above it
  (through `LeaseHandle::raise_epoch`, written and synced under the held lock)
  and the store uses it. Epochs are never lowered, so an older writer is still
  fenced out.
- If a raise is needed and the lease implementation cannot raise, the open is
  refused with `StoreError::Lease` instead of serving a store whose fenced
  writes would all fail. A database that has never taken a fenced write is not
  changed: the check only reads, and the fence table is still created by the
  first fenced write.
- Requires `cortexkit-lease` 0.1.4. No ownership checks, maintenance leases, or
  other 0.3-only behavior are introduced.

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
