# cortexkit-lease

Durable single-writer leases using OS advisory locks. Persisted epoch numbers let storage backends reject writes from a superseded writer.

Add to `Cargo.toml`:

```toml
[dependencies]
cortexkit-lease = "0.1.4"
```

Start with `LeaseKey` and `LeaseStore`; see the [crate source](src/lib.rs) for acquisition and fencing semantics, owner-only file/directory helpers (`protect_file`, `create_private_dir`), and durable file replacement (`durable_replace`).
