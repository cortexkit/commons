# cortexkit-role-llm-runner

Wire types, small validation helpers and checksum-pinned vectors for
`llm-runner/v1`: transcript reads, run results, subscriptions, session sends
and the frozen session baseline. Version **0.8.0** is a breaking Rust and wire
release; the role capability name remains `llm-runner/v1`.

[CONTRACT.md](CONTRACT.md) is the complete normative contract. The separate
`cortexkit-role-llm-runner-conformance` 0.17.0 crate drives real runners over
their management routes; this crate has no transport or runtime.

## Compaction and entry counting

Compaction is defined by `cortexkit-role-compaction`:
`compaction-runner/v1` and `compaction-provider/v2`. This crate no longer
publishes the compaction-provider/v1 integration, checkpoint/anchor APIs,
lineages, negotiated truncate edits, tag rows or `session.resolve`.
`model_view` reads and replacement-entry pages are removed too. A plan still
names its provider with `compaction_item {provider, preset, params}`. The
session-role preset is required; omission is refused at admission as
`invalid_params` naming `plan.compaction_item.preset`. It is passed unchanged
to Setup, never invented or defaulted.

Ordinals follow compaction W4.1/W4.2 entries, from **1** over the whole active
path. Marker records are not entries; a finished OpenCode 1 compaction pair
is a marker, while dead steps and unfinished pairs remain entries.
`serializer_profile` provides:

- `opencode1_session_projection`, ordering hydrated `{info, parts}` rows by
  creation time and id and recognizing completed compaction pairs;
- `opencode_v2_session_projection`, using native `seq` order and excluding
  idle, agent-switched and background-shell controls;
- `pi_session_projection`, walking the active branch and counting every
  message role, custom message and branch summary;
- `native_log_ordinals`, counting all native messages except host markers.

Part numbering, tag notation and cleaning belong only to the compaction
contract. A `delivery_order` template's `part_tag` slot remains: its `shown`
callback supplies the part's compaction A3 number. Templates do not count.

## Reads and cache identity

`session.read` serves the whole active path, including entries below the
latest host marker, from the host's own store. `from_ordinal: 0` and `1` both
start at the first entry. A marker hides entries from the model, never from
reads. `session.head.first_ordinal` and `cut` describe that model boundary.
Every history and display entry has a required ordinal; neither operation
serves markers or control records as entries.

Every head and every read/history page carries required `history_version`
and `incarnation`. Readers bind cached pages to that pair. A rewrite raises
the version; appends and cuts do not. Incarnation is minted with the runner's
session record and is new if that record was discarded and the session
returns. A durable per-id high-water outside discarded/restored data prevents
reallocating an old version to replacement history, as well as pair reuse.
An `after_mid` cursor carries `history_version`;
a mismatch is `history_changed`, never a fallback to tail.

A host-backed runner may keep each served message's digest and a `stripped`
flag instead of its body. `served_bytes::served_body` applies the profile's
fixed strip list; `check_served` detects changed rebuilt bytes. The runner
refuses `served_bytes_changed` with `{mid}` and raises `history_version`
before serving rewritten content. `opencode-aisdk` strips user-message
`info.summary`; the other profiles' lists are empty.

Descriptors publish their own opaque part keys in `part_digests`, one per
stored top-level part in order. They do not depend on tag rows. Readers
never construct part keys from array indexes or native part ids.

## Run settings and results

`SendRequest::route_decision_id` is typed beside `service_tier` and fixed per
run. An explicit different value on a joining send is refused by name on
every runner. Runners using routing accounts forward it to `route.account_for`
as `decision_id` with the run's own provider and model; an unknown or expired
decision ends the run before a model call under `routing_decision_refused`.
There is no fallback allocation, and the caller alone reports that decision.
Other runners ignore only forwarding and reporting.

A runner measures every provider result before rendering using tool-provider
0.15.0's shared 65,536-byte model-visible measure. `tool_result_limit` reuses
those helpers. Oversized output becomes a whole error result with reason
`tool_result_too_large`, measured `bytes` and `limit`, explicitly saying the
tool ran, its output was discarded, and the call must not be repeated just
to see it. Output is never truncated or redispatched. Held first replies stay
pending and refusals remain refusals.

The optional `send_status` group serves `session.send_status` and the
`send.status` alias until the next breaking release. User-command request
bodies omit `session_ref`; their route is already session-bound.

## Archive pruning and immutable output references

`archive_prunes_reasoning` still declares the exact native pruning kinds.
Head/read `reasoning_pruned` and per-page `pruned_reasoning` are retained.
Each served entry publishes its stored-order keys in `part_digests`, or in
`part_keys` when digests are absent. Every listed pruned part names a served
key. Parts keep their type, position, identity and write-time digest.

A runner whose plan names a compaction provider does not prune reasoning
until the compaction contract defines provider acceptance. Sessions without
a provider may prune under CONTRACT §9.14.1. The old Setup acceptance and
post-prune view machinery are removed.

`output_refs` no longer requires the removed `tags` group. It captures the
writing request's part-number mapping, addresses stored parts by message/part
identity and retains immutable snapshots. It never re-resolves an old record
against a later request.

## Vectors and validation

Enable `test-vectors` to compile the shipped vectors into the public `vectors`
module. `vectors::FILES` is the ordered inventory, and `SHA256SUMS` hashes each
file's exact bytes. Public extensible wire types have constructors and are
non-exhaustive. Decode requests strictly where omission would change the
question; decode reply extensions leniently, as CONTRACT §13 specifies.
