# `llm-runner/v1` — role contract

Stability: **alpha**. The role owner's decisions are recorded under
"Open questions". This document is the role definition; the
Rust types in this crate are its wire shapes, this crate's
`test-vectors/llm-runner-v1/` holds its vectors (they ship in its package, and
the `test-vectors` feature exposes them as the public `vectors` module, with
a `SHA256SUMS` pinning every file), and a separate `-conformance` crate
holds its suite (§15). Paths to vector files below are relative to this
crate.

Derives from the CK extensibility design r7.3, §3.2–§3.5, §5, §6, §9.3, §10,
§12.1, §13.2 and §16.7–§16.8, with `session.send` delivery modes replacing
separate steer and queue ops.

Every item is marked:

- **[pinned]**: the design states it. A runner must do it, and a consumer may
  rely on it.
- **[open: Qn]**: the design is silent or ambiguous, and this document records one
  option down so the types and vectors have something to pin. Question `Qn`
  under "Open questions" lists the options.
- **[open]** (no question number): not yet defined, because it depends on a
  section this crate has not written yet. Nothing may build against it.
- **[provisional]**: settled in substance, but its exact name or position
  depends on an unwritten section and may still change. Do not build
  against the spelling yet.
- **[informative]**: guidance for callers. It adds no obligation on a
  runner, and no conformance case checks it.

A question the role's owner has settled keeps its number and is marked
"settled" there; the items it governs are marked **[pinned]** here. In this
revision every numbered question, including Q22 (§6.1), is settled, and
the `session_change` request, reply and refusal shapes (§10.2) are pinned.

An LLM runner is any module that runs model sessions. Nothing here names a
particular implementation; "Gaps in broca today" at the end compares the one
shipping runner with this document.

## 1. Role identity and addressing

- [pinned] A runner lists `llm-runner/v1` in its manifest's
  `capabilities.provides` (`PROVIDES`). A module may serve several majors.
- [pinned] Required ops (`REQUIRED_OPS`): `role.describe` and
  `session.baseline`.
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
  request names its session.
- [pinned] Every role reply is a `RESPONSE` frame whose body is exactly
  `{"result": <answer>}`, an object with the single key `result` holding the
  op's answer (`wire::reply_body` builds it). A reader refuses a bare answer,
  and a body with any other top-level key, by name (`wire::read_reply`,
  `wire::ReplyShapeError`: `bare_answer`, `extra_keys`, or `bad_answer` for
  an answer of the wrong shape inside `result`). A `session.subscribe`
  `STREAM_DATA` frame carries the bare event, never wrapped. A refusal is an
  `ERROR` frame whose body is `{code, message, detail?}` (§12): `class`, and
  every other refusal field, lives inside `detail`, never at the top level.
  A run error (`RunResult::error`, §6) is part of an answer, not a refusal,
  and keeps `class` at its own top level. `reply-envelope.json` pins a
  wrapped answer accepted, bare answers and an extra key refused, refusal
  bodies and a bare stream event.
- [pinned] The transport delivers a frame of at most 4,194,304 bytes, its
  21-byte header included (`TRANSPORT_FRAME_LIMIT`, equal to the
  subconscious daemon's `MAX_DELIVERED_FRAME_LEN`). It is the limit
  `reply_too_large` names (§12.1).

## 2. `role.describe`

- [pinned] The answer is `{majors: [{version, ops, stability}],
  implementation_version, capabilities, session_capabilities_from,
  max_bytes?, steer_receipt?, delivery_order_modes?, retention?, session_retention?, run_cancel?, step_events?, archive_prunes_reasoning?}` (`RoleDescribe`), the shape `tool-provider/v1`
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
  declared, optional cancellation-point metadata (§3), and optional delivery
  list mode metadata (§9.7).
- [pinned] `delivery_order_modes` optionally lists the delivery list modes the
  runner can ever land (§9.7). Absent means the capability-group mapping is
  unchanged; present means only its listed modes count as declared, still
  requiring their groups. `check_describe` refuses an unknown mode name, a
  duplicate, an empty list, a listed `steer` without the `steer` group, a listed
  `idle_start` without `queue`, or the member without the `delivery_order`
  group. The member is an array of strings (`Option<Vec<String>>` in
  `RoleDescribe`, built with `with_delivery_order_modes`).
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
| `output_refs` | `session.output_refs`, `session.output_ref_read` (requires `tags`) | §9.13 |
| `transcript_reads` | `session.read`, `session.head` | §4 |
| `host_history` | `session.history` | §4.6 |
| `history_display` | `session.history` with `form: "display"` | §4.7 |
| `live_events` | `session.subscribe` with typed events, `seq` and the attach replay | §7.1 |
| `step_events` | No added op; plain bus hints after native step recording | §14.3 |
| `call_identity` | No added op; `call_key` and `schema_pin` on display tool calls and live tool events (it requires `history_display` or `live_events`) | §9.16 |
| `client_message_id` | `session.send` with `client_message_id` | §9.11 |
| `run_cause` | `session.subscribe` with `send_id` on `run_started` (it requires `live_events`) | §7.1 |
| `attachments` | `session.upload_begin`, `session.upload_append`, `session.upload_status`, `session.upload_commit`, `session.media_read`, `session.send`, `session.read` | §9.12 |
| `run_ops` | `run.result`, `session.read` (attribution) | §6 |
| `run_status` | `run.status` | §6.1 |
| `held_calls` | `run.status` and run status events with held tool calls | §6.1 |
| `run_cancel` | `run.cancel` | §6.2 |
| `dispatch_attribution` | `session.read` (per-call fields) | §5 |
| `streaming` | `session.read` (`head`), `session.subscribe` | §7 |
| `model_view` | `session.read` (`view: "model"`) | §8 |
| `steer` | `session.send` with `delivery: "steer"` | §9 |
| `queue` | `session.send` with `delivery: "queue"` | §9 |
| `interrupt` | `session.send` with `delivery: "interrupt"` | §9 |
| `plans` | `session.send` with `plan` (baseline via required `session.baseline`) | §10.1 |
| `system_text_default` | `session.send` with preset-less `system_text` plan items | §10.1 |
| `compaction` | `session.read`, `session.resolve` (it requires `tags`), and the runner half of compaction-provider 0.3.0 §3 | §11.1 |
| `tag_checkpoint` | `session.read`, `session.resolve` (it requires `compaction`), and honouring a Setup checkpoint | §9.6 |
| `session_change` | `session.refresh`, `session.refresh_policy`, `session.flush_prefix` | §10.2 |
| `retention` | `session.send` with `retention` | §9.1 |
| `session_retention` | `session.send`, `session.head`, `session.release`; extends `session.adopt` where declared | §9.14 |
| `archive_prunes_reasoning` | `session.read`; extends every served `session.read` and `session.history` page form | §9.14.1 |
| `retract` | `session.retract` | §9.2 |
| `input_token_cap` | `session.send` with `budget.max_input_tokens` | §9.3 |
| `max_steps` | `session.send` with `stop_when.max_steps` | §9.3.1 |
| `max_output_tokens` | `session.send` with `generation.max_output_tokens` | §9.3.1 |
| `no_tools` | `session.send` with `tools: []` | §9.3.1 |
| `structured_output` | `session.send` with `structured_output`; its answer on `run.result` and `run.status` | §9.4 |
| `adopt` | `session.adopt` | §9.5 |
| `tags` | `session.read` (`tags`), `session.resolve` | §9.6 |
| `run_tool_calls` | `run.tool_calls` | §6.3 |
| `run_usage` | `run.result` with `usage`, and `run.status` with it where the runner serves that op | §6.4 |
| `session_usage` | `session.head` with durable, incremental session spend in `usage` | §4.3 |
| `head_sender` | `session.head` with the attested caller's effective delivery grant in `sender`; absence means no grant | §4.3 |
| `delivery_order` | `session.send` with a list in `delivery`, plan-named senders and plan templates | §9.7 |
| `send_status` | `send.status` | §9.8 |
| `user_commands` | `session.user_command`, `session.user_command_status` | §9.10 |
| `turn_directives` | `session.send` with `directives`; its answer on `run.result` and `run.status` where served | §9.9 |
| `interrupt_reason` | `run.result` with durable `reason`; the same on `run.status` and `last_run_state` where served | §6.5 |

- [pinned] A group that only adds fields to `session.read` requires
  `session.read`, not all of `transcript_reads`.
- [pinned] A runner declares `system_text_default` only when it accepts a
  plan whose `system_text` item has no preset and forwards that item unchanged
  to its provider. `check_describe` checks the group for `session.send`; no
  additional `role.describe` fields are required.
- [pinned] A composer omits `preset` on a `system_text` item only when the
  runner carrying the plan declares `system_text_default` and the item's
  provider declares `system-text-default/v1` in its module manifest's
  `capabilities.provides`, read from `catalog.list`. Otherwise it keeps
  naming the preset. These are declared facts, never inferred from crate
  versions.
- [pinned] `host_history` is declared on its own (§4.6). A runner with no
  host under it, one that is its own host and keeps the session's whole
  history itself, must declare it. A runner on a host (one that runs its
  sessions inside another program's store, such as OpenCode's) may declare
  it, and then serves the history from the host's own storage on demand,
  never from a copy of its own. `check_describe` checks the group for its
  op; whether a runner is its own host is not visible in the answer, so a
  consumer that needs the history checks for the group.
- [pinned] `run_status`, `run_cancel`, `retract`, `input_token_cap`,
  `structured_output`, `adopt`, `tags`, `run_tool_calls`, `run_usage`,
  `delivery_order`, `send_status`, `max_steps`, `max_output_tokens` and
  `no_tools`, `turn_directives` and `interrupt_reason` are each declared on their own: a runner may serve any of them without the others,
  and without `run_ops` or `transcript_reads`. `tags` adds fields to
  `session.read`, so it requires that op (and `session.resolve`), not all of
  `transcript_reads`.
- [pinned] A caller MUST NOT send `directives` unless the runner advertises
  `turn_directives` (§9.9). `check_describe` checks the group for
  `session.send` and `run.result`; `run.status` carries the same answer when
  the runner also serves `run_status`. It adds no discovery metadata.
- [pinned] `history_display`, `live_events` and `client_message_id` are each
  declared on their own (added in 0.5.1). A caller MUST NOT send `form` or
  `limits` on `session.history` unless the runner declares
  `history_display`, nor `limits` on `session.subscribe` unless it declares
  `live_events`, nor `client_message_id` on `session.send` unless it
  declares `client_message_id`: an older runner decodes those requests
  strictly and refuses the member. A consumer relies on the typed events of
  §7.1 only from a runner declaring `live_events`. `check_describe` checks
  each group for its op; none adds discovery metadata.
- [pinned] `run_cause` is a group of its own (added in 0.5.2), declared
  only beside `live_events`, whose `run_started` event it extends:
  `check_describe` refuses it without that group by name
  (`run_cause_without_live_events`) and checks it for `session.subscribe`.
  It adds no discovery metadata. It is a group, not a bare optional member,
  because absence has to mean something: from a runner before 0.5.2, a
  `run_started` without `send_id` may only mean an older runner, while from
  a runner declaring `run_cause` it means that no send started the run
  (§7.1). A consumer that tells a person's turns from its own (the session
  owner reads its own `send_id` as its own message, any other `send_id` as
  another sender's, and an absent one as a person typing in the host)
  relies on that reading only from a runner declaring the group. A runner
  that cannot name the send that started a send-started run does not
  declare it.
- [pinned] `call_identity` is a group of its own (added in 0.7.2, §9.16),
  declared only beside `history_display` or `live_events`, whose tool-call
  entries it extends: `check_describe` refuses it by name when neither is declared
  (`call_identity_without_display`). It lists no op, because either of
  those groups may be the one it extends. It is a group, not bare optional
  members, because absence has to mean something: from a runner declaring
  it, an absent `call_key` or `schema_pin` follows §9.16's presence rules;
  from any other runner it says nothing.
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
- [pinned] `run_usage` adds `usage` to `run.result` and `run.status`
  (§6.4) and no op of its own; `check_describe` checks the group for
  `run.result`, the op whose answer it extends, and it adds no discovery
  metadata. A runner that declares it without `run_status` reports usage on
  `run.result` alone. A runner that does not declare it may still send
  `usage`, but a consumer relies on it only from a runner that declares the
  group.
- [pinned] A caller MUST NOT send a list in `delivery`, nor a plan naming
  `senders`, unless the runner advertises `delivery_order` (§9.7). A caller
  MUST NOT send `dedupe_key`, `expires_at_ms` or `template` on a send whose
  `delivery` is not a list: runners tolerate unknown send fields, so an
  older runner would ignore those members and deliver the send as a legacy
  `queue` (a reminder starting a run, a template send going out as an empty
  prompt). Beside a list they fail closed, because every runner without the
  group, older ones included, refuses the whole send naming `delivery`.
- [pinned] A caller MUST NOT send `send.status` unless the runner
  advertises `send_status` (§9.8). Against any other runner it follows a
  send by re-sending the full body under the same `send_id` (§9).
  `check_describe` checks the group for `send.status`; it adds no discovery
  metadata.
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
- [pinned] `compaction` is all of §11.1 or none of it: Setup, advance
  notices, views applied at step boundaries, rides, the ceiling and native
  markers. A runner that hosts no sessions of its own (one that only steers
  sessions another runner hosts) declares none of it, never a stub.
- [pinned] `compaction` requires `tags`: an answer declaring it without
  `tags` is malformed, and `check_describe` refuses it by name
  (`compaction_without_tags`), because a compaction provider addresses every
  view edit by part key and tag. The group lists `tags`' ops,
  `session.read` and `session.resolve`.
- [pinned] `tag_checkpoint` requires `compaction`: an answer declaring it
  without `compaction` is malformed, and `check_describe` refuses it by
  name (`tag_checkpoint_without_compaction`), because the checkpoint rides
  the compaction provider's Setup answer. It lists `compaction`'s ops and
  adds none (§9.6).
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
include_originals?, view?, media?}` (`ReadRequest`). Three modes (`ReadRequest::mode`):

- [pinned] **Tail.** With no cursor, the newest page within the current
  lineage (§4.5). This is a contract, not a default a runner may change; the
  suite checks it on a session longer than one page.
- [pinned] **Range.** `from_ordinal` reads forward from that ordinal. An
  ordinal below the lineage's `first_ordinal` is refused
  `before_lineage_start` with `detail.first_ordinal`
  (`errors::BeforeLineageStartDetail`, §4.5).
- [pinned] **After a message.** `{after_mid, lineage_id}` reads strictly after
  that message. It is refused by name as `lineage_changed` (the session's
  lineage is not `lineage_id`), `unknown_mid` (the lineage has no such
  message) or `before_lineage_start` (the message lies below the lineage's
  `first_ordinal`), and never silently answered from the tail.
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
- [pinned] `media` is `"inline"` (the default), `"descriptors"`, or, on an
  `attachments` runner, `"blocks"` (§9.12). Unknown values are refused
  `invalid_params {field: "media"}`. Inline preserves existing non-media
  payload bytes, whole-message paging and `reply_too_large` behavior; §9.12
  defines the text rendering of media on an `attachments` runner.
  Descriptors are offered only on raw transcript reads and `session.resolve`.
  `view: "model"` with `media: "descriptors"` is refused before read work with
  `invalid_params {field: "media"}`; model-view descriptors are not offered.
- [pinned] The request is strict: an unknown field is refused
  `invalid_params` naming it, because a missing cursor changes the question
  (a misspelled `from_ordinal` would otherwise get the newest page).

### 4.2 The page

`{messages, lineage_id, next_from_ordinal?, head?, pruned_reasoning?}` (`ReadPage`).

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
  Otherwise, in inline mode, the runner refuses `reply_too_large`, naming the message by its
  session and ordinal (§12).
- [pinned] A page is an ordinal range and nothing more. It is not aligned to
  turns, runs or tool-call boundaries: a tool call and its result may arrive
  on different pages.
- [pinned] Each message (`ReadMessage`) carries `ordinal`, `mid` and its
  final values: what later steps render and what executed. The raw view and
  the model view then differ only by compaction.
- [pinned] **Only finished messages.** `session.read` returns only messages
  the runner will no longer change, except for declared archive pruning
  (§9.14.1). A message still being generated (a
  model step streaming, a tool result being written) is never an entry
  until it completes. This is stricter than, and contains, the final-values
  rule above; streaming progress reaches a consumer through the stream
  (§7), never through a partial entry.
- [pinned] **Read stability.** Read stability is scoped to the lineage
  (§4.5). Once `session.read` has returned an entry
  under a `lineage_id`, its `message` bytes, `ordinal`, `mid`, `role`,
  `written_at`, `delivered_as`, and its part keys and tags never change
  under that lineage, except for declared archive pruning (§9.14.1).
  Envelope members that report later state move forward
  only as their own rules say (`run.final`, §6; `tool_calls`'
  `call_key`, `dispatched_to` and `indeterminate`, §5). A runner that reads
  from a host freezes the bytes it first ingested and serves its own
  journal, never the host's live history. A host change that reaches the
  model (a revert, an edited part) and a recovery that drops or rewrites a
  written message (§14, property 4) start a new lineage, so reads of the old
  one are refused `lineage_changed`; changed bytes never appear under the
  same lineage except for declared archive pruning (§9.14.1). A recovery that only appends, such as closing a never-sent
  call with its never-executed result (§14, property 4, shape (b)), keeps
  the lineage, because no byte already written changes. A reader may treat
  changed bytes under one lineage as a runner defect, except for the
  precisely listed archive-pruning changes of §9.14.1. That exception changes
  only listed part payloads, never identity, ordering, tags or write-time digests.
- [pinned] **Bookkeeping stays out.** Host records a runner writes for its
  own compaction bookkeeping (a host's native compaction message, summary or
  entry) are outside the transcript: they never appear in `session.read`
  and are never rendered to the model on top of the view.
- [pinned] **The envelope.** A read entry is `{ordinal, mid, message, run?,
  originals?, tool_calls?, tags?, role?, written_at?, delivered_as?, digest?,
  part_digests?}`. Every
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

#### Descriptor payloads and identity

- [pinned] In descriptors mode every byte-backed image, file, PDF or audio
  payload becomes `{type: "media_ref", part?, kind, mime, byte_size, sha256,
  width?, height?}`. Size and SHA-256 are over decoded bytes, never base64.
  Dimensions are present only when read from an image header. URL-only media
  stays inline; a text-sourced document is text, not byte-backed media.
- [pinned] `PART_INLINE_MAX = 262_144` UTF-8 bytes. Every other payload over
  that threshold is a ref: `text_ref` adds `char_count` (Unicode scalar values)
  and `head` (the first 4,096 UTF-8 bytes, cut back to a character boundary);
  `args_ref` adds the tool `name`; `reasoning_ref` describes thinking or
  reasoning data. Each has `part?`, `byte_size` and lowercase `sha256`.
  Text SHA-256 covers exposed text, cleaned where the writing run requires
  it; arguments cover their bytes as written. These payload hashes differ
  from the JSON content identities below.
- [pinned] Refs replace only payloads, in place. A top-level ref carries the
  replaced part's key. A nested ref has no `part`: its containing top-level
  part remains the unit for tags, `replace_part` and content digests. Large
  text includes a tool result's string content and an OpenCode tool output.
  A described call keeps its id and name beside its `args_ref`; a result keeps
  its call id or `tool_use_id`, `is_error` and every other non-payload field;
  reasoning retains its kind, signatures and provider metadata, except for
  payloads already pruned under §9.14.1.
- [pinned] `DESCRIPTOR_MESSAGE_MAX = 3_145_728` bytes bounds one complete
  encoded `ReadMessage`, leaving 1 MiB of the daemon frame for the page
  envelope and transport header. If describing all media and all payloads
  over the threshold still exceeds it, describe additional payloads largest
  first, ties broken by earlier part order, until it fits. Selection depends
  only on that message's own content and these constants, never `max_bytes`,
  page position or neighboring messages: repeated reads on one lineage return
  identical bytes except for declared archive pruning (§9.14.1). The runner
  selects fewer messages to keep the complete
  page inside the frame. `max_bytes` is still soft for one message: an
  over-cap message is served alone. The only descriptors-mode
  `reply_too_large` is non-payload structure exceeding the fixed budget; its
  detail names `ordinal`. `read::descriptors::describe` supplies the shared
  largest-first policy; schema adapters identify payload locations.
- [pinned] A part key is a runner-defined string, opaque to providers. Providers
  never derive keys from message bytes; they use only keys the runner served
  for that message, in `part_digests` and in `tags` rows. Key stability is
  governed by **Read stability** above (§4.2), not by a serializer-profile key
  convention.
- [pinned] Within one message, part keys are unique. In descriptors mode,
  `part_digests` lists exactly the same keys as the entry's `tags` rows, in
  the same order. The complete tag-row key list is defined by **Reads** in
  §9.6; a part without a tag number still has a row there.
- [pinned] In descriptors mode every `ReadMessage` MUST carry `digest` and
  `part_digests: [{part, digest}]`, every top-level part in message order,
  nested media excluded. Both digest fields are optional in inline mode.
  Digests use `content_digest::content_digest`:
  lowercase SHA-256 over RFC 8785 (JCS) of the full stored message or full
  stored part as originally written, before archive pruning (§9.14.1) or any
  media rendering (§9.12). This is the same function used for compaction
  notices' `through_digest` and `descends_from`, re-exported by the provider
  crate; it never hashes descriptors. Digest rows with keys not matching the
  actual parts, missing rows, duplicate keys or wrong order are decode errors
  in the kit. Ref shapes decode strictly. A compaction provider uses these
  served identities and ref hashes as given, never rehashing a descriptor.
  `ReadMessage::check_descriptor_rows(profile)` checks the provider-visible
  envelope: digest shapes, unique keys, the profile's top-level part count,
  and exact equality with the entry's tag-row keys in order. Missing tag rows
  are rejected. Unknown profiles and malformed part containers return errors
  naming the profile.
  `serializer_profile::top_level_part_count(profile, message)` counts the
  following containers without deriving part keys:

  | Profile | Part container |
  | --- | --- |
  | `anthropic-messages` | `content`: array length, or one part for a string (even empty) or whole-content `text_ref`. |
  | `opencode-aisdk` | `parts`: array length. |
  | `owned-broca` | `content`: array length only; strings, missing content and whole-content `text_ref` are malformed. |
  | `pi` | `/message/content` in a wrapped journal entry: array length, or one part for a string (even empty) or whole-content `text_ref`. Bare bodies and other content types are malformed. |

  Arrays count only their immediate elements, never nested media. No other
  member contributes parts, including `role`, `origin` or `tool_calls` for
  `owned-broca`, or `details` on a `pi` message with role `toolResult`.
  `ReadMessage::check_descriptors(keys)` is the runner-side strict check using
  independently known keys in message order; a provider uses the complete keys
  the entry itself serves in its `tags` rows, never keys derived from bytes.
- [pinned] Ordinals and `mid`s are never reused or renumbered within a
  lineage, across any crash. Ordinals are session-wide: a cut starts a new
  lineage but renumbers nothing, so a message keeps its ordinal and `mid`
  on every lineage that still holds it (§4.5).
- [pinned] With `include_originals`, a hooked message carries `originals:
  [{field, value, provider}]` (`Original`), oldest first.
- [pinned] `head` is the last durable event covered by the same snapshot as
  the page (§7). It is a JSON object, opaque to consumers, and absent on a
  session with no events.
- [pinned] Decoded leniently: a consumer ignores fields it does not know.
  Runners may add their own (broca's `lineage_state`, say).

### 4.3 Head metadata

- [pinned] Head metadata without bodies: `{lineage_id, last_ordinal,
  first_ordinal, cut?, history_version, head, last_run_state, last_run_error?, updated_at,
  input?, model?, usage?, sender?, incarnation?}`
  (`HeadMeta`).
- [pinned] `history_version` is a `u64`, on `session.head` and on every
  `session.history` page. It rises at every lineage change except a cut: a
  revert, a recovery that rewrites written messages, or a restart after the
  runner lost its state. It never changes at a cut or when messages are
  appended. It never decreases, a store restore included: a runner keeps a
  floor outside the store it restores, so a restored store cannot hand out
  a lower value. Readers that hold a position in the visible history (an
  app's claim on a message, say) key on it, so a compaction does not
  invalidate the position while a rewrite of the visible history does.
- [pinned] `cut: {mid, digest, written_at}` (`read::HeadCut`) describes the
  cut that started the lineage's readable history, never its content: the
  message the cut folds last (the one at the view cut's `through_ordinal`,
  one below `first_ordinal` for a cut the runner made), its content digest
  (the digest a cut lineage's `descends_from` names, compaction-provider
  §5.1) and its creation time. A cut made before the runner numbered
  anything, in a host's history before the runner adopted the session,
  names the host's message, and its lineage's `first_ordinal` may be 0. A
  lineage with no cut carries no `cut`, and its `first_ordinal` is 0.
- [pinned] `first_ordinal` is the current lineage's first ordinal (§4.5):
  0 for a lineage no cut started, the cut's first retained ordinal for one a
  cut started. It is present whenever `lineage_id` is, and is at most one
  above `last_ordinal` (a cut that retained no message).
- [pinned] It is its own op, `session.head`, taking no fields
  (`HeadRequest`); `last_ordinal` is absent on an empty transcript;
  `last_run_state` is `{run_id, state, reason?}` (`LastRunState`) and
  describes the most recent run, never the transcript; `updated_at` is
  milliseconds since the Unix epoch. `last_run_state` is the pinned run
  summary; a page's `lineage_state` is a runner member, not pinned.
- [pinned] `last_run_error?: {class, retry_after_secs?, resets_at_ms?}`
  (`read::LastRunError`, added in 0.7.5) may be present only with
  `last_run_state` whose `state` is `"error"`, and describes that same run.
  A `paused` run (for example `account_changed`) or an `interrupted` run
  carries none: `reason` already explains why it stopped.
- [pinned] Its values equal that run's `RunError.class`, `retry_after_secs`
  and `resets_at_ms` as served by `run.status`, never a separate derivation.
  A runner should write both from the same terminal record.
  `retry_after_secs` is fractional (for example 1.5), absent when the provider
  gave none, never 0 as a stand-in; `resets_at_ms` is an optional `u64`.
  On the head, `retry_after_secs` is finite and at least 0; a runner whose
  `RunError.retry_after_secs` is negative or non-finite omits it on the head.
- [pinned] The head intentionally leaves out `message`, `provider_code`,
  `status`, `account_ref` and provider diagnostics. A client app needs only
  class and timing for its retry decision; a reader wanting the text calls
  `run.status` once.
- [pinned] An error state without `last_run_error` means "not reported"
  (for example a runner predating 0.7.5), never "permanent". No describe
  group is needed. `class` is an open string: known examples today are
  `transient`, `permanent`, `auth_required`, `context_overflow` and
  `provider_unavailable`; older records may say `auth`. Readers decode and
  retain any value, including unknown ones.
- [pinned] `input?: {attachments, modalities?, mime_types?}`
  (`read::HeadInput`, added in 0.5.1) describes what a send that names no
  model can carry. `attachments` is `yes`, `no` or `unknown`, decoded open
  (an unknown value reads as `unknown`). `modalities` come from the model
  catalog (Fusiform's `capability.input_modalities`); `mime_types` come only
  from the runner's own provider adapter, and absent they mean the
  modalities are accepted but the exact types are unknown. A fact the runner
  lacks is `unknown`, never `no`; `no` is a known fact, such as a runner that
  cannot carry attachments at all. An absent `input` means unknown, so the
  member needs no group. It is outside the lineage rule below: a
  never-written session may state it.
- [pinned] `model?: {provider, model, variant?}` (`read::HeadModel`, added
  in 0.6.1) is the selection a send without a named model would use next.
  It is optional, needs no describe group, and absent means unknown. The
  `variant` member is absent when the selection names none, exactly as in
  `by_model` (§6.4). The runner reads its own record of the latest model
  switch, never the frozen plan's bytes: core does not rewrite a head's plan
  when the person switches models. This describes the next send, not history;
  each run's `by_model` already describes what that run used. Like `input`,
  `model` is independent of the lineage and may be known before the first send.
- [pinned] `sender?: {modes: [<delivery-mode names>], interrupt: bool,
  owner?: true}` (`read::HeadSender`, added in 0.6.1) is the attested caller's
  own delivery grant: the route principal, never anyone else's. The mode
  names are exactly those used by the plan. `modes` is the runner's supported
  delivery modes after describe's `delivery_order_modes` narrowing, with
  `idle_start` included for a named caller only when its plan entry has
  `idle_start: true`; `interrupt` is that entry's flag. This is derived from
  the same rule that send admission (`delivery_order::admit`) uses, so they
  cannot disagree. A runner declares the `head_sender` describe group (added
  in 0.6.2) when its `session.head` serves `sender`. Under that group, absent
  means "no grant for you": the caller is neither the owner nor named by the
  plan. Without the group, readers treat an absent `sender` as unknown, even
  if the runner reports `sender` for some callers. A present `sender` reports
  a grant with or without the group. Readers can use
  `HeadMeta::sender_grant` (`read::SenderGrant`: `Granted`, `NoGrant`,
  `Unknown`) to make this distinction. It is independent of lineage.
- [pinned] `sender` uses the plan the runner would check on that caller's
  next send. After an accepted `session.refresh` takes effect, this is the
  effective plan, not the original frozen bytes: `sender` never disagrees
  with the next send's grant check. A runner declaring `head_sender` must
  report `sender` to the owner and every caller named by the plan. The owner
  receives `owner: true`, every mode the runner would accept from the owner
  on its next send, and `interrupt: true`. A session with no plan reports
  `sender` only to its owner.
- [pinned] `incarnation?` (added in 0.6.1) is an opaque string of 1–128
  bytes, compared by equality only. It needs no group. It is minted when
  the key's log is first created and changes only when that key's log is
  created again, such as after a retained session expires and the key
  restarts at ordinal 0. A new incarnation must never repeat an earlier one
  for that key, even across a store restore: at least 122 random bits from
  the OS random source (a UUID v4 qualifies), or anything else that can't
  come back for a new incarnation; never a counter a restore could rewind.
- [pinned] The incarnation stays the same across compaction cuts, reverts,
  restarts, restores of the same log, and moves. The runner carries it in
  the session's exported move data, so the destination reports the same
  value. A runner omits `incarnation` unless it can vouch that the value
  survives every case in that lifetime list (restart, restore, move, cut,
  revert). It never mints one per process: a per-process value would silently
  break readers' claims after a restart. Readers key claims on
  `(incarnation, mid)` when it is present. Absent means unknown; readers
  then key claims on `lineage_id`, which is safe but expires claims at cuts.
- [pinned] `usage?` (`usage::SessionUsage`, added in 0.6.1) is durable session
  spend, behind the `session_usage` describe group. A runner that does not
  declare that group leaves `usage` absent. It has exactly §6.4's `run_usage`
  shape and rules, plus required `since: run_id`: `steps` and
  `unreported_steps` are required; `input_tokens`, `cached_input_tokens`,
  `cache_write_tokens`, `output_tokens` and `reasoning_tokens` are absent
  until a step reports them, never zero for unknown. `by_model` is keyed by
  exactly the same (provider, model, variant) selection and follows the same
  ordering, absence and consistency rules.
- [pinned] Every usage count in the session total and in each `by_model`
  entry (`steps`, `unreported_steps` and every present token count) is a
  non-negative integer no greater than 2^53 − 1 (9,007,199,254,740,991).
  Readers that decode JSON numbers as doubles can therefore read each count
  exactly. A consumer rejects an out-of-range session usage as malformed
  (`SessionUsage::consistent` checks the upper bound).
- [pinned] Session usage counts every step of every run in the session,
  across compactions, history versions and person-started turns on adopted
  sessions. It is spend, so it never decreases on a rewind, revert or
  compaction. Rewinding visible history without completing a new step leaves
  the total unchanged. The runner maintains the total incrementally, in the
  same durable commit that stores a step's usage, never by scanning history.
  Reading it costs the same at 150,000 messages as at 1,000.
- [pinned] After a restart counting continues from the durable total.
  Required `since` names the session's first counted run (`run_id`), or, for
  an adopted or imported session whose history the runner never counted, the
  first run after adoption. Before there is a first counted run, `usage` may
  be absent. Readers must display that boundary rather than present a partial
  total as the whole session's spend. For runs since `since`, their `run_usage`
  figures add up to session usage field by field, but only for fields every
  counted step reported. Readers must not add an absent value as zero.
- [pinned] It follows the read rule (§4.2): an absent `lineage_id` means the
  session has no lineage yet. For a session never written, `lineage_id`,
  `last_ordinal`, `first_ordinal`, `head`, `last_run_state` and
  `updated_at` are all absent.
- [pinned] An answer without `lineage_id` carries none of the other
  lineage-dependent members, and an answer with `lineage_id` carries `updated_at` and
  `first_ordinal` and `history_version`, with `cut` whenever
  `first_ordinal` is above 0 (`HeadMeta::lineage_consistent`). A consumer treats any other answer as
  malformed; it is not a refusal.

### 4.4 Serializer profiles

- [pinned] A serializer profile names the schema of an entry's `message`
  body (`serializer_profile`). It is the value a session's providers
  receive as their plan item's `params.serializer_profile`, which the
  step-transform-provider and compaction-provider contracts pin; they name
  `opencode-aisdk`. This role names `anthropic-messages`
  (`serializer_profile::ANTHROPIC_MESSAGES`), `opencode-aisdk`
  (`serializer_profile::OPENCODE_AISDK`), `owned-broca`
  (`serializer_profile::OWNED_BROCA`) and `pi` (`serializer_profile::PI`).
- [pinned] **`anthropic-messages`.** An entry's body is the message exactly
  as the client sent it in its Anthropic Messages request, with
  `cache_control` members removed in two places only: at the message's
  top-level content blocks, whatever their type, and in the `text`, `image`
  and `document` blocks nested in a `tool_result`'s content
  (`serializer_profile::strip_cache_control`). Never inside a `tool_use`
  block's `input`, never inside a block of a type the profile does not
  name, and a string `content` is kept as sent.
- [pinned] **`opencode-aisdk`** (`serializer_profile::OPENCODE_AISDK`). An
  entry's body is OpenCode's stored message `{info, parts}`, not an AI SDK
  model message: `info` is the message record (`id`, `role`, `time`, and
  `providerID` and `modelID` on an assistant message, `model: {providerID,
  modelID}` on a user message) and `parts` its parts in order, each `{id,
  type, ...}`: `text`, `reasoning`, `tool` (`callID`, `tool`, `state:
  {status, input, output?, error?, attachments?}`), `file` (`mime`, `url`,
  `filename?`) and OpenCode's bookkeeping parts. Fields OpenCode changes
  after a message finished stay as the runner first read them (read
  stability, §4.2). `display::opencode_aisdk` is its decoder (§4.7).
- [pinned] **`owned-broca`.** An entry's body has a `content` array of parts.
  Descriptors replace payloads inside a part, never the whole array.
- [pinned] **`pi`.** An entry's body is the wrapped journal entry
  `{type: "message", id, parentId, message: {role, content, ...}}`, not a
  bare message body. Only `/message/content` holds parts, as specified in
  §4.2; a whole-content `text_ref` replaces a single string part.
- [pinned] Read stability (§4.2) applies: the runner serves the form it
  first ingested (`serializer_profile::ingest`). A client re-sending the
  same message with its cache marker moved (a string `content` turning
  into a one-block array, or back) does not change the entry.
- [pinned] For such a session, a compaction view's `replace` and edit
  `bytes` (compaction-provider 0.3.0) are in this profile's message format.

### 4.5 Lineage and the read bound

A lineage is the readable history from the session's latest cut: the
messages `session.read` and `session.resolve` serve. What lies before the
cut is carried to the model only by the compaction view's head (compaction
provider 0.5.0 §6) and to readers only by `session.history` (§4.6).

- [pinned] A cut starts a new lineage. A cut is the point through which an
  applied compaction view's replacements stand for the whole transcript
  (the view's `cut`, compaction-provider §6, §9); the new lineage's first
  ordinal is the cut's first retained ordinal, `cut.through_ordinal + 1`.
- [pinned] `lineage_id` changes at a cut, at a revert, and at a recovery
  that drops or rewrites written messages (§14, property 4, shape (a)), and
  at nothing else. A runner may derive the id from its own storage identity
  plus the cut, as long as every lineage of the session gets an id of its
  own.
- [pinned] A read naming an older lineage, `after_mid` included, is
  refused `lineage_changed` (§4.1). Read stability (§4.2) holds within one
  lineage: frozen bytes from the runner's own journal, never the host's
  live history.
- [pinned] Ordinals stay session-wide and never renumber. A message keeps
  its ordinal across every cut. A revert removes messages, and a later
  message may then take a removed message's ordinal, on the new lineage.
- [pinned] **A revert below the latest cut.** A revert that removes the
  latest cut's message (the one the cut folds last, which
  `session.head.cut` names), or anything before it, undoes that cut. A cut
  is at or below the revert point while its message survives the revert. The new lineage starts at the newest cut still at or below the
  revert point: its `first_ordinal` is that cut's first retained ordinal
  and `session.head.cut` describes that cut. Its first advance notice
  carries `descends_from` naming, at that cut, the lineage the runner was
  on when it made the cut (`<lineage>:<cut through_ordinal>:<digest>`, as
  that cut's own lineage first declared it), only when the runner has such
  a lineage: a cut made before the runner adopted the session has none, and
  the notice then carries no `descends_from`. With no cut left, the
  lineage starts at the session's first message, `first_ordinal` 0 and no
  `cut`, without `descends_from`. Either way Setup runs again for the new
  lineage: that is §11.1's rule that Setup runs once per lineage, before
  the lineage's first model call, which covers every new lineage, one a cut
  starts included, whether or not it carries `descends_from`; it is not an
  exception.
- [pinned] `session.head` states the current lineage's `first_ordinal`
  (§4.3). A range read, an after-message read and `session.resolve` that
  name a position below it are refused `before_lineage_start` with
  `detail.first_ordinal` (§12.1). Nothing is ever served from before the
  lineage, or from anywhere else, in their place. For `session.resolve`, the
  position is the part the tag names: a tag at or below the lineage's
  numbering anchor (the checkpoint the latest cut's marker or the lineage's
  Setup answer carries, §9.6) names a part before the cut.
- [pinned] The tail reads the newest page within the lineage.
- [pinned] `before_lineage_start`, `cursor_expired` and `unknown_ordinal`
  (§4.6) are new codes. Codes are open strings (§12), so a consumer that predates them
  treats them as terminal refusals of the one request; no public enum of
  this crate gains a variant for them.

### 4.6 `host_history`: `session.history`

`session.read` serves the lineage. `session.history` pages through the
session's whole surviving history, from its first message through the
current lineage's ancestry: the surviving prefix, never a branch a revert
left. It is on demand only, for an app's reader and for raw recovery (a
model asking to expand a folded range). It runs per page, never per step,
and sits outside the cost rule (§14.2). It is paged by opaque cursors,
never by ordinal: a runner on a host that adopted an existing session has
no ordinals for the history before the adoption, and numbering it would
need the whole-session walk the cost rule removes.

- [pinned] The request is `{cursor?, from_mid?, before_mid?, from_ordinal?,
  before_ordinal?, limit?, max_bytes?, media?, include_originals?, view?}`
  (`history::HistoryRequest`),
  strict like `session.read`'s (§13): an unknown field is refused
  `invalid_params` naming it.
  - With none of `cursor`, `from_mid` and `before_mid`, it reads the newest
    page. `from_mid` reads forward from that message, inclusive.
    `before_mid` reads the page that ends directly before that message,
    exclusive. `cursor` reads the page a previous page's `older` or `newer`
    named.
  - `from_ordinal` reads forward from the entry with that ordinal,
    inclusive; `before_ordinal` reads the page that ends directly before it,
    exclusive. They are valid only for an ordinal the runner assigned and
    still holds: one never assigned, or one a revert removed, is refused
    `unknown_ordinal`. `before_ordinal` at the lowest assigned ordinal
    returns the newest entries without an ordinal (an adopted session's
    history before adoption), by the boundary rule below. Paging onward
    from a seek uses the returned cursors, as from any other position.
  - At most one of `cursor`, `from_mid`, `before_mid`, `from_ordinal` and
    `before_ordinal` is given; a second one is refused `invalid_params`
    naming it, in that order (`HistoryRequest::mode`). An unknown `from_mid` or `before_mid` is
    refused `unknown_mid`. A cursor that does not parse is refused
    `invalid_params {field: "cursor"}`.
  - `view: "model"` is refused `invalid_params {field: "view"}`: history
    serves raw entries only. `limit`, `max_bytes` (with the byte caps of
    `role.describe.max_bytes`), `media` and `include_originals` are those of
    `session.read`, with the same oversize and `reply_too_large` rules.
- [pinned] The page is `{messages, lineage_id, history_version, older?,
  newer?, pruned_reasoning?}` (`history::HistoryPage`), entries oldest to newest. `lineage_id`
  is the session's current lineage and `history_version` its history
  version (§4.3), as `session.head` states them; a never-written session
  answers an empty page without either.
- [pinned] **Order.** Entries run oldest first within a page. Following
  `older` or `newer` never overlaps or skips an entry.
- [pinned] **Explicit end.** `older` is absent exactly when nothing older
  exists. `newer` is absent exactly when nothing newer existed when the
  page was read. A reader never infers either end from an empty page.
- [pinned] **Cursors.** `older` and `newer` are opaque. A cursor stays
  valid while the entry it was minted next to, and every entry older than
  it, are unchanged in the session's current history. A cut changes
  neither, so cursors survive cuts. A revert or a rewrite that removes or
  changes that entry, or anything older, makes the runner refuse the cursor
  `cursor_expired`, never answer it with an empty page. A cursor minted for
  another session is refused `cursor_expired` too.
  Declared archive pruning (§9.14.1), like a cut, preserves cursors: a cursor
  issued before pruning stays valid. Pages served afterward carry
  `pruned_reasoning` for the parts they serve pruned. A reader continuing a
  walk learns of the payload change from its page, not from `cursor_expired`.
- [pinned] **Entries** (`history::HistoryEntry`) are `session.read`'s entry
  envelope (§4.2) with two differences: every entry carries `mid` and
  `written_at`, and `ordinal` appears only where the runner has one. A
  runner never numbers history on demand.
  - `written_at` is the message's creation time; for a host-served entry,
    the host's own.
  - An entry's `mid` never changes for the life of the session. Where
    `session.read` and `session.history` both serve a message, the `mid` is
    the same; on a runner over a host, a host-served entry's `mid` is the
    host's own message id.
  - An entry served with an ordinal carries that same ordinal in every
    later read, below a cut included. An entry served without one never
    gains one.
  - **Boundary order.** The newest entry without an ordinal comes directly
    before the entry with the lowest ordinal, with nothing between them.
  - **Window entries.** An entry at or after `first_ordinal` is served
    exactly as `session.read` serves it: its bytes, its ordinal and its
    `tags` (`HistoryEntry::matches_read`). An entry below `first_ordinal`
    carries no `tags`: tag numbers belong to the lineage that shows them.
  - Frozen bytes: an entry the runner wrote or ingested keeps the bytes it
    first served, as read stability (§4.2) keeps them.
- [pinned] **Before the window.** A reader holding the `session.read`
  window continues into older history with `{before_mid: <mid of the
  window's first message>}` on a runner that declares `host_history`. It
  never infers the session's start from `first_ordinal`: an adopted
  session's window can start at ordinal 0 with host history before it.
- [pinned] **Edits outside the runner.** Below the window, a runner on a
  host serves what the host holds at the time of the read. Changes a client
  makes to the host's stored history outside the runner (deleting or
  editing an old message through the host's own API) are outside this
  contract: they are not lineage changes, and a runner need not detect
  them. A revert is in scope: history never serves a branch a revert left.
- [pinned] **The lane.** A compaction provider's sync and view building
  read only through `session.read`, within the current lineage, never
  through `session.history` (compaction-provider 0.5.0 §5.2). Raw recovery
  of folded content reads on demand through `session.history` or the
  host's own reader.
- Informative, not pinned: a runner on a host typically keeps a durable
  index from `mid` to ordinal, written once at ingest, kept on disk and
  never resident. That satisfies the cost rule (§14.2), which bounds
  per-step work and resident memory, not disk.

### 4.7 `history_display`: display rows on `session.history`

`session.read` stays the compaction lane's read: raw or descriptors,
finished messages only. `session.history` additionally serves a
display-ready row for an app's reader on a runner declaring
`history_display` (added in 0.5.1).

- [pinned] The request takes `form?: "raw" | "display"` (`history::HistoryForm`,
  strict: an unknown value is refused naming `form`) and `limits?:
  {arguments_bytes, result_bytes, error_result_bytes}` (`display::Limits`,
  strict, every member required). Absent `form` means `raw`. `limits` is
  valid only with `form: "display"`; otherwise it is refused
  `invalid_params {field: "limits"}`. With `form: "display"`, `media` and
  `include_originals` are refused naming them: a row always describes media
  by refs and carries no originals (`HistoryRequest::mode`). Positions,
  cursors, `limit`, `max_bytes` and every refusal are those of a raw read.
- [pinned] The display page (`display::DisplayPage`) is a history page whose
  entries (`display::DisplayEntry`) are `{ordinal?, mid, written_at, row,
  tags?}`: the raw entry's `mid`, `written_at`, `ordinal` where present and
  `tags`, with the canonical `row` in place of `message`. It follows every
  rule of §4.6, and `DisplayPage::check` checks the ones a page shows.
- [pinned] A row (`display::DisplayRow`) is `{role, text, parts, model?,
  client_message_id?, sender?}`. `parts` keeps the stored message's order; each part
  is `{type, part, ...}`, where `part` is the stored part's key (a content
  block's index for `anthropic-messages`, the part's `id` for
  `opencode-aisdk`): `text {text}`, `reasoning {text}`, `tool_call
  {tool_call_id, name, arguments, arguments_truncated?, call_key?,
  schema_pin?}` with `arguments`
  the call's arguments as compact JSON text (`call_key` and `schema_pin`
  only on a runner declaring `call_identity`, §9.16), `tool_result {tool_call_id,
  is_error?, content, truncated?, files?}`, `file {media? | url, mime?,
  filename?}` and `other {kind}` for a part the projection does not show.
  A reader decodes a part type it does not know as `other`. `text` joins the
  text parts as `run.result` does (§6). Media is described by the
  `media_ref` descriptors mode serves (§4.2), never inlined; a URL stays a
  URL.
