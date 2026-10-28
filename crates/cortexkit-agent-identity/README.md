# cortexkit-agent-identity

Serde-only requests and replies for the agent registry's `context.project`,
`context.session`, `agent.resolve`, `agent.where`, `agent.where_many`,
`agent.session_for_ref`, `agent.session_for_runner`, `agent.session_for_harness`,
and `agent.fleet_identity` reads. Version 0.1.2 adds harness session lookup
compatibly to 0.1.1, including sessions without a recorded runner.

The crate does no I/O, ID/path validation, or cross-field validation. Params
types refuse unknown members, as the handler does; ID syntax, `agentIds` count
and duplicates are still checked only by the handler. It uses workspace
`cortexkit-project-identity` with version requirement `0.1.4` (retained for
publication), and reuses its `Request`, `Outcome`, `Success`,
`Refusal`, `RefusalCode`, `Via`, and `Presence` types. These replies contain no
full project/workspace records to duplicate: the context document is only
metadata, not a workspace document. Fleet project/workspace summaries use `id`,
not `projectId`/`workspaceId`, and have different fields and presence rules from
the shared crate's full records, so they have separate types here.
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
project crate, readers generally also accept omission and serialize `None` as null.
The harness lookup reply's `runner` is the exception: null is accepted but omission
is refused. Optional
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
extra params, as do these serde params readers.

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

## `agent.session_for_ref`

Request: `AgentSessionForRefParams`, required non-null strings `ref_kind` and
`ref` (Rust `session_ref`). No aliases, including `session_ref`. Unknown members
are refused. The handler checks 1–64 UTF-8 bytes for `ref_kind` and 1–512 for
`ref`; this crate does not. Reference kinds, references, harnesses and runners
are open strings, not a closed provider vocabulary.

Reply: `AgentSessionForRefReply`.

| Field | Type / presence | Meaning |
|---|---|---|
| `agent_id` | string, R | Owning agent. |
| `ref_kind` | string, R | Exact stored reference kind. |
| `ref` | string, R | Exact stored reference; Rust `session_ref`. |
| `harness` | string, R+N | Registered harness; null for a moved-in placeholder. |
| `session_id` | string, R+N | Host session ID; null together with placeholder harness. |
| `project_root` | string, R+N | Stored root; null for a legacy or initially omitted empty root. |
| `runner` | string, O | Present only when recorded; **never null**. |
| `heldOn` | string, O | Remote holder of finished session data; exact camelCase spelling. |
| `incarnation` | string, R | Handler lifetime nonce. |

A missing reference, including a structurally valid unknown kind, refuses
`not_found` (`RefusalCode::NotFound`), not a null result or `found: false`.
There is no `session`, lifecycle, network, registration or liveness field.
Unlike runner lookup, a runnerless binding can succeed and omits `runner`.

Published coverage: five `session-for-ref.json` records and records 1–6 of
`session-lookup-cases.json`. Every declared field has a populated example,
including `heldOn` from real finished-session movement. **Untested by captures:**
null `harness`/`session_id`, which require the moved-in placeholder path rather
than registration. The nullable reader accepts omission, as elsewhere in this
crate; it does not treat `session` as an alias for `session_id`.

## `agent.session_for_runner`

Request: `AgentSessionForRunnerParams`, required non-null strings `runner` and
**`session`**, never `session_id` (no alias). Extra members are refused even
when `session` is also supplied. The handler checks runner length 1–64 UTF-8
bytes without Unicode control characters and session length 1–256 bytes;
these bounds are not validated by this crate.

Reply: `AgentSessionForRunnerReply`, deliberately separate from the ref row.

