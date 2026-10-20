# cortexkit-role-llm-runner-conformance

The conformance suite for the `llm-runner/v1` role
(`crates/cortexkit-role-llm-runner/CONTRACT.md`). A runner adds it as a
dev-dependency and runs it in its own CI against its real module over its
real management route. It is never run against a double; the in-process
fake under `tests/fake/` exists only to test the suite itself.

## What a runner supplies

The crate-level Rust documentation defines compaction terms in its Contract
vocabulary section and identifies the governing contract sections.

- A `cortexkit_role_harness::Harness`: spawn the runner on a state root,
  kill it at a point of `cortexkit_role_llm_runner::points`, and restart it
  on the same root. The points it declares decide which crash cases run.
- A `RunnerRoute` adapter: send one `{method, params}` request over the real
  route and return the reply; collect a `session.subscribe` stream until it
  goes quiet.
- An `LlmRunnerSubject`: the capability groups and harness capabilities it
  declares, the owner and stranger identities, how to open a session's
  route, the fields of a session's first send (its plan, whose shape the
  role has not pinned yet), the module id of its scripted tool provider, and
  a scripted model. Conformance cases supply each session's model as a
  `Script`:
  assistant turns made of text parts, reasoning parts and tool calls with
  their arguments and scripted results. The runner adapter must back the
  script with a mock provider that speaks the runner's model protocol. The
  adapter's scripted model answers a request with the turn whose index is
  the number of assistant messages in the request's history, and its scripted tool
  provider (serving `SCRIPTED_TOOL`) answers each call with its scripted
  result, counts invocations, and can hold a call until released.
- For `run_attribution_per_message`, which checks that each message names
  the run that produced it, that only a finished run's last message is marked
  final, and that a message whose run is unknown names none: a held scripted
  tool call and `install_attribution_fixture(handle, session)`. The case creates two runs
  through the real route, reads while the second is held, then releases it
  and checks durable finals. If `run_cancel` is declared, it cancels that held
  run over the real route before release. The hook is only for the
  unknown-origin rule: install one native persistent historical/imported
  message in the existing session whose run and episode cannot be recovered.
  Return `AttributionFixture::Installed { ordinal, marker }`, with its
  ordinal and a unique body marker determined by fixture construction,
  **never** by `session.read`. Every check still reads the real route.
  `NotSupported` (including the default hook) fails the case by name.
  `CannotHoldUnattributable` is an explicit **claim by the runner's adapter**,
  usable only when every native write path names a run and the runner truly
  cannot hold such a message. Route-producible rules still run; only the
  unknown-origin rule is reported not applicable by declaration, with its
  reason, through `CaseOutcome::PassedWithInapplicableRule`, never a plain pass.
- For `retention`, an unpaused Tokio runtime, and
  `retention_deletion_finished`: inspect the runner's durable completion
  report and runner-held transcript/derived stores, never infer deletion
  from an `expired` read. `retention_probe_params` classifies every extra
  advertised op and supplies all export/list/read requests and listing pages
  for the leak check. Return `None` only for ops that cannot serve stored
  session content; an unclassified op fails the check. Retention cases send a
  unique `title` along with prompt and tool-result markers; a runner that
  stores caller titles must erase them too.
- For `compaction`, a scripted provider whose Setup answer is installed with
  `install_compaction_setup(session, CompactionSetup)`, included in the
  session plan when installed. The runner adapter must convert replacement
  text to the runner's schema and preserve ids, versions and ranges.
  `compaction_observation` captures actual Setup calls and canonical inputs
  at the scripted model boundary outside the runner's process and state root;
  it must not reconstruct inputs from the configured answer or `session.read`
  responses. Hold every model answer until `release_compaction_model`.
  `durable_compaction_setup` must read the initial answer from persistent
  runner storage while a model call is held, never from an in-memory cache,
  the configured provider answer or a `session.read` model page. The
  compaction cases check Setup only: the runner half of compaction-provider
  0.3.0 §3 (advance notices, views, rides, the ceiling, native markers) has
  no live case yet.
- Retention time can be controlled separately. A subject opts in by declaring
  `Capability::RetentionClock` and implementing
  `LlmRunnerSubject::advance_retention_clock`; the adapter must advance the
  runner's actual retention clock and complete expiry and due deletion work
  before returning. Durable activity timestamps, expiry checks, and deletion
  deadlines must all use that clock. Without the capability, cases use real
  time only when their full wait is at most 30 seconds; longer cases are
  skipped by name with the missing hook and wait bound. This hook belongs to
  the conformance test adapter only and must not be exposed by release builds.