- [pinned] The row comes from one pure function per serializer profile
  (`display::anthropic_messages`, `display::opencode_aisdk`, or
  `display::project` by name). A runner calls it on the entry's stored body
  and then fills what the body lacks: `model` (the recorded model that wrote
  it; an `anthropic-messages` body names none),
  `client_message_id` (§9.11) and `sender` (below), and on a runner
  declaring `call_identity` each tool call's `call_key` and `schema_pin`
  (§9.16). The conformance kit
  checks that a served row equals the function applied to the same entry's
  raw form, with only those members taken from the served row.
- [pinned] An assistant row carries `model` whenever the runner recorded
  which model wrote that message. A message written through the runner's
  own run always has a recorded model, so its row always carries it. Only
  a message that entered without a recorded model may omit it, for example
  one imported or adopted from another harness's transcript. Absent means
  unknown: a runner never fills it with a guess, a default, or the session's
  current model.
- [informative] Before a runner omits `model`, readers must treat its absence
  as unknown.
- [pinned] **Who sent a message.** A user message's row may carry `sender`
  (added in 0.5.2, `DisplayRow::with_sender`): the attested module id of the
  caller whose send wrote the message (the route principal it arrived on,
  the owner's or a plan-named sender's, §9.7), where the runner recorded it.
  The runner keeps it beside the stored entry, as it keeps
  `client_message_id`, and a runner on a host keeps it in its own
  per-message record. A prompt typed into the host directly has none.
  Neither `sender` nor `client_message_id` ever enters a stored message
  body or the default `session.read` entry: a reader such as a compaction
  provider decodes stored bodies strictly, and a member it does not know
  would fail that decode.
- [pinned] **Cuts.** `limits` bounds, in UTF-8 bytes, a tool call's
  `arguments` (`arguments_bytes`), a tool result's `content`
  (`result_bytes`) and an error result's `content` (`error_result_bytes`).
  A cut ends on the last UTF-8 character boundary at or below its bound
  (`display::cut`) and sets `arguments_truncated` or `truncated`; a payload
  within its bound is served whole and unflagged. Absent `limits` means
  whole content: a runner never cuts by a default of its own. The apps'
  values are `display::APP_LIMITS`, `{arguments_bytes: 2048, result_bytes:
  16384, error_result_bytes: 65536}`.
- [pinned] Pages stay finished-only, as `session.read`'s do (§4.2): there is
  no entry for a message still being generated. Live progress reaches a
  reader through §7.1.
- [pinned] Nothing new rides on stored message bodies or on the default
  `session.read` entry: everything this section adds is on the display row
  or the request.

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
- [pinned] On a runner declaring `call_identity` (§9.16), every display
  tool-call entry and live tool event of a call carries this same
  `call_key`, present exactly when it is present here.
- [pinned] `tool_call_id` is the model's id: display only, not unique.

## 6. `run_ops`: `run.result` and run attribution

