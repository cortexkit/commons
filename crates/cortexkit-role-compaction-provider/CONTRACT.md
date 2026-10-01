# `compaction-provider/v1` — role contract (draft)

Stability: **alpha, draft**. Written by the role's owner (Magic Context) for
review in the design room; nothing here ships until it signs off. This
document is the role definition; the Rust types in this crate are its wire
shapes, `test-vectors/compaction-provider-v1/` at the repository root holds
its vectors, and a separate `-conformance` crate will hold its suite (§17).

Derives from the CK extensibility design r7.3, §3.1, §5, §7, §8 and §13.2,
its owner corrections, and the rulings the design room settled on the
compaction interface. `llm-runner/v1` §11.1 states what the runner owes on
the same interface; this document states the provider's side. Appendix A
lists where the two disagree today.

Every item is marked:

- **[pinned]**: the design or a settled ruling states it. A provider must do
  it, and a runner may rely on it.
- **[open: Qn]**: the design is silent or ambiguous, and this draft writes one
  option down so the types and vectors have something to pin. Question `Qn`
  under "Open questions" lists the options.
- **[provisional]**: settled in substance, but its exact name or position
  depends on a section another role has not written yet.

Names and types this role shares with `llm-runner/v1` (the `compaction`
group, `compaction.ready`, the model view's `source`, the runner's codes)
are taken from that crate, which this crate depends on, so the two cannot
drift apart.

A compaction provider is any module that decides the shape of the history a
runner sends to the model. Nothing here names a particular implementation;
"Gaps in Magic Context today" at the end compares the one planned provider
with this document.

## 1. Role identity and addressing

- [pinned] A provider lists `compaction-provider/v1` in its manifest's
  `capabilities.provides` (`PROVIDES`). A module may serve several majors.
- [pinned] Required ops (`REQUIRED_OPS`): `role.describe`, the Setup op and
  the per-step op. [open: Q2] their names, drafted as `compaction.setup` and
  `compaction.step`.
- [pinned] A runner refuses a module whose `role.describe` lacks a required
  op, by name, before routing anything to it.
- [open: Q1] Every op is a request `{method, params}` on a route the runner
  opens to the provider under the session's scope: `method` is the op's
  name and `params` its request. The runner keeps one route per provider
  per session.
- [pinned] The provider sends one op the other way: `compaction.ready`
  (§10), which the runner serves.

## 2. `role.describe`

- [pinned] The answer is `{majors: [{version, ops, stability}],
  implementation_version, capabilities, runner_groups?}` (`RoleDescribe`),
  the shape `tool-provider/v1` and `llm-runner/v1` use. `version` is spelled
  as in the manifest, `compaction-provider/v1`.
- [pinned] Stability is data, never part of a name: `alpha`, `beta` or
  `stable`, read as `alpha` when absent. This draft is `alpha`.
- [pinned] Decoded leniently: unknown fields, majors, ops and capabilities
  are ignored, and an unknown `stability` decodes. Only the
  `compaction-provider/v1` major and its required ops are checked strictly
  (`check_describe`).
- [pinned] The answer describes the provider build, never a session, so a
  consumer may cache it for as long as it talks to the same module
  incarnation.
- [pinned] This role defines no module-level capabilities yet.
- [open: Q8] `runner_groups` lists the `llm-runner/v1` capability groups the
  provider needs from the runner besides `compaction`: `transcript_reads`
  for a provider with no reader of its own. The session's starter checks
  them against the runner's `role.describe` before it plans the session
  (`unmet_runner_groups`).

## 3. Pairing with a runner

- [pinned] A session has at most one compaction provider. Without one,
  history is sent as written.
- [pinned] Hosting a compaction provider is a capability group on the
  runner, `compaction` (`RUNNER_GROUP`), all or nothing: a runner that
  declares it serves `compaction.ready` and makes every call in this
  document. A runner without it cannot be paired with a compaction item; it
  refuses such a plan at admission with `invalid_params {field:
  "plan.compaction_item"}`.
- [pinned] The compaction provider is the session's one reduction owner:
  the only party that may remove or rewrite what was written. On the
  step-transform side, only it may `replace` on `pre_user` and
  `post_assistant`; every other step transform is preserving and runs in a
  separate ordered list (`step-transform-provider/v1` §5).
- [pinned] The plan names the provider in its `compaction_item`, with the
  item's `preset` and `params`. The runner passes the preset, the params
  and the session's composition to Setup verbatim.
- [provisional] The runner finds the session's compaction provider at
  `plan.compaction_item.provider` (`llm-runner/v1`'s
  `plan_compaction_provider`). That is what the
  `not_session_compaction_provider` check (§10) compares a route's stamp
  against. The position is provisional until the runner role's fetch-plan
  section defines `plan`.

