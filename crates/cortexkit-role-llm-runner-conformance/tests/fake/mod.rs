//! A reference in-process runner, used ONLY to test the suite.
//!
//! Conformance runs against real runners. This fake exists so the suite's
//! own tests can show that each case passes a runner that keeps the
//! contract and fails one that breaks it, so it must be no more capable than
//! a real runner. The property it models:
//!
//! > A runner that hosts sessions from a durable log of records (start,
//! > send, assistant step, dispatch intent, tool result, terminal), rebuilds
//! > every session from that log on restart, and resumes what was cut: a
//! > call with an intent and no result is closed `outcome_unknown` and its
//! > run sealed `interrupted`; a step without an intent either resumes or
//! > is sealed `interrupted`, according to the test's recovery policy.
//!
//! It runs a session's run synchronously inside the request that starts it,
//! stopping only at a call the scripted tool provider holds. Everything it
//! knows reaches the suite only as replies on a route. Its kills are fault
//! hooks inside one process, so it reports [`KillMechanism::FaultHook`];
//! `claim_process_kill` makes it report a real process kill instead, which
//! it is not, so the tests can reach the verdicts that need one.

use std::{
    collections::{BTreeMap, BTreeSet},
    fs::OpenOptions,
    io::Write,
    path::{Path, PathBuf},
    sync::{
        atomic::{AtomicBool, AtomicU64, AtomicUsize, Ordering},
        Arc, Mutex,
    },
    time::Duration,
};

use async_trait::async_trait;
use cortexkit_role_llm_runner_conformance::{
    harness::{
        Harness, HarnessError, KillMechanism, KillPoint, KillReport, PointDeclaration, RouteStamp,
        Trigger,
    },
    wire::{
        adopt::{check_drift, AdoptReply, AdoptRequest, PLAN_FIELD},
        baseline::{Baseline, BaselineItem, BaselineReply},
        capabilities as groups,
        delivery_order::{self as order, DeliveryOrder, Mode},
        describe::{Major, Retention, RoleDescribe},
        errors, ops, points,
        read::{
            HeadMeta, LastRunState, ReadMessage, ReadMode, ReadPage, ReadRequest, ReadView,
            RunAttribution, ToolCallAttribution,
        },
        retract::{RetractReply, RetractRequest},
        run::{FinalMessage, RunResult, RunResultRequest, RunState, RunUsage, StepUsage},
        send::{
            delivered_as, delivery_unsupported_detail, Delivered, Delivery, SendReply, SendRequest,
            CLIENT_MESSAGE_ID_FIELD, DELIVERY_FIELD,
        },
        structured_output::{SkippedSibling, StructuredOutputResult, SUBMIT_RESULT_TOOL},
        subscribe::{SubscribeFrom, SubscribeRequest},
        PROVIDES,
    },
    Capability, LlmRunnerSubject, ModelRequestObservation, PlanSenderIdentity, Reply, RouteFailure,
    RunnerRoute, Script, ScriptedPart, ScriptedToolCall, SubscribeOutcome, SCRIPTED_TOOL,
};
use serde_json::{json, Map, Value};
use subc_protocol::ErrorBody;

mod attachments;
mod compaction;
mod display;
mod held_calls;
mod history;
mod media;
mod output_refs;
mod session_change;
mod system_text_default;
mod tags;
mod user_commands;
pub use compaction::{CacheTtlDefect, CompactionDefect, CompactionRouteMode};
use display::{anthropic_assistant, live_events, profiled, result_text};
pub use history::LineageDefect;

pub const TOOL_MODULE: &str = "fake.tools";
/// The module id (and route principal) of the sender the delivery-order
/// cases name in a session plan's `senders` member.
pub const SENDER_MODULE: &str = "fake.sender";
/// How long the fake holds a run at the context ceiling before its bound.
const CEILING_WAIT_MS: u64 = 300_000;
/// The `ride_wait_max_ms` the fake's Setup requests carry: the longest it
/// holds a step for a rewrite ride.
pub const RIDE_WAIT_MAX_MS: u64 = 60_000;
/// The fake's own deadline for a rewrite ride when Setup set none.
const OWN_RIDE_DEADLINE_MS: u64 = 2_000;
/// How far one `pause` moves the clock rewrite rides are timed by.
const RIDE_TICK_MS: u64 = 50;
pub const RETENTION_MAX: u64 = 4;
const RETENTION_DELETE_MS: u64 = 100;
// Self-tests neutralize this switch to prove their named failure assertions
// observe the deliberate runner defects rather than a precomputed verdict.
const RETENTION_DEFECTS_ENABLED: bool = true;

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum DirectiveDefect {
    #[default]
    None,
    In,
    Note,
    Off,
    MaxSteps,
    Cancelled,
    Unrequested,
    Unknown,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum RetentionDefect {
    #[default]
    None,
    Ignore,
    Empty,
    Timestamp,
    ReuseLineage,
    Inherit,
    ShortenIgnored,
    ShortenReplaces,
    EqualRefused,
    LengthenAccepted,
    LateOptInAccepted,
    ZeroAccepted,
    AboveMaxAccepted,
    MaxDetailMissing,
    WithoutGroupIgnored,
    ExpireHeld,
    ClockNotRestarted,
    /// Moves its retention clock forward but never deletes a session's content once it expires.
    NeverPurge,
    TombstoneForgotten,
    TitleLeak,
    PromptLeak,
    ToolLeak,
    Status,
}
const DEFAULT_MAX_BYTES: u64 = 64 * 1024;
/// The provider and model the fake reports every step's usage under.
const FAKE_PROVIDER: &str = "scripted";
const FAKE_MODEL: &str = "scripted-model";
const MAXIMUM_MAX_BYTES: u64 = 1024 * 1024;
const DEFAULT_LIMIT: u64 = 100;
const MAXIMUM_LIMIT: u64 = 1000;

fn new_incarnation() -> String {
    let mut bytes = [0u8; 16];
    getrandom::fill(&mut bytes).expect("OS entropy for the fake's durable log identity");
    format!("{:032x}", u128::from_be_bytes(bytes))
}
const READ_FIELDS: &[&str] = &[
    "media",
    "from_ordinal",
    "after_mid",
    "lineage_id",
    "limit",
    "max_bytes",
    "include_originals",
    "view",
];

/// Deliberate contract breaks: each field makes the fake violate one rule a
/// case must catch.
#[derive(Clone, Copy, Debug, Default)]
pub struct Defects {
    pub system_text_default_invents_preset: bool,
    pub system_text_default_ignores_catalog: bool,
    pub incarnation_changes_on_cut: bool,
    pub incarnation_reused_on_recreation: bool,
    pub head_sender_ignores_caller: bool,
    pub head_sender_grant_for_unnamed_caller: bool,
    pub head_sender_owner_absent: bool,
    pub head_sender_omitted: bool,
    pub head_sender_a_idle_start: bool,
    pub session_usage_sum_wrong: bool,
    pub session_usage_since_wrong: bool,
    pub session_usage_reset_on_rewind: bool,
    pub session_usage_unknown_zero: bool,
    pub head_model_from_plan: bool,
    pub output_refs: Option<&'static str>,
    pub held_calls_never_listed: bool,
    pub held_calls_stale: bool,
    pub attachments: &'static str,
    /// Store attachment media in the runner's own schema as a `data:` URL
    /// `file` part, with no digest anywhere in the stored message.
    pub attachments_store_data_url: bool,
    pub media: Option<&'static str>,
    pub user_command: Option<&'static str>,
    pub compaction: CompactionDefect,
    /// The lineage, read-bound, view-start, history and cost breaks.
    pub lineage: LineageDefect,
    /// Continue to a new model step after a boundary cancellation.
    pub cancel_starts_next_step: bool,
    /// Accept an unlisted cancellation point instead of refusing it.
    pub cancel_accepts_unlisted_at: bool,
    /// Keep waiting for the step boundary when a second cancel asks for
    /// `now`, instead of cancelling immediately.
    pub cancel_ignores_escalation: bool,
    /// Omit run attribution from messages whose run is recorded.
    pub attribution_missing_run: bool,
    /// Mark the first message final, even before a durable terminal exists.
    pub attribution_wrong_final: bool,
    /// Mark a running run's newest message final before its terminal is durable.
    pub attribution_running_final: bool,
    /// Only an assistant message can be final, losing a tool-result terminal.
    pub attribution_assistant_only_final: bool,
    /// Invent a run for a historical record with no run identity.
    pub attribution_invent_run: bool,
    pub attribution_fixture_missing: bool,
    pub attribution_cannot_hold_unattributable: bool,
    /// A read with no cursor answers the oldest page instead of the newest.
    pub tail_returns_oldest: bool,
    /// A call's key is minted from the model's tool_call_id, so a model id
    /// used in two steps gets the same key twice.
    pub call_key_from_model_id: bool,
    /// Resume seals a run cut by a kill `cancelled` instead of
    /// `interrupted`.
    pub cancel_cut_runs: bool,
    /// A never-sent call in a sealed run falsely reports an unknown outcome
    /// through `indeterminate: true`.
    pub sealed_call_indeterminate: bool,
    /// Sealing leaves the never-sent call dangling in the next model history.
    pub sealed_call_without_result: bool,
    /// A seal that keeps the never-sent call ends the run without its
    /// never-executed result and writes the result only when the owner's
    /// next send arrives, in a later commit.
    pub sealed_result_in_later_commit: bool,
    /// A seal that keeps the never-sent call also changes the bytes of the
    /// session's first message, already read, under the same lineage.
    pub sealed_rewrites_read_message: bool,
    /// Sealing hides attribution but leaves the dangling call in the raw message.
    pub sealed_call_without_attribution: bool,
    /// Resumption invokes a never-sent call twice, breaking at-most-once dispatch.
    pub dispatch_twice_on_resume: bool,
    /// A step holding `submit_result` still invokes its sibling calls at the
    /// scripted tool provider before the run ends.
    pub structured_output_dispatches_siblings: bool,
    /// role.describe declares `interrupt`, which the runner does not serve
    /// and the subject does not declare.
    pub claim_unserved_group: bool,
    /// A guaranteed runner falsely answers pending for a steer send.
    pub guaranteed_steer_pending: bool,
    /// Answer `pending` to a steer sent into a running turn held by a tool
    /// call, though a guaranteed runner must never answer `pending`.
    pub held_steer_pending: bool,
    /// Answer `pending` only to the re-send of a steer into a held turn; the
    /// first answer stays correct.
    pub held_steer_retry_pending: bool,
    /// Return a different delivery receipt on the re-send of a steer into a
    /// held turn than on its first answer.
    pub held_steer_retry_unstable: bool,
    /// End the held turn before answering a steer into it, so the steer is
    /// answered while no run is in progress.
    pub held_steer_ends_run: bool,
    /// Omit the receipt on the final re-send after the held run ends.
    pub held_steer_final_absent: bool,
    /// Supply a receipt during the hold, then change its reference after release.
    pub held_steer_after_release_unstable: bool,
    /// A re-send of a delivered steer changes its receipt.
    pub resend_steer_unstable: bool,
    /// Answer a steer into a running turn with `state: "step"` instead of
    /// the turn's `active` or `finished` state.
    pub steer_reply_state_step: bool,
    /// Every answer to a send names a submission_id of its own, so a retry
    /// names another one than the send was answered with.
    pub retry_new_submission_id: bool,
    /// A queued send is answered `delivered: step`, and its retries
    /// `pending`: a receipt moving backward.
    pub retry_delivered_step_then_pending: bool,
    /// A queued send is answered `delivered: unknown`, and its retries
    /// `step`: a final receipt changing.
    pub retry_delivered_unknown_then_step: bool,
    /// A send_id reused with another prompt or delivery mode is answered as
    /// a retry of the first send, writing nothing, instead of refused
    /// `send_id_reuse`.
    pub reuse_accepted: bool,
    /// Once a queued send's run has ended, its second retry is answered
    /// `active` while the first was answered `finished`: two retries that
    /// differ in state after nothing could have moved.
    pub retry_state_moves_after_end: bool,
    /// `session.adopt` writes a user message into the transcript, which an
    /// adoption must never do.
    pub adopt_writes_message: bool,
    /// `session.adopt` from a caller other than the recorded owner takes the
    /// session over instead of being refused `scope_owner_mismatch`.
    pub adopt_takes_over_owner: bool,
    /// Tag numbering starts again at each run, so a later run's parts show
    /// numbers an earlier run's parts show in the same lineage.
    pub tags_restart_each_run: bool,
    /// The model's text is exposed as written even on a session whose plan
    /// renders tags, instead of cleaned of its own tag notation.
    pub tags_exposed_raw: bool,
    /// Parts are numbered in storage order instead of the order they are
    /// rendered in.
    pub tags_numbered_in_storage_order: bool,
    /// A steer that arrives while a step's tool call is still running moves
    /// the session to a new lineage at once, before the call has run.
    pub tags_renumber_mid_step: bool,
    /// A renumbering of exposed parts keeps the lineage.
    pub tags_renumber_without_new_lineage: bool,
    /// Reasoning parts are counted and numbered like the model's text.
    pub tags_count_reasoning: bool,
    /// With results embedded in the requesting entry's tool part, that tool
    /// part is left unnumbered as if it held only the call, though it holds
    /// the call's result.
    pub tags_embedded_result_unnumbered: bool,
    /// A tool-result entry of its own (message-array layout) is left
    /// unnumbered, as if it held only a tool call.
    pub tags_tool_result_entry_unnumbered: bool,
    /// A session whose plan renders no tags is numbered all the same.
    pub tags_numbered_without_render_tags: bool,
    /// The compaction provider's Setup checkpoint is ignored: numbering
    /// starts at the session start.
    pub tags_setup_checkpoint_ignored: bool,
    /// A send into, or an adoption of, a fork of a managed session is
    /// served instead of refused `fork_unsupported`.
    pub fork_accepted: bool,
    /// A turn the host starts creates no run: nothing answers it on
    /// `run.result`, `run.status` or the stream.
    pub host_turns_without_run: bool,
    /// A template send whose `part_tag` slot names a part the session does
    /// not show is accepted, the slot rendered as written.
    pub order_template_part_gone_accepted: bool,
    /// After a marker lost its key, numbering from the older marker counts
    /// only the parts rendered now, skipping those the newer marker
    /// summarized.
    pub tags_count_only_rendered_parts: bool,
    /// The host's save seam leaves a tag-rendering run's reply raw in the
    /// host's own store.
    pub host_seam_not_cleaning: bool,
    /// `run.tool_calls` lists a run's calls in reverse dispatch order.
    pub run_tool_calls_reversed: bool,
    /// A `delivery_order` list without `idle_start` starts a run on an idle
    /// session.
    pub order_starts_runs_without_idle_start: bool,
    /// A template send's slots are substituted without validation, so an
    /// out-of-range value renders and goes out as system text.
    pub order_template_slots_unchecked: bool,
    /// A `delivery_order` send from a module the plan does not name is
    /// admitted.
    pub order_admits_any_sender: bool,
    /// A send with the same `dedupe_key` queues beside the earlier one
    /// instead of replacing it.
    pub order_dedupe_keeps_both: bool,
    /// `expires_at_ms` is ignored: an expired send is still delivered.
    pub order_expiry_ignored: bool,
    /// The last possible mode of a list wins instead of the first.
    pub order_last_possible_mode: bool,
    pub order_before_staging: bool,
    /// A runner without the `delivery_order` group accepts a list in
    /// `delivery`, delivering it as a legacy `queue`.
    pub order_list_accepted_without_group: bool,
    /// A list member (`template`, `dedupe_key`, `expires_at_ms`) beside a
    /// string or absent `delivery` is ignored instead of refused.
    pub order_members_without_list_accepted: bool,
    /// Queue a list even when none of its modes is declared by the runner.
    pub order_undeclared_modes_queued: bool,
    /// Use capability groups alone, ignoring the advertised mode restriction.
    pub order_modes_ignored: bool,
    /// A template send carrying `"prompt": ""` is accepted as if it carried
    /// no prompt, instead of refused naming `template`.
    pub order_template_with_empty_prompt_accepted: bool,
    /// A recovery that drops a never-sent call from history keeps the
    /// session's lineage.
    pub drop_keeps_lineage: bool,
    /// `session.read` serves the host's live record, so a host-side metadata
    /// change shows up in an entry's bytes under the same lineage.
    pub read_serves_live_host_metadata: bool,
    /// A run's `usage` reports `reasoning_tokens` as `0` when no step
    /// reported any, at the top and in every `by_model` entry, instead of
    /// leaving the total absent.
    pub usage_absent_reasoning_as_zero: bool,
    /// Bill reasoning as extra output by adding it to the step's output
    /// count, as a runner that reads it as an addition rather than a
    /// breakdown would.
    pub usage_reasoning_added_to_output: bool,
    /// After a restart, `send.status` answers a queued send that was
    /// delivered before it `pending` again, as a status kept only in memory
    /// would; a re-send still answers from the durable record.
    pub send_status_pending_after_restart: bool,
    /// `session.resolve` still answers a tag whose part a recovery dropped
    /// when it moved the session to a new lineage, with the dropped part's
    /// old content.
    pub resolve_left_lineage_tags: bool,
    /// A queued send delivered as a steer is answered `delivered: {as:
    /// step, ref}` without `run_id` even though the runner declares `run_ops`,
    /// on a re-send and on `send.status`.
    pub delivered_steer_without_run_id: bool,
    /// A planned-stop seal deliberately omits its durable reason.
    pub interrupt_shutdown_reason_missing: bool,
    /// Sibling call positions are reversed without changing the listed calls.
    pub run_tool_calls_positions_swapped: bool,
    /// An open call falsely claims a stored result location.
    pub run_tool_calls_open_result_ordinal: bool,
    /// Embed results in the requesting message; keep it unreadable until every
    /// tool returns so reads never expose a message that will change.
    pub run_tool_calls_embedded_results: bool,
    /// A finished requesting message loses the arguments its listed call names.
    pub run_tool_calls_omit_finished_arguments: bool,
    /// Serve open requesting messages without arguments, then restore them
    /// after the tool returns, changing content that readers already saw.
    pub run_tool_calls_open_message_without_arguments: bool,
    pub directives: DirectiveDefect,
    /// The model-response limit is ignored.
    pub max_steps_ignored: bool,
    /// The max_steps terminal is written without its pending calls' results.
    pub max_steps_unsealed: bool,
    /// A `session.refresh` with the `immediately` rung applies inside the
    /// run that is running, so its next step sees the new tools.
    pub refresh_applies_mid_run: bool,
    /// A `session.refresh` answers `run_in_progress: false` while a run is
    /// running.
    pub refresh_hides_running_run: bool,
    /// A `session.refresh` answers `applies_from_next_run: true` whatever
    /// its rung, though a rung whose point is not met waits.
    pub refresh_claims_next_run: bool,
    /// The `immediately` rung waits for a rebuild like the other rungs.
    pub immediately_waits_for_rebuild: bool,
    /// Not a defect: the session's baseline states only the `immediately`
    /// rung, as a runner that applies every change at the next run would.
    pub states_only_immediately: bool,
    /// An identical repeat of a refresh is refused `generation_stale`.
    pub refresh_repeat_refused: bool,
    /// A refresh at or below the accepted generation is accepted.
    pub refresh_stale_accepted: bool,
    /// The host's tool set is checked by name only, so a changed
    /// definition under a kept name is accepted.
    pub host_set_by_name_only: bool,
    /// `session.flush_prefix` never applies the pending change.
    pub flush_ignored: bool,
    /// `session.flush_prefix` applies the pending change inside the run
    /// that is running.
    pub flush_applies_mid_run: bool,
    /// A step that reaches the limit ends the run `max_steps` even when it
    /// asked for no tool call, or called `submit_result`, instead of
    /// completing it.
    pub max_steps_on_final_step: bool,
    /// The requested output maximum is not passed to the scripted model.
    pub max_output_tokens_ignored: bool,
    /// Empty tools leaves the plan's tools offered to the model.
    pub no_tools_ignored: bool,
    /// A later tools member on a plan session is silently ignored.
    pub late_plan_tools_ignored: bool,
    /// A display row of an assistant message whose stored body names no
    /// model is served without `model`.
    pub display_drops_model: bool,
    /// A cut keeps exactly its bound's bytes, splitting a character and
    /// patching the split with U+FFFD.
    pub display_cut_lossy: bool,
    /// A cut payload is served without `arguments_truncated` or `truncated`.
    pub display_cut_unflagged: bool,
    /// A display read without `limits` is cut by the apps' limits.
    pub display_default_limits: bool,
    /// While a model step is held, the display page serves a row for it.
    pub display_in_progress_row: bool,
    /// The default `session.read` entry of a user message carries the
    /// send's `client_message_id` inside its body.
    pub client_id_in_read: bool,
    /// A re-send under the same `send_id` with another `client_message_id`
    /// is answered as a retry.
    pub client_id_reuse_accepted: bool,
    /// No `tool_finished` event is written.
    pub live_tool_finished_missing: bool,
    /// No `run_finished` event is written.
    pub live_run_finished_missing: bool,
    /// A call replayed from history stored without times gets the time its
    /// record was replayed at as `started_at` and `finished_at`, instead of
    /// leaving both out.
    pub live_tool_time_invented: bool,
    /// A newly attached stream gets no replay of the step in flight.
    pub live_attach_skips_replay: bool,
    /// Display frames carry a `seq` of their own, apart from control frames.
    pub live_seq_per_lane: bool,
    /// A run a `queue` send started names an id the runner made for its
    /// queue in `run_started.send_id`, not the send's own `send_id`.
    pub run_cause_internal_id: bool,
    /// A run a later `queue` send started (any but the admitting send)
    /// carries no `send_id` on `run_started`.
    pub run_cause_omitted_after_admission: bool,
    /// A run an interrupt started names the interrupted run's send in
    /// `run_started.send_id`, not the interrupting send.
    pub run_cause_names_interrupted: bool,
    /// A plan-named sender's `interrupt` is admitted without the entry's
    /// `interrupt` grant.
    pub order_interrupt_grant_ignored: bool,
    /// `interrupt` stays the owner's: a plan-named sender's interrupt is
    /// refused even with the grant.
    pub order_interrupt_owner_only: bool,
    /// A plan-named sender's string `queue` or `steer` is refused
    /// `scope_owner_mismatch`, as if only lists came from named senders.
    pub order_named_sender_strings_refused: bool,
    /// The default `session.read` entry of a user message carries the
    /// sender's module as `sender`.
    pub sender_in_read: bool,
    /// A rewrite ride waits the fake's own fixed deadline, whatever Setup's
    /// `ride_wait_ms` says.
    pub ride_wait_fixed_deadline: bool,
    /// A step waiting on a rewrite ride is never reported `compaction_wait`.
    pub ride_wait_never_reported: bool,
    /// A step waiting on a rewrite ride is reported `compaction_wait` as soon
    /// as the ride is sent, not once its answer has been pending for 1 s.
    pub ride_wait_reported_at_once: bool,
    /// A Setup answer whose `ride_wait_ms` is above the fake's own
    /// `ride_wait_max_ms` is accepted.
    pub ride_wait_cap_ignored: bool,
}

/// Recovery of a recorded assistant step whose tool calls were never sent.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum StepRecovery {
    #[default]
    Resume,
    SealInterrupted,
    SealDropStep,
    SealDropCall,
}

/// What the subject and every module incarnation share: the scripted model
/// and the scripted tool provider live outside the runner, so they survive
/// its kills.
struct World {
    system_text_default: Mutex<BTreeMap<String, system_text_default::Provider>>,
    held_replies: Mutex<BTreeMap<String, held_calls::HeldReply>>,
    standalone_in_process: bool,
    /// Host entries every session holds before the runner numbered
    /// anything, unless the hook seeded another count for it.
    host_prefix_everywhere: u64,
    /// Each session's history in the host before the runner numbered
    /// anything: the host's own storage, outside the runner's log.
    host_prefixes: Mutex<BTreeMap<String, Vec<history::HostEntry>>>,
    /// Sessions whose host history ends in a cut the host made before the
    /// runner adopted them.
    host_cuts: Mutex<BTreeSet<String>>,
    /// The runner refuses reads `transient` while a tag-rendering run waits
    /// for a Setup without descent.
    holds_reads_during_setup: bool,
    /// Sessions whose next Setup call the scripted provider holds.
    setup_holds: Mutex<BTreeMap<String, history::SetupHold>>,
    user_commands: Mutex<user_commands::Commands>,
    retention_defect: RetentionDefect,
    retention_delete_ms: u64,
    epoch: tokio::time::Instant,
    retention_clock_advanced_ms: AtomicU64,
    held_paused: bool,
    advertise_status: bool,
    cancel_step_boundary: bool,
    defects: Defects,
    /// Store media the way a runner whose own schema keeps it inline does,
    /// as a `data:` URL `file` part that carries the digest nowhere.
    data_url_media: bool,
    step_recovery: StepRecovery,
    compaction_route_mode: compaction::CompactionRouteMode,
    queue_receipt_pending_then_unknown: bool,
    steer_receipt_confirm: bool,
    omit_first_steer_receipt: bool,
    early_held_steer_receipt: Option<Delivered>,
    held_steer_receipts: Mutex<Vec<Option<Delivered>>>,
    /// Counts invalid answers produced by the held-turn steer defects,
    /// so a test can prove its defect actually took effect.
    held_steer_break_answers: AtomicUsize,
    groups: Vec<String>,
    delivery_order_modes: Option<Vec<String>>,
    scripts: Mutex<BTreeMap<String, Script>>,
    model_calls: Mutex<BTreeSet<(String, String, usize)>>,
    /// What each session's scripted route reports about schema-constrained
    /// output, as a catalog row would; a missing entry is not stated.
    structured_output_support: Mutex<BTreeMap<String, Option<bool>>>,
    /// Every model request the scripted model received, per session.
    model_requests: Mutex<BTreeMap<String, Vec<ModelRequestObservation>>>,
    pending_model_holds: Mutex<BTreeSet<String>>,
    held_models: Mutex<BTreeMap<String, String>>,
    staging_holds: Mutex<BTreeSet<String>>,
    compaction: Mutex<compaction::Providers>,
    invocations: Mutex<BTreeMap<String, usize>>,
    held: Mutex<BTreeSet<String>>,
    released: Mutex<BTreeSet<String>>,
    /// Every `session.read` and `session.head` request received, served or
    /// not.
    transcript_calls: AtomicUsize,
    /// Every `run.result` request received, served or not.
    run_result_calls: AtomicUsize,
    /// Every `session.adopt` request received, served or not.
    adopt_calls: AtomicUsize,
    /// Whether the scripted route supports a turn-scoped system message.
    turn_system: bool,
    /// Sessions whose next model request waits at the context ceiling.
    ceiling_holds: Mutex<BTreeSet<String>>,
    /// The model each session's requests go to, when switched from the
    /// fake's own.
    models: Mutex<BTreeMap<String, String>>,
    /// The prompt-cache lifetime each session's requests carry.
    observed_cache_lifetimes: Mutex<BTreeMap<String, Option<u64>>>,
    /// Each session's prompt-cache clock.
    cache_clocks: Mutex<BTreeMap<String, compaction::CacheClock>>,
    cache_ttl_defect: CacheTtlDefect,
    /// Sessions whose next model answer streams this many parts and holds.
    stream_holds: Mutex<BTreeMap<String, usize>>,
    /// The step in flight of each session whose answer is held mid-stream:
    /// the `mid` its message will have, and the parts streamed so far.
    in_flight: Mutex<BTreeMap<String, (String, Vec<ScriptedPart>)>>,
    /// How many `run.result` reads of an ended run still answer `active`,
    /// modelling a runner that reports the run as ended only a moment after
    /// the send returns.
    late_seal_reads: AtomicUsize,
    /// How each session's next rewrite ride is answered, until a ride takes
    /// it.
    ride_scripts: Mutex<BTreeMap<String, cortexkit_role_llm_runner_conformance::ScriptedRide>>,
    /// The rewrite ride each session's step waits on.
    rides: Mutex<BTreeMap<String, compaction::PendingRide>>,
    /// A resolved ride's step, by session: the run and step it held, and the
    /// notice events computed for it, which the step's notice carries.
    resolved_rides: Mutex<BTreeMap<String, ResolvedRide>>,
    /// The clock rewrite rides are timed by, in milliseconds: the scripted
    /// provider's delay and the ride's deadline both count on it. Each
    /// `pause` of the subject moves it [`RIDE_TICK_MS`].
    ride_clock: AtomicU64,
    /// Shared with [`FakeSubject::current_case`].
    current_case: Arc<Mutex<Option<&'static str>>>,
    /// Shared with [`FakeSubject::route_calls`].
    route_calls: RouteCalls,
}

