# `llm-runner/v1` — role contract (draft)

Stability: **alpha, draft**. Not yet reviewed by the role's consumers; nothing
here ships until they sign it off. This document is the role definition; the
Rust types in this crate are its wire shapes, `test-vectors/llm-runner-v1/` at
the repository root holds its vectors, and a separate `-conformance` crate
will hold its suite (§15).

Derives from the CK extensibility design r7.3, §3.2–§3.5, §5, §6, §9.3, §10,
§12.1, §13.2 and §16.7–§16.8, with `session.send` delivery modes replacing
separate steer and queue ops.

Every item is marked:

- **[pinned]**: the design states it. A runner must do it, and a consumer may
  rely on it.
- **[open: Qn]**: the design is silent or ambiguous, and this draft writes one
  option down so the types and vectors have something to pin. Question `Qn`
  under "Open questions" lists the options.

An LLM runner is any module that runs model sessions. Nothing here names a
particular implementation; "Gaps in broca today" at the end compares the one
shipping runner with this document.

## 1. Role identity and addressing

- [pinned] A runner lists `llm-runner/v1` in its manifest's
  `capabilities.provides` (`PROVIDES`). A module may serve several majors.
- [pinned] Required ops (`REQUIRED_OPS`): `role.describe`, `session.baseline`,
  and the compaction interface's one inbound op, `compaction.ready` (§11).
  [open: Q4] whether `compaction.ready` belongs in the required set.
- [pinned] The hook call sites (§11) are required too. They are calls the
  runner makes, not ops it serves, so `role.describe` does not list them.
- [pinned] Everything else is a declared capability group (§3). A runner
  serves a group only if it declares it, and a consumer uses a group only if
  it is declared.
- [pinned] A consumer refuses a module whose `role.describe` lacks a required
  op, by name, before routing anything to it. The check lives in the
  consumer, never in the daemon.
- [open: Q1] Every role op is a request on a route the runner declares, named
  by the op. The session it acts on is the session the route is bound to; no
  request names its session, except `compaction.ready`. How the op name and
  its params are carried in the request body is open.

## 2. `role.describe`

- [open: Q2] The answer is `{majors: [{version, ops, stability}],
  implementation_version, capabilities, session_capabilities_from}`
  (`RoleDescribe`), the shape `tool-provider/v1` uses. `version` is spelled as
  in the manifest, `llm-runner/v1`.
- [pinned] `capabilities` lists the module-level capabilities: the groups of
  §3 the runner declares, and `ordered_hook_phases` on a runner where it holds
  for every session.
- [pinned] The answer says where session-level capabilities come from (§10):
  the admission reply, on a runner that admits sessions, or
  `session.baseline`, on a runner without an admission reply.
  [open: Q3] the member is `session_capabilities_from`, valued `admission` or
  `baseline`.
- [pinned] Decoded leniently: unknown fields, majors, ops and capabilities
  are ignored, and an unknown `stability` decodes. Only what a consumer
  relies on is checked strictly (`check_describe`): the `llm-runner/v1`
  major, its required ops, every op of each declared group, and the
  session-capability source.
- [pinned] The answer describes the runner build, never a session. A consumer
  may cache it for as long as it talks to the same module incarnation.
- [pinned] `stability` is `alpha`, `beta` or `stable`. A role becomes stable
  only when a second, independent implementation passes its suite.
  [open: Q21] the guarantees of each level.

## 3. Capability groups

- [pinned] Each group is all or nothing: a runner that declares one serves
  every op in it. A declared group missing one of its ops makes
  `check_describe` refuse the answer (`group_incomplete`), because a consumer
  cannot use part of a group.

| Group | Ops (`capabilities::GROUPS`) | Section |
|---|---|---|
| `transcript_reads` | `session.read`, `session.head` | §4 |
| `run_ops` | `run.result`, `session.read` (attribution) | §6 |
| `dispatch_attribution` | `session.read` (per-call fields) | §5 |
| `streaming` | `session.read` (`head`), `session.subscribe` | §7 |
| `model_view` | `session.read` (`view: "model"`) | §8 |
| `steer` | `session.send` with `delivery: "steer"` | §9 |
| `queue` | `session.send` with `delivery: "queue"` | §9 |

- [open: Q11] A group that only adds fields to `session.read` requires
  `session.read`, not all of `transcript_reads`.
- [open: Q12] Whether `interrupt` is a group of its own.
- [pinned] What consumers require is the consumer's business, checked before
  use. A session owner that reads a runner's sessions requires the five
  read-side groups (`transcript_reads`, `run_ops`, `dispatch_attribution`,
  `streaming`, `model_view`). A context manager requires `transcript_reads`
  unless it has its own reader of the host's transcript.

## 4. `transcript_reads`: `session.read` and `session.head`

### 4.1 The request

`{from_ordinal?, after_mid?, lineage_id?, limit?, max_bytes?,
include_originals?, view?}` (`ReadRequest`). Three modes (`ReadRequest::mode`):

