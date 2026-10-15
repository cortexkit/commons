# `agent-data/v1` — capability contract

Stability: **alpha**, first version. This document is the contract; the Rust
types in this crate are its wire shapes, and `test-vectors/agent-data-v1/`
inside this crate holds its vectors. The vectors ship in the crate's package,
and the `test-vectors` feature exposes their bytes as the `vectors` module, so
a participant that depends on this crate by version tests against the same
bytes. There is no conformance crate yet.

Every item below is **pinned**: a participant must do it, and entorhinal may
rely on it. Nothing in this revision is open.

An agent's data lives in several modules. When entorhinal moves an agent
from one Mac to another, it asks each of those modules to stop the agent's
data at a cut, ship it through engram's session door, and bring it up on the
other side. This contract is what entorhinal calls. The move protocol itself
(the move record and its states) is entorhinal's. The door ops (`cut.*`,
`session.*`) are engram's.

## Terms

The crate's source files use these words as defined here.

- **entorhinal**: the CortexKit project registry module. It knows which
  agents exist and on which Mac each one lives, and it is the only caller
  of this contract.
- **engram**: the CortexKit module that stores session data and carries it
  between Macs.
- **Participant**: a module that holds part of an agent's data (for
  example Magic Context, broca, the OpenCode and Pi harnesses, thalamus)
  and serves this contract.
- **Move**: one transfer of an agent's data from one Mac to another,
  decided and recorded by entorhinal in its move record. Each move has one
  `cutId`.
- **Move record**: entorhinal's durable record of one move and the state
  it has reached (for example `published` once the cut is sealed,
  `activating` while receivers make it live, `activated` when they have).
  The move protocol and its states are entorhinal's; this contract names
  the states only where an op depends on them.
- **Claim**: being the Mac an agent lives on, as the move record says. The
  source holds it until the driver starts activating the receiver; from
  then on the receiver holds it, which is why a move cannot be aborted
  after `activate`.
- **Cut**: the point where an agent's data ends for one move. For each
  stream of data it is a byte length (`endOffset`) and the SHA-256 of the
  bytes up to it (`digest`). Bytes written after the cut do not move.
- **The door**: engram's `session-door` service, the only path data bytes
  take between Macs. Its ops this contract names are `session.admit` (the
  source hands bytes to engram), `session.cover` (the source checks what it
  admitted is recorded), `cut.seal` (entorhinal fixes the cut's streams,
  lengths and digests), `cut.accept` (a Mac takes its containers back on a
  move back), `cut.read` and `session.read` (the receiver reads the cut's
  bytes).
- **Driver**: entorhinal in its role of running a move: it calls the ops
  below in order, retries or aborts after a refusal, and shows the operator
  what needs a human.
- **Source** and **receiver**: the Mac the agent moves from, and the Mac it
  moves to. "Source side" and "receiver side" mean the participant on that
  Mac. A participant is the source of one move and may be the receiver of
  the next. **This Mac** is the Mac of the participant answering.
- **Step**: how far one `cutId` has got on one participant: `none`,
  `prepared`, `exported`, `imported`, `activated`, `aborted` or `released`
  (§8).
- **Freeze**: what `prepare` sets on the source: local writes to the refs
  stop (where the participant can stop them, `frozen: true`), so nothing
  lands past the cut. `abort` lifts it; `release` turns it into a
  tombstone.
- **Tombstone**: what `release` leaves on the source: a durable marker that
  these refs now live on another Mac, naming the new holder. The old copy
  is hidden, and local writes are refused when the reply says `enforced`.
- **Fork**: two Macs each holding data for the same refs that the other
  lacks, for example writes on the source after an unfrozen cut, or on
  the receiver after `diverged`. Entorhinal never resolves a fork by
  itself: it offers the operator the fork choice, which copy to keep.
- **`at_cut`**: a `diverged` answer meaning this Mac has nothing for these
  refs that the cut doesn't contain, so importing the cut loses nothing
  (§7.6).
- **External data**: part of an agent's data that is a file another program
  owns and copies itself, so it never goes through the door; the
  participant only reports it and checks it on arrival. Today the only
  external data is the Claude Code transcripts behind THALAMUS's harness
  (§7.1).
