# cortexkit-exec-remote-types

Version **0.2.10**: the caller-facing JSON types for `exec-remote/v1`, shared by
routing clients and executors. This is a types-only crate: no transport, runtime,
execution policy, local fallback or `subc-protocol` dependency. Package metadata
allows publication like other commons primitives; no publication is needed for
sibling path-dependency consumers.

## Caller API

| Operation | Request | Reply |
|---|---|---|
| `exec.run` | `RunRequest` | a stream of `StreamRecord` |
| `exec.attach` | `AttachRequest` | the same `StreamRecord` replay stream |
| `exec.cancel` | `CancelRequest` | `CancelReply` |
| `exec.status` | `StatusRequest` (empty object) | `StatusReply` |
| `workspace.prepare` | `PrepareRequest` | `PrepareReply` |
| `workspace.drop` | `DropRequest` | `DropReply` |

`StreamRecord` is internally tagged by `type`: known records are `accepted`,
`started`, `output`, `return_result`, `return_chunk`, `return_manifest`, or
`terminal`, with fields inline beside the discriminator.
`Started` is emitted once after acceptance when the job takes runner capacity,
before its first output or terminal; a job refused before start never has one.
The runner's command timeout runs from this moment, not from acceptance. A
caller can report that the job is now running instead of waiting for capacity,
but must not start a timeout of its own from `Started`. Older runners may omit
it, so callers keep their existing behavior when it is absent. It is progress,
never a terminal record. Its `seq` counts toward the attach resume cursor;
`queue_wait_ms` is measured by the runner and `started_at_ms` is for display only.

`return_result` carries returned files requested through `RunRequest.return_paths`
or a file-collection failure. It appears immediately before `terminal` and only
when the request supplied a non-empty path list. See [`CONTRACT.md`](CONTRACT.md)
for the cap, capability requirement, and outcome rules.

`Output` retains its `seq`, `stream`, raw `BytePayload(Vec<u8>)`, and optional
`truncated_before_seq`. Base64 is standard and padded. A chunk may split a UTF-8
character, so decoding never converts its payload to a string.

### Network access

Runs stay offline unless the caller requests `RunRequest::with_network(Network::Outbound)`.
`Network::Outbound` serializes as `"outbound"` and means the job may open outbound
connections to the internet, for example to install packages. It never grants
access to the caller's machine or local network; the runner must enforce that
isolation. This crate only carries the request and acknowledgement.

The runner reports the access actually granted through `Accepted::with_network`.
If a caller asked for `Outbound` and `Accepted.network` is not `Some(Outbound)`,
the job runs offline. `None` means the runner predates the field, not that access
was granted. Both fields are omitted when absent, preserving existing bytes.

Future strings round-trip as `Network::Unknown(String)`. A runner receiving an
unrecognised request must refuse before start with the non-transient
`RefusalReason::NetworkUnsupported` (`"network_unsupported"`), never downgrade or
upgrade access. A job granted outbound access may have had outside effects, so
the caller must not automatically rerun it after an unknown outcome.

### Execution platform

`RunRequest::with_platform` requests `Platform::Linux` (`"linux"`),
`Platform::Windows` (`"windows"`) or `Platform::Macos` (`"macos"`). An absent
platform means Linux and is omitted
from the request, preserving bytes for existing callers. Unknown platform strings
round-trip as `Platform::Unknown`.

A runner that understands this field always acknowledges the platform it will
actually use in `Accepted.platform`. An older runner may ignore the request field
and omit the acknowledgement. If a caller requested a non-Linux platform, it
must cancel the job and must not trust its result unless `Accepted.platform`
exactly matches the request. Unknown or unavailable platforms are refused before
start with `RefusalReason::PlatformUnsupported` (`"platform_unsupported"`); the
requested platform's wire name is reported in `refusal_detail`.

### Returned files

`RunRequest::with_return_paths` requests files relative to `cwd`; the empty list
is omitted and disables `return_result`. The optional `return_max_bytes` cap
defaults to `RETURN_MAX_BYTES_CAP` (262,144 bytes). Without `return_chunked`, a
larger request must be rejected as `invalid_params` naming that field. The types
crate has no runner-side request validator. See [`CONTRACT.md`](CONTRACT.md) for
the capability check required before sending paths, the stream ordering, and
the outcome rules.

