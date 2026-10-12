# `compaction-provider/v1` — role contract (draft)

Stability: **alpha, draft**, unpublished. This document is the role
definition; the Rust types in this crate are its wire
shapes, `test-vectors/compaction-provider-v1/` at the repository root holds
its vectors, and a separate `-conformance` crate will hold its suite (§17).

`llm-runner/v1` states what the runner owes on the same interface; this
document states the provider's side.

Every item is marked **[pinned]**: a settled, binding requirement, even before
the crate's first release. A provider must
do it, and a runner may rely on it.

Names and types this role shares with `llm-runner/v1` (the `compaction`
group, `compaction.ready`, the model view's `source`, the runner's codes)
are taken from that crate, which this crate depends on, so the two cannot
drift apart.

A compaction provider is any module that decides the shape of the history a
runner sends to the model.

Optional lineage, coverage and history-gap fields support runners that
cannot serve `session.read`. When they are absent, requests, answers and
refusals keep their existing bytes and meaning.

## Terms

- **Plan composer**: the component that writes a session's plan and freezes
  it. It checks each provider's runner groups against the runner, places
  the providers in order, and fills the plan's user-tier fields.
- **Runner group**: a capability group a runner advertises in its
  `llm-runner/v1` `role.describe`, such as `compaction` or
  `transcript_reads`. A provider lists the groups it needs in its own
  `role.describe.runner_groups`.
- **Reduction owner**: the session's compaction provider, the only party
  that may remove or rewrite history already written. Every other step
  transform may only add to it.
- **Signed thinking**: model reasoning blocks that carry a model provider's
  signature. Any change to them, even one byte, invalidates the signature,
  so nothing in these roles ever edits them.

## 1. Role identity and addressing

- [pinned] A provider lists `compaction-provider/v1` in its manifest's
  `capabilities.provides` (`PROVIDES`). A module may serve several majors.
- [pinned] Required ops (`REQUIRED_OPS`): `role.describe`, the Setup op
  `compaction.setup` and the per-step op `compaction.step`.
- [pinned] A runner refuses a module whose `role.describe` lacks a required
  op, by name, before routing anything to it.
- [pinned] Compaction routes are module-level and unscoped. Requests name
  the opaque session handle; it is not an authorization credential.
- [pinned] Every compaction request, Setup (§4) and step (§6),
  carries a required string `harness`, next to `session`: the harness
  named in the session's key, which identifies the caller (for example
  `broca`). This differs from the route's bind harness, the harness a
  connection declares when it opens the route; a runner always declares
  `runner`. A request without `harness` does not decode; it is
  never defaulted.
- [pinned] A provider keys a runner conversation, and every piece of
  per-session state it keeps for one, on `(project_root, session,
  harness)`: the project root it serves the request for, the session
  handle and the harness. Two requests that differ in any of the three
  belong to different conversations.
- [pinned] Every request in this role, in either direction, is `{method,
  params}` (`OpRequest`): `method` is the op's name and `params` its
  request. `role.describe` takes empty params (`DescribeRequest`).
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
- [pinned] The provider declares what it needs from the runner in
  `role.describe.runner_groups`: the runner groups it needs besides
  `compaction`, such as `transcript_reads` for a provider
  with no reader of its own. A list of strings, omitted when empty.
- [pinned] The plan composer checks `runner_groups` against the runner
  groups the runner advertises when it composes the session. An unmet group fails the launch, and the failure names each
  unmet group (`unmet_runner_groups`). Runner requirements are carried
  nowhere else.

## 3. Pairing with a runner

The admitted host runner lane is the scoped pairing exception in §18.1.

Admission is the runner's check of a proposed session plan before it accepts
the session and freezes that plan for use.

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
- [pinned] The runner finds the session's compaction provider at
  `plan.compaction_item.provider` (`llm-runner/v1`'s
  `plan_compaction_provider`). That is what the
  `not_session_compaction_provider` check (§10) compares the daemon-verified
  module that opened the route (its caller stamp)
  against. The item is `{provider, preset, params}`; `provider` is a module
  id, `preset` a string, and `params` an opaque object.

## 4. `compaction.setup`

- [pinned] Called once per session, before its first model call. Its answer
  is recorded before the first model call; once recorded, Setup never runs
  again for the session and nothing replaces its initial view except a
  later CompactionMessage.
- [pinned] If Setup fails or times out with no answer, no model call is
  made and the run ends `error` with `provider_code`
  `compaction_unavailable`, `llm-runner/v1`'s code, which this crate
  re-exports (`errors::runner_codes::COMPACTION_UNAVAILABLE`). That code is
  used for Setup only, and is not retryable (§14.3). A Setup answered
  `refuse` ends it with the provider's own `code` instead (§11).
  Either way the next send calls Setup again, because no initial view was
  recorded. So a provider answers Setup for a
  session it has seen before, and its answer is well formed whatever it
  answered last time (kill point `SetupRecorded`).
- [pinned] The request (`SetupRequest`) is `{session, harness, request_id,
  preset?, params, composition, model, variant?, context_window?, output_limit?,
  newest?, lineage_id?, now}`. `model` is a string; `variant`,
  `context_window` and `output_limit` are sibling fields, not a nested
  model object. `params` defaults to `{}`; an absent `preset` selects the
  provider's default. `composition` is an opaque object passed verbatim.
  `newest` is `{ordinal, mid}` when a message is already written;
  `lineage_id` is absent when no lineage exists. `now` is milliseconds
  since the Unix epoch.
