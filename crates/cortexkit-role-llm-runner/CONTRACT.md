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
revision every numbered question, including Q22 (§6.1), is settled.
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
  the operations not every runner serves: `run.status`, `run.cancel`,
  `run.tool_calls`, `session.retract`, `session.adopt` and
  `session.resolve`. A runner serves a group only if it declares it, and a
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
  max_bytes?, steer_receipt?, retention?, run_cancel?}` (`RoleDescribe`), the shape `tool-provider/v1`
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
  declared, and optional cancellation-point metadata (§3).
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
| `input_token_cap` | `session.send` with `budget.max_input_tokens` | §9.3 |
| `structured_output` | `session.send` with `structured_output`; its answer on `run.result` and `run.status` | §9.4 |
| `adopt` | `session.adopt` | §9.5 |
| `tags` | `session.read` (`tags`), `session.resolve` | §9.6 |
| `run_tool_calls` | `run.tool_calls` | §6.3 |
| `delivery_order` | `session.send` with `delivery_order`, plan-named senders and plan templates | §9.7 |

- [pinned] A group that only adds fields to `session.read` requires
  `session.read`, not all of `transcript_reads`.
- [pinned] `run_status`, `run_cancel`, `retract`, `input_token_cap`,
  `structured_output`, `adopt`, `tags`, `run_tool_calls` and
  `delivery_order` are each declared on their own: a runner may serve any of them without the others,
  and without `run_ops` or `transcript_reads`. `tags` adds fields to
  `session.read`, so it requires that op (and `session.resolve`), not all of
  `transcript_reads`.
- [pinned] A caller MUST NOT send `budget.max_input_tokens` unless the runner
  advertises `input_token_cap`. Like the other optional groups, it is checked
  by `check_describe` for its required op; it adds no discovery metadata.
- [pinned] A caller MUST NOT send `structured_output` unless the runner
  advertises `structured_output`. `check_describe` checks the group for
  `session.send` like the other optional groups; it adds no discovery
  metadata.
- [pinned] A caller MUST NOT send `session.adopt` unless the runner
  advertises `adopt` (§9.5). `check_describe` checks the group for
  `session.adopt`; it adds no discovery metadata.
- [pinned] A caller MUST NOT send `session.resolve` unless the runner
  advertises `tags`, nor `run.tool_calls` unless it advertises
  `run_tool_calls` (§9.6, §6.3). `check_describe` checks each group for its
  ops; neither adds discovery metadata.
- [pinned] A caller MUST NOT send `delivery_order` or its members
  (`dedupe_key`, `expires_at_ms`, `template`), nor a plan naming `senders`,
  unless the runner advertises `delivery_order` (§9.7). A runner of this
  revision without the group refuses them by name; an older runner, lenient
  on send fields, would ignore them and deliver the send as a legacy
  `queue`, starting a run, which is why the caller's check comes first.
- [pinned] `role.describe.run_cancel?: {at: [string, ...]}` optionally
  advertises cancellation points (§6.2). Absence means `now` only.
  `check_describe` refuses by name a `run_cancel` member without the
  `run_cancel` group (`run_cancel_undeclared`), an empty `at` list
  (`empty_run_cancel_at`), a list omitting `now` (`missing_run_cancel_now`),
  or duplicates (`duplicate_run_cancel_at`). Unknown values are tolerated
  for forward compatibility; consumers use only values they know.
- [pinned] `session.warm` has no role group. A `session_warm` group waits
  until a second consumer exists.
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
- [pinned] A single message larger than the requested byte cap comes alone
  on its page, over the cap, with `next_from_ordinal` set if more follow. It
  is never truncated: the complete encoded reply is served whole when it fits
  the transport frame limit of 4,194,304 bytes, including the 21-byte header.
  Otherwise the runner refuses `reply_too_large`, naming the message by its
  session and ordinal (§12).
- [pinned] A page is an ordinal range and nothing more. It is not aligned to
  turns, runs or tool-call boundaries: a tool call and its result may arrive
  on different pages.
- [pinned] Each message (`ReadMessage`) carries `ordinal`, `mid` and its
  final values: what later steps render and what executed. The raw view and
  the model view then differ only by compaction.
- [pinned] **Only finished messages.** `session.read` returns only messages
  the runner will no longer change. A message still being generated (a
  model step streaming, a tool result being written) is never an entry
  until it completes. This is stricter than, and contains, the final-values
  rule above; streaming progress reaches a consumer through the stream
  (§7), never through a partial entry.
- [pinned] **Read stability.** Once `session.read` has returned an entry
  under a `lineage_id`, its `message` bytes, `ordinal`, `mid`, `role`,
  `written_at`, `delivered_as`, and its part keys and tags never change
  under that lineage. Envelope members that report later state move forward
  only as their own rules say (`run.final`, §6; `tool_calls`'
  `call_key`, `dispatched_to` and `indeterminate`, §5). A runner that reads
  from a host freezes the bytes it first ingested and serves its own
  journal, never the host's live history. A host change that reaches the
  model (a revert, an edited part) and a recovery that drops or rewrites a
  written message (§14, property 4) start a new lineage, so reads of the old
  one are refused `lineage_changed`; changed bytes never appear under the
  same lineage. A reader may treat changed bytes under one lineage as a
  runner defect.
- [pinned] **Bookkeeping stays out.** Host records a runner writes for its
  own compaction bookkeeping (a host's native compaction message, summary or
  entry) are outside the transcript: they never appear in `session.read`
  and are never rendered to the model on top of the view.
- [pinned] **The envelope.** A read entry is `{ordinal, mid, message, run?,
  originals?, tool_calls?, tags?, role?, written_at?, delivered_as?}`. Every
  runner-neutral fact a consumer relies on rides in the envelope, never only
  inside `message`: the position (`ordinal`, `mid`), who spoke (`role`,
  `read::role`: `user`, `assistant`, `tool`, `system`, decoded open),
  when (`written_at`, milliseconds since the Unix epoch, of the write or of
  the runner's first ingest of a message its host stored), its part tags
  (`tags`, §9.6), its run (`run`, §6), its tool calls (`tool_calls`, §5) and,
  for a `delivery_order` send, the mode it went out in (`delivered_as`,
  §9.7). `role` and `written_at` are
  optional: a runner sets them whenever it knows them, which is always for a
  message it wrote itself, and an absent one means not known. A consumer
  never decodes `message` for any of these facts, so a reader of several
  runners needs none of their schemas for them.
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
  milliseconds since the Unix epoch. `last_run_state` is the pinned run
  summary; a page's `lineage_state` is a runner member, not pinned.
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
  [pinned] The answer is `{run_id, state, reason?, error?, detail?, final_message?:
  {ordinal, mid, text}, structured_output?, context_ceiling?}` (`RunResult`;
  `structured_output` is §9.4's result, on an ended run admitted with it;
  `context_ceiling` is below), and a run that has not ended answers
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
  terminal, and the terminal states `completed`, `max_steps`, `budget_exhausted`,
  `context_ceiling`, `cancelled`, `error` and `interrupted`. States are decoded open: a runner's own state
  (broca's `transform_unavailable`, say) decodes as a plain string, and a
  consumer does not treat an unknown state as terminal.
- [pinned] Structured output adds no state. A run that calls `submit_result`
  (§9.4) ends `completed` after that step, whether or not the submitted value
  validates: `validation: "failed"` is reported on a `completed` run, and no
  request flag turns it into `error`. A run admitted with structured output
  that ends in any state other than `completed` reports `validation:
  "absent"`, and so does a `submit_tool` run that completes without calling
  `submit_result`.
- [pinned] `budget_exhausted` is not retryable within the run. A caller needing
  more input opens a new session, not another episode on the exhausted one.
  Both `run.result` and `run.status` carry `detail: {limit, spent, cause}`
  (`BudgetExhaustedDetail`): `limit` is the frozen u64 session cap, `spent` is
  the session's measured input sum (§9.3), and `cause` is `limit_reached` or
  `usage_unreported`. Under `usage_unreported`, `spent` is the sum of measured
  steps before the unreported step; that step is never counted as zero.
- [pinned] `context_ceiling` ends a run whose session's context reached its
  ceiling at a step boundary: the runner does not send the next request, so
  it never sends one over the model's window. It is handled exactly like
  `budget_exhausted`: decoded through `RunState::Other`
  (`RUN_STATE_CONTEXT_CEILING`, `RunState::is_context_ceiling`, and
  `is_terminal` recognises it), never `error` and never a `provider_code`,
  because no model or provider failed. Both `run.result` and `run.status`
  carry `context_ceiling: {window, size, cause}`
  (`errors::ContextCeilingDetail`), the detail of the `context_ceiling`
  refusal (§12.1). A caller that routes between models MUST NOT charge
  backoff to the model, or mark it unhealthy, for it. A send that would start
  or join a run while the context is already at or over the ceiling is
  refused `context_ceiling` instead, writing nothing (§9).
- [pinned] **Waiting at the ceiling.** A run the runner holds at the
  ceiling while it waits for a compaction view (up to the provider's bound,
  300 s, say) is waiting, not stalled. It stays in the non-terminal state
  `active` and names `reason: context_ceiling_wait`
  (`run::RUN_REASON_CONTEXT_CEILING_WAIT`, `RunReason::ContextCeilingWait`)
  on `run.status`, on `session.head`'s `last_run_state` and on the stream
  event that reports the wait (§7), with `deadline_ms`, the absolute time
  the wait ends in milliseconds since the Unix epoch, on `run.status`
  (`RunStatus::context_ceiling_wait`) and on that event. This reuses the
  shape a non-terminal run already uses to say why it is not moving, a
  state with an open `reason`, as `paused` does with `auth_required`; it is
  not `paused`, because a paused run waits for a caller's retry and changes
  how sends behind it are admitted (§9), while this wait ends on its own. A
  watchdog MUST NOT nudge or stop a run waiting this way before
  `deadline_ms`. The wait ends in progress, when a view arrived (the reason
  and deadline are gone), or in the terminal `context_ceiling` state with
  `cause: view_timeout`.
- [pinned] A provider's own refusal of a request as too long is different:
  that is the provider refusing, and the run ends `error` with
  `provider_code: context_overflow` (§12.2). A runner reports
  `context_ceiling` only for a stop it decided itself before sending.
- [pinned] A hard stop (`max_steps`, `stop_when`, `budget_exhausted` or
  `context_ceiling`) leaves
  no dangling tool calls: every pending, undispatched call receives the
  runner's native not-executed result in the same durable commit as the run's
  terminal state. `stop_when` names a runner's hard-stop condition, not an
  additional role state. The result is the same refusal-before-dispatch shape
  used for hook denials such as `pre_tool_denied`; body layout stays
  runner-specific (§12.3). For `budget_exhausted` and `context_ceiling`, its
  refusal code is the state's name wherever that runner places a refusal
  code. No dispatch
  intent is recorded for these calls.
- [pinned] The same rule covers a step that calls `submit_result` (§9.4): no
  call of that step is dispatched before the step completes, and every call
  of it other than the counted `submit_result` gets the same never-executed
  result, with refusal code `skipped_after_submit`, in the same durable commit
  as the run's end. No dispatch intent is recorded for them.
- [pinned] A run error (`RunResult::error`) is `{class, message,
  provider_code, status?, retry_after_secs?, resets_at_ms?, account_ref?,
  provider?, reason?, provider_detail_code?}`. `class`, `message` and
  `provider_code` are required strings; `class` and `provider_code` are
  decoded open. `status` is an HTTP status when applicable;
  `retry_after_secs` is a number of seconds and may be fractional;
  `resets_at_ms` is milliseconds since the Unix epoch. `account_ref`,
  `provider`, `reason` and `provider_detail_code` are optional strings.
  An absent optional member means not measured, never zero.
- [pinned] `provider_code` is the code callers branch on.
  `errors::provider_codes` lists the ones this role names (§12.2). A
  compaction `REFUSE` also carries the provider that ended it as `provider`
  and its `reason` (§11.1).
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
- [pinned] An unknown `run_id` is refused `unknown_run`, by `run.result`
  and `run.tool_calls` alike (§6.3).
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

- [pinned] The request is `{run_id?}`, strict: an unknown field is refused
  `invalid_params` naming it, because a misspelled `run_id` would otherwise
  ask a different question (§13).
- [pinned] The answer is `{state, reason?, error?, detail?, undelivered_steers?,
  structured_output?, context_ceiling?, deadline_ms?}`,
  decoded leniently: a runner's own members ride beside these.
  `indeterminate_tool_calls`, `final_step_finish_reason`, `usage` and
  `retries_used` stay runner members, not pinned.
  - `state` is a run state as in `run.result` (`run::states`), decoded open
    (`RunState::parse`): a consumer does not treat an unknown state as
    terminal.
  - `reason`, where the runner names one: why a paused run is paused, why
    an active run is waiting, or why an ended run ended. It is an open
    string; the known pause reasons are `auth_required` and `restart`, and
    the known waiting reason is `context_ceiling_wait` (§6).
  - `deadline_ms`: with `reason: context_ceiling_wait`, when the wait ends
    (§6); absent otherwise.
  - `error`: on a run that ended `error`, the same run error `run.result`
    answers (§6), with the same `provider_code` and `provider_detail_code`
    rules.
  - `detail`: on `budget_exhausted`, exactly the detail `run.result` carries (§6).
  - `structured_output`: exactly what `run.result` carries for the run (§9.4).
  - `context_ceiling`: on `context_ceiling`, exactly the detail `run.result`
    carries (§6).
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
- [pinned] An unknown `run_id` answers `{state: "unknown"}`, never an
  `unknown_run` refusal. `unknown` is a `run.status` state only, not a
  terminal run state.
- [pinned] The session-level form, request `{}`, is optional within
  `run_status`. A runner MAY serve it with a session-level status in its
  own schema; declaring `run_status` does not promise this form.

### 6.2 `run_cancel`: `run.cancel`

A runner that declares `run_cancel` serves `run.cancel`: a caller stops a
run.

- [pinned] The request is `{run_id, at?}`. `at` is `now` by default,
  preserving immediate cancellation. `step_boundary` finishes the current
  model step and that step's entire tool batch, recording the tool results,
  starts no further model step, and ends the run `cancelled`. The session
  remains available for an ordinary `session.send` to continue later.
  The answer is a JSON string, decoded
  open: `ack` or `not_active`.
  - `ack`: the runner accepted the cancellation. The run still reaches
    exactly one terminal state (§6): `cancelled`, or the terminal state it
    reached first. `ack` does not say which; a caller reads it through
    `run.result` or `run.status`.
  - `not_active`: the runner did not cancel the run, because it has
    already ended or the session has no such run. Nothing is written.
  A consumer treats any other answer as not confirming a cancellation.
- [pinned] A caller sends `at` only when the runner's `role.describe`
  lists that value in `run_cancel.at`. Today's runners decode cancellation
  params leniently: they would ignore `at` and cancel immediately, losing
  an in-flight step instead of waiting for its boundary.
- [pinned] A runner that lists `run_cancel.at` refuses an `at` value not
  in its list with `invalid_params {field: "at"}`, changing nothing.
  This includes unknown values; omission still means `now`.
- [pinned] A paused run is cancellable: `run.cancel` MUST accept its
  cancellation with `ack`. It is already at a boundary, so `step_boundary`
  ends it `cancelled` with no further step, just like `now`.
- [pinned] While a `step_boundary` cancellation waits, a second
  `step_boundary` is idempotent and answers `ack`; a second cancellation
  with `now` escalates to immediate cancellation and answers `ack`.
- [pinned] If the current step completes the run anyway, the first
  terminal reached wins, including `completed`, `max_steps` or `error`.
  The existing single-terminal rule applies: `ack` does not say which won.
- [pinned] Owner-only: admitted only from one of the session's owner
  principals. On a scoped session those are two: the daemon-attested
  principal that opened the route which froze the scope (which may be a
  listed carrier), and the frozen scope's `owner`, which already holds
  authority over everything under its scope. On an unscoped session it is
  the attested principal that sent the first episode. The runner records
  them at that first send from the route bind's attested `principal` and
  the scope stamp's `owner`, never from a request field. The check
  compares principals, not routes: an owner is admitted on any route,
  scoped or unscoped, and at any scope epoch, because cleanup cancels
  often arrive after the scope has ended or reopened. Any other caller is
  refused `scope_owner_mismatch`. A local process presenting no consumer
  identity (`direct`) is not an owner, and no operator principal is
  admitted beside them.
- [pinned] A session with no recorded attested owner (one written before
  its runner recorded owners, or first sent by an unverified principal) is
  not owner-protected: a runner MAY admit `run.cancel` and
  `session.retract` from any caller that can reach the session. A consumer
  that needs the protection sends with an attested identity from the
  first episode.
- [pinned] A run a caller cancels ends `cancelled`, never `interrupted`
  (§6).
- [pinned] On an expired session, `run.cancel` answers `expired`, writes
  nothing, and never acts on a lineage started after the expiry (§9.1).

### 6.3 `run_tool_calls`: `run.tool_calls`

A runner that declares `run_tool_calls` serves `run.tool_calls`: every tool
call a run made, as a list, so a consumer (core's citations, say) learns
what a run called without decoding any message body.

- [pinned] The request is `{run_id, after?, limit?}`
  (`tool_calls::ToolCallsRequest`), strict: an unknown field is refused
  `invalid_params` naming it, because a misspelled cursor would answer the
  first page again. An unknown `run_id` is refused `unknown_run`, as
  `run.result` refuses one (§6).
- [pinned] The answer is a page `{run_id, calls, next?}`
  (`tool_calls::ToolCallsPage`), decoded leniently. Each call is
  `{tool_call_id, call_key?, name, args, outcome, result_bytes?,
  dispatched_to?, ordinal}` (`tool_calls::RunToolCall`):
  - `tool_call_id` is the model's id, display only and not unique (§5);
    `call_key` is the call's key, present once the call was prepared or
    dispatched (§11.3); `dispatched_to` is the module the call was
    dispatched to, absent for a call never dispatched (§5);
  - `name` is the model-facing tool name;
  - `args` is the arguments object exactly as dispatched, or, for a call
    never dispatched, exactly as the model asked for it. It is never
    truncated or rewritten;
  - `outcome` is an open string (`tool_calls::outcomes`): `succeeded` (it
    executed and its result is not an error), `failed` (it executed and its
    result is an error), `refused` (refused before it executed: a hook
    denial, a person's decline or an expired approval, a drained route, or
    a tool provider's refusal), `not_executed` (a hard stop's never-executed
    result, §6: `max_steps`, `stop_when`, `budget_exhausted`,
    `context_ceiling`, `skipped_after_submit`, or a run sealed `interrupted`
    with the call never sent), or `indeterminate` (a durable dispatch intent
    and no result, which `session.read` shows as `indeterminate: true` (§5),
    or a call closed `outcome_unknown` after a restart: it may or may not
    have run). A consumer treats an outcome it does not know as not known to
    have succeeded;
  - `result_bytes` is the length in bytes of the result the runner stored
    for the call, in its own stored encoding, absent when none is stored;
  - `ordinal` is the transcript ordinal of the message carrying the call.
    One message can carry several calls, so an ordinal does not identify a
    call.
- [pinned] Calls are in dispatch order: the order the model asked for them
  across the run's steps, which is the order the runner dispatches them. A
  call never dispatched keeps its place. A run's list only grows: once a call
  is listed, its place, `tool_call_id`, `call_key` (once present), `name`,
  `args` and `ordinal` never change; its `outcome`, `result_bytes` and
  `dispatched_to` move forward as the call does.
- [pinned] Paging is `session.read`'s (§4.2), by count and by bytes. A page
  holds at most `limit` calls (absent means the runner's default, and a
  value above its maximum is capped to it; `limit: 0` is refused
  `invalid_params {field: "limit"}`) and stops at the runner's byte cap. It
  carries `next` whenever it stops before the end of the list as it stands;
  the caller passes it back as `after` for the following page. `next` is an
  opaque string: the caller only passes back a value the runner gave for the
  same run, and an `after` the runner never gave for it is refused
  `invalid_params {field: "after"}`. A run still in progress may list more
  calls after a page that carried no `next`.
- [pinned] A single call larger than the byte cap comes alone on its page,
  over the cap, and is never cut short. The complete encoded reply is served
  whole when it fits the 4,194,304-byte transport frame limit, including its
  21-byte header; otherwise the runner refuses `reply_too_large` with
  `subject: {kind: "tool_call", run_id, tool_call_id, call_key?}` (§12.1).
- [pinned] It reads what the run's durable records say, as `run.result`
  does, and writes nothing.

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
- [pinned] A stream event that names why a run is paused or waiting uses
  `reason`, an open string with known values `auth_required`, `restart` and
  `context_ceiling_wait`, as `run.status` does (§6.1). A runner reports a
  wait at the context ceiling with a durable `control` event carrying
  `reason: context_ceiling_wait` and `deadline_ms` when the wait starts.

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
  range does not repeat it. The complete encoded reply is served only when it
  fits the 4,194,304-byte transport frame limit, including its 21-byte header;
  otherwise the runner refuses `reply_too_large`, naming the model-view group
  by its anchored entry source (§12).
- [pinned] An insertion at anchor A travels on the same page as the entry
  anchored at A: the message holding ordinal A or a replacement whose
  `from_ordinal` is A. Several insertions at one anchor travel together, in
  order. The page may exceed its count or byte cap to keep this group whole,
  like the oversize single-message rule; it never ends between an insertion
  and its anchored entry. The complete encoded reply is served whole only
  when it fits the 4,194,304-byte transport frame limit, including its
  21-byte header; otherwise the runner refuses `reply_too_large`, naming the
  group by its anchored entry source (§12).
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
- [pinned] A runner that declares `delivery_order` also accepts an ordered
  list of delivery modes in a separate `delivery_order` member (§9.7). The
  string `delivery` keeps its meaning exactly; a send carrying both is
  refused `invalid_params {field: "delivery_order"}`.
- [pinned] A send that would start or join a run while the session's
  context is at or over its ceiling is refused `context_ceiling {window,
  size, cause}` and writes nothing (§6, §12.1).
- [pinned] Owner sends MUST carry both `send_id` and `prompt`. A
  `delivery_order` send naming a plan template carries an empty `prompt`
  (§9.7), a transitional encoding until `prompt` becomes optional at the
  next breaking release.
  `session.adopt` (§9.5) is not a send: it carries a `send_id` and a plan,
  and no prompt.
- [pinned] One `send_id` namespace covers all three modes, and `delivery` is
  part of the send's identity:
  - the same key with the same payload and mode is answered with the
    existing state, so a retry looks like success;
  - the same key with a different payload or mode is refused `send_id_reuse`,
    with `detail.field` naming the field that differs when the runner can
    identify it.
- [pinned] The owner derives `send_id` from what it delivers, never from the
  attempt. The runner keeps it for the session's lifetime, rebuilt from
  replay.
- [pinned] The reply follows the sync: the runner answers only after the one
  record holding the message, its `send_id` on a keyed send and (for a
  steered or queued prompt) its PreUser output is durable. PreUser runs
  before the write, within the request's deadline. An unavailable PreUser under
  `on_unavailable: refuse` refuses the send `pre_user_unavailable` and writes
  nothing.
- [pinned] A steered prompt written before PreUser existed is never run
  through PreUser later, on resume or replay.
- [pinned] Owner sends use the session's frozen scope. A runner MAY admit
  unkeyed non-owner sends, without retry idempotency; otherwise any other
  caller is refused `scope_owner_mismatch`.
- [pinned] The `delivery` value is strict: an unknown mode is refused naming
  `delivery`, never read as `queue`. Except for the scoped closed `budget`
  rules on an `input_token_cap` runner (§9.3), the strict
  `structured_output` member (§9.4) and the strict `delivery_order` members
  on a `delivery_order` runner (§9.7), the request is lenient: a
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
  naming that parameter, and nothing is written. A joining steer whose
  settings differ MUST be refused `invalid_params`, naming the first
  differing field.
- [pinned] The reply is `{state, run_id?, submission_id?, reason?,
  pause_reason?, baseline?, delivered?}` (`SendReply`), `state` decoded open
  (`send::states`: `active`, `finished`, `pending`, `paused`). A paused
  reply may carry `pause_reason`; it is not renamed to `reason`.
- [pinned] A send behind a paused run MAY be queued, answering `pending`,
  or refused `run_paused`. Both conform.
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
- [pinned] Owner-only, exactly as for `run.cancel` (§6.2): the session's
  owner principals on any route and at any epoch, `scope_owner_mismatch`
  for anyone else, and no protection on a session with no recorded
  attested owner.
- [pinned] A submission is either retracted or started, never both: the
  runner decides from one durable order, and its answer says which.
- [pinned] A re-send of a retracted send, with the same `send_id` and
  payload, is refused `send_retracted` and writes nothing: it never starts
  the send again. A different payload under that `send_id` is refused
  `send_id_reuse` as in §9.
- [pinned] On an expired session, `session.retract` answers `expired` and
  writes nothing (§9.1).

### 9.3 `input_token_cap`: durable session input budget

- [pinned] `session.send.budget.max_input_tokens` (`SendRequest::budget()`,
  `Budget::max_input_tokens`) is optional, a u64 of at least 1. A runner
  advertising `input_token_cap` enforces it over **every model step in every
  episode of the session**: sum each completed step's reported input tokens
  + cache-read tokens + cache-write tokens, all at face value. Output tokens
  do not count. Cache tokens are neither discounted nor deduplicated against
  another reported counter. Both the cap and the running sum are durable
  session state and survive runner restarts. Usage is accumulated even before
  a session first receives a cap.
- [pinned] The Rust `SendRequest` keeps the complete raw `budget` object in
  `runner_params["budget"]`, as before; it has no separate typed budget field.
  Existing consumers reading `nudges` or `finalize_*` through that map retain
  the same data. `budget()` returns a typed copy without changing the raw
  object: `Ok(None)` when absent, or an offending field path when malformed
  (`budget` for a non-object, `budget.max_input_tokens` for an invalid cap).
  `with_budget(Budget)` replaces the existing map entry, so serialization
  emits one `budget` key, never separate raw and typed copies. Validation is
  explicit; decoding a send alone preserves even malformed legacy parameters.
- [pinned] The cap is frozen and lower-only. A later send may lower it or
  repeat it, never raise it. A raise is refused `invalid_params` with
  `detail.field: "budget.max_input_tokens"` and writes nothing. Omitting the
  field preserves the frozen value. A session without a cap is unlimited and
  accepts a cap later (lowering from unlimited). Zero, null, a non-integer,
  a negative number or an out-of-u64 value is refused `invalid_params` naming
  `budget.max_input_tokens`.
- [pinned] Before **each** model step, if consumed input is **≥ limit**, the
  step MUST NOT start and the run ends `budget_exhausted`, `cause:
  "limit_reached"`. A cap of N stops at exactly N, not only above it.
  Immediately after a completed step, **before any of its tool calls
  dispatch**, the runner adds its reported usage and checks again. If the
  sum is **≥ limit**, none of that step's tool calls dispatch. Every one gets
  the same not-executed result used for a hard stop (`max_steps`/`stop_when`),
  with code `budget_exhausted`, atomically with the terminal (§6).
- [pinned] A step that starts under the cap may overshoot within itself.
  Runners MUST NOT pre-estimate input usage to stop such a step. Report the
  measured sum, including the crossing step, as `spent`.
- [pinned] If a completed step's input usage is unreported, fail closed: end
  the run `budget_exhausted` with `cause: "usage_unreported"` before dispatching
  any of that step's calls, giving each the same not-executed result. `spent`
  contains only the measured steps before that step. An unmeasured step is
  never treated as zero. If usage was unreported before a cap was installed,
  the session cannot establish a measured total; the capped run must likewise
  fail closed before starting another step.
- [pinned] Only a runner advertising `input_token_cap` applies these closed
  member rules: `budget` knows exactly `nudges`, `finalize_at_tool_calls`,
  `finalize_text`, and `max_input_tokens`. Each `budget.nudges[]` element knows
  exactly `at_tool_calls` and `text`. Any unknown member is refused
  `invalid_params` naming its full path, such as `budget.foo` or
  `budget.nudges[0].foo`, without writes. This crate preserves the existing
  runner-specific values of the three older budget members verbatim; only
  their member names are pinned here. Non-advertising runners retain their
  existing budget behavior unchanged.

### 9.4 `structured_output`: schema-shaped run output

A caller asks a run for a JSON value shaped by a JSON Schema, and reads back
what the run produced and whether it validated.

**The request.**

- [pinned] `session.send` may carry `structured_output: {mode, schema,
  name?}` (`structured_output::StructuredOutput`). `mode` is `final_text` or
  `submit_tool`. `schema` is a JSON Schema object. `name` is optional, 1 to 48
  of `[a-z0-9_]`, and is used where a provider needs a schema name; absent
  means `result`. The member is strict: an unknown member, an unknown or
  missing `mode`, a malformed `name` (including `null`) and a missing or
  non-object `schema` are each refused `invalid_params` naming the member's
  path (`structured_output.mode`, `structured_output.name`,
  `structured_output.schema`, `structured_output.<member>`), and a
  non-object `structured_output` names `structured_output`. A refused send
  writes nothing.
- [pinned] The Rust `SendRequest` keeps the member in
  `runner_params["structured_output"]`, exactly as received; it has no typed
  field. `SendRequest::structured_output()` returns a typed copy, `Ok(None)`
  when absent, or the offending field path. `with_structured_output(...)`
  replaces that map entry, so one `structured_output` key is serialized.
- [pinned] **Per run, frozen at admission.** The runner records the RFC 8785
  (JCS) bytes of the schema (`StructuredOutput::schema_jcs`), with the mode,
  name, enforcement and reason, on the record that starts the run, and
  renders every request of the run from those bytes, through resume and
  retry. A send without `structured_output` starts a run with none; it does
  not inherit the previous run's. A send that joins a running run (`steer`)
  runs under that run's frozen request; one carrying a different
  `structured_output` is refused `invalid_params` naming `structured_output`.
  `structured_output` is part of a keyed send's payload: the same `send_id`
  with a different value is refused `send_id_reuse` naming it.

**What a runner refuses at admission.** Each is `invalid_params` and writes
nothing. `SendRequest::structured_output()` checks the value itself, then
`StructuredOutput::check(advertised, tools)` the rest, in this order:

- [pinned] The runner does not advertise `structured_output`: field
  `structured_output`.
- [pinned] **No remote fetches; a bounded schema.** A `$ref` or `$dynamicRef`
  that points outside the schema document is refused naming
  `structured_output.schema`. Runners never fetch a schema over the network
  or from files. A reference is inside the document when it starts with `#`,
  or when its part before any `#` equals exactly the `$id` (an empty trailing
  `#` removed) of the root or of a subschema in a draft 2020-12 schema
  position of the same document. Anything else is external, including a
  relative reference that would resolve only against a base URI, and an `$id`
  inside a keyword draft 2020-12 does not define never makes a reference
  internal. Values of `const`, `enum`, `default` and `examples` are data, not
  references (`structured_output::check_schema`).
