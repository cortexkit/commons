# cortexkit-role-llm-runner

Wire types and vectors for `llm-runner/v1`: transcript reads, run results, subscriptions, owner sends, and the session baseline.

Add to `Cargo.toml`:

```toml
[dependencies]
cortexkit-role-llm-runner = "0.6.0"
```

The [contract](CONTRACT.md) is normative. See the [conformance runner](../cortexkit-role-llm-runner-conformance/README.md) and [role/conformance guide](../../docs/role-contracts.md) for implementation testing.

The optional `held_calls` describe group reports held tool-provider calls on
run status and status events. A non-empty list is a runner-owned wait with no
deadline, independently of `deadline_ms`; watchdogs do not nudge these runs.

Since 0.6.0, `started_at` on `tool_started` and `finished_at` on
`tool_finished` are optional. A runner leaves one out only when it recorded
no time, such as a call replayed from history stored before it kept times,
and never invents one; a reader shows the call without that time.
