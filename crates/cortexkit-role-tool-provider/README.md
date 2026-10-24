# cortexkit-role-tool-provider

Wire types and vectors for `tool-provider/v1`: tool catalogs, calls, results, and withdrawal.

Add to `Cargo.toml`:

```toml
[dependencies]
cortexkit-role-tool-provider = "0.12.0"
```

To include the shared vector bytes in your build, enable `test-vectors`:

```toml
[dependencies]
cortexkit-role-tool-provider = { version = "0.12.0", features = ["test-vectors"] }
```

The feature exposes `cortexkit_role_tool_provider::vectors::FILES` and a
constant for each file, including `held-reply-v1.json`. The vectors ship inside
the crate package, pinned by `SHA256SUMS`.

The [contract](CONTRACT.md) is normative. See the [conformance runner](../cortexkit-role-tool-provider-conformance/README.md) and [role/conformance guide](../../docs/role-contracts.md) for implementation testing.

Consumers opt into `content/v1` on the route bind. Understanding held first
replies additionally requires `held-reply/v1`; providers use
`consumer_reads_held_reply_v1` to check both declarations. `ToolResult::held()`
exposes the interim marker: the answer arrives later through `late_results`.
`result::is_person_wait` says whether the held reason means a person must act.

A provider lists the presets it serves in `CatalogAnswer.presets`
(`with_presets`), only in the answer to a catalog request without a preset;
`catalog::check_presets` validates the list. A caller sends a preset, on
`tool.catalog` or a tool call, only to a provider whose preset-less answer
lists it (`lists_preset`).

A system-text item's preset is optional (`SystemTextItem.preset` is an
`Option<String>`): without one, the provider answers with its default system
text. Send an item without a preset only to a provider on 0.12.0 or later;
an older one refuses it as `invalid_params`.
