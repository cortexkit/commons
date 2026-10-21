//! The suite's cases and what a run reports.

use cortexkit_role_harness::KillReport;
use cortexkit_role_llm_runner::points;

use crate::subject::Capability;

/// One conformance case: its stable name, what it requires, and what it
/// checks.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct CaseSpec {
    pub name: &'static str,
    /// Every one of these must be declared.
    pub requires: &'static [Capability],
    /// At least one of these must be declared, when not empty.
    pub requires_any: &'static [Capability],
    /// At least one of these must be left undeclared, when not empty. A
    /// runner adapter that declares all of them cannot be asked the question, so
    /// the case is [`CaseOutcome::Inapplicable`].
    pub requires_undeclared_any: &'static [Capability],
    pub checks: &'static str,
}

use Capability::{
    Compaction, DispatchAttribution, HoldToolCalls, Interrupt, KillAt, MaxSteps, ModelView, Queue,
    Retention, Retract, RunCancel, RunOps, Steer, Streaming, TranscriptReads, TurnDirectives,
};

const fn case(
    name: &'static str,
    requires: &'static [Capability],
    checks: &'static str,
) -> CaseSpec {
    CaseSpec {
        name,
        requires,
        requires_any: &[],
        requires_undeclared_any: &[],
        checks,
    }
}

const READS: &[Capability] = &[TranscriptReads, Queue];
const RUNS: &[Capability] = &[RunOps, Queue];
const DISPATCH: &[Capability] = &[DispatchAttribution, Queue, TranscriptReads];
const SENDS: &[Capability] = &[Queue, TranscriptReads];
const RETAINED: &[Capability] = &[Retention, Queue, TranscriptReads, RunOps];
const COMPACTED: &[Capability] = &[Compaction, ModelView, Queue, TranscriptReads, RunOps];
const CANCEL: &[Capability] = &[Capability::RunCancel, Queue, TranscriptReads, RunOps];
const INPUT_CAP: &[Capability] = &[Capability::InputTokenCap, Queue, TranscriptReads, RunOps];
const STRUCTURED: &[Capability] = &[Capability::StructuredOutput, Queue, TranscriptReads, RunOps];
const ADOPT: &[Capability] = &[Capability::Adopt, TranscriptReads];
const TAGS: &[Capability] = &[Capability::Tags, Queue, TranscriptReads];
const TOOL_CALLS: &[Capability] = &[Capability::RunToolCalls, Queue, TranscriptReads];
const USAGE: &[Capability] = &[Capability::RunUsage, Queue, RunOps];
const ORDER: &[Capability] = &[Capability::DeliveryOrder, Queue, TranscriptReads];
const ORDER_STEER: &[Capability] = &[Capability::DeliveryOrder, Queue, TranscriptReads, Steer];
const STRUCTURED_RESTART: &[Capability] = &[
    Capability::StructuredOutput,
    Queue,
    TranscriptReads,
    RunOps,
    KillAt(points::TOOL_RESULT_RECORDED),
];

