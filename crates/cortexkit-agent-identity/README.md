# cortexkit-agent-identity

Serde-only requests and replies for the project-identity module's `context.project`,
`context.session`, `agent.resolve`, `agent.where`, and `agent.where_many` reads.
Version 0.1.0 is intended for review before publication.

The crate does no I/O, ID/path validation, or cross-field validation. Params
types refuse unknown members, as the handler does; ID syntax, `agentIds` count
and duplicates are still checked only by the handler. It uses workspace
`cortexkit-project-identity` with version requirement `0.1.3` (retained for
publication), and reuses its `Request`, `Outcome`, `Success`,
`Refusal`, `RefusalCode`, `Via`, and `Presence` types. These replies contain no
full project/workspace records to duplicate: the context document is only
metadata, not a workspace document.
Runtime dependencies are serde, serde_json, and the shared project crate.

```rust
use cortexkit_agent_identity::{AgentWhereParams, AgentWhereReply, Outcome, Request};

let request = Request::new("agent.where", AgentWhereParams::new("agent_12345678".into()));
assert_eq!(serde_json::to_value(request)?["params"]["agent_id"], "agent_12345678");
let reply: Outcome<AgentWhereReply> = serde_json::from_str(
    r#"{"result":{"known":true,"status":"unclaimed","holder":null,"holder_name":null,"epoch":0,"cut_id":null,"read_at_ms":null,"stale":false}}"#
)?;
# Ok::<(), serde_json::Error>(())
```

## Reader policy and notation

All public structs/enums are non-exhaustive and offer `new` constructors; their
fields are public. Unknown object members are ignored, except within explicitly
open JSON payloads, whose keys/values are preserved. Every string vocabulary has
`Other(String)` and preserves unfamiliar spelling without assigning meaning.
Never infer write or activation permission from an unfamiliar state.

`R` below means writer-required, non-null; `R+N` means writer-required, nullable;
`O` means omitted when unavailable. Nullable fields use `Option<T>`: as in the
project crate, readers also accept omission and serialize `None` as null. Optional
members use `Presence<T>` with omission preserved. JSON-valued `Presence<Value>`
also preserves a present null separately from a missing member. The types do not
validate the handler's presence/ordering/reachability invariants.

Requests use `Request<Params>` (`method`, `params`); successful replies use
`Outcome<Reply>` with a required `result` object. Refusals use `code`, `message`,
and optional JSON `detail`, not `result`. The bundled fixtures record absent
detail as null; that is a fixture convention, not a transport requirement. `op`,
`request`, `principal`, and `reply` in vectors are record metadata only.

## `context.project`

Request: `ContextProjectParams`, required non-null `folder: string`. No aliases
(`path`, `root`, or camelCase alternatives are not supported). The handler rejects
extra params even though these serde readers ignore unknown members.

Reply: `ContextProjectReply`.

| Field | Type / presence | Meaning |
|---|---|---|
| `project_id` | string, R+N | Persisted project ID; null for an implicit project without a row. |
| `workspace_id` | string, R+N | Workspace ID, null when unplaced. |
| `domain` | string, R+N | Stored project domain, null if absent/unpersisted. |
| `via` | `Via`, R | `root`, `containment`, `worktree`, `implicit`, `walk`, or future string. |
| `document` | `ContextDocument`, R+N | Latest accepted document metadata, null without a workspace/document. |
| `document.revision` | i64, R | Accepted revision. |
| `document.sha256` | string, R | Stored lowercase document digest. |
| `document.charter_bytes` | i64, R | UTF-8 charter byte length, not characters. |
| `document.document_bytes` | i64, R | UTF-8 document byte length. |
| `members_digest` | string, R+N | Null without workspace; otherwise digest of binary-sorted IDs joined by newline, even for an empty set. |
| `stamp` | `ContextStamp`, R | Same-transaction positions, detailed below. |
| `stamp.incarnation` | string, R | Module instance incarnation. |
| `stamp.generation` | i64, R | Machine-local journal head, not an identity version/shared position. |
| `stamp.logPosition` | i64, R | Last applied shared position, zero with logging disabled. Exact camelCase wire spelling. |

Published coverage: five records in `memory-context-v1/context-project.json`.
No reply field or document child is wholly unpopulated. Value gaps are nonzero
`stamp.logPosition` and `via` values `containment`/`walk`.

## `context.session`

Request: `ContextSessionParams` (alias of `AgentIdParams`), required non-null
`agent_id: string`, no aliases (`agentId` is not accepted). The handler rejects
extra params; unlike direct whereabouts it does not separately validate ID syntax.