## 4. `compaction.setup`

- [pinned] Called once per session, before its first model call. Its answer
  is recorded before the first model call; once recorded, Setup never runs
  again for the session and nothing replaces its initial view except a
  later CompactionMessage.
- [pinned] If Setup fails or times out with no answer, no model call is
  made and the run ends `error` with `provider_code`
  `compaction_unavailable`, `llm-runner/v1`'s code, which this crate
  re-exports (`errors::runner_codes::COMPACTION_UNAVAILABLE`). A Setup
  answered `refuse` ends it with the provider's own `code` instead (below).
  Either way the next send calls Setup again, because no initial view was
  recorded. So a provider answers Setup for a
  session it has seen before, and its answer is well formed whatever it
  answered last time (kill point `SetupRecorded`).
- [pinned] The request (`SetupRequest`) carries `session`, `request_id`, the
  item's `preset` and `params`, the `composition`, the first step's `model`
  (with `variant`, `context_window` and `output_limit` when known), `newest`
  when a message is already written, `lineage_id` when one exists, and
  `now`.
- [pinned] The answer (`SetupAnswer`) is `{answer: "ready", request_id,
  initial, stability?, call_when?}` or `{answer: "refuse", request_id, code,
  reason, retryable}`.
  - `initial` is a CompactionMessage (§8). Usually its range is empty at the
    start of the lineage, `{from: 0, to: 0}`: the provider's head is
    inserted before every message, and the model view shows it as the
    insertion `[0, 0)` (§12). A provider with no head returns an empty
    replacement.
  - `stability` ranks the provider's own messages: `index` is a position in
    every CompactionMessage's `replacement`, and a higher `rank` changes
    less often. A main session of Magic Context declares `[{index: 0, rank:
    2}, {index: 1, rank: 1}]`; a subagent session declares nothing. The
    runner uses these as stability segments for cache breakpoints and change
    policies. [open: Q13] the array shape.
  - `call_when` (§5).
  - A `refuse` ends the run `error` with `code` as its `provider_code`.

## 5. When the provider is called

- [pinned] Timing is data, frozen with the session, from Setup's
  `call_when`. There is no rule language.
- [pinned] The vocabulary is a default share of the context window plus
  optional per-model overrides, measured against the step's reported usage:
  `{default: 0.8, models: {"<model id or pattern>": 0.7}}` (`CallWhen`). A
  share is above 0 and at most 1. New condition kinds are additive.
- [open: Q9] A runner that meets a condition kind it does not know calls the
  provider on every step rather than guess (`has_unknown_conditions`).
  [open: Q9] how a model pattern matches.
- [pinned] With no `call_when`, the runner calls on every step. Whatever the
  conditions say, it always calls on a prefix rebuild and after an
  execution error.
- [pinned] On a step without a call, the runner keeps serving the last
  applied CompactionMessage's view: the same replacement and range, plus
  every newer message raw.

## 6. `compaction.step`: the status

The status (`StepStatus`) carries:

- [pinned] `session`, the runner's opaque name for the session. The
  provider keys its per-session state on it and echoes it in
  `compaction.ready`; it never chooses bytes from it.
- [pinned] `request_id`, the id of this request (§9): an opaque string the
  provider compares only for equality, the type `llm-runner/v1` gives it.
- [pinned] `lineage_id`. Ordinals, the cursor and every range are in this
  lineage.
- [pinned] The step: `step_id` (opaque) and `step_kind`, `user_turn` or
  `tool_step`, decoded open.
- [pinned] `model` and `variant`; `context_window` and `output_limit` when
  the runner's model catalog resolves them, absent otherwise.
- [pinned] `previous_usage {input, cache_read, cache_write, output,
  completed_at, finish_reason}` when the previous step recorded usage.
- [pinned] `previous_provider_code` when the previous run ended `error`. An
  overflow ends a run that way with no usage, so the code can come alone.
- [pinned] `estimate {request_tokens, previous_input?}`: the runner's
  estimate of the request about to be sent, labelled as one, plus the
  previous step's measured input.
- [pinned] `prefix_rebuilding {reason}` when this step's prefix is rebuilt
  anyway; `reason` is `flush`, `manifest_change`, `model_switch` or
  `expired_cache`, decoded open.
- [pinned] `newest {ordinal, mid}`, the newest message written; absent on a
  lineage with none.
- [pinned] `last_applied {compaction_id, version}`, so a provider that lost
  its counter resumes above it (`next_version`).
