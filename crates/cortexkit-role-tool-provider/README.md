# cortexkit-role-tool-provider

Wire types and vectors for `tool-provider/v1`: tool catalogs, calls, results, and withdrawal.

Add to `Cargo.toml`:

```toml
[dependencies]
cortexkit-role-tool-provider = "0.14.0"
```

To include the shared vector bytes in your build, enable `test-vectors`:

```toml
[dependencies]
cortexkit-role-tool-provider = { version = "0.14.0", features = ["test-vectors"] }
```

The feature exposes `cortexkit_role_tool_provider::vectors::FILES` and a
constant for each file, including `held-reply-v1.json` and `held-update-v1.json`. The vectors ship inside
the crate package, pinned by `SHA256SUMS`.

The [contract](CONTRACT.md) is normative. See the [conformance runner](../cortexkit-role-tool-provider-conformance/README.md) and [role/conformance guide](../../docs/role-contracts.md) for implementation testing.

Consumers opt into `tool-result/v1` on the route bind. Understanding held first
replies additionally requires `held-reply/v1`; providers use
`consumer_reads_held_reply_v1` to check both declarations. `ToolResult::held()`
exposes the interim marker: the answer arrives later through `late_results`.
`result::is_person_wait` says whether the held reason means a person must act.

Consumers that understand non-terminal held-reason updates also declare
`HELD_UPDATE_CAPABILITY` (`held-update/v1`). Readers opt in first: a provider
must not send a `kind: "held"` late entry until the call's bind declared all
three capabilities, and only if the call's first reply was held.
`consumer_reads_held_update_v1` checks the bind declarations. Providers construct
updates with `LateEntry::new_held` and validate before sending; decoding validates
received held updates. `LateEntry::held()` returns the validated `HeldReply`
payload, leaving other kinds' extension members uninterpreted.

A held update is recorded and acked, but is not an answer. Consumers apply reasons
in cursor order, using `result::is_person_wait` on the latest reason: `running`
ends person-wait accounting from the update's `settled_at`. A later `result`,
`not_started` or `expired` ends the call whatever its latest reason was. Consumers
choose their own time budgets. A decoder using the 0.13.2 `LateEntry` shape keeps
the payload and classifies the kind as `Unknown("held")`. That says nothing about
whether its consumer handles a non-terminal entry correctly, which is why providers
must wait for the new declaration rather than relying on open decoding.

A provider lists the presets it serves in `CatalogAnswer.presets`
(`with_presets`), only in the answer to a catalog request without a preset;
`catalog::check_presets` validates the list. A caller sends a preset, on
`tool.catalog` or a tool call, only to a provider whose preset-less answer
lists it (`lists_preset`).

A system-text item's preset is optional (`SystemTextItem.preset` is an
`Option<String>`): without one, the provider answers with its default system
text. Since 0.13.0, `SYSTEM_TEXT_DEFAULT` names the manifest fact
`system-text-default/v1`. Declare it in `capabilities.provides` only when
serving that default. A composer omits a system-text preset only when both
the provider declares this fact (read from `catalog.list`) and the runner
declares `system_text_default`; otherwise it keeps naming the preset.
Crate versions are not evidence of either promise.