## Verdict

A case whose requirements the runner adapter does not declare is skipped,
naming what is missing, and is never passed. A run with skips and no failure is
"conforming for declared capabilities", naming them. A run fails if any case
fails, or if no kill ended a real process (CONTRACT §14), including a run
that made no kill at all. A case the runner's own declarations make
impossible to exercise is reported inapplicable, which is neither a pass
nor a missing capability: a runner that declares every delivery mode has
no undeclared mode to refuse, and a runner whose session capabilities come
from `session.baseline` has no admission reply to compare.

## Cases in this subset

| Case | Requires |
|---|---|
| `compaction_setup_durable_once` | `compaction`, `model_view`, `queue`, `transcript_reads`, `run_ops`, `FoldRecorded` |
| `model_view_half_open_ranges_and_travel` | `compaction`, `model_view`, `queue`, `transcript_reads`, `run_ops` |
| `compaction_unavailable_distinct_from_refuse` | `compaction`, `queue`, `transcript_reads`, `run_ops` |
| `role_describe_shape`, `role_describe_cacheable`, `role_describe_groups_complete`, `extra_op_still_admitted` | — |
| `describe_states_max_bytes` | `transcript_reads` |
| `baseline_owner_only`, `baseline_matches_admission_reply` | `queue` |
| `tail_read_is_newest_page`, `range_read_stops_by_count`, `range_read_stops_by_bytes`, `oversize_message_alone_on_its_page`, `after_mid_reads_strictly_after`, `after_mid_unknown_mid_refused`, `read_other_lineage_refused`, `lineage_id_on_every_page`, `head_has_no_bodies` | `transcript_reads`, `queue` |
| `unknown_read_field_refused`, `never_written_session_empty_page`, `never_written_session_empty_head` | `transcript_reads` |
| `run_result_completed_final_text`, `run_result_text_parts_joined_unchanged`, `run_result_completed_no_text_is_empty` | `run_ops`, `queue` |
| `run_attribution_per_message` | `run_ops`, `queue`, `transcript_reads`, `hold_tool_calls`; needs the native fixture hook or explicit `CannotHoldUnattributable` declaration (missing hook fails, not skips) |
| `run_result_interrupted_not_cancelled` | `run_ops`, `queue`, `transcript_reads`, kill point `DispatchIntent` |
| `dispatched_to_per_call`, `sibling_calls_distinct_call_keys`, `recurring_model_id_distinct_call_keys` | `dispatch_attribution`, `queue`, `transcript_reads` |
| `indeterminate_until_closed` | `dispatch_attribution`, `queue`, `transcript_reads`, `hold_tool_calls` |
| `subscribe_from_head_no_gap_no_duplicate` | `streaming`, `queue`, `transcript_reads` |
| `send_id_retry_same_answer`, `send_id_reuse_refused_naming_field` | `queue` or `steer`; never reads the transcript (inapplicable without either mode) |
| `send_id_retry_settled_same_answer` | `run_ops` (skipped without it), and `queue` or `steer`; never reads the transcript, waits for the send's run through `run.result` (inapplicable without either mode, or when the send's reply names no `run_id`) |
| `delivery_change_refused` | `queue` or `steer`, and `steer` or `interrupt` as the other mode; never reads the transcript |
| `send_id_retry_written_once`, `send_id_reuse_writes_nothing` | `queue`, and `transcript_reads` (inapplicable without it) |
| `delivery_change_writes_nothing` | `queue`, `steer` or `interrupt` as the other mode, and `transcript_reads` (inapplicable without it) |
| `unknown_delivery_refused` | `queue`, `transcript_reads` |
| `guaranteed_steer_never_pending_or_unknown` | `run_ops`, `queue`, `transcript_reads`; inapplicable without `steer`, when the runner declares `steer_receipt: confirm` (its steer receipts may be `pending` or `unknown`), or without `hold_tool_calls`; a delivered step receipt names the joined run |
| `steer_without_run_ops_reports_state_and_step` | `steer`, `queue`, `transcript_reads`, `hold_tool_calls`; runs only when `run_ops` is not declared; a steer into an active turn answers `active` or `finished`, with `delivered.as: "step"`, non-empty `ref` and no `run_id` |
| `resend_steer_delivered_stable` | `queue`, `transcript_reads`; inapplicable without `steer` |
| `undeclared_delivery_refused` | `queue`, `transcript_reads`, and `steer` or `interrupt` left undeclared |
| `crash_at_Admitted`, `crash_at_SendRecorded`, `crash_at_StepRecorded`, `crash_at_ToolResultRecorded`, `crash_at_Terminal` | `queue`, `transcript_reads`, the kill point |
| `crash_at_DispatchIntent` | `queue`, `transcript_reads`, `dispatch_attribution`, the kill point |
| `tags_resolve_left_lineage_refused` | `tags`, `queue`, `transcript_reads`, `dispatch_attribution`, kill point `StepRecorded`; inapplicable unless the seal drops a tagged part onto a new lineage |
| `delivery_order_never_starts_a_run`, `delivery_order_dedupe_supersedes_queued`, `delivery_order_expired_never_delivered`, `delivery_order_unnamed_sender_refused` | `delivery_order`, `queue`, `transcript_reads`, `steer` |
| `delivery_order_members_without_list_refused` | `delivery_order`, `queue`, `transcript_reads` |
| `delivery_order_first_possible_mode` | `delivery_order`, `queue`, `transcript_reads`, `hold_tool_calls` |
| `delivery_order_template_renders_as_system` | `delivery_order`, `queue`, `transcript_reads`; needs `steer` for fallback or a route that supports `turn_system`; inapplicable if neither is available |
| `delivery_order_bad_slot_refused` | `delivery_order`, `queue`, `transcript_reads` |
| `delivery_order_retract_pending_tool_result` | `retract`, `delivery_order`, `queue`, `transcript_reads`, `hold_tool_calls`; retracts a pending `[tool_result]` send, then releases the held call and confirms the send was never delivered |
| `delivery_order_undeclared_mode_refused` | `delivery_order`, `queue`, `transcript_reads`, `hold_tool_calls`; runs when `steer` is not declared, refuses a `[steer]`-only list as `delivery_unsupported` with the sent list in `detail.delivery`, and proves `[steer, idle_start]` waits for and starts an idle turn |
| `delivery_order_steer_receipt_durable_across_restart` | `delivery_order`, `queue`, `transcript_reads`, `steer`, `run_ops`, `hold_tool_calls`; restarts its own runner |
| `send_status_answers_as_a_resend_after_restart` | `send_status`, the above; the live `role.describe` must declare `send_status` before any `send.status` is sent |
| `send_status_unknown_send_refused` | `send_status`, `queue`, `transcript_reads` |
| `retention_honoured`, `retention_no_content_served`, `retention_equal_noop`, `retention_shorten_continues_lineage`, `retention_lengthen_refused`, `retention_late_opt_in_refused`, `retention_zero_refused`, `retention_above_max_refused` | `retention`, `queue`, `transcript_reads`, `run_ops` |
| `retention_run_status_expired` | same; not applicable if `run.status` is not advertised |
| `retention_without_group_refused` | `queue`; not applicable when `retention` is declared |
| `retention_active_run_never_expires` | retention requirements plus `hold_tool_calls`; a held run may be active or paused |
| `crash_at_RetentionTombstoned` | retention requirements plus the named kill point; must end a real process between tombstone and deletion |