| Field | Type / presence | Meaning |
|---|---|---|
| `agent_id`, `ref_kind`, `ref` | strings, R | Same owning agent and exact stored reference meanings as ref lookup. |
| `harness` | string, R+N | Nullable row type, but a found runner row cannot have null harness through real registration/carry. |
| `session` | string, R | Stored host session ID under this operation's distinct spelling. |
| `project_root` | string, R+N | Empty stored root projects to null. |
| `runner` | string, R | Exact matched runner; **never null or omitted**. |
| `heldOn` | string, O | Remote holder of finished data; omitted otherwise, never null. |
| `ended_at_ms` | i64, R+N | First end timestamp; null while open. |
| `network` | `SessionNetwork`, R | `yes`, `no`, `unknown`, or a preserved future string; latest history value, `unknown` if empty. |
| `network_history` | array of `SessionNetworkEntry`, R | Oldest first; an empty array is retained. |
| `network_history[].network` | `SessionNetwork`, R | Same network vocabulary. |
| `network_history[].since_ms` | i64, R | Timestamp in milliseconds. |
| `incarnation` | string, R | Handler lifetime nonce. |

There is no `session_id` reply member. An unknown pair, wrong runner or
runnerless binding refuses `not_found`. Runnerless rows cannot match the query;
placeholder rows have neither host field nor runner, explaining the non-null
runner guarantee. Network is a closed writer vocabulary, not a lookup status;
readers preserve unfamiliar values without assigning meaning.

Published coverage: four `session-for-runner.json` records and records 7–13 of
`session-lookup-cases.json`. Every field has a populated example, including
`heldOn`, both null and populated root/end time, empty/nonempty history and all
current network values. **Untested by captures:** null `harness`, unreachable
for a found runner row through the real writers. `runner: null` is not a case.

## `agent.session_for_harness`

Request: `AgentSessionForHarnessParams`, required non-null strings `harness` and
`session`. Unknown members are refused, including `session_id` even when `session`
is supplied. `AgentSessionForHarnessRequest` aliases
`Request<AgentSessionForHarnessParams>`; construct it with
`AgentSessionForHarnessRequest::new(AGENT_SESSION_FOR_HARNESS, params)`.
`AGENT_SESSION_FOR_HARNESS` is the `"agent.session_for_harness"` operation name.
Params are data containers; the crate does not validate handler bounds.

Reply: `AgentSessionForHarnessReply`, with the same members and wire spellings as
`AgentSessionForRunnerReply`, except `runner` is `Option<String>` (R+N). It always
serializes as a string or explicit null, never as an omitted member. Decoding a
reply with no `runner` member is refused. The runner lookup reply remains unchanged
and requires a non-null runner.

The harness and session pair can find runnerless sessions, including OpenCode or
Pi heads and sessions recorded before runners existed. An absent pair refuses
`not_found` through the existing `Outcome` and `RefusalCode::NotFound` types;
malformed params use `RefusalCode::InvalidRequest`.

Published coverage: five `session-for-harness.json` records cover a runner-backed
session, a null-runner head, a remote ended session with `heldOn`, a lookup miss,
and an unknown-field request refusal. Each reply round-trips byte-for-byte.

## `agent.fleet_identity`

Request: `AgentFleetIdentityParams`, either `{}` or `{"token": value}`. `token`
is `Presence<serde_json::Value>`: **any JSON value is accepted**, with missing
and present null kept distinct. Only exact equality with the current opaque
string token produces an unchanged reply. Stale, malformed/empty/wrong-incarnation
strings, null, numbers, booleans, arrays and objects are cache misses returning
the full reply, never token refusals. Only extra members or a malformed params
object/envelope yield `invalid_request`; no aliases exist.

Reply: `AgentFleetIdentityReply` describes both branches without validating
their cross-field invariant: full has `unchanged: false, agents: [...]` (possibly
empty), unchanged has `unchanged: true` and **omits** `agents`. Missing and
present-empty are distinct through `Presence<Vec<FleetAgentIdentity>>`.

