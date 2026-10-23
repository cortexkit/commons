# cortexkit-role-llm-runner

Wire types and vectors for `llm-runner/v1`: transcript reads, run results, subscriptions, owner sends, and the session baseline.

Add to `Cargo.toml`:

```toml
[dependencies]
cortexkit-role-llm-runner = "0.6.2"
```

The [contract](CONTRACT.md) is normative. See the [conformance runner](../cortexkit-role-llm-runner-conformance/README.md) and [role/conformance guide](../../docs/role-contracts.md) for implementation testing.

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
