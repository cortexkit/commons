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
revision every numbered question is settled. Two items wait on the
fetch-plan section (§10): the `compaction_item` provider field (§10.1,
provisional) and the `session_change` request and reply types (§10.2,
open).

An LLM runner is any module that runs model sessions. Nothing here names a
particular implementation; "Gaps in broca today" at the end compares the one
shipping runner with this document.

## 1. Role identity and addressing

- [pinned] A runner lists `llm-runner/v1` in its manifest's
  `capabilities.provides` (`PROVIDES`). A module may serve several majors.
- [pinned] Required ops (`REQUIRED_OPS`): `role.describe` and
  `session.baseline`. `compaction.ready` is not required: it belongs to the
  `compaction` group (§3, §11.1).
- [pinned] The hook call sites and the tool-call rules (§11.2, §11.3) are
  required too. They are calls the runner makes, not ops it serves, so
  `role.describe` does not list them. The compaction interface (§11.1) is
  not required: it is the `compaction` group.
- [pinned] Everything else is a declared capability group (§3). A runner
  serves a group only if it declares it, and a consumer uses a group only if
  it is declared.
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
  max_bytes?, steer_receipt?}` (`RoleDescribe`), the shape `tool-provider/v1`
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
| `dispatch_attribution` | `session.read` (per-call fields) | §5 |
| `streaming` | `session.read` (`head`), `session.subscribe` | §7 |
| `model_view` | `session.read` (`view: "model"`) | §8 |
| `steer` | `session.send` with `delivery: "steer"` | §9 |
| `queue` | `session.send` with `delivery: "queue"` | §9 |
| `interrupt` | `session.send` with `delivery: "interrupt"` | §9 |
| `plans` | `session.send` with `plan` (baseline via required `session.baseline`) | §10.1 |
| `compaction` | `compaction.ready`, and the calls of §11.1 the runner makes | §11.1 |
| `session_change` | `session.refresh`, `session.refresh_policy`, `session.flush_prefix` | §10.2 |

- [pinned] A group that only adds fields to `session.read` requires
  `session.read`, not all of `transcript_reads`.
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
- [pinned] A run that ends `error` because of a provider carries the
  provider's code as `provider_code` in `error`, with the provider and its
  reason. `errors::provider_codes` lists the ones this role names.
- [pinned] An unknown `run_id` is refused `unknown_run`.
- [pinned] **Run attribution.** Each message records which run and episode
  produced it, and whether it is that run's final message.
  [pinned] It rides as `run: {run_id, episode, final}` (`RunAttribution`),
  with `episode` an opaque string.

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
  `ordinal` is its anchor). At an equal anchor, insertions come before any
  message or non-empty replacement. An insertion following either at its
  anchor is malformed (`ModelPageProblem::InsertionAfterEntry`).
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
  runner parameter it omits (model, generation settings and the like) from
  the session's frozen values, so an owner that knows only the role's
  members can send one.
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
  first step and frozen for the session. `mid_session_appends` is
  re-evaluated only on a model switch, which rebuilds the prefix anyway.
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
- [provisional] The `compaction_item` names the session's compaction provider
  as `provider`, frozen with the plan. The field name and position are
  provisional until the fetch-plan section (§10) defines `plan`; do not build
  against `plan.compaction_item.provider` yet.
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
- [pinned] `REFUSE` ends the run `error` with the provider's code as
  `provider_code`, adds nothing to history, and leaves the session usable.
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
| `unknown_run` | `run.result` names no run of the session | — | no |
| `send_id_reuse` | a `send_id` reused with another payload or mode | `field` | no |
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
- [pinned] Route refusals the daemon classifies as retryable on open (module
  reloading or warming, target unavailable) are retried by the caller's
  transport under the daemon's own list; this role does not restate them.
- [pinned] The malformed-request code is `invalid_params`, as runners already
  answer. `tool-provider/v1` spells the same refusal `invalid_request`; the
  difference is between roles, and a consumer of both maps each by its own
  role.

### 12.2 Run errors (`errors::provider_codes`)

[pinned] The provider codes this role names:

| Provider code | When |
|---|---|
| `compaction_unavailable` | Setup or a compaction call failed or timed out with no answer |
| `compaction_wait_exceeded` | a compaction `WAIT` hit its cap and the request could not be shown to fit |
| `pre_user_unavailable` | a user turn's PreUser hook was unavailable under `refuse` |

These are run errors, not request-refusal codes; `errors::provider_codes::CODES`
lists them.

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
  `SendReply`, `BaselineReply`), and `compaction.ready`. One exception: a
  model-view entry's `source` kind is strict (§8).
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
| `compaction_ready_reasks_after_wait`, `compaction_ready_stale_request_ignored`, `compaction_ready_other_caller_refused`, `compaction_cursor_never_skips` | `compaction`, and a compaction provider in the plan |
| `compaction_item_without_group_refused_writes_nothing` | a subject that does not declare `compaction` |
| `tail_read_is_newest_page` (on a session longer than one page), `range_read_stops_by_count`, `range_read_stops_by_bytes`, `after_mid_reads_strictly_after`, `after_mid_unknown_mid_refused`, `read_other_lineage_refused`, `lineage_id_on_every_page`, `unknown_read_field_refused`, `head_has_no_bodies`, `include_originals`, `oversize_message_alone_on_its_page`, `never_written_session_empty_page`, `never_written_session_empty_head`, `describe_states_max_bytes` | `transcript_reads` |
| `run_result_completed_final_text`, `run_result_text_parts_joined_unchanged`, `run_result_completed_no_text_is_empty`, `run_result_interrupted_not_cancelled`, `run_attribution_per_message` | `run_ops` |
| `dispatched_to_per_call`, `indeterminate_until_closed`, `sibling_calls_distinct_call_keys`, `recurring_model_id_distinct_call_keys` | `dispatch_attribution` |
| `subscribe_from_head_no_gap_no_duplicate` | `streaming` |
| `model_view_differs_only_by_compaction`, `model_view_entry_sources`, `model_view_replacement_once_on_its_first_page`, `model_view_after_mid_refused`, `model_view_names_compaction_state` | `model_view` |
| `refresh_unsupported_rung_refused`, `refresh_policy_superseded_refused`, `refresh_policy_already_applied_refused`, `flush_prefix_applies_pending` | `session_change` |
| `steer_lands_at_step_boundary`, `steer_never_between_call_and_result` | `steer` |
| `queue_starts_next_turn` | `queue` |
| `interrupt_cancels_then_starts`, `interrupt_waits_for_running_tool` | `interrupt` |
| `undeclared_delivery_refused` | a delivery mode the subject does not declare |
| `send_id_retry_same_answer`, `send_id_reuse_refused_naming_field`, `delivery_change_refused`, `unknown_delivery_refused`, `pre_user_refuse_writes_nothing`, `steer_inherits_frozen_runner_params` | `steer`, `queue` or `interrupt` |
| `send_id_retry_settled_same_answer` | `steer` or `queue`, and `run_ops` |
| `send_id_retry_written_once`, `send_id_reuse_writes_nothing`, `delivery_change_writes_nothing` | `steer`, `queue` or `interrupt`, and `transcript_reads` |
| `crash_at_StepRecorded`: resumes with exactly one dispatch or seals `interrupted` without dispatch; the call is never indeterminate and no later request carries it without a result | `transcript_reads`, `dispatch_attribution`, and `StepRecorded` |
| `crash_at_<point>` for the other points in §14, asserting their properties | the points the harness declares |

Consumer-side rules no live runner can be made to exercise (an unknown run
state, an unknown event kind, a describe answer with a partial group, a
model page that repeats a replacement, a completed run without its final
message) are tested against the vectors in this crate.

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

## Gaps in broca today

Where broca's shipped surface differs from this document. Paths are in the
broca repository; `Bn` labels name slices of broca's own build plan
(`docs/extensibility-build-plan.md`). None of this is a change request against
broca beyond what that plan already schedules.

1. **No `role.describe`.** The served ops are `cap.install`, `spend.delta`,
   `session.send`, `session.import`, `session.retract`, `run.cancel`,
   `run.status`, `session.read` and the `session.subscribe` stream
   (`crates/broca-module-serve/src/serve.rs:1514-1836`); any other op gets
   `unknown_method` (`crates/broca-module-serve/src/serve.rs:1837-1848`).
   Until `role.describe` ships, the manifest claims
   `session-send-delivery/v1` instead
   (`crates/broca-module-serve/src/manifest.rs:37`), so broca has nowhere to
   declare a group: no `compaction` and no `session_change` declaration
   exists. Planned in B9 (`docs/extensibility-build-plan.md:358-367`).
2. **No `session.baseline`, and the admission reply carries no baseline.**
   `SendResult` is `active {run_id} | finished {run_id, reason} | pending
   {submission_id}` (`crates/broca-wire/src/lib.rs:510-523`). Planned in B8
   (`docs/extensibility-build-plan.md:347-356`).
3. **No `compaction` group and no `model_view`.** Broca serves no
   `compaction.ready`, has no `not_session_compaction_provider` caller check,
   and does not check a plan's `compaction_item` at admission. B14 plans
   inbound `compaction.ready(session)` and `session.read {view: model}`
   (`docs/extensibility-build-plan.md:441`): the planned ready names only the
   session, where the role's also carries `request_id`. Page messages carry
   no `source` (`crates/broca-wire/src/lib.rs:525-532`), and a page names no
   compaction state (`crates/broca-wire/src/lib.rs:568-585`). Thus it also
   lacks the half-open replacement ranges, Setup head insertion `[0, 0)`,
   tail insertion placement and insertion-first ordering required by §8.
4. **`session.read` takes only `from_ordinal`, `limit` and `include_tools`**
   (`crates/broca-wire/src/lib.rs:464-476`): no `after_mid`, `lineage_id`,
   `max_bytes`, `include_originals` or `view`, so no `lineage_changed` or
   `unknown_mid` refusal. The request is already strict on unknown fields,
   as §13 requires. `include_tools` is a runner extra the role does not
   define.
5. **Pages are capped by count only:** default 200, maximum 500
   (`crates/broca-core/src/context.rs:30-34`), with no byte cap and no
   `max_bytes` in a describe answer (the role's values for broca are a 4 MiB
   default and a 16 MiB maximum). `next_from_ordinal` is already set
   whenever a page stops early.
6. **`lineage_id` is absent on legacy lineages** until their next write
   (`crates/broca-wire/src/lib.rs:563-566,574-575`), so a page of a session
   that has messages can arrive without it. The role allows an absent
   `lineage_id` only on the empty page of a session never written.
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
    beside the role's (`crates/broca-wire/src/lib.rs:669-743`). The
    `broca-session` tool's final reply already joins text parts with nothing
    inserted (`crates/broca-module-serve/src/bin/broca-session.rs:1109-1122`),
    but emits nothing for an empty text
    (`crates/broca-module-serve/src/bin/broca-session.rs:1592-1601`), where
    `run.result` answers `text: ""`.
12. **`session.subscribe` is lenient on unknown fields**
    (`crates/broca-wire/src/lib.rs:420-424`): a misspelled `from` attaches
    live. The role makes the request strict. The `head` and cursor shape
    `{wal_seq, sub_index}` (`crates/broca-wire/src/lib.rs:409-414`) and the
    event shape (`crates/broca-wire/src/lib.rs:756-766`) already fit §7.
13. **`send_id` is optional and `model` is required on every send**
    (`crates/broca-wire/src/lib.rs:112-115`). The role requires `send_id` on
    owner sends. Broca still requires `model` on every send, where the role
    has a `steer` or `interrupt` into an existing session take every omitted
    runner parameter from the session's frozen values (§9). There is no `mark` and no
    `plan` member yet (B8, B12). The `delivery` modes and their strict value
    already match §9 (`crates/broca-wire/src/lib.rs:177-190`), but there is
    no `delivery_unsupported` refusal: broca accepts every mode.
14. **`send_id_reuse` and `invalid_params` carry no `detail.field`:** the
    error body is built from the code and message alone
    (`crates/broca-module-serve/src/serve.rs:2078,2102,2123-2141`).
15. **Separate steer and queue ops in the build plan,** being corrected in
    broca. B12 still names new ops `session.steer` and `session.queue`
    (`docs/extensibility-build-plan.md:417`); the role uses `session.send`
    with `delivery` instead.
16. **No `session_change` ops.** `session.refresh` and
    `session.refresh_policy`, with `pending_superseded` and
    `already_applied`, are planned in B15
    (`docs/extensibility-build-plan.md:451,454`); `session.flush_prefix` and
    `rung_unsupported` in B17 (`docs/extensibility-build-plan.md:475,478`).
    None is served today: any of them gets `unknown_method`
    (`crates/broca-module-serve/src/serve.rs:1837-1848`).
