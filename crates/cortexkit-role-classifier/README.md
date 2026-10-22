# cortexkit-role-classifier

Wire types, validation, and vectors for `classifier/v1` requests and replies.

Add to `Cargo.toml`:

```toml
[dependencies]
cortexkit-role-classifier = "0.2.0"
```

To include the shared vector bytes in your build, enable `test-vectors`:

```toml
[dev-dependencies]
cortexkit-role-classifier = { version = "0.2.0", features = ["test-vectors"] }
```

The feature exposes `cortexkit_role_classifier::vectors::FILES` and a constant
for each file. The vectors live in `test-vectors/classifier-v1/` and ship
inside the crate package, pinned by `SHA256SUMS`. The crate's own vector tests
need the feature: `cargo test -p cortexkit-role-classifier --features test-vectors`.

The [contract](CONTRACT.md) is normative, with the batch rules it shares
with the other model-batch roles in
[`cortexkit-model-batch`'s RULES.md](../cortexkit-model-batch/RULES.md). See the [conformance runner](../cortexkit-role-classifier-conformance/README.md) and [role/conformance guide](../../docs/role-contracts.md) for implementation testing.