- [pinned] The JCS bytes of the schema are at most 65,536
  (`MAX_SCHEMA_JCS_BYTES`); a larger schema is refused naming
  `structured_output.schema`.
- [pinned] A `$schema` other than
  `https://json-schema.org/draft/2020-12/schema` is refused naming
  `structured_output.schema`.
- [pinned] A schema the runner cannot compile is refused naming
  `structured_output.schema` at admission, never discovered at run end.
- [pinned] `submit_result` is reserved: a run whose admitted tool set (the
  plan's tools and the runner's own) includes a tool of that name is refused
  naming `structured_output`, in either mode. Callers cannot choose another
  name.
- [pinned] `final_text` on a run whose admitted tool set is non-empty is
  refused naming `structured_output.mode`: providers that constrain every
  reply would break tool calls. Use `submit_tool` there.

**Validator semantics.**

- [pinned] The runner validates with JSON Schema draft 2020-12, whatever the
  enforcement. `format` is an annotation and is never asserted. Unknown
  keywords are ignored.

**Mode `final_text`.**

- [pinned] The runner asks the provider to constrain the final assistant text
  to the schema where the route supports it. When the run completes, the
  final text (joined as `run.result` joins it, §6) is parsed as JSON and
  validated.

**Mode `submit_tool`.**