- [pinned] The answer (`SetupAnswer`) is `{answer: "ready", request_id,
  initial, stability?, call_when?}` or `{answer: "refuse", request_id, code,
  reason, provider_code?}` (§11).
  - `initial` is a CompactionMessage (§8). Usually its range is empty at the
    start of the lineage, `{from: 0, to: 0}`: the provider's head is
    inserted before every message, and the model view shows it as the
    insertion `[0, 0)` (§12). A provider with no head returns an empty
    replacement.
  - `stability` ranks the provider's own messages: `index` is a position in
    every CompactionMessage's `replacement`, and a higher `rank` changes
    less often. For example, a provider whose head is a stable summary
    followed by a more volatile one declares `[{index: 0, rank: 2},
    {index: 1, rank: 1}]` for a main session, and nothing for a subagent
    session that has no head. The
    runner uses these as stability segments for cache breakpoints and change
    policies. Stability is an array of `{index, rank}`, both unsigned 32-bit
    integers; it defaults to an empty array and is omitted when empty.
  - `call_when` says when the runner calls the provider: a share of the
    context window, with per-model overrides (§5). Absent, the runner
    calls on every step.
  - A `refuse` ends the run `error` with the refusal's role `code`, and the
    next send runs Setup again (§11).

## 5. When the provider is called

- [pinned] Timing is data, frozen with the session, from Setup's
  `call_when`. There is no rule language.
- [pinned] The vocabulary is a default share of the context window plus
  optional per-model overrides, measured against the step's reported usage:
  `{default: 0.8, models: {"<model id or pattern>": 0.7}}` (`CallWhen`). A
  share is above 0 and at most 1. New condition kinds are additive.
- [pinned] A runner that meets a condition kind it does not know calls the
  provider on every step, exactly as with no `call_when`
  (`setup::calls_every_step`). It never skips a call because it could not
  read a condition.
- [pinned] A key in `call_when.models` is an exact model id, or a pattern
  ending in one `*` that matches every model id starting with the text
  before the `*`. A `*` anywhere else is literal. The share for a model is
  chosen in a fixed order (`CallWhen::share_for`): the key equal to the
  model id; else the matching pattern with the longest prefix; else
  `default`. A bare `*` matches every model and loses to any longer
  pattern.
- [pinned] With no `call_when`, the runner calls on every step. Whatever the
  conditions say, it always calls on a prefix rebuild (the runner re-sends
  the conversation from the start because its cached prefix was lost or
  invalidated, for example after a model change) and after an execution
  error.
- [pinned] On a step without a call, the runner keeps serving the last
  applied CompactionMessage's view: the same replacement and range, plus
  every newer message raw.

## 6. `compaction.step`: the status

The admitted host lane adds the optional fields in §18.2 and departs from the
suffix, final-value and size rules only as specified in §18.4.

The status (`StepStatus`) carries:

- [pinned] `session`, the runner's opaque name for the session. The
  provider keys its per-session state on it and echoes it in
  `compaction.ready`; it never chooses bytes from it.
- [pinned] `harness`, the caller's harness from the session's key (§1).
- [pinned] `request_id`, the id of this request (§9): an opaque string the
  provider compares only for equality, the type `llm-runner/v1` gives it.
- [pinned] `lineage_id`. Ordinals, the cursor and every range are in this
  lineage.
- [pinned] Optional `descends_from: {lineage_id, through_ordinal}`
  (`DescendsFrom`) declares that this lineage continues another lineage in
  the same conversation (§1), up to and including the `u64`
  `through_ordinal`. The inherited prefix keeps its message ids and
  ordinals; no later history is inherited. The provider must hold that
  prefix through the named ordinal or answer `refuse` with
  `history_unreadable`, optionally naming its first missing ordinal in
  `detail.history_gap_from` (§11). It never guesses ancestry or history.
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
- [pinned] `last_not_applied {compaction_id, version, reason}` names the
  newest CompactionMessage the runner recorded and did not apply, when it
  is newer than the last applied, with the reason: a §9 reason, or
  `structural` for the runner's own checks (`STRUCTURAL`). It is absent
  otherwise. It lets a provider tell a refused answer from one still in
  flight.
- [pinned] The cursor and the messages after it: `after_ordinal`, the last
  ordinal of this lineage the provider was sent (absent on the first call in
  the lineage), and `messages: [{ordinal, mid, message}]`, every message
  after it, oldest first. `message` is the runner's message schema, with
  final values, as `session.read` pages carry it.
- [pinned] `messages` is capped in bytes, each message counting the length
  of its compact JSON encoding as a status entry (`message_bytes`). The
  default cap is 4 MiB, 4,194,304 bytes (`DEFAULT_MESSAGE_CAP_BYTES`).
  The runner takes messages oldest first while their total stays within
  the cap, and sets `more: true` when it stops before the newest message;
  `more` is omitted when false (`take_capped`).
- [pinned] The first message after the cursor is always sent: a single
  message over the cap is sent alone. The cursor advances only to the last
  message sent (`next_cursor`), never to `newest`, and the next status
  carries the rest.
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
  lineage. Without `descends_from`, the provider's state for the old lineage
  says nothing about the new one. With it, only the declared prefix is
  inherited, and the cursor is still scoped to the new lineage.
- [pinned] Decoded leniently: a provider ignores fields it does not know, so
  a newer runner degrades rather than fails against an older provider.

## 7. The answers

`StepAnswer`, tagged by `answer`, each naming the `request_id` it answers:

| Answer | Shape | The runner |
|---|---|---|
| `noop` | `{request_id}` | records it; the cursor advances; the request goes out unchanged |
| `compaction_message` | `{request_id, compaction, coverage?}` | applies the CompactionMessage if §9 allows |
| `wait` | `{request_id, reason, bound_ms}` | holds the step (§10) |
| `refuse` | `{request_id, code, reason, provider_code?, detail?}` | ends the run (§11) |

- [pinned] The four answers, their meaning and their spellings: the step
  answers `noop`, `compaction_message`, `wait` and `refuse`, and Setup's
  `ready` and `refuse`.
- [pinned] Strict on `answer`: an unknown or misspelled answer does not
  decode. The runner treats it as a failed call and keeps the last applied
  CompactionMessage; it never reads it as `noop`.
- [pinned] A `compaction_message` answer may carry
  `coverage: {end_mid, ordinal}` (`Coverage`), beside `compaction`.
  `end_mid` is the runner's string message id and `ordinal` its `u64`
  position in the request's lineage: the newest message the published
  summary covers. A runner that needs a local trim marker may advance it
  to that message only when the answer applies under §9, never for a late,
  superseded or otherwise unapplied answer. A runner that does not need
  coverage ignores it. Coverage does not change the replacement range or
  bypass any application check; absence leaves view handling unchanged.
- [pinned] A call has a budget of about 2 s for `noop` and
  `compaction_message`. On a timeout the runner records it and proceeds with
  the last applied CompactionMessage; one always exists, because Setup's is
  recorded first. An answer to that call arriving after its deadline is
  late and never applies (§9). A provider that needs longer answers `wait`.
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
  never more. Only `from > to` is malformed (`range_inverted`). Ordinals
  are never reused within a lineage.
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
- [pinned] `compaction_id` is an opaque string minted by the provider, as
  its own name for the content. It is stable for one logical compaction
  lineage: CompactionMessages that carry on one compaction keep the same
  id at rising versions, and the provider mints a new id only when it
  starts a new one. The runner never mints, rewrites or interprets it; it
  echoes it in the status and shows it in the model view.
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
  and arrives before that request's call deadline, the instant the runner
  times the call out (`fence::dispose`). Any other answer is recorded and
  never applied, whatever its version: an answer to an earlier request, and
  an answer to the newest request that arrives at or after its deadline,
  even when no newer request has been issued. It does not apply at the next
  step boundary, nor when the fresh request's answer is slow. This holds
  for every answer: a late `refuse` never ends a run that moved on, a late
  `wait` never holds a step, a late `noop` never moves the cursor. The
  provider's answer to the next request carries the content instead, at a
  higher version.
- [pinned] A CompactionMessage that passes the fence applies only if it is
  well formed (§8) and its version is higher than the last applied. A
  delayed answer from before a provider restart, even one with a higher
  version, never applies over content the provider produced after it.
- [pinned] Checks run in a fixed order: the fence, then the deadline, then
  the message on its own, then the version, then the range against the
  request's newest message. The first that fails names the reason.
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
| `late` | the answer names the newest issued request but arrived at or after that request's call deadline |
| `range_inverted` | the CompactionMessage's `from` is above its `to` |
| `stale_version` | its version is not higher than the last applied |
| `range_beyond_newest` | `to` is past one beyond the request's newest message |
| `structural` | the runner's own checks failed (§8) |

## 10. WAIT and `compaction.ready`

The admitted host lane departs from ready-or-bound paging only under §18.3.

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
    provider: the caller stamp, the daemon-verified module that opened the
    route, must equal
    `plan.compaction_item.provider` (§3). Any other caller,
    and any caller for a session without a compaction item, is refused
    `not_session_compaction_provider` (`CompactionReady::check`);
  - a ready for an older request, or before any request was issued, is
    ignored and is not an error;
  - it is a hint: the runner's bound is the backstop.
- [pinned] The runner replies to `compaction.ready` with the empty object
  `{}` (`ReadyReply`), whether it acted on the hint or ignored it. The
  reply reveals no outcome.
- [pinned] A provider sends `compaction.ready` only once the result its next
  answer will carry is durable in its own store, so a crash between the two
  loses nothing (kill point `WaitWorkDurable`). A provider that crashed with
  its work not durable answers the next status normally and builds nothing
  from the lost work (kill point `WaitAnswered`).
- [pinned] A wait cut by a runner stop or crash seals the run `interrupted`;
  the run continues only through a later send, which calls the provider
  again with a fresh status.

## 11. REFUSE

- [pinned] `refuse {code, reason, provider_code?, detail?}` ends the run `error`.
  The runner records the role `code` as the refusal's code, with the
  provider and `reason`, and, when present, the provider's `provider_code`
  as a separate member beside it, never in place of `code`. Nothing is
  added to the model's history, and the session stays usable: the next
  send calls the provider again.
  In llm-runner run errors, this answer's role `code` maps to `provider_code`
  and its optional finer `provider_code` maps to `provider_detail_code`,
  diagnostics only, never replacing the role code or deciding retryability.
- [pinned] `code` is one of the four codes below (`RefuseCode`). Each has a
  fixed retryability, which tells the session's owner whether retrying
  without the user acting makes sense (the provider was only busy) or the
  user must act (switch model, fix configuration). The answer carries no
  retryability of its own: a `retryable` member is ignored on decode and
  never decides retry.
- [pinned] `provider_code` is an optional open string: the provider's own,
  finer reason, for logs and display. It never replaces `code` and never
  decides retry.
- [pinned] A `code` the runner does not know decodes rather than failing
  the answer (`RefuseCode::Unknown`). It still ends the run, is recorded as
  received, and is not retryable.
