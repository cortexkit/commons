# cortexkit-provider-usage

> [!WARNING]
> **Alpha.** cortexkit-provider-usage is in early development and is not yet released for general use.
> Breaking changes will happen between versions, including changes to its settings,
> stored data and interfaces. Pin an exact version if you depend on it.

Serde wire types for provider usage, quota windows, and account/reset metadata; no provider-fetching logic.

Add to `Cargo.toml`:

```toml
[dependencies]
cortexkit-provider-usage = "0.11.0"
```

See the [crate source](src/lib.rs) for `ProviderUsage`, `Usage`, and `RateWindow`.
