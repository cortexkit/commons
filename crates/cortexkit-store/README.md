# cortexkit-store

SQLite module storage: opening databases, applying versioned migrations, and holding a single-writer lease.

Add to `Cargo.toml`:

```toml
[dependencies]
cortexkit-store = "0.2.5"
```

The crate re-exports `StorageDescriptor`. See the [crate source](src/lib.rs) for opening and reading storage.

A `store.db` moved to another machine (a backup restored, a copy placed by hand) opens and takes fenced writes without its `*.lease` files. Every writer open reads the epoch the database's last fenced write stamped and, when it is at or above the lease's epoch, raises the lease's persisted epoch to one above it before serving. Epochs are never lowered, so a lingering older writer is still fenced out.