- [pinned] `reason` is user-facing text.
- [pinned] A step refusal may carry optional `detail` (`RefuseDetail`),
  with optional `history_gap_from` (`u64`): the first ordinal of the
  required history the provider lacks, so the runner can resend from
  there. It is meaningful only with `history_unreadable`, the existing
  retryable history refusal, including a missing `descends_from` prefix.
  On every other code it is ignored, and never changes retryability. A
  provider never fabricates a gap ordinal when it does not know one.
- [pinned] On a tool step, the tool results already written stay.
| Code | When | Retryable |
|---|---|---|
| `window_too_small` | the model's window cannot hold the smallest view the provider can build | no |
| `provider_busy` | the provider is busy with the session's history past the wait cap | yes |
| `misconfigured` | the configuration for this preset and params cannot be used | no |
| `history_unreadable` | required history is missing, including a cursor gap or a declared inherited prefix the provider does not hold | yes |

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
  beyond the opaque `session` handle and the caller's `harness` (§1). A
  provider uses them only as keys for its own per-session state, never to
  choose a variant. A subagent
  session gets different behaviour through its preset or params, never
  through who is asking.
- [pinned] The replacement is history content, not fetched text: it may
  depend on the session's own messages and on the provider's state for the
  session (its memories and summaries). Project-level content such as
  project docs goes into user-role messages of the replacement, never into
  system text.

## 14. Error codes