- **AGENT_SYNC**: the CortexKit agent sync design, owned by the fleet's
  agent-sync work. It defines per-agent sync classes and the data-ownership
  questions this contract leaves out (§10). It defines the four
  `sync_class` values listed in §7.1.

## 1. Identity and addressing

- A participant lists `agent-data/v1` in its manifest's
  `capabilities.provides` (`PROVIDES`). Entorhinal finds participants
  through the daemon catalog.
- A participant answers all ten ops (`REQUIRED_OPS`), since any participant
  can be a source or a receiver. Each op is a request whose method is the
  op's name (`ops`):

  | Op | Method | Changes state |
  | --- | --- | --- |
  | describe | `agent_data.describe` | no |
  | busy | `agent_data.busy` | no |
  | status | `agent_data.status` | no |
  | prepare | `agent_data.prepare` | yes |
  | export | `agent_data.export` | yes |
  | diverged | `agent_data.diverged` | no |
  | import | `agent_data.import` | yes |
  | activate | `agent_data.activate` | yes |
  | abort | `agent_data.abort` | yes |
  | release | `agent_data.release` | yes |

- The ops are served on an internal service, `agent-data` (`SERVICE_ID`),
  never on a module's tool or management surface. Entorhinal opens a route
  with target `InternalService { module_id: <participant>, service_id:
  "agent-data" }`. The participant declares a matching role in its
  manifest's `provides`:
  `{"role": "internal_service", "service_id": "agent-data", "transport":
  "bulk", "agent_facing": false, "operations": [<the ten method names>]}`.
  The daemon refuses a route to a service the module didn't declare, so the
  capability in `capabilities.provides` is for discovery and this role is
  what makes the route openable. Both are required.
- Callosum never re-exports `agent-data/v1`. A move never reaches another
  Mac through the router, only through engram's door.

## 2. Admission

- The caller is the principal the daemon stamps on the route bind, never
  anything in the request body.
- Every op, the reads included, is accepted only from `reserved:entorhinal`
  (`ADMITTED_CALLER`, `caller_admitted`). Any other caller (sessions, flows,
  `direct`, other modules) is refused `caller_forbidden`. No op is open to
  `direct`.
- Admission is decided before anything else: a refused caller learns
  nothing about the request or the participant's state.

## 3. Strictness and size

- Requests and replies are JSON objects. Top-level fields, and every
  sub-object this contract types, are strict: an unknown field is
  `invalid_request`. A request that does not decode for any reason (an
  unknown or missing field, a mistyped value, more than 256 refs, a ref
  listed twice, a malformed `cutId`) is refused `invalid_request` before
  any state is read (`errors::decode_request`, `request::Request::decode`).
- The values a participant passes through opaquely are tolerant, so a newer
  participant can add to them without breaking an older entorhinal:
  `describe.detail` (any JSON value), `prepare.items[].format` (any text)
  and `prepare.external[].check` (any JSON value).
- The refusal body (§6.2) is the daemon's ERROR body and is decoded
  leniently, like the other roles' ERROR bodies; the `detail` objects this
  contract types are strict.
- A request carries at most 256 refs (`MAX_REFS`). Each ref yields at most
  one item and one external entry, so the largest reply, `prepare`'s, has at
  most 512 entries; it stays under the 3 MiB working frame (the daemon cuts
  a connection at 4 MiB) while each entry encodes to under 6 KiB. Data
  bytes never travel in these messages: they go through the door.
  `describe` lists are bounded by the participant's own ref kinds and
  stores.

## 4. Refs and `sessionRef`

- A **ref** is `{ref_kind, ref}` (`refs::Ref`): an opaque handle for one
  piece of an agent's data, the same pair the orchestrator or host
  registered with `agent.register_session`. Both members are non-empty
  strings (`refs::check_ref`); an empty one is `invalid_request`.
- A request's `refs` list each ref at most once (`refs::check_refs`): a
  list naming the same ref twice is `invalid_request` in every op that
  carries refs, since a duplicate in `prepare` would give `cut.seal` two
  items for one stream. An empty list is allowed.