Reply: `ContextSessionReply`. All five top-level members are required and read in
one transaction. `where` is named `whereabouts` in Rust.

| Field | Type / presence | Meaning |
|---|---|---|
| `agent` | `CoreAgentResolution`, R | The resolve core below, without appended `incarnation`. |
| `agent.agent_id` | string, R | Requested live agent ID. |
| `agent.status` | `AgentStatus`, R | Only `live` on success. |
| `agent.merged_into` | nullable string, R+N | Always null on success. |
| `agent.gone` | nullable `AgentGone`, R+N | Always null on success. |
| `agent.generation` | i64, R | Journal head equal to `stamp.generation`. |
| `catalog_generations` | array of `CatalogGeneration`, R | May be empty; ordered by creation time then generation ID. |
| `catalog_generations[].generation_id` | string, R | Generation ID. |
| `catalog_generations[].state` | `CatalogGenerationState`, R | **`ready`, `active`, `degraded`**, or future string. |
| `pins` | array of `ContextPin`, R | May be empty; sorted by binary name, excludes skill pins. |
| `pins[].name` | string, R | Package name. |
| `pins[].version` | string, R | Exact pinned version, not a range. |
| `pins[].kind` | `PinKind`, R | `memory-profile`, `persona`, or future string. |
| `pins[].manifest_sha256` | string, R | Manifest digest. |
| `pins[].archive_sha256` | string, R | Archive digest. |
| `pins[].publisher_fingerprint` | string, R | Publisher key fingerprint. |
| `pins[].status` | `PinStatus`, R+N | Null without a status row for the exact name/version. |
| `pins[].status.state` | `PinStatusState`, R | Admitted strings: `available`, `withdrawn`, `tombstoned`, `revoked`; preserves future strings and non-string JSON. |
| `pins[].status.blocked` | bool, R | Handler's evaluated decision; do not reconstruct from state alone. |
| `where` | `AgentWhere`, R | Every whereabouts field/presence rule below, **except** `moving.waiting_ms` is never added here. |
| `stamp` | `ContextStamp`, R | Required `incarnation: string`, `generation: i64`, `logPosition: i64`, exactly as in `context.project`. |

Successful sessions never populate `agent.gone`: unknown/retired/merged agents
are refused by whereabouts before the core resolution is assembled. The shared
core type still describes standalone resolution; there is no gone-success
session variant. A retired-session sample is a **refusal**, not such a success.

The session query selects only ready/active/degraded catalog generations, not
installed/approved/superseded. `Other` does not authorize activation. Pin state is
copied from stored JSON without type revalidation: `PinStatusState::Json` preserves
non-string projections, including null. Invalid stored JSON refuses with
`store_corrupt`; syntactically valid JSON is not necessarily an admitted status.
The `blocked` boolean remains authoritative even for a known state spelling.

Published coverage: three original session records plus two populated-session
records (live remote/releasing and retired refusal). Wholly unpopulated successful
fields across both files: `agent.merged_into`, `agent.gone` (unreachable);
`where.moving.reason`, `detail`, `hold`, `status`, `message`, `waiting_since_ms`,
`diagnostics`, `diagnostic`; `where.moveErrors` and `[].cut_id`/`[].code`;
`where.moveWarnings` and `[].cut_id`/`[].module_id`/`[].code`/`[].message`;
`where.moveDiagnostics` and `[].cut_id`/open payload members. These abbreviated
movement names all have the `where.moving.` prefix. `where.moving.waiting_ms` is
never emitted, not a sample gap. Refusal detail shows gone `reason` and `at`, but
not `into_agent_id`. Every other session field/child has a populated example.
Additional value gaps: persona pins, non-revoked/unblocked pin status, degraded
generations, held-here/unknown/stale whereabouts, and nonzero `stamp.logPosition`.
No full identity, workspace document text, memory-profile content, bundle/resources,
top-level diagnostics, or top-level incarnation is part of this reply.

## `agent.resolve`

Request: `AgentResolveParams` (alias of `AgentIdParams`), required non-null
`agent_id: string`, no aliases. The handler rejects extra params; malformed IDs
are treated like absent identities, not an invalid-request refusal.

Reply: `AgentResolveReply`, flattening `CoreAgentResolution` plus `incarnation`.

