# Changelog

## 0.2.3

- Add opt-in `SqliteStore::with_read` for committed, consistent read-only snapshots
  without waiting on the writer mutex for file-backed WAL stores. Readers are
  opened lazily in a bounded pool (default: four); exhaustion returns a pool error
  after the busy timeout. In-memory stores use the existing writer mutex instead.
- Add `open_sqlite_with` and `SqliteOpenOptions` to configure the reader limit and
  the shared writer/reader busy timeout. Existing writer, migration, lease, and
  fence behavior remains unchanged.
