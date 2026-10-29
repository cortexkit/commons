# exec-remote/v1 caller contract

This crate defines caller-facing values for `exec-remote/v1`. It carries protocol
shapes; runners remain responsible for enforcing request limits, collecting files,
and emitting records in the required order.

## Idempotent submission

`RunRequest.submission_key` is an optional, opaque, caller-chosen key of 16 to
128 bytes, each in printable ASCII 0x21 through 0x7e (no whitespace).
`RunRequest.resume_from_seq` is an optional `u64` and is malformed without a
`submission_key`. Decoding rejects an invalid key or a cursor without a key;
the executor reports malformed request syntax as `invalid_params`, identifying
the offending field. Both fields default to absent and are omitted when absent.

**Capability gate:** a caller sends either field only to an executor advertising
`SUBMISSION_KEY_CAPABILITY` (`exec-remote-submission-key/v1`). The types define no
capability handshake. Older executors ignore unknown fields and could start a
second job on resubmit, so sending keys without this check is unsafe.

### Scope and durability

The executor scopes keys to the attested caller principal (the module identity,
not the route). Two callers never share a key namespace. The executor must not
let a caller attach to another principal's job or read its stream.

The executor records the key durably **before** submitting the job to any runner,
and keeps the key record at least as long as the job record (24 hours today).
While that record exists, a resubmit must never start a second job under the key.
An executor that has forgotten a key cannot distinguish a resubmit from a fresh
request. A caller must therefore not resubmit an unresolved key after that
retention window: forgetting the key is not evidence that nothing ran.

### Same request only

A resubmit carries the same caller-supplied request as originally sent, except
for `resume_from_seq` and `progress`. The executor compares a digest of the request
with `resume_from_seq` and `progress` cleared, over a canonical encoding using
RFC 8785 JSON canonicalization (sorted keys and no insignificant whitespace). JSON field order
cannot cause a mismatch. The crate's vector tests use
`serde_json_canonicalizer` for RFC 8785 encoding; this crate does not compute
submission digests or add an execution-policy implementation.

`progress` is excluded because it changes only display, not what runs. A caller
may opt in or out on resubmission without changing the request digest.

The digest covers every caller-supplied field that affects what runs. The
current set is `command`, `cwd`, `workspace_key`, `repository_root`, `siblings`,
`platform`, `weight_hint`, `network`, `timeout`, `queue_wait_limit_s`, `env`, and
the return options `return_paths`, `return_max_bytes`, and `return_chunked`.
The key identifies the record being compared and is unchanged on resubmit.
An executor may leave out environment entries it never forwards to the command.

The digest never covers anything the executor derives, such as a workspace
snapshot or bundle, job IDs, or placement. After a caller restart, a resubmit
may recapture a checkout whose content has moved on. It must still attach to
the existing job: that existing job, not the recaptured checkout, is what runs.

A mismatch is refused with `RefusalReason::RequestRejected` (`request_rejected`)
and `refusal_detail` `submission_key_mismatch`. The executor must never attach a
different command to an existing job. Such a refusal does not change the
existing job or imply that its earlier command did not run.

A refusal of a keyed request carries a `job_id` of its own, never the existing
job's ID. A caller must not apply the refusal to any other `job_id`. In
particular, a mismatch refusal leaves the existing job's state unaffected; that
state is learned only from the existing job's own stream or an attach to it.

For example, these two requests are the same request for digest comparison:
only JSON field order, the replay cursor and the display-only progress opt-in differ.

```json
{"workspace_key":"example-task","repository_root":"/example/repo","cwd":"/example/repo","command":"printf hello","env":{},"siblings":[],"submission_key":"example:task-0001"}
```

```json
{"resume_from_seq":7,"progress":true,"submission_key":"example:task-0001","siblings":[],"env":{},"command":"printf hello","cwd":"/example/repo","repository_root":"/example/repo","workspace_key":"example-task"}
```

### Replies and replay

For a keyed request, exactly one admission outcome applies:

- Started: `Accepted.attached` is absent or false.
- Attached: `Accepted.attached` is true and its existing `job_id` names the job
  to which the request attaches, rather than a new job.
