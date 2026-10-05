# cortexkit-exec-remote-types

Version **0.1.0**: the caller-facing JSON types for `exec.remote/v1`, shared by
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

`StreamRecord` is internally tagged by `type`: `accepted`, `output`, or
`terminal`, with the record's fields inline beside the discriminator. `Output`
retains its `seq`, `stream`, raw `BytePayload(Vec<u8>)`, and optional
`truncated_before_seq`. Base64 is standard and padded. A chunk may split a UTF-8
character, so decoding never converts its payload to a string.

The terminal's `ran`, `tree_hash`, and `workspace_changes` are explicit `Option`s
and always serialize, including as JSON null. `Some(Ran::None)` encodes the
string `"none"`, not null. Null workspace changes mean unavailable information;
an empty array means known absence of changes. `killed`, `pipestatus`, and request
scheduling overrides are omitted when absent. Repository status availability
fields likewise emit explicit nulls.

Public named structs and enums are `#[non_exhaustive]`. Construct structs with
`new`, then use `with_*` setters for optional fields, or deserialize them. Fields
remain public for inspection and mutation. `Uuid` is re-exported from uuid major
version 1, with only serde support enabled: syntax is validated on decode, while
UUID generation, version and retention policy belong to the executor.

## Additive decoding and outcome safety

Unknown object fields are ignored. Unknown refusal reason tags decode as
`RefusalReason::Unknown(String)`, retaining the raw tag. Inside
`refused_before_start` this still guarantees the command did not start.
Unknown terminal outcome tags decode as `Outcome::Unknown { kind: String }`.
Callers grade an unknown outcome exactly like `outcome_unknown`: **never assume
the command did not run, and never re-run it locally**. An expired history is
also not a refusal and must never trigger resubmission.

Each catch-all re-serializes the same tag. Unknown fields are ignored, not
retained. Only an unrecognised tag enters a catch-all: known tags with missing or
malformed required fields remain errors. Other enums, including stream record
tags and prepare outcome kinds, still reject unknown tags, as does invalid
base64. Non-exhaustiveness is a Rust API compatibility rule, not permission to
treat malformed known records as future variants.

The caller shapes and copied vectors originate in prefrontal's `motor-protocol`
at commit `64fbc5dcf`. That codec rejects unknown enum tags; the two catch-alls
above deliberately extend decoding while preserving all its known wire shapes.
The executor's runner frames (`Run`, `Prepare`, `Frame`, bundle manifests,
repository candidates, sibling snapshots, wire `Refusal`, withdrawal outcomes,
heartbeats and `prepare_accepted`) are not part of this crate. `StatusRequest`
makes the no-arguments caller operation explicit; the runner codec represents it
as the unit `Frame::Status` variant instead.

## Golden vectors

`test-vectors/exec-remote-v1/` contains byte-for-byte copies of the `.jcs` and
`.sha256` pairs from the pinned caller corpus:

- **20 outcomes**: complete `{request, stream}` cases covering every known
  outcome/refusal reason, both kills, pipeline status, lost running/queued jobs,
  and pre-snapshot rejection.
- **14 replies**: prepare (success plus seven refusal reasons), drop, cancel,
  and status, including absent workspaces, cold generations and an unreachable
  server.

Two **crate-local additions** are listed separately from the copied cases:

- `outcomes/crate-local-unknown-refusal`: a before-start refusal carrying the raw
  `future_refusal` reason tag.
- `outcomes/crate-local-unknown-outcome`: the raw `future_outcome` terminal kind,
  without claiming a before-start guarantee.

Totals: **22 outcome pairs and 14 reply pairs**. Runner `frames/` and pretty
`.json` copies are intentionally excluded. The vector README documents their
encoding. Tests enumerate the entire corpus, hash each `.jcs` file's actual
bytes, and round-trip every case through typed values to the same independent
RFC 8785 canonical bytes. The vectors and tests are included in the package.

## Verification

```sh
cargo fmt --all --check
cargo clippy -p cortexkit-exec-remote-types --all-targets --locked -- -D warnings
cargo test -p cortexkit-exec-remote-types --locked
cargo package -p cortexkit-exec-remote-types --locked --allow-dirty --no-verify
```
