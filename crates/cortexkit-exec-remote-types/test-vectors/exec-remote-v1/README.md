# exec.remote/v1 caller vectors

The `replies/` and `outcomes/` `.jcs`/`.sha256` pairs are copied byte for byte
from prefrontal's `motor-protocol` at commit `64fbc5dcf`, except for the two
`crate-local-*` cases documented below. The following encoding and shape rules
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
re-encoding to preserve every declared field. Git must not convert line endings
in the canonical bytes or digests.

## Shapes

- `outcomes/`: `{request, stream}` with the caller's env and the complete reply
  stream. The 20 copied cases cover every known outcome/reason, both kills,
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

These two outcome pairs extend the original corpus, for a total of 22 outcomes:

- `crate-local-unknown-refusal`: `refused_before_start` with reason
  `future_refusal`, decoded as `RefusalReason::Unknown` while retaining the
  guarantee that the command did not start.
- `crate-local-unknown-outcome`: type `future_outcome`, decoded as
  `Outcome::Unknown`. Callers grade it exactly like `outcome_unknown`: never
  assume the command did not run, and never re-run it locally.

Unknown tags in these two enums round-trip their raw tag; known tags with
malformed or missing fields still fail. The original motor-protocol codec
rejects unknown enum tags. Other enums retain that strict behaviour here.
