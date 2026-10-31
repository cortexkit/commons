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
const SESSION_RETAINED: &[Capability] =
    &[Capability::SessionRetention, Queue, TranscriptReads, RunOps];
const COMPACTED: &[Capability] = &[Compaction, ModelView, Queue, TranscriptReads, RunOps];
const CANCEL: &[Capability] = &[Capability::RunCancel, Queue, TranscriptReads, RunOps];
const INPUT_CAP: &[Capability] = &[Capability::InputTokenCap, Queue, TranscriptReads, RunOps];
const STRUCTURED: &[Capability] = &[Capability::StructuredOutput, Queue, TranscriptReads, RunOps];
const ADOPT: &[Capability] = &[Capability::Adopt, TranscriptReads];
const TAGS: &[Capability] = &[Capability::Tags, Queue, TranscriptReads];
const TOOL_CALLS: &[Capability] = &[Capability::RunToolCalls, Queue, TranscriptReads];
const USAGE: &[Capability] = &[Capability::RunUsage, Queue, RunOps];
const SESSION_USAGE: &[Capability] = &[
    Capability::SessionUsage,
    Capability::RunUsage,
    Queue,
    RunOps,
    TranscriptReads,
];
const ORDER: &[Capability] = &[Capability::DeliveryOrder, Queue, TranscriptReads];
const ORDER_STEER: &[Capability] = &[Capability::DeliveryOrder, Queue, TranscriptReads, Steer];
const SESSION_CHANGE: &[Capability] = &[Capability::SessionChange, Queue, RunOps];
const DISPLAY: &[Capability] = &[
    Capability::HistoryDisplay,
    Capability::HostHistory,
    Queue,
    TranscriptReads,
];
const CLIENT_ID: &[Capability] = &[
    Capability::ClientMessageId,
    Capability::HistoryDisplay,
    Capability::HostHistory,
    Queue,
    TranscriptReads,
];
const LIVE: &[Capability] = &[Capability::LiveEvents, Streaming, Queue, TranscriptReads];
const RUN_CAUSE: &[Capability] = &[
    Capability::RunCause,
    Capability::LiveEvents,
    Streaming,
    Queue,
    TranscriptReads,
];
const SENDER: &[Capability] = &[
    Capability::HistoryDisplay,
    Capability::HostHistory,
    Capability::DeliveryOrder,
    Queue,
    TranscriptReads,
];
const CACHE_TTL: &[Capability] = &[Compaction, Queue, TranscriptReads, RunOps];
const SESSION_CHANGE_HELD: &[Capability] =
    &[Capability::SessionChange, Queue, RunOps, HoldToolCalls];
const STRUCTURED_RESTART: &[Capability] = &[
    Capability::StructuredOutput,
    Queue,
    TranscriptReads,
    RunOps,
    KillAt(points::TOOL_RESULT_RECORDED),
];

/// The cut cases: a compaction session whose view with a cut applies.
const CUT: &[Capability] = &[Compaction, Queue, TranscriptReads, RunOps];
/// The history cases on a plain session.
const HISTORY: &[Capability] = &[Capability::HostHistory, Queue, TranscriptReads];
/// The history cases that apply a cut.
const HISTORY_CUT: &[Capability] = &[
    Capability::HostHistory,
    Compaction,
    Queue,
    TranscriptReads,
    RunOps,
];