| Field | Type / presence | Meaning |
|---|---|---|
| `agent_id` | string, R | Echo of the requested ID, even if unknown. |
| `status` | `AgentStatus`, R | `unknown`, `live`, `retired`, `merged`, or future string. |
| `merged_into` | string, R+N | Merge target only for merged rows; null otherwise. |
| `gone` | `AgentGone`, R+N | Null for unknown/live; object for retired/merged. |
| `gone.reason` | `GoneReason`, R | `deleted` (not `retired`), `merged`, or future string. |
| `gone.at` | i64, R | Terminal timestamp in milliseconds. |
| `gone.into_agent_id` | string, O | Emitted for merged, omitted for deleted. |
| `generation` | i64, R | Machine-local journal head, not the agent row generation. |
| `incarnation` | string, R | Module-added instance incarnation; absent from session's core agent. |

The reply is deliberately narrow: exactly `agent_id`, `status`, `gone`,
`merged_into`, `generation`, and `incarnation`. It contains no name, role, tags,
placement, or supervisor; a head and a hire have identical reply shapes. Retired
and unknown identities are successful resolutions, not refusals. Retired gives
`status: "retired"` and `gone: {at, reason: "deleted"}`. Unknown gives
`status: "unknown"` with null `gone` and `merged_into`.

Published coverage: seven records in `agent-identity-reads-v1/agent-resolve.json`:
live head, live hire, retired, unknown, and three `invalid_request` refusals for
missing `agent_id`, non-string `agent_id`, and an extra request member. Each
refusal has null `detail`. The params types refuse an extra member too, as the
handler does.

Two more records in `agent-identity-reads-v1/agent-resolve-merged.json` were
captured through a real merge: the merged agent (`status: "merged"`, populated
`merged_into`, and `gone` with `reason: "merged"` and `into_agent_id`) and the
surviving agent, which is live. Every resolve field is therefore present in a
captured sample.

## `agent.where` and `agent.where_many`

Requests have no aliases and the handler rejects extra params:

| Op / params type | Required non-null param | Handler-only checks (not done by this crate) |
|---|---|---|
| `agent.where` / `AgentWhereParams` | `agent_id: string` | `agent_` followed by 8 or 16 lowercase hex digits. No `agentId` alias. |
| `agent.where_many` / `AgentWhereManyParams` | `agentIds: array of strings` | 1–64 unique valid IDs. Exact camelCase; no `agent_ids` alias. |

`agent.where` returns `AgentWhere` (`AgentWhereReply` alias) directly under
`result`, with no incarnation/stamp. `agent.where_many` returns
`AgentWhereManyReply`: required `agents: array` (possibly empty), each entry
containing required `agent_id: string` and flattened `AgentWhere` fields below.
The handler preserves request order and skips unknown IDs; any other per-agent
refusal aborts the whole read. `AgentWhereEntry.whereabouts` is flattened in JSON.

| Field | Type / presence | Meaning |
|---|---|---|
| `known` | bool, R | Local cache exists or sharing disabled; foreign reads always false. |
| `status` | `WhereStatus`, R | `unclaimed`, `held_here`, `held_elsewhere`, or future string. |
| `holder` | string, R+N | Cached holder, null without a holder. |
| `holder_name` | string, R+N | Known member name, null without local author/holder/name. |
| `epoch` | i64, R | Cached epoch, zero without a claim. |
| `cut_id` | string, R+N | Cached cut ID; does not imply a local move. |
| `read_at_ms` | i64, R+N | Cache observation time; null without a cached claim, even with sharing off. |
| `stale` | bool, R | Independent cached bit; synthesized false only locally with sharing disabled. Foreign reads always true. |
| `moving` | `AgentMoving`, O | Last nonterminal row in creation-time/cut-ID order. |
| `moving.cut_id` | string, R | Open move ID, independent of cached `cut_id`. |
| `moving.side` | `MoveSide`, R | `source`, `receiver`, or future string. |
| `moving.step` | `MoveStep`, R | Nonterminal step (listed below). |
| `moving.peer` | string, R | Other machine's author ID. |
| `moving.peer_name` | string, R+N | Member-name lookup, null if absent. |
| `moving.reason` | `MoveReason`, O | Currently only `diverged`, emitted together with detail. |
| `moving.detail` | JSON, O | Saved diverged detail, preserved unchanged. |
| `moving.hold` | JSON, O | Saved hold, including a present null. |
| `moving.status` | `MoveStatus`, O | `waiting_for_target`, `abort_stuck`, `held_as_fork`, `contract_violation`, or future string. |
| `moving.message` | string, O | Notice explanation when status is set. |
| `moving.waiting_since_ms` | i64, O | Source/published time, falling back to row update time. |
| `moving.waiting_ms` | i64, O | Direct where/batch only; saturating elapsed time clamped to zero. Never added by session. |
| `moving.diagnostics` | array of JSON, O | Nonempty saved source/receiver errors; participant-defined items. |
| `moving.diagnostic` | JSON, O | Hold/fork/contract-violation diagnostic, preserved unchanged. |
| `moveErrors` | array of `MoveError`, O | Nonempty errors from all rows, not just current move. |
| `moveErrors[].cut_id` | string, R | Producing move. |
| `moveErrors[].code` | `MoveErrorCode`, R | Open participant/cut/hold code; known fallback `cut_accept_pending`. |
| `moveWarnings` | array of `MoveWarning`, O | Nonempty notices from all rows. |
| `moveWarnings[].cut_id` | string, R | Producing move. |
| `moveWarnings[].module_id` | string, R | Participant lacking release enforcement. |
| `moveWarnings[].code` | `MoveWarningCode`, R | `release_not_enforced` or future string. |
| `moveWarnings[].message` | string, R | Explanation including module ID. |
| `moveDiagnostics` | array of `MoveDiagnostic`, O | Nonempty diverged/plan diagnostics from all rows. |
| `moveDiagnostics[].cut_id` | string, R | Handler-inserted move ID. |
| `moveDiagnostics[]` other members | open JSON object members | Preserved in flattened `payload`, not discarded as unknown fields. |