- `RefusalReason::SubmissionNeverAccepted` (`submission_never_accepted`): a final
  refusal, sent only to a caller that supplied a key. No runner ever accepted
  this key's earlier submission, this resubmit was refused too, and nothing ran
  under the key. The caller may resubmit under a **new** key. This is not an
  outcome-unknown reply. Older decoders preserve the tag as `Unknown`, whose
  existing meaning is still "did not start".
- Any existing refusal, retaining its existing meaning for this request.

Every refusal of a keyed request, including `submission_key_mismatch`,
`submission_never_accepted`, and any other refusal of a resubmit, carries a
`job_id` of its own. It must never name the existing job, and a caller must not
apply the refusal to any other `job_id`. A mismatch refusal does not affect the
existing job's state; learn that state only from the existing job's own stream
or an attach to it.

Here "started" distinguishes a new admission from attachment; `Accepted` still
does not prove the command has begun execution (`Started` reports that).
`attached` defaults to false and is omitted when false. A runner predating the
field never sends it, so absence means "started, or a runner that doesn't support
keys". A caller relies on it only after the capability check above.

When the key's job exists, replay begins with the first record whose `seq` is
**strictly greater** than `resume_from_seq`. An absent cursor replays from the
beginning. This exclusive cursor does not change the existing inclusive
`AttachRequest.from_seq` contract. If records after the cursor have expired, the
existing expired-replay and outcome rules apply unchanged, including
`ReturnAssembler::finish_expired_replay` for expired returned bytes (see Replay
below). There is no new gap mechanism. Expired job history is not a refusal and
must never cause a caller to start the command again.

## Progress before acceptance

These types define the progress record's shape and check its `detail` syntax.
Everything else in this section is a rule for the runner or the caller that the
types cannot check: when progress may be sent, its order and rate, and what
`detail` may say.

`RunRequest.progress` defaults to false and is serialized only when true, so
requests without progress keep their existing bytes. A caller sets it only for
a runner advertising `PROGRESS_CAPABILITY` (`exec-remote-progress/v1`). Older
runners ignore unknown request fields. The caller never expects progress without
the opt-in and capability advertisement; a runner must never send it otherwise.

`StreamRecord::Progress` is a known, nonterminal `progress` record. It comes only
before `Accepted`, never after it. `Started` still reports when an accepted job
takes runner capacity, covering the wait after acceptance; progress does not
replace it. The progress record has no `seq` field: a runner never sends one, and
a caller never advances the resume cursor from progress. An executor never
replays progress on `exec.attach`. For each run, the runner sends progress at
most once per 10 s, and only after 2 s of waiting.

```json
{"type":"progress","phase":"placement_wait","phase_elapsed_ms":61200,"detail":"waiting for a host with 16 free vCPUs"}
```

`phase` is an open `ProgressPhase`: `upload`, `placement_wait`, or `connect`.
Unknown phases retain their raw string and are shown generically, as display-only
progress. `phase_elapsed_ms` is a `u64`, which the runner sets to the milliseconds since the
**current phase started**, not since it received `exec.run`.

`detail` is optional plain text. When present it is one line of at most 120 UTF-8
bytes, with no control characters or line breaks (including Unicode line and
paragraph separators). Both decoding and `Progress::new` reject violations.
Detail must never carry host names, network addresses, tenant or account
identifiers, or job contents. The runner is responsible for those privacy rules.

Progress is display only: a caller renders `detail` as plain text, never starts,
extends or resets a timer from progress, and never retries or cancels because of
it. A progress record is never terminal, even for an unknown phase. Timing,
ordering, capability and privacy requirements are runner/caller obligations;
these types enforce only the record's shape and detail syntax.

## Hosted placement and billing

A hosted runner may add `TerminalRecord.hosted` with its placement identifier, such
as `ovh-1` or `boat:<sandbox_id>`. `billed_vcpu_ms: null` or an absent field means
billing is pending. Build-server jobs do not report `hosted`; absence is not an
indication of failed execution.

## Returning requested files

`RunRequest.return_paths` lists paths relative to the request's `cwd`. It is omitted
when empty. An empty list disables returned-file collection, and the runner must
not emit a `return_result` record for that request. A runner sends `return_result`
only when the request contains a non-empty `return_paths` and does not set
`return_chunked` (see "Returning files in chunks"); it sends the record
immediately before the terminal record. This request-field gate keeps the new
record out of streams for callers that cannot request returned files. The
exec-remote/v1 types define no capability handshake; a caller must send non-empty
`return_paths` only to a runner that advertises support. The executor advertises
this support through its own `Hello` capability.