/// Route requests in arrival order: the case marked current when each
/// arrived, and its method.
pub type RouteCalls = Arc<Mutex<Vec<(Option<&'static str>, String)>>>;

/// A rewrite ride that resolved: the run and step it held, and the notice
/// events computed for that step.
type ResolvedRide = (String, usize, Vec<String>);

impl World {
    fn now(&self) -> u64 {
        1_790_000_000_000
            + self.epoch.elapsed().as_millis() as u64
            + self.retention_clock_advanced_ms.load(Ordering::SeqCst)
    }
    fn serves(&self, group: &str) -> bool {
        self.groups.iter().any(|served| served == group)
    }

    /// Whether the fake numbers from a compaction provider's Setup
    /// checkpoint: it declares `tag_checkpoint` and the defect is off.
    fn honours_checkpoint(&self) -> bool {
        self.serves(groups::TAG_CHECKPOINT) && !self.defects.tags_setup_checkpoint_ignored
    }

    fn declares_mode(&self, mode: Mode) -> bool {
        let mut describe = RoleDescribe::default().with_capabilities(self.groups.clone());
        if !self.defects.order_modes_ignored {
            describe.delivery_order_modes = self.delivery_order_modes.clone();
        }
        mode.is_declared_by(&describe)
    }

    /// The scripted tool provider: count the call, and answer its scripted
    /// result unless the script holds it.
    fn invoke(&self, arguments: &Value) -> Option<Value> {
        let key = arguments.to_string();
        *self
            .invocations
            .lock()
            .unwrap()
            .entry(key.clone())
            .or_default() += 1;
        let scripts = self.scripts.lock().unwrap();
        let call = scripts
            .values()
            .flat_map(|script| script.tool_calls())
            .find(|call| &call.arguments == arguments)?;
        if call.hold && !self.released.lock().unwrap().contains(&key) {
            self.held.lock().unwrap().insert(key);
            return None;
        }
        Some(call.result.clone())
    }

    fn result_of(&self, arguments: &Value) -> Value {
        self.scripts
            .lock()
            .unwrap()
            .values()
            .flat_map(|script| script.tool_calls())
            .find(|call| &call.arguments == arguments)
            .map(|call| call.result.clone())
            .unwrap_or(Value::Null)
    }
}

struct Killed;

#[derive(Clone, Debug)]
struct Msg {
    ordinal: u64,
    mid: String,
    body: Value,
    /// The writing run's frozen cleaning mode, independent of later plans.
    writer: cortexkit_role_llm_runner::tags::TextWriter,
    /// The host's own copy, written at its save seam rather than on read.
    host_body: Value,
    run_id: String,
    /// The keys of the calls an assistant message carries, in order.
    calls: Vec<String>,
    /// For a message a recovery dropped: the numbers its parts showed in
    /// the lineage it was dropped from.
    left_numbers: Vec<(String, u64)>,
    /// For a host compaction marker: the message it cuts at.
    marker_cut: Option<String>,
    /// The checkpoint key the runner wrote on that marker, until the host
    /// drops it.
    marker_key: Option<u64>,
    /// For an entry written by a delivered `delivery_order` send: the mode
    /// it went out in, as the entry's `delivered_as`.
    delivered_as: Option<Value>,
    /// When the record that wrote the message was committed.
    written_at: u64,
    /// The `client_message_id` of the send that wrote this user message,
    /// kept beside its body.
    client_message_id: Option<String>,
    /// The route principal whose send wrote this user message, kept beside
    /// its body; `None` for a turn the host started.
    sender: Option<String>,
}

/// A `session.send` carrying a `delivery_order` list that the fake
/// accepted, queued until a boundary where one of its modes is possible.
#[derive(Clone, Debug)]
struct Queued {
    send_id: String,
    sender: String,
    text: String,
    modes: Vec<Mode>,
    dedupe_key: Option<String>,
    expires_at_ms: Option<u64>,
    /// The text is a plan template's rendering.
    system: bool,
    /// The request as received, for idempotency.
    request: Value,
    /// `pending`, `delivered`, `superseded` or `expired`.
    state: String,
    /// Once delivered: how, and into which run.
    delivery: Option<QueuedDelivery>,
}

/// How a queued `delivery_order` send was delivered, as its re-send answers
/// it: `step` with the stored row for a send that went into a running run,
/// `turn` with the run for one that started a run.
#[derive(Clone, Debug)]
struct QueuedDelivery {
    r#as: &'static str,
    r#ref: String,
    run_id: String,
}

#[derive(Clone, Debug)]
struct SendRec {
    prompt: String,
    delivery: String,
    run_id: String,
    row_id: String,
    steered_into_running: bool,
    admitted: bool,
    resend_count: usize,
    client_message_id: Option<String>,
}

#[derive(Clone, Debug)]
struct RunRec {
    directives: Vec<cortexkit_role_llm_runner::directives::DirectiveName>,
    max_steps: Option<u64>,
    max_output_tokens: Option<u64>,
    no_tools: bool,
    run_id: String,
    state: String,
    reason: Option<String>,
    final_ordinal: Option<u64>,
    final_text: Option<String>,
    error: Option<Value>,
    detail: Option<Value>,
    cancel_requested: bool,
    cancel_at: String,
    /// The run's frozen structured-output request with its enforcement, from
    /// the send that started it.
    structured_output: Option<Value>,
    /// The structured output its terminal record carries.
    structured_output_result: Option<Value>,
    /// The `{window, size, cause}` detail of a run that ended in the
    /// `context_ceiling` state.
    ceiling: Option<Value>,
    /// What the scripted model reported for the run's completed steps.
    usage: RunUsage,
}

#[derive(Clone, Debug)]
struct CallRec {
    tool_call_id: String,
    arguments: Value,
    run_id: String,
    intent: bool,
    result: bool,
}

#[derive(Clone, Debug)]
struct UserCommandRec {
    call_key: String,
    command: String,
    input: String,
    answer: Option<Value>,
    status: Option<Value>,
}

#[derive(Clone, Debug, Default)]
struct Sess {
    incarnation: Option<String>,
    session_usage: Option<cortexkit_role_llm_runner::usage::SessionUsage>,
    head_model: Option<cortexkit_role_llm_runner::read::HeadModel>,
    output_refs: output_refs::State,
    output_refs_enabled: bool,
    attachment_accepts: Option<bool>,
    ever_served: bool,
    standalone_handed_off: bool,
    compaction: compaction::State,
    retention: Option<u64>,
    input_cap: Option<u64>,
    input_spent: u64,
    input_unreported: bool,
    last_activity: u64,
    expired_at: Option<u64>,
    deleted: bool,
    title: Option<String>,
    owner: String,
    lineage: String,
    /// The identity of the plan the session runs under: the one the host
    /// applied at start, or the one the first send admitted.
    plan_digest: Option<String>,
    /// Every answered `session.adopt`, by `send_id`: the request and the
    /// reply an exact repeat replays.
    adopts: BTreeMap<String, (Value, Value)>,
    /// The session's admitted tool set, from the first send's plan.
    tools: Vec<String>,
    messages: Vec<Msg>,
    sends: BTreeMap<String, SendRec>,
    runs: Vec<RunRec>,
    calls: BTreeMap<String, CallRec>,
    user_commands: BTreeMap<String, UserCommandRec>,
    /// The kind of every record, in order: the durable control events.
    events: Vec<String>,
    /// The session's name, which a marker's checkpoint key names.
    name: String,
    /// Whether the session's frozen plan renders tags.
    render_tags: bool,
    /// The numbering in effect for the current lineage, by message id and
    /// part key (see `tags`).
    numbering: BTreeMap<tags::Key, u64>,
    /// The managed session this one was forked from, when the host forked
    /// it.
    forked_from: Option<String>,
    /// Messages, as they were written, that a recovery dropped or rewrote
    /// when it moved the session to a new lineage. Never read or resolved,
    /// except when `Defects::resolve_left_lineage_tags` is set.
    left: Vec<Msg>,
    /// The non-owner senders the session's admitted plan names.
    senders: Vec<order::PlanSender>,
    /// Accepted `delivery_order` sends, keyed by sender and `send_id`, and
    /// those keys in arrival order.
    ordered: BTreeMap<String, Queued>,
    queue: Vec<String>,
    /// Host-side metadata changes made outside the role.
    host_meta: u64,
    /// The run waiting at the context ceiling, and the time its wait ends
    /// (milliseconds since the Unix epoch).
    ceiling_wait: Option<(String, u64)>,
    /// The run whose step waits on a rewrite ride reported as
    /// `compaction_wait`, and the ride's deadline (milliseconds since the
    /// Unix epoch).
    compaction_wait: Option<(String, u64)>,
    /// What the `session_change` group's records built.
    change: session_change::ChangeState,
    /// The current lineage's first ordinal: the latest cut's first retained
    /// ordinal, or 0.
    first_ordinal: u64,
    /// Every cut still in the ancestry, oldest first.
    cuts: Vec<history::CutRec>,
    /// The ordinal of `messages[0]`: above 0 only after seeding moved the
    /// history behind a cut into `archive`.
    base: u64,
    /// Seeded messages behind the latest cut, kept out of `messages` so a
    /// step never touches them; only `session.history` reads them.
    archive: Arc<Vec<Msg>>,
    /// Lineages the session has left, newest last.
    previous_lineages: Vec<String>,
    /// The descent the next notice declares: the lineage a cut left, the
    /// cut's through ordinal and the digest of the message there.
    pending_descent: Option<(String, u64, String)>,
    /// Event tokens the next notice carries.
    notice_events: Vec<String>,
    /// The history version: raised at every lineage change but a cut.
    history_version: u64,
    /// The typed live events each record maps onto, by the record's index
    /// in `events`.
    live: Vec<(usize, Value)>,
}

impl Sess {
    fn run(&self, run_id: &str) -> Option<&RunRec> {
        self.runs.iter().find(|run| run.run_id == run_id)
    }

    fn run_mut(&mut self, run_id: &str) -> Option<&mut RunRec> {
        self.runs.iter_mut().find(|run| run.run_id == run_id)
    }

    /// The message at `ordinal`.
    fn at_mut(&mut self, ordinal: u64) -> &mut Msg {
        self.messages
            .iter_mut()
            .find(|m| m.ordinal == ordinal)
            .expect("a pushed message is held")
    }

    fn push(&mut self, body: Value, run_id: &str, calls: Vec<String>) -> u64 {
        let ordinal = self.base + self.messages.len() as u64;
        let writer = if self
            .run(run_id)
            .is_some_and(|run| run.structured_output.is_some())
        {
            cortexkit_role_llm_runner::tags::TextWriter::StructuredOutputFinal
        } else if self.render_tags {
            cortexkit_role_llm_runner::tags::TextWriter::TagRenderingRun
        } else {
            cortexkit_role_llm_runner::tags::TextWriter::UntaggedRun
        };
        self.messages.push(Msg {
            ordinal,
            mid: format!("{}.m{ordinal}", self.lineage),
            host_body: body.clone(),
            body,
            writer,
            run_id: run_id.to_owned(),
            calls,
            left_numbers: Vec::new(),
            marker_cut: None,
            marker_key: None,
            delivered_as: None,
            written_at: 0,
            client_message_id: None,
            sender: None,
        });
        ordinal
    }
}

fn point_of(kind: &str) -> &'static str {
    match kind {
        "start" => points::ADMITTED,
        "send" => points::SEND_RECORDED,
        "step" => points::STEP_RECORDED,
        "intent" => points::DISPATCH_INTENT,
        "result" => points::TOOL_RESULT_RECORDED,
        "terminal" => points::TERMINAL,
        "tombstone" => points::RETENTION_TOMBSTONED,
        "compaction_message" => points::COMPACTION_APPLIED,
        "compaction_fold" => points::FOLD_RECORDED,
        "user_command_forwarded" => points::USER_COMMAND_FORWARDED,
        _ => "",
    }
}

fn refuse(code: &str, detail: Option<Value>) -> Reply {
    let mut body = ErrorBody::new(code, format!("refused: {code}"));
    body.detail = detail;
    Reply::Error(body)
}

fn invalid(field: &str) -> Reply {
    refuse(errors::INVALID_PARAMS, Some(json!({ "field": field })))
}

fn respond<T: serde::Serialize>(value: T) -> Reply {
    Reply::Response(serde_json::to_value(value).expect("fake replies serialize"))
}

/// The fake's identity of a plan: a hash of its JSON text, so a refusal
/// names plans without carrying their content. A real runner uses the
/// identity admission uses; the suite only compares identities.
fn plan_digest(plan: &Map<String, Value>) -> String {
    use std::hash::{Hash, Hasher};
    let mut hasher = std::collections::hash_map::DefaultHasher::new();
    Value::Object(plan.clone()).to_string().hash(&mut hasher);
    format!("plan-{:016x}", hasher.finish())
}

fn baseline() -> Baseline {
    let mut capabilities = Map::new();
    capabilities.insert(groups::session::MID_SESSION_APPENDS.into(), json!(true));
    Baseline::new(
        vec![BaselineItem::new(TOOL_MODULE, "item-digest-1")],
        "composition-1",
        vec![SCRIPTED_TOOL.to_owned()],
    )
    .with_session_capabilities(capabilities)
}

/// One running incarnation: its log on disk and what it rebuilt from it.
pub struct Module {
    attachments: Mutex<attachments::Store>,
    /// Store a sent upload as a `data:` URL `file` part, the way a runner
    /// that keeps media inline in its own schema does, instead of the
    /// default stored media block.
    data_url_media: bool,
    world: Arc<World>,
    log: PathBuf,
    alive: AtomicBool,
    kill_hook: Mutex<Option<String>>,
    hook_fired: AtomicBool,
    sessions: Mutex<BTreeMap<String, Sess>>,
    /// Serialize user-command reservation with forwarding so concurrent retries
    /// cannot both pass the durable lookup before the invocation key is recorded.
    user_command_gate: Mutex<()>,
    /// The module was opened over a log an earlier incarnation wrote: it is
    /// a restart.
    replayed: AtomicBool,
}

impl Module {
    fn sweep(&self, session: &str) -> Result<(), Killed> {
        let state = self.sessions.lock().unwrap().get(session).cloned();
        let Some(state) = state else {
            return Ok(());
        };
        if state.expired_at.is_some() {
            self.finish_deletion(session);
            return Ok(());
        }
        if self.world.retention_defect == RetentionDefect::Ignore {
            return Ok(());
        }
        if self.world.retention_defect != RetentionDefect::ExpireHeld
            && state
                .runs
                .iter()
                .any(|run| run.state == "active" || run.state == "paused")
        {
            return Ok(());
        }
        if let Some(seconds) = state.retention {
            let expiry = state.last_activity + seconds * 1000;
            if self.world.now() >= expiry {
                self.commit(session, json!({"kind":"tombstone", "expired_at_ms":expiry}))?;
                self.finish_deletion(session);
            }
        }
        Ok(())
    }

    fn finish_deletion(&self, session: &str) {
        let state = self.sessions.lock().unwrap().get(session).cloned();
        let Some(state) = state else {
            return;
        };
        if state.expired_at.is_none()
            || state.deleted
            || self.world.retention_defect == RetentionDefect::NeverPurge
        {
            return;
        }
        // The fake really removes the session's disk records as well as its
        // replay projection, retaining a content-free tombstone and completion.
        let text = std::fs::read_to_string(&self.log).unwrap_or_default();
        let mut kept: Vec<Value> = text
            .lines()
            .map(|line| serde_json::from_str(line).unwrap())
            .filter(|r: &Value| r["session"] != json!(session) || r["kind"] == json!("tombstone"))
            .collect();
        let mut record = json!({"kind":"deleted", "session":session, "at":self.world.now()});
        if self.world.retention_defect == RetentionDefect::TitleLeak {
            record["title"] = json!(state.title);
        }
        if self.world.retention_defect == RetentionDefect::PromptLeak {
            record["leaked"] = json!(state
                .messages
                .iter()
                .find(|m| m.body["role"] == "user")
                .map(|m| m.body.clone()));
        }
        if self.world.retention_defect == RetentionDefect::ToolLeak {
            record["leaked"] = json!(state
                .messages
                .iter()
                .find(|m| m.body["role"] == "tool")
                .map(|m| m.body.clone()));
        }
        kept.push(record.clone());
        let mut file = std::fs::File::create(&self.log).unwrap();
        for value in kept {
            writeln!(file, "{value}").unwrap();
        }
        self.apply(&record);
    }

    fn expiry_reply(&self, session: &str) -> Option<Reply> {
        let sessions = self.sessions.lock().unwrap();
        let expiry = sessions.get(session)?.expired_at?;
        if self.world.retention_defect == RetentionDefect::Empty {
            return None;
        }
        Some(refuse(
            errors::EXPIRED,
            Some(
                json!({"expired_at_ms":expiry + u64::from(self.world.retention_defect == RetentionDefect::Timestamp)}),
            ),
        ))
    }

    fn listing(&self, session: &str) -> Reply {
        let sessions = self.sessions.lock().unwrap();
        let title = sessions.get(session).and_then(|s| s.title.clone());
        let records: Vec<Value> = std::fs::read_to_string(&self.log)
            .unwrap_or_default()
            .lines()
            .map(|line| serde_json::from_str(line).unwrap())
            .filter(|r: &Value| r["session"] == json!(session))
            .collect();
        respond(
            json!({"title":title, "leaked":records.iter().filter_map(|r| r.get("leaked")).collect::<Vec<_>>()}),
        )
    }

    fn open(root: &Path, world: Arc<World>) -> Result<Arc<Self>, HarnessError> {
        std::fs::create_dir_all(root).map_err(|e| HarnessError::new(e.to_string()))?;
        let module = Arc::new(Self {
            attachments: Mutex::new(attachments::Store::open(root)),
            data_url_media: world.data_url_media,
            world,
            log: root.join("records.jsonl"),
            alive: AtomicBool::new(true),
            kill_hook: Mutex::new(None),
            hook_fired: AtomicBool::new(false),
            sessions: Mutex::new(BTreeMap::new()),
            user_command_gate: Mutex::new(()),
            replayed: AtomicBool::new(false),
        });
        if let Ok(text) = std::fs::read_to_string(&module.log) {
            module
                .replayed
                .store(!text.trim().is_empty(), Ordering::SeqCst);
            for line in text.lines() {
                let record: Value = serde_json::from_str(line)
                    .map_err(|e| HarnessError::new(format!("corrupt log: {e}")))?;
                module.apply(&record);
            }
        }
        let _ = module.resume();
        let pending: Vec<String> = module
            .sessions
            .lock()
            .unwrap()
            .iter()
            .filter(|(_, s)| s.expired_at.is_some() && !s.deleted)
            .map(|(n, _)| n.clone())
            .collect();
        for name in pending {
            if module.world.retention_defect == RetentionDefect::TombstoneForgotten {
                let mut sessions = module.sessions.lock().unwrap();
                sessions.get_mut(&name).unwrap().expired_at = None;
                sessions.get_mut(&name).unwrap().retention = None;
            } else {
                module.finish_deletion(&name);
            }
        }
        Ok(module)
    }

    /// On restart, record terminal `outcome_unknown` for reserved commands with
    /// no stored reply, then resume runs interrupted by a kill. The durable log
    /// stores each reservation beside session state, so it survives a process
    /// restart.
    fn resume(&self) -> Result<(), Killed> {
        let unfinished_commands: Vec<(String, String, UserCommandRec)> = self
            .sessions
            .lock()
            .unwrap()
            .iter()
            .flat_map(|(session, sess)| {
                sess.user_commands
                    .iter()
                    .filter(|(_, command)| command.answer.is_none())
                    .map(|(invocation, command)| {
                        (session.clone(), invocation.clone(), command.clone())
                    })
            })
            .collect();
        for (session, invocation, command) in unfinished_commands {
            let state = if self.world.defects.user_command == Some("report_failed") {
                "failed"
            } else {
                cortexkit_role_llm_runner::user_commands::OUTCOME_UNKNOWN
            };
            let text = if state == cortexkit_role_llm_runner::user_commands::OUTCOME_UNKNOWN {
                "The command may have run, but its reply was lost. Check before trying again."
            } else {
                "The runner failed before it could determine whether the command ran."
            };
            self.commit(
                &session,
                json!({
                    "kind":"user_command_result",
                    "invocation":invocation,
                    "answer":{"call_key":command.call_key,"reply":{"state":state,"text":text}},
                    "status":{"state":state,"text":text,"progress":[]}
                }),
            )?;
        }
        let cut: Vec<(String, String)> = self
            .sessions
            .lock()
            .unwrap()
            .iter()
            .flat_map(|(name, sess)| {
                sess.runs
                    .iter()
                    .filter(|run| run.state == "active")
                    .map(move |run| (name.clone(), run.run_id.clone()))
            })
            .collect();
        for (session, run_id) in cut {
            self.commit(&session, json!({ "kind": "resume" }))?;
            if self.seal_compaction_setup_crash(&session, &run_id)? {
                continue;
            }
            let open: Vec<String> = self.sessions.lock().unwrap()[&session]
                .calls
                .iter()
                .filter(|(_, call)| call.run_id == run_id && call.intent && !call.result)
                .map(|(key, _)| key.clone())
                .collect();
            if open.is_empty() {
                let pending: Vec<(String, Value)> = self.sessions.lock().unwrap()[&session]
                    .calls
                    .iter()
                    .filter(|(_, call)| call.run_id == run_id && !call.intent)
                    .map(|(key, call)| (key.clone(), call.arguments.clone()))
                    .collect();
                if !pending.is_empty() && self.world.step_recovery != StepRecovery::Resume {
                    if matches!(
                        self.world.step_recovery,
                        StepRecovery::SealDropStep | StepRecovery::SealDropCall
                    ) {
                        self.commit(
                            &session,
                            json!({ "kind": "drop_step", "run_id": run_id,
                            "keep_text": self.world.step_recovery == StepRecovery::SealDropCall }),
                        )?;
                    }
                    // Keeping the call closes it with a never-executed result
                    // in the same record that ends the run, as a hard stop does.
                    let keep = self.world.step_recovery == StepRecovery::SealInterrupted;
                    let defects = &self.world.defects;
                    let sealed: Vec<&String> = pending
                        .iter()
                        .map(|(key, _)| key)
                        .filter(|_| {
                            keep && !defects.sealed_call_without_result
                                && !defects.sealed_result_in_later_commit
                        })
                        .collect();
                    self.commit(
                        &session,
                        json!({ "kind": "terminal", "run_id": run_id,
                        "state": "interrupted", "sealed": sealed,
                        "rewrite_read": keep && defects.sealed_rewrites_read_message }),
                    )?;
                } else {
                    if self.world.defects.dispatch_twice_on_resume {
                        for (_, arguments) in pending {
                            self.world.invoke(&arguments);
                        }
                    }
                    self.drive(&session, &run_id)?;
                }
                continue;
            }
            for key in open {
                self.commit(
                    &session,
                    json!({ "kind": "result", "call_key": key, "content": null,
                            "reason": errors::tool_result_reasons::OUTCOME_UNKNOWN }),
                )?;
            }
            let state = if self.world.defects.cancel_cut_runs {
                "cancelled"
            } else {
                "interrupted"
            };
            self.commit(
                &session,
                json!({ "kind": "terminal", "run_id": run_id, "state": state }),
            )?;
        }
        Ok(())
    }

    /// Append one record to the log, then apply it. A kill armed for the
    /// record's point takes effect right after the record is written: nothing
    /// later is written or done. The fake never syncs: its kill only stops this
    /// process's state, and the replay reads the log back through the same OS,
    /// so a written record survives every crash these tests simulate. A real
    /// runner must sync before it replies.
    fn commit(&self, session: &str, mut record: Value) -> Result<(), Killed> {
        if !self.alive.load(Ordering::SeqCst) {
            return Err(Killed);
        }
        if matches!(record["kind"].as_str(), Some("start" | "fork"))
            && record.get("incarnation").is_none()
        {
            record["incarnation"] = json!(new_incarnation());
        }
        if record["kind"] == "cut" && self.world.defects.incarnation_changes_on_cut {
            record["incarnation_after_cut"] = json!(new_incarnation());
        }
        if record["kind"] == "step" && self.world.serves(groups::OUTPUT_REFS) {
            if let Some(sess) = self.sessions.lock().unwrap().get(session) {
                record["output_sets"] = output_refs::capture(sess, &record, &self.world.defects);
            }
        }
        if self.world.serves(groups::INTERRUPT_REASON)
            && record["kind"] == "terminal"
            && record["state"] == "interrupted"
            && record.get("reason").is_none()
        {
            record["reason"] =
                json!(cortexkit_role_llm_runner::run::interrupt_reasons::RUNNER_CRASH);
        }
        record["session"] = json!(session);
        record["at"] = json!(self.world.now());
        let mut file = OpenOptions::new()
            .create(true)
            .append(true)
            .open(&self.log)
            .expect("fake log opens");
        writeln!(file, "{record}").expect("fake log writes");
        let point = point_of(record["kind"].as_str().unwrap_or_default());
        if !point.is_empty() && self.kill_hook.lock().unwrap().as_deref() == Some(point) {
            self.alive.store(false, Ordering::SeqCst);
            self.hook_fired.store(true, Ordering::SeqCst);
            return Err(Killed);
        }
        self.apply(&record);
        Ok(())
    }

