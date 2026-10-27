# cortexkit-role-compaction-provider

Wire types and vectors for `compaction-provider/v1`: Setup, advance notices, views and `view.get`, `compaction.ride`, and the ceiling policy.

Add to `Cargo.toml`:

```toml
[dependencies]
cortexkit-role-compaction-provider = "0.7.0"
```

The [contract](CONTRACT.md) is normative. See the [role/conformance guide](../../docs/role-contracts.md) for the available suites and vector locations.

## Focused state-identity probe

The non-default `conformance` feature exposes `conformance::StateProbe` and
`conformance::conversation_state_ignores_project_root`. This probes one rule,
not a full provider conformance kit. An implementation adapter delivers Setup
through real daemon-attested runner routes and reads an opaque marker from the
provider's actual per-session state, without reconstructing it from inputs.
Use a fresh test store with `/var/probe-project` and
`/private/var/probe-project` as two spellings of one folder. The same
`(bare runner module id, harness, session)` must reach the same state through either
spelling; a different runner with the same harness and session must not.
The adapter supplies `runner` as the daemon-attested principal's bare module
id (for example `runner-a`), never `reserved:runner-a`, another prefix or
Setup's body. Calls to the shared reference helper take
`&SessionIdentity { runner, harness, session }`, so old positional calls cannot
silently use the wrong fields.

## Focused held-copies probe

The same feature exposes `conformance::PrunedReasoningProbe`,
`conformance::ScriptedRunner` and
`conformance::held_copies_follow_reasoning_pruned`, for a provider that lists
`pruned_reasoning` in its Setup answer's `accepts`. The probe plays a runner
(`PROBE_RUNNER`) that prunes reasoning the provider already read, in one
conversation without a cut and one where a cut leaves the provider holding
copies from before it. The adapter delivers the probe's Setup requests and
notices, answers the provider's reverse `session.read` and `session.head`
calls with the `ScriptedRunner` it is given, and returns the provider's next
view on request. Configure the provider to render held messages as held, in a
verbatim fold or in keep-range part edits: a view that neither replaces nor
edits a pruned message checks nothing and fails. The probe fails a
provider that renders pruned content from a held copy, from either side of its
cut or by re-serializing a held part inside a part edit (the probe's answers
carry reasoning, a part holding a tool call with its thought signature and
its result, and signed `text_with_state`), that does not re-read its held messages at or below `through_ordinal`
after the generation changes, that reads below its cut, or that re-reads again
without a generation change.
