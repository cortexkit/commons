# cortexkit-role-tool-provider

Wire types and vectors for `tool-provider/v1`: tool catalogs, calls, results, and withdrawal.

Add to `Cargo.toml`:

```toml
[dependencies]
cortexkit-role-tool-provider = "0.9.3"
```

To include the shared vector bytes in your build, enable `test-vectors`:

```toml
[dependencies]
cortexkit-role-tool-provider = { version = "0.9.3", features = ["test-vectors"] }
```

The feature exposes `cortexkit_role_tool_provider::vectors::FILES` and a
constant for each file, including `held-reply-v1.json`. The vectors ship inside
the crate package, pinned by `SHA256SUMS`.

The [contract](CONTRACT.md) is normative. See the [conformance runner](../cortexkit-role-tool-provider-conformance/README.md) and [role/conformance guide](../../docs/role-contracts.md) for implementation testing.

Consumers opt into `content/v1` on the route bind. Understanding held first
replies additionally requires `held-reply/v1`; providers use
`consumer_reads_held_reply_v1` to check both declarations. `ToolResult::held()`
exposes the interim marker: the answer arrives later through `late_results`.
