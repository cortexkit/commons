# cortexkit-role-llm-runner-conformance

The conformance suite for the `llm-runner/v1` role
(`crates/cortexkit-role-llm-runner/CONTRACT.md`). A runner adds it as a
dev-dependency and runs it in its own CI against its real module over its
real management route. It is never run against a double; the in-process
fake under `tests/fake/` exists only to test the suite itself.

## What a runner supplies

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
  a scripted model. The suite writes each session's model as a `Script`:
  assistant turns made of text parts, reasoning parts and tool calls with
  their arguments and scripted results. The subject backs it with its own
  mock provider; the suite never speaks a provider's wire protocol. The
  subject's model answers a request with the turn whose index is the number
  of assistant messages in the request's history, and its scripted tool
  provider (serving `SCRIPTED_TOOL`) answers each call with its scripted
  result, counts invocations, and can hold a call until released.

## Verdict

A case whose requirements the subject does not declare is skipped, naming
what is missing, and is never passed. A run with skips and no failure is
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
| `role_describe_shape`, `role_describe_cacheable`, `role_describe_groups_complete`, `extra_op_still_admitted` | — |
| `describe_states_max_bytes` | `transcript_reads` |
| `baseline_owner_only`, `baseline_matches_admission_reply` | `queue` |
| `tail_read_is_newest_page`, `range_read_stops_by_count`, `range_read_stops_by_bytes`, `oversize_message_alone_on_its_page`, `after_mid_reads_strictly_after`, `after_mid_unknown_mid_refused`, `read_other_lineage_refused`, `lineage_id_on_every_page`, `head_has_no_bodies` | `transcript_reads`, `queue` |
| `unknown_read_field_refused`, `never_written_session_empty_page`, `never_written_session_empty_head` | `transcript_reads` |
| `run_result_completed_final_text`, `run_result_text_parts_joined_unchanged`, `run_result_completed_no_text_is_empty` | `run_ops`, `queue` |
| `run_result_interrupted_not_cancelled` | `run_ops`, `queue`, `transcript_reads`, kill point `DispatchIntent` |
| `dispatched_to_per_call`, `sibling_calls_distinct_call_keys`, `recurring_model_id_distinct_call_keys` | `dispatch_attribution`, `queue`, `transcript_reads` |
| `indeterminate_until_closed` | `dispatch_attribution`, `queue`, `transcript_reads`, `hold_tool_calls` |
| `subscribe_from_head_no_gap_no_duplicate` | `streaming`, `queue`, `transcript_reads` |
| `send_id_retry_same_answer`, `send_id_reuse_refused_naming_field`, `unknown_delivery_refused` | `queue`, `transcript_reads` |
| `delivery_change_refused` | `queue`, `transcript_reads`, and `steer` or `interrupt` |
| `undeclared_delivery_refused` | `queue`, `transcript_reads`, and `steer` or `interrupt` left undeclared |
| `crash_at_Admitted`, `crash_at_SendRecorded`, `crash_at_StepRecorded`, `crash_at_ToolResultRecorded`, `crash_at_Terminal` | `queue`, `transcript_reads`, the kill point |
| `crash_at_DispatchIntent` | `queue`, `transcript_reads`, `dispatch_attribution`, the kill point |

`CASES` in `src/report.rs` states what each case checks.

## Where the suite narrows a check

The role contract marks some rules open (not yet defined), and some of its
guarantees cannot be observed on the wire. `NARROWINGS` in `src/report.rs` lists every such choice, and
every report prints them. In short:

- The fetch plan's shape is not pinned (§10), so the subject builds a
  session's first-send fields and the suite never inspects a plan.
- The suite writes sessions with `session.send` (absent `delivery`, meaning
  `queue`), and waits for runs through `session.head`, so cases that need a
  written session require `queue`, and cases that wait require
  `transcript_reads` (run-ops cases wait through `run.result`).
- A read's byte measure is not pinned, so byte-cap cases use a one-byte cap
  or a message far above the cap.
- Message bodies are the runner's schema, so the suite finds scripted
  content by unique marker strings, and finds an `outcome_unknown` close
  only as that string in some message.
- The subscription handoff is checked by comparing replays on an idle
  session, not the live race.
- Two §14 crash guarantees are not checked: that a read naming the old
  lineage after a lineage change is refused (nothing in this subset changes
  a lineage), and that each resume writes one informational record (where
  it sits is the runner's schema). "Replay, not re-invocation" is checked
  only for the model and the tool, since hooks and compaction are not in
  this subset; the indeterminate
  window between a restart and the close is not observed; whether a run cut
  at `StepRecorded` or `ToolResultRecorded` completes or ends interrupted is
  not pinned, so only "one terminal state, not cancelled" is checked there.
- `run_result_interrupted_not_cancelled` reads the run cut in the
  `DispatchIntent` scenario, because each point is cut once per run.
- `role_describe_groups_complete` fails a runner whose `role.describe`
  declares a group the subject does not, because the suite would skip that
  group's cases while the wire claims it.

## Not yet implemented

These cases, planned in §15 of the role contract, belong to a later
subset:

- compaction: `compaction_ready_reasks_after_wait`,
  `compaction_ready_stale_request_ignored`,
  `compaction_ready_other_caller_refused`, `compaction_cursor_never_skips`,
  `compaction_item_without_group_refused_writes_nothing`, and
  `crash_at_CompactionApplied`;
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
- also `baseline_not_yet_before_first_request` and
  `run_attribution_per_message`.