#### Chunked returned files

For more than 262,144 bytes, a caller sets `RunRequest::with_return_chunked(true)`,
but only for a runner that advertises `RETURN_CHUNKED_CAPABILITY`
(`"exec-remote-return/v2"`). The cap then defaults to, and may not exceed,
`RETURN_CHUNKED_MAX_BYTES_CAP` (10 MiB). Instead of one `return_result`, the
runner sends the files' bytes as `ReturnChunk` records of at most
`RETURN_CHUNK_MAX_BYTES` (524,288) bytes each, grouped by path and contiguous,
followed by one `ReturnManifest` immediately before `terminal`. The manifest
has one `ReturnManifestEntry` per requested path: returned with its size and
SHA-256, or failed with a `ReturnPathsFailureCause` (now including `read_error`
and `expired`).

`ReturnAssembler` is a pure checker for this stream. Feed it each chunk with
`accept_chunk`, which returns the bytes to write, then the manifest with
`finish`, which returns a `VerifiedReturn` per path. Place a file only when
`finish` reports it returned; on a `ReturnStreamError`, discard every received
byte. A replay after the runner's retention window carries no chunks, so its
`seq` jumps from the record before the chunks to the manifest; a chunked caller
must accept that one gap. See [`CONTRACT.md`](CONTRACT.md) for the full rules.

### Recovery and request refusals

- `RefusalReason::LostToRestart` (`"lost_to_restart"`): an admission that had not
  started was lost during runner recovery. The caller may resubmit fresh under
  its own time budget. The tag alone does not prove the job never started; that
  guarantee comes only from the existing refused-before-start contract.
- `RefusalReason::RequestRejected` (`"request_rejected"`): the operation or its
  parameters are not permitted. Correct or avoid the request rather than retry
  it unchanged. `refusal_detail` carries the specific code, for example
  `prepare_unsupported` or `direct_timeout_refused`.
- `RefusalReason::RunnerAuthenticationFailed`
  (`"runner_authentication_failed"`): the executor could not authenticate the
  job to its runner because credentials were unavailable, a grant was refused,
  the agent or signing failed, or SSH configuration was invalid. The job's caller
  cannot fix this, and a retry is not expected to help. `refusal_detail` carries
  a safe fault code, never credential material.

These descriptions do not add retry policy to the types. Refusal detail remains
human-only: callers may quote it verbatim but must never parse it for decisions.
Older readers retain these tags as `RefusalReason::Unknown(String)`, preserving
the raw strings on serialization; all existing tags and wire shapes are unchanged.

### Terminal reports

The terminal's `ran`, `tree_hash`, and `workspace_changes` are explicit `Option`s
and always serialize, including as JSON null. `Some(Ran::None)` encodes the
string `"none"`, not null. Null workspace changes mean unavailable information;
an empty array means known absence of changes. `killed`, `pipestatus`, and request
scheduling overrides are omitted when absent. Repository status availability
fields likewise emit explicit nulls.

The terminal also carries three optional reports of state **on the server, not
copied back**:

- `git_state_changed: Option<GitStateChange>`: before/after commit IDs, symbolic
  refs (null when detached), index tree IDs, and stash counts. Commit and index
  IDs are nullable when unavailable. `GitStateChange::changed()` compares all
  four before/after pairs, including availability differences.
- `untracked_files: Option<UntrackedFiles>`: new untracked, non-ignored `paths`
  and an explicit `truncated` flag. Producers should cap the list at 100 paths.
- `ignored_writes: Option<IgnoredWrites>`: total `count` and capped `sample_paths`
  for ignored writes outside `target/`, `node_modules/`, and `dist/`. Producers
  should cap the sample at 20 paths, not the total count.

Absent reports decode as `None` and are omitted on serialization. `None` means
**not reported**, never "nothing changed". A reporting runner must send all three
fields, even with equal before/after Git values, an empty untracked list with
`truncated: false`, or a zero ignored-write count and empty sample. The runner,
not these types, enforces caps and exclusions. Consumers must not assume that
remote commits, staged changes, or generated files are present locally.

Hosted execution may additionally report `TerminalRecord.hosted` with its
placement and billed virtual-CPU milliseconds. Billing is absent while pending;
build-server jobs omit this field.

