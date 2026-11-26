# cortexkit-cache-core

> [!WARNING]
> **Alpha.** cortexkit-cache-core is in early development and is not yet released for general use.
> Breaking changes will happen between versions, including changes to its settings,
> stored data and interfaces. Pin an exact version if you depend on it.

Keeps rendered prompt bytes stable across turns. Callers choose when to replace cached content; otherwise the core reuses the stored bytes without rendering again.

This crate is not published on crates.io. With a checkout at `../commons`, add:

```toml
[dependencies]
cortexkit-cache-core = { path = "../commons/crates/cortexkit-cache-core" }
```

See the [crate source](src/lib.rs) for the state machine and its `Action` values.
