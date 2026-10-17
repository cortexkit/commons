# cortexkit-model-catalog

Shared model catalog wire types, with exact integer conversion of pricing rates to nanodollars per million tokens.

This crate is not published on crates.io. With a checkout at `../commons`, add:

```toml
[dependencies]
cortexkit-model-catalog = { path = "../commons/crates/cortexkit-model-catalog" }
```

See the [crate source](src/lib.rs) for catalog parsing and `RateNanosPerMtok`.
