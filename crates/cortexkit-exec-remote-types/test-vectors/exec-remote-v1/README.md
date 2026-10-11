# exec-remote/v1 caller vectors

The `replies/` and `outcomes/` `.jcs`/`.sha256` pairs are the executor's own
golden cases for what a caller receives, copied byte for byte so both sides
pin the same bytes. The twenty-six `crate-local-*` cases documented below were
written in this crate rather than copied from the executor's vectors, to pin
caller-side cases the executor's set does not cover. The following encoding and shape rules
cover the caller corpus; executor-to-runner frames are not included.

## Encoding

The discriminator is `type`; fields live inline beside it. Unknown object fields
are ignored. `bytes` uses standard padded base64, including non-UTF-8 payloads.
`job_id` and `transfer_id` are UUIDv7 strings. The types validate UUID syntax;
mint-time/retention and UUID-version policy belong to the executor.

Each case has an RFC 8785 `.jcs` (no newline), and a lowercase 64-character
`.sha256` over its actual `.jcs` bytes (no newline). This corpus contains ASCII
strings and small integers only. The Rust golden test uses an independent RFC
8785 implementation, checks SHA-256, decodes every request/reply, and requires
re-encoding to preserve every declared field. The nested-unknown-fields input
re-encodes to the separately pinned `all` report, dropping only unknown fields.
Git must not convert line endings
in the canonical bytes or digests.

## Shapes

- `outcomes/`: `{request, stream}` with the caller's env and the complete reply
  stream. The 20 copied cases cover the original known outcomes/reasons, both kills,
  pipeline status, lost running and queued jobs, and pre-snapshot rejection. No
  `accepted` precedes the `runner_full` terminal. Exactly one terminal ends each
  stream.
- `replies/`: 14 caller-facing prepare, drop, cancel, and status values without a
  wire frame discriminator. Includes all seven prepare refusal reasons, absent
  workspaces, cold generations and an unreachable server.

The caller's `RunRequest` has optional queue wait and sibling canonical paths;
it cannot supply IDs, snapshots, hashes or bundles. The terminal's `outcome` is
an object discriminated by `type`; exit/signal carry `code`/`signal`, refusals
carry `reason`. `ran`, `tree_hash`, and `workspace_changes` are always emitted,
explicitly null where unavailable. `workspace_changes` is an array only for
`ran: remote`. `killed` and `pipestatus` are omitted when absent.
`outcome_unknown` can describe either a previously running or a queued job;
neither is a before-start proof.

`cancel_reply {job_id}` acknowledges durable idempotent cancellation, including
an unknown-ID tombstone; the terminal is replayed on run/attach, not this reply.
`drop` uses `workspace_key`; its reply has `dropped` and `cancelled_jobs`.
Status has `queue_depth`, `running_jobs: [{job_id, workspace_key, weight}]`
(including `base:` pseudo-keys), `server_reachable`, `repositories`, and
`rustc_version`. Repository entries have `repository_root`, nullable
`warm_target_age_s`, nullable `published_commit`, and nullable
`last_rebuild_result: building|ok|failed` (null before any rebuild).

## Crate-local additions

These twenty-three outcome cases (46 files: one `.jcs`/`.sha256` pair each) and
three reply cases (six files) extend the caller corpus described above. The
resulting corpus contains 43 outcome cases and 17 reply cases:

- `outcomes/crate-local-network-outbound`: a request with `network: "outbound"`
  and an `accepted` record acknowledging `network: "outbound"`. The runner must
  isolate internet access from the caller's machine and local network. Callers
  cannot assume outbound access without `Some(Network::Outbound)` in the
  acknowledgement, and must not automatically rerun after an unknown outcome
  because a network-enabled job may have had outside effects.
- `outcomes/crate-local-unknown-network`: `network: "future_network"` retained as
  `Network::Unknown`, with `network_unsupported` refusal before start. A runner
  must refuse unrecognised network requests rather than downgrade or upgrade them.
- `outcomes/crate-local-network-unsupported`: an outbound request refused before
  start with the known, non-transient `RefusalReason::NetworkUnsupported`.
- `outcomes/crate-local-platform-linux` and `outcomes/crate-local-platform-windows`:
  explicit platform requests and acknowledgements. Runners report the platform
  they actually use; callers requesting a non-Linux platform must require an exact
  acknowledgement or cancel without trusting the result.
- `outcomes/crate-local-platform-unsupported`: a Windows request refused before
  start with `platform_unsupported` and the requested wire name in
  `refusal_detail`.
- `outcomes/crate-local-unknown-platform`: an unrecognised platform retained on
  the request and refused before start with its wire name in `refusal_detail`.

Absent network fields preserve the existing `outcomes/exit` bytes. Tests also
decode the outbound request and accepted record with shapes mirroring 0.2.4 and
compare their re-encoding to those records in the existing `exit` vector. Older
decoders ignore the additive fields; an absent request remains offline, while an
absent acknowledgement means the runner predates the field, not access granted.

- `outcomes/crate-local-unknown-refusal`: `refused_before_start` with reason
  `future_refusal`, decoded as `RefusalReason::Unknown` while retaining the
  guarantee that the command did not start.
