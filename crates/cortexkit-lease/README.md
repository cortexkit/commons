# cortexkit-lease

Durable single-writer leases using OS advisory locks. Persisted epoch numbers let storage backends reject writes from a superseded writer.

Add to `Cargo.toml`:

```toml
[dependencies]
cortexkit-lease = "0.1.9"
```

Start with `LeaseKey` and `LeaseStore`; see the [crate source](src/lib.rs) for acquisition and fencing semantics, owner-only file/directory helpers (`protect_file`, `create_private_dir`), and durable file replacement (`durable_replace`).

Use `LeaseHandle::remove` on a held exclusive file lease. If you only know the
file name, pass it (not a path) to `FileLeaseStore::remove_if_free`. This method
does not wait; it returns `false` if the file is missing, held by any reader or
writer, or replaced after it was opened.

Use either method only after deleting the resource the lease guarded and its
stored data. Removing the file resets the fence epoch: the next writer starts at
1, so data stamped with a higher epoch would reject its writes as stale. All
acquirers must use 0.1.8 or newer before callers start removing leases: older
versions can lock a file they opened before it was removed and mistake it for
the current lease. On Windows, the filesystem must support POSIX delete
semantics; a refusal is returned as an I/O error without deleting the file.
