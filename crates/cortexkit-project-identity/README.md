# cortexkit-project-identity

Transport-independent serde types for the three reads of the project-identity
module: the module that maps a folder to the project it belongs to, and lists
the known workspaces and projects. In this crate, "the handler" means that
module's request handler.
Version **0.1.0** owns request/reply data, not routing, admission, storage, Git
probing, filesystem canonicalization, or transport framing. Runtime dependencies
are only `serde` and `serde_json`. Paths, ids, epochs, and tokens stay strings;
decoding never consults the reader's filesystem.

## Operations

```rust
use cortexkit_project_identity::{
    EnumerateParams, Request, ResolveParams, ResolveProjectIdParams,
};

let by_path = Request::new("resolve", ResolveParams::new("/a/path".into(), None));
let by_id = Request::new(
    "resolve_project_id",
    ResolveProjectIdParams::new("pj-example".into()),
);
let all = Request::new("enumerate", EnumerateParams::new(None));
assert_eq!(serde_json::to_value(all)?.get("params"), Some(&serde_json::json!({})));
# Ok::<(), serde_json::Error>(())
```

| Method | Parameters | Successful result |
|---|---|---|
| `resolve` | Required `canonicalRoot` (alias `canonical_root`); optional/null `executionBinding: {root, incarnation, registrationEpoch}` | `ResolveReply`: project/workspace metadata, canonical query path, provenance, and optional flattened root-mode fields |
| `resolve_project_id` | Required `projectId` (alias `project_id`) | `ResolveProjectIdReply`: only `projectId`, `via`, `gone`, `generation`, `incarnation` |
| `enumerate` | `{}` for unfiltered reads; optional/null `workspaceId` (alias `workspace_id`) | `EnumerateReply`: required `workspaces`, `projects`, `generation`, `incarnation` |

`resolve` accepts any absolute query path, not only a registered root. Unknown
durable ids succeed with `via: "gone"` and `gone: true`; missing paths are also
successful resolutions, not not-found refusals. The types do not validate paths
or enforce method/parameter pairings. Constructors always include `params`;
missing/null params do not replace the required parameter object.

Use `Outcome<ResolveReply>`, `Outcome<ResolveProjectIdReply>`, or
`Outcome<EnumerateReply>` to decode a handler reply. Success is **`{"result": T}`**,
not bare `T`. Refusals are top-level `{code, message, detail}`; `detail` is opaque
JSON, nullable, and also accepted when absent. Identify a refusal by `code`, not
its message. The open vocabulary includes `invalid_request`, `invalid_params`,
`storage_unavailable`, `store_unavailable`, `storage_error`, `encode_failed`,
`module_draining`, and `unknown_method`. The two unavailable spellings are
distinct. A misspelled method can produce `unknown_method`; unknown domain codes
also remain readable.

## Decoding policy

- All public structs and enums are non-exhaustive, with constructors. The types
  represent data without enforcing cross-field writer invariants.
- Unknown object keys are **ignored at every level**, including requests and
  envelopes. They are not retained on serialization. No object has
  `deny_unknown_fields`. A shape with both a valid `result` and refusal fields
  decodes as success, following the untagged envelope's success-first order.
- All enum-like strings have an explicit `Other(String)` arm that preserves and
  re-encodes unfamiliar text as a JSON string, never as a tagged object. This
  applies to `Via`, `RootIdentity`, `BindingStatus`, `ApprovalState`, `RefKind`,
  `RootKeyKind`, `RootIncarnationKind`, and `RefusalCode`. `BindingStatus::Unknown`
  is the **known** wire spelling `"unknown"`, not the fallback for new text.
- Path provenance currently uses `root`, `walk`, `containment`, `worktree`, or
  `implicit`; id lookup uses `current`, `alias`, or `gone`. `registered` is
  preserved as `Via::Other`, not advertised as a current handler output.
- Required non-null fields reject omission, null, and incorrect JSON types.
  Required **nullable** fields deliberately also accept omission, as ordinary
  serde `Option` fields do; they serialize `None` as null. This reader leniency
  does not weaken the current writer contract described below.
- Omission-only fields use `Option` and skip serialization of `None`. The four
  flattened resolve root-mode fields use `Presence` to preserve missing versus
  present values; `Presence<Option<T>>` also preserves present null. No
  `rootFields` object is introduced. `rootRecords` rejects null when present.