/// Every case, in the order the runner runs them.
pub const CASES: &[CaseSpec] = &[
    CaseSpec { requires_any: &[], requires_undeclared_any: &[], name: "user_commands_history_isolation", requires: &[Capability::UserCommands, Capability::Queue, Capability::RunOps, Capability::TranscriptReads], checks: "replies, progress and results never enter reads or actual model inputs" },
    CaseSpec { requires_any: &[], requires_undeclared_any: &[], name: "user_commands_forward_once", requires: &[Capability::UserCommands, Capability::Queue, Capability::RunOps], checks: "retry of an invocation returns the stored reply and reaches the provider once" },
    CaseSpec { requires_any: &[], requires_undeclared_any: &[], name: "user_commands_no_person", requires: &[Capability::UserCommands, Capability::Queue, Capability::RunOps], checks: "interactive false lists no commands and refuses no_person" },
    CaseSpec { requires_any: &[], requires_undeclared_any: &[], name: "user_commands_collision", requires: &[Capability::UserCommands, Capability::Queue], checks: "duplicate names at admission refuse user_command_collision with declaring providers" },
    CaseSpec { requires_any: &[], requires_undeclared_any: &[], name: "user_commands_progress_order", requires: &[Capability::UserCommands, Capability::Queue, Capability::RunOps], checks: "status keeps progress in arrival order and exposes the terminal result" },
    CaseSpec { requires_any: &[], requires_undeclared_any: &[], name: "user_commands_unsafe_host_menu", requires: &[Capability::UserCommands, Capability::Queue, Capability::RunOps], checks: "no host menu entries where replies cannot stay outside history; relay remains usable" },
    CaseSpec { requires_any: &[], requires_undeclared_any: &[], name: "user_commands_owner_only", requires: &[Capability::UserCommands, Capability::Queue, Capability::RunOps], checks: "both relay ops refuse a non-owner" },
    case("interrupt_reason_shutdown_durable", &[Capability::InterruptReason, Queue, TranscriptReads, RunOps, HoldToolCalls],
        "an orderly handle stop during a held run records interrupted reason runner_shutdown, preserved on restart across result, status and last_run_state"),
    case("run_tool_calls_positions_identify_siblings", TOOL_CALLS,
        "two calls in one assistant message carry positions 0 and 1 at the same requesting ordinal, independent of text/reasoning parts and list order"),
    case("run_tool_calls_result_ordinal_tracks_storage", &[Capability::RunToolCalls, Queue, TranscriptReads, HoldToolCalls],
        "an open call has no result_ordinal; after durable completion result_ordinal >= ordinal locates the stored result"),
    case("turn_directives_in", &[TurnDirectives, Queue, RunOps],
        "completed final text ending in a self-closing wake tag reports its duration on run.result and run.status where declared, without editing text"),
    case("turn_directives_note", &[TurnDirectives, Queue, RunOps],
        "completed final text ending in a wake tag with a note reports its duration and note on both pinned surfaces where served"),
    case("turn_directives_off", &[TurnDirectives, Queue, RunOps],
        "completed final text ending in wake off reports off:true"),
    case("turn_directives_max_steps_reports_none", &[TurnDirectives, Queue, RunOps, MaxSteps],
        "a max_steps run with a trailing wake tag reports no directives"),
    case("turn_directives_cancelled_reports_none", &[TurnDirectives, Queue, RunOps, RunCancel, HoldToolCalls],
        "a held non-terminal run and its cancelled terminal with a trailing wake tag report no directives"),
    case("turn_directives_not_requested_reports_none", &[TurnDirectives, Queue, RunOps],
        "a completed run not admitted with directives reports none even with a trailing tag"),
    case("turn_directives_unknown_refused", &[TurnDirectives, Queue],
        "unknown, non-array and duplicate directives are refused invalid_params naming directives"),
    case("max_steps_seals_pending_calls", &[Capability::MaxSteps, Queue, TranscriptReads, RunOps],
        "a tool-calling model ends at the response limit in max_steps, with every pending call sealed before the terminal is observable and none dispatched"),
    case("max_output_tokens_caps_response", &[Capability::MaxOutputTokens, Capability::RunUsage, Queue, TranscriptReads, RunOps],
        "the dispatched per-response output maximum and the single response's reported output usage never exceed the requested cap"),
    case("no_tools_offered_to_model", &[Capability::NoTools, Queue, TranscriptReads],
        "tools [] offers no tools to the model, including the tools the adapter normally supplies"),
    case("no_tools_later_plan_send_refused", &[Capability::NoTools, Queue, TranscriptReads],
        "when the adapter can create a fetch-plan session, every later tools member is refused invalid_params naming tools without writes"),
    case("read_stable_across_host_metadata_change", &[Queue, TranscriptReads, Capability::HostMetadataChange],
        "a re-read after the host changes the session's metadata, under the same lineage, answers the same entries byte for byte"),
    case("tags_on_every_read_entry", TAGS,
        "every read entry carries part rows with at least one tagged part; every tag is a well-formed §N§ used once in the session and every part key once in its message; a second read answers the same rows"),
    case("tags_never_reused_across_runs", TAGS,
        "a later run leaves the earlier messages' tags as they were and assigns only numbers above the earlier highest"),
    case("tags_exist_whether_or_not_rendered", TAGS,
        "sessions whose plan sets render_tags true and leaves it absent both carry tags on every entry"),
    case("tags_resolve_returns_stored_content", TAGS,
        "session.resolve of a prompt's and an answer's tags answers the exact stored text naming the entry's ordinal, mid and part, and a character range counted in Unicode scalar values answers exactly that text"),
    case("tags_resolve_unknown_tag_refused", TAGS,
        "a well-formed tag the session never assigned is refused unknown_tag, and malformed tags are refused invalid_params naming tag"),
    case("tags_resolve_range_refused", TAGS,
        "a range past the content or starting after its end is refused invalid_params naming range; an empty range at the end answers empty content"),
    case("tags_model_written_tags_stored_verbatim", TAGS,
        "a model-written §N§ in a text part is stored and resolved as written, and one in a tool call's arguments reaches the tool exactly once, as written"),
    case("tags_resolve_left_lineage_refused", &[Capability::Tags, Queue, TranscriptReads, DispatchAttribution, KillAt(points::STEP_RECORDED)],
        "after the crash_at_StepRecorded seal drops the never-sent call or its step onto a new lineage, every tag number below the new lineage's highest that no part it reads carries (the dropped part's among them) is refused unknown_tag, and the cut run's prompt resolves with its ordinal and mid in the new lineage; inapplicable when the runner resumes the run, keeps the call, or dropped no tagged part"),
    case("run_tool_calls_listed_in_dispatch_order", TOOL_CALLS,
        "run.tool_calls, followed one call a page through next, lists a run's three calls in dispatch order with their names, ids, exact args, succeeded outcome, dispatch module, stored result size and carrying ordinal"),
    case("run_tool_calls_unknown_run_refused", TOOL_CALLS,
        "an unknown run_id is refused unknown_run; a run without calls lists none; a misspelled cursor and limit 0 are refused invalid_params by name"),
    case("run_tool_calls_indeterminate_until_result", &[Capability::RunToolCalls, Queue, TranscriptReads, HoldToolCalls],
        "a held call lists as indeterminate with no stored result, then as succeeded with one once released"),
    case("run_usage_counts_reported_steps", USAGE,
        "after a two-step run whose scripted model reports input, cache-read, cache-write and output counts but no reasoning, run.result carries usage with steps 2, no unreported step, exactly the scripted totals, by_model summing to them, and reasoning_tokens absent rather than 0; run.status, where served, carries the same usage"),
    case("run_context_ceiling_wait_is_not_a_stall", &[Queue, TranscriptReads, Capability::HoldContextCeiling],
        "a run held at the context ceiling reads active with reason context_ceiling_wait on session.head (and on run.status with a deadline_ms, where served); with a view it completes, without one it ends context_ceiling with cause view_timeout and no error"),
    case("delivery_order_never_starts_a_run", ORDER_STEER,
        "a list without idle_start on an idle session answers pending and changes nothing, then rides the owner's next turn as a steer, delivered once"),
    case("delivery_order_declared_modes_respected", ORDER,
        "when role.describe advertises delivery_order_modes, every ordered list of only undeclared modes is refused delivery_unsupported with detail.delivery and no writes; every list containing a listed mode is accepted"),
    case(
        "delivery_order_first_possible_mode",
        &[Capability::DeliveryOrder, Queue, TranscriptReads, HoldToolCalls],
        "on an idle session [tool_result, steer, idle_start] starts a run through idle_start; during a held call [tool_result, idle_start] goes out with that call's result, recording its call_key",
    ),
    CaseSpec {
        name: "delivery_order_undeclared_mode_refused",
        requires: &[Capability::DeliveryOrder, Queue, TranscriptReads, HoldToolCalls],
        requires_any: &[],
        requires_undeclared_any: &[Steer],
        checks: "on a runner without steer, a [steer] list is refused delivery_unsupported with the same list in detail.delivery and writes nothing; [steer, idle_start] is accepted while a run is active, does not deliver the undeclared steer, then starts at idle_start",
    },
    case(
        "delivery_order_retract_pending_tool_result",
        &[Retract, Capability::DeliveryOrder, Queue, TranscriptReads, HoldToolCalls],
        "while a held tool call leaves [tool_result] impossible, its pending send has a submission_id; session.retract answers retracted then not_pending, and after the call is released the send is never delivered",
    ),
    case("delivery_order_dedupe_supersedes_queued", ORDER_STEER,
        "a second queued send with the same dedupe_key replaces the first, whose re-send answers superseded and which is never delivered"),
    case("delivery_order_expired_never_delivered", ORDER_STEER,
        "a send past its expires_at_ms is never delivered, and its re-send answers expired"),
    case("delivery_order_members_without_list_refused", ORDER,
        "template with a string delivery, dedupe_key with no delivery and expires_at_ms with a string delivery are each refused invalid_params naming the member, writing nothing"),
    CaseSpec { name: "delivery_order_list_refused_without_group", requires: &[Queue, TranscriptReads], requires_any: &[], requires_undeclared_any: &[Capability::DeliveryOrder],
        checks: "on a runner without delivery_order, a list in delivery is refused invalid_params naming delivery and writes nothing, so it never goes out as a legacy queue" },
    case("delivery_order_unnamed_sender_refused", ORDER_STEER,
        "a module the plan does not name is refused scope_owner_mismatch, even naming the plan sender in a request field; the plan sender without the idle_start grant is refused listing idle_start; its steer is delivered once"),
    case("delivery_order_template_renders_as_system", ORDER,
        "a template send carrying an empty prompt is refused invalid_params naming template and writes nothing; a plan sender's template send with valid slots and no prompt goes out once with the rendered text, as turn_system where the adapter's route supports it and as a steer otherwise"),
    case("delivery_order_bad_slot_refused", ORDER,
        "a template send whose percentage slot is 101 is refused invalid_params naming template, writes nothing, and nothing goes out as turn_system"),
    case(
        "delivery_order_steer_receipt_durable_across_restart",
        &[Capability::DeliveryOrder, Queue, TranscriptReads, RunOps, HoldToolCalls, Steer],
        "on its own state root, a [steer] list sent while a run waits on a held call answers pending with a submission_id and is delivered once as a steer when the call is released; a re-send of its send_id answers delivered step with a ref and run_id the run it joined, and after a harness restart answers the same receipt and run_id"),
    case("send_status_answers_as_a_resend_after_restart", &[Capability::SendStatus, Capability::DeliveryOrder, Queue, TranscriptReads, RunOps, HoldToolCalls, Steer],
        "the same delivered steer: send.status answers exactly what its re-send answers before the restart, and after a harness restart answers delivered step with the joined run's run_id, never pending, and the same as a re-send then"),
    case("send_status_unknown_send_refused", &[Capability::SendStatus, Queue, TranscriptReads],
        "send.status of a settled send answers exactly what its re-send answers; one of a send_id the session has no record of is refused unknown_send, and one carrying an unknown field is refused invalid_params naming it"),
    case("adopt_binds_owner_without_a_run", ADOPT,
        "on a session the host started, with no message and after the person's first turn, session.adopt answers owner_bound with the plan's identity and session.head's last_ordinal, starts no run and writes no message; afterwards session.baseline answers the adopting owner ready and refuses another caller scope_owner_mismatch"),
    case("adopt_exact_repeat_replays_reply", ADOPT,
        "an exact repeat of session.adopt is answered with the original reply and writes nothing"),
    case("adopt_send_id_reuse_names_plan", ADOPT,
        "the same send_id with another plan is refused send_id_reuse naming plan, writing nothing; the original request still replays its reply"),
    case("adopt_plan_drift_refused", ADOPT,
        "a plan other than the one the host applied is refused plan_drift with frozen and sent identities and no plan content, binding no owner and recording no send_id, so the owner then adopts the host's plan under that send_id with applied_plan_digest equal to frozen; a session the host never started answers plan_drift without frozen"),
    case("adopt_during_active_run", &[Capability::Adopt, TranscriptReads, HoldToolCalls],
        "session.adopt while the person's run waits on a held tool call succeeds, writes nothing and leaves the run in progress; released, the same run completes and its tool runs once"),
    case("adopt_other_owner_refused", ADOPT,
        "once the owner has adopted, another caller's session.adopt, even an exact repeat of the owner's request, is refused scope_owner_mismatch, and session.baseline still answers only the owner"),
    case("adopt_same_owner_readopts", ADOPT,
        "the owner adopting again with the same plan and a new send_id succeeds with the same applied_plan_digest and writes nothing"),
    case("adopt_then_owner_sends", &[Capability::Adopt, Queue, TranscriptReads],
        "after adoption the owner's session.send starts a run that writes its prompt and answer once, and another caller's send is refused scope_owner_mismatch"),
    case("structured_output_final_text_enforced_passed", STRUCTURED,
        "final_text on a tool-free run whose route reports support: the request carries the schema for the final text and no submit_result tool, and the completed run reports enforced and passed with the parsed value"),
    case("structured_output_final_text_with_tools_refused", STRUCTURED,
        "final_text on a run with tools is refused invalid_params naming structured_output.mode, writing nothing and starting no model request"),
    case("structured_output_submit_tool_ends_run", STRUCTURED,
        "submit_tool offers submit_result with the caller's schema in the first request; calling it ends the run completed after that step, the value passes, and the step's other calls are listed in skipped_siblings with never-executed results"),
    case("structured_output_reserved_name_refused", STRUCTURED,
        "an admitted tool named submit_result is refused invalid_params naming structured_output, writing nothing"),
    case("structured_output_absent_without_submit", STRUCTURED,
        "a submit_tool run whose model finishes without submit_result completes with validation absent, no value and empty skipped_siblings; an unstated catalog gives best_effort catalog_unknown"),
    case("structured_output_failed_validation_completes", STRUCTURED,
        "a submitted value that fails the schema leaves the run completed with validation failed, the value and bounded errors; a route without support gives best_effort route_unsupported"),
    case("structured_output_frozen_across_restart", STRUCTURED_RESTART,
        "a kill after a step's tool result and a restart leave the next model request of the run carrying the same tools and byte-identical schema"),
    case("structured_output_not_inherited", STRUCTURED,
        "a send without structured_output after a run with it starts a run whose request carries no schema and no submit_result tool, and whose result has no structured_output"),
    case("structured_output_external_ref_refused", STRUCTURED,
        "a schema with a $ref or $dynamicRef outside its document is refused invalid_params naming structured_output.schema at admission, writing nothing and starting no model request"),
    case("structured_output_sibling_never_dispatched", STRUCTURED,
        "a sibling emitted before submit_result in the same step is never invoked at the scripted tool provider, gets a never-executed skipped_after_submit result, and is listed; a second submit_result is a skipped sibling and the first one counts"),
    case("input_token_cap_at_limit", INPUT_CAP,
        "input plus both cache counts reach exactly the cap, output is excluded, and an omitted cap starts no model step in the next episode"),
    case("input_token_cap_crossing_tools", INPUT_CAP,
        "a step may overshoot without estimation, but none of its sibling calls dispatch and every call has a never-executed budget_exhausted result"),
    case("input_token_cap_restart_between_episodes", INPUT_CAP,
        "the frozen cap and measured sum survive a harness restart between episodes"),
    case("input_token_cap_raise_refused", INPUT_CAP,
        "a raise is refused invalid_params naming budget.max_input_tokens without writes; a lower cap takes effect before the next step"),
    case("input_token_cap_closed_members", INPUT_CAP,
        "unknown budget and nudge-element members are refused invalid_params naming their paths without writes"),
    case("input_token_cap_usage_unreported", INPUT_CAP,
        "unreported completed-step input ends budget_exhausted usage_unreported with spent excluding that step, without dispatching its calls"),
    case("run_cancel_step_boundary_finishes_tool_batch",
        &[Capability::RunCancel, Queue, TranscriptReads, RunOps, HoldToolCalls],
        "when describe lists step_boundary, cancellation during a held model step records the whole tool batch, accepts an idempotent second request, starts no further model step and ends cancelled"),
    case("run_cancel_unlisted_at_refused", CANCEL,
        "when describe lists step_boundary, an unlisted at is refused invalid_params naming at, without writes or changing the held run"),
    case("run_cancel_step_boundary_escalates_to_now", CANCEL,
        "when describe lists step_boundary, a second cancellation at now ends the run without waiting for the held model answer"),
    case("compaction_setup_durable_once",
        &[Compaction, ModelView, Queue, TranscriptReads, RunOps, KillAt(points::FOLD_RECORDED)],
        "Setup is durable before the first model call and never re-invoked after its recorded answer survives a crash"),
    case("model_view_half_open_ranges_and_travel", COMPACTED,
        "Setup [0,0) insertions precede message zero and travel with it over count/byte caps, are not repeated on the next page, and the held model request carries that canonical view"),
    case("compaction_unavailable_distinct_from_refuse", &[Compaction, Queue, TranscriptReads, RunOps],
        "Setup unavailability and a Setup REFUSE end error distinctly, send nothing to the model and leave the session usable"),
    case(
        "role_describe_shape",
        &[],
        "role.describe passes check_describe, lists the llm-runner/v1 major with a known stability, and names its session-capability source as admission or baseline",
    ),
    case(
        "role_describe_cacheable",
        &[],
        "role.describe answers identically twice on one route and on the routes of two different sessions",
    ),
    case(
        "role_describe_groups_complete",
        &[],
        "every group role.describe declares lists all its ops, and the groups it declares are exactly the groups the subject declares, so no group is claimed on the wire while its cases are skipped",
    ),
    case(
        "extra_op_still_admitted",
        &[],
        "the live role.describe answer, with an extra op, an extra major, an unknown capability and an unknown member added, is still admitted by check_describe with the same groups",
    ),
    case(
        "describe_states_max_bytes",
        &[TranscriptReads],
        "role.describe states max_bytes with a non-zero default no larger than its maximum",
    ),
    case(
        "baseline_owner_only",
        &[Queue],
        "after the owner's first send, session.baseline answers the owner ready with a baseline and refuses another caller scope_owner_mismatch",
    ),
    case(
        "baseline_matches_admission_reply",
        &[Queue],
        "on a runner whose session capabilities come from admission, the first send's reply carries a baseline equal to what session.baseline answers",
    ),
    case(
        "tail_read_is_newest_page",
        READS,
        "on a session longer than one page, a read with no cursor answers the newest messages, ending at session.head's last_ordinal, the same messages a range read of them answers",
    ),
    case(
        "range_read_stops_by_count",
        READS,
        "a range read with a limit answers that many messages and a next_from_ordinal from which the following message is read",
    ),
    case(
        "range_read_stops_by_bytes",
        READS,
        "a range read with a one-byte cap and a limit above the transcript stops after one message, untruncated, with next_from_ordinal set",
    ),
    case(
        "oversize_message_alone_on_its_page",
        READS,
        "a message larger than the read's byte cap comes alone on its page, whole, with next_from_ordinal set because more follow",
    ),
    case(
        "after_mid_reads_strictly_after",
        READS,
        "a read after a message's mid in the session's lineage answers the messages that follow it, starting with the next one",
    ),
    case(
        "after_mid_unknown_mid_refused",
        READS,
        "a read after a mid the lineage does not have is refused unknown_mid, never answered from the tail",
    ),
    case(
        "read_other_lineage_refused",
        READS,
        "a tail, range or after read naming another lineage is refused lineage_changed",
    ),
    case(
        "lineage_id_on_every_page",
        READS,
        "every page of a written session, in every read mode, carries the session's lineage_id, the one session.head reports",
    ),
    case(
        "unknown_read_field_refused",
        &[TranscriptReads],
        "session.read and session.head refuse an unknown request field as invalid_params naming it",
    ),
    case(
        "never_written_session_empty_page",
        &[TranscriptReads],
        "a read of a session never written answers an empty page with no lineage_id and no next_from_ordinal",
    ),
    case(
        "never_written_session_empty_head",
        &[TranscriptReads],
        "session.head of a session never written answers with every member absent",
    ),
    case(
        "head_has_no_bodies",
        READS,
        "session.head of a written session decodes as consistent head metadata matching the transcript and carries no message body",
    ),
    case(
        "run_result_completed_final_text",
        RUNS,
        "a completed run answers run.result completed with the final assistant message's text, not an earlier turn's",
    ),
    case(
        "run_result_text_parts_joined_unchanged",
        RUNS,
        "a final message's text parts, a JSON value split across them with reasoning between, are joined with nothing inserted and the reasoning left out",
    ),
    case(
        "run_result_completed_no_text_is_empty",
        RUNS,
        "a completed run whose final message has only reasoning answers final_message with text \"\", never without final_message",
    ),
    case(
        "run_attribution_per_message",
        &[RunOps, Queue, TranscriptReads, HoldToolCalls],
        "every attributable message names its run and episode; only the last message of a durably ended run is final, no running message is final, and unknown-origin records omit run (requires the native fixture hook or an explicit impossibility declaration)",
    ),
    case(
        "run_result_interrupted_not_cancelled",
        &[RunOps, Queue, TranscriptReads, KillAt(points::DISPATCH_INTENT)],
        "a run cut by a kill at DispatchIntent answers run.result interrupted, never cancelled",
    ),
    case(
        "dispatched_to_per_call",
        DISPATCH,
        "every scripted tool call has one tool_calls entry naming the scripted tool provider as dispatched_to, with a well-formed call_key, not indeterminate once the run ended",
    ),
    case(
        "indeterminate_until_closed",
        &[DispatchAttribution, Queue, TranscriptReads, HoldToolCalls],
        "a call the tool provider holds is read as indeterminate with its call_key and dispatched_to, and is no longer indeterminate once its result is in",
    ),
    case(
        "sibling_calls_distinct_call_keys",
        DISPATCH,
        "two tool calls from one assistant step get distinct call_keys",
    ),
    case(
        "recurring_model_id_distinct_call_keys",
        DISPATCH,
        "one model tool_call_id used in two steps gets a distinct call_key each time",
    ),
    case(
        "subscribe_from_head_no_gap_no_duplicate",
        &[Streaming, Queue, TranscriptReads],
        "a subscription from a page's head answers exactly the durable events a replay from the start holds after that page's snapshot, every one with a cursor",
    ),
    case(
        "send_id_retry_same_answer",
        &[],
        "on a runner that declares queue or steer, and without reading the transcript or waiting: a send retried twice at once with the same send_id and payload names the run_id and submission_id the send was answered with, and its delivered receipt moves only forward (absent or pending to step, turn or unknown, which are final, ref included); the send's state may still move between the retries, so they are not compared whole",
    ),
    case(
        "send_id_retry_settled_same_answer",
        &[RunOps],
        "on a runner that declares queue or steer, and without reading the transcript: once the send's run has ended, seen through run.result, a send retried twice with the same send_id and payload gets the same answer twice apart from delivered, names the run_id and submission_id the send was answered with, and its delivered receipt moves only forward; inapplicable when the send's reply names no run_id",
    ),
    case(
        "send_id_retry_written_once",
        &[Queue],
        "on a runner that declares transcript_reads, a send retried twice with the same send_id and payload is answered with the run it started, starts no other run, and its prompt is written once",
    ),
    case(
        "send_id_reuse_refused_naming_field",
        &[],
        "on a runner that declares queue or steer, and without reading the transcript: a send_id reused with another prompt is refused send_id_reuse naming prompt",
    ),
    case(
        "send_id_reuse_writes_nothing",
        &[Queue],
        "on a runner that declares transcript_reads, a send_id reused with another prompt writes nothing, however it is answered",
    ),
    CaseSpec {
        name: "delivery_change_refused",
        requires: &[],
        requires_any: &[Steer, Interrupt],
        requires_undeclared_any: &[],
        checks: "on a runner that declares queue or steer, and without reading the transcript: a send_id reused with another declared delivery mode is refused send_id_reuse naming delivery",
    },
    CaseSpec {
        name: "delivery_change_writes_nothing",
        requires: &[Queue],
        requires_any: &[Steer, Interrupt],
        requires_undeclared_any: &[],
        checks: "on a runner that declares transcript_reads, a send_id reused with another declared delivery mode writes nothing, however it is answered",
    },
    case(
        "unknown_delivery_refused",
        SENDS,
        "a send with a delivery mode the role does not define is refused invalid_params naming delivery, and nothing is written",
    ),
    CaseSpec {
        name: "undeclared_delivery_refused",
        requires: SENDS,
        requires_any: &[],
        requires_undeclared_any: &[Steer, Interrupt],
        checks: "a send with a delivery mode the runner does not declare is refused delivery_unsupported naming the mode, and nothing is written",
    },
    case(
        "guaranteed_steer_never_pending_or_unknown",
        &[RunOps, Queue, TranscriptReads],
        "on a guaranteed runner with hold_tool_calls, a steer and its same-send_id re-send are answered while the held turn is still active, naming that run, with any receipt step or turn; after the run ends, a final re-send must carry step or turn, equal to every earlier receipt",
    ),
    CaseSpec {
        name: "steer_without_run_ops_reports_state_and_step",
        requires: &[Steer, Queue, TranscriptReads, HoldToolCalls],
        requires_any: &[],
        requires_undeclared_any: &[RunOps],
        checks: "a steer sent into an active turn answers with state active or finished, delivered.as step and a non-empty ref, without run_id; step is never a state",
    },
    case(
        "resend_steer_delivered_stable",
        SENDS,
        "a steer re-sent twice with the same send_id carries a delivered receipt that moves only forward: absent or pending to step, turn or unknown, which are final, ref included",
    ),
    case(
        "crash_at_Admitted",
        &[Queue, TranscriptReads, KillAt(points::ADMITTED)],
        "killed at Admitted and restarted: another session's messages keep their ordinals, mids and bodies; the retried first send is answered and its prompt written once; the run is not cancelled",
    ),
    case(
        "crash_at_SendRecorded",
        &[Queue, TranscriptReads, KillAt(points::SEND_RECORDED)],
        "killed at SendRecorded and restarted: the retried send is answered with the existing state, nothing is written twice, and the run is not cancelled",
    ),
    case(
        "crash_at_StepRecorded",
        &[Queue, TranscriptReads, DispatchAttribution, KillAt(points::STEP_RECORDED)],
        "killed at StepRecorded and restarted: observations show either one dispatch and a continued run with a result before the later model turn, or zero dispatches and an interrupted run sealed in one of two shapes: the call dropped on a new lineage whose old reads are refused lineage_changed, or the call kept on the same lineage with its never-executed result in the message right after it, every message read before the kill unchanged, and an owner follow-up that only appends; the call is not indeterminate and the step is not generated again",
    ),
    case(
        "crash_at_DispatchIntent",
        &[
            Queue,
            TranscriptReads,
            DispatchAttribution,
            KillAt(points::DISPATCH_INTENT),
        ],
        "killed at DispatchIntent and restarted: the call is never re-dispatched, is closed outcome_unknown and no longer indeterminate, the run ends interrupted without another model call, and call_keys read before the kill are unchanged",
    ),
    case(
        "crash_at_ToolResultRecorded",
        &[Queue, TranscriptReads, KillAt(points::TOOL_RESULT_RECORDED)],
        "killed at ToolResultRecorded and restarted: the tool is not called again and its result is written once",
    ),
    case(
        "crash_at_Terminal",
        &[Queue, TranscriptReads, KillAt(points::TERMINAL)],
        "killed at Terminal and restarted: the run keeps its one terminal state, completed, and nothing is written twice",
    ),
    case("retention_honoured", RETAINED,
        "after retention plus delete_within_ms and margin all role reads answer expired with the last-activity expiry time, never-written sessions still answer empty, deletion is complete and a fresh send starts a new lineage with its own retention"),
    case("retention_run_status_expired", RETAINED,
        "run.status, if advertised, answers the same expired refusal; otherwise not applicable"),
    case("retention_no_content_served", RETAINED,
        "prompt, tool-result and run-title markers disappear from every known read and every advertised export/list/read op, including subscription replay"),
    case("retention_shorten_continues_lineage", RETAINED,
        "shortening an idle unexpired session preserves its lineage and earlier messages, then expires after the new terminal plus shorter retention"),
    case("retention_equal_noop", RETAINED,
        "an equal retention is accepted without replacing the lineage or disabling expiry; a later absent value preserves the policy"),
    case("retention_lengthen_refused", RETAINED,
        "a later longer retention is refused invalid_params naming retention and writes nothing"),
    case("retention_late_opt_in_refused", RETAINED,
        "a session first sent without retention refuses any later retention, naming retention and writing nothing"),
    case("retention_zero_refused", RETAINED,
        "retention zero is refused invalid_params naming retention and writes nothing"),
    case("retention_above_max_refused", RETAINED,
        "retention above max_seconds is refused invalid_params naming retention with max_seconds in detail; inapplicable if max_seconds is u64::MAX"),
    CaseSpec { name: "retention_without_group_refused", requires: &[Queue], requires_any: &[], requires_undeclared_any: &[Retention],
        checks: "a runner without the retention group refuses retention invalid_params naming retention, never ignores it" },
    case("retention_active_run_never_expires", &[Retention, Queue, TranscriptReads, RunOps, HoldToolCalls],
        "a held active or paused non-terminal run remains readable beyond retention, and expires only after its terminal plus retention"),
    case("crash_at_RetentionTombstoned", &[Retention, Queue, TranscriptReads, RunOps, KillAt(points::RETENTION_TOMBSTONED)],
        "real process kill after the durable expiry tombstone and before deletion; restart answers expired and reports actual deletion finished"),
];