- `outcomes/crate-local-unknown-outcome`: type `future_outcome`, decoded as
  `Outcome::Unknown`. Callers grade it exactly like `outcome_unknown`: never
  assume the command did not run, and never re-run it locally.

- `outcomes/crate-local-unknown-stream-record`: type `future_record` with seq 7,
  decoded as `StreamRecord::Unknown { kind, seq }`, followed by a known terminal.
  Skip it and keep reading; count its seq toward the resume cursor
  (`AttachRequest::from_seq`); never treat it as terminal. If the stream ends
  without a known terminal record, get the job's outcome from `exec.status` or
  `exec.attach` rather than waiting forever, and grade it outcome-unknown until
  one arrives.
- `outcomes/crate-local-started`: an `accepted`, `started` (seq 7), and terminal
  sequence. `Started` is emitted when the queued job takes capacity; its queue
  wait is runner-measured and its start time is display-only. The runner's
  command timeout runs from this moment, not acceptance. Callers may report the
  job is running but must not start their own timeout from it. Older runners may
  omit the record; it is progress, never terminal. A 0.2.3-shaped decoder sees
  it as an unknown record and retains seq 7 for the resume cursor.
- `outcomes/crate-local-unknown-killed`: reason `future_kill`, decoded as
  `Killed::Unknown`. The command was killed for an unrecognised reason.
- `outcomes/crate-local-unknown-ran`: location `future_location`, decoded as
  `Ran::Unknown`. Never treat it as `Ran::None`: the command may have run. Grade
  it like `outcome_unknown` for re-run decisions; never re-run it locally.
- `outcomes/crate-local-unknown-output-stream`: label `future_stream`, decoded as
  `OutputStream::Unknown`. Deliver the chunk's bytes and seq despite the unknown
  stream label.
- `replies/crate-local-unknown-prepare-outcome`: type `future_prepare`, decoded as
  `PrepareOutcome::Unknown`. Never treat the workspace as prepared or run against
  it on that basis.
- `replies/crate-local-unknown-rebuild-result`: status `future_result`, decoded as
  `RebuildResult::Unknown`. Informational only.

Six outcome cases in `outcomes/` (named `crate-local-server-reports-*`) pin optional terminal reports
of server state that is not copied back:

- `all`: changed commit, ref, index tree and stash count; two new untracked
  paths; a total of 25 ignored writes with a two-path sample.
- `older-runner`: all three reports absent, each decoding as `None` (not
  reported, never proof that nothing changed).
- `unknown-fields`: the same known data as `all`, with `future_extension`
  inside each new report struct. Unknown fields are tolerated and dropped on
  serialization; the expected canonical output is the independently pinned
  `all` vector, not a re-encoding used as its own oracle.
- `detached-head`: explicit null before/after refs and an unavailable index
  tree before execution, while commit IDs still differ.
- `truncated-untracked`: two sampled paths with `truncated: true`, representing
  a producer choosing a smaller cap than the recommended 100 entries.
- `unchanged`: equal Git before/after values, an empty untracked list with
  `truncated: false`, and zero ignored writes with an empty sample. A reporting
  runner emits these reports even when nothing changed.

`GitStateChange` carries nullable `head_before`/`head_after`,
`ref_before`/`ref_after`, and `index_tree_before`/`index_tree_after`, plus u32
`stash_count_before`/`stash_count_after`. `UntrackedFiles` carries `paths` and
`truncated`. `IgnoredWrites` carries the u64 total `count` and `sample_paths`.
Producers should cap untracked paths at 100 and ignored-write samples at 20;
ignored writes exclude `target/`, `node_modules/`, and `dist/`. These types only
carry the data; the runner enforces caps and exclusions.

Four vector pairs (each a `.jcs` canonical record and its `.sha256`) fix the exact bytes of the optional refusal fields and of the two newly recognised refusal reasons:

- `outcomes/crate-local-refusal-hints`: the existing `runner_full` terminal record (a refused-before-start outcome) with
  a verbatim `refusal_detail` and `retry_after_ms: 250`.
- `replies/crate-local-prepare-refusal-hints`: the existing `prepare-unreachable`
  reply with a verbatim `refusal_detail` and `retry_after_ms: 500`.
- `outcomes/crate-local-runner-draining` and `outcomes/crate-local-runner-disk-full`:
  `runner_draining` and `runner_disk_full`, each decoded to its known variant and
  round-tripped through the public string conversions.

Removing both metadata fields from the hint vectors produces the unchanged
`outcomes/runner_full` and `replies/prepare-unreachable` bytes. Tests also decode
the hint vectors with structs mirroring the 0.2.2 container shapes and compare
their output to those older vectors. Optional fields are ignored by old decoders.
Producers must keep detail within `REFUSAL_DETAIL_MAX_BYTES` (1024 UTF-8 bytes)
without secrets or credentials; decoders retain longer values. Callers may quote
detail but must never parse it. Only for `refused_before_start`, callers may use
the retry hint, waiting at most the smaller of the hint and their remaining budget.

Every enum a caller receives decodes unknown tags into a catch-all with a stated
grading. Each catch-all round-trips its raw tag; unknown stream records also
retain and round-trip `seq` when present. A present non-`u64` seq (including null)
is a decode error. Other unknown fields are ignored. Known tags with malformed or
missing required fields still fail, and string enums still reject non-string
wire shapes.
