# cortexkit-role-llm-runner

Wire types and vectors for `llm-runner/v1`: transcript reads, run results, subscriptions, owner sends, and the session baseline.

Add to `Cargo.toml`:

```toml
[dependencies]
cortexkit-role-llm-runner = "0.5.4"
```

The [contract](CONTRACT.md) is normative. See the [conformance runner](../cortexkit-role-llm-runner-conformance/README.md) and [role/conformance guide](../../docs/role-contracts.md) for implementation testing.

The optional `held_calls` describe group reports held tool-provider calls on
run status and status events. A non-empty list is a runner-owned wait with no
deadline, independently of `deadline_ms`; watchdogs do not nudge these runs.
