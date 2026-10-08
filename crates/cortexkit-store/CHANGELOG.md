# Changelog

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