- [pinned] The runner adds one tool, `submit_result`
  (`SUBMIT_RESULT_TOOL`), whose input schema is the caller's schema and whose
  description says calling it ends the task. It is in the run's tool list
  from its first request, so the prompt prefix never changes mid-run.
- [pinned] **When the model calls it, the run ends** `completed` after that
  step. With `submit_tool` active, no call of a step that contains
  `submit_result` is dispatched before the step completes, so a sibling the
  model emitted before `submit_result` is not dispatched either. Every other
  call of that step gets the never-executed result of a hard stop (§6), with
  refusal code `skipped_after_submit`, in the same durable commit as the
  run's end, and is listed in `skipped_siblings`. The counted `submit_result`
  call gets the runner's own recorded result in that commit, so no call
  dangles.
- [pinned] If the model calls `submit_result` more than once in one step, the
  first call counts; the others are skipped siblings.
- [pinned] A run that ends any other way (`max_steps`, `stop_when`,
  `budget_exhausted`, `cancelled`, `error`, `interrupted`, or the model
  finishing without calling it) has no submitted value: `validation:
  "absent"`.
- [pinned] The runner does not force the tool. Whether forcing works beside
  extended thinking differs by provider; a later minor may add an opt-in.

**Enforcement.**

- [pinned] `enforcement` and `reason` are fixed for the run at admission and
  never change mid-run.
