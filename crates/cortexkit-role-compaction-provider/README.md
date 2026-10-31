# cortexkit-role-compaction-provider

Setup's optional `view_features` negotiates `truncate` part edits and
`tag_bytes_edits` runner tagging. Without the tokens, replaced bytes stay
final as before. The runner renders truncate references with the shared
`tags::truncate_part` profile rules in `cortexkit-role-llm-runner`.
On Pi, `drop_attachments` affects only the addressed block, never siblings;
to remove a result message's images, send an ordinary `drop` edit for each
image part. The `pi-part-edits.json` vector pairs an image drop with a text
placeholder and preserves the result message's error fields.

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
view on request. The cut comes from the provider: the adapter's optional
`PrunedReasoningProbe::publish_cut` has the provider publish a view with a
cut, which the probe applies as a runner would before it sends the Setup
descending from that cut. The probe never cuts on its own, because a provider
refuses `unknown_cut` for a cut it never published. Without `publish_cut`
the cut conversation is not run, and the report names it not applicable in
`conversations_not_applicable`. A Setup the provider refuses fails the probe
with the refuse code and reason. Configure the provider to render held messages as held, in a
verbatim fold or in keep-range part edits: a view that neither replaces nor
edits a pruned message checks nothing and fails. The probe fails a
provider that renders pruned content from a held copy, from either side of its
cut or by re-serializing a held part inside a part edit (the probe's answers
carry reasoning, a part holding a tool call with its thought signature and
its result, and signed `text_with_state`), that does not re-read its held messages at or below `through_ordinal`
after the generation changes, that reads below its cut, or that re-reads again
without a generation change.

A provider set up with a serializer profile says so through
`PrunedReasoningProbe::serializer_profile`. The probe then names that profile
in each Setup's `params.serializer_profile` and serves its transcript in the
profile's own stored format (`anthropic-messages`, `opencode-aisdk`,
`owned-broca` or `pi`), with `tags` rows as a descriptors read carries them.
On `opencode-aisdk` and `pi` each `mid` it serves is the host's own message
id (`info.id`, the journal entry `id`), and every notice ends on a complete
tool call and result.
It prunes each part as that profile's rules in `llm-runner/v1` §9.14.1 say,
and checks views with `CompactionView::restored_pruned_part_in`. A profile
transcript adds a third conversation that prunes with `opaque_reasoning`,
where only opaque payloads are removed and readable summaries stay.
`held_copies_follow_reasoning_pruned_report` lists which logical parts
(`conformance::probe_parts`) and prune kinds were checked, and names those
the profile cannot carry as not applicable: `anthropic-messages` has no
thought signature, no provider-state signature on text, and so no
`signature` kind, and an `owned-broca` page serves a text-with-state part as
plain text, so its provider-state signature is never checked there. Without a profile the probe serves the generic transcript of
0.7.3, with its `text_with_state` in the real `gemini` provider-state shape.