### Writer presence

`ResolveReply` requires `projectId`, `via`, `gone`, `generation`, and process
`incarnation`. `workspaceId`, `workspaceRoot`, `projectName`, and `canonicalRoot`
are required nullable members. Current served branches always populate
`canonicalRoot`; its nullable representation mirrors the core type, not an
observed null-producing handler branch. `workspaceRoot` is operator-set, never
inferred from project paths. `worktreeOf` is omitted except for external/shared
token worktree fallback.

In root-record mode, `matchedRoot`, `sourceRoot`, `rootRecords`, and
`bindingStatus` are flattened members. They are all omitted in mode-off replies
and in `via: "worktree"` replies. Within that mode, matched/source roots and
binding status are nullable; records are an array (possibly empty). An implicit
result can have null roots and an empty records array. A supplied execution
binding is answered only when root-mode fields are present.

`Workspace` requires `workspaceId`, `name`, and nullable `root`. `Project`
requires `projectId`, `name`, `roots`, `implicit`, and `refKind`. Optional
`deviceFingerprint`, `workspaceId`, `lastRouteActivityMs`, and `rootRecords` are
omitted when unavailable. `rootRecords` is an array, including `[]`, when root
mode is enabled. Enumeration lists persisted projects and remote workspace
references, not newly materialized implicit paths; no `tags` field is emitted.
Activity is fresh liveness data, not a persisted registry timestamp.

`RootRecord` requires `root`, `remotes`, `identity`, `approval`, and nullable
`incarnation`/`registrationEpoch`. The latter two are populated only for a bound
root. `remotesError`, `rootKey`, and `rootKeyMismatch` are omission-only fields;
a mapped key includes its mismatch boolean even when false. The key object is
`{kind, rootKey}`, **not** `{kind, value}`. `Approval` requires `state` and nullable
`journalSeq`. `RootIncarnation` requires `kind` and `value`. Every `GitRemote`
requires `name`, `owner`, `repo`, and `owned`.

### Identity cautions

Do not infer registration or runnable roots from an id prefix. Empty records
do not grant a runnable root; absent records can mean mode-off or an
identity-only worktree reply. Root identity is not approval, and a worktree must
not borrow approval from `worktreeOf`.

The reply's string `incarnation` is a process-local nonce, while root-record
`incarnation: {kind, value}` and request `executionBinding.incarnation` identify
a checkout binding. Do not conflate them. Generation orders only the originating
store's journal, and the process nonce must not be compared across machines.

A root without `rootKey` is not a portable checkout address. Do not synthesize a
portable key from its local path. One repo key identifies at most one project,
but **several local roots can share the same `(project_id, root_key)`**. These
data types do not provide a lookup helper; consumers must consider all matching
roots rather than assume one checkout. Remote-probing errors and key mismatches
are data, not decoding failures.

## Samples

See [the bundled vectors](test-vectors/project-resolve-v1/README.md). Three files
hold replies captured from a running handler, normalized as that README
describes, and checked against their `SHA256SUMS`. The handler's maintainer
publishes the same files as the handler's own test data. Tests decode every
record (including the expected invalid-params request) and re-encode each reply
through a sorted-key JSON value to compare its exact compact embedded bytes.
Struct serialization alone does not promise object-key order, and unknown-key
samples intentionally lose those keys when re-encoded.

Separate, clearly labelled **synthetic** fixtures exercise unknown values and
unknown keys. Unit tests also cover aliases, binding parameters, nullable and
omitted states, all known vocabulary values, and every `Other` fallback. These
are decoder checks, not claims about captured handler branches.

Replies captured from a running handler are not yet bundled for:

- A populated `GitRemote`; root identity `unbound`, `replaced`, and `retired`.
- Supplied/null/malformed execution bindings and each non-null binding status;
  filtered/null enumeration requests and snake-case aliases.
- Non-null workspace/source roots, `containment` results, and root-mode-off
  replies; a null canonical query root is not currently a served branch.
- Remote-probe errors, root keys of both known kinds, both mismatch booleans,
  approved state with a journal sequence, and remote project references.
- Unplaced projects, activity timestamps, nonempty/omitted enumeration root
  records, and non-null/absent refusal detail.
- Other refusals, malformed envelopes, missing/non-string project ids, invalid
  enumeration parameters, and startup/draining/storage/encoding failures.
