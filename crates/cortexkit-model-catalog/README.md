# cortexkit-model-catalog

> [!WARNING]
> **Alpha.** cortexkit-model-catalog is in early development and is not yet released for general use.
> Breaking changes will happen between versions, including changes to its settings,
> stored data and interfaces. Pin an exact version if you depend on it.

Shared model catalog wire types, with exact integer conversion of pricing rates to nanodollars per million tokens.

This crate is not published on crates.io. With a checkout at `../commons`, add:

```toml
[dependencies]
cortexkit-model-catalog = { path = "../commons/crates/cortexkit-model-catalog" }
```

See the [crate source](src/lib.rs) for catalog parsing and `RateNanosPerMtok`.