/// Where this suite skips or narrows a check because the role leaves the
/// rule open, or because nothing on the wire can observe it. Printed with
/// every report. Section numbers refer to `CONTRACT.md` in
/// `cortexkit-role-llm-runner`.
pub const NARROWINGS: &[&str] = &[
    "compaction cases check Setup only (its durability, its refusals and the model view it inserts); the runner half of compaction-provider 0.3.0 §3 (advance notices, views applied at step boundaries, rides, the ceiling, native markers) has no live case yet, and a later conformance release adds them",
    "structured-output cases cannot see a runner's commits, so 'in the same commit as the run's end' is checked as: once the run reads terminal, every skipped sibling already has exactly one never-executed result; schema subsets (schema_outside_subset, mode_not_enforceable) are runner-specific and are checked against the vectors only",
    "structured_output_frozen_across_restart cuts ToolResultRecorded with a crash driver of its own, on its own state root, because the suite's driver cuts each point once and the shared crash scenario owns that cut; its kill is not in the run's kill ledger and does not count toward the real-kill rule",
    "run_attribution_per_message uses transcript_reads and a held tool call to observe a non-terminal run, cancelling it over run.cancel only when run_cancel is declared; the native fixture hook supplies unknown-origin records independently of read replies, or explicitly declares only that rule inapplicable via CannotHoldUnattributable (NotSupported fails)",
    "retention cases use advance_retention_clock when declared; otherwise waits over 30 seconds are skipped by name and shorter waits use real time",
    "retention deletion completion is inspected by the subject in the runner's durable report and runner-held stores, never inferred from an expired read; non-role export/list/read request schemas and pagination are supplied by retention_probe_params, and every advertised extra op must be classified",
    "retention_run_status_expired is inapplicable if run.status is not advertised",
    "the fetch plan's shape is not pinned (CONTRACT §10), so the subject builds each session's first-send fields and the suite never inspects a plan or a baseline's items",
    "the tags cases check part rows, tags and resolve answers, never how a tag is rendered to the model; durability of tags across a restart is driven only by tags_resolve_left_lineage_refused, after the StepRecorded crash",
    "tags_resolve_left_lineage_refused cannot read the dropped part's tag, because the kill comes before any read of the cut step; tags are assigned in increasing order and the follow-up's are assigned after the drop, so it resolves every number below the new lineage's highest tag that no part read from that lineage carries (at most 64 of them), which the rule refuses whether the session assigned it under the old lineage or never",
    "the send_status and steer-receipt restart cases restart their own runner on their own state root with the harness restart hook, without a kill, so they do not count toward the real-kill rule; they run only when `run_ops` is declared, check a delivered steer's joined run, and the separate no-`run_ops` case checks a direct steer's state and step receipt",
    "delivery_order cases observe deliveries through read entries' delivered_as and the send's text; the rendered request is not inspected, so turn_system support is the adapter's word (turn_system_supported)",
    "the context-ceiling refusal of a send has no live case: the suite cannot bring a runner's context to its ceiling except through the adapter's held wait",
    "adopt cases start sessions through the subject's host-start hook, with plans the subject builds (host_plan); a plan's identity is the runner's, so the suite compares identities only with each other, never with a digest of its own",
    "the suite writes sessions with session.send (delivery absent, which means queue), so every case that needs a written session requires queue; a runner that admits sessions declares it (§9)",
    "the suite waits for a run to end through session.head's last_run_state, or run.result where only run_ops is required, so cases that wait require transcript_reads",
    "a read's byte measure is not pinned, so byte-cap cases use a cap below any message (one byte) or a message far above the cap, never a cap between message sizes",
    "message bodies are in the runner's own schema (§4.2), so the suite finds a scripted prompt, text part or tool result by a unique marker string inside a message body, and counts messages holding it",
    "where a tool result's reason sits is the runner's schema (§12.3), so crash_at_DispatchIntent only checks that some message holds the string outcome_unknown",
    "subscribe_from_head_no_gap_no_duplicate compares a replay from the head with a replay from start on an idle session; the live handoff race is not exercised",
    "lineage_changed for an old lineage is checked by retention_honoured, for the expired lineage after a fresh send, and by crash_at_StepRecorded, after a recovery that drops a never-sent call; other ways of changing lineage are not exercised",
    "replay, not re-invocation (§14, 5) covers the model, tool and compaction provider (recorded Setup/step answers survive CompactionApplied before fold); step-transform hooks are not in this subset",
    "the crash guarantee that each resume writes one informational record (§14, 8) is not checked: where that record sits is the runner's schema",
    "a call's indeterminate window between a restart and its outcome_unknown close is not observable reliably, so crash_at_DispatchIntent checks the state after the close",
    "a run cut at DispatchIntent is expected to end interrupted, as the role's points list states for that point; StepRecorded accepts either one dispatch with continuation or zero dispatches with interrupted state; ToolResultRecorded checks only that the run has one terminal state that is not cancelled",
    "a StepRecorded seal that keeps the call must hold its never-executed result in the transcript read once the run has ended; a result written in a separate commit that the runner makes before any read can reach it (during its own resume) cannot be told apart from one written in the commit that ended the run",
    "StepRecorded checks transcript ordering rather than model inputs; compaction cases independently capture canonical inputs at the scripted model boundary",
    "compaction timing cases require an injected runner/provider clock and drain due work exactly at configured deadlines; the engine-wide WAIT cap and prompt-cache lifetime are configured by the script, since the contract pins no numeric cap",
    "Setup durability is inspected from the runner's durable store while its first model request is held, never inferred from the provider script or model view",
    "insertion-before-nonempty-replacement ordering has no v1 producer: compaction-provider §8 applies the newest CompactionMessage alone, §12 gives every replacement its entire range, and hooks transform fields on raw records; only the insertion-before-message half is live conformance",
    "the crash guarantee that messages read before a kill read the same after it (§14, 1) is checked on a second session written before the kill, because the cut session has nothing readable before its trigger",
    "run_result_interrupted_not_cancelled reads the run cut in the DispatchIntent crash scenario, because a run cuts each point only once",
    "extra_op_still_admitted reads its case as the consumer's lenient decoding (§2) applied to the live answer",
    "role_describe_groups_complete also fails a runner whose role.describe declares a group the subject does not, because the suite would skip that group's cases while the wire claims it",
    "baseline_matches_admission_reply is inapplicable to a runner whose session capabilities come from session.baseline, which has no admission reply",
    "undeclared_delivery_refused is inapplicable to a runner that declares every delivery mode",
    "the suite fails a run with no kill at all, because §14 fails every run in which no kill ended a real process",
    "each send_id case is split in two: send_id_retry_same_answer, send_id_retry_settled_same_answer, send_id_reuse_refused_naming_field and delivery_change_refused check only the reply, never read the transcript, and run on a runner that declares queue or steer (their first send is steer when queue is not declared); send_id_retry_written_once, send_id_reuse_writes_nothing and delivery_change_writes_nothing check what was written and are inapplicable without transcript_reads",
    "send_id_retry_same_answer never waits, so it does not compare its two retries whole: before the send's run ends its state may move on between them; send_id_retry_settled_same_answer makes that comparison after seeing the run end through run.result, so it requires run_ops (skipped without it) and is inapplicable when the send's reply names no run_id",
    "a delivered receipt may also move from absent to pending, because on a confirm runner an absent receipt already means pending (§9)",
];