Codes ride as the `code` of an `ERROR` frame's body `{code, message,
detail?}` (`ErrorBody`). They are open strings: a party that meets one it
does not know treats it as a terminal refusal of that one request, and
never retries it.

`KnownCode` enumerates the codes in §11 and §14 for classification, with
each one's fixed retryability (§14.3); it does not close the wire
vocabulary. `ErrorBody.code` remains a string; a `refuse` answer's `code`
decodes to `RefuseCode`, which keeps an unknown code as received (§11).
`detail` is arbitrary JSON, omitted when absent; `message` is a required
string.

### 14.1 Provider refusals (`errors`)

| Code | When | Detail | Retry |
|---|---|---|---|
| `invalid_params` | a request field is malformed, or names a preset or params value the provider does not know | `field` | no |
| `transient` | a condition that may clear by itself | — | **yes**, within the call's budget |

- [pinned] The malformed-request code is `invalid_params`, as runners
  answer. `tool-provider/v1` spells it `invalid_request`; a consumer of
  both maps each by its own role.
- [pinned] Any refusal of a step call is a failed call: the runner
  records it and sends the request with the last applied
  CompactionMessage, as after a timeout (§7).

### 14.2 Codes the runner answers or writes (`errors::runner_codes`)

| Code | When |
|---|---|
| `invalid_params {field: "plan.compaction_item"}` | admission, on a runner without the `compaction` group |
| `not_session_compaction_provider` | a `compaction.ready` whose route caller is not `plan.compaction_item.provider`, or for a session without a compaction item |
| `invalid_params {field}` | a malformed `compaction.ready` |
| `compaction_unavailable` | a run's `provider_code`, after Setup only: Setup failed or timed out with no answer, so no initial view exists and no model call is made (§4). The runner never writes it after a step call: a step call that fails or times out does not end the run, because the runner sends the request with the last applied CompactionMessage (§7) |
| `compaction_wait_exceeded` | a run's `provider_code`: a wait reached the cap and the request could not be shown to fit |

### 14.3 Retryability and where a code ends a run (`KnownCode`)

[pinned] Every code this role names has a fixed retryability and a fixed
set of calls after which it may end a run (`KnownCode::retryable`,
`KnownCode::ends_run_at`). No party chooses either per message. A code not
in this table is not retryable.

| Code | Retryable | Ends a run after |
|---|---|---|
| `invalid_params` | no | — |
| `transient` | yes | — |
| `window_too_small` | no | Setup, step |
| `provider_busy` | yes | Setup, step |
| `misconfigured` | no | Setup, step |
| `history_unreadable` | yes | Setup, step |
| `not_session_compaction_provider` | no | — |
| `compaction_unavailable` | no | Setup |
| `compaction_wait_exceeded` | yes | step |

## 15. Decoding

Host-lane omission and legacy compatibility are specified in §18.5.

- [pinned] **Lenient** (unknown fields ignored): `role.describe`, the Setup
  request, the status and its messages, `compaction.ready`, and the fields
  of every answer.
- [pinned] **Strict on values**: `answer` in every answer, because a runner
  cannot act on an answer it does not understand. Values a runner may grow
  (`step_kind`, `prefix_rebuilding.reason`, `finish_reason`,
  `last_not_applied.reason`, `previous_provider_code`, a refusal's
  `provider_code`) are decoded open, and so is a refusal's `code`, whose
  unknown values are never retried (§11). `CallWhen` preserves unknown
  condition fields so a runner can detect them (§5).
- [pinned] Versions and ordinals are `u64`, as `llm-runner/v1` types them.
- [pinned] Public types with optional members are non-exhaustive where they
  are likely to grow, built with a constructor and `with_*` setters.
- [pinned] `descends_from`, `coverage`, a step refusal's `detail` and
  `detail.history_gap_from` default to absent and are omitted when absent,
  preserving existing request and answer bytes. A present `descends_from`
  requires string `lineage_id` and `u64` `through_ordinal`; a present
  `coverage` requires string `end_mid` and `u64` `ordinal`. Unknown fields
  in these objects and refusal details remain ignored.

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

## 18. Host runner lane

This lane is an explicit exception for OpenCode 1 (`opencode`), OpenCode 2
(`opencode2`) and the attested Claude Code gateway described in §19, not a
change to ordinary runner admission or observation.

### 18.1 Admission and observation

- [pinned] Admission is keyed only on the frozen plan item's params and the
  route's bind `(project_root, session, harness)` under the route's
  authenticated principal. For direct callers, the bind harness must be
  `opencode` or `opencode2` and the principal a `direct` caller bound to that
  conversation. The other admitted pair is daemon-attested
  `reserved:thalamus` on its own connection with a `claude-code` bind harness
  and the frozen `claude_code` capability bundle (§19). Neither the
  body `harness` field nor body params select this exception or confer
  authority. The conversation key remains the §1 triple; params select
  behaviour, never another conversation.
- [pinned] For the direct OpenCode callers, the frozen plan opts in with `params.serializer_profile:
  "opencode-aisdk"`. Answer observation additionally requires
  `params.observation: "answer"`; both opt-ins and host admission are required
  for §18.2–§18.4. Missing or unrecognised required opt-ins grant no exception.
  The same frozen params are passed on declaration, Setup and hooks.
- [pinned] Only an admitted caller in this section may use an empty runner-group set
  without `transcript_reads` or a `compaction.ready` service, an exception to
  §2–§3, not a global change to `role.describe.runner_groups`. It grants no
  runner-only operation, runner/Broca transcript read or other session.
  A cross-session request refuses `invalid_params`, naming `session` (or
  `project_root` for a project mismatch). Ordinary runners still undergo full
  runner admission, including the all-or-nothing `compaction` group and
  ready service. A runner principal with body `harness: "opencode"` and host
  params does not enter this exception, even if its route claims a host bind.
- [pinned] The direct-caller exception is no stronger than today's local
  direct-caller trust for full-request transform. It requires the daemon caller-identity
  stamp once that stamp lands; body fields are never a substitute for it.
- [pinned] From a runner principal, either host-only plan value,
  `observation: "answer"` or `serializer_profile: "opencode-aisdk"`, on
  `compaction.setup` refuses `invalid_params`, with `detail.field` naming
  `params.observation` or `params.serializer_profile`, respectively.
  `transform.declare` has the same rule (step-transform-provider/v1 §13.1).
  Authentication/admission does not select variants of pure Setup bytes (§13).
- [pinned] Answer observation makes zero `session.read` calls, including
  restart and bootstrap. Ingest is not observation; missing acknowledgement
  never promotes pending output and never falls back to transcript reads.

### 18.2 Durable service and discarded subjects

A host can't answer `session.read`, so the provider learns from the host which
answers it actually served. Two operations follow from that. To **promote** an
answer is to treat a pending hook answer as served, so its tags and cadence
take effect. To **burn** an answer is to discard a pending answer the host never
served, so it never takes effect. Together they are **promote-and-burn**.

- [pinned] `compaction.step` may carry `served_through_ordinal: u64`, an
  optional sibling in the status, not on Setup or an answer. It is the highest
  ordinal durably committed for serving on the call's lineage, after the
  pass's append transaction; hooks carry the previous committed watermark.
  Absence means no ordinal-confirmed promotion, neither zero nor newest.
- [pinned] Per lineage the watermark moves only forward, except for a revert
  clamp to the revert target in the same durable transaction. A lower value
  without that revert refuses `invalid_params {field:
  "served_through_ordinal"}`; it never un-promotes output. Promotion never
  passes the newest ordinal the provider holds for the lineage. Pending
  answers promote only through the watermark in the current lineage or an
  ancestor's inherited prefix; ancestor pending answers beyond the descent
  boundary burn. A hook answer alone is never evidence of service. These
  rules survive retries and provider crashes.
- [pinned] `compaction.step` may carry `unserved_subjects:
  [{subject_mid: string, hook: Hook, subject_part?: string}]`, optional and
  not on Setup or an answer. Lineage comes from the enclosing status.
  Entries require typed identity and the step-transform role's strict `Hook`
  discriminant; unknown entry fields are ignored. Host entries name
  `pre_user`, `post_assistant` or `post_tool`. An absent or empty list names no
  burns; omit the empty list. The entire list counts toward the 3 MiB host
  request cap (§18.4).
- [pinned] Burning is idempotent; a subject never answered is ignored. Burns
  precede promotion and cadence. Burned numbers never enter tags or rendering;
  queued reductions on burned pending tags are discarded. Later status
  ingestion never resurrects discarded output. After a timeout or partial
  failure the host freezes the whole message raw and reports every subject,
  including answered ones, until a later call answers; it clears the resend
  list only then. Conflicts refuse before any promote-and-burn mutation.
- [pinned] These two fields also occur on `transform.hook`. Host answer
  identity is `(lineage, subject_mid, hook, subject_part)`.
  `subject_part`, on that hook and inside burn entries, is opaque, non-empty
  when present, at most 256 UTF-8 bytes, and compared byte for byte without
  normalisation. Empty or over-limit burn-entry parts refuse `invalid_params
  {field: "unserved_subjects[i].subject_part"}`. A tool subject supplies its
  actual part id, never the display-only `tool_call_id`; other host subjects
  omit it for the no-part slot. Distinct parts with identical call ids and
  output remain distinct answers, and burning one does not burn the other.
  There is no top-level `subject_part` on a step or its answer.
- [pinned] The companion hook's `pass_complete: boolean` is omitted unless
  true, on the last hook of a pass. Omitted or false signals no barrier. It is
  a scheduling hint for off-request historian evaluation after state sync
  (including the historian model chain) and the complete pass's ingest; only
  the barrier schedules that evaluation. It is never an acknowledgement or a
  replacement for the durable watermark. A missing barrier after unavailable
  hooks is covered by the next pass's barrier.
  Zero-append use on MC's `state_sync` is MC-internal, not an extension of
  either role. `pass_complete` is not on a compaction step or any answer
  (step-transform-provider/v1 §13.3).

### 18.3 Paging and `wait`

- [pinned] In this admitted answer-observation lane only, an incomplete
  lineage may answer `wait` only when the status has `more: true`. Omitted
  or false `more` is a final page; an incomplete final page answers `refuse`
  with `code: "history_unreadable"` and `detail.history_gap_from` naming the
  first known missing ordinal. A complete final page may run the step.
  Absent `newest` means no written message, not a reason to scan a transcript.
- [pinned] The host re-pages only on `more: true`, immediately with the next
  status and a fresh `request_id`, still subject to §9's rule that only an
  answer naming the newest request is accepted, not after ready or
  `bound_ms`. The provider releases the conversation lock before answering;
  a host-mode wait spawns no scan and calls no `compaction.ready`. A `wait`
  returned on a final page is an unavailable call, never an automatic retry
  loop. This is an exception to §10's ready-or-bound behaviour, which remains
  unchanged for ordinary runners, including their scan and durability rules.

### 18.4 Sparse pre-op status

- [pinned] **For the admitted answer-observation host lane only, this departs
  from §6's suffix rule and final-value content rule.** `after_ordinal` is the
  highest contiguous ordinal whose entries have `ingested = true` in host
  records. Here ingested means every hook of the message answered. Later
  `messages` includes only entries with `ingested = false`, oldest first, as
  opaque pre-op ingest JSON, not final ops-modified values. There is no new
  `ingested` member in the wire message schema.
- [pinned] A missing cursor is the first status, not an assertion of held
  history. `messages: []` is valid when hooks supplied all entries, with
  `after_ordinal == newest.ordinal`. A frozen-raw or unavailable message is
  still sent even when some earlier hooks ingested it; ingest-once makes the
  repeat harmless. The provider detects actual gaps from its held history,
  not from list sparsity; incomplete history follows §18.3, with no reads.
- [pinned] Validate held-history conflicts before promote-and-burn. A mid
  with different ordinal or different raw JSON bytes refuses
  `invalid_params {field: "subject_mid"}` on a hook; another mid at a held
  hook ordinal names `subject_ordinal`. Conflicting step entries refuse
  `invalid_params {field: "messages"}`. Retain raw bytes for this check;
  parsed JSON equality must not hide a conflict.
- [pinned] Host encoded hook and status requests are capped at 3 MiB
  (3,145,728 bytes), counting all fields, including burn lists and duplicate
  hook `blocks`. They are never truncated or sent over cap. The compact
  `HookCall` cap excludes its transport envelope; the host's send check also
  budgets the encoded `{method, params}` request. This host status limit is
  stricter than §6's default 4 MiB message-payload cap and overrides its
  oversized-first-entry rule only here: a too-large host status entry exits
  with `provider_message_too_large`, never an oversized-single-entry loop.

### 18.5 Compatibility

- [pinned] The new fields default to absent and are omitted when absent, not
  encoded as `null`. Unknown fields remain ignored; strict discriminants
  remain strict. Lenient decoding is not semantic host admission. No new op,
  `compaction.ready` endpoint on a host plugin, or answer discriminant is
  added. Setup is recorded once per conversation; descent is not a new Setup.
- [pinned] Without host opt-in, `owned-broca` and ordinary runners retain
  admission, transcript observation, tag/cadence, subject identity,
  declaration/answer bytes and historian behaviour even with valid extra
  fields. They still send every message after their durable provider cursor
  using §6's capped paging and final-value rules, and still wait for ready or
  the bound. Pi and OMP keep their existing paths; Rust mode never requires
  Broca to run. Host size limits do not replace their size policy.
- [pinned] The request fence, version high-water, structural checks, frozen
  budgets and `on_unavailable` are unchanged. `host-runner-lane.json` in the
  existing vector directory pins wire examples and stateful expectations
  separately; older wire decoding alone cannot verify the host protocol.

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

## Differences from `llm-runner/v1` wording

The `llm-runner/v1` contract (`cortexkit-role-llm-runner/CONTRACT.md`)
states the runner's side of this interface. Three passages there are out of
date against this contract and should be changed to match it:

- **The plan field naming the provider.** `llm-runner/v1` marks the
  `compaction_item` provider field provisional and says not to build
  against `plan.compaction_item.provider` yet. The field is settled: the
  plan names the session's compaction provider at
  `plan.compaction_item.provider`, inside `compaction_item: {provider,
  preset, params}`, where `provider` is a module id, `preset` a string and
  `params` an opaque object (§3). The runner crate's
  `plan_compaction_provider` helper already reads it there.
- **When `compaction_unavailable` is written.** `llm-runner/v1` lists it
  for "Setup or a compaction call failed or timed out with no answer". It
  should say Setup only. A step call that fails or times out does not end
  the run: the runner sends the request with the last applied
  CompactionMessage (§7, §14.3).
- **What a refusal records.** `llm-runner/v1` says a provider's refusal
  rides as the run error's `provider_code`. Here a `refuse` carries a role
  `code` and, optionally, the provider's own finer `provider_code` (§11).
  The runner contract should name one member for the role code and a
  separate one for the provider's code, and never let the second replace
  the first.

## Interoperability

Versions and ordinals are unsigned 64-bit integers, not bounded to the
exact-integer range of JSON readers that use doubles. Such readers must
preserve integer precision rather than round ids, cursors or versions.

## 19. Optional native Claude Code publication

### 19.1 Frozen capability and attested connection

- [pinned] `role.describe` and Setup requests gain optional typed `claude_code`
  data, the shared `ClaudeCodeCapabilities` bundle specified in
  step-transform-provider/v1 §14. The same bundle is selected and echoed in
  `transform.declare`, frozen in the plan and echoed unchanged in Setup.
  Existing members keep their meaning; absent optional extensions keep their
  previous wire bytes. Existing vectors and SHA256SUMS are unchanged.
- [pinned] The following lane-only rules activate only when BOTH the caller is
  daemon-attested as `reserved:thalamus` on its own bound connection AND its
  frozen plan declares `claude-code-step/v1`. A plan selecting the bundle on
  another principal refuses `invalid_params` naming `claude_code` at
  admission/declare. The bind harness must be `claude-code`, with resolved
  project/session/agent and `observation=answer`. Body harnesses, params, tags
  and plan fields never attest identity or lend authority across connections.
  An attested but undeclared connection behaves as before. Missing extensions
  off the lane are never a refusal; ordinary runner admission is unchanged.
- [pinned] This pair extends §18 host admission: answer observation, no
  transcript reads even on restart/bootstrap, no ready-service requirement,
  immediate contiguous ingress paging only when `more=true`, and a fresh
  request fence for each page. A final-page history gap refuses
  `history_unreadable`, not a wait or transcript read. Ingress paging is
  independent of optional raw replacement retrieval. Required groups are
  relaxed only for this admitted connection, not globally in discovery.
- [pinned] Bundle data includes profiles/generations, revision/package/wire
  pins, epochs, exact endpoint semantics, partial-cut support, envelope/page/
  single-message caps, raw retrieval/digest, cache revision, canonical reduction
  and guidance, notice dispatch acknowledgement and bootstrap receipt format.
  The identifier is `claude-code-step/v1`, using the daemon `<name>/v<N>`
  grammar. Epochs and generations are fields, never capability name suffixes.
  Unsupported required attachment capabilities fail attachment by name; no
  managed-to-raw fallback. Generation-aware identity, bootstrap validation and
  persistence belong to the provider/gateway, not these serde types.

### 19.2 Exact endpoints and immutable raw replacements

- [pinned] `coverage` gains optional `endpoint: {message_id, part_index}`.
  It names an exact inclusive original native top-level content-block endpoint,
  with zero-based `part_index`; scalar content is one part at index zero without
  changing scalar form. Remove through that complete block and retain later
  blocks in order. It is not a `<mid>#<part>` string, nested tool-result child
  coordinate, opaque hook `subject_part`, range endpoint or inferred summary
  marker. `coverage.end_mid` agrees with `endpoint.message_id`.