- [open: Q7] `last_not_applied {compaction_id, version, reason}`: the newest
  CompactionMessage the runner recorded and did not apply, when it is newer
  than the last applied, with the reason (§9, or `structural` for the
  runner's own checks). Without it a provider cannot tell a refused answer
  from one still in flight.
- [pinned] The cursor and the messages after it: `after_ordinal`, the last
  ordinal of this lineage the provider was sent (absent on the first call in
  the lineage), and `messages: [{ordinal, mid, message}]`, every message
  after it, oldest first. `message` is the runner's message schema, with
  final values, as `session.read` pages carry it.
- [open: Q6] The runner may stop `messages` at a byte cap before the newest
  message and set `more: true`. A single message over the cap comes alone.
  The cursor then advances only to the last message sent.
- [pinned] `now`, the runner's clock in milliseconds since the Unix epoch.
- [pinned] The cursor is kept per provider per session, scoped to
  `lineage_id`, written in the same record as that call's answer, and
  advances only once the answer is durable. A crash re-sends messages; it
  never skips them. `noop` advances it.
- [pinned] So a provider must take in each ordinal once: a status may repeat
  messages it already holds (kill point `MessagesIngested`). A status whose
  `after_ordinal` is above the last ordinal the provider holds is a gap: the
  provider reads the missing messages through the runner's
  `transcript_reads` or its own reader, or answers `refuse` with
  `history_unreadable`.
- [pinned] A status on a new `lineage_id` starts the cursor again for that
  lineage. The provider's state for the old lineage says nothing about the
  new one.
- [pinned] Decoded leniently: a provider ignores fields it does not know, so
  a newer runner degrades rather than fails against an older provider.

## 7. The answers

`StepAnswer`, tagged by `answer`, each naming the `request_id` it answers:

| Answer | Shape | The runner |
|---|---|---|
| `noop` | `{request_id}` | records it; the cursor advances; the request goes out unchanged |
| `compaction_message` | `{request_id, compaction}` | applies the CompactionMessage if §9 allows |
| `wait` | `{request_id, reason, bound_ms}` | holds the step (§10) |
| `refuse` | `{request_id, code, reason, retryable}` | ends the run (§11) |

- [pinned] The four answers and their meaning. [open: Q2] the snake_case
  wire spellings.
- [pinned] Strict on `answer`: an unknown or misspelled answer does not
  decode. The runner treats it as a failed call and keeps the last applied
  CompactionMessage; it never reads it as `noop`.
- [pinned] A call has a budget of about 2 s for `noop` and
  `compaction_message`. On a timeout the runner records it and proceeds with
  the last applied CompactionMessage; one always exists, because Setup's is
  recorded first. A provider that needs longer answers `wait`.
- [pinned] A provider's `ERROR` refusal of the call (§14) counts as a failed
  call, like a timeout.

## 8. CompactionMessage

`{compaction_id, version, range: {from, to}, replacement}`
(`CompactionMessage`).

- [pinned] It replaces one contiguous range of messages. The replacement
  stands in for the range; messages before the range are not sent; messages
  from its end onward are sent as written.
- [pinned] The range is half-open over ordinals of the request's lineage:
  `from` included, `to` excluded, the same convention as the model view's
  `from_ordinal` and `to_ordinal` (§12). `from == to` replaces nothing and
  inserts the replacement before `to`, so `{from: 0, to: 0}` puts it before
  every message. `to` may be one past the newest message (no raw tail),
  never more. Only `from > to` is malformed (`range_inverted`).
- [pinned] `to` never separates a tool call from its result. The runner
  checks this with the rest of its structural checks (roles, pairing over the
  replacement and the raw tail together, no tool-call id outside the range,
  its own validity rules) against what it rendered, and a view that fails is
  recorded as not applied (`structural`).
- [pinned] The whole working range, never a delta. Each CompactionMessage
  describes the entire working range as it stands and carries its whole
  replacement. It never refers to earlier content by id, and the runner
  applies the newest one alone.
- [pinned] `replacement` is the provider's finished output, in the runner's
  message schema. The runner does not run step transforms over it.
- [pinned] `version` is an unsigned 64-bit integer (`u64`) the provider
  raises for every CompactionMessage of the session.
- [pinned] `compaction_id` is an opaque string. The runner never
  interprets it; it echoes it in the status and shows it in the model view.
  [open: Q4] the provider mints it, as its own name for the content.
- [pinned] A CompactionMessage carries no cost label. The runner computes
  where the request first changes and maps it onto the stability segments.
- [pinned] How the runner stores replacement content (by digest, say) is
  the runner's choice. Appends in sessions with mid-session appends are the
  runner's to carry around the replacement; the provider never sees or
  re-emits them.
- [pinned] A provider keeps the stable part of its output stable: rewriting
  the top-ranked message every step defeats every cache breakpoint above the
  tail. The suite measures this (§17).

## 9. The request fence and the version rule

- [pinned] Before each call the runner durably records the id of the
  request it issues, its newest, and the request carries it.
- [pinned] An answer applies only if it names the newest issued request
  (`fence::dispose`). Any other answer, including a late one, is recorded
  and never applied: not at the next step boundary, not when the fresh
  request's answer is slow, and not when its version is higher. This holds
  for every answer: a late `refuse` never ends a run that moved on, a late
  `wait` never holds a step, a late `noop` never moves the cursor. The
  provider's answer to the fresh request carries its content, at a higher
  version.
- [pinned] A CompactionMessage that passes the fence applies only if it is
  well formed (§8) and its version is higher than the last applied. A
  delayed answer from before a provider restart, even one with a higher
  version, never applies over content the provider produced after it.
- [pinned] Checks run in a fixed order: the fence, then the message on its
  own, then the version, then the range against the request's newest
  message. The first that fails names the reason.
- [pinned] Every applied CompactionMessage, `wait` entry and exit, and call
  timeout is durable before the request it affects is sent.
- [pinned] A provider that keeps its counter in memory reads `last_applied`
  from the next status and continues above it, so it is never refused
  forever as not higher (`next_version`). A provider that keeps its counter
  durably still never goes below `last_applied`.
- [pinned] A provider never sends two different contents under one
  version. A version allocated for an answer that was never sent is skipped,
  not reused (kill point `AnswerRecorded`).

| Reason | When |
|---|---|
| `superseded_request` | the answer names a request other than the newest issued, whether an earlier one or one never issued |
| `range_inverted` | the CompactionMessage's `from` is above its `to` |
| `stale_version` | its version is not higher than the last applied |
| `range_beyond_newest` | `to` is past one beyond the request's newest message |
| `structural` | the runner's own checks failed (§8) |

## 10. WAIT and `compaction.ready`

- [pinned] `wait {reason, bound_ms}` holds the step. The runner sends
  nothing to the model, and writes the entry into the wait before it
  begins. `reason` is user-facing: the runner shows "waiting on compaction:
  <reason>".
- [pinned] The runner calls again, with the same op and a fresh status
  (a new `request_id`), on whichever comes first: `compaction.ready` from
  the provider, or the bound running out. The provider answers normally and
  may answer `wait` again; an engine-wide cap limits the total wait per
  step, and the prompt cache's lifetime caps that.
- [pinned] At the cap the runner decides from its own numbers: it sends if
  the request is estimated to fit with the last applied CompactionMessage,
  and otherwise ends the step `error` with `provider_code`
  `compaction_wait_exceeded`.
- [pinned] `compaction.ready {session, request_id}` (`CompactionReady`):
  - `session` is echoed verbatim from the status that carried the `wait`,
    and is opaque to the provider;
  - `request_id` is that status's request id. The runner ignores a ready
    for anything but the newest request it issued (`is_current`);
  - the runner accepts it only from the session's frozen compaction
    provider: the route's caller stamp must equal
    `plan.compaction_item.provider` (§3, provisional). Any other caller,
    and any caller for a session without a compaction item, is refused
    `not_session_compaction_provider` (`CompactionReady::check`);
  - a ready for an older request, or before any request was issued, is
    ignored and is not an error;
  - it is a hint: the runner's bound is the backstop.
- [open: Q1] The reply to an accepted or ignored ready is an empty object,
  so a provider learns nothing from it.
- [pinned] A provider sends `compaction.ready` only once the result its next
  answer will carry is durable in its own store, so a crash between the two
  loses nothing (kill point `WaitWorkDurable`). A provider that crashed with
  its work not durable answers the next status normally and builds nothing
  from the lost work (kill point `WaitAnswered`).
- [pinned] A wait cut by a runner stop or crash seals the run `interrupted`;
  the run continues only through a later send, which calls the provider
  again with a fresh status.

## 11. REFUSE

- [pinned] `refuse {code, reason, retryable}` ends the run `error`. The
  runner writes `code` as `provider_code`, with the provider and `reason`.
  Nothing is added to the model's history, and the session stays usable:
  the next send calls the provider again.
- [pinned] `reason` is user-facing text. `retryable` tells the session's
  owner whether retrying without the user acting makes sense (the provider
  was only busy) or the user must act (switch model, fix configuration).
- [pinned] On a tool step, the tool results already written stay.
- [open: Q14] The codes this role names (`errors::refuse_codes`), each with
  the `retryable` value a provider must send with it:

| Code | When | `retryable` |
|---|---|---|
| `window_too_small` | the model's window cannot hold the smallest view the provider can build | `false` |
| `provider_busy` | the provider is busy with the session's history past the wait cap | `true` |
| `misconfigured` | the configuration for this preset and params cannot be used | `false` |
| `history_unreadable` | a gap after the cursor, and no transcript the provider can reach | `true` |

  A provider may use codes of its own, with its own `retryable`.

## 12. The model view

The model view is `llm-runner/v1`'s; this section only says where this
role's ids show up in it.

- [pinned] A runner that declares `model_view` answers `session.read`
  with `view: "model"` as `ModelPage {messages: [ModelEntry {source,
  message}], lineage_id?, next_from_ordinal?, head?, compaction_id?,
  version?}`. Each replacement message has `source: {kind: "replacement",
  compaction_id, version, from_ordinal, to_ordinal}` (`EntrySource`),
  and each raw message `source: {kind: "message", ordinal, mid}`. An
  unknown `source.kind` fails to decode; it is not skipped. Tail and range
  reads only; `after_mid` is refused `invalid_params {field: "view"}`.
  This crate imports `EntrySource` from the runner's crate and defines no
  source kind of its own.
- [pinned] The page's `compaction_id` and `version` are those of the last
  applied CompactionMessage, both absent before any applied.
- [pinned] For a CompactionMessage with range `{from, to}`, each
  replacement message's source is `{kind: "replacement", compaction_id,
  version, from_ordinal: from, to_ordinal: to}`, the same half-open range
  (`CompactionMessage::model_view_source`, which builds the runner crate's
  `EntrySource`).
- [pinned] An empty range is an insertion: `from_ordinal == to_ordinal`,
  placed before that ordinal and covering no transcript message. Setup's
  usual head is `[0, 0)`. There is no separate source kind for inserted
  messages.

## 13. Purity and identity

- [pinned] What the runner freezes from Setup, `stability` and `call_when`,
  derives only from the preset, the params, the provider's configuration
  and the composition. The same inputs give the same bytes.
- [pinned] No request in this role carries scope, agent or session identity
  beyond the opaque `session` handle. A provider uses that handle only as a
  key for its own per-session state, never to choose a variant. A subagent
  session gets different behaviour through its preset or params, never
  through who is asking.
- [pinned] The replacement is history content, not fetched text: it may
  depend on the session's own messages and on the provider's state for the
  session (its memories and summaries). Project-level content such as
  project docs goes into user-role messages of the replacement, never into
  system text.

## 14. Error codes

Codes ride as the `code` of an `ERROR` frame's body `{code, message,
detail?}`. They are open strings: a party that meets one it does not know
treats it as a terminal refusal of that one request.

### 14.1 Provider refusals (`errors`)

| Code | When | Detail | Retry |
|---|---|---|---|
| `invalid_params` | a request field is malformed, or names a preset or params value the provider does not know | `field` | no |
| `transient` | a condition that may clear by itself | — | **yes**, within the call's budget |

- [pinned] The malformed-request code is `invalid_params`, as runners
  answer. `tool-provider/v1` spells it `invalid_request`; a consumer of
  both maps each by its own role.
- [pinned] Any refusal of a step call is a failed call (§7).

### 14.2 Codes the runner answers or writes (`errors::runner_codes`)

| Code | When |
|---|---|
| `invalid_params {field: "plan.compaction_item"}` | admission, on a runner without the `compaction` group |
| `not_session_compaction_provider` | a `compaction.ready` whose route caller is not `plan.compaction_item.provider`, or for a session without a compaction item |
| `invalid_params {field}` | a malformed `compaction.ready` |
| `compaction_unavailable` (a `provider_code`) | Setup failed or timed out with no answer (§4) |
| `compaction_wait_exceeded` (a `provider_code`) | a wait reached the cap and the request could not be shown to fit |

## 15. Decoding

- [pinned] **Lenient** (unknown fields ignored): `role.describe`, the Setup
  request, the status and its messages, `compaction.ready`, and the fields
  of every answer.
- [pinned] **Strict on values**: `answer` in every answer, because a runner
  cannot act on an answer it does not understand. Values a runner may grow
  (`step_kind`, `prefix_rebuilding.reason`, `finish_reason`,
  `last_not_applied.reason`, `provider_code`) are decoded open.
- [pinned] Versions and ordinals are `u64`, as `llm-runner/v1` types them.
- [pinned] Public types with optional members are non-exhaustive where they
  are likely to grow, built with a constructor and `with_*` setters.

## 16. Crash and durability guarantees

What a provider owes across its own crash, stated as properties the suite
checks (§17) by killing the provider at each point below and restarting it
on the same state root (`cortexkit-role-harness`). The runner's side
(`CompactionApplied` and the rest) is `llm-runner/v1` §14.

1. [pinned] **Versions only rise.** No version is used for two different
   contents, and every CompactionMessage after a restart carries a version
   above both the last applied and any version the provider allocated
   before the kill.
2. [pinned] **Messages are taken in once.** Messages a status re-sends after
   a kill are recognised by ordinal and lineage, never counted twice.
3. [pinned] **Setup is repeatable.** A Setup repeated for a session the
   provider recorded gets a well-formed answer.
4. [pinned] **Wait work is not lost and not invented.** Work that was
   durable before the kill reaches a later answer; work that was not never
   does.
5. [pinned] **No identity leaks into frozen bytes.** Setup's `stability`
   and `call_when` for the same inputs are byte-identical before and after
   a kill.

Kill points (`points.rs`). [pinned] The list below.

| Point | Durable state after the kill | Property |
|---|---|---|
| `SetupRecorded` | the provider's per-session Setup state; the answer not sent | 3, 5 |
| `MessagesIngested` | a status's messages in the provider's store; no answer sent | 2 |
| `AnswerRecorded` | the version allocated for an answer; the answer not sent | 1 |
| `WaitAnswered` | `wait` answered; its work not durable | 4 |
| `WaitWorkDurable` | the work behind a `wait`; `compaction.ready` not sent | 4 |

A provider whose state lives in a store where truncation is not meaningful
(SQLite, say) reaches these points with a fault hook followed by a real
process kill, and declares that. A suite run fails unless at least one kill
was a real process kill.

## 17. Conformance suite (planned)

A separate `cortexkit-role-compaction-provider-conformance` crate, run by
each provider in its own CI against its real module over real routes, never
a double. It drives the provider with a scripted runner that sends statuses,
serves `session.read` from a scripted transcript, and receives
`compaction.ready`. It will check:

| Case | Checks |
|---|---|
| `role_describe_shape`, `role_describe_cacheable`, `extra_op_still_admitted` | §2 |
| `setup_once_initial_well_formed`, `setup_repeat_after_failure` | §4 |
| `setup_declarations_pure` | §13: two Setups with the same inputs and different `session` give the same `stability` and `call_when` |
| `answer_names_request`, `answer_known_value` | §7 |
| `version_rises`, `version_resumes_above_last_applied` | §9 |
| `range_within_newest`, `range_never_splits_tool_pair` | §8 |
| `cursor_repeat_ingested_once`, `cursor_gap_read_or_refused`, `lineage_change_resets` | §6 |
| `wait_then_ready_names_request`, `ready_only_after_durable_work` | §10 |
| `stable_message_stays_stable` | §8: across steps that add only to the tail, the top-ranked replacement message's bytes do not change |
| `crash_at_<point>` for each point in §16 | §16 |

Runner-side rules no live provider can exercise (the fence, an unknown
answer, the model-view source of a range, empty or not) are tested against
the vectors in this crate.

Verdict, as for `tool-provider/v1`: a case whose requirements the subject
does not declare is skipped, never passed; a run fails if any case fails or
if no kill ended a real process.

## Not in v1

- A neutral, role-defined message schema. The status's messages and the
  replacement are in the runner's schema (`llm-runner/v1` §4.2); a provider
  serving more than one runner decodes each runner's schema.
- Deltas: a CompactionMessage that refers to earlier content by id.
- More than one compaction provider per session, or a chain of them.
- A cost label on a CompactionMessage.
- A rule language for `call_when`.
- The runner telling the provider which messages it stripped after a
  prefix-changing event.

## Open questions

Each open question lists the options seen and the draft's choice. A
question keeps its number when it is settled and moves to "Settled
questions" below, with the decision.

- **Q1. The envelope and the ready reply.** (a) **draft:** `{method,
  params}`, as `llm-runner/v1` settled for its own ops, and an empty object
  as the reply to `compaction.ready`; (b) `{name, arguments}`, as
  `tool-provider/v1` uses.
- **Q2. Names.** **draft:** ops `compaction.setup` and `compaction.step`;
  answers `noop`, `compaction_message`, `wait`, `refuse` (snake_case, as the
  hook names are), and `ready` for Setup.
- **Q4. Who mints `compaction_id`.** Its type is settled: an opaque
  string. (a) **draft:** the provider mints it, as its own content name;
  (b) the runner, at recording.
- **Q6. A cap on the status's messages.** (a) **draft:** a byte cap with
  `more`, the cursor advancing only to the last message sent; (b) every
  message since the cursor, uncapped. A lineage change on a long session
  makes (b) large.
- **Q7. `last_not_applied`.** (a) **draft:** in the status; (b) left out,
  the provider inferring it from `last_applied`.
- **Q8. `runner_groups`.** (a) **draft:** in `role.describe`, checked by the
  starter; (b) the starter's own configuration per provider.
- **Q9. Unknown `call_when` kinds and model patterns.** **draft:** a runner
  meeting an unknown kind calls on every step. Pattern matching is unset:
  exact id or a trailing `*`.
- **Q13. The stability shape.** **draft:** an array of `{index, rank}`.
- **Q14. Role-named `refuse` codes.** **draft:** the four in §11.

## Settled questions

- **Q3. Range ends.** Settled: half-open ordinals, `from` included and `to`
  excluded (§8). Ordinals are never reused within a lineage, they can name
  an empty range and a range running to the end, and the model view speaks
  the same half-open ordinals (§12).
- **Q5. `request_id`.** Settled with `llm-runner/v1`: an opaque string,
  compared only for equality. `version` is settled as `u64`.
- **Q10. Empty ranges in the model view.** Settled in the runner role: the
  model view's replacement source is half-open, `{kind: "replacement",
  compaction_id, version, from_ordinal, to_ordinal}`, and an empty range is
  an insertion before `from_ordinal`; Setup's head is `[0, 0)` (§12). This
  role defines no source kind of its own: the earlier draft's `inserted`
  kind is dropped, and `EntrySource` is imported from the runner crate.
- **Q11. Late answers.** Settled by the room: the fence is strict (§9). An
  answer applies only if it names the newest issued request; any other
  answer, including a late one, is recorded and never applied. The
  provider's answer to the fresh request carries the content at a higher
  version.
- **Q12. The `provider_code` when Setup times out or fails without an
  answer.** Settled in the runner role: `compaction_unavailable`, one of
  `llm-runner/v1`'s provider codes, re-exported here (§4, §14.2).
- **Q15. Pairing.** Settled: compaction is a runner capability group, all
  or nothing; a runner without it refuses a compaction item at admission
  with `invalid_params {field: "plan.compaction_item"}` (§3).
- **Q16. `compaction.ready`.** Settled: `{session, request_id}`, accepted
  only from the provider at `plan.compaction_item.provider` by route stamp,
  otherwise `not_session_compaction_provider`; ignored for any request but
  the newest; a hint (§10). The type is `llm-runner/v1`'s, re-exported.
- **Q17. The model view.** Settled in the runner role: `ModelPage
  {messages: [ModelEntry {source, message}], lineage_id?,
  next_from_ordinal?, head?, compaction_id?, version?}`; messages `{kind:
  "message", ordinal, mid}`, replacements `{kind: "replacement",
  compaction_id, version, from_ordinal, to_ordinal}`; an unknown kind
  fails to decode; tail and range reads only; `after_mid` refused with
  `invalid_params {field: "view"}` (§12).

## Appendix A. Cross-check with `llm-runner/v1`

Checked against `llm-runner/v1` at commons commit 5262544. Paths are in this
repository. Every field this contract shares with that role's §8, §11.1
and §12.2 agrees: the `compaction` group and its admission refusal
(`crates/cortexkit-role-llm-runner/CONTRACT.md:452-455`,
`crates/cortexkit-role-llm-runner/src/lib.rs:114`), `compaction.ready
{session, request_id}` and its check
(`crates/cortexkit-role-llm-runner/CONTRACT.md:524-538`,
`crates/cortexkit-role-llm-runner/src/compaction.rs:33-41`), the Setup,
cursor and fence rules (`crates/cortexkit-role-llm-runner/CONTRACT.md:506-521`),
the model view's `ModelPage` and its half-open `EntrySource`
(`crates/cortexkit-role-llm-runner/CONTRACT.md:281-330`,
`crates/cortexkit-role-llm-runner/src/read.rs:261-350`), and the
`provider_code`s `compaction_unavailable` and `compaction_wait_exceeded`
(`crates/cortexkit-role-llm-runner/CONTRACT.md:617-628`,
`crates/cortexkit-role-llm-runner/src/errors.rs:175-191`). This crate takes
those names and types from the runner crate rather than restating them.

Resolved since the first draft (commons `ff1fe00`):

- **Empty ranges in the model view.** The runner's replacement source is
  now half-open, `[from_ordinal, to_ordinal)`, and an empty range is an
  insertion (`crates/cortexkit-role-llm-runner/CONTRACT.md:295-300`,
  `crates/cortexkit-role-llm-runner/src/read.rs:312-321`). Setup's head maps
  to `[0, 0)` (Q10).
- **A `provider_code` for a Setup that fails without an answer.** The
  runner names `compaction_unavailable`
  (`crates/cortexkit-role-llm-runner/src/errors.rs:176-177`), which this
  crate re-exports (Q12).

What remains:

1. **The scope of `compaction_unavailable`.** The runner's table gives it
   for "Setup or a compaction call" that failed or timed out with no answer
   (`crates/cortexkit-role-llm-runner/CONTRACT.md:623`). In this role only
   Setup ends a run that way: a step call that fails or times out is
   recorded and the step goes on with the last applied CompactionMessage,
   which always exists (§7). Wording only, unless the runner role means a
   step call to end a run as well.
2. **The plan's provider field is provisional on both sides.**
   `crates/cortexkit-role-llm-runner/CONTRACT.md:456-459` marks
   `plan.compaction_item.provider` provisional; this document §3 and §10
   follow it and carry the same marker.
3. **The fence's wording.** The runner role states the fence for a
   CompactionMessage (`crates/cortexkit-role-llm-runner/CONTRACT.md:518-521`)
   and for `compaction.ready` (`:530-533`). This document states it for
   every answer: a late `noop`, `wait` or `refuse` is recorded and never
   acted on either (§9). No disagreement; the runner role is narrower in
   what it spells out.
4. **Not a divergence, a concern.** Versions and ordinals are `u64` on both
   sides. JSON readers that use doubles (JavaScript) lose exactness above
   2^53 − 1, so a TypeScript provider must read them as big integers. This
   draft does not bound them, to match the runner role.

Against the design itself (paths in the magic-context repository,
`.cortexkit/alfonso/plans/`):

5. **Late answers.** r7.3 §5.3 "Timeout"
   (`ck-extensibility-design-r7.3.md:721`) lets an answer that arrives after
   its timeout, with a higher version, apply at the next step boundary, and
   the owner corrections add that a late answer applies when the fresh
   request's answer does not arrive in time
   (`ck-extensibility-r7.3-errata.md:18`). The room's ruling, this document
   §9 and `llm-runner/v1` (`crates/cortexkit-role-llm-runner/CONTRACT.md:518-521`)
   apply only an answer to the newest issued request. The design text is
   the one to change.
6. **Range ends.** r7.3 §5.4 (`ck-extensibility-design-r7.3.md:725`) names
   both ends by message id. This document uses half-open ordinals (Q3), as
   the runner's model view does.
## Appendix B. Gaps in Magic Context today

Where `ck-mc` (`crates/mc-module` in the magic-context repository) differs
from this document. None of this is a change request beyond the work the
extensibility plan already schedules for Magic Context.

1. **No role claim.** The manifest provides only a `ToolProvider` route
   role and declares no capabilities
   (`crates/mc-module/src/lib.rs:18288,18317-18323`), so nothing lists
   `compaction-provider/v1`.
2. **No role ops.** The request dispatch serves `transform`,
   `historian.*`, `session.*`, `state_sync` and others; there is no
   `role.describe`, `compaction.setup`, `compaction.step` or
   `compaction.ready` client, and an unknown method is refused
   (`crates/mc-module/src/lib.rs:13755-13788,13824`).
3. **The whole array each pass, not a status after a cursor.** A
   `transform` request carries `session_id`, `render_config` and every
   message with its `mid` and `ordinal`
   (`crates/mc-module/src/transform.rs:665-686,794`,
   `crates/mc-module/src/ck_wire.rs:26-31`). The optional `tail_delta`
   (`crates/mc-module/src/transform.rs:800`) is rebuilt against a prefix
   ck-mc acknowledged, not against a runner-held cursor scoped to a
   `lineage_id`.
4. **A whole output array, not a range replacement.** The reply's
   `ck_messages` is the full array, with m0 and m1 built into every pass
   ahead of the tail, and an optional native suffix delta
   (`crates/mc-module/src/transform.rs:1584-1591,1663-1678,3304-3320`).
   There is no `compaction_id`, no version, no `request_id` and so no
   fence.
5. **No Setup.** The head is composed per pass
   (`crates/mc-module/src/transform.rs:3304-3320`), not recorded once by the
   runner before the first model call; stability ranks and `call_when` are
   not declared anywhere.
6. **No `wait` and no `compaction.ready`.** Historian completion is
   reported to ck-mc itself through `historian.complete`
   (`crates/mc-module/src/lib.rs:8039-8069`); the decision to fire is
   internal (`crates/mc-module/src/boundary.rs:257-273`). Nothing answers a
   runner with `wait` or tells it the work is done.
7. **Session identity chooses some bytes.** Historian seed selection hashes
   the session id (`crates/mc-module/src/historian_prompt.rs:252-256`), and
   the subagent path is a request flag, `is_subagent`
   (`crates/mc-module/src/transform.rs:684-686`), rather than a preset or
   params value. The first is history content and allowed by §13; the
   second must move into the compaction item's preset or params.
8. **Durability across two files, with no kill-point hooks.** Writes that
   touch both `store.db` and the host's `context.db` commit in steps with a
   recovery pass (`crates/mc-store/src/context_writes.rs:1-17`). Fault
   injection exists for some failures, but there is no hook that stops the
   module at the points in §16.