- [pinned] `run.result {run_id}` (`RunResultRequest`, strict) returns the
  run's state. For a completed run it also returns the final assistant
  message, with its final values, its text parts joined in order and its
  reasoning parts excluded.
  [pinned] The answer is `{run_id, state, reason?, error?, detail?, final_message?:
  {ordinal, mid, text}, structured_output?, context_ceiling?, usage?, directives?}` (`RunResult`;
  `structured_output` is §9.4's result, on an ended run admitted with it;
  `context_ceiling` is below; `usage` is the run's token usage, §6.4;
  `directives` is §9.9's parsed trailing tag), and a run that has not ended answers
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
  `context_ceiling`, `cancelled`, `error` and `interrupted`, each its own variant of the non-exhaustive `RunState`. States are decoded open: a runner's own state
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
  `budget_exhausted`: each decodes as its own variant
  (`RunState::ContextCeiling`, `RunState::BudgetExhausted`;
  `is_context_ceiling`, `is_budget_exhausted` and `is_terminal` recognise
  them), never `error` and never a `provider_code`,
  because no model or provider failed. Both `run.result` and `run.status`
  carry `context_ceiling: {window, size, cause}`
  (`errors::ContextCeilingDetail`), the detail of the `context_ceiling`
  refusal (§12.1). A caller that routes between models MUST NOT charge
  backoff to the model, or mark it unhealthy, for it. A send that would start
  or join a run while the context is already at or over the ceiling is
  refused `context_ceiling` instead, writing nothing (§9).
- [pinned] **Runner-owned waits** (stated in general in 0.5.2). A run in
  `active` or `paused` that carries `deadline_ms` (the absolute time, in
  milliseconds since the Unix epoch, on `run.status` (§6.1) and on the
  stream event that reports the wait (§7)) is in a wait the runner owns and
  ends by itself, whatever its `reason`. A watchdog or owner MUST NOT nudge
  it, stop it or end it before `deadline_ms`. Once `deadline_ms` has passed,
  this deadline no longer protects the run; a non-empty `held_calls` list
  still protects it independently (§6.1). A caller recognizes such a wait
  by `deadline_ms`, never by `reason` (`RunStatus::runner_wait_until`):
  `reason` stays an open, display-only string, so a wait kind added later
  needs no watchdog change.
  - A runner sets `deadline_ms` only while the run is in such a wait, never
    on a run that only waits for a caller (a `paused` run waiting for a retry
    carries none), and never later than that wait's own bound.
- [pinned] **Waiting at the ceiling** is the first such wait. A run the runner holds at the
  ceiling while it waits for a compaction view (up to the provider's bound,
  300 s, say) is waiting, not stalled. It stays in the non-terminal state
  `active` and names `reason: context_ceiling_wait`
  (`run::RUN_REASON_CONTEXT_CEILING_WAIT`, `RunReason::ContextCeilingWait`)
  on `run.status`, on `session.head`'s `last_run_state` and on the stream
  event that reports the wait (§7), with `deadline_ms`, when the wait ends
  at the latest, on `run.status`
  (`RunStatus::context_ceiling_wait`) and on that event. This reuses the
  shape a non-terminal run already uses to say why it is not moving, a
  state with an open `reason`, as `paused` does with `auth_required`; it is
  not `paused`, because a paused run waits for a caller's retry and changes
  how sends behind it are admitted (§9), while this wait ends on its own.
  As a runner-owned wait, it is not nudged or stopped before
  `deadline_ms`. The wait ends in progress, when a view arrived (the reason
  and deadline are gone), or in the terminal `context_ceiling` state with
  `cause: view_timeout`.
- [pinned] **Waiting for a rewrite ride** (added in 0.5.2) is another. A
  runner holding a step for a rewrite ride (compaction-provider §8: cause
  `ttl_expired`, `model_change` or `surface_change`) reports the wait once
  the ride's answer has been pending for 1 s
  (`run::COMPACTION_WAIT_REPORT_AFTER_MS`): the run stays `active` and names
  `reason: compaction_wait` (`run::RUN_REASON_COMPACTION_WAIT`,
  `RunReason::CompactionWait`) with `deadline_ms`, the absolute time the
  ride's deadline ends (`RunStatus::compaction_wait`), on `run.status`, on
  `session.head`'s `last_run_state` where served (the reason only), and on
  a durable `control` event when the wait starts (§7). The reason and the
  deadline go when the ride resolves: an answer, a refusal or the missed
  deadline. A ride answered within 1 s is never reported, so a quick
  `nothing` never shows. How long the ride may take is the compaction
  provider's choice, within the runner's cap (compaction-provider §4
  `ride_wait_max_ms` and `ride_wait_ms`); the user sees the wait instead of
  a silent pause.
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
  structured_output?, context_ceiling?, deadline_ms?, held_calls?, usage?, directives?}`,
  decoded leniently: a runner's own members ride beside these.
  `indeterminate_tool_calls`, `final_step_finish_reason` and
  `retries_used` stay runner members, not pinned.
  - `state` is a run state as in `run.result` (`run::states`), decoded open
    (`RunState::parse`): a consumer does not treat an unknown state as
    terminal.
  - `reason`, where the runner names one: why a paused run is paused, why
    an active run is waiting, or why an ended run ended. It is an open
    string; the known pause reasons are `auth_required` and `restart`, and
    the known waiting reasons are `context_ceiling_wait` and
    `compaction_wait` (§6).
  - `deadline_ms`: on an `active` or `paused` run in a wait the runner owns
    and ends by itself, when that wait ends at the latest (§6, runner-owned
    waits; `context_ceiling_wait` is one such wait); absent otherwise.
  - `held_calls`: on a runner declaring `held_calls`, the held tool calls
    whose late answers the run awaits, as specified below.
  - `error`: on a run that ended `error`, the same run error `run.result`
    answers (§6), with the same `provider_code` and `provider_detail_code`
    rules.
  - `detail`: on `budget_exhausted`, exactly the detail `run.result` carries (§6).
  - `structured_output`: exactly what `run.result` carries for the run (§9.4).
  - `context_ceiling`: on `context_ceiling`, exactly the detail `run.result`
    carries (§6).
  - `directives`: exactly what `run.result` carries for the run (§9.9).
  - `usage`: the run's token usage so far, on a runner that declares
    `run_usage` (§6.4).
- [pinned] **Held calls** (added in 0.5.4). A runner declares the describe
  group `held_calls` when it reports held tool-provider calls. It requires
  `run_status` and adds optional `held_calls: [{call_key, tool, provider,
  reason, held_at_ms}]` (`run::HeldCall`) to `run.status` and to run status
  events carrying the same state (§7). `call_key` is the dispatched call's
  key, `tool` its name, `provider` the provider module id, and `held_at_ms`
  the Unix epoch milliseconds when the runner received its held reply.
  The list is present only while the run waits on at least one held call;
  an empty list is omitted, preserving the exact bytes of older statuses.
- [pinned] A run whose status lists a non-empty `held_calls` is a
  runner-owned wait with no deadline. A consumer's watchdog MUST NOT nudge,
  interrupt or end that run for lack of progress, however long it waits.
  The owner MAY still cancel or interrupt it on purpose. Consumers decide
  from `held_calls`, never from the run's display-only `reason` string.
  A run MAY carry both `held_calls` and `deadline_ms`; each rule applies
  independently, so expiry of the deadline does not end held-call protection.
- [pinned] Each held call's `reason` is the provider's held reason, an open
  string for display (tool-provider §4.1, `held-reply/v1`). A consumer may
  show that a person must act only for the tool-provider person-wait reasons
  (`cortexkit_role_tool_provider::result::is_person_wait`: `consent`,
  `login_handoff`, `login_capture`). `running` and unknown reasons display
  as waiting, never as requiring a person to act.
- [pinned] The held first reply is interim content, not the tool's final
  answer. The runner pulls the provider's `late_results` (tool-provider §7),
  correlated by `(carrier, call_key)`, and drops the call from `held_calls`
  when it applies that entry. A `result` entry becomes the tool result;
  `not_started` becomes a tool result saying the call never ran, and
  `expired` becomes a tool result saying its answer expired. The run then
  continues with that result. Intake deduplication and acknowledgements
  follow tool-provider §7, including re-reading after an incarnation change.
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
  dispatched_to?, ordinal, position?, result_ordinal?}` (`tool_calls::RunToolCall`):
  - `tool_call_id` is the model's id, display only and not unique (§5);
    `call_key` is the call's key, present once the call was prepared or
    dispatched (§11.3), and the one a `call_identity` runner's display
    entries carry for it (§9.16); `dispatched_to` is the module the call was
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
  - `ordinal` is the `session.read` ordinal of the assistant message that
    requested the call, for open and finished calls alike. An open call is
    listed at the ordinal its requesting message has, or will take once
    written. One message can carry several calls;
  - `position` (`Option<u32>`) is the 0-based index of the call among that
    message's tool-call blocks in planned order, not among all its parts.
    It is optional on the wire so older runners still decode, but a runner
    declaring `run_tool_calls` MUST send it from 0.3.5 on. Callers match an
    entry by `(ordinal, position)`, never by `tool_call_id` (which may repeat)
    or by array order;
  - `result_ordinal` (`Option<u64>`) is the `session.read` ordinal of the
    entry holding this call's result, absent while open and present once the
    result is durably written. A runner declaring `run_tool_calls` MUST send
    it for every finished call from 0.3.5 on. Callers locate a result through
    it, never by assuming a transcript layout. For example, a message-array
    layout such as `anthropic-messages` can store a batch's results in one
    tool-role entry at `ordinal + 1`, in planned order, while an embedded
    layout such as `opencode-aisdk` stores output in the requesting entry's
    tool part, with `result_ordinal == ordinal`. These are layout examples,
    not rules callers rely on. Callers read `outcome` to learn what happened,
    independently of the profile's result encoding.
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

### 6.4 `run_usage`: per-run token usage

A runner that declares `run_usage` reports what its model providers
reported for a run's steps as `usage` on `run.result` (§6) and `run.status`
(§6.1), so a consumer (the phone's agent page, core) shows every worker
run's token usage whichever runner served it.

- [pinned] `usage` is `{steps, unreported_steps, input_tokens?,
  cached_input_tokens?, cache_write_tokens?, output_tokens?,
  reasoning_tokens?, by_model}` (`usage::RunUsage`), decoded leniently:
  - `steps` is the number of completed model steps in the run;
  - `unreported_steps` is the number of those steps whose provider reported
    no usage field at all;
  - each of the five totals is the sum over the steps that reported that
    field;
  - `by_model` is `[{provider, model, variant?, steps, unreported_steps,
    input_tokens?, cached_input_tokens?, cache_write_tokens?,
    output_tokens?, reasoning_tokens?}]` (`usage::ModelUsage`), the same
    counts and totals per model selection.
- [pinned] A step counts as reported if its provider reported any usage
  field, even `output_tokens: 0`. A step that reported nothing counts in
  `unreported_steps`, and in no total.
- [pinned] A total is absent, never `0`, until some step reports it. An
  absent total means "not known". A present total is a lower bound whenever
  `unreported_steps > 0`, or some reported steps omitted that field
  (`reasoning_tokens` reported on only some steps, say). A runner MUST NOT
  write a total no step reported as `0`.
- [pinned] `by_model` has one entry per distinct `(provider, model,
  variant)`, in order of first use in the run; `variant` is absent when the
  selection names none. The entries' `steps`, `unreported_steps` and each
  total sum to the top-level ones; a total absent from every entry is absent
  at the top (`RunUsage::consistent`). A consumer treats a usage that breaks
  this as malformed.
- [pinned] A run with no completed step reports `{"steps": 0,
  "unreported_steps": 0}`, with every total absent and `by_model` empty or
  absent. A decoder reads an absent `by_model` as empty; a usage with
  `steps > 0` MUST carry it. `RunUsage` always encodes it.
- [pinned] `reasoning_tokens` is a breakdown of `output_tokens`, never an
  addition to it: it counts output the provider reports as reasoning, and
  that output is already inside `output_tokens`. So `reasoning_tokens <=
  output_tokens` whenever both are present. A consumer computing cost
  charges `output_tokens` once and never adds `reasoning_tokens` on top. A
  missing `reasoning_tokens` means not reported, not zero. §6.4's totals and
  the `session_usage` totals that share its shape follow this rule.
- [pinned] Usage counts what the provider billed or reported for this run's
  steps only, never steps of earlier runs in the session.
- [pinned] A runner that declares `run_usage` includes `usage` on every
  `run.result` of an ended run, and on `run.status` once a step of the run
  has completed. It MAY include it earlier, with `steps: 0`. Before a step
  has completed, a consumer accepts `usage` present or absent; on a running
  run's `run.result` it is optional.
- [pinned] An absent `usage` means the runner does not report usage (or, on
  a `run_usage` runner, that no step has completed yet). A consumer MUST NOT
  read an absent `usage`, or an absent total, as zero.
- [pinned] `usage` is the same on `run.status` and `run.result` for the same
  run at the same moment.
- [pinned] Vectors: the `run_status.usage_*.json` files in
  `test-vectors/llm-runner-v1/`.

### 6.5 `interrupt_reason`: durable interrupted-run reasons

- [pinned] A runner declaring `interrupt_reason` records a reason durably
  when it seals an `interrupted` run. Known values are `runner_shutdown`
  (`run::interrupt_reasons::RUNNER_SHUTDOWN`), sealed during a planned stop,
  `runner_crash` (`RUNNER_CRASH`), sealed at the next start after the
  runner died, and `host_exited` (`HOST_EXITED`), sealed when the harness
  host the runner drives exited while the runner itself stayed up.
  Runner-specific values are allowed: `reason` stays decoded open, with no
  new wire member.
- [pinned] The recorded value is the same on `run.result`, `run.status` and
  `session.head`'s `last_run_state`, wherever those surfaces are served.
  It survives restart. Absence means none was recorded (including runs
  sealed before the runner adopted this group), never a reason guessed
  from the current process's lifetime. `check_describe` checks the group
  for `run.result`; it adds no discovery metadata.

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
  `reason`, an open, display-only string with known values `auth_required`,
  `restart`, `context_ceiling_wait` and `compaction_wait`, as `run.status`
  does (§6.1). A runner reports a wait it owns (§6, runner-owned waits) with a durable
  `control` event carrying `deadline_ms` when the wait starts, beside the
  wait's `reason`; a reader recognizes the wait by `deadline_ms`, never by
  `reason`. A wait at the context ceiling is reported this way with
  `reason: context_ceiling_wait`, and a step waiting on a rewrite ride with
  `reason: compaction_wait`. A runner declaring `held_calls` also carries
  the same `held_calls` list as `run.status` on status events while waiting
  for held tool calls, and emits a durable `control` status event when that
  list changes. It omits the list when empty. This wait has no deadline;
  readers recognize it by the non-empty list, independently of `deadline_ms`.

### 7.1 `live_events`: typed events on `session.subscribe`

A runner declaring `live_events` (added in 0.5.1) puts a typed `event:
{type, ...}` (`live::LiveEvent`) on its stream frames beside its own
members, such as its `unit`. A reader that predates the group ignores the
new members.

- [pinned] The request may carry `limits` (`live::LiveSubscribeRequest`,
  §4.7's `display::Limits`), which cuts `tool_started.arguments` as it cuts
  a display row's. It is otherwise `session.subscribe`'s request.
- [pinned] **Control events** are durable: each maps onto a record the
  runner already writes, so a replay from a cursor serves them again.
  - `message_started {mid}` and `message_finished {mid}` for every stored
    message, started first. `message_finished` rides the control frame that
    carries the stored message's unit, so a consumer gets the re-read signal
    and the unit together.
  - `tool_started {mid, tool_call_id, name, arguments,
    arguments_truncated?, started_at?, call_key?, schema_pin?}`: `mid` is the message holding the
    call, `tool_call_id` the model's id as the stored row carries it, and
    `arguments` the arguments as JSON text.
  - `tool_finished {mid, tool_call_id, outcome, output_bytes,
    finished_at?, call_key?, schema_pin?}`: `outcome` is an open string (`live::tool_outcomes`: `ok`,
    `error`, `cancelled` when a caller asked, `interrupted` when a restart
    cut the call and it was never sent again, `denied` when an approval or a
    hook refused it); `output_bytes` is the byte length of the result's
    content uncut, as a display row's `tool_result.content` holds it.
  - `run_started {run_id, send_id?}`, `run_finished {run_id, reason}` with `reason` an
    open string (`live::run_reasons`: `completed`, `cancelled`,
    `interrupted`, `max_steps`, `error`, `budget_exhausted`,
    `context_ceiling`, ...), and `run_paused {run_id, reason}` for a run
    that stopped and can resume under the same `run_id`. A runner that never
    pauses never sends `run_paused`; a reader never requires it.
  - `run_started` may carry `send_id`, the send that started the run
    (`live::RunStarted::send_id`). On a runner declaring `run_cause` (added
    in 0.5.2, §3):
    - a run a queued send started names that send's own `send_id`, the one
      its caller sent, never an id the runner made for its queue or
      submission;
    - a run an interrupt started (§9) names the interrupting send, never
      the send of the run it interrupted;
    - `send_id` is left out only for a turn the host started with no send:
      a person's turn on an adopted session (§9.5). It is never an empty
      string and never a guess;
    - a run a named sender's send started names that send, as any send
      (§9.7).
    A runner that cannot name the send that started a send-started run does
    not declare the group. On a runner without it, `send_id` may be absent
    for any run, and its absence says nothing.
  - Times are milliseconds since the Unix epoch.
  - `call_key` and `schema_pin` on `tool_started` and `tool_finished` are
    the call's key and the pin of the definition it was dispatched against,
    on a runner declaring `call_identity` (added in 0.7.2, §9.16), with the
    same values and the same presence rules as on the call's display row. A
    runner without the group leaves both out.
  - `started_at` and `finished_at` (optional since 0.6.0; required in
    0.5.1 to 0.5.5) are the times the runner recorded when the tool call
    started and when it finished. A runner leaves a member out only when it
    recorded no time for that moment, for example a call replayed from
    history stored before the runner kept times. It never invents,
    estimates or substitutes a time: not the current time, not zero, not
    the time of the message holding the call. A present time keeps its
    exact bytes. A call the runner sends to its tool itself has both times
    recorded, so its events carry both. A reader accepts an event without
    either member and shows
    the call without that time; it never reconstructs one
    (`live::ToolStarted::started_at`, `live::ToolFinished::finished_at`).
- [pinned] **Display events** are best-effort and never replayed, in
  Broca's deployed shape `{kind: "display", event: {type, ...}}`:
  `text_delta {message_id, delta, tool_call_id?}`, `reasoning_delta
  {message_id, delta}`, `tool_input_delta {tool_call_id, delta}`,
  `display_gap {dropped}` and `display_reset`. `message_id` is the model
  provider's id and may be empty. On a `live_events` runner the deltas also
  carry `mid` (the message's final `mid`, known from its first event and
  unchanged across a `display_reset`) and `part` (the key the stored row
  gives the part). Deltas are coalesced chunks, never one per token.
- [pinned] `display_reset` means: drop all live state for the step in
  flight (a provider retry, or a write-time hook rewriting a finished
  message); the step's deltas stream again. A reader decodes an event whose
  `type` it does not know, on either lane, as `display_reset`
  (`LiveEvent`'s lenient decoding; `LiveEvent::decode_strict` is the strict
  one).
- [pinned] **Ids.** A consumer replaces a live row with the stored one by
  `mid`, and a live tool row by `mid` plus `tool_call_id`, because the
  model's id alone is not unique in a session.
- [pinned] **Order.** Every frame of one stream carries `seq`
  (`live::SEQ_FIELD`), starting at `live::FIRST_SEQ` (1) and rising by one
  per frame, control and display together. A new stream starts again at 1.
  Control frames keep their durable `cursor`, which is what a consumer
  resumes from. A jump in `seq`, or a `display_gap`, means frames were lost:
  the consumer drops its live rows and attaches again.
- [pinned] **Attach replay.** A newly attached stream first gets the step in
  flight as it stands, after any durable replay its attach point asks for:
  `display_reset`, then one coalesced delta per part so far (with `mid` and
  `part`), then the live deltas, all in the stream's `seq` order. With no
  step in flight there is nothing to replay. It is the only recovery for
  live state.

## 8. `model_view`

- [pinned] Model-view descriptors are not offered. A request combining
  `view: "model"` with `media: "descriptors"` is refused before read work as
  `invalid_params {field: "media"}`. The existing indivisible group rule and
  `reply_too_large` for an over-frame group are unchanged.

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
- [pinned] `delivery` is a string or a list. A string keeps the meaning
  above exactly. A list is the ordered list of delivery modes of the
  `delivery_order` group (§9.7), accepted only by a runner that declares the
  group; any other runner refuses it `invalid_params {field: "delivery"}`.
- [pinned] A send that would start or join a run while the session's
  context is at or over its ceiling is refused `context_ceiling {window,
  size, cause}` and writes nothing (§6, §12.1).
- [pinned] Owner sends MUST carry a `send_id`. `prompt` is optional in the
  type (`SendRequest::prompt`, `Option<String>`) only so that a template
  send can omit it: a send naming a plan `template` (§9.7) carries no
  `prompt` member, and every other send carries a non-empty `prompt`. A
  runner checks this before writing anything (`SendRequest::check_prompt`):
  a template send carrying `prompt`, even an empty one, and a send without a
  template and without a non-empty `prompt`, are refused `invalid_params
  {field: "template"}`.
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
- [pinned] **Authority stays the owner's** (stated in 0.5.2 for every
  runner). A run that a non-owner's send starts or joins (a plan-named
  sender's, §9.7, or an unkeyed non-owner send above) runs under the
  session owner's authority: the owner's principal and the session's frozen
  scope, never the sender's. Its tool calls carry the owner's principal. The
  sender's identity decides only whether the send is admitted, and is
  recorded beside the message it wrote (§4.7 `sender`).
- [pinned] The `delivery` value is strict: an unknown mode is refused naming
  `delivery`, never read as `queue`. Except for the scoped closed `budget`
  rules on an `input_token_cap` runner (§9.3), the strict
  `structured_output` member (§9.4), the declared run-limit members (§9.3.1)
  and the strict `directives` member on a `turn_directives` runner (§9.9)
  and `delivery_order` members
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
- [pinned] A delivered steer. For any `SendReply` to a send delivered as a
  steer into a running run, including the first reply, a re-send, and
  `send.status`, `state` is the run's state as seen by the runner (`active`
  while the turn runs, `finished` once it has ended), and `delivered.as` is
  `step` with a required `ref` naming the stored delivered message. `step` is
  a value of `Delivered::as`, never of `SendReply::state`; `state: "step"` is
  an unknown send state. `run_id` is required when the runner declares
  `run_ops`, and absent when it does not. A caller that sees
  `delivered.as: "step"` without `run_id` reads it as delivered into a run
  the runner does not expose and stops following there: the absent member is
  neither an error nor a missing-field failure, and the caller never polls
  `run.status` for that delivery.
- [pinned] A re-send's `delivered` moves forward only: from absent or
  `pending` to `step`, `turn` or `unknown`. Those three are final: once one
  first appears, every later answer to the same `send_id` carries it
  unchanged, `ref` included. Absent to `pending` is not a move on a
  `confirm` runner, where absent already means `pending`; a `guaranteed`
  runner never answers `pending` at all.
- [pinned] Following a queued send. A send answered `pending` with a
  `submission_id` (a `queue` send behind a running run, or a
  `delivery_order` list waiting for a boundary, §9.7) is followed by
  re-sending the same `send_id`, or by `send.status` on a runner that
  declares `send_status` (§9.8). Once it is delivered, the answer says how
  and, where this contract requires it, which run (`SendReply::delivered`,
  `SendReply::run_id`):
  - delivered as a steer into a running run: `delivered: {as: "step", ref}`
    identifies the stored row and follows the `run_id` rule above;
  - delivered by starting a run on an idle session: `delivered: {as:
    "turn", ref}`, `ref` the run id, and `run_id` that same run;
  - handed to a run but not confirmed delivered: `delivered: {as:
    "unknown"}`, with `run_id` the run it was handed to when the runner
    knows it, absent otherwise.
  A queued send delivered with a `tool_result` or `turn_system` mode
  (§9.7) rode a step of a running run and is answered as a steer.
- [pinned] These three answers are durable. They are §9's "final" rule
  made explicit for a queued send: the runner answers them from its durable
  record of the send, so once one is given, a runner restart never turns it
  back into `pending` or absent, and every later answer to the same
  `send_id` carries the same `delivered` and the same `run_id` when that
  receipt requires one. `unknown` stays `unknown` even if the delivery is
  later proven.
- [pinned] A caller MUST NOT derive a run id from a `submission_id`.
  `submission_id` is opaque, it names a submission and never a run, and
  `run.status` on an id that names no run answers `{state: "unknown"}`
  (§6.1), which a caller would misread as a run of unknown state. When a
  reply carries `run_id`, it names the run that carried the delivered send.
  For a delivered `step` without `run_id`, the caller stops following as
  above.

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

A runner that declares `retract` serves `session.retract`.

- [pinned] A send that has not started is withdrawn before it starts. A
  `delivery_order` send that is still waiting for its next possible boundary
  (§9.7) is queued too: for example, `[tool_result]` waits for the next tool
  result. It remains pending until that boundary; on a runner declaring
  `retract`, its reply is `state: "pending"` with `submission_id`, and
  `session.retract` withdraws it until it is delivered. A pending reply to
  any send carries `submission_id` exactly when the runner declares
  `retract`. On a
  `confirm` runner, a steer whose receipt is `pending` is retractable too. On
  a `guaranteed` runner an accepted steer is already durable in the run it
  joined, is answered without `pending`, and `session.retract` does not
  withdraw it.

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

### 9.3.1 `max_steps`, `max_output_tokens`, `no_tools`: declared run parameters

- [pinned] These are three independent describe groups. A runner declares
  exactly the ones it honours. A caller MUST send `stop_when.max_steps`,
  `generation.max_output_tokens` or `tools: []` only to a runner declaring
  `max_steps`, `max_output_tokens` or `no_tools`, respectively.
  `check_describe` checks each group for `session.send`; none adds discovery
  metadata. Receiving an undeclared member keeps §9's existing rule: the
  runner honours it or refuses `invalid_params` naming the field, never
  silently ignores it.
- [pinned] The wire positions do not change. The Rust `SendRequest` keeps
  the complete raw members in `runner_params["stop_when"]["max_steps"]`,
  `runner_params["generation"]["max_output_tokens"]` and
  `runner_params["tools"]`, exactly as received. There are no separate typed
  fields. `SendRequest::max_steps()`, `max_output_tokens()` and `no_tools()`
  inspect without changing those values and report the offending field path
  on malformed input. `with_max_steps`, `with_max_output_tokens` and
  `with_no_tools` write those same positions; the limit builders preserve
  sibling members. Decoding alone preserves even malformed legacy values.
- [pinned] `stop_when.max_steps` is a positive u64 integer. One step is one
  model response. A run ends `max_steps` only when the limit cut it off:
  the step that reached the limit requested at least one tool call that
  would lead to another step. §6's hard-stop rule then seals every pending,
  undispatched call with its never-executed result in the same durable
  commit.
- [pinned] A step that reaches the limit with no tool call ends the run
  `completed`, with its final message, directives (§9.9) and structured
  output (§9.4), as any completed run does. A step that reaches the limit
  and calls `submit_result` ends the run as `submit_result` always does:
  `completed` with the submitted value, its sibling calls
  `skipped_after_submit`. Neither is `max_steps`.
- [pinned] On a runner
  declaring `max_steps`, a value that is not a representable positive u64
  integer (including zero, null, negatives, non-integers and overflow) is refused `invalid_params` naming
  `stop_when.max_steps`; a non-object `stop_when` names `stop_when`.