    fn apply(&self, record: &Value) {
        let text = |key: &str| record[key].as_str().unwrap_or_default().to_owned();
        let session = text("session");
        let kind = text("kind");
        let mut sessions = self.sessions.lock().unwrap();
        if kind == "start" {
            // A session started again over an expired one keeps raising its
            // history version: it never decreases.
            let history_version = sessions
                .get(&session)
                .map_or(0, |old| old.history_version + 1);
            sessions.insert(
                session.clone(),
                Sess {
                    incarnation: record["incarnation"].as_str().map(str::to_owned),
                    owner: text("owner"),
                    lineage: text("lineage"),
                    plan_digest: record["plan_digest"].as_str().map(str::to_owned),
                    tools: serde_json::from_value(record["tools"].clone()).unwrap_or_default(),
                    senders: serde_json::from_value(record["senders"].clone()).unwrap_or_default(),
                    render_tags: record["render_tags"] == json!(true),
                    output_refs_enabled: record["output_refs_enabled"] == json!(true),
                    name: session.clone(),
                    history_version,
                    ..Sess::default()
                },
            );
        }
        if kind == "start" {
            if let Some(sess) = sessions.get_mut(&session) {
                history::adopt_host_cut(sess, &self.world);
            }
        }
        if kind == "fork" {
            // The host copied the parent's history into the fork, as
            // OpenCode's fork copies every part.
            if let Some(parent) = sessions.get(&text("parent")).cloned() {
                sessions.insert(
                    session.clone(),
                    Sess {
                        forked_from: Some(text("parent")),
                        incarnation: record["incarnation"].as_str().map(str::to_owned),
                        ..parent
                    },
                );
            }
        }
        let Some(sess) = sessions.get_mut(&session) else {
            return;
        };
        sess.events.push(kind.clone());
        let messages_before: BTreeSet<String> =
            sess.messages.iter().map(|m| m.mid.clone()).collect();
        session_change::apply(sess, &kind, record);
        compaction::apply(&mut sess.compaction, record, self.world.defects.compaction);
        if matches!(kind.as_str(), "send" | "steer" | "step" | "terminal")
            && (self.world.retention_defect != RetentionDefect::ClockNotRestarted
                || sess.last_activity == 0)
        {
            sess.last_activity = record["at"].as_u64().unwrap_or_default();
        }
        match kind.as_str() {
            "head-model-switch" => {
                sess.head_model = Some(serde_json::from_value(record["model"].clone()).unwrap());
            }
            "usage-rewind" => {
                if let Some(last) = sess.runs.last() {
                    let id = last.run_id.clone();
                    sess.messages.retain(|message| message.run_id != id);
                }
                sess.history_version += 1;
                sess.lineage = format!("{}-rewound", sess.lineage);
                if self.world.defects.session_usage_reset_on_rewind {
                    if let Some(usage) = &mut sess.session_usage {
                        usage.totals = RunUsage::new();
                    }
                }
            }
            "host_meta" => sess.host_meta += 1,
            "host_compact" => {
                // The host compacts natively, keeping the last exchange: its
                // marker cuts at the newest user message, and the runner
                // writes its checkpoint key on the marker in the same write.
                let cut = sess
                    .messages
                    .iter()
                    .rev()
                    .find(|m| m.body["role"] == "user" && m.marker_cut.is_none())
                    .map(|m| m.mid.clone())
                    .unwrap_or_default();
                let key = tags::checkpoint_before(sess, &cut);
                let ordinal = sess.push(
                    json!({ "role": "user", "compaction": { "tail_start_id": cut } }),
                    "",
                    Vec::new(),
                );
                let marker = sess.at_mut(ordinal);
                marker.marker_cut = Some(cut);
                marker.marker_key = Some(key);
                sess.push(
                    json!({ "role": "assistant", "summary": "the history before the cut" }),
                    "",
                    Vec::new(),
                );
            }
            "drop_marker_key" => {
                if let Some(marker) = sess
                    .messages
                    .iter_mut()
                    .rev()
                    .find(|m| m.marker_cut.is_some())
                {
                    marker.marker_key = None;
                }
            }
            "ordered" => {
                let id = text("id");
                sess.queue.push(id.clone());
                sess.ordered.insert(
                    id,
                    Queued {
                        send_id: text("send_id"),
                        sender: text("sender"),
                        text: text("text"),
                        modes: serde_json::from_value(record["modes"].clone()).unwrap_or_default(),
                        dedupe_key: record["dedupe_key"].as_str().map(str::to_owned),
                        expires_at_ms: record["expires_at_ms"].as_u64(),
                        system: record["system"] == json!(true),
                        request: record["request"].clone(),
                        state: "pending".into(),
                        delivery: None,
                    },
                );
            }
            "order_end" => {
                if let Some(queued) = sess.ordered.get_mut(&text("id")) {
                    queued.state = text("state");
                    // A send delivered by starting a run records that run.
                    if let Some(run_id) = record["run_id"].as_str() {
                        queued.delivery = Some(QueuedDelivery {
                            r#as: delivered_as::TURN,
                            r#ref: run_id.to_owned(),
                            run_id: run_id.to_owned(),
                        });
                    }
                }
            }
            "order_delivered" => {
                let id = text("id");
                let Some(queued) = sess.ordered.get_mut(&id) else {
                    return;
                };
                queued.state = "delivered".into();
                let role = if record["delivered_as"]["mode"] == order::modes::TURN_SYSTEM {
                    "system"
                } else {
                    "user"
                };
                let mut body = json!({ "role": role, "text": queued.text });
                attachments::add_parts(&mut body, record);
                let sender = queued.sender.clone();
                let ordinal = sess.push(body, &text("run_id"), Vec::new());
                let message = sess.at_mut(ordinal);
                message.delivered_as = Some(record["delivered_as"].clone());
                message.sender = Some(sender);
                let row = message.mid.clone();
                if let Some(queued) = sess.ordered.get_mut(&id) {
                    queued.delivery = Some(QueuedDelivery {
                        r#as: delivered_as::STEP,
                        r#ref: row,
                        run_id: text("run_id"),
                    });
                }
            }
            "ceiling_wait" => {
                sess.ceiling_wait = Some((text("run_id"), record["deadline_ms"].as_u64().unwrap()));
            }
            "ceiling_view" => sess.ceiling_wait = None,
            "compaction_wait" => {
                sess.compaction_wait =
                    Some((text("run_id"), record["deadline_ms"].as_u64().unwrap()));
            }
            "ride_resolved" => sess.compaction_wait = None,
            "adopt" => {
                sess.owner = text("owner");
                sess.adopts.insert(
                    text("send_id"),
                    (record["request"].clone(), record["reply"].clone()),
                );
            }
            "standalone_handoff" => sess.standalone_handed_off = true,
            "require_setup" => sess.compaction = compaction::State::default(),
            "user_command_reservation" => {
                sess.user_commands.insert(
                    text("invocation"),
                    UserCommandRec {
                        call_key: text("call_key"),
                        command: text("command"),
                        input: text("input"),
                        answer: None,
                        status: None,
                    },
                );
            }
            "user_command_result" => {
                if let Some(command) = sess.user_commands.get_mut(&text("invocation")) {
                    command.answer = Some(record["answer"].clone());
                    command.status = Some(record["status"].clone());
                }
            }
            "user_command_forwarded" => {}
            "cancel" => {
                if let Some(run) = sess.run_mut(&text("run_id")) {
                    run.cancel_requested = true;
                    run.cancel_at = text("cancel_at");
                }
            }
            "unattributed" => {
                sess.push(
                    json!({"role":"user", "text":text("marker")}),
                    "",
                    Vec::new(),
                );
            }
            "tombstone" => {
                sess.expired_at = record["expired_at_ms"].as_u64();
            }
            "deleted" => {
                sess.deleted = true;
                sess.messages.clear();
                sess.calls.clear();
                sess.user_commands.clear();
                sess.sends.clear();
                sess.runs.clear();
                sess.title = record["title"].as_str().map(str::to_owned);
            }
            "send" | "steer" => {
                if let Some(accepts) = record["attachment_accepts"].as_bool() {
                    sess.attachment_accepts = Some(accepts);
                }
                if let Some(limit) = record["input_cap"].as_u64() {
                    sess.input_cap = Some(limit);
                }
                if let Some(retention) = record["retention"].as_u64() {
                    sess.retention = Some(retention);
                }
                if let Some(title) = record["title"].as_str() {
                    sess.title = Some(title.into());
                }
                let run_id = text("run_id");
                if kind == "steer"
                    && self.world.defects.tags_renumber_mid_step
                    && sess.render_tags
                    && sess
                        .calls
                        .values()
                        .any(|call| call.run_id == run_id && !call.result)
                {
                    // Renumbers at once, while the step's call is running.
                    sess.lineage = format!("{}-t", sess.lineage);
                    sess.history_version += 1;
                    sess.numbering.clear();
                }
                let mut body = if profiled(&session) {
                    json!({ "role": "user", "content": [{ "type": "text", "text": text("prompt") }] })
                } else {
                    json!({ "role": "user", "text": text("prompt") })
                };
                attachments::add_parts(&mut body, record);
                let ordinal = sess.push(body, &run_id, Vec::new());
                sess.at_mut(ordinal).client_message_id =
                    record["client_message_id"].as_str().map(str::to_owned);
                sess.at_mut(ordinal).sender = record["sender"].as_str().map(str::to_owned);
                if let Some(delivered) = record.get("delivered_as").filter(|d| !d.is_null()) {
                    sess.at_mut(ordinal).delivered_as = Some(delivered.clone());
                }
                let row_id = sess.at_mut(ordinal).mid.clone();
                sess.sends.insert(
                    text("send_id"),
                    SendRec {
                        prompt: text("prompt"),
                        delivery: text("delivery"),
                        run_id: run_id.clone(),
                        row_id,
                        steered_into_running: kind == "steer",
                        admitted: record["admitted"] == json!(true),
                        resend_count: 0,
                        client_message_id: record["client_message_id"].as_str().map(str::to_owned),
                    },
                );
                if kind == "send" && record["no_run"] != json!(true) {
                    sess.session_usage.get_or_insert_with(|| {
                        cortexkit_role_llm_runner::usage::SessionUsage::new(&run_id)
                    });
                    sess.runs.push(RunRec {
                        directives: serde_json::from_value(record["directives"].clone())
                            .unwrap_or_default(),
                        max_steps: record["max_steps"].as_u64(),
                        max_output_tokens: record["max_output_tokens"].as_u64(),
                        no_tools: record["no_tools"] == true,
                        run_id,
                        state: "active".into(),
                        reason: None,
                        final_ordinal: None,
                        final_text: None,
                        error: None,
                        detail: None,
                        cancel_requested: false,
                        cancel_at: String::new(),
                        structured_output: record
                            .get("structured_output")
                            .filter(|frozen| !frozen.is_null())
                            .cloned(),
                        structured_output_result: None,
                        ceiling: None,
                        usage: RunUsage::new(),
                    });
                }
            }
            "step" => {
                if !sess.standalone_handed_off {
                    sess.ever_served = true;
                }
                if let Some(spent) = record["input_spent"]
                    .as_u64()
                    .filter(|_| !sess.input_unreported)
                {
                    sess.input_spent = spent;
                }
                if record["input_spent"].is_null() {
                    sess.input_unreported = true;
                }
                let run_id = text("run_id");
                let reported = &record["usage"];
                let mut step = StepUsage::unreported();
                if !reported.is_null() {
                    let count = |field: &str| reported[field].as_u64().unwrap_or_default();
                    step = step
                        .with_input_tokens(count("input_tokens"))
                        .with_cached_input_tokens(count("cached_input_tokens"))
                        .with_cache_write_tokens(count("cache_write_tokens"))
                        .with_output_tokens(count("output_tokens"));
                    if let Some(reasoning) = reported["reasoning_tokens"].as_u64() {
                        step = step.with_reasoning_tokens(reasoning);
                    }
                    // A provider that billed reasoning as extra output adds
                    // it on top of the output count instead of reporting a
                    // breakdown of it.
                    if self.world.defects.usage_reasoning_added_to_output {
                        if let Some(reasoning) = reported["reasoning_tokens"].as_u64() {
                            let output = count("output_tokens") + reasoning;
                            step = step.with_output_tokens(output);
                        }
                    }
                }
                if let Some(run) = sess.run_mut(&run_id) {
                    run.usage.record_step(FAKE_PROVIDER, FAKE_MODEL, None, step);
                }
                if let Some(usage) = &mut sess.session_usage {
                    usage.record_step(FAKE_PROVIDER, FAKE_MODEL, None, step);
                }
                let mut keys = Vec::new();
                for call in record["calls"].as_array().into_iter().flatten() {
                    let key = call["call_key"].as_str().unwrap_or_default().to_owned();
                    keys.push(key.clone());
                    sess.calls.insert(
                        key,
                        CallRec {
                            tool_call_id: call["tool_call_id"].as_str().unwrap_or_default().into(),
                            arguments: call["arguments"].clone(),
                            run_id: run_id.clone(),
                            intent: false,
                            result: false,
                        },
                    );
                }
                // The model request this step answers was made at a step
                // boundary: a renumbering detected since takes effect now.
                tags::settle(sess, &self.world.defects, self.world.honours_checkpoint());
                let body = if profiled(&session) {
                    anthropic_assistant(&record["parts"])
                } else {
                    json!({ "role": "assistant", "parts": record["parts"] })
                };
                sess.push(body, &run_id, keys);
                tags::save_in_host(sess.messages.last_mut().unwrap(), &self.world.defects);
            }
            "drop_step" => {
                let run_id = text("run_id");
                // Dropping a written message starts a new lineage, so no
                // reader sees changed bytes under the old one.
                if !self.world.defects.drop_keeps_lineage {
                    sess.lineage = format!("{}-r", sess.lineage);
                    sess.history_version += 1;
                }
                sess.calls.retain(|_, call| call.run_id != run_id);
                let shown = tags::current(sess);
                let dropped: Vec<Msg> = sess
                    .messages
                    .iter()
                    .filter(|m| m.run_id == run_id && !m.calls.is_empty())
                    .cloned()
                    .map(|mut m| {
                        m.left_numbers = shown
                            .iter()
                            .filter(|((mid, _), _)| mid == &m.mid)
                            .map(|((_, part), n)| (part.clone(), *n))
                            .collect();
                        m
                    })
                    .collect();
                sess.left.extend(dropped);
                if record["keep_text"] == json!(true) {
                    for message in sess
                        .messages
                        .iter_mut()
                        .filter(|m| m.run_id == run_id && !m.calls.is_empty())
                    {
                        message.calls.clear();
                        if let Some(parts) = message.body["parts"].as_array_mut() {
                            parts.retain(|part| part["type"] != "tool_call");
                        }
                    }
                } else {
                    sess.messages
                        .retain(|m| m.run_id != run_id || m.calls.is_empty());
                }
                // The new lineage numbers afresh.
                sess.numbering.clear();
            }
            "intent" => {
                if let Some(call) = sess.calls.get_mut(&text("call_key")) {
                    call.intent = true;
                }
            }
            "result" => {
                let key = text("call_key");
                let run_id = sess
                    .calls
                    .get(&key)
                    .map(|call| call.run_id.clone())
                    .unwrap_or_default();
                if let Some(call) = sess.calls.get_mut(&key) {
                    call.result = true;
                }
                if self.world.defects.run_tool_calls_embedded_results
                    || self.world.defects.run_tool_calls_omit_finished_arguments
                {
                    let arguments = &sess.calls[&key].arguments;
                    let message = sess
                        .messages
                        .iter_mut()
                        .find(|message| message.calls.contains(&key))
                        .unwrap();
                    let part = message.body["parts"]
                        .as_array_mut()
                        .unwrap()
                        .iter_mut()
                        .find(|part| part["type"] == "tool_call" && part["arguments"] == *arguments)
                        .unwrap();
                    if self.world.defects.run_tool_calls_embedded_results {
                        part["result"] = record["content"].clone();
                    }
                    if self.world.defects.run_tool_calls_omit_finished_arguments {
                        part.as_object_mut().unwrap().remove("arguments");
                    }
                }
                if self.world.defects.run_tool_calls_embedded_results {
                    return;
                }
                let mut body = if profiled(&session) {
                    let id = sess
                        .calls
                        .get(&key)
                        .map(|c| c.tool_call_id.clone())
                        .unwrap_or_default();
                    json!({ "role": "user", "content": [{ "type": "tool_result", "tool_use_id": id,
                            "content": result_text(&record["content"]) }] })
                } else {
                    json!({ "role": "tool", "call_key": key, "content": record["content"] })
                };
                if let Some(reason) = record.get("reason") {
                    body["reason"] = reason.clone();
                }
                if self.world.serves(groups::ATTACHMENTS) {
                    attachments::ingest(self, &mut body);
                }
                sess.push(body, &run_id, Vec::new());
            }
            "terminal" => {
                let run_id = text("run_id");
                for key in record["sealed"].as_array().into_iter().flatten() {
                    let key = key.as_str().unwrap_or_default();
                    if let Some(call) = sess.calls.get_mut(key) {
                        call.result = true;
                    }
                    sess.push(
                        json!({"role":"tool", "call_key":key, "content":{"executed":false}}),
                        &run_id,
                        Vec::new(),
                    );
                }
                if record["rewrite_read"] == json!(true) {
                    if let Some(first) = sess.messages.first_mut() {
                        first.body["text"] = json!(format!("{} (rewritten)", first.body["text"]));
                    }
                }
                for blocked in record["blocked"].as_array().into_iter().flatten() {
                    let key = blocked["call_key"].as_str().unwrap();
                    if let Some(call) = sess.calls.get_mut(key) {
                        call.result = true;
                    }
                    sess.push(
                        json!({"role":"tool", "tool_call_id":blocked["tool_call_id"],
                        "content":{"executed":false,"error":{"code":record["state"]}}}),
                        &run_id,
                        Vec::new(),
                    );
                }
                if let Some(submitted) = record.get("submitted") {
                    if let Some(call) = sess.calls.get_mut(submitted["call_key"].as_str().unwrap())
                    {
                        call.result = true;
                    }
                    sess.push(
                        json!({"role":"tool", "tool_call_id":submitted["tool_call_id"],
                        "content":{"executed":true,"submitted":true}}),
                        &run_id,
                        Vec::new(),
                    );
                }
                for skipped in record["skipped"].as_array().into_iter().flatten() {
                    if let Some(call) = sess.calls.get_mut(skipped["call_key"].as_str().unwrap()) {
                        call.result = true;
                    }
                    sess.push(
                        json!({"role":"tool", "tool_call_id":skipped["tool_call_id"],
                        "content":{"executed":false,"error":{"code":errors::tool_result_reasons::SKIPPED_AFTER_SUBMIT}}}),
                        &run_id,
                        Vec::new(),
                    );
                }
                let final_ordinal = sess
                    .messages
                    .iter()
                    .rev()
                    .find(|m| m.run_id == run_id && m.body["role"] == "assistant")
                    .map(|m| m.ordinal);
                sess.ceiling_wait = None;
                sess.compaction_wait = None;
                tags::settle(sess, &self.world.defects, self.world.honours_checkpoint());
                if let Some(run) = sess.run_mut(&run_id) {
                    run.state = text("state");
                    run.reason = record["reason"].as_str().map(str::to_owned);
                    run.ceiling = record.get("context_ceiling").cloned();
                    run.error = record.get("error").cloned();
                    run.detail = record.get("detail").cloned();
                    run.structured_output_result = record.get("structured_output").cloned();
                    if run.state == "completed" {
                        run.final_ordinal = final_ordinal;
                        run.final_text = record["final_text"].as_str().map(str::to_owned);
                    }
                }
            }
            "cut" | "revert" | "seed" | "view_applied" | "view_disposed" | "notice" => {
                history::apply(sess, &kind, record, &self.world.defects);
                if let Some(value) = record["incarnation_after_cut"].as_str() {
                    sess.incarnation = Some(value.into());
                }
            }
            "legacy_call" => {
                // A finished call from history stored before the runner kept
                // tool-call times: the call and its result, with nothing
                // recording when either happened.
                let key = text("call_key");
                let id = record["tool_call_id"].as_str().unwrap_or_default();
                let parts = json!([{ "type": "tool_call", "id": id, "tool": record["tool"],
                                     "arguments": record["arguments"] }]);
                sess.calls.insert(
                    key.clone(),
                    CallRec {
                        tool_call_id: id.into(),
                        arguments: record["arguments"].clone(),
                        run_id: String::new(),
                        intent: true,
                        result: true,
                    },
                );
                let (call, result) = if profiled(&session) {
                    (
                        anthropic_assistant(&parts),
                        json!({ "role": "user", "content": [{ "type": "tool_result",
                                "tool_use_id": id, "content": result_text(&record["content"]) }] }),
                    )
                } else {
                    (
                        json!({ "role": "assistant", "parts": parts }),
                        json!({ "role": "tool", "call_key": key, "content": record["content"] }),
                    )
                };
                sess.push(call, "", vec![key]);
                sess.push(result, "", Vec::new());
            }
            _ => {}
        }
        let at = record["at"].as_u64().unwrap_or_default();
        for message in sess.messages.iter_mut().filter(|m| m.written_at == 0) {
            message.written_at = at;
        }
        live_events(sess, &kind, record, &messages_before, &self.world.defects);
        tags::note_write(sess, &self.world.defects, self.world.honours_checkpoint());
        output_refs::apply(sess, &kind, record, &self.world.defects);
    }