- [pinned] **Tail.** With no cursor, the newest page. This is a contract, not
  a default a runner may change; the suite checks it on a session longer than
  one page.
- [pinned] **Range.** `from_ordinal` reads forward from that ordinal.
- [pinned] **After a message.** `{after_mid, lineage_id}` reads strictly after
  that message. It is refused by name as `lineage_changed` (the session's
  lineage is not `lineage_id`) or `unknown_mid` (the lineage has no such
  message), and never silently answered from the tail.
- [pinned] `mid` is opaque to consumers: they only pass back a value a page
  returned.
- [pinned] A read carrying a `lineage_id` that is not the session's current
  lineage is refused `lineage_changed`, whatever its mode.
- [open: Q6] `after_mid` without `lineage_id` is refused `invalid_params
  {field: "lineage_id"}`; `after_mid` with `from_ordinal` is refused naming
  `from_ordinal`.
- [pinned] `include_originals: true` adds `originals` to each message a hook
  changed (§4.2).
- [pinned] The request is strict: an unknown field is refused
  `invalid_params` naming it, because a missing cursor changes the question
  (a misspelled `from_ordinal` would otherwise get the newest page).

### 4.2 The page

`{messages, lineage_id, next_from_ordinal?, head?}` (`ReadPage`).

- [pinned] Every page carries the session's `lineage_id`.
- [pinned] A page is limited by message count and by bytes, and carries
  `next_from_ordinal` whenever it stops before the end of the transcript.
  [open: Q6] the byte cap's request member (`max_bytes`), the runner's
  default, and a single message larger than the cap.
- [pinned] A page is an ordinal range and nothing more. It is not aligned to
  turns, runs or tool-call boundaries: a tool call and its result may arrive
  on different pages.
- [pinned] Each message (`ReadMessage`) carries `ordinal`, `mid` and its
  final values: what later steps render and what executed. The raw view and
  the model view then differ only by compaction.
  [open: Q7] the message body (`message`) is in the runner's schema, decoded
  as opaque JSON.
- [pinned] Ordinals and `mid`s are never reused or renumbered within a
  lineage, across any crash.
- [pinned] With `include_originals`, a hooked message carries `originals:
  [{field, value, provider}]` (`Original`), oldest first.
- [pinned] `head` is the last durable event covered by the same snapshot as
  the page (§7). [open: Q18] it is a JSON object, opaque to consumers, and
  absent on a session with no events.