- [pinned] `generation.max_output_tokens` is a positive u64 integer, frozen
  at admission, that caps each model response. The effective cap is the
  minimum of the request and the model's own output maximum: never above
  the request, and never refused because the model's maximum is lower.
  On a runner declaring `max_output_tokens`, a value that is not a
  representable positive u64 integer (including zero, null, negatives,
  non-integers and overflow) is
  refused `invalid_params` naming `generation.max_output_tokens`; a
  non-object `generation` names `generation`.
- [pinned] `tools: []` means the model is offered no tools at all, including
  plan tools and host built-ins. Only the empty list has role meaning in
  `no_tools`; a non-empty list stays runner-specific. A non-list is refused
  `invalid_params` naming `tools` on a runner declaring the group. A runner
  that freezes tools at admission (a session with a fetch plan) refuses
  `tools` on any later send with `invalid_params` naming `tools`, rather
  than ignoring it, even when the value is unchanged.
- [pinned] A joining steer whose limits differ from its run's is refused
  `invalid_params`, naming the first differing field, under §9's existing
  frozen-settings rule.

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
  person may be mid-turn in their app, and that run continues, observable
  as every run is (below).
- [pinned] Person-started turns are runs. On a session the runner serves
  through `adopt`, every turn the host starts, a turn the person starts in
  their own app included, is a run: it has a `run_id`, it appears on
  `session.subscribe` like any run (§7), and it answers `run.status`
  (§6.1) and `run.result` (§6), its `directives` (§9.9) included, wherever
  the runner declares those groups. `directives` follow §9.9 unchanged: a
  run whose admission named no directive reports none, so a host turn
  reports one only where the host admits its turns with the names. It carries no `send_id`, and that is
  how the owner tells it from its own runs: there is no origin field. On a
  runner declaring `run_cause` (§7.1), its `run_started` event is the one
  `run_started` that carries no `send_id`.
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

### 9.6 `tags`: runner-rendered part numbers

The runner renders the transcript, so it shows the model short names for
its parts: tags, `§N§`. A consumer (a context manager, a model referring to
an earlier message, a tool using part of an earlier output) names a part by
the number it was shown, and the runner answers with that part. A tag
number is only a name the model can see, computed from the transcript.
Everything durable is keyed by the host's own identity: a message id, the
part's key and its content digest, never a number.

**Scope.**

- [pinned] Numbering and rendering apply only to a session whose frozen
  plan renders tags (`render_tags: true`, below). On a session whose plan
  does not, every `session.read` part row carries its key and no number,
  `session.resolve` refuses every well-formed tag `unknown_tag`, and the
  model's text is exposed as written.
- [pinned] Cleaning (below) is per message: each piece of the model's text
  is cleaned according to the frozen plan of the run that wrote it,
  whatever the current run renders. A session that later stops rendering
  tags still has its earlier replies cleaned, and text written by a run
  that rendered no tags is never cleaned on any exposure.

**Which parts count.**

- [pinned] A part counts if and only if the runner renders it with a
  visible number. One predicate drives both counting and rendering, so the
  two cannot drift apart: a runner maps each part it renders onto
  `tags::PartKind` and asks `tags::counts`.
  - Counted: user text, a tool result with non-empty exposed text (error
    text included), the model's own text, and a tool result holding only
    images apart from empty text blocks, which renders
    a short text label carrying its number (`tags::image_label`, `§N§
    [image]`) where the provider accepts text beside the image (on
    Anthropic's API, a text block before the image inside the
    `tool_result`), so a context manager can reach a screenshot.
  - A tool result is one part, whatever its blocks: a compaction provider
    keeps or drops the whole result, never a block inside it. Empty exposed
    text blocks are ignored for counting and number placement. With no
    non-empty text, images alone select `ToolResultImageOnly`; any non-image
    media (a PDF or other file, alone or beside images) selects
    `ToolResultMediaOnly`, counted once with `tags::media_label` (`§N§ [media]`).
    `tags::tool_result_kind` classifies the whole result.
  - A tool result with neither non-empty exposed text nor media is
    `EmptyText` and is not counted. This includes a result carried only as
    a structured value with no text block, such as a legacy reply with only
    `structuredContent` or an imported transcript's JSON tool result: some
    providers receive the value as an object, not text, so no number can be
    written on it consistently across providers. A text block whose text
    happens to be JSON is ordinary text and counts.
  - A steer the runner stores as a user message counts, as a reminder or a
    delivered notice stored that way does. A `turn_system` message (§9.7)
    does not.
  - Never counted: images and files in a user message, reasoning and
    thinking blocks, tool calls (the call's result carries the number), the
    history head, compaction-marker messages, and a text part whose
    exposed text is empty.
- [pinned] The same parts count on every provider, including those whose
  text parts carry provider state (a Gemini thought signature, ChatGPT's
  `phase`): the model must be able to predict its own next number. Seeing
  `§100§` and `§101§`, its next text part is `§102§`.

**Numbers.**

- [pinned] A part's number is the checkpoint plus its position
  (`tags::try_number_parts`). The checkpoint counts the counted parts before
  the cut, the first retained message. Positions run in the order the
  runner renders the history from the cut, never in storage order: a steer
  stored while a tool call ran but rendered after that call's result is
  numbered after the result. The computation is deterministic, so no
  per-part ledger is stored.
- [pinned] A runner refuses a checkpoint whose numbering would pass
  `u64::MAX`, and never repeats a number: `tags::try_number_parts` answers
  `NumberingOverflow {checkpoint, counted}` instead of numbers.
  A numbering that ends exactly at `u64::MAX` is accepted. The older
  `tags::number_parts` saturates there, giving two parts one number, and is
  deprecated. `tag-numbering.json` pins both (`overflow`).
- [pinned] The cut is keyed on the field, not on who wrote the marker
  (`tags::Marker::cut`): a marker's `tail_start_id` when present, otherwise
  the message holding its compaction part. That covers standalone Magic
  Context's markers and native markers that omit `tail_start_id` when they
  retain nothing. A marker not in effect is never an anchor.
- [pinned] The anchor falls back in this order (`tags::anchor`): the newest
  in-effect marker carrying a checkpoint for this session, then earlier
  in-effect markers carrying one, then the Setup checkpoint at its own cut
  message (`tag_checkpoint`, below). Only when that message no longer
  exists, after a revert past it, does the runner number from the session
  start with the Setup checkpoint's number, on a new lineage. With no
  anchor at all, it numbers from the session start at zero.
- [pinned] A fallback renumbers nothing. When the anchor falls back past a
  newer in-effect marker that lost its key, to an older keyed marker, to the
  Setup checkpoint or to the session start, numbering counts every counted
  part from that anchor's cut, the parts the newer markers summarized
  included, by kind exactly as they were counted when rendered, in the
  order they rendered while the anchor's marker was the newest (on OpenCode,
  storage order with marker pairs skipped): the runner feeds
  `tags::try_number_parts` those parts, not only the ones it renders now. A
  compaction message and its summary never count, wherever they sit and
  with or without the key; a dropped part keeps its placeholder number and
  still counts. So the numbers equal the ones computed with every key
  present, the numbers the model saw, and the lineage does not change,
  apart from the Setup cut gone rule above. `tag-numbering.json` pins three
  shapes: the newest key lost with an older keyed marker; every key lost,
  falling back to the Setup checkpoint or to the session start; and a newer
  cut inside the older marker's retained tail.
- [pinned] A runner writes the checkpoint on its own compaction markers,
  in the same write as the marker and never sent to the model:
  `{tag_checkpoint, session}` (`tags::MarkerCheckpoint`), naming the
  session it was written for. On OpenCode it is one extra key on the
  compaction part, `"cortexkit"`; never the summary's metadata, which
  OpenCode forwards to the provider. A marker that lost its key is no
  anchor, never a checkpoint of zero.
- [pinned] Gaps exist only behind the tail: a checkpoint may count parts no
  longer rendered, or numbers an earlier tagger handed out. A part a
  compaction provider dropped keeps its number, rendered as its
  placeholder `[dropped §N§]` (`tags::dropped_placeholder`), so it is
  still counted.
- [pinned] A runner never walks the whole session to number it. It keeps
  the tail's numbers and the next number in memory, and what it holds
  always equals a fresh computation from the anchor: a revert or a reorder
  recomputes from the cut, never just keeps counting upward.
  `session.resolve` never answers a number inside summarized history: a
  number that names a part before the lineage's first ordinal is refused
  `before_lineage_start` (§4.5), which the runner decides from the anchor's
  checkpoint without walking anything. Raw recovery of such a part reads
  `session.history` (§4.6).

**Lineage.**

- [pinned] Any renumbering of a part a read has already exposed starts a
  new lineage (§4.2): a revert, a recomputed reorder, the move of a session
  from standalone Magic Context (whose every visible number changes once),
  a detected restore, and the fallback to the session start above. A switch
  from a stored ledger (below) renumbers nothing and keeps the lineage.
  Within one lineage a number names one part, and a re-read answers the
  same numbers.
- [pinned] A cut starts a new lineage too (§4.5), though it renumbers
  nothing: the new lineage numbers from the cut's checkpoint, as the
  numbering rules above say, so every part the new lineage still shows
  keeps the number it showed before the cut, and tags count from the cut as
  0.4.x specified. Numbers may repeat across lineages; a number reused after a
  revert is fine. Undo, revert and rewind stay within the session as
  lineage changes.
- [pinned] A renumbering the runner detects during a step takes effect at
  the next step boundary, never before that step's tool calls have run.
  So between a model reply and its tool calls, the current lineage is the
  one the model saw. That is what lets a context manager resolve a number
  in a tool call against the session's current numbering, locally and
  without naming the numbering it means: `session.resolve` carries no
  anchor for this reason. A runner that could change lineage within a turn
  would need that anchor back, and this contract does not give it one.

**`tag_checkpoint`: the Setup checkpoint.**

- [pinned] A runner that declares `tag_checkpoint` (it requires
  `compaction`, §3) honours a Setup checkpoint `{tag_checkpoint, cut}`
  (`tags::SetupCheckpoint`) on its compaction provider's Setup answer
  (compaction-provider §4): it anchors the number at the `cut` message,
  used only when no in-effect marker carries a checkpoint for this
  session, so the numbers are the same with or without the marker's key.
  If the cut message is gone (a revert past it), the runner numbers from
  the session start with that number, on a new lineage. The checkpoint is
  permanent for the session, needs no report back, and survives a lost
  marker key and runner restarts.
- [pinned] Setup runs once per lineage, before the lineage's first model
  call (§11.1), so each Setup checkpoint the runner records belongs to
  exactly one lineage: a lineage's numbering is anchored only by the
  checkpoint its own Setup answered, never by one recorded for another
  lineage.
- [pinned] A Setup checkpoint comes only on a Setup without
  `descends_from` (compaction-provider §4): a session moved in from
  standalone Magic Context, a Claude Code conversation coming off the
  full-array route, a runner that lost its state, and a revert below a cut
  made before the runner adopted the session. On a lineage a cut starts,
  whose Setup carries `descends_from`, numbering continues from the old
  lineage's highest number: the parts the cut retained keep their numbers,
  and the next counted part takes the number after the old lineage's
  highest. A runner that receives `checkpoint` on a Setup with
  `descends_from` treats the answer as a failed Setup: no model call, and
  the run ends `error` with `provider_code` `compaction_unavailable` and
  `reason` `checkpoint_on_descent` (compaction-provider
  `setup::CHECKPOINT_ON_DESCENT`).
- [pinned] While a tag-rendering run waits for the answer to a Setup
  without `descends_from`, whose checkpoint may move the numbering anchor,
  a runner may refuse `session.read` and `session.resolve` `transient`.
  It never does so while a Setup with `descends_from` (a cut's) is pending:
  a cut's Setup never interrupts readers.
- [pinned] A compaction provider refuses Setup for a session it moved from
  standalone Magic Context, on a runner without `tag_checkpoint`, with the
  terminal `tag_checkpoint_unsupported` (compaction-provider §10).
- [pinned] For a session with standalone state and no completed move, the
  provider refuses Setup `standalone_session_not_migrated`. Where standalone
  runs in-process, a runner that has never served the session hands it to
  standalone in the same step; one that has served it, and every runner on
  a host without standalone, fails the run by name and never forwards the
  unmanaged history (compaction-provider §4).

**Forks.**

- [pinned] A session the runner knows was forked or cloned from a managed
  session (one whose plan names a compaction provider) is never enrolled.
  The runner refuses `session.send` and `session.adopt` for it with the
  terminal `fork_unsupported {parent?}` (§12.1,
  `errors::ForkUnsupportedDetail`), writing nothing, with a message such as
  "Forks aren't supported for Magic Context sessions; use undo instead".
  An in-process host refuses the person's own turn the same way.
- [pinned] A runner refuses only forks it can know about. On OpenCode a
  fork is known by a copied marker checkpoint naming another session,
  which is the `{parent}` (`tags::anchor` answers it); a fork of a session
  that never compacted carries nothing of the runner's and is an ordinary
  session.

**Switching from a stored ledger.**

- [pinned] One rule for every runner that numbered parts from a stored
  ledger before (`runner-tags/v1`, or a gateway's own ledger): it switches
  at a session's next run, or, on a runner whose sessions advance by user
  turns, its next user turn. Parts already written or admitted keep their
  stored numbers, so no exposed part is renumbered and the lineage stays
  the same. The first run (or user turn) under these rules records the
  ledger's high-water as its starting checkpoint, on that run's frozen
  configuration, anchored at that run's first part; numbers are computed
  from there, by these rules, on the same lineage. A compaction cut before
  that anchor covers only parts numbered by the ledger; a cut after it
  counts from the anchor.
- [pinned] A detected restore of such a runner's store is not a switch: it
  starts a new lineage, numbered from the restored high-water.

**What the model writes.**

- [pinned] The model's own tag notation is cleaned by a port of the tag
  cleaning in standalone Magic Context, the plugin that manages context inside
  OpenCode and Pi (`tags::clean_model_text`, from its
  `tag-content-primitives.ts` and `text-complete.ts`): remove a leading run
  of `§N§`; remove every complete `§N§` anywhere; remove malformed `§N">§…`
  shapes; remove a `§N` closed with an improvised character, but never a
  `§` followed by digits, then `.` and a digit (a section reference such as
  `§5.1` or `§12.3`, at any digit count); remove any stray `§`, then trim.
  Bare digits are never removed. The function is idempotent. Standalone's
  own rule backtracks one digit before a decimal (`§12.3` cleans to `2.3`
  there). This port instead preserves the whole section number, so the two
  differ for a `§` followed by two or more digits and a decimal.
- [pinned] Cleaning runs on every exposure (`tags::exposed_model_text`):
  `session.read`, `session.resolve`, the final text of `run.result`, every
  render, and the summarizer's feed. Every reader and the model see the same
  text produced by the shared cleaning function. Storage may keep the raw
  text.
- [pinned] An in-process host MUST also clean at its own save seam
  (OpenCode's `experimental.text.complete`, Pi's `message_end`), because its
  own UI and other plugins read the host store outside the runner. It
  cleans only a reply whose writing run rendered tags, judged by that
  message's own record of its run's frozen plan, never by the session's
  current setting, and never a structured-output reply. Exposure stays
  authoritative: a seam that does not fire (an aborted reply, a hook error
  the host swallows) leaves raw text only in the host's own copy, never in
  anything the runner exposes, and cleaning text that is already clean
  changes nothing. This does not apply to the Claude Code gateway, which is
  not an in-process host and has no save seam: Claude Code's own file keeps
  the raw text, and the gateway cleans on every exposure.
- [pinned] Never cleaned, at any seam: signed thinking, encrypted
  reasoning, a tool call's arguments (dispatched exactly as the model wrote
  them), and the final text of a structured-output run (§9.4), which
  `run.result` answers from the runner's own record of the raw text.
- [pinned] The cleaning applies on every provider: no provider-state
  exception remains.

**Freeze.**

- [pinned] The numbering and cleaning rules are frozen per run under one
  named feature, `runner-tags/v2` (`tags::RULES_FEATURE`), recorded with
  the rest of the run's frozen configuration. A run keeps the rules it was
  frozen under for its whole life.

**Reads.**

- [pinned] On a runner that declares `tags`, every `session.read` entry
  carries `tags: [{part, tag?}]` (`tags::PartTag`), one row per part the
  runner exposes, in the message's part order, beside the message as
  `tool_calls` are: `tag` is the number the part shows in the current
  lineage, present on a counted part of a tag-rendering session and absent
  otherwise, so a reader learns every part's key from the rows alone.
  `part` is the runner's own key for the part within its message (an index
  or a part id), opaque to consumers: the host identity a consumer keys
  its durable state by, and the one `session.resolve` answers with. Once a
  part is exposed its key never changes. Decoding stays lenient: a reader
  of a runner without `tags` finds no `tags`. Where a message came from
  (its attestation) is not in this revision.
- [pinned] Content digests ride separately from tags (§4.2): required in
  descriptors mode and optional in inline mode.

**Resolve.**

- [pinned] `session.resolve {tag, range?, media?}` (`tags::ResolveRequest`) is
  strict: an unknown field is refused `invalid_params` naming it, and the
  request names no session (§1). `range` is `{start, end}`
  (`tags::CharRange`), a half-open range of the part's content counted in
  Unicode scalar values: not bytes, and not UTF-16 code units, so a range
  names the same text in every language.
- [pinned] The answer is `{tag, ordinal, mid, part, content, range?}`
  (`tags::ResolveReply`), decoded leniently: `content` is the part's
  exposed content, or the requested range of it (`tags::slice`): a text
  part's text, cleaned when it is the model's text a tag-rendering run
  wrote; an image-only tool result in the runner's own stored encoding of
  it. `range` echoes the request's. `media` defaults to `"inline"` and accepts
  only `"inline"` and `"descriptors"`, refusing other values with
  `invalid_params {field: "media"}`. Inline keeps the existing string content
  and frame refusal behavior. Descriptors uses the same in-place ref shapes,
  threshold and fixed budget as §4.2; `content`
  may then be a JSON ref or a structured part with refs. Character ranges are
  validated and applied to exposed text before describing that text.
- [pinned] `session.resolve` answers about the session's current lineage:
  the part that shows the number now, with its current `ordinal` and
  `mid`. A number that names a part below the lineage's `first_ordinal` is
  refused `before_lineage_start` with `detail.first_ordinal` (§4.5), never
  answered.
- [pinned] A malformed tag is refused `invalid_params {field: "tag"}`. A
  well-formed tag that no part of the current lineage carries is refused
  `unknown_tag`, terminal (§12.1). Resolve never answers a left lineage's
  content. A range whose `start` is after its `end`, or whose `end` is past
  the content, is refused `invalid_params {field: "range"}`; an empty range
  within the content answers `""`.
- [pinned] Resolve writes nothing.

**Rendering.**

- [pinned] With `render_tags: true`, the runner renders a counted text
  part as its number, one space (U+0020), then the part's text with any
  leading tag notation stripped (`tags::render_counted_text`,
  `tags::RENDER_SEPARATOR`): only `§`-shaped notation, never bare digits,
  in up to eight rounds (`tags::strip_leading_tag_notation`). Rendering
  what it rendered gives the same bytes again. A tool result's number
  prefixes its first non-empty exposed text block (`tags::numbered_block_index`);
  later text blocks and all media blocks render as exposed, unnumbered.
  Thus `[text, image, text]` takes one number on block 0, `[image, text]`
  and `["", text]` put it on block 1, and `[image, ""]` uses the image label.
  A tool result without non-empty text renders its image-only or media-only
  label once, where the provider accepts text beside media; a part that does
  not count renders as exposed, with
  no number (`tags::render_part`). A compaction provider builds the same
  bytes for its placeholders. `tags.json` and `tag-cleaning.json` pin
  rendered parts.
- [pinned] Whether tags are rendered to the model is the session's fetch
  plan's `render_tags` member (`tags::PLAN_RENDER_TAGS`), composed by core
  in the same plan object `SendRequest::plan` carries (§10.1): `true`
  numbers, renders and cleans by these rules; absent, `null` or `false`
  does none of it. A plan carrying a non-boolean, or `render_tags: true` to
  a runner without `tags`, is refused at admission `invalid_params {field:
  "plan.render_tags"}`, writing nothing (`tags::check_plan`,
  `SendRequest::check_render_tags`). `render_tags` is part of the plan, so
  it is frozen with it, and a later plan that differs in it is
  `plan_drift` like any other difference.

### 9.7 `delivery_order`: ordered delivery modes, plan senders and templates

A send does not always want to become a turn. A module's reminder should
ride the next request the session makes anyway, never start one; text a
plan vouches for may go out with system authority. `delivery_order` lets
a send say, in order, how it may be delivered, and lets the session's plan
say who may send and which texts are system text.

**The list.**

