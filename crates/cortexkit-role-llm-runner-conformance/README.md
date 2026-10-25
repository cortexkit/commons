# cortexkit-role-llm-runner-conformance

`display_row_is_the_profile_projection` permits an imported assistant message
with no recorded model. `display_own_run_records_model` separately drives a
unique scripted answer through the runner and requires its recorded model.

Descriptor-read adapters supply `media_tool_result` (actual decoded image
bytes, with a nested tool-result layout when requested), `multipart_tool_result`
for the fixed-budget fallback, and `read_parts` for their host's stable part
keys. The kit validates message/part digest rows against stored content
(a separate blocks read on attachment runners, inline otherwise),
never descriptor JSON; unsupported image fixtures are named inapplicable.
Compaction providers must read with `media: "descriptors"` and trust served
content digests. This runner kit exercises Setup, not a provider's reverse-read
implementation.

The conformance suite for the `llm-runner/v1` role
(`crates/cortexkit-role-llm-runner/CONTRACT.md`). A runner adds it as a
dev-dependency and runs it in its own CI against its real module over its
real management route. It is never run against a double; the in-process
fake under `tests/fake/` exists only to test the suite itself.

## What a runner supplies

New in 0.12.3, for llm-runner 0.6.5: `system_text_default` is read from
live `role.describe`. Both new cases are `N/A` when it is absent, never
skipped or failed. Declare `Capability::SystemTextDefault` alongside the
live group for the kit's group-consistency check.
`prepare_system_text_default` installs the scripted tool provider with the
supplied manifest capabilities in the daemon catalog and returns a plan
carrying the supplied system-text item. `system_text_default_observation`
captures the actual provider-received item bytes and durable state, not a
reconstruction from the plan. Declaring runners must supply these hooks.
`system_text_default_forwarded_unchanged` admits an item without a preset
with a capable provider and requires byte-equal forwarding.
`system_text_default_preset_required` uses a provider without the capability
and requires `system_text_preset_required` naming it, no durable writes and
no provider calls. Faulty fakes invent a preset or bypass the catalog check;
each fails only its matching case by name.

New in 0.11.2, for llm-runner 0.6.2: `head_sender` is discovered directly
from `role.describe` (no new `Capability` enum variant). Without that group,
`head_sender_is_callers_effective_grant` is inapplicable with a reason naming
`head_sender`, even when the adapter supplies its two-identity fixture.
The reference fake declares the group; a faulty runner that declares it but
omits every `sender` fails the case by name.

New in 0.11.1, for llm-runner 0.6.1: declare `Capability::SessionUsage`
when serving the `session_usage` group, and opt into the scripted fixtures
with `session_usage_fixtures() -> true`. The three `session_usage_*` cases
also need `RunUsage`, `Queue`, `RunOps` and `TranscriptReads`. They check
two-run sums and `since`, unchanged spend after a rewind, and unknown
reasoning tokens remaining absent. Implement `rewind_session_usage` through
the host's real history-rewind path; return false if that is unavailable.
`head_model_reflects_recorded_switch` runs when the head reports a model;
`switch_head_model` records a switch without rewriting the frozen plan and
returns the recorded selection. Unsupported default hooks make these
observations inapplicable, so existing subjects compile unchanged. Each
case has faulty-runner self-tests that must fail it by name.
`head_sender_identities` optionally supplies two attested non-owner modules
A and B. The kit installs a plan naming A without flags and B with both
`idle_start` and `interrupt`. `head_sender_is_callers_effective_grant` compares
their grants to supported modes from checked describe, also checking an
unnamed caller and the owner. Existing adapters return `None` and remain
inapplicable; adapters with the group and hook run regardless of their
supported-mode set. Declaring runners must report `sender` for both named
callers and the owner (with `owner: true`).
`cut_incarnation_log` and `recreate_incarnation_log` optionally exercise a
reported log identity through a real cut and creation of a new log at the
same key. Defaults are inapplicable; both lifetime faults have named tests.