- [pinned] `enforced` means the route supports schema-constrained output for
  the requested mode and the schema fits that route's supported subset.
  Anything else is `best_effort`: the runner still sends the schema (as the
  tool's input schema, or in instructions) and still validates, but the
  provider gave no guarantee.
- [pinned] A runner decides the subset question from its own per-family
  subset profile at admission. It never sends a schema in a provider's strict
  mode when the schema is outside that route's supported subset (some
  providers reject that only inside a stream that already answered 200); it
  sends such a schema non-strictly and reports `best_effort` with
  `schema_outside_subset`. Such a schema is never refused for that reason,
  and the reason is never discovered from a provider error.
- [pinned] `reason` is present only on `best_effort`. For runners it is a
  closed set (`structured_output::reasons`): `catalog_unknown` (the model
  catalog does not say whether the route supports schemas; Fusiform's per-row
  `capability.structured_output`, where absent means unknown, is that
  source), `route_unsupported` (the catalog says it does not),
  `schema_outside_subset`, and `mode_not_enforceable` (the route enforces
  schemas for final text but not for tool input, or the reverse). Consumers
  decode it as an open string. Where a route's support comes from, and each
  family's subset profile, is the runner's business, not this contract's.

**The result.**

- [pinned] `run.result` and `run.status` of an ended run admitted with
  structured output carry `structured_output: {mode, enforcement, reason?,
  validation, value?, errors?, errors_truncated?, skipped_siblings?}`
  (`StructuredOutputResult`), the same on both. It is absent on a run that
  has not ended and on a run admitted without it. Members that do not apply
  are omitted, never empty or zero; all string members decode open.
- [pinned] `validation` is `passed` (`value` holds the parsed JSON),
  `failed` (`value` holds what the model produced if it parsed as JSON, and
  `errors` lists messages naming the JSON Pointer of each failing instance
  location), or `absent` (no value, no errors). The run state stays
  `completed` on `failed`; the caller decides.
- [pinned] **A bounded result.** `errors` has at most 32 entries
  (`MAX_ERRORS`) of at most 256 bytes each (`MAX_ERROR_BYTES`), an entry cut
  at a UTF-8 character boundary. `errors_truncated: true` is present when an
  entry was dropped or shortened (`structured_output::bound_errors`) and
  omitted otherwise. `value` is never cut short: it counts toward the reply's
  frame limit, and an answer that exceeds it is refused `reply_too_large`
  with `subject: {kind: "run", run_id}` (§12.1).
- [pinned] `skipped_siblings: [{tool_call_id, tool}]` is present on every
  `submit_tool` result, in the order the model emitted the calls, and `[]`
  when `submit_result` came alone or was never called. It is absent on
  `final_text`.
- [pinned] A result that breaks one of these member rules is malformed, not
  a refusal (`StructuredOutputResult::well_formed` names the rule).

**Caching.**

- [pinned] A schema is part of the request and sits ahead of the input in a
  provider's cached prefix, on every provider family measured: an identical
  schema keeps the cached prefix, while one added field or a dropped schema
  loses all of it. A caller that needs more than one shape in a session
  declares one schema covering them (a union keyed by a required
  discriminator field). Once a session has used a schema, the caller sends
  the same bytes on every send in that session, including sends that need no
  shape, because dropping the schema also changes the request.

### 9.5 `adopt`: `session.adopt`

A harness runner serves sessions a person starts in their own app. The host
fixes the tool set and system text when it starts, so the runner applies the
installation's fetch plan before the first turn. The session's owner must
then take the session over, but it has no prompt to send, and owner sends
must carry one (§9). `session.adopt` makes the caller the session's owner
without a prompt.

- [pinned] A runner that declares `adopt` serves `session.adopt`. A caller
  MUST NOT send it unless the runner's `role.describe` declares `adopt`; a
  runner without the group is unaffected. The group adds no discovery
  metadata and is independent of `plans` and of the delivery modes.
