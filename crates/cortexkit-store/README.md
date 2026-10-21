# cortexkit-store

SQLite module storage: opening databases, applying versioned migrations, and holding a single-writer lease.

Add to `Cargo.toml`:

```toml
[dependencies]
cortexkit-store = "0.3.2"
```

The crate re-exports `StorageDescriptor`. See the [crate source](src/lib.rs) for opening and reading storage.

File-backed writer opens take the single-writer lease first, then record a `(module_id, storage_namespace)` owner in `cortexkit_owner`; a different owner is refused with `StoreError::OwnershipMismatch`. A second opener of a held store is refused with `StoreError::Lease` before SQLite opens the database. Read-only operations do not claim or check ownership; in-memory databases and SQLite URIs are exempt. An intentional ownership change requires stopping all writers and clearing the owner row; there is no transfer API.

Maintenance tools that must keep a module's writer out without opening its database (a backup restore, for example) call `acquire_store_lease(&descriptor)`. It takes the same lease `open_sqlite` takes and returns a `StoreLease` guard that releases it on drop. While it is held, `StoreLease::open_scratch(path, options)` opens a separate database file, such as a copy to be migrated and verified before it replaces the live one, with the store's usual pragmas and file modes and without a second lease.