/// How one case ended.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum CaseOutcome {
    Passed,
    /// Every rule checked over the real route passed, but one rule was not
    /// checked. In `run_attribution_per_message`, the adapter answered
    /// `CannotHoldUnattributable`: it declares the runner never stores a
    /// message without the run that produced it, so the rule that such a
    /// message must omit `run` cannot be exercised. `reason` names the
    /// unchecked rule, so the report never reads as a plain pass.
    PassedWithInapplicableRule {
        reason: String,
    },
    Failed {
        reason: String,
    },
    /// Not run: the runner adapter does not declare these capabilities.
    Skipped {
        missing: Vec<Capability>,
    },
    /// Not run: without a test clock, the case would have to wait longer in real time than the suite allows (30 seconds).
    SkippedWithReason {
        missing: Vec<Capability>,
        reason: String,
    },
    /// Not run: the question cannot be asked of this runner at all (for
    /// example, an undeclared delivery mode of a runner that declares them
    /// all). Never a pass, and not a missing capability either.
    Inapplicable {
        reason: String,
    },
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct CaseReport {
    pub case: &'static str,
    pub requires: Vec<Capability>,
    pub outcome: CaseOutcome,
}

/// The run's overall result.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum SuiteVerdict {
    /// Every case ran (or was inapplicable) and passed, and at least one
    /// kill ended a real process.
    Passed,
    /// Nothing failed, but the runner adapter does not declare every capability,
    /// so the cases requiring `skipped` were not run. The runner conforms
    /// for the capabilities in `declared`, and for no others. Never a plain
    /// pass.
    ConformingForDeclaredCapabilities {
        declared: Vec<Capability>,
        skipped: Vec<Capability>,
    },
    Failed {
        reasons: Vec<String>,
    },
}

