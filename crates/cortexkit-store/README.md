# cortexkit-store

SQLite module storage: opening databases, applying versioned migrations, and holding a single-writer lease.

Add to `Cargo.toml`:

```toml
[dependencies]
cortexkit-store = "0.3.0"
```

The crate re-exports `StorageDescriptor`. See the [crate source](src/lib.rs) for opening and reading storage.

File-backed writer opens record a `(module_id, storage_namespace)` owner in `cortexkit_owner`; a different owner is refused with `StoreError::OwnershipMismatch`. Read-only operations do not claim or check ownership; in-memory databases and SQLite URIs are exempt. An intentional ownership change requires stopping all writers and clearing the owner row; there is no transfer API.