    /// Continue run `run_id` of `session`: dispatch the calls of its last
    /// step, then ask the model for the next turn, and repeat until the run
    /// ends or the tool provider holds a call.
    fn drive(&self, session: &str, run_id: &str) -> Result<(), Killed> {
        loop {
            let sess = self.sessions.lock().unwrap()[session].clone();
            if sess.run(run_id).is_none_or(|run| run.state != "active") {
                return Ok(());
            }
            if let Some(limit) = sess.input_cap {
                if sess.input_unreported {
                    return self.commit(session, json!({"kind":"terminal", "run_id":run_id,
                        "state":"budget_exhausted", "detail":{"limit":limit,"spent":sess.input_spent,"cause":"usage_unreported"}}));
                }
                if sess.input_spent >= limit {
                    return self.commit(session, json!({"kind":"terminal", "run_id":run_id,
                        "state":"budget_exhausted", "detail":{"limit":limit,"spent":sess.input_spent,"cause":"limit_reached"}}));
                }
            }
            let mine: Vec<(&String, &CallRec)> = sess
                .calls
                .iter()
                .filter(|(_, call)| call.run_id == run_id)
                .collect();
            if let Some((key, call)) = mine.iter().find(|(_, call)| !call.intent) {
                self.commit(session, json!({ "kind": "intent", "call_key": key }))?;
                if self.world.held_first_reply(&call.arguments, key) {
                    return Ok(());
                }
                match self.world.invoke(&call.arguments) {
                    Some(content) => self.commit(
                        session,
                        json!({ "kind": "result", "call_key": key, "content": content }),
                    )?,
                    None => return Ok(()),
                }
                continue;
            }
            if mine.iter().any(|(_, call)| call.intent && !call.result) {
                return Ok(());
            }
            if sess.run(run_id).is_none_or(|run| run.state != "active") {
                return Ok(());
            }
            let turn_index = sess.events.iter().filter(|kind| *kind == "step").count();
            let model_started = self.world.model_calls.lock().unwrap().contains(&(
                session.into(),
                run_id.into(),
                turn_index,
            ));
            if !model_started
                && sess.run(run_id).is_some_and(|run| {
                    run.cancel_requested
                        && !(run.cancel_at == "step_boundary"
                            && self.world.defects.cancel_starts_next_step)
                })
            {
                return self.commit(
                    session,
                    json!({"kind":"terminal", "run_id":run_id, "state":"cancelled"}),
                );
            }
            if self.world.ceiling_holds.lock().unwrap().contains(session) {
                // The request would reach the ceiling: wait for a view.
                if sess.ceiling_wait.is_none() {
                    self.commit(
                        session,
                        json!({ "kind": "ceiling_wait", "run_id": run_id,
                                "deadline_ms": self.world.now() + CEILING_WAIT_MS }),
                    )?;
                }
                return Ok(());
            }
            if self.deliver_at_step(session, run_id)? {
                continue;
            }
            history::step_cost(&sess, &self.world.defects);
            // The model consumed a turn even if recovery removed it from history.
            if !self.prepare_compaction(session, run_id, turn_index)? {
                return Ok(());
            }
            let turn = self
                .world
                .scripts
                .lock()
                .unwrap()
                .get(session)
                .and_then(|script| script.turns.get(turn_index).cloned());
            let Some(mut turn) = turn else {
                return self.commit(
                    session,
                    json!({ "kind": "terminal", "run_id": run_id, "state": "error" }),
                );
            };
            let received = self.world.model_calls.lock().unwrap().insert((
                session.into(),
                run_id.into(),
                turn_index,
            ));
            let frozen = sess
                .run(run_id)
                .and_then(|run| run.structured_output.clone());
            let run = sess.run(run_id).unwrap();
            let output_cap = run
                .max_output_tokens
                .filter(|_| !self.world.defects.max_output_tokens_ignored)
                .map(|cap| cap.min(32));
            if received {
                self.capture_command_model_input(session, &sess);
                let tools = if run.no_tools && !self.world.defects.no_tools_ignored {
                    &[][..]
                } else {
                    &sess.tools[..]
                };
                let mut observation =
                    observe_request(tools, frozen.as_ref()).with_rendered(tags::rendered_texts(
                        &sess,
                        &self.world.defects,
                        self.world.honours_checkpoint(),
                    ));
                observation.max_output_tokens = output_cap;
                self.world
                    .model_requests
                    .lock()
                    .unwrap()
                    .entry(session.into())
                    .or_default()
                    .push(observation);
            }
            if !model_started
                && self.world.defects.cancel_starts_next_step
                && sess
                    .run(run_id)
                    .is_some_and(|run| run.cancel_requested && run.cancel_at == "step_boundary")
            {
                // The extra request reached the model, but its answer is discarded.
                return self.commit(
                    session,
                    json!({"kind":"terminal", "run_id":run_id, "state":"cancelled"}),
                );
            }
            if self
                .world
                .pending_model_holds
                .lock()
                .unwrap()
                .remove(session)
            {
                self.world
                    .held_models
                    .lock()
                    .unwrap()
                    .insert(session.into(), run_id.into());
            }
            if self.world.held_models.lock().unwrap().contains_key(session) {
                if let Some(parts) = self.world.stream_holds.lock().unwrap().remove(session) {
                    let mid = format!(
                        "{}.m{}",
                        sess.lineage,
                        sess.base + sess.messages.len() as u64
                    );
                    let streamed = turn.parts.iter().take(parts).cloned().collect();
                    self.world
                        .in_flight
                        .lock()
                        .unwrap()
                        .insert(session.into(), (mid, streamed));
                }
                return Ok(());
            }
            // The output-cap fixture is text-only. Its fake provider uses
            // whitespace-delimited tokens and one allowance across all parts,
            // reporting the number returned rather than merely clamping usage.
            if let Some(cap) = output_cap {
                if turn
                    .parts
                    .iter()
                    .all(|part| matches!(part, ScriptedPart::Text(_) | ScriptedPart::Reasoning(_)))
                {
                    let mut remaining = cap as usize;
                    for part in &mut turn.parts {
                        if let ScriptedPart::Text(text) | ScriptedPart::Reasoning(text) = part {
                            let tokens: Vec<_> = text.split_whitespace().take(remaining).collect();
                            remaining -= tokens.len();
                            *text = tokens.join(" ");
                        }
                    }
                    if let Some(usage) = &mut turn.usage {
                        usage.output_tokens = cap - remaining as u64;
                    }
                }
            }
            let ordinal = sess.messages.len();
            let mut parts = Vec::new();
            let mut calls = Vec::new();
            let mut tools = Vec::new();
            for (index, part) in turn.parts.iter().enumerate() {
                match part {
                    ScriptedPart::Text(text) => parts.push(json!({ "type": "text", "text": text })),
                    ScriptedPart::Reasoning(text) => {
                        parts.push(json!({ "type": "reasoning", "text": text }))
                    }
                    ScriptedPart::ToolCall(call) => {
                        let key = if self.world.defects.call_key_from_model_id {
                            format!("key-{}", call.tool_call_id)
                        } else {
                            format!("{}.{ordinal}.{index}", sess.lineage)
                        };
                        parts.push(json!({ "type": "tool_call", "id": call.tool_call_id,
                                           "tool": call.tool, "arguments": call.arguments }));
                        calls.push(json!({ "call_key": key, "tool_call_id": call.tool_call_id,
                                           "arguments": call.arguments }));
                        tools.push(call.tool.clone());
                    }
                }
            }
            let no_calls = calls.is_empty();
            // The scripted model reports the counts each turn states, with
            // reasoning only where a turn states it; a turn without usage
            // reports nothing.
            let usage = turn.usage.as_ref().map(|usage| {
                let mut reported = json!({ "input_tokens": usage.input_tokens,
                        "cached_input_tokens": usage.cache_read_tokens,
                        "cache_write_tokens": usage.cache_write_tokens,
                        "output_tokens": usage.output_tokens });
                if let Some(reasoning) = usage.reasoning_tokens {
                    reported["reasoning_tokens"] = json!(reasoning);
                }
                reported
            });
            let input_spent = turn.usage.as_ref().map(|usage| {
                sess.input_spent
                    .saturating_add(usage.input_tokens)
                    .saturating_add(usage.cache_read_tokens)
                    .saturating_add(usage.cache_write_tokens)
            });
            self.commit(
                session,
                json!({ "kind": "step", "run_id": run_id, "parts": parts, "calls": calls,
                        "input_spent": input_spent, "usage": usage }),
            )?;
            // The limit cuts the run off only when this step asked for a
            // tool call that would lead to another step. A step with no call
            // completes the run, and one calling `submit_result` ends it the
            // way `submit_result` always does, below.
            let submits = frozen.as_ref().is_some_and(|f| f["mode"] == "submit_tool")
                && tools.iter().any(|tool| tool == SUBMIT_RESULT_TOOL);
            let leads_to_another_step = !no_calls && !submits;
            if run
                .max_steps
                .is_some_and(|limit| run.usage.steps + 1 >= limit)
                && !self.world.defects.max_steps_ignored
                && (leads_to_another_step || self.world.defects.max_steps_on_final_step)
            {
                let blocked = if self.world.defects.max_steps_unsealed {
                    vec![]
                } else {
                    calls.clone()
                };
                return self.commit(
                    session,
                    json!({"kind":"terminal", "run_id":run_id,
                        "state":"max_steps", "blocked":blocked}),
                );
            }
            if let Some(limit) = sess.input_cap {
                let cause = match input_spent {
                    None => Some("usage_unreported"),
                    Some(spent) if spent >= limit => Some("limit_reached"),
                    Some(_) => None,
                };
                if let Some(cause) = cause {
                    return self.commit(session, json!({"kind":"terminal", "run_id":run_id,
                        "state":"budget_exhausted", "blocked":calls,
                        "detail":{"limit":limit,"spent":input_spent.unwrap_or(sess.input_spent),"cause":cause}}));
                }
            }
            let text: String = turn.text_parts().collect();
            if let Some(frozen) = frozen.as_ref().filter(|f| f["mode"] == "submit_tool") {
                if let Some(first) = tools.iter().position(|tool| tool == SUBMIT_RESULT_TOOL) {
                    // No call of a submitting step is dispatched: the submit
                    // ends the run, and every other call is skipped.
                    let skipped: Vec<Value> = calls
                        .iter()
                        .enumerate()
                        .filter(|(index, _)| *index != first)
                        .map(|(index, call)| {
                            json!({"call_key":call["call_key"], "tool_call_id":call["tool_call_id"],
                                   "tool":tools[index], "arguments":call["arguments"]})
                        })
                        .collect();
                    if self.world.defects.structured_output_dispatches_siblings {
                        for call in skipped.iter().filter(|c| c["tool"] == SCRIPTED_TOOL) {
                            self.world.invoke(&call["arguments"]);
                        }
                    }
                    let siblings = skipped
                        .iter()
                        .map(|c| {
                            SkippedSibling::new(
                                c["tool_call_id"].as_str().unwrap(),
                                c["tool"].as_str().unwrap(),
                            )
                        })
                        .collect();
                    let output = judge(
                        frozen,
                        Some(Ok(calls[first]["arguments"].clone())),
                        siblings,
                    );
                    return self.commit(
                        session,
                        json!({ "kind": "terminal", "run_id": run_id, "state": "completed",
                                "final_text": text, "skipped": skipped,
                                "submitted": {"call_key":calls[first]["call_key"],
                                              "tool_call_id":calls[first]["tool_call_id"]},
                                "structured_output": output }),
                    );
                }
            }
            if no_calls {
                let mut record = json!({ "kind": "terminal", "run_id": run_id, "state": "completed",
                            "final_text": text });
                if let Some(frozen) = &frozen {
                    let produced = (frozen["mode"] == "final_text")
                        .then(|| serde_json::from_str::<Value>(&text).map_err(|_| ()));
                    record["structured_output"] = json!(judge(frozen, produced, Vec::new()));
                }
                return self.commit(session, record);
            }
        }
    }

    /// The first possible declared member of a wire list at `boundary`; a
    /// legacy string keeps its equivalent modes. The defect picks the last
    /// possible member instead.
    fn choose(&self, modes: &[Mode], boundary: &order::Boundary, list_form: bool) -> Option<Mode> {
        let declared: Vec<Mode> = modes
            .iter()
            .copied()
            .filter(|mode| !list_form || self.world.declares_mode(*mode))
            .collect();
        if self.world.defects.order_last_possible_mode {
            declared
                .iter()
                .rev()
                .copied()
                .find(|mode| boundary.possible(*mode))
        } else {
            order::choose(&declared, boundary)
        }
    }

    /// Drop a queued send whose time has passed. Answers whether it did.
    fn expire(&self, session: &str, id: &str, queued: &Queued) -> Result<bool, Killed> {
        if self.world.defects.order_expiry_ignored
            || !order::expired(queued.expires_at_ms, self.world.now())
        {
            return Ok(false);
        }
        self.commit(
            session,
            json!({ "kind": "order_end", "id": id, "state": "expired" }),
        )?;
        Ok(true)
    }

    /// The boundary before run `run_id`'s next model request: deliver every
    /// queued send whose list has a mode possible there. Answers whether any
    /// was delivered.
    fn deliver_at_step(&self, session: &str, run_id: &str) -> Result<bool, Killed> {
        if !self.world.defects.order_before_staging
            && self.world.staging_holds.lock().unwrap().contains(session)
        {
            return Ok(false);
        }
        let sess = self.sessions.lock().unwrap()[session].clone();
        let result_key = sess
            .messages
            .last()
            .filter(|m| m.body["role"] == "tool")
            .and_then(|m| m.body["call_key"].as_str())
            .map(str::to_owned);
        let mut delivered = false;
        for id in &sess.queue {
            let queued = &sess.ordered[id];
            if queued.state != "pending" || self.expire(session, id, queued)? {
                continue;
            }
            let mut boundary = order::Boundary::default().with_run_in_progress();
            if result_key.is_some() {
                boundary = boundary.with_tool_result();
            }
            if queued.system && self.world.turn_system {
                boundary = boundary.with_turn_system();
            }
            let Some(mode) = self.choose(
                &queued.modes,
                &boundary,
                queued.request[DELIVERY_FIELD].is_array(),
            ) else {
                continue;
            };
            let mut delivered_as = json!({ "mode": mode.as_str() });
            if mode == Mode::ToolResult {
                delivered_as["call_key"] = json!(result_key);
            }
            self.commit(
                session,
                json!({ "kind": "order_delivered", "id": id, "run_id": run_id,
                        "attachment_parts": attachments::queued_parts(self, session, &queued.request),
                        "delivered_as": delivered_as }),
            )?;
            delivered = true;
        }
        Ok(delivered)
    }

    /// The boundary of an idle session: a queued send whose list names
    /// `idle_start` starts a run, one at a time.
    fn deliver_idle(&self, session: &str) -> Result<(), Killed> {
        loop {
            let Some(sess) = self.sessions.lock().unwrap().get(session).cloned() else {
                return Ok(());
            };
            if sess.runs.iter().any(|run| run.state == "active") {
                return Ok(());
            }
            let mut started = None;
            for id in &sess.queue {
                let queued = &sess.ordered[id];
                if queued.state != "pending" || self.expire(session, id, queued)? {
                    continue;
                }
                let idle = order::Boundary::default().with_idle();
                let mut chosen = self.choose(
                    &queued.modes,
                    &idle,
                    queued.request[DELIVERY_FIELD].is_array(),
                );
                if chosen.is_none() && self.world.defects.order_starts_runs_without_idle_start {
                    chosen = Some(Mode::IdleStart);
                }
                if chosen == Some(Mode::IdleStart) {
                    started = Some((
                        id.clone(),
                        queued.text.clone(),
                        queued.send_id.clone(),
                        queued.sender.clone(),
                    ));
                    break;
                }
            }
            let Some((id, text, cause, sender)) = started else {
                return Ok(());
            };
            let run_id = format!("run-{}", sess.runs.len() + 1);
            self.commit(
                session,
                json!({ "kind": "order_end", "id": id, "state": "delivered", "run_id": run_id }),
            )?;
            // The run is keyed internally by the queue entry, but its cause
            // is the send's own `send_id`, as `run_started` names it.
            self.commit(
                session,
                json!({ "kind": "send", "send_id": format!("order:{id}"), "prompt": text,
                        "attachment_parts": attachments::queued_parts(self, session, &sess.ordered[&id].request),
                        "delivery": groups::QUEUE, "run_id": run_id, "admitted": false,
                        "cause": cause, "sender": sender,
                        "delivered_as": { "mode": order::modes::IDLE_START } }),
            )?;
            self.drive(session, &run_id)?;
        }
    }

    /// Record the scripted result of the held call with `arguments`, then
    /// continue its run.
    fn complete_held(&self, arguments: &Value) -> Result<(), Killed> {
        let found = self
            .sessions
            .lock()
            .unwrap()
            .iter()
            .find_map(|(name, sess)| {
                sess.calls
                    .iter()
                    .find(|(_, call)| &call.arguments == arguments && call.intent && !call.result)
                    .map(|(key, call)| (name.clone(), key.clone(), call.run_id.clone()))
            });
        let Some((session, key, run_id)) = found else {
            return Ok(());
        };
        let content = self.world.result_of(arguments);
        self.commit(
            &session,
            json!({ "kind": "result", "call_key": key, "content": content }),
        )?;
        self.drive(&session, &run_id)?;
        self.deliver_idle(&session)
    }

    fn handle(
        &self,
        stamp: &RouteStamp,
        session: &str,
        method: &str,
        params: Value,
    ) -> Result<Reply, RouteFailure> {
        if !self.alive.load(Ordering::SeqCst) {
            return Err(RouteFailure::new("the module is not running"));
        }
        if matches!(
            method,
            ops::SESSION_SEND | ops::SESSION_REFRESH | ops::SESSION_ADOPT
        ) {
            if let Some(plan) = params.get("plan") {
                if self.world.defects.output_refs != Some("output_refs_unsupported_plan") {
                    if let Err(field) = cortexkit_role_llm_runner::output_refs::check_plan(
                        plan,
                        self.world.serves(groups::OUTPUT_REFS),
                    ) {
                        if self.world.defects.output_refs
                            == Some("output_refs_unsupported_plan_written")
                        {
                            self.commit(session,json!({"kind":"start","owner":stamp.principal,"lineage":"invalid-plan-written"})).map_err(|Killed|RouteFailure::new("killed before refusing"))?;
                        }
                        return Ok(invalid(field));
                    }
                }
            }
        }
        self.sweep(session)
            .map_err(|Killed| RouteFailure::new("the module was killed"))?;
        self.tick_ride(session)
            .map_err(|Killed| RouteFailure::new("the module was killed"))?;
        if matches!(
            method,
            ops::SESSION_READ
                | ops::SESSION_HEAD
                | ops::RUN_RESULT
                | ops::RUN_CANCEL
                | ops::SESSION_RETRACT
                | "run.status"
        ) {
            if let Some(reply) = self.expiry_reply(session) {
                if method != "run.status" || self.world.retention_defect != RetentionDefect::Status
                {
                    return Ok(reply);
                }
            }
        }
        if matches!(
            method,
            ops::SESSION_READ | ops::SESSION_HEAD | ops::SESSION_RESOLVE
        ) && history::reads_held(self, session)
        {
            return Ok(refuse(errors::TRANSIENT, None));
        }
        let reply = match method {
            ops::SESSION_OUTPUT_REFS | ops::SESSION_OUTPUT_REF_READ
                if self.world.serves(groups::OUTPUT_REFS) =>
            {
                Ok(output_refs::handle(self, stamp, session, method, &params))
            }
            ops::SESSION_UPLOAD_BEGIN
            | ops::SESSION_UPLOAD_APPEND
            | ops::SESSION_UPLOAD_STATUS
            | ops::SESSION_UPLOAD_COMMIT
            | ops::SESSION_MEDIA_READ
                if self.world.serves(groups::ATTACHMENTS) =>
            {
                Ok(attachments::handle(self, stamp, session, method, params))
            }
            "session.list" | "session.export" => Ok(self.listing(session)),
            "run.status" if self.world.advertise_status => Ok(self.run_status(session, params)),
            ops::SESSION_RESOLVE if self.world.serves(groups::TAGS) => {
                Ok(self.resolve(session, &params))
            }
            ops::RUN_TOOL_CALLS if self.world.serves(groups::RUN_TOOL_CALLS) => {
                Ok(self.tool_calls(session, &params))
            }
            ops::SEND_STATUS if self.world.serves(groups::SEND_STATUS) => {
                Ok(self.send_status(stamp, session, &params))
            }
            ops::SESSION_READ | ops::SESSION_HEAD => {
                self.world.transcript_calls.fetch_add(1, Ordering::SeqCst);
                if !self.world.serves(groups::TRANSCRIPT_READS) {
                    // A runner that does not declare transcript_reads does
                    // not serve its ops.
                    Ok(refuse("unknown_method", Some(json!({ "method": method }))))
                } else if method == ops::SESSION_READ {
                    Ok(self.read(session, &params))
                } else {
                    Ok(self.head(stamp, session, &params))
                }
            }
            ops::SESSION_HISTORY if self.world.serves(groups::HOST_HISTORY) => {
                Ok(self.history(session, &params))
            }
            ops::ROLE_DESCRIBE => Ok(self.describe()),
            ops::SESSION_BASELINE => Ok(self.baseline(stamp, session, &params)),
            ops::SESSION_USER_COMMAND | ops::SESSION_USER_COMMAND_STATUS => {
                self.user_command(stamp, session, method, params)
            }
            ops::SESSION_SEND => self.send(stamp, session, params),
            ops::SESSION_REFRESH | ops::SESSION_REFRESH_POLICY | ops::SESSION_FLUSH_PREFIX
                if self.world.serves(groups::SESSION_CHANGE) =>
            {
                self.session_change(stamp, session, method, params)
            }
            ops::SESSION_ADOPT => {
                self.world.adopt_calls.fetch_add(1, Ordering::SeqCst);
                if self.world.serves(groups::ADOPT) {
                    self.adopt(stamp, session, params)
                } else {
                    // A runner that does not declare adopt does not serve
                    // its op.
                    Ok(refuse("unknown_method", Some(json!({ "method": method }))))
                }
            }
            ops::SESSION_RETRACT if self.world.serves(groups::RETRACT) => {
                self.retract(stamp, session, params)
            }
            ops::RUN_CANCEL if self.world.serves(groups::RUN_CANCEL) => {
                self.cancel(stamp, session, params)
            }
            ops::RUN_RESULT => {
                self.world.run_result_calls.fetch_add(1, Ordering::SeqCst);
                if self.world.serves(groups::RUN_OPS) {
                    Ok(self.run_result(session, params))
                } else {
                    // A runner that does not declare run_ops does not
                    // serve its ops.
                    Ok(refuse("unknown_method", Some(json!({ "method": method }))))
                }
            }
            other => Ok(refuse("unknown_method", Some(json!({ "method": other })))),
        };
        reply.map_err(|Killed| RouteFailure::new("the module was killed"))
    }

    fn cancel(&self, stamp: &RouteStamp, session: &str, params: Value) -> Result<Reply, Killed> {
        let request: cortexkit_role_llm_runner::run::RunCancelRequest =
            match serde_json::from_value(params) {
                Ok(request) => request,
                Err(_) => return Ok(invalid("run_id")),
            };
        let sess = self.sessions.lock().unwrap().get(session).cloned();
        let Some(sess) = sess else {
            return Ok(respond("not_active"));
        };
        if stamp.principal != sess.owner {
            return Ok(refuse(errors::SCOPE_OWNER_MISMATCH, None));
        }
        let at = request.at();
        if self.world.cancel_step_boundary
            && !matches!(at, "now" | "step_boundary")
            && !self.world.defects.cancel_accepts_unlisted_at
        {
            return Ok(invalid("at"));
        }
        if sess.run(&request.run_id).is_none_or(|run| {
            cortexkit_role_llm_runner::run::RunState::parse(&run.state).is_terminal()
        }) {
            return Ok(respond("not_active"));
        }
        if self.world.defects.cancel_ignores_escalation
            && at == "now"
            && sess
                .run(&request.run_id)
                .is_some_and(|run| run.cancel_at == "step_boundary")
        {
            return Ok(respond("ack"));
        }
        self.commit(
            session,
            json!({"kind":"cancel", "run_id":request.run_id, "cancel_at":at}),
        )?;
        let held_model = self.world.held_models.lock().unwrap().contains_key(session);
        if held_model && at != "step_boundary" {
            self.commit(
                session,
                json!({"kind":"terminal", "run_id":request.run_id, "state":"cancelled"}),
            )?;
        }
        self.drive(session, &request.run_id)?;
        Ok(respond("ack"))
    }

    /// `session.adopt`, checked in the order the contract pins: the caller,
    /// then the `send_id`, then the plan's identity against the one the
    /// session runs under. It records the owner and the reply, and writes no
    /// message and starts no run.
    fn adopt(&self, stamp: &RouteStamp, session: &str, params: Value) -> Result<Reply, Killed> {
        if let Some(reply) = self.refuse_fork(session) {
            return Ok(reply);
        }
        if let Some(field) = unknown_field(&params, &["send_id", PLAN_FIELD]) {
            return Ok(invalid(&field));
        }
        let request: AdoptRequest = match serde_json::from_value(params) {
            Ok(request) => request,
            Err(error) if error.to_string().contains(PLAN_FIELD) => return Ok(invalid(PLAN_FIELD)),
            Err(_) => return Ok(invalid("send_id")),
        };
        let sess = self.sessions.lock().unwrap().get(session).cloned();
        if let Some(sess) = &sess {
            if !sess.owner.is_empty()
                && sess.owner != stamp.principal
                && !self.world.defects.adopt_takes_over_owner
            {
                return Ok(refuse(
                    errors::SCOPE_OWNER_MISMATCH,
                    Some(json!({ "accepted": sess.owner })),
                ));
            }
            if let Some((earlier, reply)) = sess.adopts.get(&request.send_id) {
                let earlier: AdoptRequest =
                    serde_json::from_value(earlier.clone()).expect("stored adopt requests decode");
                return Ok(match request.reused_field(&earlier) {
                    Some(field) => refuse(errors::SEND_ID_REUSE, Some(json!({ "field": field }))),
                    None => Reply::Response(reply.clone()),
                });
            }
        }
        let sent = plan_digest(&request.plan);
        let applied = sess.as_ref().and_then(|sess| sess.plan_digest.as_deref());
        if let Err(detail) = check_drift(applied, &sent) {
            return Ok(refuse(
                errors::PLAN_DRIFT,
                Some(serde_json::to_value(detail).unwrap()),
            ));
        }
        let mut reply = AdoptReply::new(sent);
        if let Some(last) = sess.as_ref().and_then(|sess| sess.messages.last()) {
            reply = reply.with_last_ordinal(last.ordinal);
        }
        let reply = serde_json::to_value(reply).unwrap();
        self.commit(
            session,
            json!({ "kind": "adopt", "owner": stamp.principal, "send_id": request.send_id,
                    "request": request, "reply": reply }),
        )?;
        if self.world.defects.adopt_writes_message {
            self.commit(
                session,
                json!({ "kind": "unattributed", "marker": "adopted" }),
            )?;
        }
        Ok(Reply::Response(reply))
    }

    /// Start `session` as a person's app would: the plan applied, no owner
    /// recorded, and, with `prompt`, the person's first turn run as far as
    /// the scripted model and tools let it.
    fn host_start(
        &self,
        session: &str,
        plan: &Map<String, Value>,
        prompt: Option<&str>,
    ) -> Result<(), Killed> {
        let tools = plan.get("tools").cloned().unwrap_or(json!([]));
        self.commit(
            session,
            json!({ "kind": "start", "owner": "", "lineage": format!("lin-{session}"),
                    "tools": tools, "plan_digest": plan_digest(plan),
                    "render_tags": plan.get(cortexkit_role_llm_runner::tags::PLAN_RENDER_TAGS) }),
        )?;
        let Some(prompt) = prompt else {
            return Ok(());
        };
        let run_id = "run-1";
        self.commit(
            session,
            json!({ "kind": "send", "send_id": format!("host-{session}"), "prompt": prompt,
                    "delivery": groups::QUEUE, "run_id": run_id, "admitted": false,
                    "no_run": self.world.defects.host_turns_without_run }),
        )?;
        self.drive(session, run_id)
    }

    fn describe(&self) -> Reply {
        let mut capabilities = self.world.groups.clone();
        if self.world.defects.claim_unserved_group {
            capabilities.push(groups::INTERRUPT.to_owned());
        }
        let mut served: Vec<String> = vec![ops::ROLE_DESCRIBE.into(), ops::SESSION_BASELINE.into()];
        if self.world.serves(groups::RETENTION) {
            served.extend(["session.list".into(), "session.export".into()]);
        }
        if self.world.advertise_status {
            served.push("run.status".into());
        }
        for group in &capabilities {
            for op in groups::group_ops(group).unwrap_or_default() {
                if !served.iter().any(|s| s == op) {
                    served.push((*op).to_owned());
                }
            }
        }
        let mut describe =
            RoleDescribe::new(vec![Major::new(PROVIDES, served, "alpha")], "fake-runner-1")
                .with_capabilities(capabilities)
                .with_session_capabilities_from(groups::source::ADMISSION)
                .with_max_bytes(DEFAULT_MAX_BYTES, MAXIMUM_MAX_BYTES);
        if self.world.steer_receipt_confirm {
            describe = describe.with_steer_receipt("confirm");
        }
        describe.delivery_order_modes = self.world.delivery_order_modes.clone();
        if self.world.serves(groups::RETENTION) {
            describe = describe.with_retention(RETENTION_MAX, self.world.retention_delete_ms);
        }
        if self.world.serves(groups::RUN_CANCEL) && self.world.cancel_step_boundary {
            describe = describe.with_run_cancel(vec!["now".into(), "step_boundary".into()]);
        }
        respond(describe)
    }

    fn baseline(&self, stamp: &RouteStamp, session: &str, params: &Value) -> Reply {
        if let Some(field) = unknown_field(params, &[]) {
            return invalid(&field);
        }
        let owner = self
            .sessions
            .lock()
            .unwrap()
            .get(session)
            .map(|sess| sess.owner.clone());
        match owner {
            None => respond(BaselineReply::not_yet()),
            Some(owner) if owner != stamp.principal => refuse(
                errors::SCOPE_OWNER_MISMATCH,
                Some(json!({ "accepted": owner })),
            ),
            Some(_) => respond(BaselineReply::ready(
                self.change_baseline(session, self.command_baseline(session)),
            )),
        }
    }