- [pinned] A runner that declares `delivery_order` accepts `delivery:
  [mode, ...]` on `session.send`: the ordered list rides in `delivery`
  itself, as an array (`SendRequest::delivery_order`; read with the members
  below as `delivery_order::DeliveryOrder`), with modes
  (`delivery_order::Mode`):
  - `tool_result`: rendered with the next tool result, the first time that
    result is sent;
  - `turn_system`: a turn-scoped system message (Anthropic's `clear_at:
    next_user_message`), for template text only (below);
  - `steer`: a user message into the run in progress, at its next step;
  - `idle_start`: a user message that starts a run on an idle session.
- [pinned] `delivery` is decoded once, as a string or a list
  (`SendRequest`): a list that is not of these modes, empty, or naming a
  mode twice is refused `invalid_params {field: "delivery"}`, writing
  nothing, as an unknown string is.
- [pinned] For a list, `tool_result` and `turn_system` are modes of the
  `delivery_order` group; `steer` requires the `steer` group, and `idle_start`
  requires the `queue` group (§3). The `steer` group does not require `run_ops`,
  since a runner may land steers into turns it does not own (the delivered-steer
  rule in §9).
  Without `role.describe.delivery_order_modes`, this is the unchanged mapping
  (`delivery_order::Mode::is_declared`). When the member is present, only its
  listed modes count as declared, still requiring the groups above
  (`Mode::is_declared_by`): the member only narrows, never adds a capability.
  For example, a runner declaring `delivery_order` and `steer` with
  `delivery_order_modes: ["steer"]` declares only `steer` for lists, not
  `tool_result`, `turn_system` or `idle_start`, even if it also declares `queue`.
  This member does not narrow legacy string delivery modes.
  `interrupt` has no list form. A structurally valid list with no declared
  member after this narrowing is refused at once as `delivery_unsupported`, with
  `detail.delivery` equal to the list as sent, and writes nothing. A list
  with at least one declared member is accepted; each undeclared member is
  impossible at every boundary and is skipped, never delivered in another
  mode.
- [pinned] At each request boundary, the runner takes the first mode in the
  list that is possible there (`delivery_order::choose`,
  `delivery_order::Boundary`): `tool_result` needs a tool result about to be
  sent for the first time; `turn_system` needs a provider that supports it,
  a runner that owns the request, and the authority rule below; `steer`
  needs a run in progress; `idle_start` needs an idle session. When none is
  possible, the send stays queued, durably, for the next boundary.
  Without `delivery_order_modes`, `turn_system` counts as declared wherever
  `delivery_order` is, even on a runner whose provider can never take it, so
  a sender lists at least one user-authority mode after `turn_system`: a list
  of `turn_system` alone can wait on such a runner until the send expires. A
  runner that can never land `turn_system` should advertise
  `delivery_order_modes` without it, so such a list is refused at once.
- [pinned] A send participates only in boundaries rendered after the
  request-rendering component has staged it. For a runner beside its host, this is when the
  host-side component has staged the send, which may be later than acceptance;
  for a runner owning its loop, the two coincide. A tool result sent before the
  send was staged is not "about to be sent for the first time" for that send.
  The send waits for the next tool result, or falls through in list order at a
  later boundary, as when no tool result is coming.
- [pinned] Staging is not delivery. A staged send still answers `pending`, is
  retractable until its first delivery, and still expires. A retract racing a
  render either withdraws the send before it is rendered or reports it
  delivered, never both.
- [pinned] Acceptance never promises the next boundary.
- [pinned] A `delivery_order` send still waiting for its next possible
  boundary is a queued send under §9.2. It answers `pending` with a
  `submission_id` on a runner that declares `retract`, and `session.retract`
  withdraws it until the first delivery. For example, `[tool_result]` waits
  while a tool call is held and no result is ready to send.
- [pinned] **A list without `idle_start` never starts a run.** It waits,
  queued, for a request the session makes anyway.
- [pinned] A string `delivery` is unchanged, and maps onto lists
  (`delivery_order::legacy_equivalent`): `queue`, or an absent `delivery`,
  is `[idle_start]` (it waits durably and starts the next turn once the run
  ends); `steer` is `[steer, idle_start]`. `interrupt` has no list form and
  stays its own mode.
- [pinned] **Why the overload is safe.** A runner that does not declare the
  group refuses a list in `delivery` with `invalid_params {field:
  "delivery"}`, writing nothing (`SendRequest::check_delivery_order`), and
  an older runner's strict decode of `delivery` already refuses an array the
  same way. A list therefore never goes out as a legacy `queue` on a runner
  that cannot honour it: the send fails loudly instead.
- [pinned] The members `dedupe_key`, `expires_at_ms` and `template` are
  valid only on a send whose `delivery` is a list
  (`delivery_order::MEMBERS`). On a runner that declares the group, a send
  carrying any of them with a string or absent `delivery` is refused
  `invalid_params` naming that member, writing nothing.
- [pinned] The list, with its members, is part of the send's identity
  (§9): the same `send_id` with another list is refused `send_id_reuse`
  naming `delivery`. A retry with the same `send_id` is the same send
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
  `submission_id`, as a queued send does today (§9). Once delivered, a
  re-send of its `send_id` (or `send.status`, §9.8) answers how it went out,
  durably, as §9 says for following a queued send: `step` with `ref` the
  stored row id and `run_id` required exactly when the runner declares
  `run_ops`, or `turn` with `run_id` the run it started. A `step` receipt's
  `ref` is required even when `run_id` is absent. One handed to a run but not
  confirmed delivered answers `unknown`, with `run_id` that run when known,
  just as durably.

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
  digest, slots}` (`delivery_order::TemplateUse`, strict) and no `prompt`
  member at all; a template beside a `prompt`, empty or not, or a send
  without a template and without a non-empty `prompt`, is refused
  `invalid_params {field: "template"}` (`SendRequest::check_prompt`, which
  `SendRequest::delivery_order` applies).
- [pinned] The runner renders the template from the plan's template for
  that sender and the send's slots, validating every slot itself
  (`delivery_order::render`); the sender's word is never enough. A send
  whose template cannot render has no text, so it is refused at admission
  `invalid_params {field: "template"}`, writing nothing
  (`delivery_order::TemplateProblem`): the sender's plan entry declares no
  template with that id, its digest differs, a slot is missing or
  undeclared, or a value is not a value of its slot (the wrong type, out of
  range, a `part_tag` value that is not `{mid, part}`, an unlisted enum
  value).
- [pinned] A `part_tag` slot carries a part's host identity `{mid, part}`
  (`delivery_order::PartRef`), never a number, and the runner renders the
  number the request carrying the send shows for that part (§9.6). Core
  passes the slot through without validating it. A part the request does
  not show ends the send terminally with `template_part_gone {slot}`
  naming the slot (`errors::TEMPLATE_PART_GONE`,
  `TemplateProblem::PartGone`): refused at admission, writing nothing, when
  the part is already gone, and, for a queued send whose part goes before
  it is delivered, the terminal send state `template_part_gone`
  (`send::states::TEMPLATE_PART_GONE`), never delivered, which a re-send of
  its `send_id` (or `send.status`) answers with `slot`, as an expired send
  answers `expired`. The sender recomposes on its next pass. Once a
  template send is delivered, its rendered text is stored and resent byte
  for byte.
- [pinned] `tool_result`, `steer` and `idle_start` deliver at user
  authority. `turn_system` is possible only for a rendered template, and
  only where the provider supports a turn-scoped system message and the
  runner owns the request. Where it is not, the runner falls through to the
  next mode in the list, carrying the same rendered text at user authority.
  A send without a template never goes out as `turn_system`.
- [pinned] A stored `turn_system` entry is part of the session's history:
  its stored bytes, role and `delivered_as` mode never change.
  If a later request goes to a model or provider that cannot accept a
  turn-scoped system message (after a model switch, for example), the runner
  renders that entry's bytes in the request as user-authority text. It never
  drops the entry or fails the request because of it. This changes only what
  the provider receives, never the stored entry or what `session.read`
  returns; it is the same authority fall-through as above, since
  `turn_system` carries template text only.
- [pinned] A template is `{id, text, slots}` (`delivery_order::Template`):
  `text` holds `{name}` placeholders, `name` in `[a-z0-9_]+`, and no other
  brace; every placeholder names a declared slot and every slot is used. Its
  digest is lowercase hex SHA-256 over its RFC 8785 (JCS) bytes
  (`delivery_order::template_digest`). Slots are typed
  (`delivery_order::Slot`), and none takes free text: `integer {min, max}`
  (inclusive), `percentage` (an integer from 0 to 100), `part_tag` (a part
  of this session, `{mid, part}`, above) and `enum {values}` (one of the
  listed values). Integers render in decimal, a `part_tag` as the number
  the carrying request shows (`§N§`), and an enum value as itself. The
  `part_tag` slot replaced llm-runner 0.3's `tag` slot, which took a
  number: a template declaring `tag` no longer decodes, and a template with
  a `part_tag` slot has a different digest than it had with `tag`.

**Senders.**

- [pinned] The plan's optional `senders: [{module, idle_start?, interrupt?,
  templates?}]` (`delivery_order::PlanSender`) names the modules other than
  the owner whose sends the session admits, and the templates each may use.
  Each entry, its grants included, is frozen at admission with the plan.
  It is malformed (a module twice, a template id twice within a sender, a
  placeholder without a slot or a slot without a placeholder, an integer
  slot with `min` over `max`, an enum with no value, an unknown member or
  slot type, a grant that is not a boolean) or present on a runner without
  the group, and is then refused
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
  string `queue` or `steer`, which stand for lists holding `idle_start`) is
  refused `invalid_params {field: "delivery"}`. A sender granted
  `idle_start` that sends the string `queue` or `steer` is admitted as its
  list would be, never refused for being a string
  (`delivery_order::admit_delivery`).
- [pinned] **The interrupt grant** (added in 0.5.2). A named sender may send
  the `interrupt` delivery string only when its entry grants `interrupt:
  true` (`PlanSender::with_interrupt`); otherwise it is refused
  `invalid_params {field: "delivery"}`, writing nothing, as an ungranted
  `idle_start` is (`delivery_order::admit_delivery`). A granted interrupt
  behaves exactly as the owner's does (§9): written durably first, then the
  running turn cancelled, then its message started. The grant does not
  grant `idle_start`, nor `idle_start` the interrupt. `interrupt` is not a
  list mode and stays a string. Before 0.5.2 `interrupt` was the owner's
  alone, which an entry without the grant still means.
- [pinned] A run a named sender's send starts or joins runs under the
  session owner's authority, never the sender's (§9).
- [informative] **When a send stops making sense.** A sender whose message
  only makes sense inside the run in flight (an app's steer into a worker,
  say) sets `expires_at_ms` on its list. Without it, a list that cannot be
  delivered now waits, queued, and may land in a later run long after it
  stopped making sense. A send still undelivered at that time ends
  `expired` and is never written, as the expiry rule above says. This is
  guidance for senders, not a new runner obligation; how long to wait is
  the sender's choice.

### 9.8 `send_status`: `send.status`

A caller that follows a queued send (§9) re-sends its full body under the
same `send_id`. A runner that declares `send_status` also answers the same
question from the `send_id` alone, so a caller polling many sends does not
re-send their bodies.

- [pinned] The request is `{send_id}` (`send_status::SendStatusRequest`),
  strict: an unknown field is refused `invalid_params` naming it, because a
  misspelled `send_id` would otherwise ask about another send. It names no
  session (§1), and carries no part of the send's body.
- [pinned] The answer is what a re-send of that `send_id`, with the
  body it was accepted with, would answer at that moment, **except that
  `send.status` never acts**: a `SendReply`
  (`send_status::SendStatusReply`), with §9's `state`, `run_id`,
  `submission_id` and `delivered` rules. In particular, a delivered steer's
  `step` receipt has `run_id` exactly when the runner declares `run_ops`, and
  otherwise omits it while still requiring `ref`; `step` is never a
  `SendReply::state`. A queued send's final `step`, `turn` or `unknown`
  answer survives a runner restart unchanged. The runner answers from its
  durable record of the send.
- [pinned] The `send_id` is looked up as a re-send's is: the caller's
  `send_id` namespace (§9, and per sender on a `delivery_order` runner,
  §9.7). A `send_id` the session has no record of for the caller is
  refused `unknown_send`, terminal (§12.1); nothing is written.
- [pinned] `send.status` writes nothing and dispatches nothing: it never
  admits, delivers, re-queues or retries a send, and it is not activity
  for retention (§9.1). Where a re-send would itself act by re-driving a run
  paused for re-authentication, `send.status` reports the current durable fate:
  `state: "paused"` with its `pause_reason`, and the run stays paused.
  Resuming is the caller's choice, by re-sending the accepted body.
- [pinned] A caller sends `send.status` only to a runner that declares
  `send_status` (§3). Against any other runner it falls back to re-sending
  the full body under the same `send_id`. This ordinary `session.send` retry
  may act as §9 describes, including re-driving a paused run.
- [pinned] Vectors: `send-status.json` (requests, refused requests,
  replies, and the `unknown_send` refusal) and `send-status-describe.json`
  in `test-vectors/llm-runner-v1/`.

### 9.9 `turn_directives`: trailing wake tags

- [pinned] A lead agent may end its final reply with a tag asking its caller
  to wake it later. A caller sends `directives: ["wake"]` only to a runner
  declaring `turn_directives` (§3). The array remains at
  `SendRequest::runner_params["directives"]`, on the wire beside `prompt`;
  `SendRequest::directives` reads typed `directives::DirectiveName` values
  without changing the raw bytes, and `with_directives` builds the array
  without replacing sibling parameters. An empty array requests none.
- [pinned] On a declaring runner an unknown name, a non-array (including
  `null`), a non-string element or a duplicate is refused `invalid_params
  {field: "directives"}` before anything is written. Names are frozen at run
  admission like §9.3.1's run limits. An omitted member on a joining steer
  inherits the run's names; an explicit different array is refused naming
  `directives` (`SendRequest::check_directives`), under §9's existing
  first-differing-field steer rule. Without the group, existing lenient
  runner-parameter decoding is unchanged.
- [pinned] Evaluation happens only for `state: "completed"`, only when the
  run was admitted with the name, and only on `final_message.text` exactly
  as §6 defines it: text parts joined in order with the empty separator,
  reasoning excluded. This is the stored model output. It never includes
  text the runner itself added for the model, such as rendered tags or
  reminders. The runner never edits that text to remove or normalize a
  directive. Every other terminal state, every non-terminal state, and a
  completed run with `text: ""` report no directives; absence is not an error.
- [pinned] `RunResult::directives` and `RunStatus::directives` are optional
  arrays of `directives::Directive`, valued `[{name: "wake", in_ms}]`,
  `[{name: "wake", in_ms, note}]`, `[{name: "wake", off: true}]` or
  `[{name: "wake", error: "<reason>"}]`. They are absent, not `[]`, when no
  directive was found or the run did not request the group. `run.status`,
  where served, carries exactly the same value as `run.result`. A runner
  MAY echo that value on its own run-end stream event; only `run.result`
  and `run.status` are pinned surfaces. Consumers decode names and errors
  open (`DirectiveError`), like `reason`; the parser produces only the
  closed `WakeError` reasons below.
- [pinned] **Grammar stability.** A directive name's grammar never changes.
  A new grammar requires a new name, so re-parsing durable text gives the
  same answer forever. `directives::parse_trailing(&str)` returns
  `Option<WakeDirective>` (`In`, `Off` or `Error`), without range policy.
- [pinned] The tag is the last non-whitespace content, outside fenced code.
  Whitespace trimming uses Unicode whitespace (`str::trim`/`trim_end`). A
  fence line's first non-blank content is a run of at least three backticks
  or tildes. It closes only on a line with the same character, at least that
  many characters, and whitespace only after them. An unclosed fence runs
  to the end; a tag inside it does not count. Indentation is whitespace.
- [pinned] The exact forms are `<wake in="90s"/>`,
  `<wake in="3m">note</wake>` and `<wake off/>`. There is exactly one ASCII
  space after `wake`, and optionally one ASCII space immediately before
  `/>` or the opening tag's `>`. No other attribute spacing, tabs, single
  quotes, extra attributes, spaces in `</wake>`, or paired `off` form are
  accepted. Attribute values use double quotes only. Notes are literal
  UTF-8, not XML entity-decoded.
- [pinned] `in` is one to nine ASCII digits followed by `s`, `m` or `h`,
  nonzero, with no sign or decimal point; leading zeros are accepted.
  Milliseconds use checked `u64` arithmetic (even `999999999h` fits).
  The parser reports `in_ms` as written; scheduling range policy, such as
  30 seconds to 24 hours, belongs to the caller, not the parser.
- [pinned] A note is trimmed, then limited to 200 UTF-8 bytes, not
  characters; an empty trimmed note means no `note`. A non-empty note has
  no control characters (`char::is_control`) and no `<`.
- [pinned] A trailing candidate starting with `<wake` that does not match
  is an error, never silently ignored: `bad_duration` for an invalid or
  zero duration; `note_too_long` for over 200 bytes; `bad_note` for a control
  character or `<`; `malformed` for any other syntax. Syntax is checked
  before note contents, note contents before duration. A complete tag
  followed by more non-whitespace text is mid-text and means no directive,
  even when the tag is malformed. Other trailing content means none.
- [pinned] Vectors in `test-vectors/llm-runner-v1/directives/` pin parser
  outcomes and the wire send/result. The parts vector joins two text parts
  with a tool part between them, excluding the tool part, under §6.

### 9.10 `user_commands`: person-only commands

- [pinned] Declaring `user_commands` promises the owner relay and isolation
  from history, not a host menu. Commands frozen from each provider's
  `tool.catalog.user_commands` appear in `session.baseline.user_commands`
  as `{provider, name, description, argument_hint?}`. The member is omitted
  when empty, preserving existing baseline bytes. Commands are never offered
  to a model, as tools or otherwise.
- [pinned] Replies, progress and final results never enter the transcript,
  `session.read`, or any model request. This rule is fixed, not chosen by a
  provider. A runner lists commands in its host's menu only when the host can
  show replies outside the transcript. A host that records command output as
  history gets no menu entries; the owner relay remains available.
- [pinned] The menu shows `/name`, description and hint. A name matching a
  built-in host command or any command already registered when the runner
  lists its commands is qualified as `<module>-<name>`. Baselines and relay
  requests use the frozen name, independent of this host alias.
- [pinned] The optional top-level plan member `interactive` is a boolean;
  absent means a person may drive the session. `interactive: false` lists
  no commands and refuses every invocation with `no_person`. Admission
  refuses duplicate command names from different plan providers with
  `user_command_collision {name, providers}`.
- [pinned] `session.user_command {session_ref, command, input,
  invocation_id}` is owner-only, like `session.baseline`; every other caller
  is refused `scope_owner_mismatch`. It returns `{call_key, reply}`, where
  `reply` is the first stored `UserCommandReply`; after a lost provider reply,
  that stored answer is runner-assigned `outcome_unknown`. Input is forwarded
  verbatim (at most 4,096 bytes). The runner mints the key from the host's id
  for that invocation when available; otherwise its host shim mints an id
  once and reuses it on every retry. For the owner relay the identity is
  `(owner, invocation_id)`. The runner durably records the invocation before
  forwarding and sends `user_command.invoke` once on the session's route to the
  declaring provider. A retry returns the stored answer, never dispatching
  again. Any invocation forwarded to the module whose reply was not stored
  ends in terminal `outcome_unknown`, whatever ended it: a crash, a forward
  timeout, connection loss, session cutoff or expiry. Whether the command ran
  is unknowable. An invocation never forwarded ends `failed`, including a
  cutoff or expiry before forwarding. After forwarding the runner records
  `outcome_unknown` rather than issuing another refusal.
  The runner stores terminal `outcome_unknown` as the answer and status instead
  of forwarding again; every retry and status read replays that outcome. Its
  text tells the person the command may have run.
- [pinned] `session.user_command_status {session_ref, call_key}` is also
  owner-only. It returns `{state, text, progress: [text], job?}`: the latest
  state, the first reply's or terminal result's text, and progress in order
  (the last 20, at most 64 KiB total). State accepts unrecognized strings and
  leaves their values available to callers. `outcome_unknown` is terminal: the
  module may have run the command but its reply was lost. A caller must never
  retry it automatically and must tell the person it may have run. Invocation
  state is retained until the session ends or 24 h after its terminal result,
  whichever comes first. There is no separate notice for the person: notices go
  to the model. The owner retrieves command replies and progress through the
  invocation reply or this status operation.
- [pinned] While accepted, the runner pulls provider `late_results` every
  2–10 s until the result, session end, or one hour. After an hour it tells
  the person no result arrived in that time. The owner polls status at the
  same 2–10 s rhythm while open. Progress is non-terminal; a result carries
  a `done` or `failed` `UserCommandReply`, or terminal `outcome_unknown` after an
  indeterminate forward.
- [pinned] An unfrozen name is refused `unknown_user_command {command}`; a status key
  not held is refused `unknown_invocation`. A runner without the group
  refuses these ops with `user_commands_unsupported`.

### 9.11 `client_message_id`: the caller's id for a sent message

- [pinned] `session.send` may carry `client_message_id` on a runner
  declaring `client_message_id` (added in 0.5.1): 1 to 64 characters of
  `[A-Za-z0-9_-]` (`display::client_message_id_valid`), the caller's own id
  for the user message the send creates. Any other value, or the member on a
  runner without the group, is refused `invalid_params {field:
  "client_message_id"}` (`SendRequest::client_message_id`).
- [pinned] The runner stores it beside the user entry the send creates,
  never inside the stored body, and a runner on a host keeps it in its own
  per-message record, never in the host's stored message, so the model
  never sees it. Prompts typed into the host directly carry none.
- [pinned] It appears on that message's display row (§4.7) and nowhere in
  the default `session.read` entry. No `session.read` member opts into it in
  this version.
- [pinned] It is part of the send's identity: a re-send under the same
  `send_id` must carry the same `client_message_id`, or none if the first
  send carried none; any other is refused `send_id_reuse {field:
  "client_message_id"}` and writes nothing (`SendRequest::same_client_message_id`).

### 9.12 `attachments`: uploads, sends and media reads

Added in 0.5.3. All operations act on the session bound to the management
route (§1); no request carries a second session selector. The runner uses
its host's file storage, not a second copy of the host's media store.

- [pinned] A caller sends upload references only to a runner declaring
  `attachments`. An older runner ignores unknown send members and would
  silently drop the file. The group is all of this section, with no extra
  discovery metadata; `session.head.input` (§4.3) still describes the model's
  input, not whether this group is declared.
- [pinned] `session.upload_begin {mime, bytes, sha256}`
  (`attachments::UploadBeginRequest`) returns `{upload_id}`. It requires an
  existing session: there are no pre-session uploads, and an absent session
  is refused `unknown_session`. `sha256` is lowercase hex over decoded bytes.
  Malformed metadata is `invalid_params` naming `mime` or `sha256`.
- [pinned] Upload operations admit exactly the owner or a plan-named sender
  who may send to the bound session (§9.7); any other caller is refused
  `scope_owner_mismatch`. An upload belongs to the session where it began.
- [pinned] `session.upload_append {upload_id, offset, data}` returns
  `{received}`. `data` is base64 of at most 2,097,152 raw bytes
  (`attachments::CHUNK_MAX`), inclusive; a larger decoded chunk is refused
  `upload_chunk_too_large`. Base64 grows a chunk by 4/3: 3 MiB raw would fill
  the 4 MiB daemon frame. Malformed base64 is `invalid_params {field: "data"}`.
  Appends are contiguous: an offset other than `received` is refused
  `upload_offset_mismatch {received}`. After a lost append reply, a sender
  resumes at that count, without replaying the bytes. An append that would
  exceed begin's declared size is refused at once `upload_size_mismatch`.
  Every refusal leaves the upload unchanged.
- [pinned] `session.upload_status {upload_id}` returns `{state, received}`,
  where the known states are `open`, `committed` and `gone`; an unknown
  state string is preserved, not a decode error (§13). `received` is the
  accepted decoded-byte count for `open`, the full size for `committed`,
  and 0 for `gone`. An uncommitted
  upload may be lost at restart; status then answers `gone`, never `open`
  with 0 bytes. `gone` has `received: 0` and also covers an unknown id.
  The caller uses `state` alone to decide whether to re-upload (`gone`
  means re-upload); for `open`, it resumes at `received`.
- [pinned] `session.upload_commit {upload_id}` verifies total size and
  SHA-256 before returning `{state: "committed", sha256, mime, bytes}`.
  Mismatches are `upload_size_mismatch` or `upload_sha256_mismatch`, and
  leave the id unusable. Only a committed id is valid in a send. Repeating
  commit of a committed id returns the same metadata; appending to it is
  `invalid_params {field: "upload_id"}`.
- [pinned] Upload ids are looked up by (bound session, id). An unknown id
  on append, commit or send is `unknown_upload`; a known lost/deleted id is
  `upload_gone`. An id from another session is indistinguishable from one
  that never existed: status answers `gone`, and append, commit and send
  refuse `unknown_upload`. A runner never reveals that another session
  holds an upload. These refusals write nothing.
- [pinned] A committed upload no send names is kept at least 3,600 seconds
  after commit, then may be deleted. Once a send names it, it lives as long
  as the session history referencing it, across runner restarts included.
- [pinned] `session.send` adds optional `content: [{type: "upload",
  upload_id}, ...]` (`attachments::ContentPart`), in order, after the
  existing prompt text. Absent `content` is omitted and prompt-only sends
  keep their bytes. The prompt and template rules of §9 are unchanged.
  Each id must be committed in this session: an open id is refused
  `upload_not_committed`. Admission checks all references before writing
  anything, including the send's idempotency record. Media types are checked
  against the model frozen by the run it joins, or the model the send would
  freeze when starting a run, using the same facts as `session.head.input`.
  A known unsupported modality or MIME type is refused `media_unsupported`
  and writes nothing; an unknown fact is not a known lack of support.
- [pinned] Default inline `session.read` never exposes new block types.
  Every media block becomes a text block whose text is exactly
  `[media not shown: kind=… mime=… bytes=… sha256=<first 12 hex> reason=not_requested]`.
  Each media part of a multi-part tool result becomes the same marker, and
  the result is the existing text form built from its parts in order (joined
  by a single `\n`, without a trailing newline), not a new multi-part shape. Strict readers, including
  compaction readers, therefore see only their existing shapes. The rule
  also applies to inline history and resolve reads where those are served.
- [pinned] `media: "blocks"` returns each message the way the runner stores
  it, media included: a stored block, or whatever bytes the runner's own
  schema keeps inline for that media (a `data:` URL, say). This group adds no
  member of its own that carries bytes, and a stored message need not hold
  the media's sha256 anywhere. Older runners refuse the unknown `MediaMode`
  value as `invalid_params {field: "media"}`, so readers fail closed.
  Descriptors mode keeps `media_ref`, and display rows still use refs (§4.7).
  `digest` and `part_digests` always identify the stored message and its
  stored parts, never the marker render or descriptors. The frame budget
  still applies (§4.2): a stored message too large for one reply, media
  bytes included, is refused `reply_too_large` in blocks mode exactly as in
  inline mode, and descriptors mode is how a reader pages such a message.
- [pinned] Model fallback and reads share `attachments::marker` and the
  grammar above. The context selects `reason`: `not_requested` for default
  reads, `unsupported` when a model cannot take stored media. Size and hash
  are over decoded bytes; `kind` and `mime` are the media's metadata.
- [pinned] `session.media_read {sha256, offset, length}` returns base64
  `{data, size, mime}` (`attachments::MediaReadReply`); `size` is the full
  decoded size. `length` is at most 2,097,152 decoded bytes; a larger value
  is `invalid_params {field: "length"}`. A zero length is a metadata probe:
  `data` is omitted. At EOF (`offset == size`), positive length returns
  empty `data`; past EOF is `invalid_params {field: "offset"}`. Reads may be
  short at EOF. Admission is exactly that of a read of this session.
- [pinned] A digest is served only when this session's history references
  it, including an image in a tool result. Every other digest, even one
  held for an unreferenced committed upload or another session, is refused
  `unknown_media`. Knowing a hash never grants another session's bytes.
- [informative] Thumbnails and full-size renders are the reader's job
  (Thalamus renders on demand); runners never store those derived renders.
  Images OpenCode already holds as data URLs in its file parts are served
  by digest like any other media.

### 9.13 `output_refs`: immutable references to tool outputs

- [pinned] The `output_refs` group requires `tags` and applies only to a
  session whose frozen plan renders tags. It serves `session.output_refs`
  and `session.output_ref_read`. A plan item may carry `output_refs: bool`,
  absent meaning false. A runner without the group refuses any plan carrying
  the member, even false, as `invalid_params {field:
  "plan.items[].output_refs"}`, before writing anything.
  This refusal binds non-advertisers on every plan-carrying operation,
  including `session.send`, `session.refresh` and `session.adopt`, whether the
  supplied value is true or false. One call to `output_refs::check_plan` or
  the request's `check_output_refs` before any durable write supplies the
  check. The kit's unsupported-plan case runs on non-advertisers and is N/A
  for advertisers; positive reference cases have the opposite applicability.
- [pinned] Scan only the model's own assistant text parts with
  `cortexkit-rich-ui-syntax` 0.1.1 `scan_references(source, reply)`. The scanner's
  `rich-ui/1` dialect, including its code, escaping, display-tag and string
  rules, is authoritative; runners never implement a second grammar.
  Record each occurrence in text-part order and scanner encounter order.
- [pinned] A plan tool's string arguments are scanned only when its provider's
  plan item has `output_refs: true`, and only in catalog `reference_slots`, in
  declaration order. A `token` slot must be exactly one token, with no trimming;
  otherwise it records `unresolved` / `not_a_reference`. A `source:<dialect>`
  slot is scanned in `program` mode, never as data or reply text. Missing and
  non-string slots are not scanned. Non-slot arguments, and tools declared on
  a send (which have no plan item or catalog), get no call set.
- [pinned] Resolve against the request numbering the writing step saw, as of
  the durable commit storing the message or call. Persist records in that
  commit or derive them identically from durably stored inputs at that commit.
  Never resolve them against later numbering. Tool-result text and nested
  images resolve to the stored output part, not rendered bytes: no tag prefix
  or text injected during rendering. User and model text record `unresolved`
  / `not_a_tool_output`; an absent number records `unresolved` / `unknown_tag`.
  Running calls have no number. A held result records `held`, its `call_key`
  and the placeholder the model saw; it never fills in with a late result.
- [pinned] A record is `{ref_id, tag, part, call_key, tool, provider, format,
  mime?, bytes, sha256, state, reason?}`. `ref_id` is opaque and unique within
  the session. `tag` is the scanner's canonical decimal string (no machine
  integer bound); it is null for `not_a_reference`. Unknown target metadata is
  null. `part` is the target part key; `call_key` is the referenced call's key;
  `format` is `text` or `image`; `bytes` and lowercase `sha256` describe the
  stored output. States and reasons are open strings on decode.
- [pinned] At most 64 occurrences per message or call can resolve. Later
  occurrences record `unresolved` / `too_many_refs`. A snapshot over 1,048,576
  bytes, or one that would exceed 4,194,304 resolved snapshot bytes in that
  message or call, records `too_large`, full size and SHA-256, without content.
  Repeated occurrences count toward both budgets. Content is retained once
  per session by SHA-256; an existing stored part or media store may serve it
  if it lives as long as every record naming it.
- [pinned] One message's or call's records form an immutable set with descriptor
  `{set_id, mid, call_key?, dialect, count, digest}`, at most 4,096 encoded
  bytes. `call_key` on the descriptor identifies the containing call, not a
  referenced one. `digest` is lowercase SHA-256 of the RFC 8785 (JCS) encoding
  of the records array in encounter order, retaining null fields and omitting
  absent `mime` and `reason`. `test-vectors/llm-runner-v1/output-refs.json`
  pins the exact canonical bytes and digest. Records survive compaction and
  lineage changes; rewind removes records with their message, and session
  retention removes them with the session.
- [pinned] `session.output_refs {set_id, after?}` returns `{records, next?}`,
  at most 32 records and 16,384 bytes for the complete encoded page. `after`
  and `next` are opaque record-id cursors within that set. An unknown cursor
  is `unknown_ref`. `session.output_ref_read {ref_id, offset, length}` returns
  `{data, size, format, mime?}` with base64 data, at most 131,072 decoded bytes.
  Length zero returns metadata only. Offset equal to size returns empty data;
  offset beyond size or length beyond the cap is `invalid_params` naming
  `offset` or `length`. A record without retained content answers `unknown_ref`.
- [pinned] Both operations admit whoever may read the session, and a carrier
  listed on the session's scope with this runner among its targets. IDs are
  session-bound: another session's `set_id` or `ref_id` answers `unknown_ref`,
  exactly like an ID that never existed. This check precedes content lookup.
- [pinned] A message's display row carries its descriptor as `output_refs`;
  a tool call's display entry carries its call set's descriptor. Default
  `session.read` and stored bodies never gain descriptors. A set may be empty
  when scanning found no references; no snapshot is copied in that case.

### 9.14 `session_retention`: per-session storage policy and release

- [pinned] This group is separate from §9.1 `retention`, which is unchanged.
  `role.describe` declares `session_retention` and limits
  `session_retention: {max_days: u32, max_unreleased_hold_ms: u64, required: bool,
  min_hold_days?: u32, deletion_waits_for_readers?: true}`
  (`describe::SessionRetention`). `check_describe` refuses a declared group
  without limits (`missing_session_retention`), or with `max_days` outside
  1..=3650, a zero `max_unreleased_hold_ms`, or `min_hold_days` greater than
  `max_days` (`invalid_session_retention`). `min_hold_days` defaults to 0 and
  is omitted at 0; `deletion_waits_for_readers` is omitted unless true.
  The group serves `session.send`, `session.head` and `session.release`;
  it also extends `session.adopt` wherever `adopt` is declared.
- [pinned] `session.send` and `session.adopt` accept optional
  `retention_policy: {keep, archive_after_days, backup, request_dumps}`
  (`session_retention::RetentionPolicy`). All four members are required inside
  the object; unknown members inside are refused. `keep` is `"none"`,
  `{"days": N}` or `"forever"`. Days are integers in 1..=the runner's
  `max_days`. `archive_after_days` is an integer: 0 means never, otherwise
  1..=3650. With days it must be less than N; with `"none"` it must be 0.
  `backup` and `request_dumps` are booleans. Every runner refuses
  `backup: true` with `keep: "none"`. Any bad value or combination is
  `invalid_params` with `detail.field` naming the member path, for example
  `retention_policy.keep.days`, `retention_policy.archive_after_days` or
  `retention_policy.backup`. A runner without the group refuses the member
  as `invalid_params {field: "retention_policy"}`, never ignores it.
- [pinned] A runner declaring `session_retention` honours and enforces every
  policy it accepts; it never accepts one and ignores it. While `required`
  is false, a first send without a policy is still accepted. With `required`
  true, a first send or first adopt without one is refused `retention_required`,
  and nothing is written. A starter sends `retention_policy` to every runner
  declaring `session_retention`, whatever `required` says, and to no other
  runner. A runner may declare both groups during the switch; a send carrying
  both `retention` and `retention_policy` is refused
  `invalid_params {field: "retention_policy"}`. A runner may stop declaring
  §9.1 `retention` in the same release in which it declares `session_retention`.
  A starter picks which member to send by which group the runner declares;
  when both are declared it sends only `retention_policy`.
- [pinned] Days and archive clocks count from the session's last durable
  activity: sends, adopt and run lifecycle, never cache warms or maintenance.
  An archived session's reads return exactly what they returned before archive;
  only storage changes, unless `archive_prunes_reasoning` is declared and
  §9.14.1's exception applies. Nothing is deleted while a run is active or a send is
  queued, whether deletion is due to days expiry, release or the fallback hold.
- [pinned] No session's content is deleted less than `min_hold_days` after the
  later of its last durable activity and its release. The floor is a minimum,
  never an addition: without release, `keep: {days: N}` expires at max(N, floor)
  after last activity. Release instead uses the floor deadline below.
  `keep: "none"` expires at release plus the floor, or, when never
  released, at max(`max_unreleased_hold_ms`, floor in milliseconds) after its
  last run ends.
- [pinned] The first send or adopt carrying a policy freezes it. Later sends
  or adopts may repeat it. Equality is on decoded values, not bytes. A different
  policy is refused `retention_conflict` with `detail.frozen` equal to the frozen
  policy. Refusals write nothing. Adopt retains §9.5's caller, idempotency and
  plan checks, then checks the policy before writing. Its idempotency comparison
  includes the policy, naming `retention_policy` on `send_id_reuse` when it differs.
- [pinned] Sessions not started by a send (person-started sessions a runner hosts,
  or sessions predating the group) get the runner's configured policy, with
  `source: "runner"`. The first send or adopt carrying a policy replaces a
  runner-sourced policy exactly once. After that it is starter-sourced and frozen.
- [pinned] `session.head` gains `retention_policy`, exactly as accepted or
  runner-assigned, and `retention_state: {source: "starter" | "runner",
  request_dumps_unsupported?: true, host_limited?: true}`. When dumps were
  requested but the runner cannot save them, it sets `request_dumps_unsupported`:
  dumps are best-effort, never a refusal, and the policy keeps `request_dumps: true`.
  `host_limited` means the host keeps the transcript and may delete it sooner
  than the policy says. Flags appear only as true, never false.
- [pinned] `session.release {}` (`ReleaseRequest`) targets the bound session
  name, for every keep value: the policy is the longest a session is served;
  a declared `min_hold_days` and a wait on a declared reader are the only extra
  storage time allowed. Release is the owner ending the session sooner.
  Owner-only means the principal that
  made the session's first send (or adopt), as the daemon attested it, never
  the owner of a parent scope. Any other principal is refused
  `scope_owner_mismatch`, including a caller under that parent scope. The
  starter's principal can release even under another principal's parent scope.
- [pinned] Release is idempotent. The answer (`ReleaseReply`) is
  `{state: "pending"}` until deletion completes,
  `{state: "deleted"}` when this call completed deletion before answering, and `{state: "absent"}`
  when the bound name has no session (never created or already deleted).
  The runner records release durably before answering pending. It then deletes
  once the active run ends, the queue drains, the floor ends and any declared
  reader permits deletion, without another starter call,
  including after a process kill and restart. Pending means release was recorded
  durably; deleted means this call completed deletion with nothing running;
  absent means no session exists under the bound name right now. A repeat after
  deletion always answers absent; no completed-deletion record is required.
  Deletion after pending must not wait for another call, including after restart:
  once the recovered run ends, the queue drains, the floor ends and any declared
  reader permits deletion, deletion completes autonomously.
  A release after that completion answers absent.
- [pinned] From the moment release is recorded until deletion completes, for
  every runner, including floor 0, `session.head` carries
  `released: {at_ms: u64, delete_after_ms: u64}`. `at_ms` is the recorded release
  time; `delete_after_ms` is the earliest deletion time, the end of the floor
  measured from the later of release and last durable activity. Deletion may
  come later only while waiting on a declared reader (and never while a run is
  active or a send queued). Reads and media are served as before, subject only
  to declared archive pruning (§9.14.1). `released`
  marks the session as ended; it does not change `last_run_state`, which keeps
  reporting the run as it is until the run ends. A send to the name is refused `session_released`
  (retryable) with `detail.delete_after_ms`. Release answers `pending`, including
  on repeated calls while deletion is incomplete. After deletion, head and reads refuse
  `unknown_session`, and `session.media_read` refuses `unknown_media` for its
  checksum. The next send starts a new incarnation under the same name.
  Release frees live runner data only: backups already taken follow the backup
  service's own retention.
- [pinned] For `keep: "none"`, if no release arrives, the runner deletes the
  session max(`max_unreleased_hold_ms`, floor in milliseconds) after its last
  run ends, subject to any declared reader wait.
- [pinned] A runner may delay a due deletion until a declared log reader has
  read the content; it declares `deletion_waits_for_readers: true`. A due log
  is deleted only when its last recorded write time is strictly before that
  reader's watermark (§9.15). With `{watermark_ms: null}` or `not_a_read_source`,
  nothing waiting on that reader is deleted. The runner's health degrades when
  any due deletion has waited more than 24 hours past its floor. The writer
  stamps every log record with its wall-clock time, taken immediately before
  an append that is durable and visible to the reader within ε = 60 seconds.
  Writer and reader run on the same computer and use the same wall clock;
  a runner never relies on a reader on another computer.
- [pinned] Deletion removes everything the session owns: log, indexes, lease,
  media and attachments the runner holds by checksum, outbox entries and request
  dumps. A session's dumps never outlive it. Files placed in the user's project
  folder (for example uploads written there) are the user's project files and
  are never deleted with the session.
- [pinned] A runner that cannot back up a single session refuses `backup: true`
  with `backup_unsupported`, whose detail names the reason (`reason`). It never
  accepts and silently skips backup. A session with `backup: false` is absent
  after a restore, never half there. A restore never brings back a deleted
  incarnation, and never deletes a later incarnation under the same name.

### 9.14.1 `archive_prunes_reasoning`: in-place pruning at rest

- [pinned] A runner declares the `archive_prunes_reasoning` capability group
  and the required describe object `archive_prunes_reasoning: {kinds: [...]}`
  (`archive_prunes_reasoning::ArchivePrunesReasoning`). `kinds` is nonempty,
  has no duplicates and contains only `reasoning`, `opaque_reasoning` and
  `signature`. `check_describe` refuses either the group or object without
  the other, or invalid kinds. Without the group, §9.14 and §4.2 hold exactly
  as before: archive never changes served content. A runner applies only the
  kinds it lists.
- [pinned] Pruning is allowed only in an archived session at rest: no active
  run and no queued send, the same exclusion as retention deletion. It is
  an in-place payload change, not a cut, revert or new lineage.
- [pinned] The kinds (`archive_prunes_reasoning::kinds`) mean:
  - `reasoning`: empty a reasoning part's payload, keeping `text: ""` when
    its format requires text, and removing optional signatures and encrypted
    data. A `redacted_reasoning {data, provider_format}` part keeps required
    `data: ""` and its `provider_format`; `reasoning {text, signature?}`
    keeps `text: ""` and drops the optional signature.
  - `opaque_reasoning`: remove only encrypted/opaque fields; a readable
    summary in the same part stays byte-identical. A `redacted_reasoning`
    part keeps required `data`, rewritten as the same item JSON with only
    encrypted fields removed, preserving its summary and all other fields.
  - `signature`: remove a provider signature on a non-reasoning part, for
    example assistant text or a tool call. Text and call arguments stay
    byte-identical. A call drops its optional `thought_signature`; a
    `text_with_state` part keeps required `provider_state`, with only the
    signature string inside it emptied to `""`.
- [pinned] Every field required by the stored format stays present. Required
  string payloads are emptied to `""`, never removed. No member is added to
  a block: only the page says what was pruned. A reader encountering a kind
  it does not know treats that part as changed in an unknown way: kept in
  place, but its payload is not trusted.
- [pinned] A pruned part stays at the same index with the same type and key.
  Thus `m<ordinal>#<index>`, tags, resolve's `part`, `part_digests` positions
  and reminder slots never move. Lineage, ordinals, tags, `history_version`
  and history cursors are unchanged. Removing a signature never changes whether a part counts;
  a `signature`-kind text part keeps its number and its exact text.
