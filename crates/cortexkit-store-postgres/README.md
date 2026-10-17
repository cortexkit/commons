# cortexkit-store-postgres

PostgreSQL module storage with native single-writer leases and versioned migrations.

This crate is not published on crates.io. With a checkout at `../commons`, add:

```toml
[dependencies]
cortexkit-store-postgres = { path = "../commons/crates/cortexkit-store-postgres" }
```

See the [crate source](src/lib.rs) for connection, migration, and lease APIs, and [CONTRIBUTING](../../CONTRIBUTING.md#build-and-test) for live PostgreSQL tests.
