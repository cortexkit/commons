# cortexkit-model-batch

The batch rules every CortexKit model-batch role shares (`classifier/v1`,
`extractor/v1`): the error vocabulary with the Cloudflare Workers AI code
table, batch identity and its canonical hashing, the item image shape and
its validation, the conservative cost estimate, and the absent-is-not-zero
and number-text helpers.

[`RULES.md`](RULES.md) is normative: it states each shared rule once, and
each role contract points to it. A role crate re-exports what it uses, so a
module depends on its role crate and rarely on this one directly.

```toml
[dependencies]
cortexkit-model-batch = "0.3.0"
```

To include the vector bytes in your build, enable `test-vectors`:

```toml
[dev-dependencies]
cortexkit-model-batch = { version = "0.3.0", features = ["test-vectors"] }
```

The feature exposes `cortexkit_model_batch::vectors::FILES` and a constant
for each file. The vectors live in `test-vectors/model-batch-v1/` and ship
inside the crate package, pinned by `SHA256SUMS`. The crate's own vector
tests need the feature: `cargo test -p cortexkit-model-batch --features
test-vectors`.