- [pinned] `CompactionMessage` gains optional `raw_view: FrozenView`. Freeze
  `view_id`, `version`, `byte_length`, lowercase hexadecimal `sha256`, `profile`,
  `profile_generation`, `provider_generation`, `lineage_id`, `endpoint` and
  `payload` as one immutable object. `payload` is either
  `{kind: "inline", bytes: <UTF-8 JSON string>}` preserving the raw replacement
  bytes without parse/re-encode, or `{kind: "reference", view: ViewRef}`.
  `ViewRef` is `{view_id, version, byte_length, sha256}`; the payload reference
  agrees with the frozen view. Digest and length are over decoded raw UTF-8
  bytes, not the enclosing JSON string's escapes. A later view never changes
  those bytes or metadata. The raw view's version agrees with the compaction
  version, and its endpoint agrees with coverage. Provider admission validates
  the endpoint against the original native part map and signed/tool-run fences.
- [pinned] Raw role replacement bytes stand for the existing `[from,to)` range,
  not automatically only the head removed by a native hook cut. For a hook cut,
  additionally freeze `cut_replacement_ref: ViewRef` for the exact head and
  `retained_tail: [{endpoint: NativeEndpoint, engine_handle}]`, the retained
  native-part identity map. Both are supplied together; an empty retained tail
  is explicit. Without the mapping the gateway declines the hook cut, retains
  the host transcript and does not acknowledge application. It must not infer
  a head from text or assume the first two messages are special slots.