/// Everything a run found.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SuiteReport {
    pub cases: Vec<CaseReport>,
    /// Every kill the run made, with the mechanism the harness reported.
    pub kills: Vec<KillReport>,
    pub verdict: SuiteVerdict,
}

impl SuiteReport {
    pub fn outcome(&self, case: &str) -> Option<&CaseOutcome> {
        self.cases
            .iter()
            .find(|report| report.case == case)
            .map(|report| &report.outcome)
    }

    /// A human-readable summary, one line per case, then the kills, the
    /// narrowings and the verdict, for a CI log.
    pub fn render(&self) -> String {
        let names = |capabilities: &[Capability]| -> String {
            capabilities
                .iter()
                .map(|c| c.name())
                .collect::<Vec<_>>()
                .join(", ")
        };
        let mut out = String::new();
        for case in &self.cases {
            let line = match &case.outcome {
                CaseOutcome::Passed => format!("PASS {}", case.case),
                CaseOutcome::PassedWithInapplicableRule { reason } => format!(
                    "PASS {} (one rule not applicable by declaration: {reason})",
                    case.case
                ),
                CaseOutcome::Failed { reason } => format!("FAIL {}: {reason}", case.case),
                CaseOutcome::Skipped { missing }
                    if case.requires.contains(&Compaction)
                        || case.requires.contains(&ModelView) =>
                {
                    format!(
                        "N/A  {} (requires {}; missing {})",
                        case.case,
                        names(&case.requires),
                        names(missing)
                    )
                }
                CaseOutcome::Skipped { missing } => format!(
                    "SKIP {} (requires {}; missing {})",
                    case.case,
                    names(&case.requires),
                    names(missing)
                ),
                CaseOutcome::SkippedWithReason { missing, reason } => format!(
                    "SKIP {} (requires {}; missing {}): {reason}",
                    case.case,
                    names(&case.requires),
                    names(missing)
                ),
                CaseOutcome::Inapplicable { reason } => {
                    format!("N/A  {}: {reason}", case.case)
                }
            };
            out.push_str(&line);
            out.push('\n');
        }
        for kill in &self.kills {
            out.push_str(&format!("KILL at {} by {}\n", kill.point, kill.mechanism));
        }
        for narrowing in NARROWINGS {
            out.push_str(&format!("NARROWED {narrowing}\n"));
        }
        let verdict = match &self.verdict {
            SuiteVerdict::Passed => "passed".to_owned(),
            SuiteVerdict::ConformingForDeclaredCapabilities { declared, skipped } => format!(
                "conforming for declared capabilities: {} (not run: {})",
                names(declared),
                names(skipped)
            ),
            SuiteVerdict::Failed { reasons } => format!("failed: {}", reasons.join("; ")),
        };
        out.push_str(&format!("VERDICT {verdict}\n"));
        out
    }
}