`return_max_bytes` caps the total bytes in the returned files. Without
`return_chunked`, an absent value selects `RETURN_MAX_BYTES_CAP` (262,144 bytes),
and requests above that cap must be rejected with `invalid_params` identifying
`return_max_bytes`. The caller-facing types do not implement runner request
limit validation; the submission-field syntax checks above run on decode.

A successful `return_result` contains `returned_files`, possibly an empty list.
Each file's `path` is cwd-relative, `sha256` is the lowercase digest of the
contents, and `data` is standard padded base64 on the wire. A failed collection
contains `return_paths_failed` with measured and cap byte totals plus a cause.
Exactly one of `returned_files` and `return_paths_failed` must be present; callers
reject a record with both or neither. Known causes are `over_cap`, `missing`,
`escape`, `not_regular`, and `frame_limit`. The cause is open-ended: callers
preserve unrecognized strings rather than failing to decode the record.

A collection failure never changes the job's exit outcome. The runner must still
send the terminal record describing the command's actual outcome; callers must
not interpret a `return_paths_failed` cause as a process failure.

The `return_result` record is a known tag to this version of the types. Older
callers treat it as an unknown stream record, skip it, and retain its `seq` for
resume-cursor advancement. It is nonterminal, so only `terminal` ends the job
stream.

## Returning files in chunks

One `return_result` record carries at most 262,144 bytes of files. Chunked
return lets a runner send up to 10 MiB, split across records small enough for
its record size limit. Everything in this section is additive: `return_result`
keeps its exact shape and meaning, and a request without `return_chunked`
behaves as before.

### Opting in

`RunRequest.return_chunked` (default false, omitted from the wire when false)
asks for chunked return. A caller sets it only for a runner that advertises the
capability `RETURN_CHUNKED_CAPABILITY` (`exec-remote-return/v2`). The executor
advertises it in its `Hello`, as it does for `return_result` support.

With `return_chunked` set and `return_paths` non-empty, the runner sends zero or
more `return_chunk` records, then exactly one `return_manifest`, immediately
before the terminal record. It never sends `return_result` for that request.
With `return_paths` empty, `return_chunked` has no effect and no returned-file
record is sent.

With `return_chunked`, an absent `return_max_bytes` selects
`RETURN_CHUNKED_MAX_BYTES_CAP` (10,485,760 bytes). A value above it is rejected
with `invalid_params` naming `return_max_bytes`.

### `return_chunk`

`{type: "return_chunk", job_id, seq, path_index, offset, data}`. `path_index` is
the file's position in `return_paths`, from 0. `offset` is the position of
`data` in the file. `data` is standard padded base64 of 1 to
`RETURN_CHUNK_MAX_BYTES` (524,288) bytes. Chunks may be smaller than the maximum.

- Chunks come grouped by path, in ascending `path_index`. The runner never
  interleaves two paths' chunks and never returns to an earlier path.
- Each path's chunks are contiguous from offset 0: every chunk starts where the
  previous chunk for that path ended.
- A zero-byte file has no chunks.

### `return_manifest`

`{type: "return_manifest", job_id, seq, files, total_bytes, cap_bytes}`.
`files` has one entry per requested path, in request order. Each entry echoes
its requested path exactly as the caller sent it. An entry is either:

- returned: `{path, size, sha256, chunks?}`, where `sha256` is the lowercase
  hexadecimal SHA-256 of the file's bytes; or
- failed: `{path, cause, size?, chunks?}`, where `size` is the file's size when the
  runner knew it, for example for an over-cap or expired file.

Exactly one of `sha256` and `cause` is present. Decoding refuses an entry with
both or neither, and a `sha256` without `size`. `total_bytes` is the sum of the
returned entries' sizes. `cap_bytes` is the effective cap.

`chunks`, when present, is a `u32` count of the `return_chunk` records originally
sent for that path, regardless of their sizes. A producer that sets it sets it
on every entry. It is 0 for an empty file or a path refused before its first
byte, the full count for a returned file, and the count sent before failure for
`read_error`. Expired replay preserves every entry's original count. Older
producers omit `chunks`; it is not serialized as null when absent. A manifest
with counts on only some entries is malformed.