- [pinned] Decoded leniently: a consumer ignores fields it does not know.
  Runners may add their own (broca's `lineage_state`, say).

### 4.3 Head metadata

- [pinned] Head metadata without bodies: `{lineage_id, last_ordinal, head,
  last_run_state, updated_at}` (`HeadMeta`).
- [open: Q5] It is its own op, `session.head`, taking no fields
  (`HeadRequest`); `last_ordinal` is absent on an empty transcript;
  `last_run_state` is `{run_id, state, reason?}` (`LastRunState`) and
  describes the most recent run, never the transcript; `updated_at` is
  milliseconds since the Unix epoch.

## 5. `dispatch_attribution`

- [pinned] The runner records the module each tool call was dispatched to.
  `session.read` exposes, per tool call, `dispatched_to` and whether the call
  is still indeterminate: it has a durable dispatch intent and no result, so
  it may or may not have run.
- [pinned] Each call's `call_key` (§11.3) is exposed with it.
- [open: Q11] They ride as `tool_calls: [{tool_call_id, call_key?,
  dispatched_to?, indeterminate}]` (`ToolCallAttribution`) beside the
  message, one entry per tool call the message carries. `dispatched_to` is
  absent for a call never dispatched (denied, say); `call_key` is absent
  until the call is prepared or dispatched.
- [pinned] `tool_call_id` is the model's id: display only, not unique.

## 6. `run_ops`: `run.result` and run attribution

- [pinned] `run.result {run_id}` (`RunResultRequest`, strict) returns the
  run's state. For a completed run it also returns the final assistant
  message, with its final values, its text parts joined in order and its
  reasoning parts excluded.
  [open: Q9] the answer is `{run_id, state, reason?, error?, final_message?:
  {ordinal, mid, text}}` (`RunResult`); a run that has not ended answers its
  current state with no final message; the join separator.
- [pinned] Each run gets exactly one terminal state, and a crash never gives
  it a second. `interrupted` (the runner stopped or crashed under the run) is
  distinct from `cancelled` (a caller stopped it).
- [open: Q9] States (`run::states`): `active`, `paused`, `completed`,
  `max_steps`, `cancelled`, `error`, `interrupted`; decoded open, and an
  unknown state is not treated as terminal.
- [pinned] A run that ends `error` because of a provider carries the
  provider's code as `provider_code` in `error`, with the provider and its
  reason. `errors::provider_codes` lists the ones this role names.
- [pinned] An unknown `run_id` is refused. [open: Q16] as `unknown_run`.
- [pinned] **Run attribution.** Each message records which run and episode
  produced it, and whether it is that run's final message.
  [open: Q10] `run: {run_id, episode, final}` (`RunAttribution`), with
  `episode` an opaque string.

## 7. `streaming`: `session.subscribe`

- [pinned] A page and its `head` come from one snapshot. A subscription from
  that `head` continues strictly after it, with no gap and no duplicate.
- [open: Q18] The request is `{from?}` (`SubscribeRequest`, strict): `"start"`,
  `"live"` (the meaning of an absent `from`) or a `head` object passed back
  verbatim. An unknown attach point is refused naming `from`.
- [open: Q18] Each event is `{kind, cursor?, ...}` (`SubscribeEvent`): `kind`
  is an open string; `control` events are durable and carry the `cursor` a
  later subscription may attach after; `display` events are best-effort and
  never replayed.

## 8. `model_view`

- [pinned] Raw is the default view. The model view is the canonical message
  list after hooks and compaction. The bytes a model provider was sent are not
  promised.
- [open: Q8] It is `session.read` with `view: "model"` (`ReadView`); the
  value is strict, so an unknown view is refused naming `view`; how
  replacement messages are numbered.

## 9. `steer` and `queue`: `session.send` from the owner

- [pinned] Prompts from the session's owner outside a human turn ride
  `session.send` with `delivery: queue | steer | interrupt`
  (`SendRequest`, `Delivery`):
  - `queue` (the default, and what an absent `delivery` means): the next turn
    after the running one;
  - `steer`: added at the next step boundary of the running turn, after any
    pending tool result, never between a call and its result. With no run
    active, a plain send;
  - `interrupt`: the message is written durably with its `send_id` first;
    then the running turn is cancelled (it ends `cancelled`); then the
    message starts. The in-flight model stream is aborted and its partial
    step discarded. A running tool call is waited on up to its deadline,
    never killed, so at-most-once holds.
- [pinned] `steer` and `queue` are each declared on their own (§3).
- [pinned] One `send_id` namespace covers all three modes, and `delivery` is
  part of the send's identity:
  - the same key with the same payload and mode is answered with the
    existing state, so a retry looks like success;
  - the same key with a different payload or mode is refused `send_id_reuse`.
    [open: Q16] `detail.field` names the field that differs.
- [pinned] The owner derives `send_id` from what it delivers, never from the
  attempt. The runner keeps it for the session's lifetime, rebuilt from
  replay.
- [pinned] The reply follows the sync: the runner answers only after the one
  record holding the message, its `send_id` and (for a steered or queued
  prompt) its PreUser output is durable. PreUser runs before the write,
  within the request's deadline. An unavailable PreUser under
  `on_unavailable: refuse` refuses the send `pre_user_unavailable` and writes
  nothing.
- [pinned] A steered prompt written before PreUser existed is never run
  through PreUser later, on resume or replay.
- [pinned] Owner-only under the session's frozen scope: any other caller is
  refused `scope_owner_mismatch`.
- [pinned] The sender's mark says the prompt is model-visible but not a human
  turn. The runner records it and does not act on it.
  [open: Q13] the member is `mark`, opaque JSON.
- [pinned] The `delivery` value is strict: an unknown mode is refused naming
  `delivery`, never read as `queue`. The rest of the request is lenient: a
  runner's own send parameters ride beside the role's members
  (`SendRequest::runner_params`). [open: Q19] which runner parameters an
  owner's steer must carry.
- [open: Q19] The reply is `{state, run_id?, submission_id?, reason?,
  baseline?}` (`SendReply`), `state` decoded open (`active`, `finished`,
  `pending`).

## 10. `session.baseline` and the admission reply

- [pinned] `session.baseline` is required of every runner and answered only
  to the session's owner; anyone else is refused `scope_owner_mismatch`. The
  request takes no fields (`BaselineRequest`, strict).
- [pinned] Before the session's first request reaches a runner that has no
  admission reply, it answers `not_yet`.
  [open: Q15] the answer is `{state: "not_yet"}` or `{state: "ready",
  baseline}` (`BaselineReply`).
- [pinned] The baseline (`Baseline`) holds:
  - the frozen per-item digests and preflight digests (`items`), and any
    optional items recorded as absent (`absent_items`);
  - the composition digest and tool-name set of the latest fold, or of the
    start record before any fold (`composition_digest`, `tool_names`): the
    frozen-prefix digest;
  - the effective baseline: the latest applied generation, whether a fold or
    an append applied it, and its manifest digest (`effective`);
  - `accepted_generation`, the highest change generation the runner
    accepted, and `applied_generation`, the highest a fold or append applied;
  - the pending change's generation and policy, when one is pending
    (`pending`);
  - the rungs the session supports, per surface, tools and system text
    (`rungs`; `rungs::ALL` lists them, cheapest first);
  - the session-level capabilities (`session_capabilities`), keyed by name,
    each declared by the value `true`: `mid_session_appends`, and
    `ordered_hook_phases` where it is session-level.
- [pinned] A send that admits a session is answered with the same facts as a
  hint (`SendReply::baseline`), read from the record the runner wrote. A
  retried first send with the same `send_id` gets the same reply. After a
  lost reply, or after a fold, the owner reads `session.baseline` instead of
  guessing. [open: Q15] the admission reply nests one `baseline` object.
- [pinned] Session-level capabilities are available before the session's
  first step and frozen for the session. `mid_session_appends` is
  re-evaluated only on a model switch, which rebuilds the prefix anyway.
- [pinned] A policy naming a rung the session does not support for that
  surface is refused when it is set. [open: Q14] the ops that set a policy.
- [open: Q15] How a baseline item is identified (`provider`, and `item` for
  a provider with more than one plan item) follows the fetch-plan section of
  this crate, still to be written.

### 10.1 Admission

- [pinned] A runner that admits sessions fetches the plan's items itself, with
  the composition, under one deadline, and admits the session only when every
  required item arrived, the staleness check passed and no tool names
  collide. The plan and composition are passed to providers verbatim.
- [pinned] Admission refuses as: `fetch_unavailable {provider}`, retryable;
  `plan_stale`, naming the differences; a tool-name collision naming both
  tools; `scope_unsupported` when the send is under a scope and the daemon
  lacks scopes. A later send whose plan differs from the frozen one is
  refused by name. [open: Q16] the collision and plan-drift codes
  (`tool_name_collision`, `plan_changed`).
- [pinned] Any refusal during admission writes nothing.
- [pinned] The runner records the session's role versions, its frozen scope
  identity, the fetched tools and text, the composition and the joined system
  text with its join version in its start record. A session keeps its role
  versions for life.
- [open: Q15] The plan travels as `plan` on the session's first send, carried
  verbatim (`SendRequest::plan`); its shape is the fetch-plan section's.

## 11. Calls the runner makes

These are required of every runner (§1). Their wire shapes belong to the
provider roles named; this role states what the runner owes.

### 11.1 The compaction interface

- [pinned] Setup is called once when a session starts. Its initial
  CompactionMessage is recorded before the first model call, and once
  recorded Setup never runs again for the session.
- [pinned] The per-step status carries the messages added since the
  provider's last call, with the ordinal cursor they start after, so a
  provider never depends on reading the runner's storage. It also carries the
  id and version of the last applied CompactionMessage, so a provider that
  lost its counter resumes above it.
- [pinned] The cursor is kept per provider per session, scoped to
  `lineage_id`, written in the same record as that call's answer, and
  advances only once the answer is durable. A crash re-sends messages; it
  never skips them. `NOOP` advances it.
- [pinned] A CompactionMessage applies only if its version is higher than
  the last applied and it answers the newest issued request (the request
  fence). Every applied CompactionMessage, `WAIT` entry and exit, and call
  timeout is durable before the request it affects is sent.
- [pinned] `REFUSE` ends the run `error` with the provider's code as
  `provider_code`, adds nothing to history, and leaves the session usable.
- [pinned] `compaction.ready {session}` (`CompactionReady`): after a `WAIT`,
  the provider signals that it is done, and the runner calls again with a
  fresh status instead of waiting out the bound. It is a hint; the bound is
  the backstop. [open: Q4] how the session is named on a module-level route.
- [pinned] The status and answers are the `compaction-provider/v1` role's
  shapes, not this crate's.

### 11.2 Hook call sites

- [pinned] Within a turn: the assistant message completes, PostAssistant
  runs and its result is written; then for each tool call PreTool (mutate,
  validate, approve, in that order where `ordered_hook_phases` holds), the
  tool executes, PostTool runs and the result is written. PreUser runs on
  every user message and every steered or queued prompt.
- [pinned] A hook's output is fields on the one record it transforms,
  written before anything carrying it is sent, applied once and never
  changed afterwards. Resume replays hook outputs and never calls a hook
  again for a record that is durable.
- [pinned] Any deny wins. A deny is the call's error result (§12.3), never a
  dispatch.
- [pinned] The hook shapes are the `step-transform-provider/v1` role's, not
  this crate's.

### 11.3 Tool calls and `call_key`

- [pinned] Every record shared between parties keys a call on `(carrier
  principal, call_key)`. A runner mints a `call_key` per call: opaque,
  bounded as `tool-provider/v1` bounds it (1 to 256 bytes, each 0x21–0x7E),
  never reused, and stable across the runner's restarts.
- [pinned] Two sibling calls from one assistant step get distinct keys, and a
  model id that recurs across steps (`call_0`) gets a distinct key each time.
- [pinned] The key is minted from the durable record that first holds the
  call alone (its prepared record for a deferred call, its dispatch intent
  otherwise), never from a record shared by sibling calls. How it is spelled
  is the runner's.
- [pinned] The runner records `tool_schema_changed`,
  `tool_semantics_changed`, `tool_unavailable` and `capability_not_admitted`
  refusals as that call's error result and never re-dispatches the call.

## 12. Error codes

Error codes ride as the `code` of an `ERROR` frame's body `{code, message,
detail?}`. Codes are open strings: a consumer that meets one it does not know
treats it as a terminal refusal of that one request.

### 12.1 Refusals (`errors`)

| Code | When | Detail | Retry |
|---|---|---|---|
| `invalid_params` | a request field is malformed, unknown, or combined with one it excludes | `field` | no |
| `lineage_changed` | a read names a lineage that is not the session's | — | no: re-read from the tail |
| `unknown_mid` | `after_mid` names no message in the lineage | — | no |
| `unknown_run` | `run.result` names no run of the session | — | no |
| `send_id_reuse` | a `send_id` reused with another payload or mode | `field` | no |
| `run_paused` | a send into a session whose last run is paused | `{run_id, reason}` | no |
| `scope_owner_mismatch` | a send or `session.baseline` from anyone but the owner | the accepted identity | no |
| `scope_adoption_refused` | a scoped send into a session recorded without a scope that cannot adopt it | `no_recorded_principal` or `principal_mismatch {recorded}` | no |
| `scope_unsupported` | a send under a scope on a daemon without scopes | — | no |
| `fetch_unavailable` | a required plan item missed the admission deadline | `provider` | **yes** |
| `plan_stale` | fetched items disagree with the composition | `differences` | no: re-plan |
| `tool_name_collision` | two tools share a model-facing name | `tools` | no |
| `plan_changed` | a later send's plan differs from the frozen one | — | no |
| `pre_user_unavailable` | a steer or queue under an unavailable `refuse` PreUser | `provider` | no |
| `transient` | a condition that may clear by itself | — | **yes** |
| `scope_not_synced` | the scope's owner has not re-synced after a daemon restart | — | **yes**, with backoff within the hold bound |

- [pinned] Only the codes marked retryable are retried with the same request
  (`errors::is_retryable`). A refusal is an answer: retrying it unchanged
  meets the same refusal.
- [pinned] Route refusals the daemon classifies as retryable on open (module
  reloading or warming, target unavailable) are retried by the caller's
  transport under the daemon's own list; this role does not restate them.
- [open: Q16] The `invalid_params` spelling (`tool-provider/v1` uses
  `invalid_request`), `detail.field` on `send_id_reuse`, and the names
  r7.3 leaves unnamed.

### 12.2 Run errors (`errors::provider_codes`)

- [pinned] `compaction_wait_exceeded`: a compaction `WAIT` hit its cap and
  the request could not be shown to fit.
- [pinned] `pre_user_unavailable`: a user turn's PreUser hook was unavailable
  under `refuse`.

### 12.3 Tool-result reasons (`errors::tool_result_reasons`)

- [pinned] `outcome_unknown`: the call had a dispatch intent and no result at
  a restart; it may or may not have run, and is never re-dispatched.
- [pinned] `pre_tool_denied`, `pre_tool_unavailable`, `pre_tool_declined`,
  `pre_tool_expired`: the call was denied before it executed. No dispatch
  intent exists.
- [pinned] `post_tool_unavailable`: the call executed and its output was
  withheld; the error text says so.
- [pinned] `route_drained`: the call was never sent, because its route closed
  under a drained scope.
- [open: Q7] Where the reason sits in a message body.

## 13. Decoding

- [pinned] Every field is declared required or optional.
- [pinned] **Lenient** (unknown fields ignored; enumerations decoded as open
  strings): `role.describe`, every reply and event a consumer decodes
  (`ReadPage`, `HeadMeta`, `RunResult`, `SubscribeEvent`, `SendReply`,
  `BaselineReply`), and `compaction.ready`.
- [pinned] **Strict on unknown fields** wherever absence changes the
  question: `session.read`, `session.head`, `run.result`,
  `session.subscribe` and `session.baseline` requests. A runner refuses an
  unknown field with `invalid_params` naming it.
- [pinned] **Lenient on fields, strict on values** where the request grows:
  `session.send` tolerates unknown fields (a newer owner against an older
  runner degrades rather than fails), but `delivery` refuses an unknown value
  naming the field. `view` on `session.read` is strict the same way.
- [pinned] A capability is declared only by being listed (`role.describe`)
  or by the value `true` (`session_capabilities`); any other value declares
  nothing.
- [pinned] A runner preserves plan items and the composition verbatim.
- [pinned] Anything a runner persists is validated strictly before it is
  persisted. A runner never refuses its own stored record for an optional
  field it does not know; a record it cannot read is refused by name as from
  a newer writer, never treated as corruption or skipped.
- [pinned] Public request and reply types with optional members are
  non-exhaustive in this crate, built with a constructor for the required
  members and `with_*` setters, so a later optional member is additive.

## 14. Crash and durability guarantees

What a runner owes across a crash, stated as properties the suite checks
(§15) by killing the runner at each point below and restarting it on the same
state root (`cortexkit-role-harness`):

1. [pinned] **Ids are never reused.** Message ids and ordinals are never
   reused or renumbered across a kill. Pages read before and after a kill
   agree on every message both contain.
2. [pinned] **A lineage change is visible.** A read naming the old lineage
   after a lineage change is refused `lineage_changed`.
3. [pinned] **One terminal per run.** Every run reaches exactly one terminal
   state. A run cut by a kill ends `interrupted`, never `cancelled`, and
   never gets a second terminal.
4. [pinned] **At-most-once dispatch.** A tool call is dispatched at most
   once. A call with a durable dispatch intent and no result is closed with
   an `outcome_unknown` result before any later model call in the session,
   is visible as indeterminate until then, and is never re-dispatched. A call
   whose step is durable and whose intent is not was never sent, and is
   dispatched on resume.
5. [pinned] **Replay, not re-invocation.** Applied compaction and every hook
   output replay from durable state without calling the provider again. A
   step whose hook outputs never became durable re-runs its hooks; nothing
   they would change was sent.
6. [pinned] **Sends are idempotent across a kill.** A send whose record was
   durable when the runner died is answered with the existing state on
   retry, and nothing is written twice.
7. [pinned] **Keys are stable.** A call's `call_key` read before a kill is
   the same after it.
8. [pinned] **Resume is recorded.** Each resume writes one informational
   record, before its first action, of what it replays, what it redoes and
   which calls it marks indeterminate. It never enters what the model is
   sent.

Kill points (`points.rs`). [open: Q20] the list.

| Point | Durable state after the kill | Property |
|---|---|---|
| `Admitted` | the start record; no model request sent | 1, 6 |
| `SendRecorded` | a send's message with its `send_id` (and PreUser output); no reply sent | 6 |
| `CompactionApplied` | a compaction answer to apply; its request not sent | 5 |
| `FoldRecorded` | a prefix rebuild applying a pending change; its request not sent | 5 |
| `StepRecorded` | an assistant step with its PostAssistant output; no dispatch intent | 4, 5 |
| `DispatchIntent` | a call's dispatch intent; the call may have been sent | 4, 7 |
| `ToolResultRecorded` | a call's result with its PostTool output | 4, 5 |
| `Terminal` | the run's terminal state | 3 |

A runner whose durable state lives in a store where truncation is not
meaningful reaches these points with a fault hook followed by a real process
kill, and declares that. A suite run fails unless at least one kill was a
real process kill.

## 15. Conformance suite (planned)

A separate `cortexkit-role-llm-runner-conformance` crate, run by each runner
in its own CI against its real module over real routes, never a double. It
drives the runner with a scripted model provider and scripted compaction and
step-transform providers. It will check:

| Case | Requires |
|---|---|
| `role_describe_shape`, `role_describe_cacheable`, `role_describe_groups_complete` | — |
| `extra_op_still_admitted` | — |
| `baseline_owner_only`, `baseline_matches_admission_reply`, `baseline_not_yet_before_first_request` | — |
| `compaction_ready_reasks_after_wait`, `compaction_cursor_never_skips` | a compaction provider in the plan |
| `tail_read_is_newest_page` (on a session longer than one page), `range_read_stops_by_count`, `range_read_stops_by_bytes`, `after_mid_reads_strictly_after`, `after_mid_unknown_mid_refused`, `read_other_lineage_refused`, `lineage_id_on_every_page`, `unknown_read_field_refused`, `head_has_no_bodies`, `include_originals` | `transcript_reads` |
| `run_result_completed_final_text`, `run_result_interrupted_not_cancelled`, `run_attribution_per_message` | `run_ops` |
| `dispatched_to_per_call`, `indeterminate_until_closed`, `sibling_calls_distinct_call_keys`, `recurring_model_id_distinct_call_keys` | `dispatch_attribution` |
| `subscribe_from_head_no_gap_no_duplicate` | `streaming` |
| `model_view_differs_only_by_compaction` | `model_view` |
| `steer_lands_at_step_boundary`, `steer_never_between_call_and_result` | `steer` |
| `queue_starts_next_turn` | `queue` |
| `send_id_retry_same_answer`, `send_id_reuse_refused_naming_field`, `delivery_change_refused`, `unknown_delivery_refused`, `pre_user_refuse_writes_nothing` | `steer` or `queue` |
| `crash_at_<point>` for each point in §14, asserting its properties | the points the harness declares |

Consumer-side rules no live runner can be made to exercise (an unknown run
state, an unknown event kind, a describe answer with a partial group) are
tested against the vectors in this crate.

Verdict, as for `tool-provider/v1`: a case whose requirements the subject
does not declare is skipped, never passed; a run with skips is "conforming
for declared capabilities", naming them; a run fails if any case fails or if
no kill ended a real process.

## Open questions

Each lists the options seen; the draft's choice is marked.

- **Q1. The request envelope.** (a) `{method, params}`, as broca's routes
  carry ops today; (b) `{name, arguments}`, as `tool-provider/v1` carries its
  role ops on the tool route. The session-bound route is common to both.
- **Q2. The `role.describe` shape.** (a) **draft:** `majors` plus top-level
  `capabilities`, as `tool-provider/v1`; (b) the flat `{role, version,
  stability, implementation_version, ops, capabilities}` r7.3 §3.3 writes;
  (c) `majors` with `capabilities` per major, since groups may differ between
  majors. The shared SDK role handle decodes this, so all roles should agree.
- **Q3. The session-capability source.** Member name and values: **draft**
  `session_capabilities_from: "admission" | "baseline"`.
- **Q4. `compaction.ready`.** r7.3 makes the compaction interface required of
  every runner, but a runner that only steers sessions it does not host
  (Thalamus) may have nothing to compact. (a) **draft:** required op; (b) a
  `compaction` group, required by any consumer that is a compaction
  provider. Also how it names its session on a module-level route: **draft**
  `{session}` as the status named it, which needs the status to name the
  session.
- **Q5. Head metadata.** (a) **draft:** its own op `session.head`; (b)
  `session.read {head_only: true}`, which mixes two reply shapes in one op.
  Also `updated_at` units (**draft** milliseconds since the epoch) and the
  `last_run_state` shape.
- **Q6. The byte cap.** The request member (**draft** `max_bytes`), the
  runner's default and maximum, and a single message larger than the cap:
  (a) a page holding only that message, over the cap; (b) refuse by name;
  never truncate a message. Also the field named by the two `after_mid`
  misuse refusals.
- **Q7. The message body.** (a) **draft:** the runner's own schema, opaque to
  the role, so a consumer that reads two runners decodes two schemas; (b) a
  neutral role-defined message schema (roles, text, tool calls, results,
  reasoning, the tool-result reason). (b) is what a context manager reading
  more than one runner needs; it is a large addition.
- **Q8. The model view.** How compaction's replacement messages are numbered
  and identified (they have no transcript ordinal), and whether range and
  `after_mid` reads apply to it.
- **Q9. `run.result`.** (a) **draft:** a run that has not ended answers its
  current state with no final message; (b) refuse it by name. Whether the
  answer also carries the final message's structured body. The separator the
  text parts are joined with. Whether `max_steps` (broca) and
  `transform_unavailable` (broca) belong in the role's state list.
- **Q10. Episodes.** What an episode is across runners and its type
  (**draft** an opaque string).
- **Q11. Dispatch attribution placement.** (a) **draft:** a per-message
  `tool_calls` list beside the message body; (b) fields inside the runner's
  message body. And whether groups that extend `session.read` require only
  that op (**draft**) or all of `transcript_reads`.
- **Q12. `interrupt`.** (a) **draft:** no group of its own; (b) its own group,
  since a runner may steer and queue without being able to abort a model
  stream. Also what a runner that declares only `steer` answers to `queue` or
  `interrupt`, and what a steer gets on a runner that can only reach a running
  session when nothing is running.
- **Q13. The mark.** Member name and shape (**draft** `mark`, opaque JSON).
- **Q14. Change ops.** `session.refresh {plan, policy, generation}`,
  `session.refresh_policy {generation, policy, send_id}` and
  `session.flush_prefix`, with their refusals (`rung_unsupported`,
  `pending_superseded`, `already_applied`): role ops under a group (which
  one?), or runner ops outside the role. r7.3 §10 does not list them.
- **Q15. The admission reply and baseline.** (a) **draft:** one nested
  `baseline` object, the same struct `session.baseline` answers; (b) three
  top-level members `baseline`, `absent_items`, `session_capabilities` on the
  admission reply. The `not_yet` spelling. Baseline item identity and the
  plan's shape wait for the fetch-plan section of this crate.
- **Q16. Unnamed codes.** r7.3 names `lineage_changed`, `unknown_mid`,
  `fetch_unavailable`, `plan_stale`, `scope_unsupported`,
  `pre_user_unavailable`, `rung_unsupported`; it leaves unnamed the
  unknown-run refusal (**draft** `unknown_run`), the tool-name collision
  (**draft** `tool_name_collision`), the plan drift on a later send
  (**draft** `plan_changed`) and the non-owner refusal (**draft**
  `scope_owner_mismatch`). Also `invalid_params` against `tool-provider/v1`'s
  `invalid_request`, and whether `send_id_reuse` names the differing field.
- **Q17. A session with no lineage yet.** What a read of a session that was
  never written answers: (a) a page with a lineage minted then; (b) refuse
  by name.
- **Q18. Subscription spellings.** The `start`/`live` names, the `head`
  object being opaque, and how much of the event shape the role pins beyond
  `kind` and `cursor`.
- **Q19. Owner sends on a runner with its own send parameters.** Which
  runner parameters (model, say) an owner's steer or queue must carry when
  the role's members are all it knows. The reply's state set.
- **Q20. Kill points.** Whether approval-gated calls' states (prepared,
  authorized, dispatch started) are cut here under the `tool-provider/v1`
  names, and whether `SendRecorded` should split admission sends from owner
  prompts.
- **Q21. Stability guarantees.** What `alpha`, `beta` and `stable` each
  promise; r7.3 says the role document states them but not what they are.

## Gaps in broca today

Where broca's shipped surface differs from this document. Paths are in the
broca repository; none of this is a change request against broca beyond what
its own build plan already schedules.

1. **No `role.describe`.** The served ops are `cap.install`, `spend.delta`,
   `session.send`, `session.import`, `session.retract`, `run.cancel`,
   `run.status`, `session.read` and the `session.subscribe` stream
   (`crates/broca-module-serve/src/serve.rs:1514-1836`); any other op gets
   `unknown_method` (`crates/broca-module-serve/src/serve.rs:1837-1848`). Until `role.describe` ships, the manifest claims
   `session-send-delivery/v1` instead. Planned in B9
   (`docs/extensibility-build-plan.md:358-367`).
2. **No `session.baseline`, and the admission reply carries no baseline.**
   `SendResult` is `active {run_id} | finished {run_id, reason} | pending
   {submission_id}` (`crates/broca-wire/src/lib.rs:510-523`). Planned in B8
   (`docs/extensibility-build-plan.md:347-356`).
3. **No `compaction.ready` and no `model_view`.** Planned in B14 as inbound
   `compaction.ready(session)` and `session.read {view: model}`
   (`docs/extensibility-build-plan.md:434-443`).
4. **`session.read` takes only `from_ordinal`, `limit` and `include_tools`**
   (`crates/broca-wire/src/lib.rs:464-476`): no `after_mid`, `lineage_id`,
   `max_bytes`, `include_originals` or `view`, so no `lineage_changed` or
   `unknown_mid` refusal. The request is already strict on unknown fields,
   as §13 requires. `include_tools` is a runner extra the role does not
   define.
5. **Pages are capped by count only:** default 200, maximum 500
   (`crates/broca-core/src/context.rs:30-34`), with no byte cap.
   `next_from_ordinal` is already set whenever a page stops early.
6. **`lineage_id` is optional on a page,** absent on legacy lineages until
   their next write (`crates/broca-wire/src/lib.rs:563-566,574-575`). The
   role requires it on every page.
7. **Messages carry only `{ordinal, mid, message}`**
   (`crates/broca-wire/src/lib.rs:525-532`): no run attribution, no
   `originals`, no per-call `dispatched_to`, `call_key` or indeterminate
   flag. Dispatch attribution and `call_key` are planned in B9, originals in
   B10, run attribution in B12 (`docs/extensibility-build-plan.md:358-367,
   377-386, 410-419`).
8. **No dispatch target is recorded:** `ToolDispatchIntent` holds
   `batch_id`, `tool_call_id`, `tool_name` and `args`, no module
   (`crates/broca-wal/src/record.rs:295-304`), and no `call_key` is minted.
   Planned in B9.
9. **`mid` is derived from the ordinal,** `m<ordinal>`
   (`crates/broca-module/src/serve.rs:174`). That meets "never reused within
   a lineage" and is opaque enough, but the same `mid` exists in every
   lineage, so a cursor is only safe with its `lineage_id`, as §4.1 requires.
10. **No `session.head`.** The nearest thing is `lineage_state` on every page
    (`crates/broca-wire/src/lib.rs:546-559`), which describes the last run
    but needs a page with bodies to reach.
11. **No `run.result`.** `run.status` answers a run's state without its final
    message, and its states include `transform_unavailable` and `unknown`
    beside the role's (`crates/broca-wire/src/lib.rs:669-743`).
12. **`session.subscribe` is lenient on unknown fields**
    (`crates/broca-wire/src/lib.rs:420-424`): a misspelled `from` attaches
    live. The role makes the request strict. The `head` and cursor shape
    `{wal_seq, sub_index}` (`crates/broca-wire/src/lib.rs:409-414`) and the
    event shape (`crates/broca-wire/src/lib.rs:756-766`) already fit §7.
13. **`send_id` is optional and `model` is required on every send**
    (`crates/broca-wire/src/lib.rs:112-115`). The role requires `send_id` on
    owner sends, and an owner's steer that carries only the role's members
    would be refused for want of `model` (Q19). There is no `mark` and no
    `plan` member yet (B8, B12). The `delivery` modes and their strict value
    already match §9 (`crates/broca-wire/src/lib.rs:177-190`).
14. **`send_id_reuse` and `invalid_params` carry no `detail.field`:** the
    error body is built from the code and message alone
    (`crates/broca-module-serve/src/serve.rs:2078,2102,2123-2141`).
15. **Separate steer and queue ops in the build plan.** B12 still names new
    ops `session.steer` and `session.queue`
    (`docs/extensibility-build-plan.md:417`); the role uses `session.send`
    with `delivery` instead.