- A request's `refs` keep their order. Replies that answer per ref (`busy`,
  `diverged`) answer in request order, one entry per ref. Order never makes
  two lists different elsewhere: idempotency (§5) and the per-cut ref-set
  rule (§8.1) compare refs as a set (`refs::Refs::same_set`).
- Entorhinal sends each ref only to the participant whose `describe`
  declares its `ref_kind`.
- **On the source**, a participant refuses a ref it doesn't hold with
  `unknown_ref`, for the whole request, naming the first such ref in
  request order (`detail: {ref_kind, ref}`). The source-side ops are
  `busy`, `prepare`, `export` and `release`.
- **On the receiver**, `diverged` and `import` accept refs the participant
  doesn't hold yet: that is a first-time import. For such a ref `diverged`
  answers `at_cut`.
- `status` and `abort` never refuse `unknown_ref`: either can reach a
  receiver for a `cutId` whose `import` never ran there.
- **`sessionRef`** is exactly the value
  `cortexkit_role_llm_runner::session_ref(harness, project_root, session)`
  returns (commons 8ebc492, `cortexkit-role-llm-runner` 0.2.1): the first 16
  bytes of a SHA-256, as 32 lowercase hex characters, the handle
  participants already share. Nobody derives their own
  (`refs::is_session_ref` checks the spelling only).

## 5. `cutId`, idempotency and lost replies

- **`cutId`**: 16 random bytes in lowercase hex, 32 characters, minted by
  entorhinal per move (`refs::CutId`, `refs::check_cut_id`). Uppercase hex
  or any other length is `invalid_request`. Every op except `describe` and
  `busy` carries one.
- **Idempotency**: every op that changes state (`prepare`, `export`,
  `import`, `activate`, `abort`, `release`) is keyed by `(cutId, op)`, and
  the whole request is compared (`idempotency::compare`):
  - a repeat that is the same request returns the first reply. `refs`
    compare as a set: a driver that rebuilds its list after a crash may
    reorder it, and a reordered list is the same request, never
    `id_reused`, which would stop the move for good;
  - a repeat with any field different (another ref set, `holder` on
    `release`, anything else) is refused `id_reused`.
  - The comparison is on the decoded request: member order and whitespace
    on the wire never make two requests differ.
  - The reads `describe`, `busy`, `status` and `diverged` are exempt: they
    are answered afresh every time.
- What is recorded for a key is a reply. A refusal records nothing, so after
  a refusal the same `(cutId, op)` is decided afresh; that is what lets the
  driver retry `busy`, `cut_not_ready`, `external_not_present` and
  `unavailable` with the same request.
- A repeat is decided before the step machine (§8): a repeat of the same
  request gets the first reply even though the step has since moved on.
- **Lost replies**: a transport failure, timeout or connection drop means
  the outcome is unknown, never "not done". The driver resolves it with
  `status`, or by sending the same op again, which idempotency makes safe. A
  participant makes every op safe to repeat after a crash between doing it
  and replying.
- **Durability**: the freeze `prepare` makes and the tombstone `release`
  makes are durable before those ops reply, so a participant restart can't
  let a write land past the cut.

## 6. Refusal codes

The closed set (`errors::RefusalCode`). The driver column is what entorhinal
does next (`errors::DriverAction`, selected by `errors::driver_action_for(op,
code)`). The code-only `RefusalCode::driver_action()` remains unchanged for
existing callers; it returns Abort for `digest_mismatch`.