Per-path failures use `ReturnPathsFailureCause`. In chunked mode the causes
mean:

- `missing`: no such path.
- `not_regular`: the path is not a regular file.
- `escape`: the path, or a symlink it resolves through, leads outside the job
  root.
- `over_cap`: the file would take the total past the cap. This is decided
  before any of its bytes are sent.
- `read_error`: reading failed, or the file's bytes changed while it was read.
- `expired`: a replay after the runner stopped keeping the job's returned bytes
  (see Replay).
- `frame_limit` does not occur in chunked mode.

Unrecognised causes still decode as `Unknown`, keeping the raw string.

### Runner rules

These rules bind the runner; the types do not enforce runner policy.

- The runner decides the cap for each file before sending any of its bytes and
  reads at most the size it admitted. The total bytes sent never exceed the cap.
- A file whose bytes change while it is read, ending up with fewer or more bytes
  than admitted, ends `read_error`. Chunks may already have been sent for it.
  `read_error` is the only cause that may follow chunks for its path.

### Caller rules

A caller places a returned file only after the manifest verifies it: the bytes
received for the path must match the entry's `size` and `sha256`. For example,
the caller writes chunks to a temporary file and renames it into place after
verification. The bytes received for a failed path are discarded. A failed entry
whose cause this version does not recognise also discards its chunks without
an error, because a newer runner may define a cause that can follow chunks.

`ReturnAssembler` applies these rules without doing IO, so a caller and a
producer's tests check streams the same way. `ReturnAssembler::new(return_paths,
cap_bytes)` starts a check. `accept_chunk` checks each chunk's `path_index`
range, the grouping, contiguity, the 1 to 524,288 byte chunk size and the
running total against the cap, hashes the bytes, and returns them for the caller
to write. `finish` checks the manifest's entry count, that each entry echoes
its path exactly, each entry's `chunks` against the received record count when
present (including `read_error`), each returned entry's size and SHA-256,
`total_bytes`, and that only `read_error` or an unrecognised cause follows chunks. It returns one
`VerifiedReturn` per requested path. On any `ReturnStreamError`, which names
the path index and, for a chunk, its offset, the caller discards every byte
received for the job's returned files.

### Replay

While the runner keeps the job's returned bytes (the executor keeps them for one
hour), a replay through `exec.attach` re-sends the same `return_chunk` records
and `return_manifest` at the same `seq` values.

After that, a replay sends no `return_chunk` records. It sends the manifest at
its original `seq`, with every returned entry replaced by a failed `expired`
entry carrying its original `size`. Failed entries keep their causes, and
`total_bytes` is 0. Retention is all or nothing per job, never part of a path's
chunks.

Because the chunks are no longer sent, the replay has a gap in `seq` where they
were: between the last record before the chunks and the `return_manifest`. A
caller that set `return_chunked` must accept a gap in exactly that place, so a
strict gap check cannot stall the replay. The gap can only be where
`return_chunk` records were, before the manifest; every other gap keeps its
existing meaning. The resume cursor advances to the manifest's `seq` as for any
other record.

When every entry carries `chunks`, the number of missing sequence numbers
between the job's last output record and the replayed manifest must equal the
sum of `chunks` over **all** entries, whatever the chunk sizes. In other words,
`manifest.seq - last_output_seq - 1 == sum(chunks)`, including chunks from paths
that ended `read_error`. This distinguishes expired chunks from lost output.
Without counts, the older rule still assigns that gap to return chunks, but
the gap is unverified.

Use `ReturnAssembler::finish_expired_replay(manifest, last_output_seq)` for this
replay instead of `finish`. It refuses any received chunks, requires counts on
all entries or none, and checks the gap against their sum when present, returning
`ReplayGapMismatch` on a mismatch. It still checks paths, outcomes and totals,
but does not compare historical per-path counts to the replay's zero received
chunks. Use normal `finish` for live streams and retained replay; it refuses
`ChunkCountMismatch` for any entry whose count differs from the chunks received.

### Compatibility

`return_chunk` and `return_manifest` are known tags to this version of the
types. Older callers never request them, and would treat them as unknown stream
records, skip them, and retain their `seq`. Both records are nonterminal.
