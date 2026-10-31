# cortexkit-role-llm-runner

Wire types and vectors for `llm-runner/v1`: transcript reads, run results, subscriptions, owner sends, and the session baseline.

`tags::truncate_part` renders negotiated compaction part edits for all five
serializer profiles. Its checksum-pinned `truncate.json` vectors pin the
rules in compaction-provider's contract §6.1, including the top-level-only,
UTF-16 edit-marker input rule. The contract and the vectors together are
the specification.
Pass `PartContainer` from the stored row, never from a provider's view.
On Pi, `drop_attachments` affects only the addressed block, never its
siblings. To remove a result message's images, the provider sends an ordinary
`drop` edit for each image part.

Add to `Cargo.toml`:

```toml
[dependencies]
cortexkit-role-llm-runner = "0.7.4"
```

The [contract](CONTRACT.md) is normative. See the [conformance runner](../cortexkit-role-llm-runner-conformance/README.md) and [role/conformance guide](../../docs/role-contracts.md) for implementation testing.

Since 0.7.29, the opt-in `retract_own_send` group (requiring `retract`)
allows a daemon-attested sender to withdraw its own pending submission on
the same session. On a session with a recorded owner, other non-owners and
submissions without a recorded sender stay owner-only; request fields never
establish identity. Answers and withdrawal semantics, including durability,
are unchanged. Without the group the owner-only rule on owner-protected
sessions is unchanged. On a session with no recorded attested owner, a
runner may admit any reachable caller; that existing rule is unchanged. See contract §§6.2
and 9.2.

Since 0.7.14, a runner declaring the `rich_ui_stamp` group serves the rich UI
stamp `{dialect, catalog_digest}` (`rich_ui::RichUiStamp`) that a run's frozen
plan carries, copied from the system-text answers it fetched: on every
assistant display row the run writes (`DisplayRow::rich_ui`), on its
`run_started`, as `session.head`'s `last_run_rich_ui` while that run is the
last, and on its assistant `session.read` entries when the read sets
`include_rich_ui`. Absent from such a runner means unstamped: render plain
Markdown. Readers accept the members before any runner declares the group.
Two different stamps in one plan are refused `plan_stale`. See contract §9.17.

Since 0.7.8, an `owned-broca` tool error may say that its provider module
is restarting or down, with optional `module`, `code`, `detail`,
`nothing_sent` and `retry` members after `message`. The set is closed and
readers refuse any violation, naming the member and the rule
(`serializer_profile::error_output::ErrorOutput::decode`). Every runner,
whatever its profile, shows the model the same block
(`serializer_profile::error_output::render`): the message, then one line
per present member, `nothing was sent` standing for `nothing_sent: true`.
See contract §4.4 and §11.4 and `test-vectors/owned-broca-tool-outage/`.

Since 0.7.4, `archive_prunes_reasoning` declares the exact pruning kinds a
runner may apply at rest. Every affected raw/model read and raw/display history
page repeats `pruned_reasoning`, carrying the original part's JCS byte length
and write-time digest. Parts keep their types, positions, required fields,
keys, tags and digests; only the declared payload changes. An enrolled
compaction provider must opt in through Setup's `accepts: ["pruned_reasoning"]`.
See §9.14.1 and the original/served vectors. Readers may replace held copies
only for listed parts with matching original digests; an unknown kind means
the changed payload is not trusted. Imported assistant display rows may omit
an unrecorded model; readers must treat absence as unknown before rollout.

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

## Provider service tiers

The opt-in `service_tier` group publishes a per-wire `service_tiers` map and
accepts `standard`, `fast` and `ultrafast` on sends. Absence means standard for
a send that starts a run; a send into a running run inherits its tier. Send
encoders omit only absence and preserve explicit standard. Unsupported levels
are refused before model dispatch,
and an explicit different tier cannot steer or queue into a running run.
`service_tier` holds the shared types and validators. `ServiceTierUsage` owns
per-step tier metadata without changing the token-only, Copy `StepUsage`;
`RunUsage::record_step_with_service_tier` updates `RunServiceTierUsage` state,
which keeps a constant number of tier strings independent of the number of
steps and examines only the new step, to aggregate a fixed applied level
and unanimous reported served values. Persist that state alongside token totals
so a restart cannot forget disagreement, without re-reading old steps each time.
Never copy applied into served. Unknown-served wires omit served.

Tier changes are cache-busting and ride before the changed request, like a
model change. A compaction provider must adopt 0.7.12 before a runner sends a
non-standard tier, because notice digests cover the new condition. The closed
`step.finished.v1` event is unchanged; consumers read tier values in usage-steps
rows. See CONTRACT §9.3.2 for the full rules.

Requests keep the closed `ServiceTier` enum; read the raw member with
`service_tier::parse_request` so an unknown level receives the named
`unsupported_tier` refusal before decoding the rest. Tier values on records are
strings and keep unknown levels on read. Producer checks restrict applied
records to fast/ultrafast (standard is omitted), while served provider values
remain open. Pricing at an unfamiliar level is unconfirmed. `WIRE_APIS` exports
the wire API list; a routing drift test keeps it equal to routing's constants.