    fn send_reply(&self, session: &str, send: &SendRec) -> Reply {
        let sessions = self.sessions.lock().unwrap();
        let run = sessions[session]
            .run(&send.run_id)
            .expect("a send's run exists");
        // The defect answers a run that has ended as still active, on the
        // second retry only, so the first and second retries differ.
        let state_moved = self.world.defects.retry_state_moves_after_end
            && send.delivery == groups::QUEUE
            && send.resend_count == 2;
        let mut reply = if run.state == "active" || state_moved {
            SendReply::new("active").with_run_id(&run.run_id)
        } else {
            SendReply::new("finished")
                .with_run_id(&run.run_id)
                .with_reason(&run.state)
        };
        let defects = &self.world.defects;
        if send.delivery == groups::STEER && send.steered_into_running {
            if !self.world.serves(groups::RUN_OPS) {
                reply.run_id = None;
            }
            if defects.steer_reply_state_step {
                reply.state = delivered_as::STEP.to_owned();
            }
        }
        if send.admitted {
            // At admission nothing is accepted or applied yet: the baseline
            // adds only the rungs a session_change runner states.
            let mut admitted = baseline();
            if self.world.serves(groups::SESSION_CHANGE) {
                admitted = admitted.with_rungs(session_change::fake_rungs(defects));
            }
            reply = reply.with_baseline(admitted);
        }
        if defects.retry_new_submission_id {
            reply = reply.with_submission_id(format!("sub-{}-{}", run.run_id, send.resend_count));
        }
        if send.delivery == groups::QUEUE {
            // The send itself is answered with the first receipt, every
            // retry with the second.
            let receipts = if defects.retry_delivered_step_then_pending {
                Some((delivered_as::STEP, delivered_as::PENDING))
            } else if defects.retry_delivered_unknown_then_step {
                Some((delivered_as::UNKNOWN, delivered_as::STEP))
            } else if self.world.queue_receipt_pending_then_unknown {
                Some((delivered_as::PENDING, delivered_as::UNKNOWN))
            } else {
                None
            };
            if let Some((sent, retried)) = receipts {
                let r#as = if send.resend_count == 0 {
                    sent
                } else {
                    retried
                };
                reply = reply.with_delivered(Delivered::new(r#as));
            }
        }
        // The tags cases' own steers into held turns (on sessions rendering
        // tags) stay out of the held-steer receipt fixtures below, which
        // count and break only the guaranteed-steer case's steers. They are
        // answered without a receipt, which a guaranteed runner may do.
        if send.delivery == groups::STEER && !sessions[session].render_tags {
            let held_turn = send.steered_into_running
                && run.state == "active"
                && sessions[session]
                    .calls
                    .values()
                    .any(|call| call.run_id == run.run_id && call.intent && !call.result);
            let held_pending = held_turn
                && ((defects.held_steer_pending && send.resend_count == 0)
                    || (defects.held_steer_retry_pending && send.resend_count > 0));
            let held_unstable =
                held_turn && defects.held_steer_retry_unstable && send.resend_count > 0;
            let after_release = send.steered_into_running && run.state != "active";
            let final_absent = after_release && defects.held_steer_final_absent;
            let final_unstable = after_release && defects.held_steer_after_release_unstable;
            if held_pending || held_unstable || final_absent || final_unstable {
                self.world
                    .held_steer_break_answers
                    .fetch_add(1, Ordering::SeqCst);
            }
            if held_pending || self.world.defects.guaranteed_steer_pending {
                reply = reply.with_delivered(Delivered::new(delivered_as::PENDING));
            } else if final_absent {
                // Deliberately leave the settled re-send without a receipt.
            } else if held_unstable || final_unstable {
                reply = reply.with_delivered(
                    Delivered::new(delivered_as::STEP)
                        .with_ref(format!("{}-unstable", send.row_id)),
                );
            } else if held_turn {
                // A real guaranteed runner cannot yet know which step or turn
                // will render the steer. The unstable-receipt fixtures seed
                // a receipt early so the check must compare its reference.
                if defects.held_steer_retry_unstable || defects.held_steer_after_release_unstable {
                    reply = reply
                        .with_delivered(Delivered::new(delivered_as::STEP).with_ref(&send.row_id));
                } else if let Some(receipt) = &self.world.early_held_steer_receipt {
                    reply = reply.with_delivered(receipt.clone());
                } else if !self.world.serves(groups::RUN_OPS) {
                    reply = reply
                        .with_delivered(Delivered::new(delivered_as::STEP).with_ref(&send.row_id));
                }
            } else if let Some(receipt) = self
                .world
                .early_held_steer_receipt
                .as_ref()
                .filter(|_| send.steered_into_running)
            {
                reply = reply.with_delivered(receipt.clone());
            } else if !send.steered_into_running
                && send.resend_count > 1
                && self.world.defects.resend_steer_unstable
            {
                reply = reply.with_delivered(
                    Delivered::new(delivered_as::TURN).with_ref(format!("{}-unstable", run.run_id)),
                );
            } else if !self.world.omit_first_steer_receipt || send.resend_count > 0 {
                let receipt = if send.steered_into_running {
                    Delivered::new(delivered_as::STEP).with_ref(&send.row_id)
                } else {
                    Delivered::new(delivered_as::TURN).with_ref(&run.run_id)
                };
                reply = reply.with_delivered(receipt);
            }
            if send.steered_into_running {
                self.world
                    .held_steer_receipts
                    .lock()
                    .unwrap()
                    .push(reply.delivered.clone());
            }
        }
        respond(reply)
    }

    /// A fork of a managed session is never enrolled: its sends and its
    /// adoption are refused `fork_unsupported`, naming the parent.
    fn refuse_fork(&self, session: &str) -> Option<Reply> {
        let parent = self
            .sessions
            .lock()
            .unwrap()
            .get(session)
            .and_then(|sess| sess.forked_from.clone())?;
        (!self.world.defects.fork_accepted).then(|| {
            refuse(
                errors::FORK_UNSUPPORTED,
                Some(json!(errors::ForkUnsupportedDetail::new(Some(parent)))),
            )
        })
    }

    fn send(&self, stamp: &RouteStamp, session: &str, params: Value) -> Result<Reply, Killed> {
        if let Some(reply) = self.admit_system_text_default(session, &params) {
            return Ok(reply);
        }
        if let Some(reply) = self.refuse_fork(session) {
            return Ok(reply);
        }
        if let Some(reply) = self.command_collision(session) {
            return Ok(reply);
        }
        if params
            .get("retention")
            .is_some_and(|value| !value.is_null() && value.as_u64().is_none())
        {
            return Ok(invalid("retention"));
        }
        if let Some(delivery) = params.get(DELIVERY_FIELD) {
            // A list is decoded below, and refused there on a runner without
            // the group; any other value must be a known mode.
            if !delivery.is_array()
                && !matches!(delivery.as_str(), Some("queue" | "steer" | "interrupt"))
            {
                return Ok(invalid(DELIVERY_FIELD));
            }
        }
        let listed = params.get(DELIVERY_FIELD).is_some_and(Value::is_array);
        let mut request: SendRequest = match serde_json::from_value(params) {
            Ok(request) => request,
            Err(_) if listed => return Ok(invalid(DELIVERY_FIELD)),
            Err(_) => return Ok(invalid("prompt")),
        };
        let attachment_parts = match attachments::admit_send(self, session, &request) {
            Ok(parts) => parts,
            Err(reply) => return Ok(reply),
        };
        // How this runner's schema stores a sent upload; recorded with the
        // send so a replay writes the same parts it wrote the first time.
        let stored_as = if self.world.data_url_media {
            json!("data_url")
        } else {
            json!("media_block")
        };
        if let Err(field) = request.check_delivery_order(&self.world.groups) {
            if !self.world.defects.order_list_accepted_without_group {
                return Ok(invalid(field));
            }
        }
        if let Err(field) = request.client_message_id(&self.world.groups) {
            return Ok(invalid(field));
        }
        // A send that is not a list send must carry a non-empty prompt; a
        // list send's prompt is checked with its members below.
        if request.delivery_order.is_none() || !self.world.serves(groups::DELIVERY_ORDER) {
            if let Err(field) = request.check_prompt() {
                return Ok(invalid(field));
            }
        }
        if self.world.serves(groups::DELIVERY_ORDER) {
            match request.delivery_order() {
                Err(_)
                    if request.delivery_order.is_none()
                        && self.world.defects.order_members_without_list_accepted => {}
                Err(_)
                    if request.prompt.as_deref() == Some("")
                        && self.world.defects.order_template_with_empty_prompt_accepted =>
                {
                    // The defect reads a template send's empty prompt as no
                    // prompt at all.
                    request.prompt = None;
                    let wanted = request
                        .delivery_order()
                        .expect("only the prompt was wrong")
                        .expect("a list send");
                    return self.ordered_send(stamp, session, &request, wanted);
                }
                Err(field) => return Ok(invalid(field)),
                Ok(Some(wanted)) => return self.ordered_send(stamp, session, &request, wanted),
                Ok(None) => {}
            }
        }
        let delivery = request.delivery().as_str().to_owned();
        let old = self.sessions.lock().unwrap().get(session).cloned();
        let mut existing = old.clone().filter(|s| s.expired_at.is_none());
        let directives = if self.world.serves(groups::TURN_DIRECTIVES) {
            match request.directives() {
                Ok(names) => names.unwrap_or_default(),
                Err(field) if self.world.defects.directives != DirectiveDefect::Unknown => {
                    return Ok(invalid(&field))
                }
                Err(_) => vec![],
            }
        } else {
            vec![]
        };
        let max_steps = match request.max_steps() {
            Ok(limit) => limit,
            Err(field) => return Ok(invalid(&field)),
        };
        let max_output_tokens = match request.max_output_tokens() {
            Ok(limit) => limit,
            Err(field) => return Ok(invalid(&field)),
        };
        let no_tools = match request.no_tools() {
            Ok(value) => value,
            Err(field) => return Ok(invalid(&field)),
        };
        let budget = match request.budget() {
            Ok(budget) => budget,
            Err(field) => return Ok(invalid(&field)),
        };
        if let Some(budget) = &budget {
            if let Err(field) = budget.check(
                self.world.serves(groups::INPUT_TOKEN_CAP),
                existing.as_ref().and_then(|s| s.input_cap),
            ) {
                return Ok(invalid(&field));
            }
        }
        let structured_output = match request.structured_output() {
            Ok(structured_output) => structured_output,
            Err(field) => return Ok(invalid(&field)),
        };
        let tools: Vec<String> = match &existing {
            Some(sess) => sess.tools.clone(),
            None => request
                .plan
                .as_ref()
                .and_then(|plan| serde_json::from_value(plan.get("tools")?.clone()).ok())
                .unwrap_or_default(),
        };
        let frozen = match &structured_output {
            None => Value::Null,
            Some(structured_output) => {
                if let Err(field) =
                    structured_output.check(self.world.serves(groups::STRUCTURED_OUTPUT), &tools)
                {
                    return Ok(invalid(&field));
                }
                let support = self
                    .world
                    .structured_output_support
                    .lock()
                    .unwrap()
                    .get(session)
                    .copied()
                    .flatten();
                let (enforcement, reason) = match support {
                    Some(true) => ("enforced", None),
                    Some(false) => ("best_effort", Some("route_unsupported")),
                    None => ("best_effort", Some("catalog_unknown")),
                };
                json!({"mode": structured_output.mode.as_str(), "schema": structured_output.schema,
                       "enforcement": enforcement, "reason": reason})
            }
        };
        let defect = self.world.retention_defect;
        if defect == RetentionDefect::WithoutGroupIgnored && !self.world.serves(groups::RETENTION) {
            request.retention = None;
        }
        let cap = self
            .world
            .serves(groups::RETENTION)
            .then(|| Retention::new(RETENTION_MAX, self.world.retention_delete_ms));
        let previous = existing.as_ref().map(|s| s.retention);
        let accept_bad = match defect {
            RetentionDefect::ZeroAccepted => request.retention == Some(0),
            RetentionDefect::AboveMaxAccepted => {
                request.retention.is_some_and(|n| n > RETENTION_MAX)
            }
            RetentionDefect::LengthenAccepted => previous
                .flatten()
                .zip(request.retention)
                .is_some_and(|(a, b)| b > a),
            RetentionDefect::LateOptInAccepted => previous == Some(None),
            _ => false,
        };
        if !accept_bad {
            if let Err(mut detail) = request.check_retention(cap.as_ref(), previous) {
                if defect == RetentionDefect::MaxDetailMissing {
                    detail.max_seconds = None;
                }
                return Ok(refuse(
                    errors::INVALID_PARAMS,
                    Some(serde_json::to_value(detail).unwrap()),
                ));
            }
        }
        if defect == RetentionDefect::EqualRefused
            && request.retention.is_some()
            && request.retention == previous.flatten()
        {
            return Ok(invalid("retention"));
        }
        if defect == RetentionDefect::ShortenIgnored && existing.is_some() {
            request.retention = None;
        }
        if defect == RetentionDefect::ShortenReplaces
            && request
                .retention
                .zip(previous.flatten())
                .is_some_and(|(a, b)| a < b)
        {
            existing = None;
        }
        if defect == RetentionDefect::Inherit
            && old.as_ref().is_some_and(|s| s.expired_at.is_some())
            && request.retention.is_none()
        {
            request.retention = old.as_ref().and_then(|s| s.retention);
        }
        if let Some(sess) = &existing {
            if sess.owner != stamp.principal {
                if !self.world.serves(groups::DELIVERY_ORDER) {
                    return Ok(refuse(errors::SCOPE_OWNER_MISMATCH, None));
                }
                // A plan-named sender's string: `queue` and `steer` stand
                // for their lists, `interrupt` needs the entry's grant.
                let defects = &self.world.defects;
                let interrupt = request.delivery() == Delivery::Interrupt;
                let named = sess
                    .senders
                    .iter()
                    .any(|sender| sender.module == stamp.principal);
                let admitted = if interrupt && named && defects.order_interrupt_grant_ignored {
                    Ok(())
                } else if interrupt && named && defects.order_interrupt_owner_only {
                    Err((errors::INVALID_PARAMS, Some(DELIVERY_FIELD)))
                } else if !interrupt && defects.order_named_sender_strings_refused {
                    Err((errors::SCOPE_OWNER_MISMATCH, None))
                } else {
                    order::admit_delivery(
                        &stamp.principal,
                        Some(&sess.owner),
                        &sess.senders,
                        request.delivery(),
                    )
                };
                if let Err((code, field)) = admitted {
                    return Ok(refuse(code, field.map(|field| json!({ "field": field }))));
                }
                if !interrupt {
                    let modes = order::legacy_equivalent(request.delivery())
                        .expect("queue and steer have lists");
                    return self.ordered_send(stamp, session, &request, DeliveryOrder::new(modes));
                }
                // A granted interrupt goes on exactly as the owner's would.
            }
            if let Some(send) = sess.sends.get(&request.send_id) {
                if send.prompt != request.prompt_text() && !self.world.defects.reuse_accepted {
                    return Ok(refuse(
                        errors::SEND_ID_REUSE,
                        Some(json!({ "field": "prompt" })),
                    ));
                }
                if send.delivery != delivery && !self.world.defects.reuse_accepted {
                    return Ok(refuse(
                        errors::SEND_ID_REUSE,
                        Some(json!({ "field": DELIVERY_FIELD })),
                    ));
                }
                if !request.same_client_message_id(send.client_message_id.as_deref())
                    && !self.world.defects.client_id_reuse_accepted
                {
                    return Ok(refuse(
                        errors::SEND_ID_REUSE,
                        Some(json!({ "field": CLIENT_MESSAGE_ID_FIELD })),
                    ));
                }
                let mut sessions = self.sessions.lock().unwrap();
                let sess = sessions.get_mut(session).unwrap();
                let send = sess.sends.get_mut(&request.send_id).unwrap();
                send.resend_count += 1;
                let send = send.clone();
                drop(sessions);
                return Ok(self.send_reply(session, &send));
            }
            if sess.plan_digest.is_some()
                && request.runner_params.contains_key("tools")
                && !self.world.defects.late_plan_tools_ignored
            {
                return Ok(invalid("tools"));
            }
            if let Some(run) = sess.runs.iter().find(|run| {
                run.state == "active"
                    && !(delivery == groups::INTERRUPT && self.world.serves(groups::INTERRUPT))
            }) {
                if delivery != groups::STEER || !self.world.serves(groups::STEER) {
                    return Ok(refuse(errors::TRANSIENT, None));
                }
                if let Err(field) = request.check_directives(&run.directives) {
                    return Ok(invalid(&field));
                }
                for (differs, field) in [
                    (
                        max_steps.is_some_and(|limit| Some(limit) != run.max_steps),
                        "stop_when.max_steps",
                    ),
                    (
                        max_output_tokens.is_some_and(|limit| Some(limit) != run.max_output_tokens),
                        "generation.max_output_tokens",
                    ),
                ] {
                    if differs {
                        return Ok(invalid(field));
                    }
                }
                // A steer accepted during a run waits for a step boundary.
                // Unlike a send to an idle session, accepting it must not
                // start a turn of its own or claim it has already rendered.
                self.commit(
                    session,
                    json!({ "kind": "steer", "send_id": request.send_id,
                            "attachment_parts": attachment_parts,
                            "stored_as": stored_as,
                            "prompt": request.prompt_text(), "delivery": delivery,
                            "run_id": run.run_id, "admitted": false, "retention":request.retention,
                            "sender": stamp.principal,
                            "input_cap": budget.as_ref().and_then(|b| b.max_input_tokens) }),
                )?;
                if self.world.defects.held_steer_ends_run
                    && !self.sessions.lock().unwrap()[session].render_tags
                {
                    self.world
                        .held_steer_break_answers
                        .fetch_add(1, Ordering::SeqCst);
                    self.commit(
                        session,
                        json!({ "kind": "terminal", "run_id": run.run_id, "state": "interrupted" }),
                    )?;
                }
                let send = self.sessions.lock().unwrap()[session].sends[&request.send_id].clone();
                return Ok(self.send_reply(session, &send));
            }
        }
        if let Err(mode) = request.check_delivery(&self.world.groups) {
            return Ok(refuse(
                errors::DELIVERY_UNSUPPORTED,
                Some(delivery_unsupported_detail(mode)),
            ));
        }
        let admitted = existing.is_none();
        if let Some(plan) = request.plan.as_ref().filter(|_| admitted) {
            for check in [
                cortexkit_role_llm_runner::tags::check_plan(plan, &self.world.groups),
                order::check_plan(plan, &self.world.groups),
            ] {
                if let Err(field) = check {
                    return Ok(invalid(field));
                }
            }
        }
        if admitted {
            let lineage = if old.is_some() && defect != RetentionDefect::ReuseLineage {
                format!("lin-{session}-{}", self.world.now())
            } else {
                format!("lin-{session}")
            };
            self.commit(
                session,
                json!({ "kind": "start", "owner": stamp.principal, "lineage": lineage,
                        "tools": tools,
                        "output_refs_enabled": request.plan.as_ref().and_then(|p|p.get("items")).and_then(Value::as_array).is_some_and(|items|items.iter().any(|item|item["output_refs"] == true)),
                        "senders": request.plan.as_ref().and_then(|plan| plan.get(order::PLAN_SENDERS)),
                        "render_tags": request.plan.as_ref().and_then(|plan| plan.get(cortexkit_role_llm_runner::tags::PLAN_RENDER_TAGS)),
                        "plan_digest": request.plan.as_ref().map(plan_digest) }),
            )?;
        }
        let run_id = format!(
            "run-{}",
            self.sessions.lock().unwrap()[session].runs.len() + 1
        );
        if self.world.defects.sealed_result_in_later_commit {
            // A recovery that kept a never-sent call and ended its run
            // `interrupted` without a result closes that call only now, in a
            // later commit than the one that ended the run.
            let open: Vec<String> = {
                let sessions = self.sessions.lock().unwrap();
                let sess = &sessions[session];
                sess.calls
                    .iter()
                    .filter(|(_, call)| {
                        !call.intent
                            && !call.result
                            && sess
                                .run(&call.run_id)
                                .is_some_and(|run| run.state == "interrupted")
                    })
                    .map(|(key, _)| key.clone())
                    .collect()
            };
            for key in open {
                self.commit(
                    session,
                    json!({ "kind": "result", "call_key": key, "content": {"executed": false} }),
                )?;
            }
        }
        // A pending change that is due applies here, at the start of the
        // run, never inside one already running.
        self.apply_due_change(session)?;
        // An interrupt into a running turn names the turn it interrupts.
        let interrupted = existing
            .as_ref()
            .filter(|_| delivery == groups::INTERRUPT)
            .and_then(|sess| sess.runs.iter().find(|run| run.state == "active"))
            .map(|run| run.run_id.clone());
        let defects = &self.world.defects;
        // The send that started the run, as `run_started.send_id` names it.
        let cause = match &interrupted {
            Some(old) if defects.run_cause_names_interrupted => {
                existing.as_ref().and_then(|sess| {
                    sess.sends
                        .iter()
                        .find(|(_, send)| send.run_id == *old && !send.steered_into_running)
                        .map(|(send_id, _)| send_id.clone())
                })
            }
            _ if delivery == groups::QUEUE && defects.run_cause_internal_id => {
                Some(format!("sub-{run_id}"))
            }
            _ if delivery == groups::QUEUE
                && !admitted
                && defects.run_cause_omitted_after_admission =>
            {
                None
            }
            _ => Some(request.send_id.clone()),
        };
        self.commit(
            session,
            json!({ "kind": "send", "send_id": request.send_id, "prompt": request.prompt_text(),
                    "attachment_parts": attachment_parts,
                    "stored_as": stored_as,
                    "attachment_accepts": request.runner_params.get("attachment_accepts"),
                    "delivery": delivery, "run_id": run_id, "admitted": admitted,
                    "cause": cause, "sender": stamp.principal,
                    "retention":request.retention, "title":request.runner_params.get("title"),
                    "input_cap": budget.as_ref().and_then(|b| b.max_input_tokens),
                    "directives":directives,
                    "max_steps":max_steps, "max_output_tokens":max_output_tokens,
                    "no_tools":no_tools == Some(true),
                    "structured_output": frozen,
                    "client_message_id": request.client_message_id(&self.world.groups).ok().flatten() }),
        )?;
        if let Some(old) = &interrupted {
            // The interrupting message is written; now the turn it
            // interrupts ends `cancelled`, its model stream aborted. The fake
            // aborts only a held model stream: no case interrupts a turn
            // while one of its tool calls runs.
            self.world.held_models.lock().unwrap().remove(session);
            self.world.in_flight.lock().unwrap().remove(session);
            self.commit(
                session,
                json!({ "kind": "terminal", "run_id": old, "state": "cancelled" }),
            )?;
        }
        self.drive(session, &run_id)?;
        self.deliver_idle(session)?;
        let send = self.sessions.lock().unwrap()[session].sends[&request.send_id].clone();
        Ok(self.send_reply(session, &send))
    }

    /// A `delivery_order` send into an admitted session: admitted by the
    /// plan's senders, rendered from a plan template when it names one,
    /// replacing a queued send with its `dedupe_key`, then queued for the
    /// next boundary where one of its modes is possible.
    fn ordered_send(
        &self,
        stamp: &RouteStamp,
        session: &str,
        request: &SendRequest,
        wanted: DeliveryOrder,
    ) -> Result<Reply, Killed> {
        let Some(sess) = self.sessions.lock().unwrap().get(session).cloned() else {
            // The fake takes lists only into sessions an owner admitted.
            return Ok(invalid(order::FIELD));
        };
        let caller = stamp.principal.as_str();
        if !self.world.defects.order_admits_any_sender {
            if let Err((code, field)) =
                order::admit(caller, Some(&sess.owner), &sess.senders, &wanted.modes)
            {
                return Ok(refuse(code, field.map(|field| json!({ "field": field }))));
            }
        }
        if request.delivery_order.is_some()
            && !self.world.defects.order_undeclared_modes_queued
            && !wanted
                .modes
                .iter()
                .any(|mode| self.world.declares_mode(*mode))
        {
            return Ok(refuse(
                errors::DELIVERY_UNSUPPORTED,
                Some(delivery_unsupported_detail(&wanted.modes)),
            ));
        }
        let id = format!("{caller} {}", request.send_id);
        let params = serde_json::to_value(request).expect("a send serializes");
        if let Some(queued) = sess.ordered.get(&id) {
            if queued.request != params {
                return Ok(refuse(
                    errors::SEND_ID_REUSE,
                    Some(json!({ "field": order::FIELD })),
                ));
            }
            if queued.state == "retracted" {
                return Ok(refuse(errors::SEND_RETRACTED, None));
            }
            return Ok(ordered_reply(
                &self.world.defects,
                self.world.serves(groups::RUN_OPS),
                &sess,
                queued,
            ));
        }
        let (text, system) = match &wanted.template {
            None => (request.prompt_text().to_owned(), false),
            Some(template) => {
                let templates = sess
                    .senders
                    .iter()
                    .find(|sender| sender.module == caller)
                    .map(|sender| sender.templates.clone())
                    .unwrap_or_default();
                let numbers = tags::current(&sess);
                let shown = |part: &order::PartRef| {
                    numbers.get(&(part.mid.clone(), part.part.clone())).copied()
                };
                match order::render(&templates, template, shown) {
                    Ok(text) => (text, true),
                    Err(order::TemplateProblem::PartGone(_))
                        if self.world.defects.order_template_part_gone_accepted =>
                    {
                        (unchecked_render(&templates, template), true)
                    }
                    Err(problem)
                        if self.world.defects.order_template_slots_unchecked
                            && !matches!(problem, order::TemplateProblem::PartGone(_)) =>
                    {
                        (unchecked_render(&templates, template), true)
                    }
                    Err(problem) => {
                        let (code, detail) = problem.refusal();
                        return Ok(refuse(code, Some(detail)));
                    }
                }
            }
        };
        if !self.world.defects.order_dedupe_keeps_both {
            for queued_id in &sess.queue {
                let queued = &sess.ordered[queued_id];
                if order::replaces(
                    caller,
                    wanted.dedupe_key.as_deref(),
                    &queued.sender,
                    queued.dedupe_key.as_deref(),
                    queued.state != "pending",
                ) {
                    self.commit(
                        session,
                        json!({ "kind": "order_end", "id": queued_id, "state": "superseded" }),
                    )?;
                }
            }
        }
        self.commit(
            session,
            json!({ "kind": "ordered", "id": id, "send_id": request.send_id, "sender": caller,
                    "text": text, "modes": wanted.modes, "dedupe_key": wanted.dedupe_key,
                    "expires_at_ms": wanted.expires_at_ms, "system": system,
                    "request": params }),
        )?;
        self.deliver_idle(session)?;
        let sess = self.sessions.lock().unwrap()[session].clone();
        Ok(ordered_reply(
            &self.world.defects,
            self.world.serves(groups::RUN_OPS),
            &sess,
            &sess.ordered[&id],
        ))
    }

    /// Start or end a context-ceiling hold on `session`. Ending it without a
    /// view ends the waiting run `context_ceiling`; with one, the run goes
    /// on.
    fn end_ceiling_wait(&self, session: &str, view_arrived: bool) -> Result<(), Killed> {
        self.world.ceiling_holds.lock().unwrap().remove(session);
        let waiting = self.sessions.lock().unwrap()[session].ceiling_wait.clone();
        let Some((run_id, _)) = waiting else {
            return Ok(());
        };
        if view_arrived {
            self.commit(session, json!({ "kind": "ceiling_view" }))?;
            self.drive(session, &run_id)?;
            self.deliver_idle(session)
        } else {
            self.commit(
                session,
                json!({ "kind": "terminal", "run_id": run_id, "state": "context_ceiling",
                        "context_ceiling": { "window": 200_000, "size": 199_000,
                                             "cause": errors::context_ceiling_causes::VIEW_TIMEOUT } }),
            )
        }
    }

    fn read(&self, session: &str, params: &Value) -> Reply {
        if let Some(field) = unknown_field(params, READ_FIELDS) {
            return invalid(&field);
        }
        let request: ReadRequest = match serde_json::from_value(params.clone()) {
            Ok(request) => request,
            Err(e) if e.to_string().contains("media") => return invalid("media"),
            Err(_) => return invalid("view"),
        };
        let mode = match request.mode() {
            Ok(mode) => mode,
            Err("media") if self.world.defects.media == Some("model_view_descriptors") => {
                ReadMode::Tail
            }
            Err(field) => return invalid(field),
        };
        if request.view == Some(ReadView::Model) && !self.world.serves(groups::MODEL_VIEW) {
            return invalid("view");
        }
        let sessions = self.sessions.lock().unwrap();
        let Some(sess) = sessions.get(session) else {
            if request.lineage_id.is_some() {
                return refuse(errors::LINEAGE_CHANGED, None);
            }
            return if request.view == Some(ReadView::Model) {
                respond(cortexkit_role_llm_runner::read::ModelPage::no_lineage())
            } else {
                respond(ReadPage::no_lineage())
            };
        };
        if request
            .lineage_id
            .as_ref()
            .is_some_and(|lineage| !history::lineage_served(sess, lineage, &self.world.defects))
        {
            return refuse(errors::LINEAGE_CHANGED, None);
        }
        if let Some(refusal) = history::read_bound(sess, &mode, &self.world.defects) {
            return refusal;
        }
        let limit = request.limit.unwrap_or(DEFAULT_LIMIT).min(MAXIMUM_LIMIT) as usize;
        if request.view == Some(ReadView::Model) {
            return self.model_read(sess, mode, &request);
        }
        let cap = request
            .max_bytes
            .unwrap_or(DEFAULT_MAX_BYTES)
            .min(MAXIMUM_MAX_BYTES) as usize;
        let size = |m: &Msg| m.body.to_string().len();
        let all: Vec<_> = sess
            .messages
            .iter()
            .filter(|message| history::in_window(sess, message, &self.world.defects))
            .filter(|message| {
                !self.world.defects.run_tool_calls_embedded_results
                    || message.calls.iter().all(|key| sess.calls[key].result)
            })
            .collect();
        let (start, end) = match mode {
            ReadMode::Tail if !self.world.defects.tail_returns_oldest => {
                let mut start = all.len();
                let mut bytes = 0;
                while start > 0 && all.len() - start < limit {
                    let next = size(all[start - 1]);
                    if start < all.len() && bytes + next > cap {
                        break;
                    }
                    bytes += next;
                    start -= 1;
                }
                (start, all.len())
            }
            ReadMode::Tail | ReadMode::Range { .. } | ReadMode::After { .. } => {
                let start = match mode {
                    ReadMode::Range { from_ordinal } => all
                        .iter()
                        .position(|m| m.ordinal >= from_ordinal)
                        .unwrap_or(all.len()),
                    ReadMode::After { mid, .. } => match all.iter().position(|m| m.mid == mid) {
                        Some(index) => index + 1,
                        None => return refuse(errors::UNKNOWN_MID, None),
                    },
                    _ => 0,
                };
                let mut end = start;
                let mut bytes = 0;
                while end < all.len() && end - start < limit {
                    let next = size(all[end]);
                    if end > start && bytes + next > cap {
                        break;
                    }
                    bytes += next;
                    end += 1;
                }
                (start, end)
            }
        };
        let messages = all[start..end]
            .iter()
            .map(|m| history::served_ordinal(sess, self.read_message(sess, m), &self.world.defects))
            .collect();
        let messages = history::served_numbers(self, sess, &all, messages);
        let mut page = ReadPage::new(&sess.lineage, messages).with_head(cursor(sess.events.len()));
        if end < all.len() {
            page = page.with_next_from_ordinal(all[end].ordinal);
        }
        media::page(page, request.media, cap, &self.world.defects)
    }

    fn read_message(&self, sess: &Sess, m: &Msg) -> ReadMessage {
        let run = sess.run(&m.run_id);
        let is_final = run.is_some_and(|run| {
            cortexkit_role_llm_runner::run::RunState::parse(&run.state).is_terminal()
                && sess
                    .messages
                    .iter()
                    .rev()
                    .find(|message| message.run_id == m.run_id)
                    .is_some_and(|last| last.ordinal == m.ordinal)
        });
        let tool_calls = m
            .calls
            .iter()
            .filter(|key| {
                !(self.world.defects.sealed_call_without_attribution
                    && run.is_some_and(|run| run.state == "interrupted")
                    && !sess.calls[*key].intent)
            })
            .map(|key| {
                let call = &sess.calls[key];
                let mut entry = ToolCallAttribution::new(&call.tool_call_id)
                    .with_call_key(key)
                    .with_indeterminate(
                        (call.intent && !call.result)
                            || (self.world.defects.sealed_call_indeterminate
                                && !call.intent
                                && run.is_some_and(|run| run.state == "interrupted")),
                    );
                if call.intent {
                    entry = entry.with_dispatched_to(TOOL_MODULE);
                }
                entry
            })
            .collect();
        let mut body = m.body.clone();
        if self.world.defects.client_id_in_read {
            if let Some(id) = &m.client_message_id {
                body["client_message_id"] = json!(id);
            }
        }
        if self.world.defects.sender_in_read {
            if let Some(sender) = &m.sender {
                body["sender"] = json!(sender);
            }
        }
        for part in body
            .get_mut("parts")
            .and_then(Value::as_array_mut)
            .into_iter()
            .flatten()
        {
            if part["type"] == "text" {
                let text = part["text"].as_str().unwrap_or_default().to_owned();
                part["text"] = json!(tags::exposed(sess, m, &text, &self.world.defects));
            }
        }
        if self
            .world
            .defects
            .run_tool_calls_open_message_without_arguments
        {
            for part in body["parts"].as_array_mut().into_iter().flatten() {
                if part["type"] == "tool_call"
                    && m.calls.iter().any(|key| {
                        let call = &sess.calls[key];
                        !call.result
                            && part["id"] == call.tool_call_id
                            && part["arguments"] == call.arguments
                    })
                {
                    part.as_object_mut().unwrap().remove("arguments");
                }
            }
        }
        if self.world.defects.read_serves_live_host_metadata && sess.host_meta > 0 {
            body["host_title"] = json!(format!("renamed {}", sess.host_meta));
        }
        let mut message = ReadMessage::new(m.ordinal, &m.mid, body)
            .with_tool_calls(tool_calls)
            .with_written_at(m.written_at);
        if let Some(role) = m.body["role"].as_str() {
            message = message.with_role(role);
        }
        if self.world.serves(groups::TAGS) {
            let numbers = tags::current(sess);
            message = message.with_tags(tags::rows(sess, m, &self.world.defects, &numbers));
        }
        if let Some(delivered) = &m.delivered_as {
            message = message.with_delivered_as(serde_json::from_value(delivered.clone()).unwrap());
        }
        if !m.run_id.is_empty() && !self.world.defects.attribution_missing_run {
            let is_final = if self.world.defects.attribution_wrong_final {
                sess.messages
                    .iter()
                    .find(|message| message.run_id == m.run_id)
                    .is_some_and(|first| first.ordinal == m.ordinal)
            } else if self.world.defects.attribution_running_final
                && run.is_some_and(|r| {
                    !cortexkit_role_llm_runner::run::RunState::parse(&r.state).is_terminal()
                })
            {
                sess.messages
                    .iter()
                    .rev()
                    .find(|message| message.run_id == m.run_id)
                    .is_some_and(|last| last.ordinal == m.ordinal)
            } else if self.world.defects.attribution_assistant_only_final {
                is_final && m.body["role"] == "assistant"
            } else {
                is_final
            };
            message = message.with_run(RunAttribution {
                run_id: m.run_id.clone(),
                episode: "episode-1".into(),
                is_final,
            });
        } else if m.run_id.is_empty() && self.world.defects.attribution_invent_run {
            message = message.with_run(RunAttribution {
                run_id: sess.runs.last().unwrap().run_id.clone(),
                episode: "episode-1".into(),
                is_final: false,
            });
        }
        message
    }

    fn head(&self, stamp: &RouteStamp, session: &str, params: &Value) -> Reply {
        if let Some(field) = unknown_field(params, &[]) {
            return invalid(&field);
        }
        let sessions = self.sessions.lock().unwrap();
        let Some(sess) = sessions.get(session) else {
            let mut head = HeadMeta::no_lineage();
            if stamp.principal == "reserved:owner" && !self.world.defects.head_sender_owner_absent {
                head = head.with_sender(cortexkit_role_llm_runner::read::HeadSender::for_owner(
                    self.head_supported_modes(),
                ));
            }
            if self.world.serves(groups::SESSION_USAGE) {
                head = head.with_model(cortexkit_role_llm_runner::read::HeadModel::new(
                    FAKE_PROVIDER,
                    FAKE_MODEL,
                ));
            }
            if self.world.defects.head_sender_omitted {
                head.sender = None;
            }
            return respond(head);
        };
        if sess.expired_at.is_some() && self.world.retention_defect == RetentionDefect::Empty {
            return respond(HeadMeta::no_lineage());
        }
        let mut head = HeadMeta::new(&sess.lineage, sess.first_ordinal, sess.last_activity)
            .with_head(cursor(sess.events.len()))
            .with_history_version(sess.history_version);
        if let Some(incarnation) = &sess.incarnation {
            head = head.with_incarnation(incarnation);
        }
        let caller = if self.world.defects.head_sender_ignores_caller {
            sess.senders
                .first()
                .map(|s| s.module.as_str())
                .unwrap_or(&stamp.principal)
        } else {
            &stamp.principal
        };
        let supported = self.head_supported_modes();
        if caller == sess.owner {
            if !self.world.defects.head_sender_owner_absent {
                head = head.with_sender(cortexkit_role_llm_runner::read::HeadSender::for_owner(
                    supported,
                ));
            }
        } else if let Some(sender) = sess.senders.iter().find(|s| s.module == caller) {
            head = head.with_sender(cortexkit_role_llm_runner::read::HeadSender::new(
                supported
                    .into_iter()
                    .filter(|m| {
                        m != "idle_start"
                            || sender.idle_start
                            || self.world.defects.head_sender_a_idle_start
                    })
                    .collect(),
                sender.interrupt,
            ));
        } else if self.world.defects.head_sender_grant_for_unnamed_caller {
            head = head.with_sender(cortexkit_role_llm_runner::read::HeadSender::for_owner(
                supported,
            ));
        }
        if self.world.serves(groups::SESSION_USAGE) {
            let original =
                cortexkit_role_llm_runner::read::HeadModel::new(FAKE_PROVIDER, FAKE_MODEL);
            head = head.with_model(if self.world.defects.head_model_from_plan {
                original
            } else {
                sess.head_model.clone().unwrap_or(original)
            });
            if let Some(mut usage) = sess.session_usage.clone() {
                if self.world.defects.session_usage_sum_wrong {
                    usage.totals = sess.runs.last().unwrap().usage.clone();
                }
                if self.world.defects.session_usage_since_wrong {
                    usage.since = "not-the-first-run".into();
                }
                if self.world.defects.session_usage_unknown_zero {
                    usage.totals.reasoning_tokens = Some(0);
                    for model in &mut usage.totals.by_model {
                        model.reasoning_tokens = Some(0);
                    }
                }
                head = head.with_usage(usage);
            }
        }
        if self.world.serves(groups::ATTACHMENTS) {
            let accepts = sess.attachment_accepts.unwrap_or(true);
            head = head.with_input(
                cortexkit_role_llm_runner::read::HeadInput::new(if accepts { "yes" } else { "no" })
                    .with_modalities(if accepts {
                        vec!["text".into(), "image".into(), "document".into()]
                    } else {
                        vec!["text".into()]
                    })
                    .with_mime_types(if accepts {
                        vec!["image/png".into(), "application/pdf".into()]
                    } else {
                        vec![]
                    }),
            );
        }
        if let Some(cut) = history::head_cut(sess, &self.world.defects) {
            head = head.with_cut(cut);
        }
        if let Some(last) = sess.messages.last() {
            head = head.with_last_ordinal(last.ordinal);
        }
        if let Some(run) = sess.runs.last() {
            head = head.with_last_run_state(LastRunState {
                run_id: run.run_id.clone(),
                state: if self.world.held_paused
                    && sess.retention.is_some()
                    && run.state == "active"
                {
                    "paused".into()
                } else {
                    run.state.clone()
                },
                reason: sess
                    .ceiling_wait
                    .as_ref()
                    .filter(|(waiting, _)| *waiting == run.run_id)
                    .map(|_| cortexkit_role_llm_runner::RUN_REASON_CONTEXT_CEILING_WAIT.to_owned())
                    .or_else(|| {
                        sess.compaction_wait
                            .as_ref()
                            .filter(|(waiting, _)| *waiting == run.run_id)
                            .map(|_| {
                                cortexkit_role_llm_runner::RUN_REASON_COMPACTION_WAIT.to_owned()
                            })
                    })
                    .or_else(|| run.reason.clone()),
            });
        }
        if self.world.defects.head_sender_omitted {
            head.sender = None;
        }
        respond(head)
    }

    fn head_supported_modes(&self) -> Vec<String> {
        let Reply::Response(value) = self.describe() else {
            unreachable!()
        };
        let describe: RoleDescribe = serde_json::from_value(value).unwrap();
        [
            Mode::ToolResult,
            Mode::TurnSystem,
            Mode::Steer,
            Mode::IdleStart,
        ]
        .into_iter()
        .filter(|mode| mode.is_declared_by(&describe))
        .map(|mode| mode.as_str().to_owned())
        .collect()
    }

    fn run_result(&self, session: &str, params: Value) -> Reply {
        let request: RunResultRequest = match serde_json::from_value(params.clone()) {
            Ok(request) => request,
            Err(_) => {
                return invalid(&unknown_field(&params, &["run_id"]).unwrap_or("run_id".into()))
            }
        };
        let sessions = self.sessions.lock().unwrap();
        let Some(run) = sessions.get(session).and_then(|s| s.run(&request.run_id)) else {
            return refuse(errors::UNKNOWN_RUN, None);
        };
        let sealed_late = run.state != "active"
            && self
                .world
                .late_seal_reads
                .try_update(Ordering::SeqCst, Ordering::SeqCst, |n| n.checked_sub(1))
                .is_ok();
        if sealed_late {
            return respond(RunResult::new(&run.run_id, "active"));
        }
        let mut result = RunResult::new(
            &run.run_id,
            if self.world.held_paused
                && sessions[session].retention.is_some()
                && run.state == "active"
            {
                "paused"
            } else {
                &run.state
            },
        );
        if run.state == "completed" {
            let ordinal = run.final_ordinal.unwrap_or_default();
            let sess = &sessions[session];
            let message = &sess.messages[ordinal as usize];
            result = result.with_final_message(FinalMessage {
                ordinal,
                mid: message.mid.clone(),
                text: tags::exposed(
                    sess,
                    message,
                    &run.final_text.clone().unwrap_or_default(),
                    &self.world.defects,
                ),
            });
        }
        if let Some(reason) = &run.reason {
            result = result.with_reason(reason);
        }
        let directive_defect = self.world.defects.directives;
        let requested =
            !run.directives.is_empty() || directive_defect == DirectiveDefect::Unrequested;
        let evaluate = run.state == "completed"
            || (directive_defect == DirectiveDefect::MaxSteps && run.state == "max_steps")
            || (directive_defect == DirectiveDefect::Cancelled && run.state == "cancelled");
        if self.world.serves(groups::TURN_DIRECTIVES) && requested && evaluate {
            let text = sessions[session]
                .messages
                .iter()
                .rev()
                .find(|message| message.run_id == run.run_id && message.body["parts"].is_array())
                .map(|message| {
                    message.body["parts"]
                        .as_array()
                        .unwrap()
                        .iter()
                        .filter(|part| part["type"] == "text")
                        .filter_map(|part| part["text"].as_str())
                        .collect::<String>()
                })
                .unwrap_or_default();
            if let Some(parsed) = cortexkit_role_llm_runner::directives::parse_trailing(&text) {
                let suppress = match directive_defect {
                    DirectiveDefect::In => {
                        text.ends_with("<wake in=\"90s\"/>")
                            && run.state == "completed"
                            && !run.directives.is_empty()
                    }
                    DirectiveDefect::Note => text.ends_with("</wake>"),
                    DirectiveDefect::Off => text.ends_with("<wake off/>"),
                    _ => false,
                };
                if !suppress {
                    result = result.with_directives(vec![parsed.into()]);
                }
            }
        }
        if let Some(error) = &run.error {
            result = result.with_error(serde_json::from_value(error.clone()).unwrap());
        }
        if let Some(detail) = &run.detail {
            result = result.with_detail(serde_json::from_value(detail.clone()).unwrap());
        }
        if let Some(ceiling) = &run.ceiling {
            result = result.with_context_ceiling(serde_json::from_value(ceiling.clone()).unwrap());
        }
        if let Some(frozen) = &run.structured_output {
            if RunState::parse(&result.state).is_terminal() {
                result = result.with_structured_output(match &run.structured_output_result {
                    Some(output) => serde_json::from_value(output.clone()).unwrap(),
                    None => judge(frozen, None, Vec::new()),
                });
            }
        }
        // Usage on every ended run, and on a running one once a step has
        // completed; `run.status` answers the same.
        if self.world.serves(groups::RUN_USAGE)
            && (RunState::parse(&result.state).is_terminal() || run.usage.steps > 0)
        {
            let mut usage = run.usage.clone();
            if self.world.defects.usage_absent_reasoning_as_zero {
                usage.reasoning_tokens.get_or_insert(0);
                for entry in &mut usage.by_model {
                    entry.reasoning_tokens.get_or_insert(0);
                }
            }
            result = result.with_usage(usage);
        }
        respond(result)
    }

    /// `run.status`: the `run.result` answer, plus the waiting reason and
    /// deadline of a run held at the context ceiling or on a rewrite ride.
    fn run_status(&self, session: &str, params: Value) -> Reply {
        let run_id = params["run_id"].as_str().unwrap_or_default().to_owned();
        let mut reply = self.run_result(session, params);
        if let Reply::Response(body) = &mut reply {
            let held = self.held_status(session, &run_id);
            if !held.is_empty() {
                body["held_calls"] = json!(held);
            }
            let (ceiling, ride) = self
                .sessions
                .lock()
                .unwrap()
                .get(session)
                .map(|sess| (sess.ceiling_wait.clone(), sess.compaction_wait.clone()))
                .unwrap_or_default();
            for (wait, reason) in [
                (
                    ceiling,
                    cortexkit_role_llm_runner::RUN_REASON_CONTEXT_CEILING_WAIT,
                ),
                (ride, cortexkit_role_llm_runner::RUN_REASON_COMPACTION_WAIT),
            ] {
                if let Some((_, deadline)) = wait.filter(|(run, _)| *run == run_id) {
                    body["reason"] = json!(reason);
                    body["deadline_ms"] = json!(deadline);
                }
            }
        }
        reply
    }

    /// The content of part `part` of `message`, as stored.
    fn part_content(sess: &Sess, message: &Msg, part: &str, defects: &Defects) -> String {
        match message.body["parts"].as_array() {
            Some(parts) => {
                let part = &parts[part.parse::<usize>().unwrap_or_default()];
                match part["text"].as_str() {
                    Some(text) if part["type"] == "text" => {
                        tags::exposed(sess, message, text, defects)
                    }
                    Some(text) if part["type"] != "tool_call" => text.to_owned(),
                    _ => part.to_string(),
                }
            }
            None => match message.body["text"].as_str() {
                Some(text) => text.to_owned(),
                None => message.body["content"].to_string(),
            },
        }
    }

    /// `send.status`: the answer a re-send of the caller's `send_id` would
    /// get, read from the session's records, writing nothing.
    fn send_status(&self, stamp: &RouteStamp, session: &str, params: &Value) -> Reply {
        if let Some(field) = unknown_field(params, &["send_id"]) {
            return invalid(&field);
        }
        let request: cortexkit_role_llm_runner::send_status::SendStatusRequest =
            match serde_json::from_value(params.clone()) {
                Ok(request) => request,
                Err(_) => return invalid("send_id"),
            };
        let Some(sess) = self.sessions.lock().unwrap().get(session).cloned() else {
            return refuse(errors::UNKNOWN_SEND, None);
        };
        let id = format!("{} {}", stamp.principal, request.send_id);
        if let Some(queued) = sess.ordered.get(&id) {
            if self.world.defects.send_status_pending_after_restart
                && self.replayed.load(Ordering::SeqCst)
                && queued.state == "delivered"
            {
                return respond(SendReply::new("pending").with_submission_id(&queued.send_id));
            }
            return ordered_reply(
                &self.world.defects,
                self.world.serves(groups::RUN_OPS),
                &sess,
                queued,
            );
        }
        match sess.sends.get(&request.send_id) {
            Some(send) => self.send_reply(session, send),
            None => refuse(errors::UNKNOWN_SEND, None),
        }
    }

    /// Withdraw a still-pending `delivery_order` submission durably.
    fn retract(&self, stamp: &RouteStamp, session: &str, params: Value) -> Result<Reply, Killed> {
        let request: RetractRequest = match serde_json::from_value(params) {
            Ok(request) => request,
            Err(_) => return Ok(invalid("submission_id")),
        };
        let Some(sess) = self.sessions.lock().unwrap().get(session).cloned() else {
            return Ok(respond(RetractReply::NotPending));
        };
        if sess.owner != stamp.principal {
            return Ok(refuse(errors::SCOPE_OWNER_MISMATCH, None));
        }
        let id = sess
            .ordered
            .iter()
            .find(|(_, queued)| {
                queued.send_id == request.submission_id && queued.state == "pending"
            })
            .map(|(id, _)| id.clone());
        let Some(id) = id else {
            return Ok(respond(RetractReply::NotPending));
        };
        self.commit(
            session,
            json!({ "kind": "order_end", "id": id, "state": "retracted" }),
        )?;
        Ok(respond(RetractReply::Retracted))
    }

    fn resolve(&self, session: &str, params: &Value) -> Reply {
        if let Some(field) = unknown_field(params, &["tag", "range", "media"]) {
            return invalid(&field);
        }
        let request: cortexkit_role_llm_runner::tags::ResolveRequest =
            match serde_json::from_value(params.clone()) {
                Ok(request) => request,
                Err(error) if error.to_string().contains("tag") => return invalid("tag"),
                Err(error) if error.to_string().contains("media") => return invalid("media"),
                Err(_) => return invalid("range"),
            };
        let number = match request.number() {
            Ok(number) => number,
            Err(field) => return invalid(field),
        };
        let sessions = self.sessions.lock().unwrap();
        let found = sessions.get(session).and_then(|sess| {
            // Only the current lineage's parts resolve; with
            // `Defects::resolve_left_lineage_tags` set, resolve also searches
            // parts a recovery dropped with the lineage it left.
            let left = self
                .world
                .defects
                .resolve_left_lineage_tags
                .then(|| {
                    sess.left.iter().find_map(|m| {
                        m.left_numbers
                            .iter()
                            .find(|(_, n)| *n == number)
                            .map(|(part, _)| (m, part.clone()))
                    })
                })
                .flatten();
            left.or_else(|| tags::find(sess, number))
                .map(|found| (sess, found))
        });
        let Some((sess, (message, part))) = found else {
            return refuse(errors::UNKNOWN_TAG, None);
        };
        // A tag naming a part below the lineage's first ordinal names
        // history before the latest cut, which resolve never answers.
        if message.ordinal < sess.first_ordinal
            && sess.messages.iter().any(|m| m.mid == message.mid)
        {
            return refuse(
                errors::BEFORE_LINEAGE_START,
                Some(json!({ "first_ordinal": sess.first_ordinal })),
            );
        }
        let content = Self::part_content(sess, message, &part, &self.world.defects);
        let sliced = match cortexkit_role_llm_runner::tags::slice(&content, request.range.as_ref())
        {
            Ok(sliced) => sliced,
            Err(field) => return invalid(field),
        };
        let mut reply = cortexkit_role_llm_runner::tags::ResolveReply::new(
            &request.tag,
            message.ordinal,
            &message.mid,
            part,
            sliced,
        );
        if let Some(range) = request.range {
            reply = reply.with_range(range);
        }
        if request.media == Some(cortexkit_role_llm_runner::read::MediaMode::Descriptors) {
            let is_text = message.body.get("text").is_some()
                || message.body["parts"]
                    .as_array()
                    .and_then(|p| reply.part.parse::<usize>().ok().and_then(|i| p.get(i)))
                    .is_some_and(|p| p["type"] == "text");
            if !is_text {
                if let Ok(value) = serde_json::from_str::<Value>(&content) {
                    reply.content = match cortexkit_role_llm_runner::read::descriptors::describe(
                        &value,
                        &media::payloads(&value),
                        cortexkit_role_llm_runner::read::descriptors::DESCRIPTOR_MESSAGE_MAX - 2048,
                    ) {
                        Ok(value) => value,
                        Err(_) => {
                            return refuse(
                                errors::REPLY_TOO_LARGE,
                                Some(json!({"ordinal":message.ordinal})),
                            )
                        }
                    };
                }
            } else if let Some(text) = reply
                .content
                .as_str()
                .filter(|s| s.len() > cortexkit_role_llm_runner::read::descriptors::PART_INLINE_MAX)
            {
                reply.content = serde_json::to_value(
                    cortexkit_role_llm_runner::read::descriptors::Reference::text(
                        Some(reply.part.clone()),
                        text,
                    ),
                )
                .unwrap();
            }
        } else if serde_json::to_vec(&reply).unwrap().len() + 21
            > cortexkit_role_llm_runner::TRANSPORT_FRAME_LIMIT
        {
            return refuse(
                errors::REPLY_TOO_LARGE,
                Some(json!({"ordinal":message.ordinal})),
            );
        }
        respond(reply)
    }

    fn tool_calls(&self, session: &str, params: &Value) -> Reply {
        use cortexkit_role_llm_runner::tool_calls::{
            outcomes, RunToolCall, ToolCallsPage, ToolCallsRequest,
        };
        if let Some(field) = unknown_field(params, &["run_id", "after", "limit"]) {
            return invalid(&field);
        }
        let request: ToolCallsRequest = match serde_json::from_value(params.clone()) {
            Ok(request) => request,
            Err(_) => return invalid("run_id"),
        };
        if let Err(field) = request.check() {
            return invalid(field);
        }
        let sessions = self.sessions.lock().unwrap();
        let Some(sess) = sessions
            .get(session)
            .filter(|s| s.run(&request.run_id).is_some())
        else {
            return refuse(errors::UNKNOWN_RUN, None);
        };
        let mut calls = Vec::new();
        for message in sess.messages.iter().filter(|m| m.run_id == request.run_id) {
            for (position, key) in message.calls.iter().enumerate() {
                let call = &sess.calls[key];
                let outcome = match (call.intent, call.result) {
                    (true, true) => outcomes::SUCCEEDED,
                    (true, false) => outcomes::INDETERMINATE,
                    (false, true) => outcomes::NOT_EXECUTED,
                    (false, false) => continue,
                };
                let name = message.body["parts"]
                    .as_array()
                    .into_iter()
                    .flatten()
                    .find(|part| part["type"] == "tool_call" && part["arguments"] == call.arguments)
                    .and_then(|part| part["tool"].as_str())
                    .unwrap_or(SCRIPTED_TOOL);
                let mut listed = RunToolCall::new(
                    &call.tool_call_id,
                    name,
                    call.arguments.clone(),
                    outcome,
                    message.ordinal,
                )
                .with_call_key(key)
                .with_position(
                    if self.world.defects.run_tool_calls_positions_swapped
                        && message.calls.len() == 2
                    {
                        1 - position as u32
                    } else {
                        position as u32
                    },
                );
                if !call.result && self.world.defects.run_tool_calls_open_result_ordinal {
                    listed = listed.with_result_ordinal(message.ordinal);
                }
                if call.intent {
                    listed = listed.with_dispatched_to(TOOL_MODULE);
                }
                if call.result {
                    let stored = if self.world.defects.run_tool_calls_embedded_results {
                        message.body["parts"]
                            .as_array()
                            .into_iter()
                            .flatten()
                            .find(|part| {
                                part["type"] == "tool_call" && part["id"] == call.tool_call_id
                            })
                            .and_then(|part| part.get("result"))
                            .map(|result| (message.ordinal, result.to_string().len()))
                    } else {
                        sess.messages
                            .iter()
                            .find(|m| {
                                m.body["role"] == "tool"
                                    && (m.body["call_key"] == json!(key)
                                        || (m.body.get("call_key").is_none()
                                            && m.run_id == call.run_id
                                            && m.body["tool_call_id"] == json!(call.tool_call_id)))
                            })
                            .map(|stored| (stored.ordinal, stored.body.to_string().len()))
                    };
                    if let Some((ordinal, bytes)) = stored {
                        listed = listed
                            .with_result_bytes(bytes as u64)
                            .with_result_ordinal(ordinal);
                    }
                }
                calls.push(listed);
            }
        }
        if self.world.defects.run_tool_calls_reversed {
            calls.reverse();
        }
        let from = match request.after.as_deref().map(str::parse::<usize>) {
            None => 0,
            Some(Ok(after)) if after <= calls.len() => after,
            Some(_) => return invalid(cortexkit_role_llm_runner::tool_calls::AFTER_FIELD),
        };
        let limit = request.limit.unwrap_or(DEFAULT_LIMIT).min(MAXIMUM_LIMIT) as usize;
        let end = (from + limit).min(calls.len());
        let mut page = ToolCallsPage::new(&request.run_id, calls[from..end].to_vec());
        if end < calls.len() {
            page = page.with_next(end.to_string());
        }
        respond(page)
    }

    fn subscribe(&self, session: &str, params: Value) -> SubscribeOutcome {
        if self.world.serves(groups::LIVE_EVENTS) {
            return self.live_subscribe(session, params);
        }
        let request: SubscribeRequest = match serde_json::from_value(params) {
            Ok(request) => request,
            Err(_) => {
                let Reply::Error(error) = invalid("from") else {
                    unreachable!()
                };
                return SubscribeOutcome::Refused(error);
            }
        };
        let sessions = self.sessions.lock().unwrap();
        let events = sessions
            .get(session)
            .map(|s| s.events.clone())
            .unwrap_or_default();
        let after = match request.attach_point() {
            SubscribeFrom::Start => 0,
            SubscribeFrom::Live => events.len(),
            SubscribeFrom::After(position) => {
                position.get("seq").and_then(Value::as_u64).unwrap_or(0) as usize
            }
        };
        SubscribeOutcome::Events(
            events
                .iter()
                .enumerate()
                .skip(after)
                .flat_map(|(index, kind)| {
                    [
                        json!({ "kind": "display", "delta": kind }),
                        json!({ "kind": "control", "cursor": cursor(index + 1), "record": kind }),
                    ]
                })
                .collect(),
        )
    }
}

/// What the scripted model receives for a request of a run with the frozen
/// structured-output request `frozen`, rendered only from frozen data.
fn observe_request(tools: &[String], frozen: Option<&Value>) -> ModelRequestObservation {
    let mut offered = tools.to_vec();
    let Some(frozen) = frozen else {
        return ModelRequestObservation::new(offered);
    };
    if frozen["mode"] == "submit_tool" {
        offered.push(SUBMIT_RESULT_TOOL.into());
        ModelRequestObservation::new(offered).with_submit_result_schema(frozen["schema"].clone())
    } else {
        ModelRequestObservation::new(offered).with_final_text_schema(frozen["schema"].clone())
    }
}

/// The structured output of a run with the frozen request `frozen`.
/// `produced` is `None` when no value was produced, `Some(Err)` when the
/// final text was not JSON.
fn judge(
    frozen: &Value,
    produced: Option<Result<Value, ()>>,
    skipped: Vec<SkippedSibling>,
) -> StructuredOutputResult {
    let mode = frozen["mode"].as_str().unwrap_or_default();
    let enforcement = frozen["enforcement"].as_str().unwrap_or_default();
    let (validation, value, errors) = match produced {
        None => ("absent", None, Vec::new()),
        Some(Err(())) => ("failed", None, vec![": not valid JSON".to_owned()]),
        Some(Ok(value)) => {
            let mut errors = Vec::new();
            validate(&frozen["schema"], &value, "", &mut errors);
            let validation = if errors.is_empty() {
                "passed"
            } else {
                "failed"
            };
            (validation, Some(value), errors)
        }
    };
    let mut output = StructuredOutputResult::new(mode, enforcement, validation);
    if let Some(reason) = frozen["reason"].as_str() {
        output = output.with_reason(reason);
    }
    if let Some(value) = value {
        output = output.with_value(value);
    }
    if !errors.is_empty() {
        output = output.with_errors(errors);
    }
    if mode == "submit_tool" {
        output = output.with_skipped_siblings(skipped);
    }
    output
}

/// The few JSON Schema keywords the suite's schemas use: `type`,
/// `properties` and `required`. A real runner uses a full draft 2020-12
/// validator.
fn validate(schema: &Value, value: &Value, path: &str, errors: &mut Vec<String>) {
    if let Some(wanted) = schema["type"].as_str() {
        let matches = match wanted {
            "object" => value.is_object(),
            "string" => value.is_string(),
            "integer" => value.is_i64() || value.is_u64(),
            "number" => value.is_number(),
            "boolean" => value.is_boolean(),
            "array" => value.is_array(),
            "null" => value.is_null(),
            _ => true,
        };
        if !matches {
            errors.push(format!("{path}: expected {wanted}"));
            return;
        }
    }
    let Some(members) = value.as_object() else {
        return;
    };
    for (name, property) in schema["properties"].as_object().into_iter().flatten() {
        if let Some(member) = members.get(name) {
            validate(property, member, &format!("{path}/{name}"), errors);
        }
    }
    for name in schema["required"].as_array().into_iter().flatten() {
        if let Some(name) = name.as_str().filter(|name| !members.contains_key(*name)) {
            errors.push(format!("{path}: missing required member {name}"));
        }
    }
}

/// The opaque position after the `seq`-th durable event.
/// The answer to a `delivery_order` send, or its re-send: `pending` while
/// queued; once delivered, the state of the run that carried it, that run,
/// and how it went in; otherwise its terminal state. With
/// `Defects::delivered_steer_without_run_id` set while the runner declares
/// `run_ops`, a steer's answer leaves the run out. Without `run_ops`, a `step`
/// receipt also omits `run_id` by design.
fn ordered_reply(defects: &Defects, run_ops: bool, sess: &Sess, queued: &Queued) -> Reply {
    match (queued.state.as_str(), &queued.delivery) {
        ("pending", _) => respond(SendReply::new("pending").with_submission_id(&queued.send_id)),
        ("delivered", Some(delivery)) => {
            let reply = match sess.run(&delivery.run_id) {
                Some(run) if run.state == "active" => SendReply::new("active"),
                Some(run) => SendReply::new("finished").with_reason(&run.state),
                None => SendReply::new("finished"),
            };
            let omit_run_id = delivery.r#as == delivered_as::STEP
                && (!run_ops || defects.delivered_steer_without_run_id);
            let reply = if omit_run_id {
                reply
            } else {
                reply.with_run_id(&delivery.run_id)
            };
            respond(reply.with_delivered(Delivered::new(delivery.r#as).with_ref(&delivery.r#ref)))
        }
        ("delivered", None) => respond(SendReply::new("finished")),
        (other, _) => respond(SendReply::new(other)),
    }
}

/// The defect's rendering: slot values substituted with no check at all.
fn unchecked_render(templates: &[order::Template], template: &order::TemplateUse) -> String {
    let Some(found) = templates.iter().find(|t| t.id == template.id) else {
        return String::new();
    };
    let mut text = found.text.clone();
    for (name, value) in &template.slots {
        let value = value
            .as_str()
            .map_or_else(|| value.to_string(), str::to_owned);
        text = text.replace(&format!("{{{name}}}"), &value);
    }
    text
}

fn cursor(seq: usize) -> Map<String, Value> {
    let mut position = Map::new();
    position.insert("seq".into(), json!(seq));
    position
}

/// The first member of `params` not in `allowed`.
fn unknown_field(params: &Value, allowed: &[&str]) -> Option<String> {
    params
        .as_object()?
        .keys()
        .find(|key| !allowed.contains(&key.as_str()))
        .cloned()
}

pub struct FakeHandle(Arc<Module>);

impl Drop for FakeHandle {
    fn drop(&mut self) {
        if self.0.alive.load(Ordering::SeqCst) && self.0.world.serves(groups::INTERRUPT_REASON) {
            let active: Vec<_> = self
                .0
                .sessions
                .lock()
                .unwrap()
                .iter()
                .flat_map(|(name, sess)| {
                    sess.runs
                        .iter()
                        .filter(|run| run.state == "active")
                        .map(move |run| (name.clone(), run.run_id.clone()))
                })
                .collect();
            for (session, run) in active {
                let reason = (!self.0.world.defects.interrupt_shutdown_reason_missing)
                    .then_some(cortexkit_role_llm_runner::run::interrupt_reasons::RUNNER_SHUTDOWN);
                let _ = self.0.commit(&session, json!({"kind":"terminal", "run_id":run, "state":"interrupted", "reason":reason}));
            }
        }
        self.0.alive.store(false, Ordering::SeqCst);
    }
}

pub struct FakeRoute {
    module: Arc<Module>,
    session: String,
    stamp: RouteStamp,
}

#[async_trait]
impl RunnerRoute for FakeRoute {
    async fn request(&self, method: &str, params: Value) -> Result<Reply, RouteFailure> {
        let world = &self.module.world;
        let case = *world.current_case.lock().unwrap();
        world
            .route_calls
            .lock()
            .unwrap()
            .push((case, method.to_owned()));
        self.module
            .handle(&self.stamp, &self.session, method, params)
    }

    async fn subscribe(&self, params: Value) -> Result<SubscribeOutcome, RouteFailure> {
        if !self.module.alive.load(Ordering::SeqCst) {
            return Err(RouteFailure::new("the module is not running"));
        }
        Ok(self.module.subscribe(&self.session, params))
    }
}

pub struct FakeSubject {
    pub incarnation_fixtures: bool,
    pub head_sender_fixtures: bool,
    pub head_sender_group: bool,
    pub output_refs_group: bool,
    pub held_calls_group: bool,
    pub descriptor_case: Option<&'static str>,
    pub descriptor_fixtures: bool,
    /// Store attachment media in the runner's own schema as a `data:` URL
    /// `file` part, with no digest anywhere in the stored message.
    pub data_url_media: bool,
    /// The tool provider cannot answer an image-only result, so a scripted
    /// call that would carry one carries its default text result instead.
    pub no_image_results: bool,
    pub standalone_in_process: bool,
    /// The first `late_seal_reads` `run.result` reads of an ended run answer
    /// `active`, as if the runner reported the end late.
    pub late_seal_reads: usize,
    pub retention_defect: RetentionDefect,
    pub retention_delete_ms: u64,
    pub held_paused: bool,
    pub advertise_status: bool,
    pub cancel_step_boundary: bool,
    pub defects: Defects,
    pub step_recovery: StepRecovery,
    pub compaction_route_mode: compaction::CompactionRouteMode,
    pub capabilities: BTreeSet<Capability>,
    pub delivery_order_modes: Option<Vec<String>>,
    pub seal_compaction_setup_crash: bool,
    pub kill_points: Vec<&'static str>,
    /// Report every kill as a real process kill, which this in-process fake
    /// cannot make. Only for tests of the verdict.
    pub claim_process_kill: bool,
    /// Answer a queued send `delivered: pending` and its retries
    /// `delivered: unknown`: a receipt moving forward, which is allowed.
    pub queue_receipt_pending_then_unknown: bool,
    /// Declare `steer_receipt: confirm` (a steer's receipt may be `pending` or
    /// `unknown` until delivery is confirmed) instead of guaranteed delivery,
    /// where every steer receipt is `step` or `turn`.
    pub steer_receipt_confirm: bool,
    /// Exercise the contract's optional receipt on a steer's first answer.
    pub omit_first_steer_receipt: bool,
    /// Supply a final receipt even during the hold, preserving it after release.
    pub early_held_steer_receipt: Option<Delivered>,
    /// How long `pause` waits between the suite's polls. `None` only yields,
    /// so the suite's poll bound passes in no time; a duration makes that
    /// bound real time, as on a runner that polls a live process.
    pub pause_for: Option<Duration>,
    /// Whether the scripted route supports a turn-scoped system message.
    pub turn_system: bool,
    /// This adapter cannot create fetch-plan sessions.
    pub without_plans: bool,
    /// The adapter can give a session host history before its first send
    /// (`seed_host_history`), as a runner over a host that adopted it.
    pub host_prefix_hook: bool,
    /// The adapter can seed finished messages behind a cut
    /// (`seed_session_history`), for the cost case.
    pub seed_hook: bool,
    /// Every session holds this many host entries before the runner's first
    /// numbered message, as on a runner over a host that adopted each
    /// session with history.
    pub host_prefix_everywhere: u64,
    /// Refuse reads `transient` while a tag-rendering run waits for a Setup
    /// without descent, as the contract lets a runner.
    pub holds_reads_during_setup: bool,
    /// How the fake judges the prompt cache's lifetime.
    pub cache_ttl_defect: CacheTtlDefect,
    /// The case the suite is in, as a test's progress callback sets it: the
    /// case's name from its `CaseStarted` until its `CaseFinished`, `None`
    /// between cases. The fake only reads it, to label `route_calls`.
    pub current_case: Arc<Mutex<Option<&'static str>>>,
    /// Every route request the fake received, in order: the `current_case`
    /// when it arrived, and the method.
    pub route_calls: RouteCalls,
    world: Mutex<Option<Arc<World>>>,
    /// Every module started, so a released call reaches the one holding it.
    modules: Mutex<Vec<Arc<Module>>>,
}

/// The capabilities the conforming fake declares: every group it serves,
/// and held tool calls. It leaves `interrupt` and `session_change` undeclared.
pub fn served() -> BTreeSet<Capability> {
    [
        Capability::TranscriptReads,
        Capability::RunOps,
        Capability::RunCancel,
        Capability::DispatchAttribution,
        Capability::Streaming,
        Capability::Compaction,
        // `compaction` requires `tags`, whose ops the group lists.
        Capability::Tags,
        Capability::ModelView,
        Capability::Steer,
        Capability::Queue,
        Capability::Retract,
        Capability::HoldToolCalls,
        Capability::HoldContextCeiling,
        Capability::HostMetadataChange,
        Capability::TagCheckpoint,
        Capability::RenderedModelInput,
    ]
    .into()
}

/// The kill points the fake can reach.
pub const KILL_POINTS: &[&str] = &[
    points::ADMITTED,
    points::SEND_RECORDED,
    points::STEP_RECORDED,
    points::DISPATCH_INTENT,
    points::TOOL_RESULT_RECORDED,
    points::TERMINAL,
    points::RETENTION_TOMBSTONED,
    points::COMPACTION_APPLIED,
    points::FOLD_RECORDED,
    points::USER_COMMAND_FORWARDED,
];

impl FakeSubject {
    pub fn new(defects: Defects) -> Self {
        Self {
            held_calls_group: false,
            head_sender_fixtures: false,
            head_sender_group: true,
            incarnation_fixtures: false,
            output_refs_group: false,
            descriptor_case: None,
            data_url_media: false,
            descriptor_fixtures: false,
            no_image_results: false,
            standalone_in_process: false,
            late_seal_reads: 0,
            retention_defect: RetentionDefect::None,
            retention_delete_ms: RETENTION_DELETE_MS,
            held_paused: false,
            advertise_status: false,
            cancel_step_boundary: true,
            defects,
            step_recovery: StepRecovery::Resume,
            compaction_route_mode: compaction::CompactionRouteMode::SingleSession,
            capabilities: served(),
            delivery_order_modes: None,
            without_plans: false,
            seal_compaction_setup_crash: false,
            kill_points: KILL_POINTS.to_vec(),
            claim_process_kill: false,
            queue_receipt_pending_then_unknown: false,
            steer_receipt_confirm: false,
            omit_first_steer_receipt: false,
            early_held_steer_receipt: None,
            pause_for: None,
            turn_system: true,
            host_prefix_hook: false,
            seed_hook: false,
            host_prefix_everywhere: 0,
            holds_reads_during_setup: false,
            cache_ttl_defect: CacheTtlDefect::None,
            current_case: Arc::new(Mutex::new(None)),
            route_calls: Arc::new(Mutex::new(Vec::new())),
            world: Mutex::new(None),
            modules: Mutex::new(Vec::new()),
        }
    }

    /// The shared world, built on first use from the configuration as it
    /// stands then.
    fn world(&self) -> Arc<World> {
        self.world
            .lock()
            .unwrap()
            .get_or_insert_with(|| {
                Arc::new(World {
                    held_replies: Mutex::new(BTreeMap::new()),
                    standalone_in_process: self.standalone_in_process,
                    host_prefix_everywhere: self.host_prefix_everywhere,
                    host_prefixes: Mutex::new(BTreeMap::new()),
                    host_cuts: Mutex::new(BTreeSet::new()),
                    holds_reads_during_setup: self.holds_reads_during_setup,
                    setup_holds: Mutex::new(BTreeMap::new()),
                    retention_defect: if RETENTION_DEFECTS_ENABLED {
                        self.retention_defect
                    } else {
                        RetentionDefect::None
                    },
                    retention_delete_ms: self.retention_delete_ms,
                    epoch: tokio::time::Instant::now(),
                    retention_clock_advanced_ms: AtomicU64::new(0),
                    held_paused: self.held_paused,
                    advertise_status: self.advertise_status,
                    cancel_step_boundary: self.cancel_step_boundary,
                    defects: self.defects,
                    data_url_media: self.data_url_media,
                    step_recovery: self.step_recovery,
                    queue_receipt_pending_then_unknown: self.queue_receipt_pending_then_unknown,
                    steer_receipt_confirm: self.steer_receipt_confirm,
                    omit_first_steer_receipt: self.omit_first_steer_receipt,
                    early_held_steer_receipt: self.early_held_steer_receipt.clone(),
                    held_steer_receipts: Mutex::new(Vec::new()),
                    held_steer_break_answers: AtomicUsize::new(0),
                    groups: self
                        .capabilities
                        .iter()
                        .filter_map(|c| c.group())
                        .map(str::to_owned)
                        .chain(self.held_calls_group.then(|| groups::HELD_CALLS.to_owned()))
                        .chain(
                            self.head_sender_group
                                .then(|| groups::HEAD_SENDER.to_owned()),
                        )
                        .chain(
                            self.output_refs_group
                                .then(|| groups::OUTPUT_REFS.to_owned()),
                        )
                        .collect(),
                    delivery_order_modes: self.delivery_order_modes.clone(),
                    scripts: Mutex::new(BTreeMap::new()),
                    model_calls: Mutex::new(BTreeSet::new()),
                    structured_output_support: Mutex::new(BTreeMap::new()),
                    model_requests: Mutex::new(BTreeMap::new()),
                    user_commands: Mutex::new(user_commands::Commands::default()),
                    system_text_default: Mutex::new(BTreeMap::new()),
                    pending_model_holds: Mutex::new(BTreeSet::new()),
                    held_models: Mutex::new(BTreeMap::new()),
                    staging_holds: Mutex::new(BTreeSet::new()),
                    compaction: Mutex::new(compaction::Providers::new(
                        self.seal_compaction_setup_crash,
                    )),
                    compaction_route_mode: self.compaction_route_mode,
                    invocations: Mutex::new(BTreeMap::new()),
                    held: Mutex::new(BTreeSet::new()),
                    released: Mutex::new(BTreeSet::new()),
                    transcript_calls: AtomicUsize::new(0),
                    run_result_calls: AtomicUsize::new(0),
                    adopt_calls: AtomicUsize::new(0),
                    turn_system: self.turn_system,
                    ceiling_holds: Mutex::new(BTreeSet::new()),
                    models: Mutex::new(BTreeMap::new()),
                    observed_cache_lifetimes: Mutex::new(BTreeMap::new()),
                    cache_clocks: Mutex::new(BTreeMap::new()),
                    cache_ttl_defect: self.cache_ttl_defect,
                    stream_holds: Mutex::new(BTreeMap::new()),
                    in_flight: Mutex::new(BTreeMap::new()),
                    late_seal_reads: AtomicUsize::new(self.late_seal_reads),
                    ride_scripts: Mutex::new(BTreeMap::new()),
                    rides: Mutex::new(BTreeMap::new()),
                    resolved_rides: Mutex::new(BTreeMap::new()),
                    ride_clock: AtomicU64::new(0),
                    current_case: Arc::clone(&self.current_case),
                    route_calls: Arc::clone(&self.route_calls),
                })
            })
            .clone()
    }

    fn start(&self, root: &Path) -> Result<FakeHandle, HarnessError> {
        let module = Module::open(root, self.world())?;
        self.modules.lock().unwrap().push(module.clone());
        Ok(FakeHandle(module))
    }

    fn mechanism(&self) -> KillMechanism {
        if self.claim_process_kill {
            KillMechanism::FaultHookThenProcessKill
        } else {
            KillMechanism::FaultHook
        }
    }

    /// How many `session.read` and `session.head` requests the runner has
    /// received, served or not.
    pub fn transcript_calls(&self) -> usize {
        self.world().transcript_calls.load(Ordering::SeqCst)
    }

    /// How many `run.result` requests the runner has received, served or
    /// not.
    pub fn run_result_calls(&self) -> usize {
        self.world().run_result_calls.load(Ordering::SeqCst)
    }

    /// How many `session.adopt` requests the runner has received, served or
    /// not.
    pub fn adopt_calls(&self) -> usize {
        self.world().adopt_calls.load(Ordering::SeqCst)
    }

    pub fn held_steer_break_answers(&self) -> usize {
        self.world().held_steer_break_answers.load(Ordering::SeqCst)
    }

    pub fn held_steer_receipts(&self) -> Vec<Option<Delivered>> {
        self.world().held_steer_receipts.lock().unwrap().clone()
    }

    pub fn enable_retention(&mut self) {
        self.capabilities.insert(Capability::Retention);
        self.capabilities.insert(Capability::RetentionClock);
        self.advertise_status = true;
    }
}

#[async_trait]
impl Harness for FakeSubject {
    type Handle = FakeHandle;
    type Route = FakeRoute;

    fn declared_points(&self) -> Vec<PointDeclaration> {
        self.kill_points
            .iter()
            .map(|point| PointDeclaration {
                point: KillPoint::new(*point),
                mechanisms: vec![self.mechanism()],
                note: "in-process fake: stops after the record syncs and discards its memory"
                    .into(),
            })
            .collect()
    }

    async fn spawn(&self, state_root: &Path) -> Result<FakeHandle, HarnessError> {
        self.start(state_root)
    }

    async fn route(&self, _: &FakeHandle, _: &RouteStamp) -> Result<FakeRoute, HarnessError> {
        Err(HarnessError::new(
            "every route of this role is session-bound",
        ))
    }

    async fn kill_at(
        &self,
        handle: FakeHandle,
        point: &KillPoint,
        trigger: Trigger<'_>,
    ) -> Result<KillReport, HarnessError> {
        *handle.0.kill_hook.lock().unwrap() = Some(point.as_str().to_owned());
        trigger.await;
        let reached = handle.0.hook_fired.load(Ordering::SeqCst);
        drop(handle);
        if !reached {
            return Err(HarnessError::new(format!(
                "the trigger finished without reaching {point}"
            )));
        }
        Ok(KillReport {
            point: point.clone(),
            mechanism: self.mechanism(),
        })
    }

    async fn restart(&self, state_root: &Path) -> Result<FakeHandle, HarnessError> {
        self.start(state_root)
    }
}

#[async_trait]
impl LlmRunnerSubject for FakeSubject {
    async fn prepare_system_text_default(
        &self,
        session: &str,
        provides: &[String],
        item: &Value,
    ) -> Result<Map<String, Value>, HarnessError> {
        self.prepare_default_text(session, provides, item)
    }

    async fn system_text_default_observation(
        &self,
        handle: &FakeHandle,
        session: &str,
    ) -> Result<cortexkit_role_llm_runner_conformance::SystemTextDefaultObservation, HarnessError>
    {
        self.observe_default_text(handle, session)
    }
    async fn cut_incarnation_log(&self, session: &str) -> Result<bool, HarnessError> {
        if !self.incarnation_fixtures {
            return Ok(false);
        }
        for module in self.modules.lock().unwrap().clone() {
            if !module.alive.load(Ordering::SeqCst) {
                continue;
            }
            let sess = module.sessions.lock().unwrap()[session].clone();
            let message = sess
                .messages
                .first()
                .ok_or_else(|| HarnessError::new("no message to cut"))?;
            let digest = cortexkit_role_llm_runner::content_digest::content_digest(
                &module.read_message(&sess, message).message,
            );
            module.commit(session, json!({"kind":"cut", "first_ordinal":message.ordinal + 1,
                "through_digest":digest, "through_mid":message.mid, "through_written_at":message.written_at}))
                .map_err(|_| HarnessError::new("killed while cutting the log"))?;
        }
        Ok(true)
    }

    async fn recreate_incarnation_log(&self, session: &str) -> Result<bool, HarnessError> {
        if !self.incarnation_fixtures {
            return Ok(false);
        }
        for module in self.modules.lock().unwrap().clone() {
            if !module.alive.load(Ordering::SeqCst) {
                continue;
            }
            let sess = module.sessions.lock().unwrap()[session].clone();
            // The fake shares one file across keys, so replace only this key's
            // records; replay must see a new log, not an old log plus a new ID.
            let original = std::fs::read_to_string(&module.log)
                .map_err(|e| HarnessError::new(e.to_string()))?;
            let kept: Vec<&str> = original
                .lines()
                .filter(|line| {
                    let record: Value = serde_json::from_str(line).unwrap();
                    record["session"] != json!(session)
                })
                .collect();
            let mut remaining = kept.join("\n");
            if !remaining.is_empty() {
                remaining.push('\n');
            }
            std::fs::write(&module.log, remaining).map_err(|e| HarnessError::new(e.to_string()))?;
            module.sessions.lock().unwrap().remove(session);
            let mut start = json!({"kind":"start", "owner":sess.owner,
                "lineage":format!("recreated-{}", new_incarnation()), "tools":[]});
            if self.defects.incarnation_reused_on_recreation {
                start["incarnation"] = json!(sess.incarnation);
            }
            module
                .commit(session, start)
                .map_err(|_| HarnessError::new("killed while recreating log"))?;
        }
        Ok(true)
    }

    fn head_sender_identities(&self) -> Option<[PlanSenderIdentity; 2]> {
        self.head_sender_fixtures.then(|| {
            [
                PlanSenderIdentity::new(
                    RouteStamp {
                        principal: SENDER_MODULE.into(),
                        scope: None,
                    },
                    SENDER_MODULE,
                ),
                PlanSenderIdentity::new(
                    RouteStamp {
                        principal: "fake.sender-b".into(),
                        scope: None,
                    },
                    "fake.sender-b",
                ),
            ]
        })
    }
    fn session_usage_fixtures(&self) -> bool {
        true
    }

    async fn rewind_session_usage(&self, session: &str) -> Result<bool, HarnessError> {
        for module in self.modules.lock().unwrap().clone() {
            if module.alive.load(Ordering::SeqCst) {
                module
                    .commit(session, json!({"kind":"usage-rewind"}))
                    .map_err(|_| HarnessError::new("killed while rewinding"))?;
            }
        }
        Ok(true)
    }

    async fn switch_head_model(
        &self,
        session: &str,
    ) -> Result<Option<cortexkit_role_llm_runner::read::HeadModel>, HarnessError> {
        let model =
            cortexkit_role_llm_runner::read::HeadModel::new(FAKE_PROVIDER, "recorded-next-model")
                .with_variant("fast");
        for module in self.modules.lock().unwrap().clone() {
            if module.alive.load(Ordering::SeqCst) {
                module
                    .commit(session, json!({"kind":"head-model-switch", "model":model}))
                    .map_err(|_| HarnessError::new("killed while switching model"))?;
            }
        }
        Ok(Some(model))
    }

    async fn hold_send_staging(&self, session: &str) -> Result<bool, HarnessError> {
        self.world()
            .staging_holds
            .lock()
            .unwrap()
            .insert(session.into());
        Ok(true)
    }
    async fn send_staged(&self, session: &str, _send_id: &str) -> Result<(), HarnessError> {
        self.world().staging_holds.lock().unwrap().remove(session);
        Ok(())
    }
    fn output_refs_fields(&self, session: &str) -> Result<Map<String, Value>, HarnessError> {
        let mut fields = self.send_fields(session, true);
        fields["plan"]["render_tags"] = json!(true);
        fields["plan"]["items"] = json!([{"provider":TOOL_MODULE,"output_refs":true}]);
        Ok(fields)
    }

    async fn install_output_ref_held(
        &self,
        handle: &FakeHandle,
        session: &str,
        placeholder: &str,
    ) -> Result<String, HarnessError> {
        handle
            .0
            .commit(
                session,
                json!({"kind":"refs_held","placeholder":placeholder}),
            )
            .map_err(|Killed| HarnessError::new("killed installing held part"))?;
        let sessions = handle.0.sessions.lock().unwrap();
        let sess = &sessions[session];
        let mid = &sess.messages.last().unwrap().mid;
        Ok(tags::current(sess)
            .iter()
            .find(|((m, _), _)| m == mid)
            .unwrap()
            .1
            .to_string())
    }

    async fn complete_output_ref_held(
        &self,
        handle: &FakeHandle,
        session: &str,
        answer: &str,
    ) -> Result<(), HarnessError> {
        handle
            .0
            .commit(session, json!({"kind":"refs_late","answer":answer}))
            .map_err(|Killed| HarnessError::new("killed completing held part"))
    }

    async fn renumber_output_refs(
        &self,
        handle: &FakeHandle,
        session: &str,
    ) -> Result<(), HarnessError> {
        handle
            .0
            .commit(session, json!({"kind":"refs_renumber"}))
            .map_err(|Killed| HarnessError::new("killed renumbering"))
    }
    async fn install_held_reply(
        &self,
        _session: &str,
        arguments: &Value,
        reason: &str,
    ) -> Result<(), HarnessError> {
        self.install_held(arguments, reason);
        Ok(())
    }
    async fn await_held_reply(&self, arguments: &Value) -> Result<String, HarnessError> {
        self.observed_held(arguments)
    }
    async fn publish_late_entry(
        &self,
        arguments: &Value,
        entry: cortexkit_role_llm_runner_conformance::ScriptedLateEntry,
    ) -> Result<(), HarnessError> {
        self.publish_late(arguments, entry)
    }
    fn attachment_model_fields(&self, _session: &str, accepts: bool) -> Option<Map<String, Value>> {
        Some(Map::from_iter([(
            "attachment_accepts".into(),
            json!(accepts),
        )]))
    }

    fn attachment_tool_result(&self, png: &[u8]) -> Option<Value> {
        use base64::{engine::general_purpose::STANDARD, Engine};
        Some(
            json!({"type":"tool_result","tool_use_id":"attachment-tool-media","content":[{"type":"text","text":"before-media"},{"type":"image","source":{"type":"base64","media_type":"image/png","data":STANDARD.encode(png)}},{"type":"text","text":"after-media"}]}),
        )
    }

    async fn restart_attachments(
        &self,
        handle: &FakeHandle,
        session: &str,
    ) -> Result<FakeRoute, HarnessError> {
        let broken = self.defects.attachments == "attachments_restart_gone"
            && session.contains("attachments_restart_gone");
        *handle.0.attachments.lock().unwrap() =
            attachments::Store::reopen(handle.0.log.parent().unwrap(), broken);
        self.session_route(handle, session, &self.owner_stamp())
            .await
    }
    fn multipart_tool_result(&self, texts: &[String]) -> Option<Value> {
        self.descriptor_fixtures.then(|| json!({"type":"tool_result","tool_use_id":"fallback-call","content":texts.iter().map(|s| json!({"type":"text","text":s})).collect::<Vec<_>>()}))
    }
    fn read_parts(&self, message: &Value) -> Vec<(String, Value)> {
        media::parts(message)
    }

    fn media_tool_result(&self, bytes: &[u8], nested: bool, scenario: &str) -> Option<Value> {
        use base64::{engine::general_purpose::STANDARD, Engine};
        if !self.descriptor_fixtures || self.descriptor_case.is_some_and(|case| case != scenario) {
            return None;
        }
        let image = json!({"type":"image","source":{"type":"base64","media_type":"image/png","data":STANDARD.encode(bytes)}});
        Some(if nested {
            json!({"type":"tool_result","tool_use_id":"descriptor-call","is_error":true,"content":[image]})
        } else {
            image
        })
    }
    async fn cut_off_user_command(
        &self,
        _handle: &FakeHandle,
        session: &str,
        after: bool,
    ) -> Result<(), HarnessError> {
        self.world()
            .user_commands
            .lock()
            .unwrap()
            .sessions
            .get_mut(session)
            .unwrap()
            .cutoff = Some(after);
        Ok(())
    }
    fn standalone_in_process(&self) -> bool {
        self.standalone_in_process
    }

    async fn require_setup_again(
        &self,
        handle: &FakeHandle,
        session: &str,
    ) -> Result<(), HarnessError> {
        handle
            .0
            .commit(session, json!({"kind":"require_setup"}))
            .map_err(|Killed| HarnessError::new("killed"))
    }
    async fn publish_compaction_view(
        &self,
        session: &str,
        view: cortexkit_role_llm_runner_conformance::ScriptedView,
    ) -> Result<u64, HarnessError> {
        Ok(self.publish_view(session, view))
    }

    async fn seed_session_history(
        &self,
        handle: &FakeHandle,
        session: &str,
        messages: u64,
        window: u64,
    ) -> Result<(), HarnessError> {
        if !self.seed_hook {
            return Err(HarnessError::new("this fake is configured without seeding"));
        }
        handle
            .0
            .commit(
                session,
                json!({"kind":"seed", "count": messages, "window": window}),
            )
            .map_err(|Killed| HarnessError::new("killed seeding"))
    }

    async fn seed_host_history(
        &self,
        _handle: &FakeHandle,
        session: &str,
        count: u64,
        cut: bool,
    ) -> Result<(), HarnessError> {
        if !self.host_prefix_hook {
            return Err(HarnessError::new(
                "this fake is configured without host history",
            ));
        }
        let world = self.world();
        world
            .host_prefixes
            .lock()
            .unwrap()
            .insert(session.into(), history::host_entries(session, count));
        if cut {
            world.host_cuts.lock().unwrap().insert(session.into());
        }
        Ok(())
    }

    async fn seed_untimed_tool_call(
        &self,
        handle: &FakeHandle,
        session: &str,
        call: &ScriptedToolCall,
    ) -> Result<(), HarnessError> {
        handle
            .0
            .commit(
                session,
                json!({"kind":"legacy_call", "call_key": format!("legacy-{}", call.tool_call_id),
                       "tool_call_id": call.tool_call_id, "tool": call.tool,
                       "arguments": call.arguments, "content": call.result}),
            )
            .map_err(|Killed| HarnessError::new("killed seeding a call"))
    }

    async fn hold_next_compaction_setup(&self, session: &str) -> Result<(), HarnessError> {
        self.world()
            .setup_holds
            .lock()
            .unwrap()
            .insert(session.into(), history::SetupHold::Armed);
        Ok(())
    }

    async fn await_compaction_setup_held(&self, session: &str) -> Result<(), HarnessError> {
        match self.world().setup_holds.lock().unwrap().get(session) {
            Some(history::SetupHold::Held) => Ok(()),
            other => Err(HarnessError::new(format!(
                "the runner made no Setup call to hold: {other:?}"
            ))),
        }
    }

    async fn release_compaction_setup(&self, session: &str) -> Result<(), HarnessError> {
        self.world()
            .setup_holds
            .lock()
            .unwrap()
            .insert(session.into(), history::SetupHold::Released);
        self.drive_compaction_modules(session)
    }

    fn holds_reads_during_setup(&self) -> bool {
        self.holds_reads_during_setup
    }

    async fn revert_session(
        &self,
        handle: &FakeHandle,
        session: &str,
        mid: &str,
    ) -> Result<(), HarnessError> {
        handle
            .0
            .commit(session, json!({"kind":"revert", "mid": mid}))
            .map_err(|Killed| HarnessError::new("killed reverting"))
    }

    async fn prepare_user_commands(
        &self,
        session: &str,
        fixture: &cortexkit_role_llm_runner_conformance::UserCommandFixture,
    ) -> Result<(String, Map<String, Value>), HarnessError> {
        self.prepare_commands(session, fixture)
    }

    async fn user_command_observation(
        &self,
        session: &str,
    ) -> Result<cortexkit_role_llm_runner_conformance::UserCommandObservation, HarnessError> {
        Ok(self.world().user_commands.lock().unwrap().sessions[session]
            .observation
            .clone())
    }
    fn capabilities(&self) -> BTreeSet<Capability> {
        self.capabilities.clone()
    }

    fn owner_stamp(&self) -> RouteStamp {
        RouteStamp {
            principal: "reserved:owner".into(),
            scope: None,
        }
    }

    fn stranger_stamp(&self) -> RouteStamp {
        RouteStamp {
            principal: "reserved:stranger".into(),
            scope: None,
        }
    }

    async fn session_route(
        &self,
        handle: &FakeHandle,
        session: &str,
        stamp: &RouteStamp,
    ) -> Result<FakeRoute, HarnessError> {
        self.world()
            .user_commands
            .lock()
            .unwrap()
            .routed
            .insert(session.to_owned());
        Ok(FakeRoute {
            module: handle.0.clone(),
            session: session.to_owned(),
            stamp: stamp.clone(),
        })
    }

    fn send_fields(&self, session: &str, first: bool) -> Map<String, Value> {
        let mut fields = Map::new();
        if first && !self.without_plans {
            fields.insert("plan".into(), json!({ "tools": [SCRIPTED_TOOL] }));
            fields.insert("model".into(), json!("fake-model"));
            if self.has_compaction_script(session) {
                fields.get_mut("plan").unwrap()["compaction_item"] =
                    json!({"provider":"fake.compaction"});
            }
        }
        fields
    }

    fn send_fields_with_tools(
        &self,
        _session: &str,
        first: bool,
        tools: &[&str],
    ) -> Option<Map<String, Value>> {
        let mut fields = Map::new();
        if first {
            fields.insert("plan".into(), json!({ "tools": tools }));
            fields.insert("model".into(), json!("fake-model"));
        }
        Some(fields)
    }

    async fn set_structured_output_support(
        &self,
        session: &str,
        supported: Option<bool>,
    ) -> Result<(), HarnessError> {
        self.world()
            .structured_output_support
            .lock()
            .unwrap()
            .insert(session.into(), supported);
        Ok(())
    }

    async fn model_requests(
        &self,
        session: &str,
    ) -> Result<Vec<ModelRequestObservation>, HarnessError> {
        Ok(self
            .world()
            .model_requests
            .lock()
            .unwrap()
            .get(session)
            .cloned()
            .unwrap_or_default())
    }

    fn send_fields_with_plan(
        &self,
        session: &str,
        first: bool,
        members: &Map<String, Value>,
    ) -> Option<Map<String, Value>> {
        if self.without_plans {
            return None;
        }
        let mut fields = self.send_fields(session, first);
        if let Some(plan) = fields.get_mut("plan").and_then(Value::as_object_mut) {
            plan.extend(members.clone());
        }
        Some(fields)
    }

    fn plan_sender(&self) -> Option<PlanSenderIdentity> {
        Some(PlanSenderIdentity::new(
            RouteStamp {
                principal: SENDER_MODULE.into(),
                scope: None,
            },
            SENDER_MODULE,
        ))
    }

    fn turn_system_supported(&self) -> bool {
        self.turn_system
    }

    async fn hold_context_ceiling(&self, session: &str) -> Result<(), HarnessError> {
        self.world()
            .ceiling_holds
            .lock()
            .unwrap()
            .insert(session.to_owned());
        Ok(())
    }

    async fn end_context_ceiling_wait(
        &self,
        session: &str,
        view_arrived: bool,
    ) -> Result<(), HarnessError> {
        let modules = self.modules.lock().unwrap().clone();
        let module = modules
            .iter()
            .rev()
            .find(|module| module.alive.load(Ordering::SeqCst))
            .ok_or_else(|| HarnessError::new("no module is running"))?;
        module
            .end_ceiling_wait(session, view_arrived)
            .map_err(|Killed| HarnessError::new("the module was killed"))
    }

    async fn compact_in_host(
        &self,
        handle: &FakeHandle,
        session: &str,
    ) -> Result<(), HarnessError> {
        handle
            .0
            .commit(session, json!({ "kind": "host_compact" }))
            .map_err(|Killed| HarnessError::new("the module was killed"))
    }

    async fn drop_marker_key(
        &self,
        handle: &FakeHandle,
        session: &str,
    ) -> Result<(), HarnessError> {
        handle
            .0
            .commit(session, json!({ "kind": "drop_marker_key" }))
            .map_err(|Killed| HarnessError::new("the module was killed"))
    }

    async fn host_stored_text(
        &self,
        handle: &FakeHandle,
        session: &str,
        marker: &str,
    ) -> Result<Option<String>, HarnessError> {
        let sessions = handle.0.sessions.lock().unwrap();
        Ok(sessions
            .get(session)
            .and_then(|sess| tags::host_stored(sess, marker, &self.defects)))
    }

    fn image_tool_result(&self, marker: &str) -> Option<Value> {
        (!self.no_image_results).then(|| tags::image_result(marker))
    }

    async fn fork_session(
        &self,
        handle: &FakeHandle,
        parent: &str,
        fork: &str,
    ) -> Result<(), HarnessError> {
        handle
            .0
            .commit(fork, json!({ "kind": "fork", "parent": parent }))
            .map_err(|Killed| HarnessError::new("the module was killed"))
    }

    async fn change_host_metadata(
        &self,
        handle: &FakeHandle,
        session: &str,
    ) -> Result<(), HarnessError> {
        handle
            .0
            .commit(session, json!({ "kind": "host_meta" }))
            .map_err(|Killed| HarnessError::new("the module was killed"))
    }

    fn refresh_plan(
        &self,
        _session: &str,
        variant: cortexkit_role_llm_runner_conformance::RefreshVariant,
    ) -> Option<cortexkit_role_llm_runner_conformance::RefreshPlan> {
        session_change::refresh_plan(variant)
    }

    fn host_plan(&self, _session: &str, alternate: bool) -> Option<Map<String, Value>> {
        let mut plan = Map::new();
        plan.insert("tools".into(), json!([SCRIPTED_TOOL]));
        if alternate {
            plan.insert("variant".into(), json!("alternate"));
        }
        Some(plan)
    }

    async fn start_host_session(
        &self,
        handle: &FakeHandle,
        session: &str,
        plan: &Map<String, Value>,
        prompt: Option<&str>,
    ) -> Result<(), HarnessError> {
        handle
            .0
            .host_start(session, plan, prompt)
            .map_err(|Killed| HarnessError::new("the module was killed"))
    }

    fn tool_provider_module(&self) -> String {
        TOOL_MODULE.into()
    }

    async fn install_script(&self, session: &str, script: Script) -> Result<(), HarnessError> {
        self.world()
            .scripts
            .lock()
            .unwrap()
            .insert(session.to_owned(), script);
        Ok(())
    }

    async fn install_attribution_fixture(
        &self,
        handle: &FakeHandle,
        session: &str,
    ) -> Result<cortexkit_role_llm_runner_conformance::AttributionFixture, HarnessError> {
        use cortexkit_role_llm_runner_conformance::AttributionFixture;
        if self.defects.attribution_fixture_missing {
            return Ok(AttributionFixture::NotSupported);
        }
        if self.defects.attribution_cannot_hold_unattributable {
            return Ok(AttributionFixture::CannotHoldUnattributable);
        }
        let ordinal = {
            let sessions = handle.0.sessions.lock().unwrap();
            sessions[session].base + sessions[session].messages.len() as u64
        };
        let marker = format!("{session}-legacy-unknown-origin");
        handle
            .0
            .commit(session, json!({"kind":"unattributed", "marker":marker}))
            .map_err(|Killed| HarnessError::new("killed installing attribution fixture"))?;
        Ok(AttributionFixture::Installed { ordinal, marker })
    }

    async fn tool_invocations(&self, arguments: &Value) -> usize {
        self.world()
            .invocations
            .lock()
            .unwrap()
            .get(&arguments.to_string())
            .copied()
            .unwrap_or(0)
    }

    fn not_executed_tool_result(
        &self,
        message: &Value,
        tool_call_id: &str,
    ) -> Option<cortexkit_role_llm_runner_conformance::ToolResultObservation> {
        if message["tool_call_id"].as_str()? != tool_call_id {
            return None;
        }
        Some(
            cortexkit_role_llm_runner_conformance::ToolResultObservation::new(
                message["content"]["executed"].as_bool()?,
                message["content"]["error"]["code"].as_str()?,
            ),
        )
    }

    async fn install_compaction_setup(
        &self,
        session: &str,
        setup: cortexkit_role_llm_runner_conformance::CompactionSetup,
    ) -> Result<(), HarnessError> {
        self.install_compaction(session, setup);
        Ok(())
    }
    async fn compaction_observation(
        &self,
        session: &str,
    ) -> Result<cortexkit_role_llm_runner_conformance::CompactionObservation, HarnessError> {
        Ok(self.observe_compaction(session))
    }
    async fn advance_retention_clock(
        &self,
        session: &str,
        milliseconds: u64,
    ) -> Result<(), HarnessError> {
        self.world()
            .retention_clock_advanced_ms
            .fetch_add(milliseconds, Ordering::SeqCst);
        for module in self.modules.lock().unwrap().clone() {
            if module.alive.load(Ordering::SeqCst) {
                module.sweep(session).map_err(|Killed| {
                    HarnessError::new("module was killed while advancing retention clock")
                })?;
            }
        }
        Ok(())
    }
    async fn release_compaction_model(&self, session: &str) -> Result<(), HarnessError> {
        self.release_model(session)
    }
    async fn durable_compaction_setup(
        &self,
        handle: &FakeHandle,
        session: &str,
    ) -> Result<Option<cortexkit_role_llm_runner_conformance::CompactionMessage>, HarnessError>
    {
        self.durable_setup(handle, session)
    }

    async fn await_tool_call(&self, arguments: &Value) -> Result<(), HarnessError> {
        for _ in 0..100 {
            if self
                .world()
                .held
                .lock()
                .unwrap()
                .contains(&arguments.to_string())
            {
                return Ok(());
            }
            tokio::task::yield_now().await;
        }
        Err(HarnessError::new("the call was never held"))
    }

    fn serializer_profile(&self, session: &str) -> Option<String> {
        profiled(session)
            .then(|| cortexkit_role_llm_runner::serializer_profile::ANTHROPIC_MESSAGES.to_owned())
    }

    async fn hold_next_model_stream(
        &self,
        session: &str,
        parts: usize,
    ) -> Result<(), HarnessError> {
        let world = self.world();
        world
            .stream_holds
            .lock()
            .unwrap()
            .insert(session.into(), parts);
        world
            .pending_model_holds
            .lock()
            .unwrap()
            .insert(session.into());
        Ok(())
    }

    async fn advance_prompt_cache_clock(
        &self,
        session: &str,
        milliseconds: u64,
    ) -> Result<(), HarnessError> {
        self.world()
            .cache_clocks
            .lock()
            .unwrap()
            .entry(session.into())
            .or_default()
            .now += milliseconds;
        Ok(())
    }

    async fn switch_model(&self, session: &str, model: &str) -> Result<(), HarnessError> {
        self.world()
            .models
            .lock()
            .unwrap()
            .insert(session.into(), model.into());
        Ok(())
    }

    async fn set_observed_cache_lifetime(
        &self,
        session: &str,
        milliseconds: Option<u64>,
    ) -> Result<(), HarnessError> {
        self.world()
            .observed_cache_lifetimes
            .lock()
            .unwrap()
            .insert(session.into(), milliseconds);
        Ok(())
    }

    async fn hold_next_model_step(&self, session: &str) -> Result<(), HarnessError> {
        self.world()
            .pending_model_holds
            .lock()
            .unwrap()
            .insert(session.into());
        Ok(())
    }

    async fn await_model_step(&self, session: &str) -> Result<(), HarnessError> {
        if self
            .world()
            .held_models
            .lock()
            .unwrap()
            .contains_key(session)
        {
            Ok(())
        } else {
            Err(HarnessError::new("the model request was never held"))
        }
    }

    async fn release_model_step(&self, session: &str) -> Result<(), HarnessError> {
        let run_id = self
            .world()
            .held_models
            .lock()
            .unwrap()
            .remove(session)
            .ok_or_else(|| HarnessError::new("the model request is not held"))?;
        self.world().in_flight.lock().unwrap().remove(session);
        for module in self.modules.lock().unwrap().clone() {
            if module.alive.load(Ordering::SeqCst) {
                module
                    .drive(session, &run_id)
                    .map_err(|_| HarnessError::new("runner was killed"))?;
            }
        }
        Ok(())
    }

    async fn model_invocations(&self, session: &str) -> Result<usize, HarnessError> {
        Ok(self
            .world()
            .model_calls
            .lock()
            .unwrap()
            .iter()
            .filter(|(name, _, _)| name == session)
            .count())
    }

    async fn release_tool_call(&self, arguments: &Value) -> Result<(), HarnessError> {
        let world = self.world();
        let key = arguments.to_string();
        if !world.held.lock().unwrap().remove(&key) {
            return Err(HarnessError::new("the call is not held"));
        }
        world.released.lock().unwrap().insert(key);
        let modules = self.modules.lock().unwrap().clone();
        for module in modules {
            if module.alive.load(Ordering::SeqCst) {
                module
                    .complete_held(arguments)
                    .map_err(|Killed| HarnessError::new("the module was killed"))?;
            }
        }
        Ok(())
    }

    async fn retention_deletion_finished(
        &self,
        handle: &FakeHandle,
        session: &str,
    ) -> Result<bool, HarnessError> {
        handle
            .0
            .sweep(session)
            .map_err(|Killed| HarnessError::new("killed during deletion inspection"))?;
        let state = handle.0.sessions.lock().unwrap().get(session).cloned();
        let disk = std::fs::read_to_string(&handle.0.log).unwrap_or_default();
        let content_records = disk
            .lines()
            .map(|line| serde_json::from_str::<Value>(line).unwrap())
            .any(|r| {
                r["session"] == json!(session)
                    && !matches!(r["kind"].as_str(), Some("tombstone" | "deleted"))
            });
        Ok(
            state.is_some_and(|s| s.deleted && s.messages.is_empty() && s.calls.is_empty())
                && !content_records,
        )
    }

    fn retention_probe_params(
        &self,
        _session: &str,
        method: &str,
    ) -> Result<Option<Vec<Value>>, HarnessError> {
        match method {
            "session.list" | "session.export" => Ok(Some(vec![json!({})])),
            _ => Err(HarnessError::new(format!("unclassified op: {method}"))),
        }
    }

    async fn observed_ride_wait_max_ms(&self, session: &str) -> Result<Option<u64>, HarnessError> {
        if self.setup_received(session) {
            Ok(Some(RIDE_WAIT_MAX_MS))
        } else {
            Err(HarnessError::new(
                "the session's provider received no Setup",
            ))
        }
    }

    async fn script_next_ride(
        &self,
        session: &str,
        ride: cortexkit_role_llm_runner_conformance::ScriptedRide,
    ) -> Result<(), HarnessError> {
        self.world()
            .ride_scripts
            .lock()
            .unwrap()
            .insert(session.into(), ride);
        Ok(())
    }

    async fn pause(&self) {
        // Rewrite rides are timed on their own clock, so a case waiting on
        // one moves it without real or Tokio time passing.
        self.world()
            .ride_clock
            .fetch_add(RIDE_TICK_MS, Ordering::SeqCst);
        match self.pause_for {
            None => tokio::task::yield_now().await,
            Some(pause) => tokio::time::sleep(pause).await,
        }
    }
}