`CASES` in `src/report.rs` states what each case checks.
Missing compaction/model-view requirements are rendered `N/A` by case name
(represented by `CaseOutcome::Skipped`, preserving missing-capability
verdicts), never passed. The equal-anchor insertion-before-message rule is
tested live. Nothing in the current contracts produces an insertion and a
nonempty replacement at one anchor: compaction-provider §8 applies only the
newest message, §12 assigns its one entire range to every replacement, and step-transform hooks
change fields on a raw message rather than inserting range entries. The
conformance driver does not claim live coverage of
insertion-before-replacement ordering.

## Where the suite narrows a check

The role contract marks some rules open (not yet defined), and some of its
guarantees cannot be observed on the wire. `NARROWINGS` in `src/report.rs` lists every such choice, and
every report prints them. In short:

- The fetch plan's shape is not pinned (§10), so the runner adapter builds a
  session's first-send fields and cases do not inspect the plan.
- Cases write sessions with `session.send` (absent `delivery`, meaning
  `queue`), and wait for runs through `session.head`, so cases that need a
  written session require `queue`, and cases that wait require
  `transcript_reads` (run-ops cases wait through `run.result`).
- A read's byte measure is not pinned, so byte-cap cases use a one-byte cap
  or a message far above the cap.
- Message bodies are the runner's schema, so cases find scripted
  content by unique marker strings, and find an `outcome_unknown` close
  only as that string in some message.