- [pinned] Partial cuts require the frozen `partial_message_cut` capability.
  Unknown parts, digest/view conflicts or invalid signed/tool runs fence
  publication. An unrepresentable endpoint records
  `last_not_applied.reason=structural` for that exact view. Never snap earlier,
  flatten or discard the remainder. A boundary just after signed thinking is
  not automatically a valid complete tool/reasoning run. A separate admissible
  prior managed view is required to continue without applying the cut.

### 19.3 Optional view retrieval and size refusals

- [pinned] `compaction.view.read` is an optional provider-served operation a
  runner MAY call; it is not a required op and MUST NOT be a prerequisite to
  applying an answer. Existing replacements still apply inline, finished and
  without re-hooking, with the existing range/application/request fences.
- [pinned] Its request is `ViewReadRequest {request_id, view_id, version,
  sha256, byte_cursor}` in the usual op envelope. Lookup is scoped to the
  admitted connection, generation and lineage's frozen object. `byte_cursor`
  is a zero-based offset in the raw UTF-8 bytes, initially zero; it is not a
  message ordinal. Pages do not split UTF-8 characters. Each page echoes
  `request_id`, `view_id`, `version`, total `byte_length`, whole-view `sha256`,
  `byte_cursor`, `bytes` and optional `next_byte_cursor`; absence of next cursor
  means complete. An EOF cursor may return an empty final page. Concatenating
  pages must reproduce the exact frozen bytes and whole-view digest.
