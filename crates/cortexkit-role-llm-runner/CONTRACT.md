# `llm-runner/v1` — role contract (draft)

Stability: **alpha, draft**. Approved by the role's owner with the
decisions recorded under "Open questions", every one of which is now
settled; nothing here ships until the design room signs off. This document is the role definition; the
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
- **[open]** (no question number): not yet defined, because it depends on a
  section this crate has not written yet. Nothing may build against it.
- **[provisional]**: settled in substance, but its exact name or position
  depends on an unwritten section and may still change. Do not build
  against the spelling yet.

A question the role's owner has settled keeps its number and is marked
"settled" there; the items it governs are marked **[pinned]** here. In this
revision every numbered question is settled except Q22, the parts of the
`run.status` answer one runner serves and the role has not chosen (§6.1).
One item waits on the fetch-plan section (§10): the `session_change` request
and reply types (§10.2, open).

An LLM runner is any module that runs model sessions. Nothing here names a
particular implementation; "Gaps in broca today" at the end compares the one
shipping runner with this document.

## 1. Role identity and addressing

- [pinned] A runner lists `llm-runner/v1` in its manifest's
  `capabilities.provides` (`PROVIDES`). A module may serve several majors.
- [pinned] Required ops (`REQUIRED_OPS`): `role.describe` and
  `session.baseline`. `compaction.ready` is not required: it belongs to the
  `compaction` group (§3, §11.1).
- [pinned] `session.baseline` is required of every runner whether or not a
  given consumer calls it: it is where a session's owner reads what was
  frozen and the session-level capabilities after a lost admission reply
  or a fold (§10).
- [pinned] The hook call sites and the tool-call rules (§11.2, §11.3) are
  required too. They are calls the runner makes, not ops it serves, so
  `role.describe` does not list them. The compaction interface (§11.1) is
  not required: it is the `compaction` group.
- [pinned] Everything else is a declared capability group (§3), including
  the operations not every runner serves: `run.status`, `run.cancel` and
  `session.retract`. A runner serves a group only if it declares it, and a
  consumer uses a group only if it is declared.
- [pinned] A consumer refuses a module whose `role.describe` lacks a required
  op, by name, before routing anything to it. The check lives in the
  consumer, never in the daemon.
- [pinned] Every role op is a request `{method, params}` on the runner's
  session-bound management route: `method` is the op's name and `params` its
  request. The session it acts on is the session the route is bound to; no
  request names its session, except `compaction.ready`, which arrives on the
  compaction provider's module-level route (§11.1).

## 2. `role.describe`

- [pinned] The answer is `{majors: [{version, ops, stability}],
  implementation_version, capabilities, session_capabilities_from,
  max_bytes?, steer_receipt?, retention?}` (`RoleDescribe`), the shape `tool-provider/v1`
  uses. `version` is spelled as in the manifest, `llm-runner/v1`.
- [pinned] `capabilities` lists the module-level capabilities: the groups of
  §3 the runner declares, and `ordered_hook_phases` on a runner where it holds
  for every session.
- [pinned] The answer says where session-level capabilities come from (§10):
  the admission reply, on a runner that admits sessions, or
  `session.baseline`, on a runner without an admission reply.
  [pinned] The member is `session_capabilities_from`, valued `admission` or
  `baseline`.
- [pinned] A runner that declares `transcript_reads` must state its
  `session.read` byte cap as `max_bytes: {default, maximum}` (`MaxBytes`),
  in bytes: the cap a page stops at when the request names none, and the
  largest cap it honours. A consumer refuses a describe answer that
  declares `transcript_reads` without it (`check_describe`:
  `missing_max_bytes`).
- [pinned] `steer_receipt` is optional, valued `guaranteed` or `confirm`, with
  absent meaning `guaranteed`:
  - on a `guaranteed` runner, a durably accepted steer is delivered:
    `delivered` in `SendReply` may be absent, and must never be `pending` or
    `unknown`;
  - on a `confirm` runner, an absent `delivered` in `SendReply` means `pending`,
    never delivered.
- [pinned] Decoded leniently: unknown fields, majors, ops and capabilities
  are ignored, and an unknown `stability` decodes. Only what a consumer
  relies on is checked strictly (`check_describe`): the `llm-runner/v1`
  major, its required ops, every op of each declared group, the
  session-capability source, and the byte cap when `transcript_reads` is
  declared.
- [pinned] The answer describes the runner build, never a session. A consumer
  may cache it for as long as it talks to the same module incarnation.
- [pinned] Stability is data, never part of a capability name. Each major
  carries `stability`, one of `alpha`, `beta` or `stable`, for the major it
  describes; a major that omits it is read as `alpha`. This draft is
  `alpha`. A role becomes stable only when a second, independent
  implementation passes its suite.

## 3. Capability groups

- [pinned] Each group is all or nothing: a runner that declares one serves
  every op in it. A declared group missing one of its ops makes
  `check_describe` refuse the answer (`group_incomplete`), because a consumer
  cannot use part of a group.

| Group | Ops (`capabilities::GROUPS`) | Section |
|---|---|---|
| `transcript_reads` | `session.read`, `session.head` | §4 |
| `run_ops` | `run.result`, `session.read` (attribution) | §6 |
| `run_status` | `run.status` | §6.1 |
| `run_cancel` | `run.cancel` | §6.2 |
| `dispatch_attribution` | `session.read` (per-call fields) | §5 |
| `streaming` | `session.read` (`head`), `session.subscribe` | §7 |
| `model_view` | `session.read` (`view: "model"`) | §8 |
| `steer` | `session.send` with `delivery: "steer"` | §9 |
| `queue` | `session.send` with `delivery: "queue"` | §9 |
| `interrupt` | `session.send` with `delivery: "interrupt"` | §9 |
| `plans` | `session.send` with `plan` (baseline via required `session.baseline`) | §10.1 |
| `compaction` | `compaction.ready`, and the calls of §11.1 the runner makes | §11.1 |
| `session_change` | `session.refresh`, `session.refresh_policy`, `session.flush_prefix` | §10.2 |
| `retention` | `session.send` with `retention` | §9.1 |
| `retract` | `session.retract` | §9.2 |

- [pinned] A group that only adds fields to `session.read` requires
  `session.read`, not all of `transcript_reads`.
- [pinned] `run_status`, `run_cancel` and `retract` are each declared on
  their own: a runner may serve any of them without the others, and
  without `run_ops` or `transcript_reads`. This crate does not yet list
  them in `capabilities::GROUPS` or carry their op names and types, so
  `check_describe` does not check their ops; until it does, a consumer
  checks that the major lists the op (`Major::serves`) as well as that the
  group is declared (`RoleDescribe::declares`).
- [pinned] `interrupt` is a group of its own: a runner may steer and queue
  without being able to abort a model stream.
- [pinned] `compaction` is all of §11.1 or none of it: Setup, the per-step
  status with its cursor rules, the request fence, durable `WAIT` and its
  timeout, `REFUSE`, and the inbound `compaction.ready`. A runner that hosts
  no sessions of its own (one that only steers sessions another runner
  hosts) declares none of it, never a stub.
- [pinned] A compaction provider requires `compaction` of any runner it
  serves. A session starter never pairs a plan's `compaction_item` with a
  runner that lacks it, and the runner refuses such a plan at admission
  (§10.1) as `invalid_params {field: "plan.compaction_item"}`
  (`SendRequest::check_compaction_item`), so a session never runs with a
  compaction owner nothing calls.
- [pinned] A session owner sends a mid-session change only to a runner that
  declares `session_change`. A runner without it gets no mid-session
  changes; its owner applies them at the next session instead.
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
- [pinned] `after_mid` without `lineage_id` is refused `invalid_params
  {field: "lineage_id"}`; `after_mid` with `from_ordinal` is refused naming
  `from_ordinal`; `after_mid` with `view: "model"` is refused naming `view`
  (§8).