- [pinned] Message `digest` and every `part_digests` entry retain their
  write-time values, computed over original content, not pruned content.
- [pinned] Every page serving a pruned part carries
  `pruned_reasoning: [{mid, part, kind, bytes, sha256}]`
  (`archive_prunes_reasoning::PrunedReasoning`), for exactly the pruned parts
  it serves. This applies to raw and model `session.read` (`ReadPage`,
  `ModelPage`) and raw and display `session.history` (`HistoryPage`,
  `DisplayPage`), on every read, not only the first after pruning. A model
  page does not list a stored part hidden by compaction. The member is absent
  when no served part was pruned. `bytes` is the length of the original
  stored part's RFC 8785 (JCS) bytes, the bytes `content_digest` hashes;
  `sha256` equals its write-time `part_digests` entry's `digest`.
- [pinned] **Re-read rule for readers.** A message re-read after pruning
  differs from its prior served copy only in the parts its page lists, and
  each listed entry's `sha256` equals that part's write-time digest. A reader
  may rely on this to accept the change and replace its held copy. Part
  identity and digests do not move when it does so.
- [pinned] A runner never prunes a session enrolled with a compaction provider
  whose `compaction.setup` ready answer did not list `pruned_reasoning` in
  `accepts`. Omission or an empty list grants no permission. Sessions with no
  compaction provider are unaffected by this enrollment gate.
- [informative] Reopening continues without the pruned reasoning. Before
  pruning a model family, the runner is responsible for ensuring that its
  model provider accepts the resulting history.

### 9.15 `log-read-watermark/v1`: durable log-reader progress

- [pinned] A log reader declares `log-read-watermark/v1` in its manifest's
  `provides`. Its one operation is `watermark.get {}`. The request is strict:
  any member is `invalid_params`. The reply is `{watermark_ms: u64 | null}`:
  every record the calling module wrote before `watermark_ms` has been durably
  read and recorded. The crate exposes the names and wire types in `watermark`.
- [pinned] The reader answers only for the daemon-attested caller (the bind
  principal), never for another module's logs. A caller it does not read,
  excluded callers included, is refused `not_a_read_source`. `null` means the
  watermark is not established, for example before the first complete pass.
- [pinned] T is the start of a complete pass over every log of that caller,
  minus ε = 60 seconds, set only after that pass's recording commits.
  An attempt is a model call whose start the writer has logged and whose final
  record the reader has not yet read. While an open attempt comes from the
  writer's current boot, T stays at or before that attempt's start, because its
  final record is still to come. An open attempt from an earlier writer boot
  can never finish: the reader records it as possibly billed, and from then it
  counts as read. T never passes an earlier-boot attempt the reader has not yet
  recorded that way. An unparseable record counts as read once it is set aside
  as evidence. T is persisted and never goes backwards,
  including across restarts.

### 9.16 `call_identity`: a tool call's key and dispatched definition on display entries

An app that renders a tool call's own page from its display entry needs two
facts the entry did not carry before 0.7.2: the call's key, by which it names
the call to the module that renders the page, and the identity of the exact
tool definition the call ran against, by which that module keys its views.
A runner declaring `call_identity` (added in 0.7.2) puts both on every
display tool-call entry it serves.

- [pinned] The group adds no op. It extends the display tool-call entries
  of `history_display` (§4.7) and the live `tool_started` and
  `tool_finished` events of `live_events` (§7.1), so a runner declares it
  only beside at least one of those groups: `check_describe` refuses it
  by name when neither is declared (`call_identity_without_display`). It adds no
  discovery metadata.
- [pinned] The entries are every display row's `tool_call` part
  (`display::ToolCallPart`) on `session.history` with `form: "display"`,
  and every `tool_started` and `tool_finished` event (`live::ToolStarted`,
  `live::ToolFinished`), replays included. `session.read` serves no display
  form, so it gains nothing; its attribution already carries `call_key`
  (§5).
- [pinned] **`call_key`** is the call's key (§11.3), the same value
  `session.read`'s attribution (§5) and `run.tool_calls` (§6.3) expose for
  that call, so a call has one key on every surface. It is present on an
  entry exactly when §5 exposes it: from the moment the call is prepared or
  dispatched, and absent before. A call prepared and then denied keeps its
  key; a call denied before any record held it alone has none.
- [pinned] **`schema_pin`** is the `tool-provider/v1` `schema_pin` of the
  tool definition the call was dispatched against (`tp1:<pct(tool)>:
  <schema_digest>:<semantics>`, the same string the call carried to its
  provider). The runner records it with the call's dispatch and serves that
  record, frozen: it never recomputes the pin from a catalog read later, so
  a provider deploy that changes the tool's definition after the call
  leaves the call's pin as it was. A description edit never changes a pin
  (tool-provider §3 `schema_digest`, §4 `schema_pin`). It is present on every entry of a call dispatched to
  a tool provider, and absent for a call never dispatched (denied, refused
  before dispatch, or sealed unsent) and for a runner-native tool, which has
  no provider definition.
- [pinned] Absent means unknown, never empty: neither member is ever an
  empty string. Both are opaque strings to readers: a reader compares them
  whole and never parses them, the pin included.
- [pinned] Neither member enters a stored message body or the default
  `session.read` entry; the runner keeps them beside the stored call, as it
  keeps `client_message_id` (§4.7).
- [pinned] A runner without the group leaves both members out of every
  entry, and a reader treats their absence there as saying nothing. A
  reader relies on the members only from a runner declaring the group.
- [pinned] `call-identity.json` pins a call carrying both members (dispatched
  to a tool provider), `call_key` only (a runner-native tool, and a call
  prepared then denied) and neither (a call denied before it was keyed), on
  display rows and on both live events, and the discovery answers.


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
  provider with more than one plan item. The `item` value identifies which of
  that provider's plan items the baseline records (§10.1).

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
- [pinned] A runner declaring `system_text_default` checks at plan admission
  that each provider of a preset-less `system_text` item declares
  `system-text-default/v1` in the daemon catalog. If it does not, the runner
  refuses the plan by name as `system_text_preset_required` with
  `detail: {provider}` naming the provider module id, writing nothing and
  forwarding nothing. The refusal is terminal and never timed. With the
  declaration present, the runner admits the item and forwards it unchanged:
  it never invents a preset.
- [pinned] A plan carrying a `compaction_item`, sent to a runner that does
  not declare `compaction`, is refused at admission `invalid_params {field:
  "plan.compaction_item"}` and writes nothing
  (`SendRequest::check_compaction_item`).
- [pinned] The `compaction_item` names the session's compaction provider as
  `provider`, a module id, inside `compaction_item: {provider, preset?,
  params}`, frozen with the plan (`plan_compaction_provider`). It is the
  provider the runner calls Setup on and sends notices and rides to (§11.1).
- [pinned] A plan item names a preset only for a provider whose preset-less
  catalog answer lists it. When `compaction_item` carries no `preset`, the
  runner's calls to that compaction provider carry none either: it never
  invents one and never sends an empty string.
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

The group stays opt-in: a runner declares it only when it serves all three
ops below. Their types are in `session_change`; `session-change.json` pins
every request, answer and refusal.

- [pinned] The `session_change` group holds the owner's mid-session change
  ops: `session.refresh`, `session.refresh_policy` and
  `session.flush_prefix`. They are role ops, so one owner drives every
  runner through the same interface. Like `session.baseline`, they are
  answered only to the session's owner; anyone else is refused
  `scope_owner_mismatch`.
- [pinned] A runner that does not declare the group gets no mid-session
  changes: its owner never sends one undeclared, and applies the change at
  the next session instead.
- [pinned] A runner that declares the group states `rungs` in every ready
  baseline, so the owner can choose a policy the session supports.

**The next-run rule.**

- [pinned] A change never applies inside a run that is already running.
  The earliest it applies is the start of the next run, and the
  `immediately` rung means "from the next run". No step ever sees text or
  tools its run did not start with.
- [pinned] A change applies at the start of the first run at or after its
  rung's point: `immediately` at the next run; `on_prefix_rebuild`,
  `on_system_rebuild` and `on_history_rebuild` at the first run that starts
  with the prefix, the system text or the history rebuilt (a compaction
  fold, a model switch, a `session.flush_prefix`). Applying it moves
  `applied_generation` to its generation and sets `effective`, and clears
  `pending` (§10).

**`session.refresh {plan, policy, generation}`** (`RefreshRequest`, strict).

- [pinned] `plan` is the full fetch-plan object, as `SendRequest::plan`
  carries it (§10.1). `policy` is one rung name from `rungs::ALL`; any other
  value is refused `invalid_params {field: "policy"}`. `generation` is a
  u64 the owner chooses, strictly increasing per session.
- [pinned] The runner fetches the new plan's items under one deadline,
  exactly as admission does (§10.1), and holds them as the pending change,
  replacing any change pending before. It applies the change at the rung's
  point, under the next-run rule.
- [pinned] The answer is `{accepted_generation, pending: {generation,
  policy}, run_in_progress, applies_from_next_run}` (`RefreshReply`):
  - `applies_from_next_run` is true when the next run will use the new
    plan: the `immediately` rung, or a rung whose point the session already
    meets (a `session.flush_prefix` is waiting, say). It is false when the
    change waits for a later rebuild; the current plan then stays live until
    `session.baseline` shows `applied_generation` at or above the
    generation;
  - `run_in_progress` says whether that next run is queued behind one
    running now.
- [pinned] An identical repeat (the same generation, plan identity and
  policy as an accepted refresh) gets the same answer as the refresh it
  repeats, and fetches and writes nothing (`session_change::refresh_order`).
- [pinned] Refusals, none of which writes anything:
  - admission's `fetch_unavailable {provider}` (retryable), `plan_stale
    {differences}` and `tool_name_collision {name, providers}` (§10.1);
  - `rung_unsupported {surface, rung}` (`RungUnsupportedDetail`), when the
    session does not support the rung for a surface the change touches,
    `tools` or `system_text` (`session_change::surfaces`);
  - `generation_stale {accepted}` (`GenerationStaleDetail`), for a
    generation at or below the accepted one that is not an identical
    repeat: the same generation with another plan or policy, or a lower one;
  - `tool_outside_host_set {names}` (`ToolOutsideHostSetDetail`), terminal.
    A host that fixes its tool set when it starts (OpenCode registers each
    tool's name, description and input schema then) refuses a plan naming
    any tool outside that set. A tool is inside the set only if its
    definition is identical: name, description and input schema
    (`session_change::outside_host_set`). A changed definition under a kept
    name is refused the same way, naming those tools. `names` is sorted.
    System text, and which of the set's tools the plan offers, may change.

**`session.refresh_policy {generation, policy, send_id}`**
(`RefreshPolicyRequest`, strict).

- [pinned] Changes the pending change's rung. The answer is `{pending:
  {generation, policy}}` (`RefreshPolicyReply`).
- [pinned] Idempotent on `send_id`: a repeat with the same body gets the
  same answer; the same `send_id` with another `generation` or `policy` is
  refused `send_id_reuse {field}` (`RefreshPolicyRequest::reused_field`).
- [pinned] Refusals, none retryable and each writing nothing:
  `pending_superseded {generation, pending}` (`PendingSupersededDetail`: a
  later refresh superseded the generation named), `already_applied
  {generation, applied_by}` (`AlreadyAppliedDetail`: a rebuild or an append
  applied it) and `rung_unsupported {surface, rung}`.

**`session.flush_prefix {send_id}`** (`FlushPrefixRequest`, strict).

- [pinned] Applies any pending change, whatever its policy, and drops the
  frozen prefix, at the start of the next run, never mid-run. The answer is
  `{pending_generation?}` (`FlushPrefixReply`): the pending change the flush
  will apply, absent when none is pending. Idempotent on `send_id`: a repeat
  gets the same answer.

**Drift and lost replies.**

- [pinned] While a change is pending, an owner send whose plan is either the
  effective plan or the latest accepted one is not drift. Any other plan is
  `plan_drift {frozen, sent}`, with `frozen` naming the latest accepted plan
  (`session_change::check_send_drift`).
- [pinned] After a lost reply, the owner reads `session.baseline`: its
  `accepted_generation`, `applied_generation`, `pending` and `effective`
  (§10) carry everything a `session_change` answer says.

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
- [pinned] It implements the runner half of compaction-provider 0.3.0 §3,
  which that contract defines (its §4 to §10) and this one does not restate:
  - Setup (`compaction.setup`), called once per lineage, before the
    lineage's first model call, for every new lineage, one a cut starts
    included, so each Setup answer, and each tag checkpoint it carries,
    belongs to exactly one lineage (§4.5, §9.6). A runner may run a cut's
    Setup as soon as it prepares the view that carries the cut, off the
    step, so the first step after a cut never waits on the provider; no
    model call is made without a `ready` answer, and a refusal means no
    model call. A lineage a cut, a revert or a recovery started calls Setup
    again before its first model call. The Setup of a lineage a cut
    started, or a revert restarted at a cut the runner had a lineage at,
    carries `descends_from: {lineage_id, through_ordinal}` naming the old
    lineage at that cut, whatever the provider's `setup_descends_from`
    says, because the provider published that cut;
    the provider keeps its state across the call (compaction-provider
    §4). The initial CompactionMessage is recorded before that model call,
    and Setup does not run again within the lineage while the runner holds
    that record. A runner that no longer holds it, as when a detected
    restore starts a new lineage whose recorded Setup answer did not
    survive or that the runner discards with the lost lineage (§9.6),
    calls Setup again before its next model call, as after a failed Setup
    (compaction-provider §4);
  - advance notices on ck-bus after every committed transcript change the
    provider should see, from which the provider reads with `session.read`
    (§4) and `session.resolve` (§9.6), within the current lineage, never
    with `session.history`. The first notice of a lineage a cut started
    carries `descends_from` naming the previous lineage at the cut
    (compaction-provider §5.1);
  - views: fetched with `view.get` on `view_ready`, checked, written durably
    to the runner's write-ahead log and applied only at a step boundary,
    then rendered for every request until a newer view applies, including
    across restarts and while the provider is down. A view's ranges tile
    from the lineage's `first_ordinal`; the runner refuses a view whose
    ranges start anywhere else (`view_refused:<version>:ranges_start`,
    compaction-provider §7). A view with a cut starts a new lineage once it
    applies (§4.5);
  - rides (`compaction.ride`) when the runner's prefix is being rewritten
    anyway, and at the ceiling;
  - the ceiling Setup sets, with the `context_ceiling` refusal and run state
    and the `context_ceiling_wait` status (§6) wherever those surfaces are
    served. A runner that serves none of them, such as one without
    `run_ops` that fronts a host's own requests, reports the ceiling to its
    host instead: the refused host request carries the window, the size and
    the cause;
  - native markers, where the runner's host has them, kept outside the
    transcript (§4.2).
- [pinned] It requires `tags` (§9.6): views address parts by their key and
  tag.
- [pinned] A Setup `REFUSE` ends the run `error`. The run error's
  `provider_code` is the answer's role `code` (`RefuseCode` in the
  compaction-provider contract), with `provider` and `reason`; the answer's
  optional finer code, which the provider's wire names `provider_code`,
  becomes the run error's `provider_detail_code` (§6). Nothing is added to
  history, no model call is made, and the session remains usable: the next
  send calls Setup again.
- [pinned] `compaction_unavailable` is Setup-only: Setup failed or timed
  out with no answer. No model call is made, the run error carries no
  `provider_detail_code`, and the session remains usable: the next send
  calls Setup again, because no initial CompactionMessage was recorded.
- [pinned] The runner serves no compaction op: every call goes from the
  runner to the provider, and the provider learns what changed from
  notices. The shapes are the `compaction-provider/v1` role's, not this
  crate's.

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
detail?}`. Every other refusal field, `class` included, lives inside
`detail`, never at the top level of the body (§1). Codes are open strings: a consumer that meets one it does not know
treats it as a terminal refusal of that one request.