- [pinned] The request is `{send_id, plan}` (`adopt::AdoptRequest`),
  strict: an unknown field is refused `invalid_params` naming it, and a
  `plan` that is not an object is refused naming `plan`. The request names no
  session and carries no `session_ref`: the session is the one the route is bound to
  (§1). `plan` is the full fetch-plan object, the same object
  `SendRequest::plan` carries (§10.1), never a digest of it.
- [pinned] `session.adopt` is admitted from exactly the callers an owner's
  keyed `session.send` is admitted from (§9), with the owner principals of
  §6.2: the runner records them from the route bind's attested principal
  and the scope stamp's owner, never from a request field, as it would at
  the session's first owner send. Once the session has a recorded owner, any
  other caller is refused `scope_owner_mismatch`, as §9 refuses it. A caller
  that could not be recorded as an owner (a `direct` caller, §6.2) is
  refused `scope_owner_mismatch`: adopt is never an unkeyed non-owner send.
- [pinned] Effect: the runner records the caller as the session's owner and
  binds the session's frozen scope to the plan, exactly as owner admission
  records a plan (§10.1), so `session.baseline` answers the owner afterwards
  as it would after that admission. It starts no run and writes no message.
  It does not interrupt, wait for or refuse a run already in progress: the
  person may be mid-turn in their app, and that run goes on as before.
- [pinned] Drift: the runner computes the identity of the request's plan,
  the same identity admission uses for `plan_drift` (§10.1), and compares
  it with the identity of the plan it applied when the host started the
  session. A mismatch is refused `plan_drift {frozen, sent}`, the refusal
  and detail of §10.1 (`errors::PlanDriftDetail`, `adopt::check_drift`):
  `frozen` is the identity of the plan applied at host start and `sent` the
  identity of the request's plan, identities only, never plan content.
  `frozen` is absent when the runner applied no plan to the session (the
  host never started it, or started it without one): adopt never
  establishes a plan, it only confirms the one applied. `plan_drift` is
  terminal and not retried with the same request.
- [pinned] Idempotency is keyed by the bound session and `send_id`, and
  compares the whole request, as owner sends do (§9). An exact repeat is
  answered with the original reply, unchanged, even after the transcript
  has grown. The same `send_id` with another plan is refused
  `send_id_reuse` with `detail.field: "plan"`
  (`AdoptRequest::reused_field`). The owner derives `send_id` from the
  session and plan it adopts, so a retry after a lost reply carries the same
  `send_id` and replays; it never derives it from the attempt (a counter or a
  time), and never reuses a `send_id` of its `session.send`s.
- [pinned] Already owned: the session's owner adopting again with the same
  plan, under a new `send_id`, succeeds as an adoption does, with the same
  `applied_plan_digest`. Adopt never takes over a session another owner
  holds.
- [pinned] The runner checks a request in this order, and the first
  failing check answers: the caller (`scope_owner_mismatch`), then the
  `send_id` (a replay, or `send_id_reuse`), then the plan (`plan_drift`).
  So another caller repeating the owner's request exactly is still refused
  `scope_owner_mismatch`.
- [pinned] A refusal writes nothing, its `send_id` and caller included: a
  later request may use the same `send_id`, and a refused caller did not
  become the owner.
- [pinned] The reply is `{owner_bound, applied_plan_digest, last_ordinal?}`
  (`adopt::AdoptReply`), decoded leniently. `owner_bound` is `true`.
  `applied_plan_digest` is the identity of the plan the session is bound to.
  `last_ordinal` is the session's last committed ordinal when it was
  adopted, so the caller knows where the transcript stands; it is absent
  when the session has no message yet, as in `session.head` (§4.3).
- [pinned] The reply follows the sync: the runner answers only after the
  record of the adoption (the owner, the plan's identity, the `send_id` and
  the reply) is durable. That record is not a transcript message and takes
  no ordinal.
- [pinned] After adoption, the owner's sends follow §9 unchanged, and the
  owner reads `session.baseline` as §10 describes.
- [pinned] `session.adopt` is not scope adoption: `scope_adoption_refused`
  (§12.1) concerns a scoped send into a session recorded without a scope,
  and is unchanged.

### 9.6 `tags`: runner-owned part tags

The runner owns the transcript, so it owns short, stable addresses for its
parts: tags. A consumer (a context manager, a model referring to an earlier
message, a tool using part of an earlier output) names a part by its tag, and the
runner answers with the exact stored content.

**Assignment.**

- [pinned] A runner that declares `tags` gives every part it exposes a
  stable key (`part`), and a tag to every part it tags. It tags at least
  every part that carries text: user text, a tool result's text and the
  model's text. It may tag others (a tool call, a reasoning block); a part
  with nothing to address by text (an image, say) may stay untagged. A tag is
  `§N§` (`tags::SIGIL`), `N` a positive decimal integer without a leading
  zero that fits a `u64` (`tags::parse_tag`, `tags::format_tag`). That one
  spelling is the tag: no other spelling names it.
- [pinned] Tags are per session, durable and never reused: not across a
  fork, a reset or a restart. A fork continues numbering after its parent's
  highest tag (`tags::next_after`), and numbers are assigned in increasing
  order.
- [pinned] A part is tagged at write time: before the runner stores it, or,
  for a part its host stored itself (in-process hosts store the model's
  text before any hook sees it), at the runner's first ingest of that part.
  Both readings conform. Either way the tag is recorded durably in the
  runner's ledger, keyed by (message, part), and every later read and
  resolve answers it byte-identical.
- [pinned] Once a part is exposed, its key never changes, tagged or not;
  once assigned, its tag and its stored content never change. A compaction
  provider addresses part-level view edits by that key.

**What the model writes.**

- [pinned] A runner stores what the model wrote byte for byte. A `§N§` the
  model writes is never a tag, wherever it appears, and is never rewritten
  in storage: `session.read` and `session.resolve` answer it as written.
- [pinned] In a signed thinking block, in encrypted reasoning, or in a tool
  call's arguments, a model-written `§N§` stays as written in every
  rendering too: editing it would break the signature or change what the
  tool was asked. A tool call's arguments are dispatched exactly as the
  model wrote them.
- [pinned] Correcting a model-written `§N§` in the model's text (so the
  model does not later read its own invention as a tag) is allowed only in
  the rendering, and only where two conditions hold. First, the provider's
  text part carries no provider state (an Anthropic text block, say). Where
  a text part carries provider state (a thought signature, ChatGPT's
  `phase`, encrypted reasoning beside it), the assistant's tags are
  metadata only and a stray `§N§` is rendered as written. Second, the
  corrected rendering is fixed at the runner's first render of that message
  and never changes afterwards, so every later request carries the same
  bytes. This is the runner's provider duty: it applies the rule per
  provider and model, and a consumer never sees the difference in a read.

**Reads.**

- [pinned] On a runner that declares `tags`, every `session.read` entry
  carries `tags: [{part, tag?}]` (`tags::PartTag`), one row per part the
  runner exposes, in the message's part order, beside the message as
  `tool_calls` are: `tag` is present on a tagged part and absent on an
  untagged one, so a reader learns every part's key, tagged or not, from
  the rows alone. `part` is the runner's own key for the part within its
  message schema (an index or a part id), opaque to consumers: the key its
  ledger records the tag under and the one `session.resolve` answers with.
  Decoding stays lenient: a reader of a runner without `tags` finds no
  `tags`. Where a message came from (its attestation) is not in this
  revision.
- [pinned] No digest rides on an entry: a reader computes one from the
  bytes it reads.

**Resolve.**

- [pinned] `session.resolve {tag, range?}` (`tags::ResolveRequest`) is
  strict: an unknown field is refused `invalid_params` naming it, and the
  request names no session (§1). `range` is `{start, end}`
  (`tags::CharRange`), a half-open range of the part's content counted in
  Unicode scalar values: not bytes, and not UTF-16 code units, so a range
  names the same text in every language.
- [pinned] The answer is `{tag, ordinal, mid, part, content, range?}`
  (`tags::ResolveReply`), decoded leniently: `content` is the
  part's exact stored content, or the requested range of it
  (`tags::slice`). For a text part that is its text; for any other part
  (a tool call, a reasoning block, a structured result) it is the runner's
  own stored encoding of the part, the same bytes on every resolve. `range`
  echoes the request's.
- [pinned] A malformed tag is refused `invalid_params {field: "tag"}`; a
  well-formed tag the session never assigned is refused `unknown_tag`,
  terminal, since tags are never reused (§12.1). A range whose `start` is
  after its `end`, or whose `end` is past the content, is refused
  `invalid_params {field: "range"}`; an empty range within the content
  answers `""`.
- [pinned] Resolve writes nothing.

**Visibility.**

- [pinned] With `render_tags: true`, the runner renders a tagged part's tag
  as an exact prefix: the tag, one space (U+0020), then the part's text
  (`tags::render_text`, `tags::RENDER_SEPARATOR`). It does so for user
  text, for a tool result's first text block, and for the model's text only
  where the provider's text part carries no provider state and the
  rendering is fixed at the first render (above); assistant text that
  carries provider state renders as stored, its tag metadata only. A part
  with no text (an image, a tool call, a reasoning or thinking block), an
  empty text part and an untagged part carry no prefix. A compaction
  provider builds the same bytes for a drop placeholder; `tags.json` pins
  rendered parts.