- The subscription handoff is checked by comparing replays on an idle
  session, not the live race.
- That a read naming the old lineage after a lineage change is refused is
  checked only after a retention expiry and after a `StepRecorded` recovery
  that drops the never-sent call. That each resume writes one informational
  record is not checked (where it sits is the runner's schema). "Replay, not re-invocation" is checked
   for the model, tool and compaction provider; step-transform hooks remain
   outside this subset. The indeterminate
  window between a restart and the close is not observed; whether a run cut
  at `StepRecorded` or `ToolResultRecorded` completes or ends interrupted is
  not pinned, so only "one terminal state, not cancelled" is checked there.
- `run_result_interrupted_not_cancelled` reads the run cut in the
  `DispatchIntent` scenario, because each point is cut once per run.
- `role_describe_groups_complete` fails a runner whose `role.describe`
  declares a group the subject does not, because the suite would skip that
  group's cases while the wire claims it.
- Each `send_id` case has a reply half, which checks only what
  `session.send` answers and never reads the transcript, and a
  written-once half, which reads what the send wrote and is inapplicable
  without `transcript_reads`. The retry's reply half never waits, so it
  checks only the ids and the `delivered` rule: before the run ends, a
  send's state may move on between retries. Comparing the two retries
  whole is its own case, `send_id_retry_settled_same_answer`, which waits
  for the run to end through `run.result` and so is skipped without
  `run_ops`, and is inapplicable when the send's reply names no `run_id`.
  A `delivered` receipt may move from absent to `pending`, since on a
  `confirm` runner absent already means `pending`.

### Retention coverage

Retention chooses at most two seconds (one if the maximum is one), then
waits through the advertised deletion bound with a 300 ms margin. Reads are
checked at expiry and again after that bound. An idle shortening send keeps
the earlier messages and lineage; a fresh send to an expired session uses a
new lineage and its own retention. Active and paused held runs outlive the
bound and restart the retention clock at their terminal record. Positive shortening
and within-max lengthening are not applicable when the maximum is one;
above-max is not applicable when it is `u64::MAX`.

The leak check scans raw/range/original/model reads, head, run result,
optional run status, baseline and subscription replay, plus every advertised
extra content op classified by the runner adapter. It looks for prompt, tool-result
and caller-title markers in both success and error answers. The deletion
inspection additionally covers copies that are not served.

The self-tests use a virtual Tokio clock and a disk-backed fake, with
deliberate defects that show the conformance case names that go red. These
are not evidence of production conformance: a runner's own CI must run the
suite against its real process and unpaused clock.

## Not yet implemented

These cases, planned in §15 of the role contract, belong to a later
subset:

- compaction: the runner half of compaction-provider 0.3.0 §3 (advance
  notices, views applied at step boundaries, rides, the ceiling, native
  markers), `compaction_item_without_group_refused_writes_nothing` and
  `crash_at_CompactionApplied` (a fetched view recorded, not yet rendered);
- `model_view`: `model_view_differs_only_by_compaction`,
  `model_view_entry_sources`, `model_view_replacement_once_on_its_first_page`,
  `model_view_after_mid_refused`, `model_view_names_compaction_state`;
- `steer` placement and the other delivery behaviours:
  `steer_lands_at_step_boundary`, `steer_never_between_call_and_result`,
  `queue_starts_next_turn`, `interrupt_cancels_then_starts`,
  `interrupt_waits_for_running_tool`, `steer_inherits_frozen_runner_params`;
- hooks: `include_originals`, `pre_user_refuse_writes_nothing`;
- `session_change`: `refresh_unsupported_rung_refused`,
  `refresh_policy_superseded_refused`,
  `refresh_policy_already_applied_refused`, `flush_prefix_applies_pending`,
  and `crash_at_FoldRecorded`;
- also `baseline_not_yet_before_first_request`;
- `run_status_undelivered_steers_present_when_clean` is planned and not
  implemented. It will check that `run.status` on an ended run lists the
  steering messages that were never delivered, and lists none when every
  steer was delivered. It requires `run_status` and `queue`.

## Step-boundary cancellation

The `run_cancel_step_boundary_finishes_tool_batch`,
`run_cancel_unlisted_at_refused` and
`run_cancel_step_boundary_escalates_to_now` cases run only when the live
`role.describe.run_cancel.at` lists `step_boundary`. Otherwise the report
names each skipped case and the missing cancellation point, never a pass.
The cases also require their listed read/run/send capabilities; the batch
case requires held tool calls.

An adapter for a runner advertising this point implements the defaulted
`hold_next_model_step`, `await_model_step`, `release_model_step` and
`model_invocations` hooks. The hold is in the scripted model, after it has
received an actual request, not a runner-side pause before a step starts.
Count requests at the model independently of transcript reads so a discarded
extra model answer cannot hide a violation. The batch case cancels while the
model is held, then releases its answer and an entire two-call tool batch;
escalation keeps the model held until `now` has ended the run.

## Structured output

The ten `structured_output_*` cases run only when the subject declares
`Capability::StructuredOutput` (the `structured_output` group); otherwise the
report names each one as skipped, never as passed. They also require
`queue`, `transcript_reads` and `run_ops`, and
`structured_output_frozen_across_restart` also requires the
`ToolResultRecorded` kill point.

An adapter for such a runner implements three defaulted hooks:

- `send_fields_with_tools(session, first, tools)`: send fields admitting
  exactly `tools`, so the cases can open a tool-free session for
  `final_text` and one listing `submit_result` for the reserved-name refusal;
- `set_structured_output_support(session, supported)`: what the scripted
  route reports about schema-constrained output, as a catalog row would
  (`Some(true)`, `Some(false)`, or `None` for not stated);
- `model_requests(session)`: every model request the scripted model
  received, as `ModelRequestObservation`s (offered tool names, the final-text
  schema, the `submit_result` input schema), kept outside the runner.

A scripted call whose `tool` is `submit_result` is the model calling the
runner's submit tool: its `arguments` are the submitted value, and the
scripted tool provider never sees it. Decode the never-executed result of a
skipped sibling through `not_executed_tool_result`; its code is
`skipped_after_submit`.

The restart case kills the runner right after a tool result is saved. The
suite's crash driver kills at each point only once per run, and the general
crash cases already use that point, so this case uses a separate driver on a
separate state root. The suite's count of real kills does not include it.

## Adoption

The eight `adopt_*` cases run only when the subject declares
`Capability::Adopt` (the `adopt` group); otherwise the report names each one
as skipped, never as passed, and the suite sends no `session.adopt`. Each
case also reads the live `role.describe` and sends nothing unless it declares
`adopt`. They require `transcript_reads`; `adopt_during_active_run` also
requires held tool calls, and `adopt_then_owner_sends` also requires `queue`.

An adapter for such a runner implements two defaulted hooks:

- `host_plan(session, alternate)`: the fetch plan the host applies to
  `session` when it starts, which must make the scripted tool available; with
  `alternate`, another plan with a different identity that the runner would
  otherwise accept, for the drift and reuse cases;
- `start_host_session(handle, session, plan, prompt)`: start `session` as a
  person's own app would, with `plan` applied and no role request yet, and
  with `prompt`, the person's first turn, answered by the session's script.

The cases compare the transcript before and after every adoption: the
lineage, the last ordinal, the last run and its state, and every message.
They never compare a plan identity with a digest of their own; they compare
the identities the runner reports with each other.

## Tags

The eight `tags_*` cases run only when the subject declares
`Capability::Tags` (the `tags` group); otherwise each is reported as skipped,
by name, and the suite sends no `session.resolve`. They require `queue` and
`transcript_reads`, and read the live `role.describe` before resolving. They
check part rows (a stable key per exposed part, at least one tagged part per
entry, each tag a well-formed `§N§` used once), that a later run never reuses
or lowers numbers, that tags exist whether or not the plan renders them,
`session.resolve`'s exact stored content (with a character range counted in
Unicode scalar values) and its `unknown_tag`, `tag` and `range` refusals, and
that a model-written `§N§` is stored and dispatched as written. The rendered
case needs the defaulted `send_fields_with_plan(session, first, members)`
hook, which adds members (here `render_tags`) to the first send's plan.