| Code | Meaning | Driver |
| --- | --- | --- |
| `caller_forbidden` | Not `reserved:entorhinal`. | Abort |
| `invalid_request` | Malformed request, unknown typed field, or a ref listed twice. | Abort |
| `unknown_ref` | Source side: this participant doesn't hold that ref. `detail` names it. | Abort |
| `id_reused` | Same `(cutId, op)` with a different request. | Abort |
| `invalid_state` | Op out of order for this `cutId`, or naming another ref set than the `cutId` started with (§8.1). `detail.step` names the current step; `detail.refs` names the expected refs in the second case. | Abort |
| `cut_unknown` | No state for this `cutId` here (for example, `export` before `prepare`). | Abort |
| `cut_not_ready` | The cut isn't readable through the door yet. | Retry within the step |
| `busy` | The data is in use: on `prepare` it can't be cut now; on `import` a run started on the receiver since `diverged`; on `activate` the staged data can't go live now. `detail.refs` lists the refs with a reason each. | Retry later |
| `not_exportable` | This data must never leave this Mac (for example, broca sessions with caller-chosen retention). | Abort; the move is refused |
| `incompatible_version` | This participant's store can't accept this cut. `detail.found` and `detail.required` give both versions. | Abort; upgrade that Mac first |
| `digest_mismatch` | Bytes this participant receives or holds don't hash to their recorded digest: on `import` the incoming cut; on `activate` and `abort` this participant's own staged bytes or undo images. Optional `detail.reason` identifies the failed bytes (§6.2). | Abort on `import`, never retry the same import; Terminal for the step on `activate`, never retry or return the claim, hold as a fork for the operator (§7.8); Retry after the operator repairs the participant's undo state on `abort`, holding the move stuck, not aborted (§7.9) |
| `local_advanced` | Receiver side: this Mac's copy of these refs gained data after `diverged` answered `at_cut`. | Terminal for the step, never retried. After `import` the driver aborts the move and offers the fork choice; after `activate` the move holds as a fork for the operator (§7.7, §7.8) |
| `external_not_present` | The host's own data for a ref isn't on this Mac yet, or doesn't match the cut's check. `detail.note` repeats the `note` of that ref's `prepare.external` entry. | Retry after the operator copies it |
| `unavailable` | Temporary failure. | Retry |

"Retry" always means the same request again; idempotency (§5) makes that
safe.

### 6.1 Unknown codes

- A participant cannot invent codes. A code outside this set still decodes,
  as `RefusalCode::Unknown`, kept as received, so one unfamiliar code never
  makes the refusal undecodable.
- A code outside the set is never retried. The driver aborts the move and
  shows the code.

### 6.2 The refusal body

- A refusal is the daemon's ERROR body, `{code, message, detail?}`
  (`errors::ErrorBody`). `message` is short operator text and is never
  parsed.
- Five codes carry a typed `detail` (`errors::Detail`), and it is required
  for them:

  | Code | `detail` |
  | --- | --- |
  | `unknown_ref` | `{ref_kind, ref}`: the first ref of the request the participant doesn't hold. |
  | `invalid_state` | `{step, refs?}`: the participant's current step for the `cutId` (§8). `refs` is present only for a ref-set mismatch (§8.1): the refs the `cutId` started with, in the order its first op named them. |
  | `busy` | `{refs: [{ref_kind, ref, reason_code?, reason}]}`: the refs that are in use, each with operator text and optionally a `reason_code` from the same set the `busy` op's reply uses (§7.2). |
  | `incompatible_version` | `{found, required}`: the version the cut carries and the one this store needs, both opaque text. |
  | `external_not_present` | `{note}`: the operator note `prepare` gave for the host's data. |

- `digest_mismatch` may optionally carry `{reason?}`
  (`errors::DigestMismatchDetail`). `reason` is an open string; known values
  are `staging_digest_mismatch` and `undo_digest_mismatch`
  (`errors::digest_mismatch_reason`). Use
  `ErrorBody::digest_mismatch_detail()` to decode it; `typed_detail()` keeps
  its existing five-code behavior. The remaining codes carry no detail.

### 6.3 Which op may refuse with which code

Each op refuses only with the codes in its row (`errors::allowed_refusals`,
`errors::may_refuse`). A code outside an op's row is a participant bug; a
code outside the closed set is in no row.