- [pinned] `max_bytes` asks for a byte cap on the page. Absent means the
  runner's default; a value above the runner's maximum is capped to it. Both
  are stated in `role.describe` (§2).
- [pinned] `include_originals: true` adds `originals` to each message a hook
  changed (§4.2).
- [pinned] The request is strict: an unknown field is refused
  `invalid_params` naming it, because a missing cursor changes the question
  (a misspelled `from_ordinal` would otherwise get the newest page).

### 4.2 The page

`{messages, lineage_id, next_from_ordinal?, head?}` (`ReadPage`).

- [pinned] Every page of a session that has a lineage carries `lineage_id`.
  A session that was never written has no lineage yet: a read of it returns
  an empty page with no `lineage_id` and no `next_from_ordinal`, and an
  absent `lineage_id` means exactly that.
- [pinned] A page is limited by message count and by bytes, and carries
  `next_from_ordinal` whenever it stops before the end of the transcript.
- [pinned] A single message larger than the byte cap comes alone on its
  page, over the cap, with `next_from_ordinal` set if more follow. A message
  is never truncated, and its size is never a reason to refuse the read.
- [pinned] A page is an ordinal range and nothing more. It is not aligned to
  turns, runs or tool-call boundaries: a tool call and its result may arrive
  on different pages.
- [pinned] Each message (`ReadMessage`) carries `ordinal`, `mid` and its
  final values: what later steps render and what executed. The raw view and
  the model view then differ only by compaction.
- [pinned] The message body (`message`) is in the runner's own schema,
  opaque to the role and decoded as JSON. A neutral message schema is not in
  v1 (see "Not in v1").
- [pinned] Ordinals and `mid`s are never reused or renumbered within a
  lineage, across any crash.
- [pinned] With `include_originals`, a hooked message carries `originals:
  [{field, value, provider}]` (`Original`), oldest first.
- [pinned] `head` is the last durable event covered by the same snapshot as
  the page (§7). It is a JSON object, opaque to consumers, and absent on a
  session with no events.