New in 0.11.0, for `cortexkit-role-llm-runner` 0.6.0, whose `started_at`
and `finished_at` on `tool_started` and `tool_finished` are now optional:
`seed_untimed_tool_call` puts into a session's stored history a finished
call the way history stored before the runner kept tool-call times holds
it. The new case `live_untimed_history_call_leaves_times_out` replays it
and requires both events without a time; a runner that invents one (now,
zero, the message's time) fails it by name. The hook has a default that
makes the case inapplicable by name, so existing adapters build
unchanged. `live_message_and_tool_events_durable` now also requires both
times on a call the runner ran itself, the finish no earlier than the
start.

New in 0.10.4: `held_calls` is discovered directly by its open string on
`role.describe` (no new `Capability` enum variant). Its four cases require
`run_status`, `queue`, `transcript_reads` and `run_ops`; continuation cases
also use `RenderedModelInput` and `model_requests`. Implement
`install_held_reply`, `await_held_reply` (return the call key observed at the
provider, not status), and `publish_late_entry(ScriptedLateEntry)`. The provider
must send an actual `held-reply/v1` first reply, then expose the separately
published entry through `late_results`; merely holding an unanswered request
is not sufficient. Missing hooks fail when the group is declared; an absent
group makes the cases N/A (inapplicable) by name, without lowering the
runner's verdict. The existing `ScriptedToolCall::hold` seam remains
unchanged for older cases.

New in 0.10.3: declare `Capability::Attachments` only for the complete
`attachments` group, then implement `attachment_model_fields` (configure
real provider/catalog PNG/PDF or text-only input facts),
`attachment_tool_result` (text/image/text in the host's result format), and
`restart_attachments` (simulate loss of open uploads and reopen the owner's
route, preserving committed media and history). Hooks configure fixtures;
they never synthesize management replies. All have defaults, so older
adapters build unchanged and every attachment case is skipped by name when
the group is absent. Declaring the group without its fixture hooks fails
the affected case. The plan-sender and one-hour lifetime cases additionally
  use the existing `DeliveryOrder` and `RetentionClock` capabilities/hooks.

New in 0.10.1, each with a default that makes its cases inapplicable by
name: `serializer_profile` (the profile of a session's stored bodies, for
the display projection and provider descriptor-row cases), `hold_next_model_stream` (stream the first
parts of the next answer, then hold it), `advance_prompt_cache_clock`,
`switch_model` and `set_observed_cache_lifetime` (the live `cache_ttl`
cases), and the `CompactionSetup::ReadyWithCacheTtl` answer. Display
sessions are minted with the label `display` (`cf-display-<n>`).


The crate-level Rust documentation defines compaction terms in its Contract
vocabulary section and identifies the governing contract sections.

- A `cortexkit_role_harness::Harness`: spawn the runner on a state root,
  kill it at a point of `cortexkit_role_llm_runner::points`, and restart it
  on the same root. The points it declares decide which crash cases run.
- A `RunnerRoute` adapter: send one `{method, params}` request over the real
  route and return the reply; collect a `session.subscribe` stream until it
  goes quiet. The suite sees decoded answers, never a reply's raw bytes, so
  no case can catch a runner that sends a bare answer: the adapter decodes
  every `RESPONSE` body, `{"result": <answer>}`, with
  `cortexkit_role_llm_runner::wire::read_reply` and returns the answer
  inside, reporting a body it refuses (a bare answer, extra top-level keys)
  as a route failure naming the problem. Stream events arrive bare.
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
   `compaction_observation` captures the Setup body's `harness` and `session`,
   the actual route bind's `harness`, `session` and `project_root`, and
   canonical model inputs, all outside the runner's process and state root.
   Route identity must be observed at provider ingress, not reconstructed
   from Setup or `session.read` responses. A single-session route's bind
   session must equal Setup's session; a shared project route with an empty
   session and a generic harness is valid. The named case
   `compaction_setup_route_session_matches_setup` checks both forms, and
   that a Setup refused `setup_identity_mismatch` ends the run `error` under
   that code. Hold
   every model answer until `release_compaction_model`.
   `durable_compaction_setup` must read the initial answer from persistent
   runner storage while a model call is held, never from an in-memory cache,
   the configured provider answer or a `session.read` model page. Each Setup
   call also records the `lineage_id` its body carried
   (`CompactionCall::lineage_id`): Setup runs once per lineage.
   `publish_compaction_view(session, ScriptedView)` publishes a view from the
   scripted provider (ranges, ordinals, `mid`s and cut kept verbatim, the
   replacement text translated to the runner's schema, the digest computed by
   the adapter) and returns its version; `compaction_observation` also
   returns every advance notice the runner published, as the header set the
   bus carried (`CompactionObservation::notices`). Without the view hook the
   cut cases are inapplicable by name. Rides, the ceiling and native markers
   still have no live case.
- For `host_history`, nothing beyond the route, plus three optional hooks:
  `revert_session(handle, session, mid)` reverts a session as its host does;
  `seed_host_history(handle, session, count)` gives a session `count`
  messages of host history before the runner numbers anything, as a runner
  over a host that adopted a session with history; each case needing one is
  inapplicable by name without it.
- For the opt-in cost case, `seed_session_history(handle, session, messages,
  window)` writes finished messages through the runner's own store and a
  cut leaving `window` of them in the lineage. Without it the case is
  inapplicable, never passed.
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
The discovery-gated `held_calls` cases are also inapplicable when
`role.describe` does not declare that optional group, not skipped.

## Progress while a run is in flight

A full run takes many minutes, and a CI job can be cut off before the report
is printed. `run_suite_with` takes `SuiteOptions`; its progress callback is
called synchronously before each case starts and right after its outcome is
known (`SuiteProgress::CaseStarted` / `CaseFinished`). Every case gets both
events, including one skipped or inapplicable before it runs.
`progress::stderr_lines()` prints them to stderr, flushing after each line:

```rust
let options = SuiteOptions::new().with_progress(progress::stderr_lines());
let report = run_suite_with(&my_runner_subject, tempdir.path(), options).await?;
```

```text
START tail_read_is_newest_page
END tail_read_is_newest_page PASS 412
```

`END` carries the case's outcome word as the report renders it (`PASS`,
`FAIL`, `SKIP` or `N/A`) and its elapsed milliseconds. A log cut off after a
`START` with no matching `END` names the case that was running. `run_suite`
is unchanged and prints nothing.

## Cases in this subset

| Case | Requires |
|---|---|
| `held_calls_lists_waiting_call`, `held_calls_applies_late_result`, `held_calls_not_started_becomes_tool_result`, `held_calls_expired_becomes_tool_result` | `held_calls` on `role.describe`, `run_status`, `queue`, `transcript_reads`, `run_ops`; the last three also require `RenderedModelInput`; late entries continue the run with the result, a note that the call never ran, or a note that its answer expired, respectively |
| `cut_starts_a_new_lineage`, `old_lineage_reads_refused_after_a_cut`, `read_below_first_ordinal_refused`, `ordinals_never_renumber_across_a_cut`, `setup_once_per_lineage`, `view_ranges_start_at_first_ordinal`, `cut_lineage_notice_descends_from` | `compaction`, `queue`, `transcript_reads`, `run_ops`, and the view hook |
| `history_pages_back_to_the_first_message`, `history_mid_matches_read`, `history_never_serves_a_reverted_branch`, `history_cursor_expired_after_a_revert_past_it`, `history_boundary_order`, `history_before_mid_continues_below_the_window` | `host_history`, `queue`, `transcript_reads`; the revert cases need the revert hook, the last two the host-history hook |
| `head_names_the_cut`, `notice_carries_first_ordinal`, `revert_below_the_cut_restarts_at_the_previous_cut`, `revert_below_a_cut_before_adoption_has_no_descent`, `checkpoint_on_descending_setup_refused`, `cut_setup_leaves_reads_alone`, `cut_numbering_continues_from_the_old_lineage` | as the cut cases; the revert cases need the revert hook, the adoption case `seed_host_history` with `cut`, the checkpoint case `tag_checkpoint`, the reads case the Setup-hold hooks and `holds_reads_during_setup`, the numbering case `tags` |
| `history_version_unchanged_by_a_cut`, `history_version_rises_at_a_revert`, `history_from_ordinal_and_before_ordinal`, `history_unknown_ordinal_refused`, `history_before_lowest_ordinal_reaches_unnumbered_entries` | `host_history`, `queue`, `transcript_reads`; the cut case the cut requirements, the revert case the revert hook, the last the host-history hook |
| `history_no_tags_below_first_ordinal`, `history_ordinal_kept_below_the_cut`, `history_cursor_survives_a_cut` | `host_history`, `compaction`, `queue`, `transcript_reads`, `run_ops`, and the view hook |
| `display_row_is_the_profile_projection`, `display_own_run_records_model`, `display_limits_cut_on_a_utf8_boundary`, `display_without_limits_is_whole`, `display_pages_carry_finished_messages_only` | `history_display`, `host_history`, `queue`, `transcript_reads`; the projection case needs `serializer_profile`, the finished-only case `hold_next_model_step` |
| `client_message_id_on_display_rows_never_in_read` | `client_message_id`, `history_display`, `host_history`, `queue`, `transcript_reads` |
| `client_message_id_resend_with_another_id_refused` | `client_message_id`, `queue` |
| `live_message_and_tool_events_durable`, `live_run_events`, `live_attach_replays_the_current_step`, `live_seq_rises_across_lanes` | `live_events`, `streaming`, `queue`, `transcript_reads`; the attach and order cases need `hold_next_model_stream` |
| `live_untimed_history_call_leaves_times_out` | as the other `live_` cases; inapplicable without `seed_untimed_tool_call` |
| `run_cause_queued_start_names_its_send`, `run_cause_send_started_runs_never_omit_send_id` | `run_cause`, `live_events`, `streaming`, `queue`, `transcript_reads` |
| `run_cause_interrupt_names_the_new_send` | as the other `run_cause` cases; inapplicable without `interrupt` or `hold_next_model_step` |
| `display_row_names_the_sender_never_in_read` | `history_display`, `host_history`, `delivery_order`, `queue`, `transcript_reads`, and a plan sender; inapplicable when `idle_start` is not a declared list mode |
| `cache_ttl_entry_for_another_model_ignored`, `cache_ttl_configured_null_never_expires`, `cache_ttl_missed_model_change_falls_through_to_observed` | `compaction`, `queue`, `transcript_reads`, `run_ops`, and the hooks `switch_model`, `set_observed_cache_lifetime` and `advance_prompt_cache_clock`; inapplicable when the runner never declares `ttl_expired` for its own model's expired entry |
| `ride_wait_provider_wait_honoured`, `ride_wait_slow_answer_reported`, `ride_wait_fast_answer_not_reported`, `ride_wait_above_cap_fails_setup` | `compaction`, `queue`, `transcript_reads`, `run_ops`, and the hooks `observed_ride_wait_max_ms`, `script_next_ride`, `switch_model`, `set_observed_cache_lifetime` and `advance_prompt_cache_clock`; inapplicable when the runner sends no `ride_wait_max_ms` in its Setup request or never declares `ttl_expired`, and the first three when its cap is below 4,000 ms |
| `per_step_cost_scales_with_new_messages` | `queue`, `transcript_reads` or `run_ops`, and the seeding hook (opt-in) |
| `compaction_setup_durable_once` | `compaction`, `model_view`, `queue`, `transcript_reads`, `run_ops`, `FoldRecorded` |
| `model_view_half_open_ranges_and_travel` | `compaction`, `model_view`, `queue`, `transcript_reads`, `run_ops` |
| `compaction_unavailable_distinct_from_refuse` | `compaction`, `queue`, `transcript_reads`, `run_ops` |
| `role_describe_shape`, `role_describe_cacheable`, `role_describe_groups_complete`, `extra_op_still_admitted` | — |
| `describe_states_max_bytes` | `transcript_reads` |
| `head_sender_is_callers_effective_grant` | `head_sender`, `queue`, `transcript_reads`; inapplicable without the describe group or `head_sender_identities` |
| `baseline_owner_only`, `baseline_matches_admission_reply` | `queue` |
| `tail_read_is_newest_page`, `range_read_stops_by_count`, `range_read_stops_by_bytes`, `oversize_message_alone_on_its_page`, `after_mid_reads_strictly_after`, `after_mid_unknown_mid_refused`, `read_other_lineage_refused`, `lineage_id_on_every_page`, `head_has_no_bodies` | `transcript_reads`, `queue` |
| `unknown_read_field_refused`, `never_written_session_empty_page`, `never_written_session_empty_head` | `transcript_reads` |
| `run_result_completed_final_text`, `run_result_text_parts_joined_unchanged`, `run_result_completed_no_text_is_empty` | `run_ops`, `queue` |
| `run_attribution_per_message` | `run_ops`, `queue`, `transcript_reads`, `hold_tool_calls`; needs the native fixture hook or explicit `CannotHoldUnattributable` declaration (missing hook fails, not skips) |
| `run_result_interrupted_not_cancelled` | `run_ops`, `queue`, `transcript_reads`, kill point `DispatchIntent` |
| `user_commands_outcome_unknown` | `user_commands`, `queue`, `run_ops`, kill point `UserCommandForwarded` |
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
| `tags_resolve_answers_the_current_lineage` | `tags`, `queue`, `transcript_reads`, `dispatch_attribution`, kill point `StepRecorded`; inapplicable unless the seal moves the session onto a new lineage |
| `tags_numbered_in_rendered_order` | `tags`, `queue`, `transcript_reads`, `steer`, `hold_tool_calls`, `rendered_model_input`; inapplicable when the runner renders a steer where it stores it |
| `tags_renumbering_waits_for_the_step_boundary` | `tags`, `queue`, `transcript_reads`, `steer`, `hold_tool_calls` |
| `tags_setup_checkpoint_anchors_at_its_cut` | `tags`, `tag_checkpoint`, `compaction`, `queue`, `transcript_reads` |
| `tags_fork_of_a_managed_session_refused` | `tags`, `compaction`, `queue`, `transcript_reads`; inapplicable unless the adapter can create a fork the runner can detect (`fork_session`) |
| `adopt_host_turn_is_a_run` | `adopt`, `transcript_reads`, `run_ops`; inapplicable unless the adapter can start a host turn |
| `delivery_order_never_starts_a_run`, `delivery_order_dedupe_supersedes_queued`, `delivery_order_expired_never_delivered`, `delivery_order_unnamed_sender_refused` | `delivery_order`, `queue`, `transcript_reads`, `steer` |
| `delivery_order_members_without_list_refused` | `delivery_order`, `queue`, `transcript_reads` |
| `delivery_order_named_sender_queue_string_admitted` | `delivery_order`, `queue`, `transcript_reads` |
| `delivery_order_ungranted_interrupt_refused` | `delivery_order`, `queue`, `transcript_reads`; inapplicable without `interrupt` |
| `delivery_order_granted_interrupt_interrupts` | `delivery_order`, `queue`, `transcript_reads`, `run_ops`; inapplicable without `interrupt` or `hold_next_model_step` |
| `delivery_order_first_possible_mode` | `delivery_order`, `queue`, `transcript_reads`, `hold_tool_calls` |
| `delivery_order_declared_modes_respected` | `delivery_order`, `queue`, `transcript_reads`; inapplicable without live `delivery_order_modes`; all lists containing only undeclared modes are refused with `detail.delivery` and no writes, and lists with a listed mode are accepted |
| `delivery_order_template_renders_as_system` | `delivery_order`, `queue`, `transcript_reads`; needs `steer` for fallback or a route that supports `turn_system`; inapplicable if neither is available |
| `delivery_order_bad_slot_refused` | `delivery_order`, `queue`, `transcript_reads` |
| `delivery_order_template_part_gone_refused` | `delivery_order`, `queue`, `transcript_reads`, `tags` |
| `delivery_order_retract_pending_tool_result` | `retract`, `delivery_order`, `queue`, `transcript_reads`, `hold_tool_calls`; retracts a pending `[tool_result]` send, then releases the held call and confirms the send was never delivered |
| `delivery_order_undeclared_mode_refused` | `delivery_order`, `queue`, `transcript_reads`, `hold_tool_calls`; runs when `steer` is not declared, refuses a `[steer]`-only list as `delivery_unsupported` with the sent list in `detail.delivery`, and proves `[steer, idle_start]` waits for and starts an idle turn |
| `delivery_order_steer_receipt_durable_across_restart` | `delivery_order`, `queue`, `transcript_reads`, `steer`, `run_ops`, `hold_tool_calls`; restarts its own runner |
| `send_status_answers_as_a_resend_after_restart` | `send_status`, the above; the live `role.describe` must declare `send_status` before any `send.status` is sent |
| `send_status_unknown_send_refused` | `send_status`, `queue`, `transcript_reads` |
| `max_steps_final_step_completes` | `max_steps`, `queue`, `run_ops`; with `max_steps: 1`, a one-text-step model ends `completed` with its final message, and its directives where `turn_directives` is declared |
| `max_steps_submit_result_completes` | `max_steps`, `structured_output`, `queue`, `transcript_reads`, `run_ops`; with `max_steps: 1`, a step calling `submit_result` ends `completed` with the value and its sibling `skipped_after_submit` |
| `session_refresh_applies_from_next_run`, `session_refresh_waits_for_its_rung`, `session_refresh_repeat_same_answer`, `session_refresh_generation_stale_refused`, `session_refresh_tool_outside_host_set_refused` | `session_change`, `queue`, `run_ops`, and the adapter's `refresh_plan` and `model_requests`; inapplicable by name when the session states no fitting rung or the adapter has no fitting plan |
| `session_refresh_never_changes_a_running_run`, `flush_prefix_applies_pending` | the above, and `hold_tool_calls` |
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

- The prompt cache's lifetime (compaction-provider §8.1) is checked live
  only through the `ttl_expired` token of advance notices, with entries on
  Setup's answer. A ride's `ttl_expired` cause, and the newest-entry rule
  across several entries, are checked only at unit level, against
  `cache_ttl::lifetime` and its decision vectors.
- `session.head`'s `input` has no live case; it is checked against the
  role crate's vectors.

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
  `refresh_policy_already_applied_refused`, and `crash_at_FoldRecorded`;
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

The nine `adopt_*` cases run only when the subject declares
`Capability::Adopt` (the `adopt` group); otherwise the report names each one
as skipped, never as passed, and the suite sends no `session.adopt`. Each
case also reads the live `role.describe` and sends nothing unless it declares
`adopt`. They require `transcript_reads`; `adopt_during_active_run` also
requires held tool calls, `adopt_then_owner_sends` also requires `queue`, and
`adopt_host_turn_is_a_run` also requires `run_ops`: the person's own turn,
started through `start_host_session`, must be a run with a `run_id` that
`run.result` answers (and `run.status` and `session.subscribe` show, where
declared). It is inapplicable when the adapter cannot start a host turn.

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

The thirteen `tags_*` cases run only when the subject declares
`Capability::Tags` (the `tags` group); otherwise each is reported as skipped,
by name, and the suite sends no `session.resolve`. They require `queue` and
`transcript_reads`, and read the live `role.describe` before resolving.
Numbering applies only to sessions whose plan renders tags (llm-runner 0.4.0
§9.6), so the cases open their sessions through the defaulted
`send_fields_with_plan(session, first, members)` hook with `render_tags:
true`; an adapter without it fails them by name.

They check part rows (a stable key per exposed part, each number a
well-formed `§N§` used once in its lineage, the numbers consecutive), that a
later run on the same lineage keeps every earlier number, that a session
whose plan leaves `render_tags` absent numbers nothing, `session.resolve`'s
exposed content (with a character range counted in Unicode scalar values)
and its `unknown_tag`, `tag` and `range` refusals, and that the model's own
tag notation is cleaned on every exposure of a tag-rendering run, exposed
byte for byte for an untagged run, and left raw in a structured-output run's
final text (where `structured_output` is declared; the case names that rule
as unchecked otherwise), while tool-call arguments reach the tool as written.
`tags_count_only_parts_rendered_with_a_number` checks that reasoning and a
tool call carry no number; with the defaulted `image_tool_result(marker)`
hook it also checks that an image-only tool result carries one.

Two cases steer into a held tool call, the one renumbering every runner
shares (a steer stored before the call's result and rendered after it).
`tags_numbered_in_rendered_order` needs `Capability::RenderedModelInput`: the
adapter records, in `ModelRequestObservation::rendered`, every text each
model request carried as the model received it, and the case checks that the
last request's numbers are the ones `session.read` reports, increasing in
rendered order. `tags_renumbering_waits_for_the_step_boundary` checks that
nothing a read showed changes while the call is still held, and that a
renumbered part afterwards comes with a new lineage.

`tags_setup_checkpoint_anchors_at_its_cut` needs `Capability::TagCheckpoint`
and `compaction`: its scripted provider answers Setup with
`CompactionSetup::ReadyWithCheckpoint`, a checkpoint cut at the Setup
request's `newest` message. `tags_fork_of_a_managed_session_refused` needs the
defaulted `fork_session(handle, parent, fork)` hook, which makes a fork the
runner can detect; without one the case is inapplicable, by name, since a
runner refuses only forks it can know about.

`tags_resolve_answers_the_current_lineage` also requires
`dispatch_attribution` and the `StepRecorded` kill point: it reads the
`crash_at_StepRecorded` scenario, whose session B renders tags on a tags
runner. After a seal that moves the session to a new lineage (shape (a)),
every number the new lineage shows (up to 64) must resolve to the part
showing it there, never to a dropped part, a number above its highest must
be refused `unknown_tag`, and the cut run's prompt must carry a number. It
is inapplicable, by name, when the runner resumes the run or keeps the call
(shape (b)).

## Run tool calls

The five `run_tool_calls_*` cases run only with `Capability::RunToolCalls`.
They follow `next` one call a page and check dispatch order, names, ids,
exact `args`, outcomes, the dispatch module, stored result sizes and the
carrying ordinal; `unknown_run`, a misspelled cursor and `limit: 0`; and, with
held tool calls, `indeterminate` until the result is stored.

`run_tool_calls_result_ordinal_tracks_storage` requires an open call to have
position 0 and no `result_ordinal`. Its requesting-message ordinal may be
unreadable until release because an embedded call/result message is not yet
finished. Any already-readable message at that ordinal must carry the arguments.
After release, the requesting message must exist and carry the arguments, and
`result_ordinal >= ordinal` must locate the stored result; equal ordinals are
valid for embedded layouts.

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

The `delivery_order_*` cases run only with `Capability::DeliveryOrder`.
They need `send_fields_with_plan` (to name `senders` in the plan) and
`plan_sender()`, the route identity and module id of a sender the plan
names. `turn_system_supported()` says whether the scripted route takes a
turn-scoped system message: the template case then expects `turn_system`,
and otherwise the same rendered text at user authority. They check that a
list without `idle_start` never starts a run, that the first possible mode
wins (with a held tool call for `tool_result`), dedupe, expiry, the refusal
by name of `template`, `dedupe_key` and `expires_at_ms` sent without a list in
`delivery`, plan-named senders and the `idle_start`
grant, that an out-of-range slot refuses the send, and, on a session
rendering tags (which needs `tags`), that a `part_tag` slot naming a part
the session does not show is refused `template_part_gone`. The list rides in
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

## User commands

Declare the independent `UserCommands` capability for runners serving the
`user_commands` group. `prepare_user_commands` installs the two scripted
commands described by the trait, returns the session reference and admission
fields, configures interactivity, optionally adds a second provider declaring the same
command name, and sets up the host's reply interface before admission. `user_command_observation` records actual provider
invocations, complete model requests and host-menu registrations outside runner
state. Neither hook may synthesize management replies.

The named cases cover transcript/model-input isolation through a follow-up
turn, once-only forwarding with verbatim input, `interactive: false`, admission
provider collisions for the same command name, ordered late progress,
owner-only access and a host that cannot keep user-command replies out of
conversation history. `user_commands_outcome_unknown` kills the runner after
forwarding but before storing the reply, then checks that restart replays
`outcome_unknown` through both invocation and status without a second provider
call. Callers must tell the person it may have run and must not retry it
automatically. A menu is not required: even with no safe menu,
user-command requests must still reach the provider. The asynchronous fixture must settle within 30 seconds;
the protocol's normal one-hour pull limit is a contract rule, not a one-hour
self-test. Each case has a deliberately faulty fake that fails it by name.

## Mid-session changes

The `session_change` cases need `LlmRunnerSubject::refresh_plan`, which
builds a plan for `session.refresh` of a session admitted with
`send_fields`' first plan, and `model_requests`, which shows the tools each
model request offered. `RefreshVariant::ChangedTools` is a plan the runner
accepts whose tool set differs from the first plan's; its `tools` are the
names a request offers once it applies. `OutsideHostSet` and
`ChangedDefinition` name a tool the host never registered at start and keep
a tool's name with another description or input schema; their `tools` are
the names `tool_outside_host_set` must list. A host that does not fix its
tool set at start answers `None` for both, and the host-set case is then
inapplicable by name. Each case reads the rungs the session states in
`session.baseline`: the next-run cases need `immediately`, and the rung case a
rung other than `immediately`. The flush case prefers such a rung, so that only
the flush makes the change apply at the next run; when the session states only
`immediately`, it still checks the next-run rule and names the flush's own
effect unchecked. The cases check that a
step of a run that was running when the change arrived offers exactly the
tools the run started with.


## Output references and host staging

The `output_refs_*` positive cases discover the runner's live `output_refs`
declaration. They are **N/A**, not missing-capability skips, when it is absent.
The exception is `output_refs_unsupported_plan`: it binds exactly runners that
**do not** declare the group, and is N/A for advertisers. A non-advertising
0.5.5 runner satisfies that refusal with one call to `output_refs::check_plan`
(or the request's `check_output_refs`) before any durable write, on every
plan-carrying operation: `session.send`, `session.refresh` and `session.adopt`.
Both explicit true and explicit false must be refused with
`invalid_params {field: "plan.items[].output_refs"}` and no state change.

Advertising adapters implement `output_refs_fields` with a tag-rendering plan
and a scripted tool provider whose slots are `data: token` and
`program: source:rich-ui/1`. The suite uses the published syntax crate's two
sample sets verbatim and observes sets and snapshots over management routes.
The held-result and renumbering hooks write native fixtures, never saved
reference records or synthetic management replies. The runner's real storage
and resolution path must produce the records. The compaction and rewind cases
use the existing host-compaction and revert hooks.

`send_staged(session, send_id)` waits until the request-rendering component
holds an accepted send. Its default returns at once for a runner owning its
loop. The first-possible-mode case waits for staging before releasing its
scripted tool result. `hold_send_staging` lets a host-side adapter delay staging
for the separate already-sent-result case; it returns false only where
acceptance and staging coincide, making that scenario N/A. Releasing a staging
hold does not deliver the send or render a boundary.

`tags_tool_result_blocks_share_one_number` uses the defaulted
`numbering_tool_result(marker, blocks)` hook to script mixed text/media, empty
and structured-only tool results. Preserve empty text blocks. Without
`run_tool_calls`, keep the marker in metadata, not an extra text block. With
`run_tool_calls`, marker-free results are located by `result_ordinal` and
`tool_result_part(call, result)`, which returns the stored part's `read_parts`
key. Its default covers common call-id fields and single-part separate result
entries; override it for other native schemas. Unsupported shapes are named
unchecked; block placement and image/media labels are checked when
`rendered_model_input` is declared. Reads always check whole-result numbering
and the next part.
