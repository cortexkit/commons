# cortexkit-lease

Durable single-writer leases using OS advisory locks. Persisted epoch numbers let storage backends reject writes from a superseded writer.

Add to `Cargo.toml`:

```toml
[dependencies]
cortexkit-lease = "0.1.8"
```

Start with `LeaseKey` and `LeaseStore`; see the [crate source](src/lib.rs) for acquisition and fencing semantics, owner-only file/directory helpers (`protect_file`, `create_private_dir`), and durable file replacement (`durable_replace`).

Use `LeaseHandle::remove` on a held exclusive file lease only after deleting
the resource it guarded and its stored data. Removing the file resets the fence
epoch: the next writer starts at 1, so data stamped with a higher epoch would
reject its writes as stale. All acquirers must use 0.1.8 or newer before callers
start removing leases. On Windows, the filesystem must support POSIX delete
semantics; a refusal is returned as an I/O error without deleting the file.
