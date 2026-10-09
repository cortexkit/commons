# cortexkit-exec-remote-types

Version **0.2.2**: the caller-facing JSON types for `exec-remote/v1`, shared by
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
`output`, or `terminal`, with fields inline beside the discriminator. `Output`
retains its `seq`, `stream`, raw `BytePayload(Vec<u8>)`, and optional
`truncated_before_seq`. Base64 is standard and padded. A chunk may split a UTF-8
character, so decoding never converts its payload to a string.

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
| `Outcome` | `Unknown { kind: String }` | Grade like `outcome_unknown`: never assume the command did not run, and never re-run it locally. |
| `StreamRecord` | `Unknown { kind: String, seq: Option<u64> }` | Skip and keep reading; count its seq toward the resume cursor; never treat it as terminal. |
| `Killed` | `Unknown(String)` | The command was killed for a reason this version does not recognise. |
| `Ran` | `Unknown(String)` | Never treat as `Ran::None`: the command may have run. Grade like `outcome_unknown` for re-run decisions; never re-run it locally. |
| `OutputStream` | `Unknown(String)` | Deliver the chunk's bytes and seq; only the stream label is unknown. |
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

Eighteen **crate-local additions**, written in this crate rather than copied from the executor's corpus, cover the cases below:

- `outcomes/crate-local-unknown-refusal`: a before-start refusal carrying the raw
  `future_refusal` reason tag.
- `outcomes/crate-local-unknown-outcome`: the raw `future_outcome` terminal kind,
  without claiming a before-start guarantee.
- `outcomes/crate-local-unknown-stream-record`: `future_record` with seq 7,
  skipped while advancing the resume cursor, followed by a known terminal.
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

Totals: **35 outcome pairs and 17 reply pairs**. Runner `frames/` and pretty
`.json` copies are intentionally excluded. The vector README documents their
encoding. Tests enumerate the entire corpus, hash each `.jcs` file's actual
bytes, and round-trip every case through typed values to the same independent
RFC 8785 canonical bytes. The nested-unknown-fields input re-encodes to the
separately pinned `all` case, dropping only unknown fields. The vectors and tests
are included in the package.

## Verification

```sh
cargo fmt --all -- --check
cargo clippy --workspace --all-targets --locked -- -D warnings
cargo test --workspace --locked
cargo publish -p cortexkit-exec-remote-types --dry-run --locked
```