Public named structs and enums are `#[non_exhaustive]`. Construct structs with
`new`, then use `with_*` setters for optional fields, or deserialize them. Fields
remain public for inspection and mutation. `Uuid` is re-exported from uuid major
version 1, with only serde support enabled: syntax is validated on decode, while
UUID generation, version and retention policy belong to the executor.

## Additive decoding and outcome safety

Unknown object fields are ignored. **Every enum a caller receives decodes unknown
tags into a catch-all with a stated grading:**

| Enum | Catch-all | Caller grading |
|---|---|---|
| `RefusalReason` | `Unknown(String)` | Inside `refused_before_start`, the command still did not start. |
| `Network` | `Unknown(String)` | Never assume outbound access from an unknown acknowledgement. Runners must refuse unknown requests before start. |
| `Platform` | `Unknown(String)` | Runners must refuse unknown requests before start. Callers requesting non-Linux platforms must require an exact acknowledgement or cancel without trusting the result. |
| `Outcome` | `Unknown { kind: String }` | Grade like `outcome_unknown`: never assume the command did not run, and never re-run it locally. |
| `StreamRecord` | `Unknown { kind: String, seq: Option<u64> }` | Skip and keep reading; count its seq toward the resume cursor; never treat it as terminal. |
| `Killed` | `Unknown(String)` | The command was killed for a reason this version does not recognise. |
| `Ran` | `Unknown(String)` | Never treat as `Ran::None`: the command may have run. Grade like `outcome_unknown` for re-run decisions; never re-run it locally. |
| `OutputStream` | `Unknown(String)` | Deliver the chunk's bytes and seq; only the stream label is unknown. |
| `ReturnPathsFailureCause` | `Unknown(String)` | Preserve an unrecognized cause; the returned-file failure remains readable. |
| `PrepareOutcome` | `Unknown { kind: String }` | Never treat the workspace as prepared or run against it on that basis. |
| `RebuildResult` | `Unknown(String)` | Informational only. |

For an unknown stream record, **skip it and keep reading; count its seq toward
the resume cursor (`AttachRequest::from_seq`); never treat it as terminal**. If
the stream ends without a known terminal record, get the job's outcome from
`exec.status` or `exec.attach` rather than waiting forever, and grade it
outcome-unknown until one arrives. Advancing past a skipped sequence prevents
reattach from replaying from before it forever or reporting a false gap. A
present `seq` must be a `u64`; a non-`u64` value, including null, is a decode error.
An expired history is also not a refusal and must never trigger resubmission.

Each catch-all re-serializes the same raw tag. Unknown stream records also
re-serialize `seq` when present; other unknown fields are ignored, not retained.
Only an unrecognised tag enters a catch-all: known tags with missing or malformed
required fields remain errors, as does invalid base64. String enums still
require strings, not objects or arrays carrying a tag. Non-exhaustiveness is a
Rust API compatibility rule, not permission to treat malformed known records as
future variants. The string-owning catch-alls mean `Killed`, `Ran`, `OutputStream`,
and `RebuildResult` implement `Clone`, not `Copy`.

These are the caller-side shapes of the executor's protocol. The executor's own
codec rejects unknown enum tags; the catch-alls above deliberately extend
decoding for callers while preserving every known wire shape. The executor's
runner frames (`Run`, `Prepare`, `Frame`, bundle manifests,
repository candidates, sibling snapshots, wire `Refusal`, withdrawal outcomes,
heartbeats and `prepare_accepted`) are not part of this crate. `StatusRequest`
makes the no-arguments caller operation explicit; the runner codec represents it
as the unit `Frame::Status` variant instead.

## Golden vectors

`test-vectors/exec-remote-v1/` holds the golden cases a caller decodes: each is
a canonical JSON `.jcs` file with a `.sha256` of its bytes. They are copied byte
for byte from the executor's own test corpus, so caller and executor pin the
same bytes:

- **20 outcomes**: complete `{request, stream}` cases covering every known
  original outcome/refusal reason, both kills, pipeline status, lost running/queued jobs,
  and pre-snapshot rejection.
- **14 replies**: prepare (success plus seven refusal reasons), drop, cancel,
  and status, including absent workspaces, cold generations and an unreachable
  server.