- [pinned] Decoded leniently: a consumer ignores fields it does not know.
  Runners may add their own (broca's `lineage_state`, say).

### 4.3 Head metadata

- [pinned] Head metadata without bodies: `{lineage_id, last_ordinal, head,
  last_run_state, updated_at}` (`HeadMeta`).
- [pinned] It is its own op, `session.head`, taking no fields
  (`HeadRequest`); `last_ordinal` is absent on an empty transcript;
  `last_run_state` is `{run_id, state, reason?}` (`LastRunState`) and
  describes the most recent run, never the transcript; `updated_at` is
  milliseconds since the Unix epoch.
- [pinned] It follows the read rule (§4.2): an absent `lineage_id` means the
  session has no lineage yet. For a session never written, `lineage_id`,
  `last_ordinal`, `head`, `last_run_state` and `updated_at` are all absent.
- [pinned] An answer without `lineage_id` carries none of the other four
  members, and an answer with `lineage_id` carries `updated_at`
  (`HeadMeta::lineage_consistent`). A consumer treats any other answer as
  malformed; it is not a refusal.

## 5. `dispatch_attribution`

- [pinned] The runner records the module each tool call was dispatched to.
  `session.read` exposes, per tool call, `dispatched_to` and whether the call
  is still indeterminate: it has a durable dispatch intent and no result, so
  it may or may not have run.
- [pinned] Each call's `call_key` (§11.3) is exposed with it.
- [pinned] They ride as `tool_calls: [{tool_call_id, call_key?,
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
  [pinned] The answer is `{run_id, state, reason?, error?, final_message?:
  {ordinal, mid, text}}` (`RunResult`), and a run that has not ended answers
  its current state with no final message. [pinned] The separator is the
  empty string (`run::TEXT_SEPARATOR`, `run::join_text_parts`): text parts
  are concatenated exactly as emitted, with nothing inserted, so a JSON
  value a provider split across parts reads back whole.
- [pinned] A completed run whose final message has no text parts (reasoning
  only, or a provider's safety halt) answers `final_message` with `text:
  ""`, never an absent `final_message`. An absent `final_message` then means
  only that the run has not ended, so a consumer tells "completed with no
  text" (which it reads through `session.read` for the reasoning) from "no
  final message yet" (which it waits on). An answer that breaks this (a
  `completed` run without `final_message`, or an `active` or `paused` one
  with it) is malformed, not a refusal (`RunResult::final_message_consistent`).
- [pinned] Each run gets exactly one terminal state, and a crash never gives
  it a second. `interrupted` (the runner stopped or crashed under the run) is
  distinct from `cancelled` (a caller stopped it).
- [pinned] States (`run::states`): `active` and `paused`, which are not
  terminal, and the terminal states `completed`, `max_steps`, `cancelled`,
  `error` and `interrupted`. States are decoded open: a runner's own state
  (broca's `transform_unavailable`, say) decodes as a plain string, and a
  consumer does not treat an unknown state as terminal.
- [pinned] A run that ends `error` because of a provider carries, in
  `error` (`RunResult::error`, whose other members are the runner's
  schema), `provider_code`, the provider that ended it as `provider`, and
  its `reason`. `provider_code` is the code callers branch on.
  `errors::provider_codes` lists the ones this role names (§12.2).
- [pinned] For a compaction `REFUSE`, `provider_code` is the refusal's role
  `code`: one of the compaction-provider contract's `RefuseCode` values
  (its §11), or an unknown code recorded as received. Whether a caller
  retries follows from that code, as the compaction-provider contract fixes
  it, never from anything else in the error.
- [pinned] The refusal's optional finer code goes in the separate, optional
  `provider_detail_code`: diagnostics only, for logs and display. It never
  replaces `provider_code`, a caller never branches on it, and it never
  decides retry. The compaction-provider wire calls that finer code
  `provider_code`; on a run error it is `provider_detail_code`. The two
  `provider_code` members are different fields: the runner's carries the
  role code, the provider's carries the finer one.
- [pinned] A run error without a finer code omits `provider_detail_code`.
  `compaction_unavailable` never carries one, because no answer arrived.
- [pinned] An unknown `run_id` is refused `unknown_run`.
- [pinned] **Run attribution.** Each message records which run and episode
  produced it, and whether it is that run's final message.
  [pinned] It rides as `run: {run_id, episode, final}` (`RunAttribution`),
  with `episode` an opaque string.
  [pinned] `final` means the last message this run produced, whatever its
  role: an `interrupted` run that ended on a tool result marks that tool
  result `final: true`. A run whose terminal is not yet durable has no final
  message, so its newest message reads `final: false`.
  [pinned] A message the runner cannot attribute omits `run`. A runner never
  invents an attribution, and an absent `run` never means "the last run".

### 6.1 `run_status`: `run.status`

A runner that declares `run_status` serves `run.status`: a run's current
state, polled without reading the transcript.

- [pinned] The request is `{run_id}`, strict: an unknown field is refused
  `invalid_params` naming it, because a misspelled `run_id` would otherwise
  ask a different question (§13).
- [pinned] The answer is `{state, reason?, error?, undelivered_steers?}`,
  decoded leniently: a runner's own members ride beside these (broca's
  `usage`, say).
  - `state` is a run state as in `run.result` (`run::states`), decoded open
    (`RunState::parse`): a consumer does not treat an unknown state as
    terminal.
  - `reason`, where the runner names one: why a paused run is paused, or
    why an ended run ended.
  - `error`: on a run that ended `error`, the same run error `run.result`
    answers (§6), with the same `provider_code` and `provider_detail_code`
    rules.
- [pinned] `undelivered_steers` is the list of `send_id`s of the steers
  this run accepted and never rendered to the model before it ended, in
  the order they were accepted. On the status of a run that has ended:
  - it is present whenever the runner could determine delivery, and is
    `[]` when the runner checked and every steer was rendered, or none was
    accepted;
  - it is absent only when the runner could not determine delivery (a run
    whose records predate the runner tracking it, say). A consumer reads an
    absent list as "not known", never as "none".
  On a run that has not ended it is absent, and its absence says nothing.
  The list describes this run only: a steer it names may still be
  delivered by a later turn, and the send's `delivered` receipt (§9) is
  what reports that.
- [pinned] On an expired session, `run.status` answers `expired` (§9.1).
- [open: Q22] An unknown `run_id` is refused `unknown_run`, as `run.result`
  refuses it. A request without `run_id` asks no question this role
  defines: a runner may answer it with a session-level status of its own,
  which a consumer reads only through that runner's schema.

### 6.2 `run_cancel`: `run.cancel`

A runner that declares `run_cancel` serves `run.cancel`: a caller stops a
run.

- [pinned] The request is `{run_id}`. The answer is a JSON string, decoded
  open: `ack` or `not_active`.
  - `ack`: the runner accepted the cancellation. The run still reaches
    exactly one terminal state (§6): `cancelled`, or the terminal state it
    reached first. `ack` does not say which; a caller reads it through
    `run.result` or `run.status`.
  - `not_active`: the runner did not cancel the run, because it has
    already ended or the session has no such run. Nothing is written.
  A consumer treats any other answer as not confirming a cancellation.
- [pinned] A run a caller cancels ends `cancelled`, never `interrupted`
  (§6).
- [pinned] On an expired session, `run.cancel` answers `expired`, writes
  nothing, and never acts on a lineage started after the expiry (§9.1).

## 7. `streaming`: `session.subscribe`

- [pinned] A page and its `head` come from one snapshot. A subscription from
  that `head` continues strictly after it, with no gap and no duplicate.
- [pinned] The request is `{from?}` (`SubscribeRequest`, strict): `"start"`,
  `"live"` (the meaning of an absent `from`) or a `head` object passed back
  verbatim. An unknown attach point is refused naming `from`.
- [pinned] Each event is `{kind, cursor?, ...}` (`SubscribeEvent`): `kind`
  is an open string; `control` events are durable and carry the `cursor` a
  later subscription may attach after; `display` events are best-effort and
  never replayed.

## 8. `model_view`

- [pinned] Raw is the default view. The model view is the canonical message
  list after hooks and compaction. The bytes a model provider was sent are not
  promised.
- [pinned] It is `session.read` with `view: "model"` (`ReadView`), answered
  as `{messages, lineage_id?, next_from_ordinal?, head?, compaction_id?,
  version?}` (`ModelPage`). The value is strict, so an unknown view is
  refused naming `view`. `lineage_id`, `next_from_ordinal` and `head` mean
  what they mean on a raw page (§4.2).
- [pinned] Each entry is `{source, message}` (`ModelEntry`), `message` in the
  runner's own schema. `source` (`EntrySource`) says what the entry is:
  - `{kind: "message", ordinal, mid}`: a transcript message passed through,
    with its hooks applied, keeping its transcript ordinal;
  - `{kind: "replacement", compaction_id, version, from_ordinal,
    to_ordinal}`: a compaction replacement, naming the half-open raw range
    `[from_ordinal, to_ordinal)` it stands for and the CompactionMessage
    it came from. Only `to_ordinal < from_ordinal` is an invalid range.
    An empty range inserts before `from_ordinal`; Setup's head message is
    `[0, 0)`.
- [pinned] Entries are ordered non-decreasingly by `from_ordinal` (a message's
  `ordinal` is its anchor). At an equal anchor, insertions come first: an
  insertion following a message or a non-empty replacement at its anchor
  is malformed (`ModelPageProblem::InsertionAfterEntry`).
  - An insertion before a message has a v1 producer: an empty compaction
    range, such as Setup's head message `[0, 0)` before message 0.
  - An insertion before a non-empty replacement at the same anchor has no
    v1 producer. The compaction-provider contract applies the newest
    CompactionMessage alone (its §8) and gives all of that message's
    replacement entries its one range (its §12), and hook outputs transform
    fields on a raw record (§11.2), not additional range entries. A
    consumer still refuses such a page as malformed (`ModelPage::check`);
    no live runner can be made to exercise this half (§15).
- [pinned] A runner applies at most one view per step (§11.1). A second
  CompactionMessage answered for a step whose view already applied never
  changes the model view, whatever its version.
- [pinned] Tail and range reads apply, keyed on transcript ordinals. A
  replacement is returned whole, exactly once, on the page holding its
  `from_ordinal`: it is never split, and a later page that intersects its
  range does not repeat it.
- [pinned] An insertion at anchor A travels on the same page as the entry
  anchored at A: the message holding ordinal A or a replacement whose
  `from_ordinal` is A. Several insertions at one anchor travel together, in
  order. The page may exceed its count or byte cap to keep this group whole,
  like the oversize single-message rule; it never ends between an insertion
  and its anchored entry.
- [pinned] That page's `next_from_ordinal` is past the anchored entry: A + 1
  for a message, or the replacement's exclusive `to_ordinal`. An insertion
  past the last ordinal goes on the last page, with no cursor.
- [pinned] A page ending on an insertion with `next_from_ordinal` present is
  malformed (`ModelPageProblem::NonTailInsertion`). A next cursor on or
  before an insertion anchor whose anchored entry was not returned is also
  malformed (`ModelPageProblem::NextInsideEntry`).
- [pinned] `after_mid` with `view: "model"` is refused `invalid_params
  {field: "view"}` in v1. `after_mid` is defined against the raw lineage,
  and a message inside a replaced range has no clean "after" in the model
  view; incremental readers use ordinal cursors.
- [pinned] The page names the compaction state it reflects: the
  `compaction_id` and `version` of the latest applied CompactionMessage,
  both absent before any compaction applied. A page carrying a replacement
  names it. The view is what the next request would be built from, not the
  bytes sent.
- [pinned] A page that breaks these rules is malformed, not a refusal
  (`ModelPage::check`). The `source` kind is strict: an entry of a kind this
  role does not define does not decode, because a consumer cannot place it
  by ordinal.

## 9. `steer`, `queue` and `interrupt`: `session.send` from the owner

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
- [pinned] `steer`, `queue` and `interrupt` are each declared on their own
  (§3). An absent `delivery` means `queue`, and `queue` is a declared group
  like the others. A runner refuses a mode it does not declare as
  `delivery_unsupported`, with `detail.delivery` naming the mode, and writes
  nothing. It never delivers the send in another mode instead
  (`SendRequest::check_delivery`).
- [pinned] A runner that admits sessions declares `queue`, because its first
  send is one.
- [pinned] One `send_id` namespace covers all three modes, and `delivery` is
  part of the send's identity:
  - the same key with the same payload and mode is answered with the
    existing state, so a retry looks like success;
  - the same key with a different payload or mode is refused `send_id_reuse`,
    with `detail.field` naming the field that differs.
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
  [pinned] The member is `mark`, opaque JSON.
- [pinned] The `delivery` value is strict: an unknown mode is refused naming
  `delivery`, never read as `queue`. The rest of the request is lenient: a
  runner's own send parameters ride beside the role's members
  (`SendRequest::runner_params`).
- [pinned] A `steer` or `interrupt` into an existing session takes every
  parameter the runner freezes for the session, when the send omits it,
  from the frozen value, so an owner that knows only the role's members can
  send one. A runner that freezes none has nothing to inherit.
- [pinned] A runner never accepts a send and delivers it under settings
  other than the ones the send named. A runner parameter the runner cannot
  honour for that delivery (a steer naming a model or generation settings
  other than those of the run it joins, say) is refused `invalid_params`
  naming that parameter, and nothing is written.
- [pinned] The reply is `{state, run_id?, submission_id?, reason?,
  baseline?, delivered?}` (`SendReply`), `state` decoded open (`active`,
  `finished`, `pending`).
- [pinned] `delivered` (`Delivered`) is optional: `{as, ref?}`:
  - `as` is one of `step | turn | pending | unknown`, decoded open: an
    unknown string survives as itself and does not fail the decode;
  - `ref` is an opaque string: a stored row id, or the run id when `as` is
    `turn`;
  - it is set on a re-send of the same `send_id` once the steer is delivered
    or known `unknown`, and may be set on the first reply; on a `guaranteed`
    runner it may be absent until the steer is rendered;
  - `step` / `turn`: delivered;
  - `pending`: accepted, not yet delivered, still deliverable;
  - `unknown`: accepted, but delivery cannot be confirmed; the owner records
    `outcome_unknown` and never re-sends;
  - on a `guaranteed` runner (`role.describe.steer_receipt` absent or
    `guaranteed`), a durably accepted steer is delivered: `delivered` may be
    absent, and must never be `pending` or `unknown`;
  - on a `confirm` runner (`role.describe.steer_receipt: confirm`), an absent
    `delivered` means `pending`, never delivered;
  - the owner re-checks a `confirm` runner's undelivered steers by re-sending
    the same `send_id`, only on runner reconnect and on its next send to the
    same session.
- [pinned] A re-send's `delivered` moves forward only: from absent or
  `pending` to `step`, `turn` or `unknown`. Those three are final: once one
  first appears, every later answer to the same `send_id` carries it
  unchanged, `ref` included. Absent to `pending` is not a move on a
  `confirm` runner, where absent already means `pending`; a `guaranteed`
  runner never answers `pending` at all.

### 9.1 `retention`: whole-session transcript lifetime

A runner declaring the `retention` capability group must implement all the
rules in this section. These rules govern the entire session lineage, not
individual runs: the transcript is append-only and each run's prompt replays
earlier runs. Deleting individual runs would break that replay.

**Discovery and sends.** `role.describe` lists `retention` in `capabilities`
and carries `retention: {max_seconds: u64, delete_within_ms: u64}`
(`Retention`). `max_seconds` must be positive; `delete_within_ms` is the
maximum delay from expiry to complete deletion of runner-held content.
`check_describe` refuses a declared group without limits (`missing_retention`)
or with a zero maximum (`invalid_retention`).

`session.send` accepts optional `retention: u64`, in seconds
(`SendRequest::retention`). The first accepted send freezes it for the whole
session. Absence on that first send means kept forever. On an existing
lineage, a later send may shorten the current value; an equal value leaves
the policy unchanged. A longer value, or any later retention on a lineage
first sent without retention, is refused `invalid_params {field: "retention"}`.
Absence on a later send leaves the current policy unchanged, never resets it
to forever. Zero is refused by the same name. A value above `max_seconds` is
refused `invalid_params` with `detail: {field: "retention", max_seconds}`
(`RetentionInvalidDetail`). A runner without the group refuses any retention
as `invalid_params {field: "retention"}`, never ignores it. Refusals write
nothing. These checks apply to every delivery mode.

**Expiry and activity.** Expiry is wall-clock last activity plus the current
retention. Last activity is the time of the newest durable send, step or
terminal record; reads do not extend it. Records a runner writes on its own
initiative, such as cache-warm records, are not activity either, so a
keep-warm loop never keeps an idle session past its retention. `expired_at_ms` is that expiry time
in milliseconds since the Unix epoch, not the time deletion finishes. A
session with any non-terminal run never expires, even when it has no new
records for longer than its retention. In particular, a paused run (including
an authentication or restart pause) is non-terminal. Long tool calls and held
steers do not expire either. The clock restarts from the terminal record when
the last non-terminal run ends.

An accepted send that shortens an idle, unexpired session continues the same
lineage, preserving earlier messages. It records new activity, and the
shorter clock applies from that activity; its active run then defers expiry
as above. Whether the session was already expired is decided using its old
retention, never the shorter value supplied by the new send. A session past
its old retention is expired even if deletion has not finished yet. A send to
that session starts a fresh lineage, with that send's own retention (absent
means forever), never replays the expired content and never reuses its lineage
id. Deletion of the old lineage must not delete the new one.

**Deletion.** At expiry the runner durably records a content-free tombstone
first, then deletes the transcript and every runner-held derived copy,
including projections, snapshots and archives, within `delete_within_ms`.
It must finish an interrupted deletion on restart. The tombstone survives
deletion so expiry remains distinguishable from a session never written.

Only content-free accounting and identity metadata may remain: identifiers
of the session, runs, provider, model, credential and account; timestamps;
token counts and cost; charge and billing classification; terminal status
and error codes. None may contain message, system or tool text, tool arguments
or results, or caller-supplied free text such as a run `title`. Copies held
by other modules, including a compaction provider's own state and backups,
are their owners' responsibility. This contract covers runner-held copies only.

**Reads.** `session.read` (including model view), `session.head` and
`run.result` on an expired session answer an `ERROR` with code `expired` and
`detail: {expired_at_ms}` (`ExpiredDetail`). A runner that declares
`run_status` answers `run.status` with the same refusal. A runner that
declares `run_cancel` or `retract` also answers `expired` to `run.cancel`
or `session.retract` for an expired session and writes nothing; neither
may act on a fresh lineage started after the expiry. The conformance suite
probes `run.status` only. `expired`
is terminal and never retried. It is not an empty page: that would falsely
say the session has no messages, rather than that its messages expired.
A session never written keeps the existing empty success answers. No read,
export or listing may serve expired content, including free-text titles,
even when retained accounting or identity metadata is served.

### 9.2 `retract`: `session.retract`

A runner that declares `retract` serves `session.retract`: a send that has
not started is withdrawn before it starts. That is a queued send and, on a
`confirm` runner, a steer whose receipt is `pending`. A `pending` send reply
carries `submission_id` exactly when the runner declares `retract`. On a
`guaranteed` runner an accepted steer is already durable in the run it
joined, and `session.retract` does not withdraw it.

- [pinned] The request is `{submission_id}`, the id a `pending` send reply
  named (§9). The answer is a JSON string, decoded open:
  - `retracted`: the submission is durably withdrawn and never starts;
  - `already_started`: it started a run, which `session.retract` does not
    stop (on a runner that declares `run_cancel`, `run.cancel` does);
  - `not_pending`: the session has no pending submission by that id: it
    was already retracted, or never existed. Nothing is written.
  A consumer treats any other answer as not confirming a retraction.
- [pinned] A submission is either retracted or started, never both: the
  runner decides from one durable order, and its answer says which.
- [pinned] A re-send of a retracted send, with the same `send_id` and
  payload, is refused `send_retracted` and writes nothing: it never starts
  the send again. A different payload under that `send_id` is refused
  `send_id_reuse` as in §9.
- [pinned] On an expired session, `session.retract` answers `expired` and
  writes nothing (§9.1).

## 10. `session.baseline` and the admission reply

- [pinned] `session.baseline` is required of every runner and answered only
  to the session's owner; anyone else is refused `scope_owner_mismatch`. The
  request takes no fields (`BaselineRequest`, strict).
- [pinned] Before the session's first request reaches a runner that has no
  admission reply, it answers `not_yet`.
  [pinned] The answer is `{state: "not_yet"}` or `{state: "ready",
  baseline}` (`BaselineReply`).
- [pinned] The baseline (`Baseline`) holds:
  - the frozen per-item digests and preflight digests (`items`), and any
    optional items recorded as absent (`absent_items`);
  - the composition digest and tool-name set of the latest fold, or of the
    start record before any fold (`composition_digest`, `tool_names`): the
    frozen-prefix digest. `composition_digest` is absent on a session
    admitted without a fetch plan and present whenever the session was
    admitted with one. A plan-less session also has `items: []` and no
    `absent_items` (`Baseline::plan_consistent`);
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
  guessing. [pinned] The admission reply nests one `baseline` object, the
  same struct `session.baseline` answers.
- [pinned] Session-level capabilities are available before the session's
  first step and frozen for the session, with one exception:
  `mid_session_appends` depends on the session's model
  (`capabilities::session::MID_SESSION_APPENDS`), so a runner whose
  sessions can switch model re-evaluates it on a switch, which rebuilds
  the prefix anyway, and at no other time. After a switch the owner reads
  the current value through `session.baseline`.
- [pinned] A policy naming a rung the session does not support for that
  surface is refused `rung_unsupported` when it is set. [pinned] The ops
  that set a policy are `session.refresh` and `session.refresh_policy`, in
  the `session_change` group (§10.2).
- [pinned] A baseline item is identified by `provider`, and by `item` for a
  provider with more than one plan item. The `item` values follow the
  fetch-plan section of this crate, still to be written.

- [pinned] An absent optional item (`AbsentItem`) is `{provider, kind,
  reason, provider_code?}`. `kind` is an open string; `system_text` is the
  only kind currently defined. `provider_code` is present exactly when
  `reason` is `refused`; decoding refuses either a refusal without a code or
  a code with another reason (`AbsentItem::check` also checks constructed
  items). Fetched items retain their separate `item` identifier above.

### 10.1 `plans`: admission

- [pinned] Core sends a top-level `plan` only when `role.describe` declares
  `plans`. A runner without `plans` refuses any send carrying `plan` as
  `invalid_params {field: "plan"}` and writes nothing
  (`SendRequest::check_plan`). This group is independent of delivery modes.
- [pinned] A runner declaring `plans` accepts a fetch plan on the session's
  first `session.send`, freezes the fetched manifest, and answers
  `session.baseline` with the plan's `composition_digest` and items. An
  identical later repeat is a no-op for the plan: it does not fetch or freeze
  again, but the send's prompt and `send_id` still follow §9. A different
  later plan is refused `plan_drift {frozen?, sent}`.
- [pinned] A runner declaring `plans` fetches the plan's items itself, with
  the composition, under one deadline, and admits the session only when every
  required item arrived, the staleness check passed and no tool names
  collide. The plan and composition are passed to providers verbatim.
- [pinned] Admission refuses as: `fetch_unavailable {provider}`, retryable;
  `plan_stale`, naming the differences; `tool_name_collision {name, providers}`,
  naming the colliding model-facing name and every provider that offered it;
  `scope_unsupported` when the send is under a scope and the daemon lacks scopes. A later send whose plan differs from the frozen one is
  refused `plan_drift {frozen?, sent}`, naming the two plan identities.
  `frozen` is absent when the session's first episode was plan-less.
- [pinned] Any refusal during admission writes nothing.
- [pinned] A plan carrying a `compaction_item`, sent to a runner that does
  not declare `compaction`, is refused at admission `invalid_params {field:
  "plan.compaction_item"}` and writes nothing
  (`SendRequest::check_compaction_item`).
- [pinned] The `compaction_item` names the session's compaction provider as
  `provider`, a module id, inside `compaction_item: {provider, preset,
  params}`, frozen with the plan (`plan_compaction_provider`). It gates
  `compaction.ready` (§11.1): only that provider may send it.
- [pinned] The runner records the session's role versions, its frozen scope
  identity, the fetched tools and text, the composition and the joined system
  text with its join version in its start record. A session keeps its role
  versions for life.
- [pinned] The plan travels as `plan` on the session's first send, carried
  verbatim (`SendRequest::plan`); its shape is the fetch-plan section's.

### 10.2 `session_change`: mid-session changes

- [pinned] The `session_change` group holds the owner's mid-session change
  ops: `session.refresh {plan, policy, generation}` (fetch a new plan's
  items and hold them as the pending change, under its generation, with its
  policy), `session.refresh_policy {generation, policy, send_id}` (a new
  policy for the pending change) and `session.flush_prefix` (drop the
  frozen prefix and apply any pending change on the next step, whatever its
  policy). They are role ops, so one owner drives every runner through the
  same interface.
- [pinned] Their refusals, none retryable and each writing nothing:
  `rung_unsupported` (a policy naming a rung the session does not support
  for that surface), `pending_superseded` (a `session.refresh_policy` for a
  generation a later refresh has superseded) and `already_applied` (a
  `session.refresh_policy` for a generation a prefix rebuild or an append
  already applied) (`errors::SESSION_CHANGE_CODES`).
- [pinned] A runner that does not declare the group gets no mid-session
  changes: its owner never sends one undeclared, and applies the change at
  the next session instead.
- [pinned] The members are the ones listed.
- [open] Their request and reply types, the detail shape of each refusal,
  and the refusals of a stale or reused generation are not pinned. They
  follow the fetch-plan section of this crate (§10), which defines `plan`.
  Do not build against them yet.

## 11. Calls the runner makes

The hook call sites and the tool-call rules (§11.2, §11.3) are required of
every runner (§1). The compaction interface (§11.1) is owed by a runner
that declares the `compaction` group (§3), and only by one. Their wire
shapes belong to the provider roles named; this role states what the runner
owes.

### 11.1 The compaction interface

- [pinned] A runner that declares `compaction` owes everything in this
  section; a runner that does not owes none of it, and never calls a
  compaction provider.

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
  fence). An answer applies only if it names the newest issued request and
  arrives before that call's deadline. An answer arriving at or after the
  deadline is late: it is recorded and never applied. Every applied
  CompactionMessage, `WAIT` entry and exit, and call timeout is durable before
  the request it affects is sent.
- [pinned] A runner applies at most one view per step. Accepting a view
  consumes that step's answer fence; a second view for the same step is
  refused by the version and fence check, even if it names the same newest
  request, arrives before its deadline and carries a higher version.
- [pinned] `REFUSE` ends the run `error`. The run error's `provider_code`
  is the answer's role `code` (`RefuseCode` in the compaction-provider
  contract), with `provider` and `reason`; the answer's optional finer
  code, which the provider's wire names `provider_code`, becomes the run
  error's `provider_detail_code` (§6). Nothing is added to history, no
  model call is made for the refused step, and the session remains
  usable: the next send calls the provider again.
- [pinned] A failed or timed-out step call is not a refusal. The runner
  records it and sends with the last applied CompactionMessage; an
  over-window request then goes through the runner's own overflow handling,
  which never truncates history or sends it unmanaged. The run does not end
  `compaction_unavailable`.
- [pinned] `compaction_unavailable` is Setup-only: Setup failed or timed
  out with no answer. No model call is made, the run error carries no
  `provider_detail_code`, and the session remains usable: the next send
  calls Setup again, because no initial CompactionMessage was recorded.
- [pinned] `compaction.ready {session, request_id}` (`CompactionReady`):
  after a `WAIT`, the provider signals that it is done, and the runner calls
  again with a fresh status instead of waiting out the bound. It is a hint;
  the bound is the backstop. `session` is echoed verbatim from the status
  that carried the `WAIT`, and is opaque to the provider; `request_id` is
  that status's request id.
- [pinned] The ready is fenced like the answers: it acts only when
  `request_id` is the newest request the runner issued for the session. A
  ready for an older request, or before any was issued, is ignored, and is
  not an error (`CompactionReady::check`). An answer applies only if it names
  the newest issued request and arrives before that call's deadline. An
  answer arriving at or after the deadline is late: it is recorded and never
  applied.
- [pinned] The runner accepts `compaction.ready` only from the session's
  frozen compaction provider: the route's caller stamp must equal the
  provider of the plan's `compaction_item`. Any other caller, and any caller
  for a session without a `compaction_item`, is refused
  `not_session_compaction_provider`, and the session is not woken.
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
| `invalid_params` | a request field is malformed, unknown, or combined with one it excludes; a plan's `compaction_item` sent to a runner without `compaction` | `field` | no |
| `lineage_changed` | a read names a lineage that is not the session's | — | no: re-read from the tail |
| `unknown_mid` | `after_mid` names no message in the lineage | — | no |
| `unknown_run` | `run.result`, or `run.status` (§6.1, open: Q22), names no run of the session | — | no |
| `expired` | a read of an expired session (§9.1), including `run.status`, `run.cancel` and `session.retract` where their groups are declared | `expired_at_ms` (Unix epoch milliseconds) | no |
| `send_id_reuse` | a `send_id` reused with another payload or mode | `field` | no |
| `send_retracted` | a re-send of a send whose submission `session.retract` withdrew (§9.2) | — | no |
| `delivery_unsupported` | a send whose delivery mode the runner does not declare | `delivery` | no |
| `run_paused` | a send into a session whose last run is paused | `{run_id, reason}` | no |
| `scope_owner_mismatch` | a send or `session.baseline` from anyone but the owner | the accepted identity | no |
| `scope_adoption_refused` | a scoped send into a session recorded without a scope that cannot adopt it | `no_recorded_principal` or `principal_mismatch {recorded}` | no |
| `scope_unsupported` | a send under a scope on a daemon without scopes | — | no |
| `fetch_unavailable` | a required plan item missed the admission deadline | `provider` | **yes** |
| `plan_stale` | fetched items disagree with the composition | `differences` | no: re-plan |
| `tool_name_collision` | tools share a model-facing name | `name`, `providers` (every provider that offered it) | no |
| `plan_drift` | a later send's plan differs from the frozen one | `frozen?`, `sent` (plan identities; no `frozen` after a plan-less first episode) | no |
| `pre_user_unavailable` | a steer or queue under an unavailable `refuse` PreUser | `provider` | no |
| `transient` | a condition that may clear by itself | — | **yes** |
| `scope_not_synced` | the scope's owner has not re-synced after a daemon restart | — | **yes**, with backoff within the hold bound |
| `not_session_compaction_provider` | a `compaction.ready` from anyone but the session's frozen compaction provider | — | no |
| `rung_unsupported` | a `session.refresh` or `session.refresh_policy` whose policy names a rung the session does not support for that surface | — | no |
| `pending_superseded` | a `session.refresh_policy` for a generation a later refresh superseded | — | no |
| `already_applied` | a `session.refresh_policy` for a generation already applied | — | no |

- [pinned] Only the codes marked retryable are retried with the same request
  (`errors::is_retryable`). A refusal is an answer: retrying it unchanged
  meets the same refusal.
- [pinned] `send_retracted` is not yet in `errors::CODES`; it is not
  retryable, which `errors::is_retryable` already answers for it.
- [pinned] Route refusals the daemon classifies as retryable on open (module
  reloading or warming, target unavailable) are retried by the caller's
  transport under the daemon's own list; this role does not restate them.
- [pinned] The malformed-request code is `invalid_params`, as runners already
  answer. `tool-provider/v1` spells the same refusal `invalid_request`; the
  difference is between roles, and a consumer of both maps each by its own
  role.

### 12.2 Run errors (`errors::provider_codes`)

[pinned] A run that ends `error` because of a provider names the code
callers branch on as `provider_code` in its run error, and any finer
diagnostic as `provider_detail_code`, which never decides anything (§6).
The provider codes this role names:

| Provider code | When |
|---|---|
| `compaction_unavailable` | Setup only: Setup failed or timed out with no answer; no model call is made, and no `provider_detail_code` is carried. A failed or timed-out step call never writes it (§11.1) |
| `compaction_wait_exceeded` | a compaction `WAIT` hit its cap and the request could not be shown to fit |
| `pre_user_unavailable` | a user turn's PreUser hook was unavailable under `refuse` |

A compaction `REFUSE` adds the compaction-provider contract's `RefuseCode`
values (its §11) as `provider_code`; this role does not repeat them. Their
retryability, and that of the two compaction codes above, is the
compaction-provider contract's.

These are run errors, not request-refusal codes; `errors::provider_codes::CODES`
lists the three above.

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
- [pinned] Where the reason sits in a message body is the runner's message
  schema.

## 13. Decoding

- [pinned] Every field is declared required or optional.
- [pinned] **Lenient** (unknown fields ignored; enumerations decoded as open
  strings): `role.describe`, every reply and event a consumer decodes
  (`ReadPage`, `ModelPage`, `HeadMeta`, `RunResult`, `SubscribeEvent`,
  `SendReply`, `BaselineReply`, and the `run.status`, `run.cancel` and
  `session.retract` answers), and `compaction.ready`. One exception: a
  model-view entry's `source` kind is strict (§8).
- [pinned] **Strict on unknown fields** wherever absence changes the
  question: `session.read`, `session.head`, `run.result`, `run.status`,
  `session.subscribe` and `session.baseline` requests. A runner refuses an
  unknown field with `invalid_params` naming it.
- [pinned] **Lenient on fields, strict on values** where the request grows:
  `session.send` tolerates unknown fields (a newer owner against an older
  runner degrades rather than fails), but `delivery` refuses an unknown value
  naming the field. `retention` is a recognized field even on a runner without
  its group, and is refused there (§9.1), never treated as an unknown extra.
  `view` on `session.read` is strict the same way.
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
   whose step is durable but whose intent is not was never sent. The runner
   either resumes the run and dispatches it exactly once, or seals the run
   `interrupted` without dispatching it. Either way the call is never marked
   indeterminate, its tool runs at most once, and no later request carries
   the call without a result; a runner may drop the never-sent call from
   history. In that dropped shape, neither the cut-run transcript nor the
   follow-up transcript carries its id or `call_key`, attribution, or result.
   The assistant step's text may also be dropped, or retained exactly once
   without the call. The same shape is preserved through the follow-up.
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
9. **Expiry deletion survives a crash.** A durable retention tombstone makes
   the lineage unreadable; restart finishes deleting its transcript and all
   runner-held derived copies (§9.1).

Kill points (`points.rs`). [pinned] The list below. Approval-gated calls'
points join it with the runner's approve-gate work.

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
| `RetentionTombstoned` | a content-free expiry tombstone; transcript and derived copies not yet deleted | 9 |

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
| `compaction_setup_durable_once` | `compaction`, `model_view`, `queue`, `transcript_reads`, `run_ops`, and `FoldRecorded` (initial answer/fold durable before first model call) |
| `compaction_fence_crash_replays_model_view` | `compaction`, `model_view`, `queue`, `transcript_reads`, `run_ops`, and `CompactionApplied` (step answer durable before fold) |
| `model_view_half_open_ranges_and_travel` | `compaction`, `model_view`, `queue`, `transcript_reads`, `run_ops`; live head insertion precedes/travels with message zero, summaries stay whole on their first page, and the exclusive-end message survives |
| `compaction_wait_cap`, `compaction_late_or_stale_answers_discarded`, `compaction_step_timeout_uses_last_view`, `compaction_one_view_per_step` | `compaction`, `model_view`, `queue`, `transcript_reads`, `run_ops`; scripted provider and injected runner/model clock |
| `compaction_unavailable_distinct_from_refuse` | `compaction`, `queue`, `transcript_reads`, `run_ops`; Setup-only unavailability and role/finer refusal codes |
| `role_describe_shape`, `role_describe_cacheable`, `role_describe_groups_complete` | — |
| `extra_op_still_admitted` | — |
| `baseline_owner_only`, `baseline_matches_admission_reply`, `baseline_not_yet_before_first_request` | — |
| `compaction_ready_reasks_after_wait`, `compaction_ready_stale_request_ignored`, `compaction_ready_other_caller_refused`, `compaction_cursor_never_skips` | `compaction`, and a compaction provider in the plan |
| `compaction_item_without_group_refused_writes_nothing` | a subject that does not declare `compaction` |
| `tail_read_is_newest_page` (on a session longer than one page), `range_read_stops_by_count`, `range_read_stops_by_bytes`, `after_mid_reads_strictly_after`, `after_mid_unknown_mid_refused`, `read_other_lineage_refused`, `lineage_id_on_every_page`, `unknown_read_field_refused`, `head_has_no_bodies`, `include_originals`, `oversize_message_alone_on_its_page`, `never_written_session_empty_page`, `never_written_session_empty_head`, `describe_states_max_bytes` | `transcript_reads` |
| `run_result_completed_final_text`, `run_result_text_parts_joined_unchanged`, `run_result_completed_no_text_is_empty`, `run_result_interrupted_not_cancelled`, `run_attribution_per_message` | `run_ops` |
| `dispatched_to_per_call`, `indeterminate_until_closed`, `sibling_calls_distinct_call_keys`, `recurring_model_id_distinct_call_keys` | `dispatch_attribution` |
| `subscribe_from_head_no_gap_no_duplicate` | `streaming` |
| `model_view_differs_only_by_compaction`, `model_view_entry_sources`, `model_view_replacement_once_on_its_first_page`, `model_view_after_mid_refused`, `model_view_names_compaction_state` | `model_view` |
| `refresh_unsupported_rung_refused`, `refresh_policy_superseded_refused`, `refresh_policy_already_applied_refused`, `flush_prefix_applies_pending` | `session_change` |
| `steer_lands_at_step_boundary`, `steer_never_between_call_and_result` | `steer` |
| `guaranteed_steer_never_pending_or_unknown` | `queue`, `transcript_reads`; not applicable without `steer`, on a runner that declares `steer_receipt: confirm`, or without a held tool call |
| `resend_steer_delivered_stable` | `queue`, `transcript_reads`; not applicable without `steer` |
| `queue_starts_next_turn` | `queue` |
| `interrupt_cancels_then_starts`, `interrupt_waits_for_running_tool` | `interrupt` |
| `undeclared_delivery_refused` | a delivery mode the subject does not declare |
| `send_id_retry_same_answer`, `send_id_reuse_refused_naming_field`, `delivery_change_refused`, `unknown_delivery_refused`, `pre_user_refuse_writes_nothing`, `steer_inherits_frozen_runner_params` | `steer`, `queue` or `interrupt` |
| `send_id_retry_settled_same_answer` | `steer` or `queue`, and `run_ops` |
| `send_id_retry_written_once`, `send_id_reuse_writes_nothing`, `delivery_change_writes_nothing` | `steer`, `queue` or `interrupt`, and `transcript_reads` |
| `crash_at_StepRecorded`: resumes with exactly one dispatch or seals `interrupted` without dispatch; the call is never indeterminate and no later request carries it without a result | `transcript_reads`, `dispatch_attribution`, and `StepRecorded` |
| `crash_at_<point>` for the other points in §14, asserting their properties | the points the harness declares |
| `retention_honoured`, `retention_no_content_served`, `retention_shorten_continues_lineage`, `retention_equal_noop`, `retention_lengthen_refused`, `retention_late_opt_in_refused`, `retention_zero_refused`, `retention_above_max_refused` | `retention`, `queue`, `transcript_reads`, `run_ops` |
| `retention_run_status_expired` | the above; not applicable unless the major lists `run.status` (§6.1) |
| `retention_without_group_refused` | `queue`; not applicable when `retention` is declared |
| `retention_active_run_never_expires` | `retention`, `queue`, `transcript_reads`, `run_ops`, held non-terminal run (active or paused) |
| `crash_at_RetentionTombstoned` | `retention`, `queue`, `transcript_reads`, `run_ops`, and `RetentionTombstoned`; real process kill, expiry remains readable by name and deletion completion is reported after restart |
| `run_status_undelivered_steers_present_when_clean` | `run_status`, `queue`; an ended run's status carries `undelivered_steers`, `[]` when every steer was rendered or none was sent |

Consumer-side rules no live runner can be made to exercise (an unknown run
state, an unknown event kind, a describe answer with a partial group, a
model page that repeats a replacement, a completed run without its final
message) are tested against the vectors in this crate.
The equal-anchor rule's insertion-before-message half is live conformance
(`model_view_half_open_ranges_and_travel`). Its
insertion-before-non-empty-replacement half has no v1 producer (§8):
compaction-provider §8 applies the newest CompactionMessage alone, §12
gives every replacement its entire range, and hook outputs are fields on a
transformed raw record, not range insertions. It is tested against the
vectors only and is not claimed as live runner conformance.

Verdict, as for `tool-provider/v1`: a case whose requirements the subject
does not declare is skipped, never passed; a run with skips is "conforming
for declared capabilities", naming them; a run fails if any case fails or if
no kill ended a real process.

## Not in v1

- A neutral, role-defined message schema. Message bodies stay in each
  runner's own schema (§4.2); a consumer reading more than one runner decodes
  each runner's schema.
- An observation stream of read-only session events.

## Open questions

Each open question lists the options seen. A settled question keeps its number and records
the decision; the items it governs are pinned above.

- **Q1. The request envelope.** Settled: `{method, params}` on the runner's
  session-bound management route (§1).
- **Q2. The `role.describe` shape.** Settled: `majors` plus top-level
  `capabilities`, the shape `tool-provider/v1` uses (§2).
- **Q3. The session-capability source.** Settled as drafted:
  `session_capabilities_from: "admission" | "baseline"` (§2).
- **Q4. `compaction.ready`.** Settled, as the compaction provider's role
  owner proposed: not a required op but a `compaction` group, all or nothing,
  owning everything in §11.1 plus the inbound `compaction.ready {session,
  request_id}`. A runner that only steers sessions it does not host declares
  none of it. `REQUIRED_OPS` drops to `role.describe` and
  `session.baseline`. `session` is echoed verbatim from the status that
  carried the `WAIT` and is opaque to the provider; `request_id` is fenced to
  the newest issued request, and a ready for an older one is ignored. A
  ready from anyone but the session's frozen compaction provider is refused
  `not_session_compaction_provider`. A plan with a `compaction_item` on a
  runner without the group is refused at admission `invalid_params {field:
  "plan.compaction_item"}` and writes nothing (§3, §10.1, §11.1).
- **Q5. Head metadata.** Settled as drafted: its own op `session.head`, with
  `updated_at` in milliseconds since the epoch and `last_run_state` as
  `{run_id, state, reason?}` (§4.3).
- **Q6. The byte cap.** Settled: the request member is `max_bytes`; the
  runner states its default and maximum in `role.describe`; a single message
  larger than the cap comes alone on its page, over the cap, with
  `next_from_ordinal` set, and is never truncated or refused (§2, §4).
- **Q7. The message body.** Settled: the runner's own schema, opaque to the
  role. A neutral schema is not in v1.
- **Q8. The model view.** Settled, in the compaction provider's role
  owner's shape: each entry carries a `source`, `{kind: "message", ordinal,
  mid}` or `{kind: "replacement", compaction_id, version, from_ordinal,
  to_ordinal}`, with half-open ranges and empty ranges inserting before
  their anchor. Entries are non-decreasing by anchor, insertions first at a
  tie. Tail and range reads apply, keyed on transcript ordinals; a
  replacement is returned
  whole, once, on the page holding its `from_ordinal` (tail insertions on
  the last page). `after_mid` with
  `view: "model"` is refused `invalid_params {field: "view"}` in v1. The page
  names the compaction state it reflects, `compaction_id` and `version`,
  both absent before any compaction (§8).
- **Q9. `run.result`.** Settled: a run that has not ended answers its current
  state with no final message; `max_steps` is a terminal state of the role;
  `transform_unavailable` stays a runner state, decoded open (§6). The
  separator, chosen by the session owner's role owner, is the empty string:
  text parts are concatenated exactly as emitted. A completed run whose
  final message has no text parts answers `final_message {text: ""}`, so an
  absent `final_message` means only that the run has not ended (§6).
- **Q10. Episodes.** Settled as drafted: an opaque string (§6).
- **Q11. Dispatch attribution placement.** Settled as drafted: a per-message
  `tool_calls` list beside the message body, and a group that extends
  `session.read` requires only that op (§3, §5).
- **Q12. `interrupt`.** Settled: its own capability group. A runner refuses a
  delivery mode it does not declare as `delivery_unsupported` with
  `detail.delivery`, never falling back to `queue`. An absent `delivery` is
  `queue`, a declared group like the others, and a runner that admits
  sessions declares it (§3, §9).
- **Q13. The mark.** Settled as drafted: `mark`, opaque JSON (§9).
- **Q14. Change ops.** Settled, as the session owner's role owner chose:
  role ops in their own group, `session_change`, holding `session.refresh
  {plan, policy, generation}`, `session.refresh_policy {generation, policy,
  send_id}` and `session.flush_prefix`, with their refusals
  (`rung_unsupported`, `pending_superseded`, `already_applied`). A runner
  without the group gets no mid-session changes, and owners never send them
  undeclared (§3, §10.2).
- **Q15. The admission reply and baseline.** Settled as drafted: one nested
  `baseline` object, the same struct `session.baseline` answers, with
  `not_yet` as the answer before the first request (§10).
- **Q16. Unnamed codes.** Settled: `invalid_params` stays, and
  `tool-provider/v1`'s `invalid_request` is a difference between roles;
  `send_id_reuse` names the differing field in `detail.field`; the drafted
  names `unknown_run`, `tool_name_collision`, `plan_drift` and
  `scope_owner_mismatch` stay (§12).
- **Q17. A session with no lineage yet.** Settled, a third option: a read of
  a session never written returns an empty page with no `lineage_id` and no
  `next_from_ordinal`, and an absent `lineage_id` means no lineage yet (§4.2).
  `session.head` follows the same rule: for such a session every member of
  its answer is absent (§4.3).
- **Q18. Subscription spellings.** Settled as drafted: `start`, `live` or a
  `head` object passed back verbatim, and events pinned only as far as
  `kind` and `cursor` (§7).
- **Q19. Owner sends on a runner with its own send parameters.** Settled: a
  `steer` or `interrupt` into an existing session takes every runner
  parameter it omits from the session's frozen values (§9).
- **Q20. Kill points.** Settled: the list in §14. Approval-gated calls'
  points join with the runner's approve-gate work; `SendRecorded` is not
  split.
- **Q21. Stability.** Settled: stability is data, never part of a capability
  name. Each major in `role.describe` carries `stability`, `alpha`, `beta` or
  `stable`, read as `alpha` when absent; this draft is `alpha` (§2).
- **Q22. `run.status` beyond its pinned members.** Open. One runner answers
  a `run_id` it does not know with `{state: "unknown"}`, and answers a
  request without `run_id` with a session-level status, `{state: "idle"}`
  or `{state: "active", run_id, step}`. Options: (a) refuse an unknown
  `run_id` `unknown_run`, as `run.result` does, and leave the session-level
  status to each runner (drafted, §6.1); (b) make `unknown` a role state
  for `run.status` only, never terminal; (c) pin the session-level status
  as a second mode of the op.

## Gaps in broca today

Where broca's served surface, at broca commit `f06a487`, differs from this
document. Paths are in the broca repository. An optional group broca does
not declare (`model_view`, `compaction`, `session_change`, `retention`) is
not a gap. None of this is a change request against broca.

1. **Served ops it does not declare.** broca serves `run.status`,
   `run.cancel` and `session.retract` (`crates/broca-module-serve/src/serve.rs:2013-2051`;
   `crates/broca-module/src/serve.rs:222-240`), but its `role.describe`
   lists neither the ops nor the `run_status`, `run_cancel` and `retract`
   groups (`crates/broca-wire/src/role.rs:181-191`).
2. **`run.status` against §6.1.** An unknown run is answered `{state:
   "unknown"}`, not refused `unknown_run` (Q22); a request without
   `run_id` answers a session-level status
   (`crates/broca-module/src/serve.rs:242-248`; `crates/broca-wire/src/lib.rs:752-853`).
   `undelivered_steers` is emitted only when it is not empty, so an ended
   run with every steer rendered omits it where §6.1 requires `[]`
   (`crates/broca-module/src/serve.rs:455-518`).
3. **No per-message run attribution,** although broca declares `run_ops`:
   a page message carries no `run: {run_id, episode, final}`
   (`crates/broca-wire/src/lib.rs:604-634`).
4. **No `session_capabilities` in the baseline.** broca declares
   `session_capabilities_from: "admission"`, and its baseline has no
   `session_capabilities` member (`crates/broca-wire/src/role.rs:214-217,239-255`).
   An absent member declares no session-level capability (§13), so broca
   declares neither `mid_session_appends` nor `ordered_hook_phases`.
5. **`lineage_id` is absent on legacy lineages** until their next write,
   even on a page that has messages (`crates/broca-wire/src/lib.rs:644-659`).
   §4.2 allows an absent `lineage_id` only on the empty page of a session
   never written.
6. **`send_id` is optional, and `prompt` may be given as `prompt_blocks`**
   instead (`crates/broca-wire/src/lib.rs:121-135`). The role requires
   both members.
7. **`mark` is not recorded:** it is not a send field, so it is ignored as
   an unknown key (`crates/broca-wire/src/lib.rs:108-226,310-318`).
8. **A steer or interrupt inherits a fixed set of frozen parameters**
   (model, tool choice, generation settings, stop conditions, cache,
   budget, context limit, work class, service tier), while `keep_warm` and
   `on_restart` stay per send
   (`crates/broca-module-serve/src/serve.rs:1750-1804`). §9 has every
   omitted runner parameter take its frozen value.
9. **A queued send behind a restart pause is answered `pending`,** not
   refused `run_paused` (`crates/broca-module/src/actor.rs:2240-2245`); a
   send behind an authentication pause is refused `run_paused`
   (`crates/broca-module/src/actor.rs:2187-2207`).
10. **`send_id_reuse` names the differing field only when it can identify
    one:** an aggregate or legacy identity mismatch omits `detail.field`
    (`crates/broca-module-serve/src/serve.rs:2554-2575`).
11. **`session.send` may answer `state: "paused"`,** with `run_id` and
    `pause_reason`, to a keyed retry of a run paused by a restart
    (`crates/broca-module/src/actor.rs:2095-2108`). `state` is decoded open
    (§9), but the role names no `paused` send state.