- [pinned] `ViewReadAnswer` is typed as `answer=page` or `answer=refuse` with
  `request_id` and a `ViewReadRefusal` code: `unknown_version`,
  `digest_mismatch`, `invalid_cursor`, `invalid_view`, `unavailable_lane` or
  `envelope_too_large`. An unknown id/version never silently selects latest;
  a requested or stored digest mismatch refuses. Off-lane view reads refuse
  `unavailable_lane`; absence of view retrieval is never a refusal. These op
  refusals do not alter ordinary compaction refusal retryability.
- [pinned] Every new reply envelope stays strictly below 3 MiB, reusing
  step-transform's `DEFAULT_HOOK_CAP_BYTES` via `MAX_ENVELOPE_BYTES`. Raw page
  size alone is not enough: account for envelope and JSON escaping overhead.
  Declared `caps.view_page_bytes` also bounds raw bytes per page. The same
  bundle declares ingress/hook/single-message caps (§14.2 of step-transform).
  An unrepresentable single message refuses `invalid_params` naming `messages`
  (status) or `message` (hook), classified by the host as
  `provider_message_too_large`, never truncated or raw-forwarded. The ordinary
  paging helper's oversized-first-message allowance is not this lane's policy.

### 19.4 Current-request geometry and attributed fill

- [pinned] `StepStatus` gains optional typed `geometry: {usable_soft,
  usable_hard, absolute_wall, derivation}`, `output_reserve: u64`,
  `mid_turn: bool`, and `fill: {input_tokens, basis, observed_request_id?}`.
  Geometry/fill token values are `u64`; derivation and basis are open strings
  for attribution, never discriminants on which readers branch. Examples are
  `derivation="provider-observed"`, `basis="measured"` and `basis="estimate"`.
  Existing usage, estimate and output-limit members retain their meaning.
- [pinned] On the negotiated attested lane, geometry is required:
  `0 < usable_soft <= usable_hard <= absolute_wall`, and
  `absolute_wall <= context_window` when the model window is known. Missing or
  contradictory geometry refuses `invalid_params` naming `geometry`. Everywhere
  else geometry is optional, and if supplied it is accepted without lane
  validation, never refused. Lane validation is separate from ordinary status
  decoding and message/burn validation.
- [pinned] Ceilings already represent usable input after native allowances and
  the actual planned `output_reserve`; never subtract reserve twice or treat
  catalog `output_limit` as that reserve. Use soft for bands and hard for hard
  fit/refusal evidence; preserve independent protected-token/tool and signed
  arc gates, evaluated against the current status model/variant and actual
  window. `mid_turn` is explicit native continuation evidence, not an alias
  for `step_kind` or permission to finalize a streaming tail.
- [pinned] Fill describes the current request's causally anchored measured input
  or an explicitly labeled estimate. The gateway owns overlapping-response
  sequencing and byte/request matching; the provider validates measured fill
  against admitted request/generation evidence rather than fabricating history
  from an unrelated last response. Missing measurement falls back to the
  labeled estimate, not a guessed model window.

### 19.5 Notice dispatch acknowledgement

- [pinned] Applicable `noop` and `compaction_message` replies gain optional
  `channel2_directive: {directive_id, text, armed_at_ms}`. Next status gains
  optional `channel2_delivered_id` and `channel2_receipt: DispatchReceipt`,
  supplied together when acknowledging. Receipt fields are
  `{provider_generation, lineage_id, request_id, dispatch_id,
  applied_view: {view_id, version}, final_wire_sha256}`. The wire digest is
  lowercase hex SHA-256 over the actual final provider-request bytes.
- [pinned] Match the explicit notice id, never queue position, a part id, ordinal
  watermark or `pass_complete`. Bind receipt to generation, lineage, admitted
  request/dispatch identity, applied view and final-wire digest. Unknown ids
  refuse `invalid_params` naming `channel2_delivered_id`; cross-generation,
  stale or conflicting receipts refuse naming `channel2_receipt`, without
  clearing another notice. Providers validate publication fences using their
  durable state before accepting the receipt.
- [pinned] A repeated acknowledgement carrying the same receipt is accepted as
  a no-op: no refusal and no second effect. A conflicting replay refuses
  `invalid_params` naming `channel2_receipt`. No additional reply member is
  defined. The typed acknowledgement helper returns the original receipt;
  integrations persist its pending state and receipts transactionally before
  replying, so lost responses replay the same effect across restart.
- [pinned] The runner MUST acknowledge only after actual provider dispatch,
  never after status ingestion, answer commit, local append, a hook cut or WAL
  intent. The provider trusts the attested caller's dispatch receipt; it does
  not independently observe the network event. An absent acknowledgement keeps
  the exact pending notice armed and re-serves it unchanged. There is no
  implicit expiry or position-based clearing in this extension. A separately
  agreed explicit lease/rearm policy must not clear a different notice or
  manufacture delivery; no such policy is negotiated by these fields.
- [pinned] Pending notices are never armed off the attested, negotiated lane.
  Off-lane absence is valid. Selecting the notice capability in a plan without
  the attested principal refuses `invalid_params` naming `claude_code` at
  admission; emitting a directive off lane refuses naming `channel2_directive`.
  The optional extension cannot grant reduction authority, weaken protection
  gates or imply delivery from ordinary subject service evidence.
