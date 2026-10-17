# cortexkit-cache-core

Keeps rendered prompt bytes stable across turns. Callers choose when to replace cached content; otherwise the core reuses the stored bytes without rendering again.

This crate is not published on crates.io. With a checkout at `../commons`, add:

```toml
[dependencies]
cortexkit-cache-core = { path = "../commons/crates/cortexkit-cache-core" }
```

See the [crate source](src/lib.rs) for the state machine and its `Action` values.