Thirty-six **crate-local additions** were written in this crate, rather than copied byte for byte from the executor's own test vectors described above. They cover the cases below:

- `outcomes/crate-local-network-outbound`: an outbound request and acknowledged
  grant; 0.2.4-shaped request and accepted decoders ignore the additive fields.
- `outcomes/crate-local-unknown-network`: a future network string retained as
  `Unknown`, with a before-start refusal rather than an offline downgrade.
- `outcomes/crate-local-network-unsupported`: the known `network_unsupported`
  refusal reason for a runner that cannot grant the requested access.
- `outcomes/crate-local-platform-linux`, `outcomes/crate-local-platform-windows`
  and `outcomes/crate-local-platform-macos`: explicit platform requests and
  acknowledgements.
- `outcomes/crate-local-platform-unsupported`: a Windows request refused before
  start, with its wire name in `refusal_detail`.
- `outcomes/crate-local-unknown-platform`: an unrecognised platform retained on
  the request and refused before start with its wire name in `refusal_detail`.

- `outcomes/crate-local-unknown-refusal`: a before-start refusal carrying the raw
  `future_refusal` reason tag.
- `outcomes/crate-local-unknown-outcome`: the raw `future_outcome` terminal kind,
  without claiming a before-start guarantee.
- `outcomes/crate-local-unknown-stream-record`: `future_record` with seq 7,
  skipped while advancing the resume cursor, followed by a known terminal.
- `outcomes/crate-local-return-result-files`: a successful `return_result` with
  two base64 files immediately before terminal.
- `outcomes/crate-local-return-result-over-cap`, `-missing`, `-escape`,
  `-not-regular`, and `-frame-limit`: each known file-collection failure cause,
  while the command's terminal outcome remains successful.
- `outcomes/crate-local-return-result-unknown-cause`: an open-ended failure cause
  retained as `Unknown`.
- `outcomes/crate-local-unknown-killed`: a killed command with the unknown
  `future_kill` reason.
- `outcomes/crate-local-unknown-ran`: the `future_location` execution location,
  never a guarantee that the command did not run.
- `outcomes/crate-local-unknown-output-stream`: `future_stream`, retaining the
  chunk's raw bytes and seq.
- `replies/crate-local-unknown-prepare-outcome`: `future_prepare`, never proof
  that the workspace is prepared.
- `replies/crate-local-unknown-rebuild-result`: the informational `future_result`
  rebuild status.
- Six `outcomes/crate-local-server-reports-*` cases: `all`, `older-runner`,
  `unknown-fields` (inside each new struct), `detached-head`,
  `truncated-untracked`, and `unchanged` (all reports present with no changes).

- `outcomes/crate-local-refusal-hints` and
  `replies/crate-local-prepare-refusal-hints`: optional refusal detail and retry
  hints on the terminal and prepare containers, accepted by 0.2.2 decoders.
- `outcomes/crate-local-runner-draining` and `outcomes/crate-local-runner-disk-full`:
  the newly recognised transient refusal reasons.
- `outcomes/crate-local-lost-to-restart`, `outcomes/crate-local-request-rejected`,
  and `outcomes/crate-local-runner-authentication-failed`: the recovery, request
  rejection, and runner authentication refusals, with human-only detail codes
  where reported. An older reader retains their reason strings as `Unknown`.

Totals: **54 outcome pairs and 17 reply pairs**. Runner `frames/` and pretty
`.json` copies are intentionally excluded. The vector README documents their
encoding. Tests enumerate the entire corpus, hash each `.jcs` file's actual
bytes, and round-trip every case through typed values to the same independent
RFC 8785 canonical bytes. The nested-unknown-fields input re-encodes to the
separately pinned `all` case, dropping only unknown fields. The vectors and tests
are included in the package.

`test-vectors/exec-remote-v1/return-chunked/` holds 21 more crate-local cases
for chunked returned files, in the same `.jcs`/`.sha256` format. Each is a
record sequence with the requested paths, the cap, and the result
`ReturnAssembler` must reach: ten accepted streams and eleven refusals, each
checked by its own test. The vector README lists them.

## Verification

```sh
cargo fmt --all -- --check
cargo clippy --workspace --all-targets --locked -- -D warnings
cargo test --workspace --locked
cargo publish -p cortexkit-exec-remote-types --dry-run --locked
```