Current nonterminal steps are `preparing`, `exported`, `sealed`, `published`,
`releasing`, `picked_up`, `imported`, `staged`, `claimed`, `activating`, `activated`.
Terminal `released`, `aborted`, `done`, `taken_over` are excluded by the handler;
future reply strings still round-trip. These types describe observed whereabouts
and move notices, not the data used to transfer an agent between machines.

Published coverage: `where.json` (four records), `where-sharing-off.json`,
`where-moving-source.json`, `where-moving-receiver.json`, `where-many.json`;
session files provide related nested core coverage. Every fixed base field, all
five base moving fields, and moving `status`/`message`/`waiting_since_ms`/`waiting_ms`
are populated somewhere. Wholly unpopulated fields: `moving.reason`,
`moving.detail`, `moving.hold`, `moving.diagnostics` and open item payloads,
`moving.diagnostic`; `moveErrors` and `[].cut_id`/`[].code`; `moveWarnings` and
`[].cut_id`/`[].module_id`/`[].code`/`[].message`; `moveDiagnostics` and
`[].cut_id`/open payload members. Batch `agents` and `agents[].agent_id` are
populated; the batch introduces no additional missing fields. Value gaps: move
steps other than published/claimed/releasing, notice statuses other than
waiting-for-target, and foreign-store reads (which synthesize unknown/unclaimed,
epoch zero, null holder/cut/read time, stale true).

## Refusals

All operations preserve unknown refusal codes/message/detail through shared
`RefusalCode::Other` and `Refusal`. Common admission/infrastructure codes are
`store_unavailable` (startup), `module_draining`, `invalid_request` (envelope),
`storage_unavailable` (no installed store), `storage_error`, `encode_failed`.
Domain errors retain their own code/message/detail.

| Op | Additional/method-specific codes and distinctions |
|---|---|
| `context.project` | `invalid_params`; unregistered existing folders resolve implicitly rather than `unknown_project`. |
| `context.session` | `invalid_params`, `unknown_agent` (also malformed IDs), `gone` with gone detail, `claim_integrity_failed`, `store_corrupt`. |
| `agent.resolve` | `invalid_request`; unknown and terminal identities are successful answers, not `unknown_agent`/`gone` refusals. |
| `agent.where` | `invalid_request`, `unknown_agent`, `gone` with gone detail, `claim_integrity_failed`; internal projection serialization can yield `invalid_params`. |
| `agent.where_many` | Same as where, but skips `unknown_agent`; duplicate/out-of-range/invalid IDs yield `invalid_request`. |

The lists above are not an exhaustive future refusal vocabulary.

## Vectors and tests

See [test-vectors/README.md](test-vectors/README.md). Each published record has its
own exact compact-reply round-trip test through a sorted-key JSON value; SHA-256
manifests cover exactly the copied JSON files. Separate synthetic tests cover
unknown enum spellings, ignored object members, open participant JSON, non-string
pin-state projections, and missing versus present-null payloads.
