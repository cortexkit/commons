# cortexkit-role-compaction

Public wire types for **compaction-provider/v2**, served by a compaction
provider, and **compaction-runner/v1**, served by a runner. Providers, runners,
and view libraries share this contract without depending on an implementation.

```toml
[dependencies]
cortexkit-role-compaction = "0.1.0"
```

See [CONTRACT.md](CONTRACT.md) for the protocol rules and
[`test-vectors/compaction-v2`](test-vectors/compaction-v2) for wire examples.
The earlier `compaction-provider/v1` protocol remains in
`cortexkit-role-compaction-provider` during migration.
