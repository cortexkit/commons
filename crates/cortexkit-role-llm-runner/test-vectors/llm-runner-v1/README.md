# llm-runner/v1 vectors for runner 0.8.3

The role capability remains `llm-runner/v1`; 0.8.0 is a breaking contract
release. `src/vectors.rs::FILES` is the exact ordered inventory and
`SHA256SUMS` hashes the raw bytes in that order. The unit tests round-trip the
wire shapes and check the literal expectations. Enable the `test-vectors`
feature to use the same inventory from another crate.

## Read and identity vectors

`read-requests.json`, `read-pages.json`, `head.json`, `history.json`, and the
head/display extension vectors use one-based entry ordinals and required
`history_version`/`incarnation`. Zero is a range-start alias for the first
entry, not a message ordinal. Stale after-mid cursors are refused
`history_changed`. Native marker boundaries are model boundaries, never read
bounds. The vector identities are opaque; neither a mid nor a part key may
be derived from an ordinal by a reader.

`read-renumbering-switch.json` illustrates one session before and after it
adopts 0.8 numbering. The same two messages move from ordinals 0,1 to 1,2.
The before page carries `lineage_id` and neither `history_version` nor
`incarnation`. The after page omits `lineage_id`, carries `incarnation`, and
reports `history_version: 5`, above the old head's version 4, which the
wrapper records as `history_version_before_switch`. It illustrates the
shapes; it does not run an upgrade.

`read-descriptors.json` pins in-place payload references and write-time
message/part digests. Digest rows themselves publish stored-order opaque
part keys; no tag-row pairing is required. Unknown request fields remain
strict. Model-view pages, replacement-entry sources, tag rows, resolve and
v1 checkpoint/negotiated-edit vectors were removed.

## Native profile provenance

- `pi-1.3.0-1.4.0.jsonl`, `pi-session.json` and `display-pi.json` cover active
  branches, message roles, custom messages, branch summaries, edits and native
  compaction markers. See `README-pi.md` for capture provenance.
- The OpenCode 2 rows-only recordings are unchanged host records. Their
  expected projections in `opencode-v2-session.json` follow compaction W4.2:
  `seq` order; one-based entries; no idle, agent-switched or background-shell
  entries; compaction rows are markers. See `README-opencode-v2.md`.
- OpenCode 1's W4.2 row rules and native-log counting have explicit fixtures
  in the serializer-profile unit tests. Dead steps count; completed compaction
  pairs do not. Unfinished pairs and error/abort part projection are tested.

Source-capture files remain byte-for-byte recordings. A helper's expected
entry projection is separate from a capture and changes when the contract's
entry selection changes. Descriptor part counts describe native stored
containers, not a second ordinal counting scheme.

## Pruning and result limits

`archive-prunes-reasoning.json`, `reasoning-pruned.json` and
`reasoning-pruned-profiles.json` pin native original/served payloads,
write-time digests and retained head/read prune state. Provider-plan pruning
is gated off until the compaction contract defines acceptance. The old
Setup/view restoration vectors are not part of 0.8.0. Archive-pruning entries
carry their keys in `part_digests`, or `part_keys` when digests are absent.

Provider result sizes use tool-provider 0.15.0's published measure. Boundary
and held-result tests are in `tool_result_limit`; oversized output becomes
`tool_result_too_large`, with measured bytes and the 65,536-byte limit, not a
truncated success or another dispatch.

## Other retained surfaces

The other files cover admission/baselines, plans, sends and receipts, run
results/status/cancel/usage, subscriptions, delivery templates and senders,
attachments, immutable output references, user commands, retention,
structured output, tool identity, display/live events and rich-UI stamps.
`plan-compaction-preset.json` pins the required explicit session-role preset
and its named admission refusal. Encoded user-command bodies have no session_ref.
The send-status describe vectors require both canonical session.send_status
and its send.status alias for the optional group.

Template `part_tag` slots and output references use the writing request's
compaction A3 numbers, supplied by the runner. These vectors do not define a
second numbering function or a compaction-runner conformance suite.