/// Every case, in the order the runner runs them.
pub const CASES: &[CaseSpec] = &[
    case("service_tier_unsupported_before_model", &[Queue, RunOps],
        "an unsupported level is refused with unsupported_tier before any model call"),
    case("service_tier_run_fixed", &[Queue, RunOps],
        "a different explicit tier queued or steered into a running run is refused run_tier_fixed"),
    case("service_tier_ride_before_changed_request", &[Queue, RunOps],
        "a service_tier_change ride precedes the first changed-tier model request"),
    case("service_tier_unknown_never_echoed", &[Queue, RunOps],
        "an unknown-served wire reports applied but never copies it into served"),
    case("archive_pruning_history_cursor", &[Capability::ArchivePrunesReasoning, TranscriptReads, Capability::HostHistory], "A pre-prune cursor stays valid and its later page lists the pruned parts."),
    case("archive_pruning_reopen", &[Capability::ArchivePrunesReasoning, TranscriptReads, Queue], "Reopening continues without restoring or changing pruned history."),
    case("display_own_run_records_model", DISPLAY, "An assistant message driven through the runner's own run carries its recorded model."),
    case("archive_pruning_declared_kinds", &[Capability::ArchivePrunesReasoning, TranscriptReads], "Only declared pruning kinds are applied."),
    case("archive_pruning_payloads", &[Capability::ArchivePrunesReasoning, TranscriptReads], "Native payloads match the independent fixture, preserving required fields."),
    case("archive_pruning_identity", &[Capability::ArchivePrunesReasoning, TranscriptReads], "Part positions, types, keys, tags, lineage and history_version never move."),
    case("archive_pruning_write_time_digests", &[Capability::ArchivePrunesReasoning, TranscriptReads], "Message and part digests retain their write-time values."),
    case("archive_pruning_original_attestation", &[Capability::ArchivePrunesReasoning, TranscriptReads], "Listed SHA-256 and JCS length describe the original part."),
    case("archive_pruning_only_listed_parts", &[Capability::ArchivePrunesReasoning, TranscriptReads], "A reader can replace its held copy: only listed parts change."),
    case("archive_pruning_every_page", &[Capability::ArchivePrunesReasoning, TranscriptReads], "Every served read/history page form repeats precisely its pruning metadata."),
    case("archive_pruning_active_run", &[Capability::ArchivePrunesReasoning, TranscriptReads, Queue], "No pruning during an active model request."),
    case("archive_pruning_queued_send", &[Capability::ArchivePrunesReasoning, TranscriptReads, Queue], "No pruning while an idle queue holds an accepted send."),
    case("archive_pruning_setup_opt_in", &[Capability::ArchivePrunesReasoning, TranscriptReads, Compaction], "A Setup without acceptance prevents pruning."),
    case("archive_pruning_no_provider", &[Capability::ArchivePrunesReasoning, TranscriptReads], "No-provider sessions may prune at rest."),
    case("archive_pruning_generation", &[Capability::ArchivePrunesReasoning, TranscriptReads, Queue], "reasoning_pruned is absent before the first prune, generation 1 after it and one higher after a later prune, through_ordinal names the highest pruned ordinal, and pages and notices repeat the head."),
    case("archive_pruning_restored_view_refused", &[Capability::ArchivePrunesReasoning, TranscriptReads, Queue, Compaction], "A view whose replace range replays pre-prune copies is refused view_refused:<version>:pruned_reasoning_restored and never applied, while the same fold of the pruned copies applies."),
    case("archive_pruning_restored_edit_refused", &[Capability::ArchivePrunesReasoning, TranscriptReads, Queue, Compaction], "A view whose keep-range part edit sends a pruned part's original bytes is refused view_refused:<version>:pruned_reasoning_restored and never applied, while the same edit carrying the pruned part applies."),
    case("archive_pruning_setup_after_prune", &[Capability::ArchivePrunesReasoning, TranscriptReads, Queue, Compaction], "After a prune the runner's first provider call is a Setup carrying session.head's reasoning_pruned."),
    case("archive_pruning_setup_initial_replaced", &[Capability::ArchivePrunesReasoning, TranscriptReads, Queue, Compaction, Capability::ModelView], "While no view has applied, the post-prune Setup's new initial replaces the pre-prune one."),
    case("archive_pruning_applied_edit_pruned", &[Capability::ArchivePrunesReasoning, TranscriptReads, Queue, Compaction], "A prune rewrites an applied view's part edit of a signed part by the same rule, in the same commit: right after the prune the model view serves the edit without its pre-prune signature, not dropped; not applicable by name without model_view or a signature-kind fixture part."),
    case("session_retention_floor_describe", &[Capability::SessionRetention], "shared describe check refuses a floor above max_days"),
    case("session_retention_floor", SESSION_RETAINED, "content survives the declared floor and expires at max(policy, floor), never their sum"),
    case("session_retention_release_hold", SESSION_RETAINED, "release stays visible and readable, refuses sends with its deadline and answers pending twice"),
    case("session_retention_release_after_idle", SESSION_RETAINED, "release after an idle gap anchors a keep-none session's floor and actual deletion to the later release time"),
    case("session_retention_zero_floor_drain", SESSION_RETAINED, "floor zero still exposes released metadata, reads and pending release while draining"),
    case("session_retention_honoured", SESSION_RETAINED, "accepted policy appears unchanged in head and keep none fallback deletes without release"),
    case("session_retention_days_and_archive", &[Capability::SessionRetention, Queue, TranscriptReads, RunOps, Capability::RetentionClock], "archive preserves reads and days expiry deletes the idle session"),
    case("session_retention_adopt_policy", &[Capability::SessionRetention, Capability::Adopt, TranscriptReads], "adopt follows required, replaces runner policy once, freezes decoded values and records release owner"),
    case("session_retention_queued_never_deleted", &[Capability::SessionRetention, Queue, TranscriptReads, RunOps, Capability::RetentionClock], "days expiry, release and fallback cannot delete an idle queued send"),
    case("session_retention_required", SESSION_RETAINED, "first-send omission follows required and refusal freezes nothing"),
    case("session_retention_legacy_required", &[Capability::SessionRetention, Retention, Queue, TranscriptReads, RunOps], "when both groups are declared and the new policy is required, a legacy retention-only first send is retention_required"),
    case("session_retention_conflict", SESSION_RETAINED, "a later different policy is refused with the frozen policy; decoded-equal repeats succeed"),
    case("session_retention_owner_can_release", SESSION_RETAINED, "the starter principal can release under another principal's parent scope"),
    case("session_retention_parent_cannot_release", SESSION_RETAINED, "the parent scope owner is not the release owner"),
    case("session_retention_release_idempotent", SESSION_RETAINED, "release after completed deletion answers absent"),
    case("session_retention_deletion_observable", SESSION_RETAINED, "deleted head and reads refuse unknown_session; next send creates a new incarnation"),
    case("session_retention_media_deleted", &[Capability::SessionRetention, Queue, TranscriptReads, RunOps, Capability::Attachments], "deleted session cannot serve its referenced checksum"),
    case("session_retention_active_never_deleted", &[Capability::SessionRetention, Queue, TranscriptReads, RunOps, Capability::RetentionClock], "days expiry, release and fallback cannot delete an active run or its queued send"),
    case("session_retention_backup_none_refused", SESSION_RETAINED, "backup true with keep none is invalid_params naming retention_policy.backup"),
    case("session_retention_dumps_preserved", SESSION_RETAINED, "unsupported dumps set a state flag, never rewrite the accepted request_dumps"),
    case("session_retention_runner_replaced_once", SESSION_RETAINED, "a runner-assigned policy can be replaced once, then freezes as starter-sourced"),
    case("session_retention_pending_durable", &[Capability::SessionRetention, Queue, TranscriptReads, RunOps, Capability::Attachments, KillAt(points::RELEASE_PENDING)], "a real process kill after pending preserves release; recovery deletes head, reads and media without another release"),
    case("step_events_one_per_recorded_step", &[Capability::StepEvents, Queue, TranscriptReads],
        "one event per native recorded step at the declared per-attempt or per-completed-step granularity"),
    case("step_events_match_durable_position", &[Capability::StepEvents, Queue, TranscriptReads],
        "every event step_ref equals its independently read native durable position, never a message ordinal"),
    case("step_events_after_durability", &[Capability::StepEvents, Queue, TranscriptReads],
        "the adapter observes each actual publication after its record's durability point"),
    case("call_identity_key_matches_attribution", &[],
        "when role.describe declares call_identity, every display tool-call entry and live tool event of two dispatched sibling calls carries the call_key session.read attributes to that call, or run.tool_calls exposes without dispatch_attribution"),
    case("call_identity_pin_frozen_at_dispatch", &[],
        "when role.describe declares call_identity, a dispatched call's display entries carry the schema_pin the tool provider received with it, unchanged after the catalog changes the tool's definition"),
    case("call_identity_denied_call_has_no_pin", &[],
        "when role.describe declares call_identity, a call denied before dispatch carries no schema_pin, and call_key exactly as session.read attributes it, or run.tool_calls exposes without dispatch_attribution"),
    case("rich_ui_stamp_served_on_stamped_run", &[],
        "when role.describe declares rich_ui_stamp, a run whose plan's fetched system-text answers declare one stamp (twice, beside an unstamped answer) serves it verbatim on every assistant display row, on run_started, as head.last_run_rich_ui and on every assistant session.read entry read with include_rich_ui, and on no user, tool or other row"),
    case("rich_ui_stamp_absent_on_unstamped_run", &[],
        "when role.describe declares rich_ui_stamp, a run whose plan's system-text answer declares no stamp serves none on any display row, run_started, the head or an opt-in session.read entry"),
    case("rich_ui_stamp_read_needs_opt_in", &[],
        "when role.describe declares rich_ui_stamp, a session.read without include_rich_ui carries no rich_ui on any entry of a stamped run, and its entries are the opt-in entries without the member"),
    case("tool_upfront_preserves_offered_tools", &[],
         "an upfront hint admits the plan without changing the tools offered to the model"),
    case("tool_upfront_stale_refused", &[],
         "an unoffered upfront name is refused plan_stale before any write"),
    case("rich_ui_stamp_conflict_refused", &[],
        "when role.describe declares rich_ui_stamp, a plan whose fetched system-text answers declare two different stamps is refused plan_stale naming both, and nothing is written"),
    case("rich_ui_stamp_head_follows_last_run", &[],
        "when role.describe declares rich_ui_stamp, after a replan drops the stamp the next run is unstamped: head.last_run_rich_ui is gone, and the earlier run's assistant rows and opt-in entries keep their stamp"),
    case("system_text_default_forwarded_unchanged", &[],
        "when system_text_default is declared and the provider advertises system-text-default/v1, a preset-less item is admitted and reaches the provider byte-equal"),
    case("system_text_default_preset_required", &[],
        "when system_text_default is declared but the provider lacks system-text-default/v1, admission refuses system_text_preset_required naming the provider, with no writes or forwards"),
    case("head_incarnation_survives_cut_and_changes_on_recreation", &[Queue, TranscriptReads],
        "a reported log identity holds across a real cut but changes when the same key's log is created again"),
    case("head_sender_is_callers_effective_grant", &[Queue, TranscriptReads],
         "when role.describe declares head_sender, A gets supported modes minus idle_start, B gets every supported mode and interrupt, the unnamed caller gets no grant, and the owner gets owner=true"),
    case("refresh_added_sender_admitted_at_acceptance", &[],
         "with session_change, delivery_order, queue and transcript_reads, a sender added by a durable refresh is admitted before the next run, also mid-run when held calls are available"),
    case("refresh_removed_sender_refused_at_acceptance", &[],
         "with session_change, delivery_order, queue and transcript_reads, a removed sender is refused scope_owner_mismatch at acceptance and writes nothing"),
    case("app_readers_listed_reader_admitted", &[],
         "with app_readers, queue and transcript_reads, a plan-listed principal can read session.head and session.read"),
    case("app_readers_unlisted_reader_refused", &[],
         "with app_readers, queue and transcript_reads, an unlisted non-implicit principal is refused scope_owner_mismatch on both reads"),
    case("app_readers_reader_cannot_send", &[],
         "with app_readers, queue and transcript_reads, a reader without a send grant cannot send and writes nothing"),
    case("head_last_run_error_only_on_error", &[Queue, TranscriptReads],
        "reported last_run_error is absent on every state other than error, including after a successful later run"),
    case("head_last_run_error_matches_run_status", &[Queue, TranscriptReads, RunOps, Capability::RunStatus],
        "reported class, exact fractional retry seconds and reset timestamp equal run.status for the same run"),
    case("head_last_run_error_requires_last_run_state", &[Queue, TranscriptReads],
        "reported last_run_error always accompanies last_run_state"),
    case("session_usage_sums_runs_since_first", SESSION_USAGE,
        "two runs' reported totals add up to durable session spend; since names the first counted run"),
    case("session_usage_rewind_never_decreases", SESSION_USAGE,
        "rewinding visible history leaves session spend unchanged"),
    case("session_usage_unknown_fields_absent", SESSION_USAGE,
        "a token field no step reported stays absent, never zero"),
    case("head_model_reflects_recorded_switch", &[Queue, TranscriptReads],
        "a reported next-send selection follows the runner's recorded switch, not the frozen plan"),
    case("output_refs_unsupported_plan", &[], "a non-advertiser refuses output_refs plan members before any send, refresh or adoption write"),
    case("delivery_order_result_before_staging", &[Capability::DeliveryOrder, Queue, TranscriptReads, HoldToolCalls], "a tool result already sent before staging cannot carry a subsequently staged send"),
    case("output_refs_scanning", &[], "code spans, fences and escaped markers copy no output"),
    case("output_refs_shared_samples", &[], "reply, program and token-slot scanning uses the published scanner samples"),
    case("output_refs_writing_request", &[], "references retain the writing request's meaning after renumbering"),
    case("output_refs_target_reasons", &[], "user/model text and unknown numbers retain their refusal reasons"),
    case("output_refs_held_placeholder", &[], "held references keep the placeholder and call key after a late result"),
    case("output_refs_snapshot_size", &[], "oversized snapshots retain full byte size and SHA-256, without content"),
    case("output_refs_snapshot_budget", &[], "snapshot bytes across one message are capped at four MiB, including repeated references"),
    case("output_refs_descriptor_digest", &[], "descriptor digest matches records in canonical encounter order"),
    case("output_refs_rewind", &[], "rewind removes sets with their message"),
    case("output_refs_compaction", &[], "compaction preserves stored sets and snapshots"),
    case("output_refs_session_isolation", &[], "foreign session ids are indistinguishable from unknown ids"),
    case("output_refs_slots", &[], "only declared slots resolve; token slots reject extra text"),
    case("output_refs_reference_limit", &[], "occurrences after 64 are too_many_refs across a message"),
    case("output_refs_read_limits", &[], "metadata, EOF and range refusals obey snapshot bounds"),
    case("held_calls_lists_waiting_call", &[Queue, TranscriptReads, RunOps],
        "when role.describe declares held_calls, a provider's held reply is listed with its call_key, tool, provider, reason and held_at_ms while waiting"),
    case("held_calls_status_event_matches_run_status", &[Queue, TranscriptReads, RunOps, Capability::RunStatus, Streaming],
        "when role.describe declares held_calls, run_status, queue, transcript_reads, run_ops and streaming, control events carry the same typed held_calls as run.status while waiting, omit empty lists and never relist the call after its late result applies; the open event schema cannot identify a specific clearing status event"),
    case("held_calls_applies_late_result", &[Queue, TranscriptReads, RunOps, Capability::RenderedModelInput],
        "when role.describe declares held_calls, applying the provider's late result removes the held call and continues the run with that result in model input"),
    case("held_calls_not_started_becomes_tool_result", &[Queue, TranscriptReads, RunOps, Capability::RenderedModelInput],
        "when role.describe declares held_calls, a not_started late entry becomes a tool result saying the call never ran and the run continues"),
    case("held_calls_expired_becomes_tool_result", &[Queue, TranscriptReads, RunOps, Capability::RenderedModelInput],
        "when role.describe declares held_calls, an expired late entry becomes a tool result saying its answer expired and the run continues"),
    case("cut_starts_a_new_lineage", CUT,
        "after a view with a cut applies, session.head names a new lineage_id and first_ordinal is the cut's first retained ordinal; inapplicable without the view-publishing hook"),
    case("old_lineage_reads_refused_after_a_cut", CUT,
        "after a cut, a range read and an after_mid read naming the lineage the cut left are refused lineage_changed"),
    case("read_below_first_ordinal_refused", CUT,
        "after a cut, range reads below first_ordinal and an after_mid read naming a message below it are refused before_lineage_start with detail.first_ordinal, and the tail serves nothing below it"),
    case("ordinals_never_renumber_across_a_cut", CUT,
        "messages the cut retained keep their ordinals and mids, and the exchange after the cut continues the session-wide numbering"),
    case("setup_once_per_lineage", CUT,
        "Setup runs once on the first lineage and once more before the cut lineage's first model call, never again within it; a Setup's lineage_id, where the adapter observes it, names its lineage"),
    case("view_ranges_start_at_first_ordinal", CUT,
        "on a cut lineage, a view tiling from 0 is refused view_refused:<version>:ranges_start and never applied, while the same coverage tiled from first_ordinal applies"),
    case("cut_lineage_notice_descends_from", CUT,
        "the first advance notice of the cut lineage carries descends_from naming the old lineage at the cut, with the content digest of the message at the cut's through_ordinal"),
    case("head_names_the_cut", CUT,
        "after a cut, session.head names the cut by the mid, content digest and creation time of the message it folds last, never its content"),
    case("notice_carries_first_ordinal", CUT,
        "every advance notice on a cut lineage carries first_ordinal, the cut's first retained ordinal"),
    case("revert_below_the_cut_restarts_at_the_previous_cut", CUT,
        "with two cuts, a revert that removes the second cut's message restarts at the first cut (first_ordinal, session.head.cut) with descends_from naming the lineage at that cut, and Setup runs again; a revert below every cut restarts at ordinal 0 with no cut and no descends_from; inapplicable without the revert hook"),
    case("revert_below_a_cut_before_adoption_has_no_descent", CUT,
        "on a session adopted after its host cut, a revert below the runner's own cut restarts at the host's cut, with first_ordinal 0 and that cut on session.head, and without descends_from, because the runner has no lineage at it; inapplicable without the revert hook or a host cut"),
    case("checkpoint_on_descending_setup_refused", &[Compaction, Capability::TagCheckpoint, Queue, TranscriptReads, RunOps],
        "a Setup answer carrying checkpoint for a lineage a cut started (whose Setup carries descends_from) is a failed Setup: the run ends error with provider_code compaction_unavailable and reason checkpoint_on_descent, and no model request is made"),
    case("cut_setup_leaves_reads_alone", CUT,
        "while a cut lineage's Setup is held unanswered, session.head and session.read of the new lineage are served, never refused transient; inapplicable to a runner that never holds reads, or without the Setup-hold hooks"),
    case("cut_numbering_continues_from_the_old_lineage", &[Compaction, Capability::Tags, Queue, TranscriptReads, RunOps],
        "on a tag-rendering session, every part the cut retained keeps its number, and the first part written after the cut numbers one above the highest number the old lineage showed"),
    case("history_version_unchanged_by_a_cut", HISTORY_CUT,
        "session.head and session.history pages state the same history_version, and a cut with the exchange after it leaves it unchanged"),
    case("history_version_rises_at_a_revert", HISTORY,
        "a revert raises history_version on session.head and history pages; inapplicable without the revert hook"),
    case("history_from_ordinal_and_before_ordinal", HISTORY,
        "from_ordinal and before_ordinal serve the entries from and before an assigned ordinal, paging on with the returned cursors; both together are refused naming before_ordinal"),
    case("history_unknown_ordinal_refused", HISTORY,
        "from_ordinal and before_ordinal naming an ordinal never assigned are refused unknown_ordinal, and so is an ordinal a revert removed where the adapter can revert"),
    case("history_before_lowest_ordinal_reaches_unnumbered_entries", HISTORY,
        "with host history before the runner's first numbered message, before_ordinal 0 serves the newest entries without an ordinal, with an older cursor; inapplicable without the host-history hook"),
    case("history_pages_back_to_the_first_message", HISTORY,
        "from the newest page, following older reaches the session's first message in order with no gap or duplicate mid; following newer from the first message serves exactly the same sequence; a second walk serves the same bytes; the model view is refused naming view"),
    case("history_mid_matches_read", HISTORY,
        "every message session.read serves is served by session.history under the same mid, with the same ordinal, bytes and tags"),
    case("history_no_tags_below_first_ordinal", HISTORY_CUT,
        "after a cut, history entries below first_ordinal carry no tags, and window entries carry exactly what session.read serves"),
    case("history_ordinal_kept_below_the_cut", HISTORY_CUT,
        "a message read with an ordinal before a cut carries the same ordinal from history after it"),
    case("history_cursor_survives_a_cut", HISTORY_CUT,
        "an older cursor minted before a cut serves, after it, exactly the entries directly before the page that minted it, ending with no older cursor"),
    case("history_never_serves_a_reverted_branch", HISTORY,
        "after a revert and a new exchange, history serves the surviving prefix and what followed, never a message the revert left; inapplicable without the revert hook"),
    case("history_cursor_expired_after_a_revert_past_it", HISTORY,
        "a revert that removes the entry a cursor was minted next to refuses that cursor, and one minted on a removed page, cursor_expired, never an empty page; inapplicable without the revert hook"),
    case("history_boundary_order", HISTORY,
        "with host history before the runner's first numbered message, those entries carry no ordinal and the newest of them comes directly before the lowest ordinal; inapplicable without the host-history hook"),
    case("history_before_mid_continues_below_the_window", HISTORY,
        "with host history before a window starting at ordinal 0, before_mid of the window's first message serves the newest host entries with an older cursor, and paging older ends at the first entry with no older cursor; inapplicable without the host-history hook"),
    CaseSpec {
        name: "per_step_cost_scales_with_new_messages",
        requires: &[Queue],
        requires_any: &[TranscriptReads, RunOps],
        requires_undeclared_any: &[],
        checks: "opt-in through the seeding hook, never passed without it: with 1,000 and 150,000 finished messages seeded behind a cut that leaves the same window, the median of nine timed exchanges on the large session is at most 2.0 times the small one's (medians under 5 ms count as 5 ms); memory is each runner's own benchmark",
    },
    CaseSpec { requires_any: &[], requires_undeclared_any: &[], name: "user_commands_history_isolation", requires: &[Capability::UserCommands, Capability::Queue, Capability::RunOps, Capability::TranscriptReads], checks: "replies, progress and results never enter reads or actual model inputs" },
    CaseSpec { requires_any: &[], requires_undeclared_any: &[], name: "user_commands_forward_once", requires: &[Capability::UserCommands, Capability::Queue, Capability::RunOps], checks: "retry of an invocation returns the stored reply and reaches the provider once" },
    CaseSpec { requires_any: &[], requires_undeclared_any: &[], name: "user_commands_no_person", requires: &[Capability::UserCommands, Capability::Queue, Capability::RunOps], checks: "interactive false lists no commands and refuses no_person" },
    CaseSpec { requires_any: &[], requires_undeclared_any: &[], name: "user_commands_collision", requires: &[Capability::UserCommands, Capability::Queue], checks: "duplicate names at admission refuse user_command_collision with declaring providers" },
    CaseSpec { requires_any: &[], requires_undeclared_any: &[], name: "user_commands_progress_order", requires: &[Capability::UserCommands, Capability::Queue, Capability::RunOps], checks: "status keeps progress in arrival order and exposes the terminal result" },
    CaseSpec { requires_any: &[], requires_undeclared_any: &[], name: "user_commands_unsafe_host_menu", requires: &[Capability::UserCommands, Capability::Queue, Capability::RunOps], checks: "no host menu entries where replies cannot stay outside history; relay remains usable" },
    CaseSpec { requires_any: &[], requires_undeclared_any: &[], name: "user_commands_owner_only", requires: &[Capability::UserCommands, Capability::Queue, Capability::RunOps], checks: "both relay ops refuse a non-owner" },
    case("user_commands_outcome_unknown", &[Capability::UserCommands, Capability::Queue, Capability::RunOps, KillAt(points::USER_COMMAND_FORWARDED)], "after the provider receives a command but its reply is lost, restart returns terminal outcome_unknown through invocation and status without forwarding again; the module observed one invocation"),
    case("user_commands_cutoff_after_forward", &[Capability::UserCommands, Queue, RunOps], "a cutoff after forwarding stores outcome_unknown and replays it without forwarding again; inapplicable without a cutoff adapter"),
    case("user_commands_cutoff_before_forward", &[Capability::UserCommands, Queue, RunOps], "a cutoff before forwarding stores failed and replays it without forwarding; inapplicable without a cutoff adapter"),
    case("compaction_standalone_refusal_lifecycle", &[Capability::Compaction, Queue, RunOps, TranscriptReads], "standalone refusal hands off only before first service on in-process hosts; elsewhere and after service it fails by name without unmanaged model input"),
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
    case("session_refresh_applies_from_next_run", SESSION_CHANGE,
        "on an idle session, a refresh with the immediately rung answers its generation as pending, no run in progress and applies_from_next_run; the next run offers the new plan's tools and session.baseline then shows it applied"),
    case("session_refresh_waits_for_its_rung", SESSION_CHANGE,
        "a refresh with a rung other than immediately answers whether the next run uses the new plan, and the next run and session.baseline agree with that answer"),
    case("session_refresh_never_changes_a_running_run", SESSION_CHANGE_HELD,
        "a refresh while a run is running answers run_in_progress, the running run's later steps keep the tools it started with, and the next run uses the new plan"),
    case("session_refresh_repeat_same_answer", SESSION_CHANGE,
        "an identical repeat of a refresh (same generation, plan and policy) gets the same answer and changes nothing"),
    case("session_refresh_generation_stale_refused", SESSION_CHANGE,
        "a refresh at the accepted generation with another policy or plan, or below it, is refused generation_stale naming the accepted generation, writing nothing"),
    case("session_refresh_tool_outside_host_set_refused", SESSION_CHANGE,
        "on a host that fixes its tool set at start, a plan naming a tool outside it, or keeping a tool's name with a changed definition, is refused tool_outside_host_set naming those tools, writing nothing"),
    case("refresh_relaunch_preserves_runs_and_history", &[],
        "declared refresh relaunches apply new tools only at their rung's run boundary, preserve history bytes and ordinals, and ride before the first changed-surface request"),
    case("run_cancel_by_interrupt_now_cancelled", &[],
        "a currently interrupt-granted plan sender cancels now and the run ends cancelled"),
    case("run_cancel_by_interrupt_without_grant_refused", &[],
        "a plan-named sender without interrupt cannot cancel"),
    case("run_cancel_by_interrupt_non_now_refused", &[],
        "an interrupt-granted sender cannot cancel at a point other than now"),
    case("run_cancel_by_interrupt_other_session_refused", &[],
        "a grant on one session never authorizes cancellation on another"),
    case("retract_own_send_sender_retracted", &[],
        "the daemon-attested sender retracts its own pending submission and it never runs"),
    case("retract_own_send_other_sender_refused", &[],
        "another non-owner cannot retract the submission, even with a forged sender field, and it stays pending"),
    case("retract_own_send_shared_send_id_isolated", &[],
        "two non-owner senders reuse a send_id; retracting one submission twice never withdraws the other's pending submission"),
    case("flush_prefix_applies_pending", SESSION_CHANGE_HELD,
        "a flush while a run is running answers the pending generation, leaves the running run's tools unchanged, and applies the pending change from the next run; a repeat of its send_id gets the same answer"),
    case("max_steps_final_step_completes", &[Capability::MaxSteps, Queue, RunOps],
        "with max_steps 1, a step that asks for no tool call ends the run completed with its final message, and its directives where turn_directives is declared, never max_steps"),
    case("max_steps_submit_result_completes", &[Capability::MaxSteps, Capability::StructuredOutput, Queue, TranscriptReads, RunOps],
        "with max_steps 1, a step calling submit_result ends the run completed with the submitted value and its sibling skipped_after_submit, never max_steps"),
    case("max_output_tokens_caps_response", &[Capability::MaxOutputTokens, Capability::RunUsage, Queue, TranscriptReads, RunOps],
        "the dispatched per-response output maximum and the single response's reported output usage never exceed the requested cap"),
    case("no_tools_offered_to_model", &[Capability::NoTools, Queue, TranscriptReads],
        "tools [] offers no tools to the model, including the tools the adapter normally supplies"),
    case("no_tools_later_plan_send_refused", &[Capability::NoTools, Queue, TranscriptReads],
        "when the adapter can create a fetch-plan session, every later tools member is refused invalid_params naming tools without writes"),
    case("read_stable_across_host_metadata_change", &[Queue, TranscriptReads, Capability::HostMetadataChange],
        "a re-read after the host changes the session's metadata, under the same lineage, answers the same entries byte for byte"),
    case("tags_on_every_read_entry", TAGS,
        "on a session whose plan renders tags, every read entry carries one row per part; every tag is a well-formed §N§ used once in the lineage, the numbers are consecutive, every part key is used once in its message, the prompt and each answer carry a number, and a second read answers the same rows"),
    case("tags_numbers_stable_within_a_lineage", TAGS,
        "after a second run on the same lineage, the numbers are still consecutive and used once, and every earlier part keeps its number; a part's number changes only with a lineage change"),
    case("media_descriptors_image_bytes", &[TranscriptReads, Queue, RunOps], "media_ref hashes decoded image bytes; adapter image fixture required"),
    case("attachments_lifecycle", &[Capability::Attachments, TranscriptReads, Queue, RunOps], "chunked upload into an existing session, status, lost-append-reply resume, commit, send and digest read including metadata probe and EOF"),
    case("attachments_restart_gone", &[Capability::Attachments, TranscriptReads, Queue, RunOps], "restart loses open uploads as gone, never open at zero; committed references survive"),
    case("attachments_chunk_cap", &[Capability::Attachments, TranscriptReads, Queue, RunOps], "exactly 2 MiB accepted, one byte over refused without changing received"),
    case("attachments_commit_sha256", &[Capability::Attachments, TranscriptReads, Queue, RunOps], "wrong SHA-256 refused and upload remains unusable"),
    case("attachments_commit_size", &[Capability::Attachments, TranscriptReads, Queue, RunOps], "short commit and append beyond declared size refused without changing upload"),
    case("attachments_model_unsupported", &[Capability::Attachments, TranscriptReads, Queue, RunOps], "a new run checks its model's media input and refuses unsupported types without writes"),
    case("attachments_model_mime", &[Capability::Attachments, TranscriptReads, Queue, RunOps], "a model accepting images but only PNG/PDF MIME types refuses a GIF without writes"),
    case("attachments_frozen_model", &[Capability::Attachments, TranscriptReads, Queue, RunOps, Capability::Steer, Capability::HoldToolCalls], "joining send checks frozen model, not replacement send parameters, and writes nothing on refusal"),
    case("attachments_send_uncommitted", &[Capability::Attachments, TranscriptReads, Queue, RunOps], "open upload refused without writes"),
    case("attachments_send_unknown", &[Capability::Attachments, TranscriptReads, Queue, RunOps], "unknown upload refused without writes"),
    case("attachments_send_other_session", &[Capability::Attachments, TranscriptReads, Queue, RunOps], "another session's upload refused without writes"),
    case("attachments_upload_other_session_status", &[Capability::Attachments, TranscriptReads, Queue, RunOps], "status of another session's id is gone, exactly as for an id that never existed"),
    case("attachments_upload_other_session_commit", &[Capability::Attachments, TranscriptReads, Queue, RunOps], "commit and append of another session's id are unknown_upload and leave its owner's upload unchanged"),
    case("attachments_send_gone", &[Capability::Attachments, TranscriptReads, Queue, RunOps], "lost upload refused without writes"),
    case("attachments_default_read", &[Capability::Attachments, TranscriptReads, Queue, RunOps], "default media renders as text markers; blocks opts in; descriptors and digests identify stored media"),
    case("attachments_tool_result", &[Capability::Attachments, TranscriptReads, Queue, RunOps], "nested media renders as a legacy text result by default and is readable by digest"),
    case("attachments_media_unreferenced", &[Capability::Attachments, TranscriptReads, Queue, RunOps], "holding committed bytes does not authorize an unreferenced digest"),
    case("attachments_media_other_session", &[Capability::Attachments, TranscriptReads, Queue, RunOps], "a digest referenced only by another session is unknown_media"),
    case("attachments_media_read_admission", &[Capability::Attachments, TranscriptReads, Queue, RunOps], "media_read admits exactly the callers admitted by session.read"),
    case("attachments_admission", &[Capability::Attachments, TranscriptReads, Queue, RunOps], "no pre-session uploads; only send-authorized callers upload"),
    case("attachments_plan_sender", &[Capability::Attachments, Capability::DeliveryOrder, TranscriptReads, Queue, RunOps], "plan-named sender may upload and send; other callers may not upload"),
    case("attachments_retention", &[Capability::Attachments, Capability::RetentionClock, TranscriptReads, Queue, RunOps], "unreferenced commits survive at least one hour and referenced media survives later and restart"),
    case("media_descriptors_nested_media", &[TranscriptReads, Queue, RunOps], "nested tool-result media becomes a ref in place without its own part key"),
    case("media_descriptors_frame_budget", &[TranscriptReads, Queue, RunOps], "over-frame arguments are described and served; mixed-size fallback chooses the largest payloads, breaking a tie at the cut by earlier part order; inline still refuses reply_too_large"),
    case("media_descriptors_utf8_head", &[TranscriptReads, Queue, RunOps], "text_ref head ends on a UTF-8 boundary at or before byte 4096"),
    case("media_descriptors_inline_unchanged", &[TranscriptReads, Queue, RunOps, Capability::Tags], "inline is the default; unknown media is refused invalid_params naming media"),
    case("media_descriptors_full_content_digests", &[TranscriptReads, Queue, RunOps], "message and part digests equal independently computed stored-content digests; attachments uses blocks, older runners inline"),
    case("media_descriptors_pairing", &[TranscriptReads, Queue, RunOps], "descriptors preserve call/result pairing fields and signatures"),
    case("media_descriptors_part_keys", &[TranscriptReads, Queue, RunOps], "descriptor decoding rejects digest rows whose keys do not match the actual parts"),
    case("media_descriptors_provider_rows", &[TranscriptReads, Queue, RunOps], "provider-side descriptor validation requires exact equality with complete tag-row keys and the serializer profile's part count, never a derived key scheme"),
    case("media_descriptors_model_view_refused", &[TranscriptReads, Capability::ModelView], "model-view descriptors are refused invalid_params naming media before read work"),
    case("media_descriptors_stable_across_pages", &[TranscriptReads, Queue, RunOps], "below-threshold frame fallback is deterministic per message, independent of page caps and position"),
    case("tags_only_on_tag_rendering_sessions", TAGS,
        "a session whose plan sets render_tags true numbers its prompt and answer; one whose plan leaves it absent keeps a row per part but numbers none, and refuses session.resolve unknown_tag"),
    case("tags_resolve_returns_exposed_content", TAGS,
        "session.resolve of a prompt's and an answer's tags answers their exposed text naming the entry's ordinal, mid and part in the current lineage, and a character range counted in Unicode scalar values answers exactly that text"),
    case("tags_resolve_unknown_tag_refused", TAGS,
        "a well-formed tag no part of the current lineage carries is refused unknown_tag, and malformed tags are refused invalid_params naming tag"),
    case("tags_resolve_range_refused", TAGS,
        "a range past the content or starting after its end is refused invalid_params naming range; an empty range at the end answers empty content"),
    case("tags_model_written_tags_cleaned_on_exposure", TAGS,
        "the model's own tag notation is cleaned by standalone's rules on session.read, session.resolve and run.result for a tag-rendering run, exposed byte for byte for an untagged run, and left raw in a structured-output run's final text where structured_output is declared; a tool call's arguments holding §N§ reach the tool once, as written"),
    case("tags_numbered_in_rendered_order", &[Capability::Tags, Queue, TranscriptReads, Steer, HoldToolCalls, Capability::RenderedModelInput],
        "a steer sent while a tool call is held: the last model request's texts carry exactly the numbers session.read reports, increasing in the order the model saw them; inapplicable when the runner rendered the steer where it stored it"),
    case("tags_renumbering_waits_for_the_step_boundary", &[Capability::Tags, Queue, TranscriptReads, Steer, HoldToolCalls],
        "while a step's tool call is held, a steer changes neither the lineage nor any number a read before it showed; the call runs once; afterwards a part a read exposed carries another number only on a new lineage"),
    case("tags_tool_result_blocks_share_one_number", TAGS,
        "a whole tool result has one number on its first non-empty text or media label; empty and structured-only results do not count"),
    case("tags_count_only_parts_rendered_with_a_number", TAGS,
        "a step holding reasoning, text and a tool call numbers its text and the part holding the call's result, found through result_ordinal (where run_tool_calls is declared) or the result's marker, whether that is an entry of its own or the step's own tool part in an embedded layout; reasoning and a part holding only the call carry none; an image-only tool result carries a number and, where the adapter records rendered input, its label §N§ [image] (named as unchecked when the adapter's tool provider cannot answer an image-only result)"),
    case("message_checkpoint_numbering_starts_at_message_count", &[], "message_checkpoint only: the adoption lineage starts at message_count"),
    case("message_checkpoint_head_matches_certified_cut", &[], "message_checkpoint only: session.head.cut equals the offline-certified head_cut"),
    case("message_checkpoint_anchors_after_folded_message", &[], "message_checkpoint only: a removed cut anchors after the surviving head_cut.mid"),
    case("message_checkpoint_native_marker_carries_both_counters", &[], "message_checkpoint only: actual native marker keys carry message_count, head_cut and tag_checkpoint"),
    case("message_checkpoint_missing_counter_is_not_zero", &[], "message_checkpoint only: reloading an incomplete marker without fallback refuses, never manufactures zero"),
    case("message_checkpoint_uncounted_rows_never_served", &[], "message_checkpoint only: finished host summaries and, on OpenCode 1, compaction-only rows are absent from reads on adoption and native lineages, and counted ordinals are contiguous"),
    case("message_checkpoint_head_during_setup_uses_marker", &[], "message_checkpoint only: session.head answers during the first outstanding Setup without waiting on its gates, using the pre-existing marker's message_count and head_cut"),
    case("message_checkpoint_read_during_adoption_setup", &[], "message_checkpoint only: session.read answers during adoption-cut Setup from first_ordinal without waiting on Setup gates, and refuses rows below that bound"),
    case("tags_setup_checkpoint_anchors_at_its_cut", &[Capability::Tags, Capability::TagCheckpoint, Capability::Compaction, Queue, TranscriptReads],
        "with a Setup checkpoint of 500 cut at the session's first message and no marker carrying one, the prompt is numbered 501 and the answer 502"),
    case("tags_fork_of_a_managed_session_refused", &[Capability::Tags, Capability::Compaction, Queue, TranscriptReads],
        "session.send into a fork of a managed session, and session.adopt of it where declared, is refused fork_unsupported, writing nothing; inapplicable unless the adapter can create a fork the runner can detect"),
    case("tags_lost_marker_key_keeps_numbers", TAGS,
        "losing the newest marker key falls back without renumbering; inapplicable unless the adapter can compact in the host and drop its key"),
    case("tags_host_store_cleaned_at_the_save_seam", TAGS,
        "the host store keeps tagged replies clean and untagged and structured-output replies raw; inapplicable unless the adapter can read the host store"),
    case("tags_resolve_answers_the_current_lineage", &[Capability::Tags, Queue, TranscriptReads, DispatchAttribution, KillAt(points::STEP_RECORDED)],
        "after the crash_at_StepRecorded seal moves the session to a new lineage, every number the new lineage shows (up to 64) resolves to the part showing it there, a number above its highest is refused unknown_tag, and the cut run's prompt carries a number; inapplicable when the runner resumes the run or keeps the call on its lineage"),
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
    case("delivery_order_ungranted_interrupt_refused", ORDER,
        "a plan-named sender whose entry grants idle_start but not interrupt sends the interrupt string and is refused invalid_params naming delivery, writing nothing; inapplicable without interrupt"),
    case("delivery_order_granted_interrupt_interrupts", &[Capability::DeliveryOrder, Queue, TranscriptReads, RunOps],
        "while the owner's run waits on a held model step, a plan-named sender granted interrupt sends the interrupt string: it is admitted, the interrupted run ends cancelled, and its message starts a run that completes; inapplicable without interrupt or the held-step hook"),
    case("delivery_order_named_sender_queue_string_admitted", ORDER,
        "a plan-named sender granted idle_start sends the string queue (not a list): it is admitted, its message is written once and starts a run that completes"),
    case("delivery_order_template_renders_as_system", ORDER,
        "a template send carrying an empty prompt is refused invalid_params naming template and writes nothing; a plan sender's template send with valid slots and no prompt goes out once with the rendered text, as turn_system where the adapter's route supports it and as a steer otherwise"),
    case("delivery_order_bad_slot_refused", ORDER,
        "a template send whose percentage slot is 101 is refused invalid_params naming template, writes nothing, and nothing goes out as turn_system"),
    case("delivery_order_template_part_gone_refused", &[Capability::DeliveryOrder, Queue, TranscriptReads, Capability::Tags],
        "on a session whose plan renders tags, a plan sender's template send whose part_tag slot names a part the session does not show is refused template_part_gone naming the slot, writing nothing; one naming a part it shows is accepted"),
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
    case("adopt_host_turn_is_a_run", &[Capability::Adopt, TranscriptReads, RunOps],
        "a turn the person starts in their own app is a run: session.head names it, run.result answers it, run.status and session.subscribe show it where declared, and the adopting owner's own runs are told from it by their send_id only; inapplicable when the adapter cannot start a host turn"),
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
        "without structured_output_per_session, a send without structured_output after a run with it has no schema, submit_result tool or structured result; with the group, later changes to the first admitted send's presence or whole-member JCS value are refused invalid_params naming structured_output without writes"),
    case("structured_output_external_ref_refused", STRUCTURED,
        "a schema with a $ref or $dynamicRef outside its document is refused invalid_params naming structured_output.schema at admission, writing nothing and starting no model request"),
    case("structured_output_sibling_never_dispatched", STRUCTURED,
        "without submit_streamed_siblings, a sibling emitted before submit_result in the same step is never invoked at the scripted tool provider, gets a never-executed skipped_after_submit result, and is listed; a second submit_result is a skipped sibling and the first one counts"),
    case("structured_output_streamed_siblings", STRUCTURED,
        "with submit_streamed_siblings, an earlier sibling may retain its dispatched result and is then not skipped; a later sibling and later submit_result never dispatch, get skipped_after_submit results and are listed in emission order; the run completes with the first submitted value validated"),
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
    case("compaction_setup_route_session_matches_setup", &[Compaction, Queue],
        "a route bound to one session uses the same session Setup reports; a shared project route with an empty session and generic harness is accepted; a Setup refused setup_identity_mismatch ends the run error under that code, with no model request"),
    case("session_ref_uses_runner_identity", &[Compaction, Queue],
        "advance-notice session_ref headers match the shared function of the declared runner module id, Setup harness and session, never the project folder"),
    case("model_view_half_open_ranges_and_travel", COMPACTED,
        "Setup [0,0) insertions precede message zero and travel with it over count/byte caps, are not repeated on the next page, and the held model request carries that canonical view"),
    case("compaction_unavailable_distinct_from_refuse", &[Compaction, Queue, TranscriptReads, RunOps],
        "Setup unavailability and a Setup REFUSE end error distinctly, send nothing to the model and leave the session usable"),
    case("view_due_first_model_uses_view", &[Compaction, Queue, TranscriptReads, RunOps],
        "an opted-in runner's first model request equals Setup's due view rendering, never initial"),
    case("part_edits_truncate_renders_with_own_tag", &[Compaction, Capability::Tags, Queue, TranscriptReads, RunOps],
        "an opted-in truncate edit renders exactly the shared profile renderer's output with the counted part's own tag"),
    case("part_edits_bytes_render_with_own_tag", &[Compaction, Capability::Tags, Queue, TranscriptReads, RunOps],
        "an opted-in bytes edit carries the counted part's own runner-rendered tag"),
    case("view_due_refused_fetch_blocks_model", &[Compaction, Queue, TranscriptReads, RunOps],
        "a refused due-view fetch makes no model call and ends compaction_unavailable with reason view_due_unapplied"),
    case("view_due_later_step_retries", &[Compaction, Queue, TranscriptReads, RunOps],
        "after a refused fetch a later step retries and applies the same due view without repeating Setup"),
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
    case("display_row_is_the_profile_projection", DISPLAY,
        "a display row equals the serializer profile's projection of the same entry's raw body, without limits and with the apps' limits, taking only model and client_message_id from the served row; every assistant row carries model; the envelope keeps ordinal, written_at and tags; inapplicable when the adapter names no profile"),
    case("display_limits_cut_on_a_utf8_boundary", DISPLAY,
        "with limits falling inside a multi-byte character of a call's arguments and of its result, each is cut to the last character boundary at or below its bound with arguments_truncated or truncated set, and nothing else changes"),
    case("display_without_limits_is_whole", DISPLAY,
        "without limits a call's arguments and result larger than the apps' bounds are served whole and unflagged; with the apps' limits both fit their bounds"),
    case("display_pages_carry_finished_messages_only", DISPLAY,
        "while a model step is held, the display walk serves exactly the raw walk's finished entries and no row for the step in flight; once the run ends, the answer is there"),
    case("client_message_id_on_display_rows_never_in_read", CLIENT_ID,
        "a send's client_message_id appears on the display row of the user message it created and on no other row, and nowhere in the default session.read page"),
    case("client_message_id_resend_with_another_id_refused", &[Capability::ClientMessageId, Queue],
        "a re-send under the same send_id with the same client_message_id is a retry; with another id or without one it is refused send_id_reuse naming client_message_id; an id outside [A-Za-z0-9_-]{1,64} is refused invalid_params naming it"),
    case("live_message_and_tool_events_durable", LIVE,
        "a replay from start carries message_started then message_finished for every stored mid, tool_started (the holding message's mid, the model's id, the tool, the whole arguments, started_at) then tool_finished with outcome ok and a finished_at no earlier; a second replay serves the same control frames and one after the tool_started cursor exactly what followed it"),
    case("live_run_events", LIVE,
        "a completed run has one run_started and, after it, one run_finished with reason completed; any run_paused lies between"),
    case("live_attach_replays_the_current_step", LIVE,
        "a stream attached while a model answer is held after two parts streamed gets display_reset, then one coalesced delta per part so far with mid and part, in seq order; the mid is the stored message's; inapplicable without the held-stream hook"),
    case("live_seq_rises_across_lanes", LIVE,
        "a stream from start while a step is in flight carries control and display frames whose seq starts at 1 and rises by one across both lanes; inapplicable without the held-stream hook"),
    case("live_untimed_history_call_leaves_times_out", LIVE,
        "after one run the adapter seeds a finished call stored before the runner kept tool-call times; a replay from start carries its tool_started without started_at and its tool_finished without finished_at, never an invented time; inapplicable without the seed_untimed_tool_call hook"),
    case("run_cause_queued_start_names_its_send", RUN_CAUSE,
        "the run a queue send started on an idle session has run_started naming that send's own send_id, never an id the runner made"),
    case("run_cause_send_started_runs_never_omit_send_id", RUN_CAUSE,
        "on a session whose every run a send started (the admitting send and two later queue sends), every run_started carries a non-empty send_id"),
    case("run_cause_interrupt_names_the_new_send", RUN_CAUSE,
        "while a run's model step is held, an owner's interrupt cancels it and starts a run whose run_started names the interrupting send, not the interrupted one; inapplicable without interrupt or the held-step hook"),
    case("display_row_names_the_sender_never_in_read", SENDER,
        "a plan-named sender's [idle_start] message has a display row naming that sender's module as sender, and the default session.read page carries no sender member; inapplicable when idle_start is not a declared list mode"),
    case("cache_ttl_entry_for_another_model_ignored", CACHE_TTL,
        "a configured cache_ttl entry naming another model never leads to ttl_expired, however long the prompt cache idles; inapplicable when the runner declares no ttl_expired for its own model's expired entry, or without the clock, model and observed-lifetime hooks"),
    case("cache_ttl_configured_null_never_expires", CACHE_TTL,
        "a configured entry with ms null never leads to ttl_expired, even with an observed lifetime that has passed"),
    case("ride_wait_provider_wait_honoured", CACHE_TTL,
        "Setup answers ride_wait_ms 4000; the provider answers the ttl_expired ride with a view after 3 s, and the view applies at the ride's step, whose run completes; inapplicable unless the runner sends ride_wait_max_ms of at least 4000, declares ttl_expired, and the adapter has the ride and cache-clock hooks"),
    case("ride_wait_slow_answer_reported", CACHE_TTL,
        "the provider answers the ttl_expired ride with nothing after 2.5 s, and the run reads compaction_wait while the step waits, with deadline_ms on run.status where served; inapplicable as the other ride cases"),
    case("ride_wait_fast_answer_not_reported", CACHE_TTL,
        "the provider answers the ttl_expired ride with nothing within 200 ms, and the run never reads compaction_wait; inapplicable as the other ride cases"),
    case("ride_wait_above_cap_fails_setup", CACHE_TTL,
        "Setup answers a ride_wait_ms one above the ride_wait_max_ms the runner's Setup request carried: the run ends error with provider_code compaction_unavailable and a reason naming ride_wait_ms, and no model call is made; inapplicable unless the runner sends ride_wait_max_ms"),
    case("cache_ttl_missed_model_change_falls_through_to_observed", CACHE_TTL,
        "after a model switch whose ride brought no entry, the old model's lifetime is never used: no ttl_expired before the observed lifetime passes, ttl_expired once it has"),
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
    "compaction cases check Setup durability, refusals and model-view insertion, plus whether a single-session provider route's session agrees with Setup; adapters must observe the actual route bind separately from Setup, and shared project routes with an empty bind session are valid. The cut cases publish views through the adapter's hook and read advance notices the adapter captured at the bus; rides, the ceiling and native markers still have no live case",
    "read_below_first_ordinal_refused checks range and after-message reads; session.resolve's before_lineage_start is not exercised, because the cut fixture's session renders no tags",
    "per_step_cost_scales_with_new_messages measures wall time over the real route, which includes writing the model request; the two sessions keep the same window, so that part is equal on both sides. Memory is not measured",
    "the prompt cache's lifetime (compaction-provider §8.1) is observed only through the ttl_expired token of advance notices; a ride's ttl_expired cause is not seen, and entries reach the runner only on Setup's answer (the scripted provider answers a model_change ride with none, as a missed ride would), so the newest-entry-wins rule across several entries is checked only against cache_ttl::lifetime and its vectors",
    "the display cases read rows of a session whose tool results are not errors, because a scripted tool result cannot be made an error result; error_result_bytes is checked against each profile's vectors, and live only as a bound that cuts nothing",
    "the attach replay is checked up to the moment the stream goes quiet, because RunnerRoute::subscribe collects a stream only until then; the live deltas that follow the replay are not seen, and message_finished riding the frame of the stored message's unit is not checked, because a unit is the runner's own",
    "the ride_wait cases time a ride by the scripted provider's own delay, on the adapter's clock, and read compaction_wait from session.head between polls: a report shorter than the gap between two polls can be missed, and a deadline_ms on run.status is checked as present, not against the runner's clock, which the suite does not share",
    "session.head's input has no live case: what a send that names no model can carry depends on the runner's model catalog, so input is checked against the vectors only",
    "a run started or joined by a plan-named sender's send runs under the session owner's authority (CONTRACT §9, §9.7), but no case checks it: the principal a tool call carries reaches the tool provider, and no route reply or adapter hook shows it to the suite",
    "run_cause's one omission, a turn the host started with no send on an adopted session (§7.1, §9.5), has no live case; the cases check the send-started runs, and the omission against the vectors only",
    "the sender a display row names is checked for a plan-named sender's message only; whether the runner also records the owner's own sends is its choice",
    "the history cases page with a limit of two entries; byte caps and descriptor media on session.history are not exercised separately from session.read's cases",
    "structured-output cases cannot see a runner's commits, so 'in the same commit as the run's end' is checked as: once the run reads terminal, every skipped sibling already has exactly one never-executed result; schema subsets (schema_outside_subset, mode_not_enforceable) are runner-specific and are checked against the vectors only",
    "structured_output_frozen_across_restart cuts ToolResultRecorded with a crash driver of its own, on its own state root, because the suite's driver cuts each point once and the shared crash scenario owns that cut; its kill is not in the run's kill ledger and does not count toward the real-kill rule",
    "run_attribution_per_message uses transcript_reads and a held tool call to observe a non-terminal run, cancelling it over run.cancel only when run_cancel is declared; the native fixture hook supplies unknown-origin records independently of read replies, or explicitly declares only that rule inapplicable via CannotHoldUnattributable (NotSupported fails)",
    "retention cases use advance_retention_clock when declared; otherwise waits over 30 seconds are skipped by name and shorter waits use real time",
    "retention deletion completion is inspected by the subject in the runner's durable report and runner-held stores, never inferred from an expired read; non-role export/list/read request schemas and pagination are supplied by retention_probe_params, and every advertised extra op must be classified",
    "retention_run_status_expired is inapplicable if run.status is not advertised",
    "the fetch plan's shape is not pinned (CONTRACT §10), so the subject builds each session's first-send fields and the suite changes only the plan members its cases' sections define, never validating the rest or a baseline's items",
    "the tags cases check part rows, tags and resolve answers; how a tag is rendered to the model is checked only where the adapter records the texts each request carried (rendered_model_input); durability across a restart is driven only by tags_resolve_answers_the_current_lineage, after the StepRecorded crash",
    "tags_resolve_answers_the_current_lineage cannot read the dropped part's number, because the kill comes before any read of the cut step; numbers may repeat across lineages, so it checks that every number the new lineage shows resolves to the part showing it there, never to a dropped part",
    "the step-boundary rule is driven only by a steer into a held tool call, the one renumbering trigger every runner shares; a runner that renders the steer where it stores it renumbers nothing there, and the case then checks only that the numbers stay as they were",
    "a fork is checked only where the adapter can create one the runner can detect (fork_session): a runner refuses only forks it can know about, so the suite never requires detecting a fork that carries nothing of the runner's",
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
    "lineage_changed for an old lineage is checked by retention_honoured, for the expired lineage after a fresh send, and by crash_at_StepRecorded, after a recovery that drops a never-sent call, and by old_lineage_reads_refused_after_a_cut after a cut; a revert is exercised only through session.history's cases, where the adapter can revert",
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