`tags_resolve_left_lineage_refused` also requires `dispatch_attribution`
and the `StepRecorded` kill point: it reads the `crash_at_StepRecorded`
scenario. After a seal that drops the never-sent call or its step and moves
the session to a new lineage (shape (a)), the kill came before any read of
the cut step, so the case cannot name the dropped part's tag; it resolves
every tag number below the new lineage's highest that no part read from it
carries (at most 64), which must each be refused `unknown_tag`, and resolves
the cut run's prompt, which must answer its ordinal and `mid` in the new
lineage. It is inapplicable, by name, when the runner resumes the run or
keeps the call (shape (b)), and when the seal dropped no tagged part.

## Run tool calls

The three `run_tool_calls_*` cases run only with `Capability::RunToolCalls`.
They follow `next` one call a page and check dispatch order, names, ids,
exact `args`, outcomes, the dispatch module, stored result sizes and the
carrying ordinal; `unknown_run`, a misspelled cursor and `limit: 0`; and, with
held tool calls, `indeterminate` until the result is stored.

## Run usage

`run_usage_counts_reported_steps` runs only with `Capability::RunUsage` (the
`run_usage` group) and needs `queue` and `run_ops`. Its scripted model reports
input, cache-read, cache-write and output counts on both steps of a two-step
run, and no reasoning; the runner adapter must pass those counts to the runner
as its provider would, and report no usage field the script does not set.
`run.result` must then carry `usage` with `steps: 2`, no unreported step,
exactly the scripted totals, `by_model` summing to them, and
`reasoning_tokens` absent, not `0`. Where the subject declares `run_status`,
the ended run's `run.status` must carry the same `usage`.

