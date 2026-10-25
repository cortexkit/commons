# cortexkit-role-llm-runner

Wire types and vectors for `llm-runner/v1`: transcript reads, run results, subscriptions, owner sends, and the session baseline.

Add to `Cargo.toml`:

```toml
[dependencies]
cortexkit-role-llm-runner = "0.7.2"
```

The [contract](CONTRACT.md) is normative. See the [conformance runner](../cortexkit-role-llm-runner-conformance/README.md) and [role/conformance guide](../../docs/role-contracts.md) for implementation testing.

Since 0.7.2, a runner declaring the `call_identity` group puts `call_key` and
`schema_pin` on display tool calls (`display::ToolCallPart`) and on the live
`tool_started` and `tool_finished` events. Both are opaque. `call_key` is the
call's key exactly as `session.read` attributes it, so it is absent only while
that attribution has none. `schema_pin` is the pin of the tool definition the
call was dispatched against, recorded at dispatch; it is absent for a call
never dispatched and for a runner-native tool. See contract §9.16.

Since 0.7.1, `session_retention` may declare a default-zero `min_hold_days` and
`deletion_waits_for_readers: true`. The floor is a minimum, never added to a
policy's lifetime. Head exposes `released` until deletion, reads remain served,
and refused sends carry `detail.delete_after_ms`. The separate `watermark`
module defines a log reader's caller-scoped `log-read-watermark/v1` capability,
strict empty `watermark.get` request and timestamp-or-null reply. See contract
§9.14–9.15 for the durability and same-computer clock requirements.

## Hosted session references

Use the shared function with named inputs; legacy positional calls no longer
compile, preventing a folder from silently becoming a hash input:

```rust
use cortexkit_role_llm_runner::{session_ref, SessionIdentity};

let reference = session_ref(&SessionIdentity {
    runner: "runner-a",
    harness: "harness-a",
    session: "session-1",
});
assert_eq!(reference, "f93ec93e3559a70a8db019b29e0fa883");
```

`runner` is the daemon-attested principal's **bare module id**, exactly as
recorded in `session_binding.runner`, never `reserved:runner-a` or another
prefixed/display spelling. Extract the bare id before constructing the
identity; the helper hashes all fields exactly as supplied and strips nothing.
Use deprecated `session_ref_v1(harness, project_root, session)` only to migrate
stored refs: re-key live sessions once; ended rows keep v1 refs.

Since 0.6.5, `system_text_default` promises preset-less system-text admission
and unchanged forwarding. `RoleDescribe::system_text_default()` reads this
group. A composer omits a preset only with both this runner fact and the
provider's manifest `system-text-default/v1` fact from `catalog.list`.
Admission checks that provider fact before writing or forwarding; its absence
is terminal `system_text_preset_required {provider}`, never a timed refusal.
The provider name is exposed as `SYSTEM_TEXT_DEFAULT_PROVIDER_CAPABILITY`
without a tool-provider runtime dependency. `system-text-default.json` adds
describe and refusal vectors; existing vector data is unchanged.

Since 0.6.1, `session.head.model` optionally reports the runner's recorded
next-send selection, not the frozen plan. Absence means unknown; no group is
needed. The new `session_usage` group enables `session.head.usage`: the same
fields as run usage plus a required `since` run ID. Session spend is durable,
incrementally updated with each step and never reduced by a rewind or cut.
The `since` boundary makes uncounted imported history explicit. Unknown token
fields remain absent, not zero. See contract §4.3 for scope and cost rules.
`session.head.sender` is the attested caller's own delivery grant under the
effective next-send plan, including accepted refreshes. Since 0.6.2, the
`head_sender` describe group makes absence mean no grant for that caller;
without the group, absence means unknown, even if some callers receive
`sender`. Declaring runners report `sender` to every plan-named caller and
the owner, who gets `owner: true`, every accepted mode and `interrupt: true`.
Readers can use `HeadMeta::sender_grant` to distinguish `Granted`, `NoGrant`
and `Unknown`. Model, usage and incarnation absence still means unknown.
`compaction_item.preset` is optional; a preset-less plan item stays preset-less
on provider calls, and named presets must come from the provider's catalog.
`session.head.incarnation`, when known, identifies the key's log across cuts,
reverts, restarts, restores and moves. Claims key on `(incarnation, mid)`;
without it they fall back to lineage-scoped claims. A runner reports it only
when it can vouch for that full lifetime, never with a per-process value.

The optional `held_calls` describe group reports held tool-provider calls on
run status and status events. A non-empty list is a runner-owned wait with no
deadline, independently of `deadline_ms`; watchdogs do not nudge these runs.

Since 0.6.0, `started_at` on `tool_started` and `finished_at` on
`tool_finished` are optional. A runner leaves one out only when it recorded
no time, such as a call replayed from history stored before it kept times,
and never invents one; a reader shows the call without that time.