| Field | Type / presence | Meaning |
|---|---|---|
| `generation` | i64, R | Local journal head, not comparable between machines. |
| `incarnation` | string, R | Handler lifetime nonce. |
| `logPosition` | i64, R | Last shared identity-log position applied locally, zero with sharing off; present on both branches. |
| `token` | string, R | Opaque cache token; currently `incarnation:generation`. |
| `unchanged` | bool, R | Whether the supplied string exactly matched the current token. |
| `agents` | array of `FleetAgentIdentity`, O | Every live agent sorted by ID, required on full replies; omitted on unchanged. |
| `agents[].agentId` | string, R | Stored ID. |
| `agents[].displayName` | string, R | Stored display name. |
| `agents[].tag` | string, R | May be empty for imported legacy rows. |
| `agents[].labels` | array of strings, R | May be empty; stored order preserved. |
| `agents[].role` | `FleetAgentRole`, R | `assistant`, `workspace_head`, `head`, `hiree`, or future string. No separate status field. |
| `agents[].avatar` | `FleetAvatar`, O | Summary when a genome is stored, never the genome itself. |
| `avatar.fingerprint` | string, R | First 32 lowercase hex characters of BLAKE3 over genome, type and version joined with newlines; missing version uses empty string. |
| `avatar.type` | `FleetAvatarType`, R | `creature.classic`, `creature.wild`, or future string; Rust `avatar_type`. |
| `avatar.version` | i64, O | Stored version when available, never null. |
| `agents[].project` | `FleetProject`, O | Present when project-bound; required strings `id`, `name`. |
| `project.canonicalRoot` | string, O | Lexically first canonical root, omitted for a rootless project. |
| `agents[].workspace` | `FleetWorkspace`, O | Required strings `id`, `name`; project workspace or workspace head's named workspace, omitted if unplaced/unavailable. |

All optional objects/children above are omitted, never null. Unplaced assistants
omit both placement objects. Roles and avatar types are open reader vocabularies;
unknown values confer no permission. A merged multi-machine fleet uses the reply
with the highest `logPosition`, not local `generation` or token. Token equality
does not include `logPosition`.

Published coverage: three `fleet-identity.json` records and records 14–19 of
`session-lookup-cases.json`; every declared field is populated somewhere.
**Untested by captures:** nonzero `logPosition` (**every fleet sample is 0**),
`hiree`/`workspace_head` roles, `creature.wild`, an empty fleet, and cache misses
using null/object/array/boolean/wrong-incarnation tokens. Synthetic decoder tests
cover these token representations and empty-fleet presence, not handler behavior.

For all three new reads, infrastructure failures (startup/draining, missing
store, storage and encoding errors), null/wrong-type params objects, and
empty/overlong string bounds/control-character failures are **untested by the
published captures**. Only the session lookups have string bounds; no fleet token
value is invalid. The crate does not simulate these handler checks.

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
| `agent.session_for_ref` | `invalid_request` for decode/bounds; `not_found` for an absent exact reference. |
| `agent.session_for_runner` | `invalid_request` for decode/bounds; `not_found` for an absent pair, including runnerless bindings. |
| `agent.session_for_harness` | `invalid_request` for malformed params; `not_found` for an absent harness/session pair. |
| `agent.fleet_identity` | `invalid_request` for extra params/envelope/decode, never a token value. No unknown-agent, bad-token, stale-token or `not_found` refusal; an empty fleet succeeds. |

The lists above are not an exhaustive future refusal vocabulary.

## Vectors and tests

See [test-vectors/README.md](test-vectors/README.md). Each published record has its
own exact compact-reply round-trip test through a sorted-key JSON value; SHA-256
manifests cover exactly the copied JSON files. Separate synthetic tests cover
unknown enum spellings, ignored object members, open participant JSON, non-string
pin-state projections, and missing versus present-null payloads.
The session lookup and fleet JSON files are copied byte-for-byte from published handler exports;
`session-lookup-cases.json` contains nineteen records and has SHA-256
`5e2a306b6a05bd4ab1c922c0c57541d16e3fb33c2c403a797fe5a34edb774107`.
