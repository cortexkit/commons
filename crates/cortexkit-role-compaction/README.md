# cortexkit-role-compaction

Public wire types for **compaction-provider/v2**, served by a compaction
provider, and **compaction-runner/v1**, served by a runner. Providers, runners,
and view libraries share this contract without depending on an implementation.

```toml
[dependencies]
cortexkit-role-compaction = "0.3.2"
```

`Delivered` adds optional `send_id`, ordered `forms` and ride-text `tokens`
without changing older omitted-field bytes. Counts follow the existing
tokenizer-family rules and remain outside checksums. Shared ride rendering
is available through `cortexkit_role_compaction::rides`; views suppress rides
when replacing output unless they explicitly choose `delivered` (CONTRACT W6).
Runners opt into checksummed view anchors through
Setup's `view_anchor`; without it providers preserve the pre-anchor view shape.

See [CONTRACT.md](CONTRACT.md) for the protocol rules and
[`test-vectors/compaction-v2`](test-vectors/compaction-v2) for wire examples.
The earlier `compaction-provider/v1` protocol remains in
`cortexkit-role-compaction-provider` during migration.