### 12.1 Refusals (`errors`)

| Code | When | Detail | Retry |
|---|---|---|---|
| `invalid_params` | an unknown or duplicate directive name, a non-array `directives` value, or a non-string array element on a `turn_directives` runner (field `directives`, §9.9); a request field is malformed, unknown, or combined with one it excludes; a plan's `compaction_item` sent to a runner without `compaction`; a plan's `render_tags` that is not a boolean, or `true` without `tags`; a plan's malformed `senders`, or `senders` without `delivery_order`; a list in `delivery` on a runner without `delivery_order` (field `delivery`), a malformed list, a list member (`dedupe_key`, `expires_at_ms`, `template`) malformed or sent without a list, or an `idle_start` its sender's grant excludes (§9.7); a malformed tag or an out-of-range `range` on `session.resolve`; an `after` cursor never given, or `limit: 0`, on `run.tool_calls`; a `session.history` request naming more than one of `cursor`, `from_mid` and `before_mid`, naming `view: "model"`, or carrying a cursor that does not parse (§4.6) | `field` | no |
| `lineage_changed` | a read names a lineage that is not the session's | — | no: re-read from the tail |
| `unknown_mid` | `after_mid` names no message in the lineage; a `session.history` `from_mid` or `before_mid` names no message of the session's history (§4.6) | — | no |
| `before_lineage_start` | a range read, an after-message read or a `session.resolve` names a position below the current lineage's `first_ordinal` (§4.5) | `{first_ordinal}` (`errors::BeforeLineageStartDetail`) | no: read from `first_ordinal`, or read older history with `session.history` |
| `unknown_ordinal` | a `session.history` `from_ordinal` or `before_ordinal` names an ordinal the runner never assigned, or one a revert removed (§4.6) | — | no |
| `cursor_expired` | a `session.history` cursor whose entry, or an entry older than it, a revert or rewrite removed or changed, or one minted for another session (§4.6) | — | no: page again from the newest page or a held `mid` |
| `reply_too_large` | the complete encoded `session.read` reply, a `run.result` or `run.status` answer carrying a structured-output `value` (§9.4), or a `run.tool_calls` page holding one call (§6.3), exceeds the 4,194,304-byte transport frame limit (`TRANSPORT_FRAME_LIMIT`), including its 21-byte header | `{bytes, limit, subject}`; `bytes` is the whole encoded success frame length, and `limit` is `4194304`; `subject` is `{kind: "message", session, ordinal}`, `{kind: "model_group", source}` (the model-view anchored entry's existing `EntrySource`), `{kind: "run", run_id}` or `{kind: "tool_call", run_id, tool_call_id, call_key?}`; no message content, value or arguments | no: terminal for this request; lowering `max_bytes` or `limit` cannot shrink a message, atomic group or call |
| `unknown_run` | `run.result` or `run.tool_calls` names no run of the session | — | no |
| `unknown_tag` | `session.resolve` names a well-formed tag that no part of the session's current lineage carries: one no part shows in this lineage, including one that named a part only under a lineage it has left (§9.6) | — | no |
| `fork_unsupported` | a `session.send` or `session.adopt` for a session the runner knows was forked or cloned from a managed session, one whose plan names a compaction provider (§9.6) | `{parent?}`: the session it was forked from, when known (`errors::ForkUnsupportedDetail`) | no |
| `template_part_gone` | a template send whose `part_tag` slot names a part the request that would carry it does not show (§9.7); a queued send meeting it later ends in the send state of the same name instead | `{slot}` (`errors::TemplatePartGoneDetail`) | no |
| `unknown_send` | `send.status` names a `send_id` the session has no record of for the caller (§9.8) | — | no |
| `unknown_session` | an upload into a session that does not exist (§9.12) | — | no |
| `unknown_upload` | append, commit or send names an unknown upload (§9.12) | — | no |
| `upload_not_committed` | a send names an open upload | — | no |
| `upload_gone` | append, commit or send names a known lost/deleted upload | — | no |
| `upload_chunk_too_large` | an append exceeds 2 MiB decoded bytes | — | no |
| `upload_offset_mismatch` | an append is not at the current offset | `{received}` (`errors::UploadOffsetMismatchDetail`) | no: resume at received |
| `upload_size_mismatch` | an append overruns the declared size, or commit finds a different size | — | no |
| `upload_sha256_mismatch` | commit finds a different SHA-256 | — | no |
| `media_unsupported` | a send's media is unsupported by the frozen model | — | no |
| `unknown_media` | the bound session's history does not reference the digest | — | no |
| `context_ceiling` | a `session.send` that would start or join a run while the session's context is at or over its ceiling (§6); never a model or provider failure | `{window, size, cause}` (`errors::ContextCeilingDetail`): the window and the request's estimated size in tokens, and `cause`, an open string: `view_timeout` (the runner waited the compaction provider's bound and no view came), `provider_unavailable` (no compaction provider at setup, or it is unreachable at the ceiling) or `over_window` (the request cannot fit even after the current view) | no; a router charges no backoff to the model for it |
| `expired` | a read of an expired session (§9.1), including `run.status`, `run.cancel` and `session.retract` where their groups are declared | `expired_at_ms` (Unix epoch milliseconds) | no |
| `retention_required` | first send or adopt omitted a required policy (§9.14) | — | no |
| `retention_conflict` | policy differs from the starter's frozen policy (§9.14) | `frozen` (policy) | no |
| `session_released` | send while release deletion is pending (§9.14) | — | **yes** |
| `backup_unsupported` | runner cannot back up a single session (§9.14) | `reason` | no |
| `send_id_reuse` | a `send_id` reused with another payload or mode (another list in `delivery` included, §9.7), a `session.adopt` `send_id` reused with another plan (§9.5), or a `session.refresh_policy` `send_id` reused with another `generation` or `policy` (§10.2) | `field` when the runner can identify it; `plan` on `session.adopt` | no |
| `send_retracted` | a re-send of a send whose submission `session.retract` withdrew (§9.2) | — | no |
| `delivery_unsupported` | a send whose legacy delivery mode is undeclared, or a `delivery_order` list with no declared member (§9.7) | `delivery`: the mode string or the ordered list as sent | no |
| `run_paused` | a send behind a paused run that the runner refuses rather than queues (§9) | `{run_id, reason}` | no |
| `scope_owner_mismatch` | an owner-only call from anyone but the owner: `session.send` (except an admitted unkeyed non-owner send, §9, or, on a `delivery_order` runner, a send from a sender the plan names, §9.7), `session.baseline`, `run.cancel`, `session.retract`, `session.adopt` (§9.5), `session.user_command` or `session.user_command_status` (§9.10) | the accepted identity | no |
| `scope_adoption_refused` | a scoped send into a session recorded without a scope that cannot adopt it | `no_recorded_principal` or `principal_mismatch {recorded}` | no |
| `scope_unsupported` | a send under a scope on a daemon without scopes | — | no |
| `fetch_unavailable` | a required plan item missed the admission deadline, including when a `session.refresh` fetch misses its deadline (§10.2) | `provider` | **yes** |
| `system_text_preset_required` | a preset-less `system_text` item's provider lacks `system-text-default/v1`; nothing is written or forwarded; never timed (§10.1) | `provider` (module id) | no: terminal |
| `plan_stale` | fetched items disagree with the composition | `differences` | no: re-plan |
| `tool_name_collision` | tools share a model-facing name | `name`, `providers` (every provider that offered it) | no |
| `user_command_collision` | plan providers share a command name | `name`, `providers` (every declaring provider) | no |
| `unknown_user_command` | the session did not freeze this command | `command` | no |
| `unknown_invocation` | status key is not held by the session | — | no |
| `no_person` | the session plan has `interactive: false` | — | no |
| `user_commands_unsupported` | runner does not declare `user_commands` | — | no |
| `plan_drift` | a later send's plan differs from the frozen one (while a change is pending, from both the effective and the latest accepted plan, which `frozen` then names, §10.2); a `session.adopt` plan differs from the one applied at host start (§9.5) | `frozen?`, `sent` (plan identities; no `frozen` after a plan-less first episode, or on `session.adopt` when no plan was applied) | no |
| `pre_user_unavailable` | a steer or queue under an unavailable `refuse` PreUser | `provider` | no |
| `transient` | a condition that may clear by itself | — | **yes** |
| `scope_not_synced` | the scope's owner has not re-synced after a daemon restart | — | **yes**, with backoff within the hold bound |
| `rung_unsupported` | a `session.refresh` or `session.refresh_policy` whose policy names a rung the session does not support for a surface the change touches | `{surface, rung}` | no |
| `pending_superseded` | a `session.refresh_policy` for a generation a later refresh superseded | `{generation, pending}` | no |
| `already_applied` | a `session.refresh_policy` for a generation already applied | `{generation, applied_by}` | no |
| `generation_stale` | a `session.refresh` at or below the accepted generation that is not an identical repeat (§10.2) | `{accepted}` | no |
| `tool_outside_host_set` | a `session.refresh` plan naming a tool outside the tool set the host fixed at start, a changed definition under a kept name included (§10.2) | `{names}` | no |

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
| `compaction_unavailable` | Setup only: Setup failed or timed out with no answer; no model call is made, and no `provider_detail_code` is carried (§11.1) |
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
values (its §10) as `provider_code`; this role does not repeat them. Their
retryability, and that of `compaction_unavailable`, is the
compaction-provider contract's. A stop at the context ceiling is never a
provider code: it is the `context_ceiling` refusal or run state (§6).

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
`errors::provider_codes::CODES` in the role crate lists every provider code
above.

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
  strings): `role.describe`, every reply and session-stream event a consumer decodes
  (`ReadPage`, `ModelPage`, `HeadMeta`, `HistoryPage`, `RunResult`, `SubscribeEvent`,
  `SendReply`, `BaselineReply`, `AdoptReply`, `ResolveReply`,
  `ToolCallsPage`, `RefreshReply`, `RefreshPolicyReply`,
  `FlushPrefixReply`, and the `run.status`, `run.cancel` and
  `session.retract` answers). Leniency is inside the answer: the reply body
  around it is exactly `{"result": <answer>}` (§1). A run call's `outcome`, a ceiling's
  `cause` and a read entry's `delivered_as.mode` are open strings. One
  exception: a model-view entry's `source` kind is strict (§8).
- [pinned] **Closed live hint**: `step.finished.v1` refuses every unknown
  member (§14.3), while its status and classification strings stay open.
  `role.describe.step_events` is likewise closed and checked by name.
- [pinned] **Strict on unknown fields** wherever absence changes the
  question: `session.read`, `session.head`, `session.history`, `run.result`, `run.status`,
  `run.tool_calls`, `session.subscribe`, `session.baseline`,
  `session.adopt`, `session.resolve`, `session.refresh`,
  `session.refresh_policy` and `session.flush_prefix` requests. A
  runner refuses an unknown field with `invalid_params` naming it.
- [pinned] **Lenient on fields, strict on values** where the request grows:
  `session.send` tolerates unknown fields (a newer owner against an older
  runner degrades rather than fails), but `delivery` refuses an unknown value
  naming the field: a string it does not know, or a list (on a runner
  without `delivery_order`), or a malformed list. `retention` is a recognized field even on a runner without
  its group, and is refused there (§9.1), never treated as an unknown extra.
  `view` on `session.read` is strict the same way, and so are the list
  members `dedupe_key`, `expires_at_ms` and `template` on a runner that
  declares `delivery_order` (§9.7).
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
   after a lineage change, a cut's included, is refused `lineage_changed`.
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
   the call without a result. A seal has exactly two valid shapes for such a
   written but never-dispatched call, and the follow-up preserves the shape
   the seal chose:
   - **(a) Drop it.** The runner drops the never-sent call from history.
     Neither the cut-run transcript nor the follow-up transcript carries its
     id or `call_key`, attribution, or result. The assistant step's text may
     also be dropped, or retained exactly once without the call. Dropping or
     rewriting a written message starts a new lineage, whether or not a
     reader saw the message (the runner cannot know): reads of the old
     lineage are refused `lineage_changed`, and a reader resyncs from the
     new one.
   - **(b) Keep it and close it.** The runner keeps the call and appends a
     never-executed result for it in the same durable commit as the run's
     `interrupted` terminal state, the shape a hard stop gives every
     pending, undispatched call (§6): the runner's native not-executed
     result, the same refusal-before-dispatch shape used for hook denials,
     with its body layout runner-specific (§12.3) and no dispatch intent
     recorded. The result is written directly after the message carrying
     the call. The call is never dispatched and never reported
     indeterminate; `run.tool_calls` (§6.3) lists it with `outcome:
     not_executed`; no later model request carries the call without its
     result; and the follow-up never writes a second result for it. Nothing
     written is dropped or rewritten, so the lineage stays the same and
     every message already read keeps its bytes.

   Read stability (§4.2) holds without exception in both shapes above:
   dropping the call (a) moves to a new lineage because it changes written
   bytes, and keeping and closing it (b) keeps the lineage only because it
   appends and changes none.
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
    which calls it marks indeterminate. The informational record is never
    included in model input.
 9. **Expiry deletion survives a crash.** A durable retention tombstone makes
    the lineage unreadable; restart finishes deleting its transcript and all
    runner-held derived copies (§9.1).
10. [pinned] **A forwarded person command is never resent after an indeterminate
    result.** Its invocation key is durable before forwarding. If the reply is not
    stored after a crash, timeout or lost connection, restart stores and replays
    terminal `outcome_unknown`; it never forwards that key again (§9.10).

Kill points (`points.rs`). [pinned] The list below names durable states at which
conformance can stop a runner. A runner serving `user_commands` includes
`UserCommandForwarded`. Approval-gated calls may add kill points defined by
their role.
| Point | Durable state after the kill | Property |
|---|---|---|
| `Admitted` | the start record; no model request sent | 1, 6 |
| `SendRecorded` | a send's message with its `send_id` (and PreUser output); no reply sent | 6 |
| `CompactionApplied` | a fetched view recorded in the write-ahead log; the step that first renders it not sent | 5 |
| `FoldRecorded` | a prefix rebuild applying a pending change; its request not sent | 5 |
| `StepRecorded` | an assistant step with its PostAssistant output; no dispatch intent | 4, 5 |
| `DispatchIntent` | a call's dispatch intent; the call may have been sent | 4, 7 |
| `ToolResultRecorded` | a call's result with its PostTool output | 4, 5 |
| `Terminal` | the run's terminal state | 3 |
| `RetentionTombstoned` | a content-free expiry tombstone; transcript and derived copies not yet deleted | 9 |
| `ReleasePending` | release pending was durably recorded and answered, before deferred deletion (§9.14) | 9 |
| `UserCommandForwarded` | the invocation is durably reserved and its provider received it; the reply is not stored | 10, §9.10 |

A runner whose durable state lives in a store where truncation is not
meaningful reaches these points with a fault hook followed by a real process
kill, and declares that. A suite run fails unless at least one kill was a
real process kill.

## 14.1 Session reference

- [pinned] A session's `session_ref` is the first 16 bytes of SHA-256 over
  the RFC 8785 (JCS) bytes of `{"harness", "runner", "session"}`. It is
  written as 32 lowercase hexadecimal characters. `runner` is the runner
  principal's bare module id exactly as recorded in `session_binding.runner`
  (for example, `runner-a`). It is never the principal's `reserved:runner-a`
  display form, any other prefix, the harness id or a folder. Extract the bare
  module id from the attested principal before constructing `SessionIdentity`;
  the shared function never strips prefixes. Session values are unique per
  runner and harness. Every field is passed exactly as given, never normalised.
- [pinned] Project folders are not part of the reference. A folder can have
  multiple spellings (for example `/var/…` and `/private/var/…` on macOS);
  those aliases must not split one hosted session's identity.
- [pinned] Any module that keeps state for a runner session (a compaction
  provider, a backup or move participant) derives the same `session_ref`
  through the shared `cortexkit_role_llm_runner::session_ref` function with
  `&SessionIdentity { runner, harness, session }`, without asking the runner
  or implementing its own hash. Named fields ensure an old positional call
  cannot compile and silently hash the wrong inputs.
  `test-vectors/llm-runner-v1/session-ref.json` pins the derivation, including
  runner separation, bare id versus principal display form, non-ASCII strings
  and escaping.
- [pinned] Migration: a module that stored folder-based v1 references re-keys
  live sessions once; ended rows keep their v1 references. The deprecated
  `session_ref_v1(harness, project_root, session)` exists only to migrate
  stored refs and will be removed in a later release. Its legacy derivation
  is pinned by `test-vectors/llm-runner-v1/session-ref-v1.json`.
- [pinned] `session_ref` is a correlation key, not proof of identity: a
  direct caller chooses its own bind fields. Authorization still rests on
  the daemon-stamped principal and scope.

- [pinned] The conformance case `session_ref_uses_runner_identity` requires
  `compaction` and `queue` and the adapter's runner module id declaration.
  Bus-captured advance-notice headers must match the shared derivation, not
  a folder-based reference.

## 14.2 Per-step cost and memory

A session can run for months: a seven-month session holds 150,000 or more
messages. A runner never does per-step work, or holds memory, in proportion
to the session's whole length.

- [pinned] **Work.** Per step, a runner's work scales with the messages
  added since its previous step, plus a constant. It never re-reads,
  re-checks or re-renders the window each step: what it derived for earlier
  messages (rendered bytes, tags, sizes, digests) it keeps and extends.
  Writing the model request itself is excluded, because the model needs
  the whole window.
- [pinned] **Memory.** A runner's resident memory for a session scales
  with the window since the latest cut, never with the session's length.
  History before the cut stays on disk, read on demand only
  (`session.history`, §4.6, which sits outside this rule).
- [pinned] **Acceptance.** Each runner proves it with its own benchmark: a
  session seeded with 150,000 messages against one seeded with 1,000, the
  same window since the latest cut in both, with the median step time
  within 2× and resident memory flat. This is the acceptance rule, and the
  runner's own CI runs it. The conformance kit offers the timing half as
  the opt-in case `per_step_cost_scales_with_new_messages` (§15); it does
  not measure memory and does not enforce the rule on a runner that does
  not opt in.

## 14.3 `step_events`: content-free live step hints

- [pinned] A runner publishing these hints lists `step_events` in
  `role.describe.capabilities` and supplies `step_events: {granularity,
  history_key}`. `granularity` is exactly `per_attempt` or
  `per_completed_step`; `history_key` is exactly `step_ref` or `harness`.
  The declaration is closed. `check_describe` refuses malformed declarations
  as `invalid_step_events`, a declared group without metadata as
  `step_events_missing`, and metadata without the group as
  `step_events_undeclared`.
- [pinned] The runner publishes one `step.finished.v1` event per durably
  recorded step at its declared granularity, **only after the record is
  durable, never before**. With `per_attempt`, each recorded attempt is
  visible. With `per_completed_step`, attempts the host retried inside one
  step are invisible: a missing attempt is not evidence of a lost event.
  Terminal failed, cancelled and interrupted steps are included, not only
  successful steps.
- [pinned] The subject is `ck.{acct}.live.<runner>.step.finished.v1`, where
  `runner` is the publishing module id. The `live` root is beside `event`,
  not beneath it. This is a plain publish; no stream captures it and nothing
  is stored on the bus. A deleted session therefore leaves nothing behind
  there. Consumers subscribe to granted live subjects without connecting to
  every runner. An app with account-wide live access can use
  `ck.{acct}.live.*.step.finished.v1`; a filtered consumer uses its allowed
  per-runner subjects.
- [pinned] The subject's module token is the authority for who published:
  bus grants let a module publish only on its own live subjects. A consumer
  takes the publisher from that token and drops an event whose body `runner`
  differs. `step_events::decode_step_finished` checks the concrete subject,
  closed body and publisher agreement before returning a hint. Publisher
  identity does not make the hint authoritative history or grant any right.
- [pinned] The body (`step_events::StepFinished`) has exactly the following
  closed field list. Unknown members, including content-looking members such
  as `prompt` or `project_root`, are refused on decode. Identifiers and open classifications
  must never carry prompt text, tool names or arguments, error text, or any
  other host-supplied user content. Optional means unknown when absent, never
  zero; optional members are omitted when unavailable.

  | Fields | Wire type and meaning |
  |---|---|
  | `runner`, `session_ref`, `log_incarnation`, `seq` | Required strings for the first three, unsigned 64-bit integer for `seq`; together the durable `step_ref` |
  | `run_id`, `step_id`, `attempt`, `terminal_status` | Required identifier strings, positive 64-bit integer attempt (1-based), and open status string (`completed`, `failed`, `cancelled`, `interrupted`, or a future value) |
  | `harness_step_ref?`, `provider_request_id?` | Optional host-record and vendor-request identifier strings |
  | `attempt_id?` | Opaque provider-attempt identifier string, required when `harness_step_ref` is absent; optional when it is present |
  | `harness`, `session` | Required strings: the harness and session the runner received |
  | `work_class?`, `provider`, `model` | Optional open work-class string, required provider identifier and the model actually sent |
  | `input_tokens?`, `cached_input_tokens?`, `cache_write_tokens?`, `output_tokens?`, `reasoning_tokens?` | Optional unsigned 64-bit per-step counters, with the same names as run usage |
  | `cache_write_5m_tokens?`, `cache_write_1h_tokens?` | Optional unsigned 64-bit cache-write counters split by TTL; absent is not zero and a partial split need not add up to the total |
  | `reasoning_included_in_output?` | Optional boolean stating whether reasoning tokens are included in output tokens; absence does not imply either convention |
  | `request_body_bytes?`, `context_limit?`, `finish_reason?` | Optional unsigned 64-bit request-byte count and context-token limit, and optional open finish-reason string |
  | `started_at_ms`, `finished_at_ms` | Required unsigned 64-bit Unix epoch millisecond timestamps |

- [pinned] `attempt_id` is 1–128 characters of `[A-Za-z0-9_-]`, unique per
  provider attempt and never reused, including after a session restore. It
  is the exact value the runner writes in its own durable step record.
  Before publishing, `StepFinished::check` refuses an event with neither
  `harness_step_ref` nor `attempt_id`, or with an `attempt_id` outside that
  grammar (`step_events::valid_attempt_id`); `decode_step_finished` checks
  the same rule. Without `harness_step_ref`, the row identity of a runner
  step is `(runner, attempt_id)`. `run_id`, `step_id` and `attempt` are display
  facts: their triple can repeat after a restore and is not an identity.
- [pinned] `seq` is the runner's **native durable log position**, scoped by
  runner, session reference and log incarnation: the position acknowledged by
  the step-history consumer. It is never assumed to equal a `session.read`
  message ordinal or a `session.subscribe` frame sequence. This adds no field
  to `session.read` or `session.history`.
- [pinned] `session_ref` is the §14.1 reference, computed by the publisher with
  the shared `cortexkit_role_llm_runner::session_ref` function on
  `&SessionIdentity { runner, harness, session }`. `runner` is the attested
  principal's bare module id from `session_binding.runner`, never its
  `reserved:<id>` display form or a folder. The reference remains opaque to
  consumers; consumers never derive it. A live hint carries no folder path or project-root
  member: paths name user directories and do not belong in a content-free hint.
- [pinned] With `history_key: "harness"`, `harness_step_ref` is the host's own
  record id (for example a request id or message id). Its absence means the
  attempt has no host record, such as a refusal before forwarding; an app
  shows it as unmatched. With `history_key: "step_ref"`, the member is
  omitted. `provider_request_id` is absent when the provider gave none.
- [pinned] The event is a hint for a live view, **never authority, never
  billing, and grants nothing**. Missed or duplicate delivery may briefly
  stale a view; durable reads repair it. Consumer rule: **subscribe first
  and buffer, then read the native durable step history** (directly or through
  the step-history consumer), then drop buffered events whose `step_ref`
  (or present `harness_step_ref` under the harness key) is already in history.
  After reconnecting, re-query history by cursor shortly after reconnect:
  the step-history consumer may take seconds to read a just-recorded step.
- [pinned] The event owns a separate closed wire shape. `StepUsage` supplies
  the five per-step counters through `with_usage`, but is not itself a wire
  shape. `RunUsage` is a lenient aggregate with required step counts and a
  per-model list, so neither type is changed or flattened into the event.
- [pinned] The kit gates its three cases on `step_events`. Its adapter reads
  native durable step positions independently of actual captured publishes,
  and observes publish order at the runner's durability point. Missing
  adapter evidence fails by case name; a later read alone does not prove
  publish-after-durability.

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
| `model_view_half_open_ranges_and_travel` | `compaction`, `model_view`, `queue`, `transcript_reads`, `run_ops`; Setup's head insertion precedes and travels with message zero over both caps and is not repeated on the next page, and the held model request carries the canonical view |
| `compaction_unavailable_distinct_from_refuse` | `compaction`, `queue`, `transcript_reads`, `run_ops`; Setup-only unavailability and role/finer Setup refusal codes |
| the runner half of compaction-provider 0.3.0 §3 (notices, views, rides, the ceiling, native markers) | no live case yet; a later conformance release adds them |
| `role_describe_shape`, `role_describe_cacheable`, `role_describe_groups_complete` | — |
| `extra_op_still_admitted` | — |
| `baseline_owner_only`, `baseline_matches_admission_reply`, `baseline_not_yet_before_first_request` | — |
| `compaction_item_without_group_refused_writes_nothing` | a subject that does not declare `compaction` |
| `tail_read_is_newest_page` (on a session longer than one page), `range_read_stops_by_count`, `range_read_stops_by_bytes`, `after_mid_reads_strictly_after`, `after_mid_unknown_mid_refused`, `read_other_lineage_refused`, `lineage_id_on_every_page`, `unknown_read_field_refused`, `head_has_no_bodies`, `include_originals`, `oversize_message_alone_on_its_page`, `never_written_session_empty_page`, `never_written_session_empty_head`, `describe_states_max_bytes` | `transcript_reads` |
| `run_result_completed_final_text`, `run_result_text_parts_joined_unchanged`, `run_result_completed_no_text_is_empty`, `run_result_interrupted_not_cancelled`, `run_attribution_per_message` | `run_ops` |
| `dispatched_to_per_call`, `indeterminate_until_closed`, `sibling_calls_distinct_call_keys`, `recurring_model_id_distinct_call_keys` | `dispatch_attribution` |
| `subscribe_from_head_no_gap_no_duplicate` | `streaming` |
| `model_view_differs_only_by_compaction`, `model_view_entry_sources`, `model_view_replacement_once_on_its_first_page`, `model_view_after_mid_refused`, `model_view_names_compaction_state` | `model_view` |
| `session_refresh_applies_from_next_run`, `session_refresh_waits_for_its_rung`, `session_refresh_repeat_same_answer`, `session_refresh_generation_stale_refused`, `session_refresh_tool_outside_host_set_refused` | `session_change`, `queue`, `run_ops`, and the adapter's refresh plans and model-request observations; each answer shape of `session.refresh`, an identical repeat answered the same, `generation_stale`, and `tool_outside_host_set` for a tool outside the host's fixed set and for a changed definition under a kept name (inapplicable on a host that fixes no tool set) |
| `session_refresh_never_changes_a_running_run`, `flush_prefix_applies_pending` | the above, and held tool calls; a refresh, or a flush, while a run is running leaves that run's tools unchanged and applies at the next run |
| `refresh_unsupported_rung_refused`, `refresh_policy_superseded_refused`, `refresh_policy_already_applied_refused` | `session_change`; planned |
| `steer_lands_at_step_boundary`, `steer_never_between_call_and_result` | `steer` |
| `guaranteed_steer_never_pending_or_unknown` | `run_ops`, `queue`, `transcript_reads`; not applicable without `steer`, on a runner that declares `steer_receipt: confirm`, or without a held tool call; a delivered step receipt names the joined run |
| `steer_without_run_ops_reports_state_and_step` | `steer`, `queue`, `transcript_reads`, held tool call, and no `run_ops`; a steer into the active turn answers `active` or `finished`, `delivered.as: "step"`, a non-empty `ref`, and no `run_id`; `step` is never a state value |
| `resend_steer_delivered_stable` | `queue`, `transcript_reads`; not applicable without `steer` |
| `queue_starts_next_turn` | `queue` |
| `interrupt_cancels_then_starts`, `interrupt_waits_for_running_tool` | `interrupt` |
| `undeclared_delivery_refused` | a delivery mode the subject does not declare |
| `send_id_retry_same_answer`, `send_id_reuse_refused_naming_field`, `delivery_change_refused`, `unknown_delivery_refused`, `pre_user_refuse_writes_nothing`, `steer_inherits_frozen_runner_params` | `steer`, `queue` or `interrupt` |
| `send_id_retry_settled_same_answer` | `steer` or `queue`, and `run_ops` |
| `send_id_retry_written_once`, `send_id_reuse_writes_nothing`, `delivery_change_writes_nothing` | `steer`, `queue` or `interrupt`, and `transcript_reads` |
| `crash_at_StepRecorded`: resumes with exactly one dispatch or seals `interrupted` without dispatch; the call is never indeterminate and no later request carries it without a result; a seal either drops the call (or its step) and moves the session to a new lineage whose old reads are refused `lineage_changed`, or keeps the call and lineage with its never-executed result in the message right after it, already written once the run ended, and every message read before the kill unchanged | `transcript_reads`, `dispatch_attribution`, and `StepRecorded` |
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
| `adopt_host_turn_is_a_run` | `adopt`, `transcript_reads`, `run_ops`, and the adapter's host-start hook; the person's own turn has a `run_id` on `session.head`, `run.result` answers it, and `run.status` and `session.subscribe` show it where declared; the owner's next send is a run of its own; inapplicable when the adapter cannot start a host turn |
| `tags_on_every_read_entry`, `tags_numbers_stable_within_a_lineage`, `tags_only_on_tag_rendering_sessions`, `tags_resolve_returns_exposed_content`, `tags_resolve_unknown_tag_refused`, `tags_resolve_range_refused`, `tags_model_written_tags_cleaned_on_exposure`, `tags_count_only_parts_rendered_with_a_number` | `tags`, `queue`, `transcript_reads`; the adapter's plan-member send fields carrying `render_tags: true` (§9.6: numbering applies only to tag-rendering sessions), and without it for the untagged halves; the live `role.describe` must declare `tags` before any `session.resolve` is sent; the structured-output half of the cleaning case where `structured_output` is declared, and the image-only half of the counting case where the adapter's tool provider answers an image-only result, each named as unchecked otherwise; reported as a named skip without `tags` |
| `tags_numbered_in_rendered_order` | `tags`, `queue`, `transcript_reads`, `steer`, held tool calls, and the adapter's record of each model request's rendered texts; a steer sent while a call is held: the last request's texts carry exactly the numbers `session.read` reports, increasing in the order the model saw them; inapplicable when the runner rendered the steer where it stored it |
| `tags_renumbering_waits_for_the_step_boundary` | `tags`, `queue`, `transcript_reads`, `steer`, held tool calls; while the call is held, a steer changes neither the lineage nor a number read before it; the call runs once; afterwards a part a read exposed carries another number only on a new lineage |
| `tags_setup_checkpoint_anchors_at_its_cut` | `tags`, `tag_checkpoint`, `compaction`, `queue`, `transcript_reads`; a Setup checkpoint of 500 cut at the session's first message numbers the prompt 501 and the answer 502 |
| `tags_lost_marker_key_keeps_numbers` | `tags` and adapter hooks to compact in the host and drop a key; fallback preserves every exposed number and lineage |
| `tags_host_store_cleaned_at_the_save_seam` | `tags` and a host-store read hook; tagged replies are clean there, untagged replies and structured-output replies remain raw |
| `compaction_standalone_refusal_lifecycle` | `compaction`; handoff only before first service on in-process hosts, otherwise a named failure; the late rule requires an adapter Setup-repeat hook |
| `user_commands_cutoff_after_forward`, `user_commands_cutoff_before_forward` | `user_commands` and a cutoff adapter; `outcome_unknown` after forwarding, `failed` before, retained and replayed without another forward |
| `media_descriptors_image_bytes`, `media_descriptors_nested_media` | `transcript_reads` and byte-backed adapter fixtures; top-level and nested media hashes cover decoded bytes |
| `media_descriptors_frame_budget`, `media_descriptors_utf8_head` | fixture-backed over-frame arguments are served as refs; a text head ends before a crossing UTF-8 character |
| `media_descriptors_inline_unchanged`, `media_descriptors_full_content_digests`, `media_descriptors_pairing`, `media_descriptors_part_keys` | inline compatibility, full-inline identities, pairing preservation and strict digest-row keys |
| `media_descriptors_stable_across_pages` | multipart fixture; fixed-budget fallback is independent of page position and byte cap |
| `media_descriptors_model_view_refused` | `model_view`; descriptors are refused before read work, naming `media` |
| `head_names_the_cut`, `notice_carries_first_ordinal`, `revert_below_the_cut_restarts_at_the_previous_cut`, `revert_below_a_cut_before_adoption_has_no_descent` | the cut cases' requirements; the revert cases also need the adapter's revert hook, and the last the hook that gives a session a host cut before adoption; inapplicable by name without them: `session.head.cut` names the cut message by `mid`, digest and time, and a lineage without a cut has none; every notice on a cut lineage carries `first_ordinal`; a revert below the latest cut restarts at the previous cut with `descends_from` naming the lineage at that cut, or at the first message with no cut and no descent, and Setup runs again; a cut that predates the runner's first lineage gives no `descends_from` |
| `cut_starts_a_new_lineage`, `old_lineage_reads_refused_after_a_cut`, `read_below_first_ordinal_refused`, `ordinals_never_renumber_across_a_cut`, `setup_once_per_lineage`, `view_ranges_start_at_first_ordinal`, `cut_lineage_notice_descends_from` | `compaction`, `queue`, `transcript_reads`, and the adapter's view-publishing hook and notice observation; inapplicable by name without the hook: after a view with a cut applies, `lineage_id` changes and `first_ordinal` is the cut's first retained ordinal; reads naming the old lineage (`after_mid` included) are refused `lineage_changed`; range and after-message reads below `first_ordinal` are refused `before_lineage_start` with `detail.first_ordinal`; retained messages keep their ordinals and `mid`s; Setup runs exactly once in each lineage; a view tiling from 0 on the cut lineage is refused `view_refused:<version>:ranges_start`; the cut lineage's first notice descends from the old lineage at the cut |
| `checkpoint_on_descending_setup_refused`, `cut_setup_leaves_reads_alone`, `cut_numbering_continues_from_the_old_lineage` | the cut cases' requirements, with `tag_checkpoint` for the first, the adapter's Setup-hold hooks and a runner that ever holds reads for the second, and `tags` with a tag-rendering plan for the third: a checkpoint on a cut lineage's Setup fails it `checkpoint_on_descent` with no model request; reads of the new lineage are served while a cut's Setup is pending; retained parts keep their numbers and the next part numbers one above the old lineage's highest |
| `history_version_unchanged_by_a_cut`, `history_version_rises_at_a_revert`, `history_from_ordinal_and_before_ordinal`, `history_unknown_ordinal_refused`, `history_before_lowest_ordinal_reaches_unnumbered_entries` | `host_history`, `queue`, `transcript_reads`; the cut case also `compaction`, `run_ops` and the view hook, the revert case the revert hook, the last a session with host history before the runner's first numbered message: `history_version` is unchanged on `session.head` and history pages by a cut and rises at a revert; ordinal seeks page from and before an assigned ordinal; an unassigned or reverted ordinal is refused `unknown_ordinal`; `before_ordinal` at the lowest ordinal reaches the unnumbered entries |
| `history_pages_back_to_the_first_message`, `history_mid_matches_read`, `history_no_tags_below_first_ordinal`, `history_ordinal_kept_below_the_cut`, `history_cursor_survives_a_cut`, `history_never_serves_a_reverted_branch`, `history_cursor_expired_after_a_revert_past_it`, `history_boundary_order`, `history_before_mid_continues_below_the_window` | `host_history`, `queue`, `transcript_reads`; the cut cases need the view-publishing hook, the revert cases the adapter's revert hook, and the last two a session with host history before the runner's first numbered message; each inapplicable by name without its hook |
| `per_step_cost_scales_with_new_messages` | `queue`, `transcript_reads` or `run_ops`, and the adapter's seeding hook, which writes finished messages through the runner's own store behind a cut; inapplicable without it, never passed: the median step time with 150,000 seeded messages is at most 2.0 times the median with 1,000 |
| `tags_fork_of_a_managed_session_refused` | `tags`, `compaction`, `queue`, `transcript_reads`, and the adapter's fork hook; a send into, and an adoption of, a fork of a managed session is refused `fork_unsupported`, writing nothing; inapplicable when the adapter cannot create a fork the runner can detect, since a runner refuses only forks it can know about |
| `tags_resolve_answers_the_current_lineage` | `tags`, `queue`, `transcript_reads`, `dispatch_attribution`, and `StepRecorded`; reads the `crash_at_StepRecorded` scenario: after a seal that moves the session to a new lineage (§14, shape (a)), every number the new lineage shows (up to 64) resolves to the part showing it there, never to a part of the lineage left, a number above its highest is refused `unknown_tag`, and the cut run's prompt carries a number; inapplicable when the runner resumes the run or keeps the call on its lineage (shape (b)); reported as a named skip without `tags` |
| `delivery_order_never_starts_a_run`, `delivery_order_dedupe_supersedes_queued`, `delivery_order_expired_never_delivered`, `delivery_order_unnamed_sender_refused` | `delivery_order`, `queue`, `transcript_reads`, `steer`; the adapter's plan-member send fields and a plan-named sender identity; the live `role.describe` must declare `delivery_order` before any list is sent; reported as a named skip without `delivery_order` or `steer` |
| `delivery_order_members_without_list_refused` | `delivery_order`, `queue`, `transcript_reads`; list members beside a string or absent `delivery` are refused naming the member and write nothing |
| `delivery_order_list_refused_without_group` | `queue`, `transcript_reads`; not applicable when `delivery_order` is declared: a list in `delivery` is refused `invalid_params` naming `delivery` and writes nothing |
| `delivery_order_first_possible_mode` | the above, and a held tool call: a list goes out with the next tool result when it names `tool_result` first, and starts a run on an idle session through `idle_start` |
| `delivery_order_undeclared_mode_refused` | `delivery_order`, `queue`, `transcript_reads`, held tool call, and `steer` left undeclared; `[steer]` is refused `delivery_unsupported` with the exact list in `detail.delivery` and writes nothing; `[steer, idle_start]` is accepted, skips the undeclared `steer` at the active boundary, then starts at the idle boundary; inapplicable when `steer` is declared |
| `delivery_order_declared_modes_respected` | `delivery_order`, `queue`, `transcript_reads`, and the live describe member `delivery_order_modes`; all non-empty ordered lists of distinct undeclared modes are refused `delivery_unsupported` with the original list in `detail.delivery` and no writes; every list containing a listed mode is accepted |
| `delivery_order_retract_pending_tool_result` | `retract`, `delivery_order`, `queue`, `transcript_reads`, and a held tool call; a `[tool_result]` send waits with `pending` and `submission_id`, `session.retract` answers `retracted` then `not_pending`, and after the call is released the send is never delivered |
| `delivery_order_steer_receipt_durable_across_restart` | `delivery_order`, `queue`, `transcript_reads`, `steer`, `run_ops`, and a held tool call; on a separate storage directory for this case, the harness restart hook (without a process kill): a `[steer]` list sent while a run waits on a held call answers `pending` with a `submission_id`, is delivered once as a steer when the call is released, and a re-send of its `send_id` answers `delivered: {as: "step", ref}` with `run_id` the run it joined, before the restart and unchanged after it; reported as a named skip without `delivery_order`, `steer` or `run_ops` |
| `send_status_answers_as_a_resend_after_restart` | `send_status`, `delivery_order`, `queue`, `transcript_reads`, `steer`, `run_ops`, and a held tool call; `send.status` of the same send answers what a re-send answers, `step` with the joined run's `run_id`, before the restart and after it, never `pending`; the live `role.describe` must declare `send_status` before any `send.status` is sent; reported as a named skip without `send_status`, `delivery_order`, `steer` or `run_ops` |
| `send_status_unknown_send_refused` | `send_status`, `queue`, `transcript_reads`; `send.status` of a settled send answers what its re-send answers, one of a `send_id` the session has no record of is refused `unknown_send`, and one carrying an unknown field is refused `invalid_params` naming it |
| `run_context_ceiling_wait_is_not_a_stall` | `queue`, `transcript_reads`, and the adapter's held context-ceiling wait; the run reads `active` with `reason: context_ceiling_wait` on `session.head` (and on `run.status`, with a future `deadline_ms`, where the major lists it) while held, then completes when a view arrives, or ends `context_ceiling` with `cause: view_timeout` when none does |
| `delivery_order_template_renders_as_system` | `delivery_order`, `queue`, `transcript_reads`; needs `steer` for user-authority fallback or a route that supports `turn_system`, and is inapplicable when neither is available; the template send carrying empty `prompt` is refused naming `template` |
| `delivery_order_bad_slot_refused` | `delivery_order`, `queue`, `transcript_reads`; an out-of-range slot is refused naming `template` and writes nothing |
| `delivery_order_template_part_gone_refused` | `delivery_order`, `queue`, `transcript_reads`, `tags`; on a tag-rendering session, a template send whose `part_tag` slot names a part the session does not show is refused `template_part_gone` naming the slot and writes nothing; one naming a shown part is accepted |
| `read_stable_across_host_metadata_change` | `queue`, `transcript_reads`, and the adapter's host-side metadata change: a re-read under the same lineage returns identical entries |
| `run_tool_calls_positions_identify_siblings` | `run_tool_calls`, `queue`, `transcript_reads`; two sibling calls have positions 0 and 1 at their requesting message's ordinal, independent of intervening text/reasoning parts |
| `run_tool_calls_result_ordinal_tracks_storage` | `run_tool_calls`, `queue`, `transcript_reads`, held tool call; no result ordinal while open, and a finished call's `result_ordinal >= ordinal` locates the stored result |
| `interrupt_reason_shutdown_durable` | `interrupt_reason`, `queue`, `transcript_reads`, `run_ops`, held tool call; orderly handle stop and restart preserve `runner_shutdown` across result, status where served and `last_run_state` |
| `turn_directives_in`, `turn_directives_note`, `turn_directives_off`, `turn_directives_not_requested_reports_none`, `turn_directives_unknown_refused` | `turn_directives`, `queue` (`run_ops` for result cases); scripted trailing tags on completed runs report exactly the requested forms, unrequested tags report none, invalid names/shapes/duplicates are refused naming `directives`; `run.status` is checked where served |
| `turn_directives_max_steps_reports_none`, `turn_directives_cancelled_reports_none` | the above plus `max_steps`, or `run_cancel` and held tool calls; non-completed runs report no directives, including the held non-terminal state |
| `run_tool_calls_listed_in_dispatch_order`, `run_tool_calls_unknown_run_refused` | `run_tool_calls`, `queue`, `transcript_reads`; one call per page under `limit: 1`, following `next`; reported as a named skip without `run_tool_calls` |
| `run_tool_calls_indeterminate_until_result` | the above, and a held tool call |
| `run_usage_counts_reported_steps` | `run_usage`, `queue`, `run_ops`; after a two-step scripted run whose provider reports input, cache-read, cache-write and output counts but no reasoning, `run.result` carries `usage` with `steps: 2`, the scripted totals, `by_model` summing to them, and `reasoning_tokens` absent rather than `0`; `run.status`, where the major lists it, carries the same `usage`; reported as a named skip without `run_usage` |
| `session_usage_sums_runs_since_first` | `session_usage`, `run_usage`, `queue`, `run_ops`, `transcript_reads`; two runs sum to session spend for reported fields, with `since` naming the first counted run |
| `session_usage_rewind_never_decreases` | same groups; a rewind of visible history does not reduce or reset spend |
| `session_usage_unknown_fields_absent` | same groups; reasoning usage no scripted step reported stays absent, never zero |
| `head_model_reflects_recorded_switch` | `queue`, `transcript_reads`; when the runner reports a model and the adapter can record a switch, the head follows that switch, not the frozen plan |
| `head_sender_is_callers_effective_grant` | `head_sender`, `queue`, `transcript_reads`; a plan names A with neither flag and B with both; A gets supported modes minus `idle_start`, B gets all supported modes and interrupt, an unnamed caller gets no grant, and the owner gets all supported modes, interrupt and `owner: true`; an undeclared `head_sender` group or defaulted identity hooks are inapplicable, with no supported-mode-based fallback |
| `call_identity_key_matches_attribution` | `call_identity`, `queue`, `transcript_reads`, `dispatch_attribution`, and `history_display` with `host_history` or `live_events` with `streaming`; every display tool call and live tool event of two dispatched sibling calls carries the `call_key` §5 attributes to that call; not applicable without the group, never skipped |
| `call_identity_pin_frozen_at_dispatch` | as above; each entry of a dispatched call carries the `schema_pin` the tool provider received with it, and still does after the catalog changes the tool's definition and another run; the catalog half is reported unchecked when the adapter cannot change the definition |
| `call_identity_denied_call_has_no_pin` | as above; a call denied before dispatch carries no `schema_pin`, and `call_key` exactly as §5 attributes it; inapplicable without the adapter's denial fixture |
| `head_incarnation_survives_cut_and_changes_on_recreation` | `queue`, `transcript_reads`; a reported incarnation stays equal across a real cut and differs after creation of a new log at the same key; defaulted cut/re-create hooks are inapplicable |
| `max_steps_seals_pending_calls` | `max_steps`, `queue`, `transcript_reads`, `run_ops`; a tool-calling model stops after the requested number of responses in `max_steps`, with every pending call's never-executed result already readable at the terminal and no pending call dispatched |
| `max_steps_final_step_completes` | `max_steps`, `queue`, `run_ops`; with `max_steps: 1`, a model whose one step is text ends `completed` with its final message, and with its directives where `turn_directives` is declared, never `max_steps` |
| `max_steps_submit_result_completes` | `max_steps`, `structured_output`, `queue`, `transcript_reads`, `run_ops`; with `max_steps: 1`, a step calling `submit_result` ends `completed` with the submitted value and its sibling `skipped_after_submit`, never `max_steps` |
| `max_output_tokens_caps_response` | `max_output_tokens`, `run_usage`, `queue`, `transcript_reads`, `run_ops`; the actual dispatched output maximum and the single response's reported usage are positive and never above the requested cap, including a request above the model's own maximum |
| `no_tools_offered_to_model` | `no_tools`, `queue`, `transcript_reads`; an unsuppressed control offers tools and `tools: []` offers none, observed at the scripted model |
| `no_tools_later_plan_send_refused` | `no_tools`, `queue`, `transcript_reads`; a fetch-plan session refuses any later `tools` member naming `tools`, writing nothing; reported inapplicable by name when the adapter cannot create a plan session |
| `run_status_undelivered_steers_present_when_clean` | `run_status`, `queue`; an ended run's status carries `undelivered_steers`, `[]` when every steer was rendered or none was sent |
| `held_calls_lists_waiting_call` | `held_calls`, `run_status`, `queue`, `transcript_reads`, `run_ops`; a scripted held first reply lists its call key, tool, provider, held reason and receipt time while waiting |
| `held_calls_applies_late_result` | the above plus rendered model-input observations; applying the provider's late result removes the entry and continues the run with that result |
| `held_calls_not_started_becomes_tool_result` | the above plus rendered model-input observations; a `not_started` late entry continues the run with a tool result saying the call never ran |
| `held_calls_expired_becomes_tool_result` | the above plus rendered model-input observations; an `expired` late entry removes the held call and continues the run with a tool result saying its answer expired |

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
- **Q4. Hosting compaction.** Settled: not a required op but a
  `compaction` group, all or nothing, owning everything in §11.1. A runner
  that only steers sessions it does not host declares none of it.
  `REQUIRED_OPS` is `role.describe` and `session.baseline`. In 0.3.0 the
  group is the runner half of compaction-provider 0.3.0 §3 and requires
  `tags`; the synchronous `compaction.ready` op it once owned is removed. A
  plan with a `compaction_item` on a runner without the group is refused at
  admission `invalid_params {field: "plan.compaction_item"}` and writes
  nothing (§3, §10.1, §11.1).
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
  `detail.delivery`, never falling back to `queue`. A `delivery_order` list
  with no individually declared mode is refused the same way, with the whole
  list in `detail.delivery` (§9.7). An absent `delivery` is `queue`, a
  declared group like the others, and a runner that admits sessions declares
  it (§3, §9).
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
