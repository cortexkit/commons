# cortexkit-paths

> [!WARNING]
> **Alpha.** cortexkit-paths is in early development and is not yet released for general use.
> Breaking changes will happen between versions, including changes to its settings,
> stored data and interfaces. Pin an exact version if you depend on it.

Canonical project-root identities and absolute user-directory paths, including cross-platform path normalization.

Add to `Cargo.toml`:

```toml
[dependencies]
cortexkit-paths = "0.1.2"
```

The public API includes `ProjectRootId` for project identity. See the [crate source](src/lib.rs) for path-resolution helpers.
