# cortexkit-store-types

> [!WARNING]
> **Alpha.** cortexkit-store-types is in early development and is not yet released for general use.
> Breaking changes will happen between versions, including changes to its settings,
> stored data and interfaces. Pin an exact version if you depend on it.

Dependency-light storage descriptors, backend choices, and isolation types for module storage.

Add to `Cargo.toml`:

```toml
[dependencies]
cortexkit-store-types = "0.2.2"
```

Use `StorageDescriptor` to describe storage without depending on a backend implementation. See the [crate source](src/lib.rs).