| Op | Codes |
| --- | --- |
| describe | `caller_forbidden`, `invalid_request`, `unavailable` |
| busy | `caller_forbidden`, `invalid_request`, `unavailable`, `unknown_ref` |
| status | `caller_forbidden`, `invalid_request`, `unavailable` |
| prepare | `caller_forbidden`, `invalid_request`, `unavailable`, `id_reused`, `unknown_ref`, `invalid_state`, `busy`, `not_exportable` |
| export | `caller_forbidden`, `invalid_request`, `unavailable`, `id_reused`, `unknown_ref`, `invalid_state`, `cut_unknown` |
| diverged | `caller_forbidden`, `invalid_request`, `unavailable`, `external_not_present`, `cut_not_ready`, `incompatible_version` |
| import | `caller_forbidden`, `invalid_request`, `unavailable`, `id_reused`, `invalid_state`, `cut_not_ready`, `incompatible_version`, `digest_mismatch`, `busy`, `local_advanced` |
| activate | `caller_forbidden`, `invalid_request`, `unavailable`, `id_reused`, `invalid_state`, `cut_unknown`, `external_not_present`, `busy`, `local_advanced`, `digest_mismatch` |
| abort | `caller_forbidden`, `invalid_request`, `unavailable`, `id_reused`, `invalid_state`, `digest_mismatch` |
| release | `caller_forbidden`, `invalid_request`, `unavailable`, `id_reused`, `unknown_ref`, `invalid_state`, `cut_unknown` |

## 7. Ops

### 7.1 `agent_data.describe {}`

- Request: `{}` (`describe::DescribeRequest`). Any member is
  `invalid_request`.
- Reply: `{ref_kinds: [string], stores: [{name, sync_class}], external:
  bool, detail?}` (`describe::DescribeReply`).
  - `ref_kinds`: the ref kinds this participant holds.
  - `stores`: its per-agent stores, each with the `sync_class` AGENT_SYNC
    assigns it (Terms). The set is closed; a store declares exactly one of:

    | `sync_class` | AGENT_SYNC class |
    |---|---|
    | `agent` | 1, owned by one agent |
    | `shared_one_writer` | 2, shared, with one writer |
    | `shared_merged` | 3, shared and merged |
    | `machine_local` | 4, local to this Mac |

    On the wire it is a plain string, and the crate's `sync_class`
    constants give the spellings. What each class means for a move is
    AGENT_SYNC's to define.
  - `external`: true when part of an agent's data is a file another program
    owns and ships itself. Today that is only the Claude Code transcripts
    behind THALAMUS's harness. A participant whose data a door item can
    carry ships it through the door as items and declares `external:
    false`: the OpenCode and Pi harnesses do.
  - `detail`: optional and opaque. It accepts any JSON value, and entorhinal
    passes it on without reading it, so a newer participant can add to it
    without breaking an older entorhinal.
- A read. No step change.

### 7.2 `agent_data.busy {refs}`

- Request: `{refs}` (`busy::BusyRequest`). No `cutId`.
- Reply: `{refs: [{state, reason_code?, reason?}]}`, in request order
  (`busy::BusyReply`). `state` is `idle`, `busy` or `unknown`.
  - `busy` covers anything that would change the data during a move: a turn
    or run in flight, a queued send, an open approval card, a background
    job.
  - `unknown` counts as busy.
  - `reason_code` is a short code the phone can show without parsing text,
    for example `turn_in_flight`, `run_active`, `send_queued`,
    `approval_open`, `historian_running`, `dreamer_running`
    (`busy::reason_codes`). The set is open. `reason` is short operator
    text.
- Refusals: `unknown_ref` (source side, §4).
- A read. No step change.

### 7.3 `agent_data.status {refs, cutId}`

- Request: `{refs, cutId}` (`request::CutRequest`).
- Reply: `{step}` (`status::StatusReply`), one of `none`, `prepared`,
  `exported`, `imported`, `activated`, `aborted`, `released`. A `cutId` this
  participant never saw answers `none`.
- A read, used by the driver to learn whether an op whose reply was lost
  took effect (§5). No step change.

### 7.4 `agent_data.prepare {refs, cutId}`

On the source: decide where the data ends, and freeze it where possible.

- Request: `{refs, cutId}`.
- Reply (`prepare::PrepareReply`):

  ```
  {items: [{ref_kind, ref, sessionRef, stream, endOffset, digest, format, frozen}],
   external: [{ref_kind, ref, note, check}]}
  ```

- **Fields that feed the seal:** `sessionRef`, `stream`, `endOffset` and
  `digest` go into `cut.seal` unchanged.
  - `sessionRef` is computed with `cortexkit_role_llm_runner::session_ref`
    (§4).
  - `stream` is this participant's principal (`reserved:<module_id>`).
  - `endOffset` is a length in bytes of that stream, an unsigned 64-bit
    integer.
  - `digest` is SHA-256 over the stream's bytes `[0, endOffset)`, lowercase
    hex, matching the door.