- [pinned] Whether tags are rendered to the model is the session's fetch
  plan's `render_tags` member (`tags::PLAN_RENDER_TAGS`), composed by core
  in the same plan object `SendRequest::plan` carries (§10.1): `true`
  renders each part's tag; absent, `null` or `false` renders none. A plan
  carrying a non-boolean, or `render_tags: true` to a runner without `tags`,
  is refused at admission `invalid_params {field: "plan.render_tags"}`,
  writing nothing (`tags::check_plan`, `SendRequest::check_render_tags`).
- [pinned] Tags exist either way: a runner that declares `tags` assigns,
  records, reads and resolves them whether or not the plan renders them.
  `render_tags` is part of the plan, so it is frozen with it, and a later
  plan that differs in it is `plan_drift` like any other difference.

### 9.7 `delivery_order`: ordered delivery modes, plan senders and templates

A send does not always want to become a turn. A module's reminder should
ride the next request the session makes anyway, never start one; text a
plan vouches for may go out with system authority. `delivery_order` lets
a send say, in order, how it may be delivered, and lets the session's plan
say who may send and which texts are system text.

**The list.**

- [pinned] A runner that declares `delivery_order` accepts an optional
  `delivery_order: [mode, ...]` on `session.send`
  (`delivery_order::DeliveryOrder`, read with
  `SendRequest::delivery_order`), with modes (`delivery_order::Mode`):
  - `tool_result`: rendered with the next tool result, the first time that
    result is sent;
  - `turn_system`: a turn-scoped system message (Anthropic's `clear_at:
    next_user_message`), for template text only (below);
  - `steer`: a user message into the run in progress, at its next step;
  - `idle_start`: a user message that starts a run on an idle session.
- [pinned] The list is strict: not an array of these modes, empty, or a
  mode twice is refused `invalid_params {field: "delivery_order"}`, writing
  nothing.
- [pinned] At each request boundary, the runner takes the first mode in the
  list that is possible there (`delivery_order::choose`,
  `delivery_order::Boundary`): `tool_result` needs a tool result about to be
  sent for the first time; `turn_system` needs a provider that supports it,
  a runner that owns the request, and the authority rule below; `steer`
  needs a run in progress; `idle_start` needs an idle session. When none is
  possible, the send stays queued, durably, for the next boundary.
- [pinned] **A list without `idle_start` never starts a run.** It waits,
  queued, for a request the session makes anyway.
- [pinned] The legacy string `delivery` is unchanged, and maps onto lists
  (`delivery_order::legacy_equivalent`): `queue`, or an absent `delivery`,
  is `[idle_start]` (it waits durably and starts the next turn once the run
  ends); `steer` is `[steer, idle_start]`. `interrupt` has no list form and
  stays its own mode. A send carrying both `delivery` and `delivery_order`
  is refused `invalid_params {field: "delivery_order"}`, so a caller never
  has two answers.
- [pinned] A runner of this revision that does not declare the group
  refuses a send carrying `delivery_order`, `dedupe_key`, `expires_at_ms`
  or `template` with `invalid_params` naming that member, writing nothing
  (`SendRequest::check_delivery_order`): it never delivers such a send
  another way. A caller still checks discovery first (§3).
- [pinned] The list, with its members, is part of the send's identity
  (§9): the same `send_id` with another list is refused `send_id_reuse`
  naming `delivery_order`. A retry with the same `send_id` is the same send
  and answers its current state. Idempotency is keyed by the sender and
  `send_id`, so two senders' `send_id`s never collide.

**Dedupe and expiry.**

- [pinned] `dedupe_key?` (a non-empty string) replaces a queued,
  undelivered send from the same sender carrying the same key
  (`delivery_order::replaces`); a delivered send, another sender's or
  another key's is never replaced. `dedupe_key` is not `send_id`: a retry
  under the same `send_id` is the same send, never a replacement.
- [pinned] A replaced send ends in the terminal send state `superseded`
  (`send::states::SUPERSEDED`), and is never delivered. A caller observes it
  by re-sending its `send_id`, which answers `{state: "superseded"}`.
- [pinned] `expires_at_ms?` (milliseconds since the Unix epoch) drops a
  send still undelivered at that time, durably, in the terminal send state
  `expired` (`send::states::EXPIRED`, `delivery_order::expired`); it is then
  never delivered, and a re-send of its `send_id` answers `{state:
  "expired"}`. A send whose time has passed when it arrives is dropped at
  its first boundary.
- [pinned] Until it is delivered, a queued send answers `pending` with its
  `submission_id`, as a queued send does today (§9).

**What is written.**

- [pinned] A delivered send is a transcript entry of its own, carrying
  `delivered_as: {mode, call_key?}` (`delivery_order::EntryDelivery`) in its
  envelope (§4.2): the mode it went out in, and for `tool_result` the
  `call_key` of the tool result it went out with. Readers and a resumed run
  render exactly what was sent from that entry. Once delivered, it is resent
  byte-identical on every later request.
- [pinned] `tool_result` never changes the stored tool result. The runner
  records that send S was delivered with result R, and renders R followed
  by S's text from that record, so a resume renders the same bytes.

**Text and authority.**

- [pinned] A send has one text, whatever mode carries it: its `prompt`, or,
  for a send naming a plan template, the text the runner renders from that
  template and the send's slots. A template send carries `template: {id,
  digest, slots}` (`delivery_order::TemplateUse`, strict) and an empty
  `prompt`; a template beside a non-empty `prompt`, or a `delivery_order`
  send with an empty `prompt` and no template, is refused `invalid_params
  {field: "template"}` (`SendRequest::delivery_order`). The empty `prompt` is
  a transitional encoding: `prompt` becomes optional at the next breaking
  release.
- [pinned] The runner renders the template from the plan's template for
  that sender and the send's slots, validating every slot itself
  (`delivery_order::render`); the sender's word is never enough. A send
  whose template cannot render has no text, so it is refused at admission
  `invalid_params {field: "template"}`, writing nothing
  (`delivery_order::TemplateProblem`): the sender's plan entry declares no
  template with that id, its digest differs, a slot is missing or
  undeclared, or a value is not a value of its slot (the wrong type, out of
  range, an unassigned tag, an unlisted enum value).
- [pinned] `tool_result`, `steer` and `idle_start` deliver at user
  authority. `turn_system` is possible only for a rendered template, and
  only where the provider supports a turn-scoped system message and the
  runner owns the request. Where it is not, the runner falls through to the
  next mode in the list, carrying the same rendered text at user authority.
  A send without a template never goes out as `turn_system`.
- [pinned] A template is `{id, text, slots}` (`delivery_order::Template`):
  `text` holds `{name}` placeholders, `name` in `[a-z0-9_]+`, and no other
  brace; every placeholder names a declared slot and every slot is used. Its
  digest is lowercase hex SHA-256 over its RFC 8785 (JCS) bytes
  (`delivery_order::template_digest`). Slots are typed
  (`delivery_order::Slot`), and none takes free text: `integer {min, max}`
  (inclusive), `percentage` (an integer from 0 to 100), `tag` (a tag this
  session assigned, §9.6) and `enum {values}` (one of the listed values).
  Integers render in decimal; a tag and an enum value render as themselves.

**Senders.**

- [pinned] The plan's optional `senders: [{module, idle_start?,
  templates?}]` (`delivery_order::PlanSender`) names the modules other than
  the owner whose sends the session admits, and the templates each may use.
  It is malformed (a module twice, a template id twice within a sender, a
  placeholder without a slot or a slot without a placeholder, an integer
  slot with `min` over `max`, an enum with no value, an unknown member or
  slot type) or present on a runner without the group, and is then refused
  at admission `invalid_params {field: "plan.senders"}`, writing nothing
  (`delivery_order::check_plan`). The owner's own templates, when it has
  any, are an entry naming the owner.
- [pinned] On a runner that declares `delivery_order`, a send from anyone
  but the owner is admitted only when the plan names the sender, matched on
  the route's attested principal, never on a request field
  (`delivery_order::admit`); any other caller is refused
  `scope_owner_mismatch`, writing nothing. This replaces §9's permission to
  admit unkeyed non-owner sends on such a runner.
- [pinned] A named sender may list `idle_start`, and so start a run, only
  when its entry grants `idle_start: true`; otherwise its list (or its
  legacy `queue` or `steer`, which stand for lists holding `idle_start`) is
  refused `invalid_params {field: "delivery_order"}`. `interrupt` stays the
  owner's.

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
  `session.adopt` refuses a plan other than the one applied at host start
  with the same `plan_drift {frozen, sent}` (§9.5).
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
- [pinned] The plan's optional `render_tags` member decides whether part
  tags are rendered to the model (§9.6); absent means off. Its optional
  `senders` member names the session's other senders and their templates
  (§9.7); absent means the owner alone.

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
| `invalid_params` | a request field is malformed, unknown, or combined with one it excludes; a plan's `compaction_item` sent to a runner without `compaction`; a plan's `render_tags` that is not a boolean, or `true` without `tags`; a plan's malformed `senders`, or `senders` without `delivery_order`; a send's `delivery_order` beside a legacy `delivery`, a malformed list or member, a member sent to a runner without the group, or an `idle_start` its sender's grant excludes (§9.7); a malformed tag or an out-of-range `range` on `session.resolve`; an `after` cursor never given, or `limit: 0`, on `run.tool_calls` | `field` | no |
| `lineage_changed` | a read names a lineage that is not the session's | — | no: re-read from the tail |
| `unknown_mid` | `after_mid` names no message in the lineage | — | no |
| `reply_too_large` | the complete encoded `session.read` reply, a `run.result` or `run.status` answer carrying a structured-output `value` (§9.4), or a `run.tool_calls` page holding one call (§6.3), exceeds the 4,194,304-byte transport frame limit, including its 21-byte header | `{bytes, limit, subject}`; `bytes` is the whole encoded success frame length, and `limit` is `4194304`; `subject` is `{kind: "message", session, ordinal}`, `{kind: "model_group", source}` (the model-view anchored entry's existing `EntrySource`), `{kind: "run", run_id}` or `{kind: "tool_call", run_id, tool_call_id, call_key?}`; no message content, value or arguments | no: terminal for this request; lowering `max_bytes` or `limit` cannot shrink a message, atomic group or call |
| `unknown_run` | `run.result` or `run.tool_calls` names no run of the session | — | no |
| `unknown_tag` | `session.resolve` names a well-formed tag the session never assigned (§9.6) | — | no |
| `context_ceiling` | a `session.send` that would start or join a run while the session's context is at or over its ceiling (§6); never a model or provider failure | `{window, size, cause}` (`errors::ContextCeilingDetail`): the window and the request's estimated size in tokens, and `cause`, an open string: `view_timeout` (the runner waited the compaction provider's bound and no view came), `provider_unavailable` (no compaction provider at setup, or it is unreachable at the ceiling) or `over_window` (the request cannot fit even after the current view) | no; a router charges no backoff to the model for it |
| `expired` | a read of an expired session (§9.1), including `run.status`, `run.cancel` and `session.retract` where their groups are declared | `expired_at_ms` (Unix epoch milliseconds) | no |
| `send_id_reuse` | a `send_id` reused with another payload or mode (another `delivery_order` list included, §9.7), or a `session.adopt` `send_id` reused with another plan (§9.5) | `field` when the runner can identify it; `plan` on `session.adopt` | no |
| `send_retracted` | a re-send of a send whose submission `session.retract` withdrew (§9.2) | — | no |
| `delivery_unsupported` | a send whose delivery mode the runner does not declare | `delivery` | no |
| `run_paused` | a send behind a paused run that the runner refuses rather than queues (§9) | `{run_id, reason}` | no |
| `scope_owner_mismatch` | an owner-only call from anyone but the owner: `session.send` (except an admitted unkeyed non-owner send, §9, or, on a `delivery_order` runner, a send from a sender the plan names, §9.7), `session.baseline`, `run.cancel`, `session.retract` or `session.adopt` (§9.5) | the accepted identity | no |
| `scope_adoption_refused` | a scoped send into a session recorded without a scope that cannot adopt it | `no_recorded_principal` or `principal_mismatch {recorded}` | no |
| `scope_unsupported` | a send under a scope on a daemon without scopes | — | no |
| `fetch_unavailable` | a required plan item missed the admission deadline | `provider` | **yes** |
| `plan_stale` | fetched items disagree with the composition | `differences` | no: re-plan |
| `tool_name_collision` | tools share a model-facing name | `name`, `providers` (every provider that offered it) | no |
| `plan_drift` | a later send's plan differs from the frozen one; a `session.adopt` plan differs from the one applied at host start (§9.5) | `frozen?`, `sent` (plan identities; no `frozen` after a plan-less first episode, or on `session.adopt` when no plan was applied) | no |
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
- [pinned] `send_retracted` is in `errors::CODES` and is not retryable.
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
| `context_overflow` | the provider refused the request as exceeding the model's context window. Not the ceiling: a stop the runner decides itself before sending is the `context_ceiling` run state or refusal (§6, §12.1), never this code |
| `output_ceiling` | the provider's output-token ceiling prevents a complete response |
| `response_deadline` | the provider's response deadline ended the request |
| `account_limit_exhausted` | the provider account's usage limit is exhausted |
| `billing_exhausted` | the provider account's billing or spending allowance is exhausted |
| `account_changed` | the provider served an account different from the run's frozen account |
| `cyber_policy` | a provider content-policy refusal |
| `bio_policy` | a provider content-policy refusal |
| `misalignment_policy_violation` | a provider content-policy refusal |
| `content_filter` | the normalized safety finish, treated as a provider policy refusal |
| `access_terminated_error` | the provider terminated the account's access; the seat is unable to run |
| `insufficient_quota` | a legacy alias of `billing_exhausted`, for already recorded run errors only |

A compaction `REFUSE` adds the compaction-provider contract's `RefuseCode`
values (its §11) as `provider_code`; this role does not repeat them. Their
retryability, and that of the two compaction codes above, is the
compaction-provider contract's.

- [pinned] A consumer MUST treat `cyber_policy`, `bio_policy`,
  `misalignment_policy_violation` and `content_filter` as provider policy
  refusals. `content_filter` is the normalized safety finish.
- [pinned] A consumer MUST treat a seat whose provider reports
  `access_terminated_error` as unable to run. The broca runner normalizes a
  Kimi usage-limit message under that type to `account_limit_exhausted`,
  rather than reporting access termination.
- [pinned] Runners MUST stop emitting `insufficient_quota`. Already
  recorded run errors may still carry it; consumers MUST treat it as
  `billing_exhausted`. broca emits `billing_exhausted` from 0.3.193 on.

These are run errors, not request-refusal codes. Plan-fetch refusals, such
as `plan_param_refused`, are send refusals and do not belong in this list.
`errors::provider_codes::CODES` in the role crate still lists only the
three compaction and PreUser codes above; it must add the newly pinned
provider codes.

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
- [pinned] `skipped_after_submit`: the call shared a step with a counted
  `submit_result` call, which ended the run before any call of that step was
  dispatched (§9.4). It is marked not executed, with no dispatch intent, using
  the same native result as other hard stops (§6).
- [pinned] `budget_exhausted`: the session input cap stopped the call before
  dispatch (§9.3). It is marked not executed, with no dispatch intent, using
  the same native result as other hard stops (§6).
- [pinned] `context_ceiling`: the session's context reached its ceiling at
  a step boundary, which ended the run `context_ceiling` before the call was
  dispatched (§6). It is marked not executed, with no dispatch intent, using
  the same native result as other hard stops.
- [pinned] Where the reason sits in a message body is the runner's message
  schema.

## 13. Decoding

- [pinned] Every field is declared required or optional.
- [pinned] **Lenient** (unknown fields ignored; enumerations decoded as open
  strings): `role.describe`, every reply and event a consumer decodes
  (`ReadPage`, `ModelPage`, `HeadMeta`, `RunResult`, `SubscribeEvent`,
  `SendReply`, `BaselineReply`, `AdoptReply`, `ResolveReply`,
  `ToolCallsPage`, and the `run.status`, `run.cancel` and `session.retract`
  answers), and `compaction.ready`. A run call's `outcome`, a ceiling's
  `cause` and a read entry's `delivered_as.mode` are open strings. One
  exception: a model-view entry's `source` kind is strict (§8).
- [pinned] **Strict on unknown fields** wherever absence changes the
  question: `session.read`, `session.head`, `run.result`, `run.status`,
  `run.tool_calls`, `session.subscribe`, `session.baseline`,
  `session.adopt` and `session.resolve` requests. A
  runner refuses an unknown field with `invalid_params` naming it.
- [pinned] **Lenient on fields, strict on values** where the request grows:
  `session.send` tolerates unknown fields (a newer owner against an older
  runner degrades rather than fails), but `delivery` refuses an unknown value
  naming the field. `retention` is a recognized field even on a runner without
  its group, and is refused there (§9.1), never treated as an unknown extra.
  `view` on `session.read` is strict the same way, and so are the
  `delivery_order` members on a runner that declares the group (§9.7).
  `run.cancel` remains lenient on fields, but a runner that advertises
  `run_cancel.at` refuses unlisted `at` values (§6.2).
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
   Dropping or rewriting a written message starts a new lineage, whether or
   not a reader saw the message (the runner cannot know), so read stability
   (§4.2) holds without exception: reads of the old lineage are refused
   `lineage_changed`, and a reader resyncs from the new one.
5. [pinned] **Replay, not re-invocation.** Applied compaction and every hook
   output replay from durable state without calling the provider again. A
   step whose hook outputs never became durable re-runs its hooks; nothing
   they would change was sent.
6. [pinned] **Keyed sends are idempotent across a kill.** A keyed send
   whose record was durable when the runner died is answered with the
   existing state on retry, and nothing is written twice.
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

## 14.1 Session reference

- [pinned] A session's `session_ref` is the first 16 bytes of SHA-256 over
  the RFC 8785 (JCS) bytes of `{"harness", "project_root", "session"}`, the
  bind identity exactly as the route bind carried it. It is written as 32
  lowercase hexadecimal characters.
- [pinned] `project_root` is not normalised: no realpath, no symlink
  resolution, no trailing-slash removal. Two binds whose roots differ by a
  single byte are different sessions, because a runner keys its durable state
  by those same bytes.
- [pinned] Any module that keeps state for a runner session (a compaction
  provider, a backup or move participant) derives the same `session_ref`
  from the same bind identity, without asking the runner.
  `test-vectors/llm-runner-v1/session-ref.json` pins the derivation,
  including a trailing slash and non-ASCII escaping, and the crate's
  `session_ref` function implements it.
- [pinned] `session_ref` is a correlation key, not proof of identity: a
  direct caller chooses its own bind fields. Authorization still rests on
  the daemon-stamped principal and scope.

## 15. Conformance suite (planned)

A separate `cortexkit-role-llm-runner-conformance` crate, run by each runner
in its own CI against its real module over real routes, never a double. It
drives the runner with a scripted model provider and scripted compaction and
step-transform providers. It will check:

Crash recovery may be runner-driven (the runner resumes the cut run) or sender-driven (the runner seals the cut run `interrupted`, then the sender resumes with a new send on the same session), and the compaction cases accept both.

| Case | Requires |
|---|---|
| `run_cancel_step_boundary_finishes_tool_batch` | `run_cancel`, `queue`, `transcript_reads`, `run_ops`, held model/tool calls; reported as a named skip unless the live `role.describe.run_cancel.at` lists `step_boundary`; owner-only, idempotent repeat, recorded tool results and no further model step |
| `run_cancel_unlisted_at_refused` | `run_cancel`, `queue`, `transcript_reads`, `run_ops`, held model call; reported as a named skip unless the live `role.describe.run_cancel.at` lists `step_boundary`; `invalid_params` naming `at`, no state or transcript change |
| `run_cancel_step_boundary_escalates_to_now` | `run_cancel`, `queue`, `transcript_reads`, `run_ops`, held model call; reported as a named skip unless the live `role.describe.run_cancel.at` lists `step_boundary`; terminal reached without releasing the model |
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
| `crash_at_StepRecorded`: resumes with exactly one dispatch or seals `interrupted` without dispatch; the call is never indeterminate and no later request carries it without a result; a recovery that drops the call or its step moves the session to a new lineage | `transcript_reads`, `dispatch_attribution`, and `StepRecorded` |
| `crash_at_<point>` for the other points in §14, asserting their properties | the points the harness declares |
| `retention_honoured`, `retention_no_content_served`, `retention_shorten_continues_lineage`, `retention_equal_noop`, `retention_lengthen_refused`, `retention_late_opt_in_refused`, `retention_zero_refused`, `retention_above_max_refused` | `retention`, `queue`, `transcript_reads`, `run_ops` |
| `retention_run_status_expired` | the above; not applicable unless the major lists `run.status` (§6.1) |
| `retention_without_group_refused` | `queue`; not applicable when `retention` is declared |
| `retention_active_run_never_expires` | `retention`, `queue`, `transcript_reads`, `run_ops`, held non-terminal run (active or paused) |
| `crash_at_RetentionTombstoned` | `retention`, `queue`, `transcript_reads`, `run_ops`, and `RetentionTombstoned`; real process kill, expiry remains readable by name and deletion completion is reported after restart |
| `structured_output_final_text_enforced_passed`, `structured_output_final_text_with_tools_refused`, `structured_output_submit_tool_ends_run`, `structured_output_reserved_name_refused`, `structured_output_absent_without_submit`, `structured_output_failed_validation_completes`, `structured_output_not_inherited`, `structured_output_external_ref_refused`, `structured_output_sibling_never_dispatched` | `structured_output`, `queue`, `transcript_reads`, `run_ops`; the adapter's tool-set send fields, route support switch and model-request observations; reported as a named skip without `structured_output` |
| `structured_output_frozen_across_restart` | the above, and `ToolResultRecorded`; the next model request after a restart carries the same tools and schema bytes |
| `adopt_binds_owner_without_a_run`, `adopt_exact_repeat_replays_reply`, `adopt_send_id_reuse_names_plan`, `adopt_plan_drift_refused`, `adopt_other_owner_refused`, `adopt_same_owner_readopts` | `adopt`, `transcript_reads`; the adapter's host-start hook and host plans; the live `role.describe` must declare `adopt` before any `session.adopt` is sent; reported as a named skip without `adopt` |
| `adopt_during_active_run` | the above, and a held tool call; a run the person started in their own app (§9.5), before any role request, is in progress during the adoption and completes after it |
| `adopt_then_owner_sends` | the above, and `queue`; the owner's send after adoption runs, another caller's is refused |
| `tags_on_every_read_entry`, `tags_never_reused_across_runs`, `tags_exist_whether_or_not_rendered`, `tags_resolve_returns_stored_content`, `tags_resolve_unknown_tag_refused`, `tags_resolve_range_refused`, `tags_model_written_tags_stored_verbatim` | `tags`, `queue`, `transcript_reads`; the adapter's plan-member send fields for the rendered half; the live `role.describe` must declare `tags` before any `session.resolve` is sent; reported as a named skip without `tags` |
| `delivery_order_never_starts_a_run`, `delivery_order_dedupe_supersedes_queued`, `delivery_order_expired_never_delivered`, `delivery_order_with_legacy_delivery_refused`, `delivery_order_unnamed_sender_refused` | `delivery_order`, `queue`, `transcript_reads`; the adapter's plan-member send fields and a plan-named sender identity; the live `role.describe` must declare `delivery_order` before any list is sent; reported as a named skip without `delivery_order` |
| `delivery_order_first_possible_mode` | the above, and a held tool call: a list goes out with the next tool result when it names `tool_result` first, and starts a run on an idle session through `idle_start` |
| `run_context_ceiling_wait_is_not_a_stall` | `queue`, `transcript_reads`, and the adapter's held context-ceiling wait; the run reads `active` with `reason: context_ceiling_wait` on `session.head` (and on `run.status`, with a future `deadline_ms`, where the major lists it) while held, then completes when a view arrives, or ends `context_ceiling` with `cause: view_timeout` when none does |
| `delivery_order_template_renders_as_system`, `delivery_order_bad_slot_refused` | the above; the rendered text goes out as `turn_system` where the adapter says its scripted route supports it, and as the same text at user authority otherwise; an out-of-range slot is refused naming `template` and writes nothing |
| `read_stable_across_host_metadata_change` | `queue`, `transcript_reads`, and the adapter's host-side metadata change: a re-read under the same lineage returns identical entries |
| `run_tool_calls_listed_in_dispatch_order`, `run_tool_calls_unknown_run_refused` | `run_tool_calls`, `queue`, `transcript_reads`; one call per page under `limit: 1`, following `next`; reported as a named skip without `run_tool_calls` |
| `run_tool_calls_indeterminate_until_result` | the above, and a held tool call |
| `run_status_undelivered_steers_present_when_clean` | `run_status`, `queue`; an ended run's status carries `undelivered_steers`, `[]` when every steer was rendered or none was sent |

The `context_ceiling` refusal and run state (§6, §12.1) have no live case:
the suite has no way to bring a runner's context to its ceiling. They are
tested against `context-ceiling.json` in this crate.

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
  `next_from_ordinal` set, and is never truncated. It is served whole only
  when the complete encoded reply fits the 4,194,304-byte transport frame
  limit, including the 21-byte header; otherwise it is refused
  `reply_too_large` (§2, §4, §12).
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
  the last page) when the complete encoded reply fits the 4,194,304-byte
  transport frame limit, including the 21-byte header; otherwise the reply
  is refused `reply_too_large` (§12). `after_mid` with
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
- **Q13. The mark.** Settled: `mark` is not a member of the role (§9).
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
  `send_id_reuse` names the differing field in `detail.field` when the
  runner can identify it; the drafted
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
- **Q22. `run.status` beyond its pinned members.** Settled: an unknown
  `run_id` answers `{state: "unknown"}`, not an error. The session-level
  form, request `{}`, is optional within `run_status` (§6.1).

## Gaps in broca today

Where broca 0.3.194, at commit `549f678`, differs from this document.
Paths are in the broca repository. An optional group a broca build does not
declare is not a gap. None of this is a change request against broca.

1. **Served ops it does not declare as groups.** broca serves `run.status`,
   `run.cancel` and `session.retract`
   (`crates/broca-module-serve/src/serve.rs:2159-2243`;
   `crates/broca-module/src/serve.rs:822-860`), but its `role.describe`
   declares none of `run_status`, `run_cancel` and `retract`. Its ordinary
   op list omits all three; a retention-enabled build adds `run.status`
   as an op, still without its group (`crates/broca-wire/src/role.rs:192-244`).
2. **No per-message run attribution,** although broca declares `run_ops`:
   a page message carries no `run: {run_id, episode, final}`
   (`crates/broca-wire/src/lib.rs:662-673`;
   `crates/broca-module/src/serve.rs:278-289`). broca will implement §6 run
   attribution after merge; page-level `lineage_state` is not a substitute.
3. **No `session_capabilities` in the baseline.** broca declares
   `session_capabilities_from: "admission"`, but its baseline has no
   `session_capabilities` member
   (`crates/broca-wire/src/role.rs:226-238,258-282`). An absent member
   declares no session-level capability (§13); the baseline does not report
   `mid_session_appends` or session-level `ordered_hook_phases`.
4. **`lineage_id` is absent on legacy lineages** until their next write,
   even on a page that has messages (`crates/broca-wire/src/lib.rs:702-717`;
   `crates/broca-module/src/serve.rs:278-295`). Identity is minted on append,
   not read (`crates/broca-wal/src/file_wal.rs:758-798`). §4.2 allows an
   absent `lineage_id` only on the empty page of a session never written.
5. **Owner sends do not require both `send_id` and `prompt` in broca's
   request shape.** `send_id` is optional, and `prompt_blocks` may supply
   the prompt instead (`crates/broca-wire/src/lib.rs:126-140`;
   `crates/broca-module-serve/src/wire.rs:112-154`). A send without a key
   uses a minted run id (`crates/broca-module-serve/src/serve.rs:311-322,2044-2048`);
   ownership is checked separately (`crates/broca-module/src/actor.rs:2192-2240`).
   §9's permission for unkeyed non-owner sends does not relax the required
   members of owner sends.