## Delivery order

The nine `delivery_order_*` cases run only with `Capability::DeliveryOrder`.
They need `send_fields_with_plan` (to name `senders` in the plan) and
`plan_sender()`, the route identity and module id of a sender the plan
names. `turn_system_supported()` says whether the scripted route takes a
turn-scoped system message: the template case then expects `turn_system`,
and otherwise the same rendered text at user authority. They check that a
list without `idle_start` never starts a run, that the first possible mode
wins (with a held tool call for `tool_result`), dedupe, expiry, the refusal
by name of `template`, `dedupe_key` and `expires_at_ms` sent without a list in
`delivery`, plan-named senders and the `idle_start`
grant, and that an out-of-range slot refuses the send. The list rides in
`delivery` as an array. `delivery_order_list_refused_without_group` runs only
when the subject does not declare the group: such a runner must refuse a
list naming `delivery`.

## Following a queued send

`delivery_order_steer_receipt_durable_across_restart` needs
`Capability::DeliveryOrder`, `queue`, `transcript_reads` and held tool
calls. On a state root of its own it sends a `[steer]` list while a run waits
on a held call, which must answer `pending` with a `submission_id`; releases
the call, so the steer is delivered once into that run; and re-sends the
`send_id`, which must answer `delivered: {as: "step", ref}` with `run_id`
the run it joined. It then stops the runner, reopens the same root with the
harness restart hook (`Harness::restart`, no kill point), and re-sends again:
the answer must carry the same `delivered` and `run_id`.

The two `send_status_*` cases run only with `Capability::SendStatus` (the
`send_status` group), and read the live `role.describe` before sending
`send.status`. `send_status_answers_as_a_resend_after_restart` follows the
same steer through `send.status`: before the restart it must answer exactly
what a re-send answers, and after it `step` with the joined run's `run_id`,
never `pending`, and exactly what a re-send then answers.
`send_status_unknown_send_refused` needs `queue` and `transcript_reads`: the
status of a settled send must equal its re-send's answer, an unrecorded
`send_id` must be refused `unknown_send`, and an unknown field must be
refused `invalid_params` naming it. The restart cases restart the runner on
their own root without a kill, so they do not count toward the real-kill
rule.

## Context ceiling and read stability

`run_context_ceiling_wait_is_not_a_stall` needs the harness capability
`Capability::HoldContextCeiling` and the hooks `hold_context_ceiling` and
`end_context_ceiling_wait`. `read_stable_across_host_metadata_change` needs
`Capability::HostMetadataChange` and `change_host_metadata`. At
`crash_at_StepRecorded`, session B first finishes one run so its lineage can
be read before the kill. A seal that drops the never-sent call or its step
must leave B on a new lineage, and a read naming the old one must be
refused `lineage_changed`. A seal that keeps the call must keep B's lineage,
read back every message of B read before the kill unchanged, and hold the
call's never-executed result in the message right after it once the run has
ended `interrupted`; the owner's follow-up must leave that transcript as it
was and write no message carrying the call. A failure names the shape it
was closest to.