- **`format`:** opaque text naming the data's store version. The receiver's
  `import` checks it before reading, and refuses `incompatible_version` if
  it can't accept it.
- **`frozen`:** `false` means the participant can't stop writes. Anything
  written after `endOffset` stays on this Mac and becomes a fork if the
  agent is used here again.
- **`external`:** data the host program ships itself; only the Claude Code
  transcripts behind THALAMUS's harness (§7.1).
  - `note` is the operator text (for example, "Claude Code session
    transcript: copy it with the project").
  - `check` is opaque to entorhinal and required: whatever the receiving
    participant needs to recognise the right host data (for THALAMUS, the
    host's native session id, the last host row id, and a digest of the
    transcript up to it).
- **Before replying:** the participant checks with `session.cover` that
  everything it admitted so far is covered, and makes the freeze durable.
- **Refusals:** `busy` (checked again here, atomically with the freeze),
  `not_exportable`, `unknown_ref`.
- **Step:** `none` → `prepared`.

### 7.5 `agent_data.export {refs, cutId}`

On the source: make sure every item's bytes up to `endOffset` are admitted
through the door (`session.admit` on engram's `session-door` service).

- Request: `{refs, cutId}`.
- Reply: `{items: [{sessionRef, endOffset}]}` (`export::ExportReply`), once
  all are admitted. Entorhinal then calls `cut.seal`.
- Refusals: `cut_unknown` (no `prepare` for this `cutId`), `unknown_ref`,
  `invalid_state` (another ref set than `prepare`'s, §8.1).
- **Step:** `prepared` → `exported`.

### 7.6 `agent_data.diverged {refs, cutId}`

On the receiver, before import: compare this Mac's copy with the cut.

- Request: `{refs, cutId}`.
- Reply: `{refs: [{state}]}`, in request order (`diverged::DivergedReply`).
  - `at_cut`: this Mac has nothing for these refs that the cut doesn't
    contain. For an append-only stream that is the byte-prefix rule: this
    Mac's bytes are a prefix of the cut's. For a row store, its newest rows
    match the last cut it exported or imported. A ref this participant
    doesn't hold yet answers `at_cut`.
  - `advanced`: this Mac has data for these refs that the cut doesn't
    contain. That includes host data past an external `check`.
  - `unknown`: the participant cannot tell whether this Mac's copy is at
    or past the cut (for example, it could not read its copy).
- `advanced` anywhere stops the move and shows the operator the fork
  choice. `unknown` blocks too, and is never read as `at_cut`.
- May refuse `external_not_present`, and `cut_not_ready` (retried within
  the step, as on `import`). The receiver runs `diverged` as soon as the
  move record reaches `published`, and on a Mac that didn't seal the cut,
  reading it through the door can lag. A participant whose `diverged` reads
  the incoming cut answers `cut_not_ready` while it can't yet read it,
  never `unknown`.
- May refuse `incompatible_version`, terminal and never retried, with the
  same meaning as on `import`: this Mac can't read the cut's `format`, so
  the receiving Mac must be upgraded first. A participant whose `diverged`
  reads the incoming cut answers `incompatible_version` instead of
  `unknown` when it can't parse the cut's items.
- A read. No step change.

### 7.7 `agent_data.import {refs, cutId}`

On the receiver: read the cut through the door (`cut.read`, then
`session.read` with the `cutId`, which needs no lease) into staging, and
check every digest.

- Request: `{refs, cutId}`. Refs this participant doesn't hold yet are
  accepted (a first-time import).
- Reply: `{items: [{sessionRef, endOffset, digest}]}`
  (`import::ImportReply`).
- **A released copy here:** if this Mac holds a released (tombstoned) copy
  of the same refs, which happens on a move back, the import replaces it.
  That's the expected case.
- **Compatibility:** the receiver decides compatibility from the cut's
  authenticated bytes, reading only as far as it needs to decide (typically
  the first frame). It stages privately, and on `incompatible_version` it
  discards the staging and installs nothing; nothing becomes visible before
  the decision.
- **Refusals:** `incompatible_version` (decided as above), `cut_not_ready` (the cut isn't
  readable through the door yet; the driver retries it within the step),
  `digest_mismatch`, `busy` and `local_advanced`.
  - `busy`: the receiving Mac isn't frozen, so a run can start on the same
    session between `diverged` and `import`. Retryable; the participant
    stays at its current step and the driver retries later.
  - `local_advanced`: the participant re-checks divergence atomically with
    staging, and answers `local_advanced` instead of overwriting when this
    Mac's copy gained data after `diverged` answered `at_cut`. Terminal for
    the step, never retried; what follows is the driver's. The receiver
    hasn't taken the claim yet, so the driver aborts the move and offers
    the operator the same fork choice as `diverged` answering `advanced`.
    Refusing leaves the participant's local copy untouched, so whichever
    side the operator picks can still be applied.
- **Step:** `none` → `imported`.

### 7.8 `agent_data.activate {refs, cutId}`

On the receiver: make the staged data live.

- Request: `{refs, cutId}`.
- Reply: `{}` (`EmptyReply`).
- Refuses `external_not_present` until the host data matches its `check`,
  and `busy` while the participant's own data for these refs is in use and
  can't go live yet (retryable; the driver retries later). In both cases
  the participant stays at `imported`. Also `cut_unknown` (no `import` for
  this `cutId`) and `invalid_state` (another ref set than the one `import`
  staged, §8.1).
- `local_advanced`: the participant re-checks divergence atomically with
  swapping the staged data in, and answers `local_advanced` instead of
  overwriting when this Mac's copy gained data after `diverged` answered
  `at_cut`. Terminal for the step, never retried; what follows is the
  driver's. The driver cannot abort here: the receiver already holds the
  claim, other participants may already be live, and `abort` after
  `activate` is refused. The move holds at `activating` as a fork waiting
  for the operator, never retried or aborted automatically, and fork
  resolution settles it. Refusing leaves the participant's staged import
  intact and its local copy untouched (it stays at `imported`), so
  whichever side the operator picks can still be applied.
- `digest_mismatch`: before a destructive replace, the participant checks its
  own staged bytes and undo images against their recorded digests. A mismatch
  refuses activation without replacing data; the participant stays at
  `imported`. Like `local_advanced` here, it is terminal for the step: the
  move holds as a fork for the operator, never retried and never returning the
  claim, because other participants may already be live.
- **Step:** `imported` → `activated`.

### 7.9 `agent_data.abort {refs, cutId}`

On either side, only before the move record reads `activated`. The source
unfreezes and removes its durable freeze; the receiver drops its staging.

- Request: `{refs, cutId}`.
- Reply: `{}`.
- Aborting a `cutId` this participant never saw succeeds and touches no
  data. Its step reads `aborted` afterwards, so a delayed `prepare` or
  `import` for that `cutId` is refused `invalid_state` instead of starting
  a move the driver has given up on.
- Refused `invalid_state` after `activated` (receiver) or `released`
  (source), and for another ref set than the `cutId` started with
  (`prepare`'s on the source, `import`'s on the receiver; §8.1). An abort of
  a `cutId` never seen has no such set and is never refused for its refs.
- `digest_mismatch`: the participant's own staged bytes or undo images no
  longer match their recorded digests. The abort holds until the operator
  repairs that participant's undo state, then the driver retries the same
  abort. While it holds, the move is stuck, not aborted, and the participant
  stays at its earlier step. On the source Mac the agent stays frozen because
  the freeze could not be lifted. Never retry automatically before repair.
- **Step:** any earlier step (`none`, `prepared`, `exported`, `imported`),
  or `aborted` itself, → `aborted`.

### 7.10 `agent_data.release {refs, cutId, holder}`

On the source, after the move record reads `activated`. The freeze becomes
a durable tombstone naming the new holder (`holder` is its author
pseudonym), and the old copy is hidden.

- Request: `{refs, cutId, holder}` (`release::ReleaseRequest`).
- Reply: `{enforced: bool}` (`release::ReleaseReply`). `enforced` is
  required.
  - `true`: later local writes for these refs are refused, naming the
    holder, by the participant to its own local callers, with the
    participant's own error codes (§10).
  - `false`: the tombstone is recorded and the old copy hidden, but writes
    aren't blocked, so anything written afterwards becomes a fork that
    `diverged` reports (`advanced`) on a move back.
- Refusals: `cut_unknown` (no `prepare` for this `cutId`), `unknown_ref`,
  `invalid_state` (another ref set than `prepare`'s, §8.1).
- **Step:** `exported` → `released`.

## 8. The step machine

A participant keeps per-`cutId` state, the step (`step::Step`), and refuses
an op that skips a step with `invalid_state`, naming its current step in
`detail.step`. An op that needs earlier state the participant doesn't have
is `cut_unknown`. `step::advance` is this table; it decides a first request
for `(cutId, op)` only, since a repeat is decided by idempotency first (§5).

| Op | From | To | From `none` | From any other step |
| --- | --- | --- | --- | --- |
| prepare | `none` | `prepared` | (allowed) | `invalid_state` |
| export | `prepared` | `exported` | `cut_unknown` | `invalid_state` |
| import | `none` | `imported` | (allowed) | `invalid_state` |
| activate | `imported` | `activated` | `cut_unknown` | `invalid_state` |
| abort | `none`, `prepared`, `exported`, `imported`, `aborted` | `aborted` | (allowed) | `invalid_state` from `activated` or `released` |
| release | `exported` | `released` | `cut_unknown` | `invalid_state` |

The reads (`describe`, `busy`, `status`, `diverged`) never change the step
and are never refused by it. `steps.json` lists every (op, step) pair with
its outcome.

The source walks `none` → `prepared` → `exported` → `released`; the
receiver walks `none` → `imported` → `activated`. Either may end at
`aborted` before its last step.

### 8.1 One ref set per `cutId`

- Every op after the first one for a `cutId` carries exactly the ref set
  that `cutId` started with, compared as a set (order doesn't matter)
  (`step::check_ref_set`).
  - On the source the set is `prepare`'s, and the rule applies to
    `export`, `release` and the source's `abort`.
  - On the receiver the set is `import`'s, and the rule applies to
    `activate` and the receiver's `abort`.
- A different set (a ref left out or added) is refused `invalid_state`,
  with the current step in `detail.step` and the expected refs in
  `detail.refs`, in the order the first op named them. The mismatch is
  against stored per-cut state, not a malformed request. Refs left out
  would otherwise stay frozen or staged with no op left to clear them.
- The check comes after the step check (§8) and after idempotency (§5):
  an op the step machine refuses gets that refusal, and a repeat gets its
  first reply or `id_reused`.
- An `abort` of a `cutId` the participant never saw has no stored set, so
  it still succeeds and records `aborted`, whatever its refs.

## 9. A move back

The original Mac is just another receiver: `diverged`, then `import` and
`activate` on the incoming cut, after entorhinal calls `cut.accept` so the
participant can write to its door containers again. The incoming cut has
its own `cutId`, so the participant's step for it starts at `none`. There is
no separate resume op. `diverged` answers `at_cut` only when this Mac has
nothing for these refs that the incoming cut doesn't contain (§7.6): a
participant whose data is an append-only log (broca's WAL) accepts the
incoming cut only if it extends the local copy byte for byte, and a row
store only if its newest rows match the last cut it exported or imported;
otherwise `diverged` answers `advanced`.

## 10. Settled elsewhere, not in this contract

- Project memories are shared by every agent in a project rather than owned
  by one. Whether and how they move is a data-ownership question for
  AGENT_SYNC, the CortexKit agent sync design (Terms).
- A participant's own refusals to its local callers while frozen or
  released (for example broca's `session_moving`) belong on that
  participant's own wire.

## 11. Test vectors

`test-vectors/agent-data-v1/` in this crate; its `README.md` lists the original
files' cases. The additive `digest-mismatch.json` covers the 0.1.2 refusal
and driver-action amendments, including additions to the original op rows.
`SHA256SUMS` pins every file's bytes. Every canonical
vector round-trips through the types to the same canonical (RFC 8785) bytes.
Changing a vector changes the contract: bump this crate's version and say
why in the commit.
