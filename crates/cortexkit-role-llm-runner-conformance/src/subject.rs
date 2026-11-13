//! What the runner adapter supplies beyond its harness: the capabilities
//! it declares, how to reach a session's management route, and a scripted
//! model and tool endpoints.

use std::collections::BTreeSet;

use async_trait::async_trait;
use cortexkit_role_harness::{Harness, HarnessError, RouteStamp};
use cortexkit_role_llm_runner::capabilities as groups;
use serde_json::{Map, Value};

/// A native delayed-storage fixture's captured wire frames. Construct with
/// [`Self::new`]; the phases are separated by actual storage (or an abort).
#[derive(Clone, Debug)]
#[non_exhaustive]
pub struct LiveIdsObservation {
    pub before_storage: Vec<Value>,
    pub after_storage: Vec<Value>,
    pub stored_mid: Option<String>,
}

impl LiveIdsObservation {
    pub fn new(
        before_storage: Vec<Value>,
        after_storage: Vec<Value>,
        stored_mid: Option<String>,
    ) -> Self {
        Self {
            before_storage,
            after_storage,
            stored_mid,
        }
    }
}

/// A capability a conformance case requires of the runner or its harness.
/// A case whose requirement the runner adapter does not declare is reported as
/// skipped with the missing names, never as passed.
///
/// Non-exhaustive: the role adds groups over time, so a `match` outside this
/// crate keeps a wildcard arm.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
#[non_exhaustive]
pub enum Capability {
    /// Session cache lifetime and warming requests.
    CachePolicy,
    /// Opaque session labels with frozen labelled-session admission.
    SessionClass,
    /// Applied plan system text and offered tools, admitted like session reads.
    PlanView,
    /// Provider service levels; the kit checks omitted served metadata on unknown wires.
    ServiceTier,
    /// Durable plan-selected model and next-run tier.
    HeadSelection,
    /// In-place pruning of archived reasoning, with the native fixture hooks.
    ArchivePrunesReasoning,
    /// Preset-less system-text admission checked against the daemon catalog.
    SystemTextDefault,
    /// Chunked uploads, send references and session-scoped media reads.
    Attachments,
    /// Non-template sends with media content need no prompt text.
    MediaOnlySend,
    /// The `transcript_reads` group: `session.read` and `session.head`.
    TranscriptReads,
    /// The `host_history` group: `session.history`, the session's whole
    /// surviving history paged by opaque cursors.
    HostHistory,
    /// The `run_ops` group: `run.result` and run attribution.
    RunOps,
    /// Poll run.status, without transcript reads.
    RunStatus,
    /// Cancel active or paused runs under the session owner's scope.
    RunCancel,
    /// Withdraw a submission before it starts.
    Retract,
    /// The `dispatch_attribution` group: per-call fields on a page.
    DispatchAttribution,
    /// The `streaming` group: `session.subscribe` from a page's `head`.
    Streaming,
    /// The `steer` delivery mode of `session.send`.
    Steer,
    /// The `queue` delivery mode of `session.send`. Cases start every
    /// session it writes with a `queue` send (the mode an absent `delivery`
    /// means), so every case that needs a written session requires it.
    Queue,
    /// Admission of a session's frozen plan, including explicit compaction preset validation.
    Plans,
    /// The `interrupt` delivery mode of `session.send`.
    Interrupt,
    /// The `session_change` group: `session.refresh`,
    /// `session.refresh_policy` and `session.flush_prefix`. Its cases need
    /// [`LlmRunnerSubject::refresh_plan`] and
    /// [`LlmRunnerSubject::model_requests`].
    SessionChange,
    /// Opt-in plan notes; needs the defaulted `plan_notes_fixture` hook.
    PlanNotes,
    /// Inline reads that reference media without describing large text.
    MediaRefs,
    /// Initial host-history intake state on the head; optional seeded-intake hooks.
    HistoryIntake,
    /// Whole-session retention with limits from `role.describe.retention`.
    Retention,
    /// Per-session storage policies and owner release.
    SessionRetention,
    /// The durable session input-token cap on `session.send.budget`.
    InputTokenCap,
    /// Positive model-response limit in `stop_when.max_steps`.
    MaxSteps,
    /// Positive per-response output cap in `generation.max_output_tokens`.
    MaxOutputTokens,
    /// Per-response `generation.temperature`. The scripted model must accept
    /// the full 0 to 2 range so the kit can observe unchanged forwarding.
    Temperature,
    /// An empty `tools` list suppresses plan tools and host built-ins.
    NoTools,
    /// Stable trailing directives on completed runs, requested at admission.
    TurnDirectives,
    /// Durable reasons recorded when sealing interrupted runs.
    InterruptReason,
    /// Schema-shaped run output on `session.send.structured_output`.
    StructuredOutput,
    /// `session.adopt`: the owner takes over a session the host started
    /// with a plan applied, without sending a prompt.
    Adopt,
    /// Open an unknown plain-scope session through `session.adopt`, applying
    /// its plan without a prompt. Requires `adopt` and `plans` on the runner.
    AdoptOpen,
    /// `run.tool_calls`: a run's tool calls as a paged list.
    RunToolCalls,
    /// Per-run token usage (`usage`) on `run.result` and `run.status`.
    RunUsage,
    /// Normalized last-model-step finish on results and ended status.
    FinishReason,
    /// Durable session spend on `session.head`.
    SessionUsage,
    /// `session.send` with `delivery_order`, plan-named senders and plan
    /// templates.
    DeliveryOrder,
    /// One-hop retry/status lookup through a plan sender's predecessor.
    SenderSuccession,
    /// Trusted relay headers, verified facts and header-free display rows.
    SenderAttribution,
    /// Runner-owned notices, reader facts and delivery events.
    Notices,
    /// Positive agent/automatic-notice marks on sends and display facts.
    MessageMarks,
    /// Agent display names and notice marks on carried agent words.
    NoticeWithSpeaksFor,
    /// Owner-only protection for notices carrying a person's words.
    NoticeKeep,
    /// Resolve model-visible part numbers in the request that produced a call.
    TagRead,
    /// Opt-in client ids, tool times, assistant completion times and reasoning
    /// summaries on raw read entries.
    ReadDisplayFacts,
    /// Guaranteed caller-visible module-send identities on read envelopes.
    SendIds,
    /// `send.status`: what a re-send of a `send_id` would answer, without
    /// its body.
    SendStatus,
    /// Owner user-command relay, with replies isolated from model history.
    UserCommands,
    /// The `history_display` group: `session.history` with `form:
    /// "display"`, each entry as a canonical display row.
    HistoryDisplay,
    /// The `live_events` group: typed control and display events, `seq`
    /// on every frame and the attach replay of the step in flight on
    /// `session.subscribe`.
    LiveEvents,
    /// Opt-in provisional display ids until the host stores a message.
    LiveIds,
    /// The `client_message_id` group: `client_message_id` on
    /// `session.send`, shown on the user message's display row.
    ClientMessageId,
    /// The `run_cause` group: `send_id` on the live `run_started` event,
    /// naming the send that started the run, absent only for a turn the
    /// host started with no send.
    RunCause,
    /// Non-stored step-finished hints joined to native durable history.
    StepEvents,
    /// Sign-in methods whose accounts the runner admits through routing.
    RoutedAccounts,
    /// Routing's provider and model names for runner-owned account admission.
    AccountRoute,
    /// Runner-held admissions and runner-sent blocking account observations.
    AdmissionReports,
    /// Private read-only model conversations rooted in a head snapshot.
    ForkAnswer,
    /// The runner adapter can hold a session's next request at the context
    /// ceiling, as a missing compaction view would, and end the wait either
    /// way. See [`LlmRunnerSubject::hold_context_ceiling`].
    HoldContextCeiling,
    /// The runner adapter can change a session's metadata in the host (a
    /// title, a timestamp the host keeps), a change that does not reach the
    /// model. See [`LlmRunnerSubject::change_host_metadata`].
    HostMetadataChange,
    /// The runner under test can move its retention clock forward on request, so expiry and deletion happen without waiting in real time. For tests only; release builds must not offer it.
    RetentionClock,
    /// The runner adapter's scripted tool provider can hold a received call
    /// until the case releases it. See
    /// [`LlmRunnerSubject::await_tool_call`] and
    /// [`LlmRunnerSubject::release_tool_call`].
    HoldToolCalls,
    /// The runner adapter records the texts each model request carried, as
    /// the model received them, in
    /// [`ModelRequestObservation::rendered`]. See
    /// [`LlmRunnerSubject::model_requests`].
    RenderedModelInput,
    /// The harness can kill the runner at this point of
    /// `cortexkit_role_llm_runner::points`. Never listed by
    /// [`LlmRunnerSubject::capabilities`]: the conformance driver derives it from
    /// [`Harness::declared_points`].
    KillAt(&'static str),
}

impl Capability {
    #[allow(non_upper_case_globals)]
    #[deprecated(since = "0.8.11", note = "renamed to Notices")]
    pub const ResultRides: Self = Self::Notices;

    /// The role groups represented by this enum, in the order of
    /// `cortexkit_role_llm_runner::capabilities::GROUPS`.
    pub const GROUPS: &'static [Capability] = &[
        Self::CachePolicy,
        Self::SessionClass,
        Self::PlanView,
        Self::ServiceTier,
        Self::HeadSelection,
        Self::ArchivePrunesReasoning,
        Self::SystemTextDefault,
        Self::Attachments,
        Self::MediaOnlySend,
        Self::TranscriptReads,
        Self::HostHistory,
        Self::RunOps,
        Self::RunStatus,
        Self::RunCancel,
        Self::Retract,
        Self::DispatchAttribution,
        Self::Streaming,
        Self::Steer,
        Self::Queue,
        Self::Plans,
        Self::Interrupt,
        Self::SessionChange,
        Self::PlanNotes,
        Self::MediaRefs,
        Self::HistoryIntake,
        Self::Retention,
        Self::SessionRetention,
        Self::InputTokenCap,
        Self::MaxSteps,
        Self::MaxOutputTokens,
        Self::Temperature,
        Self::NoTools,
        Self::TurnDirectives,
        Self::InterruptReason,
        Self::StructuredOutput,
        Self::Adopt,
        Self::AdoptOpen,
        Self::RunToolCalls,
        Self::RunUsage,
        Self::FinishReason,
        Self::SessionUsage,
        Self::DeliveryOrder,
        Self::SenderSuccession,
        Self::SenderAttribution,
        Self::Notices,
        Self::MessageMarks,
        Self::NoticeWithSpeaksFor,
        Self::NoticeKeep,
        Self::TagRead,
        Self::ReadDisplayFacts,
        Self::SendIds,
        Self::SendStatus,
        Self::UserCommands,
        Self::HistoryDisplay,
        Self::LiveEvents,
        Self::LiveIds,
        Self::ClientMessageId,
        Self::RunCause,
        Self::StepEvents,
        Self::RoutedAccounts,
        Self::AccountRoute,
        Self::AdmissionReports,
        Self::ForkAnswer,
    ];

    /// The name the report prints. A group prints as the role spells it.
    pub fn name(self) -> String {
        match self.group() {
            Some(group) => group.to_owned(),
            None => match self {
                Self::HoldToolCalls => "hold_tool_calls".to_owned(),
                Self::HoldContextCeiling => "hold_context_ceiling".to_owned(),
                Self::HostMetadataChange => "host_metadata_change".to_owned(),
                Self::RetentionClock => "advance_retention_clock".to_owned(),
                Self::RenderedModelInput => "rendered_model_input".to_owned(),
                Self::KillAt(point) => format!("kill_at:{point}"),
                _ => unreachable!("every other capability is a group"),
            },
        }
    }

    /// The role's name for a capability group, or `None` for a harness
    /// capability.
    pub fn group(self) -> Option<&'static str> {
        Some(match self {
            Self::CachePolicy => groups::CACHE_POLICY,
            Self::SessionClass => groups::SESSION_CLASS,
            Self::PlanView => groups::PLAN_VIEW,
            Self::ServiceTier => groups::SERVICE_TIER,
            Self::HeadSelection => groups::HEAD_SELECTION,
            Self::ArchivePrunesReasoning => groups::ARCHIVE_PRUNES_REASONING,
            Self::SystemTextDefault => groups::SYSTEM_TEXT_DEFAULT,
            Self::Attachments => groups::ATTACHMENTS,
            Self::MediaOnlySend => groups::MEDIA_ONLY_SEND,
            Self::UserCommands => groups::USER_COMMANDS,
            Self::TranscriptReads => groups::TRANSCRIPT_READS,
            Self::HostHistory => groups::HOST_HISTORY,
            Self::RunOps => groups::RUN_OPS,
            Self::RunStatus => groups::RUN_STATUS,
            Self::RunCancel => groups::RUN_CANCEL,
            Self::Retract => groups::RETRACT,
            Self::DispatchAttribution => groups::DISPATCH_ATTRIBUTION,
            Self::Streaming => groups::STREAMING,
            Self::Steer => groups::STEER,
            Self::Queue => groups::QUEUE,
            Self::Plans => groups::PLANS,
            Self::Interrupt => groups::INTERRUPT,
            Self::SessionChange => groups::SESSION_CHANGE,
            Self::PlanNotes => groups::PLAN_NOTES,
            Self::MediaRefs => groups::MEDIA_REFS,
            Self::HistoryIntake => groups::HISTORY_INTAKE,
            Self::Retention => groups::RETENTION,
            Self::SessionRetention => groups::SESSION_RETENTION,
            Self::InputTokenCap => groups::INPUT_TOKEN_CAP,
            Self::MaxSteps => groups::MAX_STEPS,
            Self::MaxOutputTokens => groups::MAX_OUTPUT_TOKENS,
            Self::Temperature => groups::TEMPERATURE,
            Self::NoTools => groups::NO_TOOLS,
            Self::TurnDirectives => groups::TURN_DIRECTIVES,
            Self::InterruptReason => groups::INTERRUPT_REASON,
            Self::StructuredOutput => groups::STRUCTURED_OUTPUT,
            Self::Adopt => groups::ADOPT,
            Self::AdoptOpen => groups::ADOPT_OPEN,
            Self::RunToolCalls => groups::RUN_TOOL_CALLS,
            Self::RunUsage => groups::RUN_USAGE,
            Self::FinishReason => groups::FINISH_REASON,
            Self::SessionUsage => groups::SESSION_USAGE,
            Self::DeliveryOrder => groups::DELIVERY_ORDER,
            Self::SenderSuccession => groups::SENDER_SUCCESSION,
            Self::SenderAttribution => groups::SENDER_ATTRIBUTION,
            Self::Notices => groups::NOTICES,
            Self::MessageMarks => groups::MESSAGE_MARKS,
            Self::NoticeWithSpeaksFor => groups::NOTICE_WITH_SPEAKS_FOR,
            Self::NoticeKeep => groups::NOTICE_KEEP,
            Self::TagRead => groups::TAG_READ,
            Self::ReadDisplayFacts => groups::READ_DISPLAY_FACTS,
            Self::SendIds => groups::SEND_IDS,
            Self::SendStatus => groups::SEND_STATUS,
            Self::HistoryDisplay => groups::HISTORY_DISPLAY,
            Self::LiveEvents => groups::LIVE_EVENTS,
            Self::LiveIds => groups::LIVE_IDS,
            Self::ClientMessageId => groups::CLIENT_MESSAGE_ID,
            Self::RunCause => groups::RUN_CAUSE,
            Self::StepEvents => groups::STEP_EVENTS,
            Self::RoutedAccounts => groups::ROUTED_ACCOUNTS,
            Self::AccountRoute => groups::ACCOUNT_ROUTE,
            Self::AdmissionReports => groups::ADMISSION_REPORTS,
            Self::ForkAnswer => groups::FORK_ANSWER,
            Self::HoldToolCalls
            | Self::HoldContextCeiling
            | Self::HostMetadataChange
            | Self::RetentionClock
            | Self::RenderedModelInput
            | Self::KillAt(_) => return None,
        })
    }

    /// The group capability the role spells `name`, if any.
    pub fn from_group(name: &str) -> Option<Self> {
        Self::GROUPS
            .iter()
            .copied()
            .find(|capability| capability.group() == Some(name))
    }
}

/// A scripted model, written without any provider's wire protocol: a list
/// of assistant turns, each the whole answer to one model request, in the
/// order the model gives them. The runner adapter must translate the script
/// to its runner's model protocol without changing the scripted content.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Script {
    pub turns: Vec<ScriptedTurn>,
}

/// One assistant turn: its parts, in the order the model emits them.
#[derive(Clone, Debug, Default, PartialEq)]
#[non_exhaustive]
pub struct ScriptedTurn {
    pub parts: Vec<ScriptedPart>,
    /// Raw reason the scripted model or host reports, in the source named by
    /// `model_finish_reason_source`. `None` means no reason, not a natural stop.
    /// For output-cap fixtures, a response actually truncated by the received
    /// cap reports that source's length reason instead of this value.
    pub finish_reason: Option<String>,
    /// Report exactly these counts, or omit input usage entirely for `None`.
    /// Adapters must not fill in missing usage with an estimate or zero.
    /// For output-cap cases, the scripted provider honours the maximum it
    /// actually receives, truncating the response and reporting its actual
    /// output usage rather than the full, untruncated scripted count.
    pub usage: Option<ScriptedUsage>,
    /// Fail the model request instead of returning parts. The adapter drives
    /// its real provider-failure path, preserving classification and timing;
    /// it must not synthesize management replies. Used only when
    /// [`LlmRunnerSubject::supports_model_failure_fixture`] returns true.
    pub model_failure: Option<cortexkit_role_llm_runner::run::RunError>,
}

/// Usage reported by one completed model request, at face value.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
#[non_exhaustive]
pub struct ScriptedUsage {
    pub input_tokens: u64,
    pub cache_read_tokens: u64,
    pub cache_write_tokens: u64,
    pub output_tokens: u64,
    /// Output the provider reports as reasoning, already part of
    /// `output_tokens`; a step that reports none leaves it `None`.
    pub reasoning_tokens: Option<u64>,
}

/// Decoded facts from a native tool-result body, not a shared wire shape.
#[derive(Clone, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub struct ToolResultObservation {
    pub executed: bool,
    pub code: String,
}

/// An oversized result decoded from the runner's stored native error result,
/// never inferred from the script that supplied the original output.
#[derive(Clone, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub struct ToolResultLimitObservation {
    pub reason: String,
    pub bytes: u64,
    pub limit: u64,
    pub text: String,
    pub is_error: bool,
}

impl ToolResultLimitObservation {
    pub fn new(
        reason: impl Into<String>,
        bytes: u64,
        limit: u64,
        text: impl Into<String>,
        is_error: bool,
    ) -> Self {
        Self {
            reason: reason.into(),
            bytes,
            limit,
            text: text.into(),
            is_error,
        }
    }
}

impl ToolResultObservation {
    pub fn new(executed: bool, code: impl Into<String>) -> Self {
        Self {
            executed,
            code: code.into(),
        }
    }
}

/// What one model request carried, as the scripted model received it.
/// Recorded by the runner adapter outside the runner, never reconstructed
/// from configuration or replies.
#[derive(Clone, Debug, Default, PartialEq)]
#[non_exhaustive]
pub struct ModelRequestObservation {
    /// Applied abstract level decoded from the request actually received by
    /// the model, not inferred from the send or runner's usage.
    pub service_tier_applied: Option<String>,
    /// The output-token maximum actually sent to the model, if any.
    pub max_output_tokens: Option<u64>,
    /// The sampling temperature actually sent to the model, if any.
    pub temperature: Option<f64>,
    /// The names of the tools the request offered, in request order.
    pub tools: Vec<String>,
    /// The schema the request asked the provider to shape the final text
    /// with, if any.
    pub final_text_schema: Option<Value>,
    /// The input schema of the `submit_result` tool, if the request offered
    /// it.
    pub submit_result_schema: Option<Value>,
    /// Every text the request carried (each text part, each tool result's
    /// text, each label), in request order, exactly as the model received
    /// it, a rendered tag prefix included. Recorded when the adapter
    /// declares [`Capability::RenderedModelInput`]; empty otherwise.
    pub rendered: Vec<String>,
    /// System/developer-authority texts actually received by the scripted
    /// provider. Required for `Notices`, not inferred from delivery modes.
    /// `None` means not observed, distinct from an observed empty list.
    pub system_text: Option<Vec<String>>,
    /// System texts bearing the provider's next-user-message clearing marker,
    /// observed at model ingress, not inferred from send modes. Required when
    /// `turn_system_cleared_supported` opts into the cleared-ride cases.
    pub cleared_system_text: Option<Vec<String>>,
}

/// A raw request captured by the scripted model, outside the runner's state.
/// `fork_id: None` identifies the head; a fork request names its issued id.
/// Preserve the JSON bytes as received, including member order and whitespace;
/// parsing and reserializing would hide frozen-prefix changes.
#[derive(Clone, Debug, PartialEq)]
#[non_exhaustive]
pub struct RawModelRequestObservation {
    pub fork_id: Option<String>,
    pub body: Vec<u8>,
}

impl RawModelRequestObservation {
    pub fn new(fork_id: Option<String>, body: Vec<u8>) -> Self {
        Self { fork_id, body }
    }
}

impl ModelRequestObservation {
    pub fn with_temperature(mut self, temperature: f64) -> Self {
        self.temperature = Some(temperature);
        self
    }

    pub fn with_max_output_tokens(mut self, limit: u64) -> Self {
        self.max_output_tokens = Some(limit);
        self
    }

    pub fn new(tools: Vec<String>) -> Self {
        Self {
            tools,
            ..Self::default()
        }
    }

    pub fn with_final_text_schema(mut self, schema: Value) -> Self {
        self.final_text_schema = Some(schema);
        self
    }

    pub fn with_submit_result_schema(mut self, schema: Value) -> Self {
        self.submit_result_schema = Some(schema);
        self
    }

    pub fn with_rendered(mut self, rendered: Vec<String>) -> Self {
        self.rendered = rendered;
        self
    }
}

impl ScriptedUsage {
    pub fn new(
        input_tokens: u64,
        cache_read_tokens: u64,
        cache_write_tokens: u64,
        output_tokens: u64,
    ) -> Self {
        Self {
            input_tokens,
            cache_read_tokens,
            cache_write_tokens,
            output_tokens,
            reasoning_tokens: None,
        }
    }

    /// Report some of this step's output as reasoning. It is a breakdown of
    /// `output_tokens`, not a count added to it, so it must not exceed them.
    pub fn with_reasoning_tokens(mut self, tokens: u64) -> Self {
        self.reasoning_tokens = Some(tokens);
        self
    }
}

/// One part of an assistant turn.
#[derive(Clone, Debug, PartialEq)]
pub enum ScriptedPart {
    /// A text part, emitted exactly as written.
    Text(String),
    /// A reasoning part. Never part of a run's final text.
    Reasoning(String),
    /// A tool call, with the result the scripted tool provider answers it
    /// with.
    ToolCall(ScriptedToolCall),
}

/// A scripted tool call and its scripted result.
#[derive(Clone, Debug, PartialEq)]
pub struct ScriptedToolCall {
    /// The model's id for the call. Display only: cases reuse one id
    /// across turns on purpose.
    pub tool_call_id: String,
    /// [`crate::SCRIPTED_TOOL`], served by the scripted tool provider, or
    /// `submit_result` (`cortexkit_role_llm_runner::structured_output::SUBMIT_RESULT_TOOL`)
    /// in a structured-output case, which the runner handles itself: its
    /// `arguments` are the submitted value and its `result` is unused.
    pub tool: String,
    /// The call's arguments. Unique within a conformance run, so the scripted
    /// tool provider can find the call's result by them.
    pub arguments: Value,
    /// What the scripted tool provider answers the call with.
    pub result: Value,
    /// Hold the call in the tool provider until the case releases it.
    /// Only set when the runner adapter declares [`Capability::HoldToolCalls`].
    pub hold: bool,
}

/// An entry the scripted provider publishes after its held first reply.
/// Kinds and reasons remain open strings, like the tool-provider wire contract.
#[derive(Clone, Debug, PartialEq)]
#[non_exhaustive]
pub struct ScriptedLateEntry {
    pub kind: String,
    pub result: Option<Value>,
    pub reason: Option<String>,
}

impl ScriptedLateEntry {
    pub fn result(result: Value) -> Self {
        Self {
            kind: "result".into(),
            result: Some(result),
            reason: None,
        }
    }
    pub fn not_started(reason: impl Into<String>) -> Self {
        Self {
            kind: "not_started".into(),
            result: None,
            reason: Some(reason.into()),
        }
    }
    pub fn expired() -> Self {
        Self {
            kind: "expired".into(),
            result: None,
            reason: None,
        }
    }
    pub fn kind(&self) -> &str {
        &self.kind
    }
    pub fn result_body(&self) -> Option<&Value> {
        self.result.as_ref()
    }
    pub fn reason(&self) -> Option<&str> {
        self.reason.as_deref()
    }
}

impl Script {
    /// Every tool call in the script, in order.
    pub fn tool_calls(&self) -> impl Iterator<Item = &ScriptedToolCall> {
        self.turns
            .iter()
            .flat_map(|turn| turn.parts.iter())
            .filter_map(|part| match part {
                ScriptedPart::ToolCall(call) => Some(call),
                _ => None,
            })
    }
}

impl ScriptedTurn {
    pub fn new(parts: Vec<ScriptedPart>) -> Self {
        Self {
            parts,
            finish_reason: None,
            usage: None,
            model_failure: None,
        }
    }

    pub fn with_finish_reason(mut self, reason: impl Into<String>) -> Self {
        self.finish_reason = Some(reason.into());
        self
    }

    pub fn with_model_failure(mut self, error: cortexkit_role_llm_runner::run::RunError) -> Self {
        self.model_failure = Some(error);
        self
    }

    pub fn with_usage(mut self, usage: ScriptedUsage) -> Self {
        self.usage = Some(usage);
        self
    }

    /// The turn's text parts, in order.
    pub fn text_parts(&self) -> impl Iterator<Item = &str> {
        self.parts.iter().filter_map(|part| match part {
            ScriptedPart::Text(text) => Some(text.as_str()),
            _ => None,
        })
    }
}

/// A module the suite names in a session plan's `senders`, and the route
/// identity it reaches the session with.
#[derive(Clone, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub struct PlanSenderIdentity {
    /// The stamp a route opened by that module carries.
    pub stamp: RouteStamp,
    /// The module id the plan names, which the runner matches against the
    /// stamp's attested principal.
    pub module: String,
}

impl PlanSenderIdentity {
    pub fn new(stamp: RouteStamp, module: impl Into<String>) -> Self {
        Self {
            stamp,
            module: module.into(),
        }
    }
}

/// Which plan [`LlmRunnerSubject::refresh_plan`] builds for a
/// `session.refresh`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub enum RefreshVariant {
    /// A plan the runner accepts for the session whose model-facing tool
    /// set differs from the session's first plan's, every tool of it inside
    /// any tool set the host fixed at start.
    ChangedTools,
    /// A plan naming a tool outside the tool set the host fixed at start.
    OutsideHostSet,
    /// A plan keeping a tool's name but changing its description or input
    /// schema from the definition the host fixed at start.
    ChangedDefinition,
}

/// A plan for `session.refresh` and what the cases expect of it.
#[derive(Clone, Debug, PartialEq)]
#[non_exhaustive]
pub struct RefreshPlan {
    /// The full fetch plan, as the request's `plan` carries it.
    pub plan: Map<String, Value>,
    /// For [`RefreshVariant::ChangedTools`], the model-facing tool names a
    /// model request offers once the plan applies. For the other variants,
    /// the names a `tool_outside_host_set` refusal must list.
    pub tools: Vec<String>,
}

/// A pending-plan scenario. The adapter configures its real plan provider
/// with this plan and keeps the model/route/cache/view stable until a flush.
/// The initial plan offers `removed_tool`; this plan removes it and changes
/// system text. Observations still come from the model and tool endpoints.
#[derive(Clone, Debug)]
#[non_exhaustive]
pub struct PlanNotesFixture {
    pub plan: Map<String, Value>,
    pub system_note: String,
    pub new_system_text: String,
    pub removed_tool: String,
}

impl PlanNotesFixture {
    pub fn new(
        plan: Map<String, Value>,
        system_note: impl Into<String>,
        new_system_text: impl Into<String>,
        removed_tool: impl Into<String>,
    ) -> Self {
        Self {
            plan,
            system_note: system_note.into(),
            new_system_text: new_system_text.into(),
            removed_tool: removed_tool.into(),
        }
    }
}

impl RefreshPlan {
    pub fn new(plan: Map<String, Value>, tools: Vec<String>) -> Self {
        Self { plan, tools }
    }
}

/// The native-record fixture for the unknown-origin attribution rule.
/// Its ordinal and marker come from fixture construction, never from a read reply.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum AttributionFixture {
    /// A durably installed message whose run cannot be determined.
    Installed { ordinal: u64, marker: String },
    /// No fixture hook: the named conformance case must fail.
    NotSupported,
    /// The adapter declares that every native write path records a run.
    /// Only the unknown-origin rule is inapplicable, not the route-driven rules.
    CannotHoldUnattributable,
}

/// The adapter for a live runner under test: its harness and runner-specific
/// configuration. Every case request must go over the runner's real management
/// route; the adapter must not synthesize management replies for the runner.
///
/// Cases open routes with [`LlmRunnerSubject::session_route`], because every
/// role op rides a session-bound route; they do not call [`Harness::route`].
/// Compaction terms are defined by the separate `cortexkit-role-compaction`
/// contract; this kit does not check that protocol.
#[async_trait]
pub trait LlmRunnerSubject: Harness {
    /// An IANA zone the runner knows, used for cache-policy hours requests.
    /// Zone knowledge is not discoverable through the flag-only capability.
    fn cache_policy_zone(&self) -> &str {
        "Etc/UTC"
    }

    /// Whether this adapter drives hosts that outlive a runner process restart.
    /// False leaves the restart-survival experiments not applicable and keeps
    /// the existing crash cases unchanged. True requires `restart_survival`.
    fn host_survives_restart(&self) -> bool {
        false
    }

    /// Drive a fresh experiment on `state_root`, restarting only the runner
    /// (crash or orderly) at `point`. Hold the actual model answer, provider
    /// reply, or send confirmation until after restarting. The send point is
    /// after host handoff but before confirmation; record all later handoffs,
    /// including after a replay of its send_id. For a held keyed call, use a
    /// provider keeping its reply under `reply_custody`, and count executions
    /// independently across both processes. Finish the turn naturally with
    /// `marker` as its final assistant text; capture the active run before and
    /// during recovery, then its terminal result and native stored reply.
    ///
    /// Save a durable cursor before restart; let the host store events and
    /// lose some live text while the runner is down. Resume using that cursor,
    /// recording complete prior/resumed frames and independent host events in
    /// order. Include the run's start and end and a later run's start/end.
    /// Never reconstruct frames, replies, host records or counts from scripts.
    /// Reads and Stop must use the real management route. Capture native pid
    /// and start-time identities independently for both runner processes and
    /// the surviving host; a no-op restart or replacement host earns no credit.
    ///
    /// `UnknownHost` withholds liveness evidence, checks the head, issues owner
    /// Stop while the host is still unknown, then resolves it alive. Observe
    /// that only that session's turn is interrupted and another active session
    /// on the same host is unchanged. `Recovering` holds reattach, reads every
    /// served read op, tries a host-dependent mutation, and observes store writes.
    /// The adapter owns process/host cleanup. Missing hooks fail opted-in cases;
    /// they never count as a pass or as not applicable.
    async fn restart_survival_observation(
        &self,
        _state_root: &std::path::Path,
        _point: crate::RestartPoint,
        _mode: crate::RestartMode,
        _marker: &str,
    ) -> Result<crate::RestartSurvivalObservation, HarnessError> {
        Err(HarnessError::new(
            "restart-survival experiment hook is not implemented",
        ))
    }

    /// Script at least two real session-bound providers, including one failure
    /// when requested. Observe frozen and currently applied plans independently.
    /// Do not synthesize runner replies, turn termination, or durable storage.
    async fn install_stop_all(
        &self,
        _session: &str,
        _one_provider_fails: bool,
    ) -> Result<bool, HarnessError> {
        Ok(false)
    }
    /// Read actual stop requests and durable turn/reply commits from outside
    /// the runner, using one monotonically ordered observation log.
    async fn observe_stop_all(
        &self,
        _session: &str,
        _request_id: &str,
    ) -> Result<crate::StopAllObservation, HarnessError> {
        Err(HarnessError::new("no stop-all observation hook"))
    }
    /// Install a real waiting provider with `call-yield/v1` on its manifest
    /// and session route, `call.yield/v1` on its catalog tool, plus late-results
    /// and held-reply support.
    /// If `block_answer`, withhold the yield answer/final frame until released.
    /// Multiple calls in the same session may install independent fixtures;
    /// keep each answer blocked until that call is released.
    /// The default supplies no fixture; it does not simulate a passing runner.
    async fn install_call_yield(
        &self,
        _session: &str,
        _arguments: &Value,
        _block_answer: bool,
    ) -> Result<bool, HarnessError> {
        Ok(false)
    }

    /// Read actual incoming yield/withdraw requests and durable notice/store
    /// events from outside the runner; never infer these from configuration.
    async fn observe_call_yield(
        &self,
        _session: &str,
        _arguments: &Value,
        _send_id: &str,
    ) -> Result<crate::call_yield::CallYieldObservation, HarnessError> {
        Err(HarnessError::new("no call-yield observation hook"))
    }

    /// Snapshot actual durable admission state and the plan-fetch request log
    /// outside the runner, including baseline creation and send-id reservations.
    /// Notice-yield refusal cases compare these bytes before and after a send.
    /// Return None when the adapter cannot observe these effects independently;
    /// those cases then report inapplicable rather than assuming no write.
    async fn call_yield_admission_state(
        &self,
        _session: &str,
    ) -> Result<Option<Vec<u8>>, HarnessError> {
        Ok(None)
    }

    /// Release the provider's withheld yield answer and call final frame.
    async fn release_call_yield(&self, arguments: &Value) -> Result<(), HarnessError> {
        self.release_tool_call(arguments).await
    }
    /// Read the carrying result directly from the host's store, independently
    /// of runner read replies. Required for `Notices`. The returned output
    /// is the native tool's text payload, without wrappers or ride rendering;
    /// identify the carrying message, its stored top-level part index and key.
    async fn notices_stored_output(
        &self,
        _handle: &Self::Handle,
        _session: &str,
        _arguments: &Value,
    ) -> Result<(String, u32, String, String), HarnessError> {
        Err(HarnessError::new(
            "native notice result inspection is not implemented",
        ))
    }

    #[deprecated(since = "0.8.11", note = "renamed to notices_stored_output")]
    async fn result_rides_stored_output(
        &self,
        handle: &Self::Handle,
        session: &str,
        arguments: &Value,
    ) -> Result<(String, u32, String, String), HarnessError> {
        self.notices_stored_output(handle, session, arguments).await
    }

    /// Drive one fresh session whose host names an assistant message only
    /// when storing it. Attach with `request` before streaming any words,
    /// capture the message's frames while storage is held, then store it
    /// (`store = true`) or abort without persisting it (`false`). Capture any
    /// later display deltas too; a host that stores only at message end may
    /// have none.
    ///
    /// Return only that message's frames and its `display_reset` signals, not
    /// other messages' or runs' events. Return a loss-free observation: if the
    /// stream reports a `display_gap`, drive the scenario again rather than
    /// returning a capture with a gap. Keep each phase in wire order on the
    /// same subscription. `stored_mid` must come from native storage,
    /// not from the stream. Observe the real runner; do not synthesize frames.
    /// `None` means this host/adapter cannot drive delayed message storage,
    /// making each `live_ids` case inapplicable by name.
    async fn live_ids_observation(
        &self,
        _handle: &Self::Handle,
        _request: cortexkit_role_llm_runner::live::LiveSubscribeRequest,
        _store: bool,
    ) -> Result<Option<LiveIdsObservation>, HarnessError> {
        Ok(None)
    }

    /// The scripted provider or host's finish-reason source, using a spelling
    /// accepted by `finish::normalize`. Opt in only when the fixture emits
    /// `ScriptedTurn::finish_reason` through the real model/host path, and a
    /// response truncated at its received output cap reports a length reason.
    /// The default leaves reason-emitting cases not applicable; adapters must
    /// never synthesize management replies to supply a finish.
    fn model_finish_reason_source(&self) -> Option<&'static str> {
        None
    }

    /// A valid caller decision for the run-fixed routing-decision case.
    /// The default literal is suitable for runners that do not use routing
    /// accounts and ignore forwarding/reporting. A routing-backed adapter
    /// overrides this hook with a decision it has seeded in routing for this
    /// session's provider and model, valid when the next send starts its run.
    async fn route_decision_id_fixture(&self, _session: &str) -> Result<String, HarnessError> {
        Ok("decision-a".into())
    }

    /// The routing declaration's raw `runner` slot. Required with
    /// `routed_accounts`; it must declare `follows_account_choice: true`.
    async fn routing_runner_declaration(&self) -> Result<Value, HarnessError> {
        Err(HarnessError::new(
            "routing_runner_declaration is not implemented",
        ))
    }

    /// Public ids `{provider, account_id, credential_id}` for the listed
    /// sign-in `method`. Provision the credential in the runner's own store
    /// and arrange for the scripted model to accept it; never return a secret.
    /// Required with `routed_accounts`.
    async fn routed_account_fixture(&self, _method: &str) -> Result<Value, HarnessError> {
        Err(HarnessError::new(
            "routed_account_fixture is not implemented",
        ))
    }

    /// Direct this session's real routing calls to `peer`, preserving the
    /// authenticated calling runner's identity. Configure its plan/model for
    /// `peer.method()` and `peer.fixture()` before the first send. Forward the
    /// actual request bodies, not reconstructions. Required with
    /// `routed_accounts`; clones of the peer share the recorded calls.
    /// If [`Self::routing_peer_error_replies`] is true, a peer answer shaped as
    /// `{"error": ErrorBody}` must be emitted as an ERROR frame. All other
    /// answers remain RESPONSE bodies; do not wrap an error in a RESPONSE.
    async fn install_routing_peer(
        &self,
        _session: &str,
        _peer: crate::RecordingRoutingPeer,
    ) -> Result<(), HarnessError> {
        Err(HarnessError::new("install_routing_peer is not implemented"))
    }

    /// Opt into scripted routing ERROR replies. Older adapters keep the
    /// refusal-mapping cases inapplicable while still running account pinning.
    fn routing_peer_error_replies(&self) -> bool {
        false
    }

    /// Set the next head's routing conversation key before its first send.
    /// Return false when the adapter cannot configure this fixture; the bounded
    /// fork-key case then reports the hook inapplicable, never passed.
    async fn configure_fork_head_key(
        &self,
        _session: &str,
        _key: &str,
    ) -> Result<bool, HarnessError> {
        Ok(false)
    }

    /// Two model choices for head-selection cases: first selectable and served,
    /// second unavailable to the host. The first route supports fast but not
    /// ultrafast when `service_tier` is declared; otherwise omit its tier and
    /// choose a model distinct from the initial selection. Begin at standard.
    /// Arrange real host/model fixtures, never synthesize management answers.
    /// None reports each affected case as `Inapplicable` with its name and
    /// reason, never as passed.
    async fn head_selection_models(
        &self,
        _session: &str,
    ) -> Result<Option<[cortexkit_role_llm_runner::head_selection::PlanModel; 2]>, HarnessError>
    {
        Ok(None)
    }

    /// Arrange a current route supporting fast but not ultrafast, with served
    /// unknown, and a scripted model that does not state a served tier. The
    /// refusal scenario changes the tier on another send while the first model
    /// response is held; the active run's setting must remain fixed. Implement
    /// hold_next_model_step, await_model_step and release_model_step so
    /// that the run remains active. Do not synthesize refusals, usage or request
    /// observations. Return false when this fixture is unsupported; each affected
    /// case is then not applicable by name.
    async fn configure_service_tier(
        &self,
        _session: &str,
        _case: &str,
    ) -> Result<bool, HarnessError> {
        Ok(false)
    }
    /// Whether the scripted provider can exercise a real terminal error from
    /// [`ScriptedTurn::model_failure`], including fractional retry timing.
    /// Without this fixture, only the head-error comparison with run.status
    /// is inapplicable. Fresh and completed heads are still checked.
    fn supports_model_failure_fixture(&self) -> bool {
        false
    }

    /// Seed idle native history with independently scripted original/pruned
    /// parts. Include every declared kind in two messages and an unpruned part.
    /// `with_compaction_provider` names a provider in the plan but requires no
    /// compaction protocol: pruning must then leave all payloads untouched.
    async fn archive_pruning_fixture(
        &self,
        _session: &str,
        _with_compaction_provider: bool,
    ) -> Result<Vec<ArchivePruningPart>, HarnessError> {
        Err(HarnessError::new(
            "archive_pruning_fixture is not implemented",
        ))
    }

    /// Archive and attempt pruning now, using the runner's real maintenance
    /// path. Return after the attempt finishes, including when busy or denied
    /// by the provider-plan gate. Do not force past the run, queue or enrollment exclusions.
    async fn prune_archived_session(&self, _session: &str) -> Result<(), HarnessError> {
        Err(HarnessError::new(
            "prune_archived_session is not implemented",
        ))
    }

    /// Return actual publication subjects and bodies for the fixture session and each
    /// step's position read independently from the runner's native durable log.
    /// Capture publication order against the actual durability point at publish
    /// time. Wait until the fixture's publication work has settled; do not
    /// reconstruct events from records, inputs or session.read ordinals.
    /// Required with `step_events`: the default fails its cases by name.
    async fn step_event_observation(
        &self,
        _handle: &Self::Handle,
        _session: &str,
    ) -> Result<crate::step_events::StepEventObservation, HarnessError> {
        Err(HarnessError::new(
            "native step-event observations are not implemented",
        ))
    }

    /// The `schema_pin` the scripted tool provider received on the actual
    /// incoming call whose arguments are `arguments`, captured by the
    /// provider when the call arrived, never read back from the runner. `None`
    /// when the call arrived without one. Required when the runner declares
    /// `call_identity`: the default fails those cases by name.
    async fn dispatched_schema_pin(
        &self,
        _arguments: &Value,
    ) -> Result<Option<String>, HarnessError> {
        Err(HarnessError::new(
            "dispatched schema_pin observation is not implemented",
        ))
    }

    /// Change the definition the scripted tool provider's catalog gives
    /// [`crate::SCRIPTED_TOOL`] for `session`, as a provider deploy would,
    /// to [`crate::call_identity::CHANGED_DEFINITION`]'s input schema and
    /// semantics, so the catalog's current pin for the tool differs from the
    /// one any earlier call was dispatched against. Change the catalog only;
    /// never touch the runner's records. Return the pin the catalog gives the
    /// tool now, or `None` when the adapter cannot change it; the case then
    /// reports that rule as not checked.
    async fn change_tool_definition(&self, _session: &str) -> Result<Option<String>, HarnessError> {
        Ok(None)
    }

    /// Seed a finished historical call whose key was minted by `call_carrier`,
    /// another runner's attested module id. Preserve `call_key` verbatim and
    /// record its carrier beside the call, without dispatching it. The session
    /// has a finished run and no active run. Serve the call on every declared
    /// read surface, including live replay. Return false when unsupported.
    async fn seed_imported_tool_call(
        &self,
        _handle: &Self::Handle,
        _session: &str,
        _call: &ScriptedToolCall,
        _call_carrier: &str,
        _call_key: &str,
    ) -> Result<bool, HarnessError> {
        Ok(false)
    }

    /// Make the runner refuse the scripted call whose arguments are
    /// `arguments` in `session` before it is dispatched, through the
    /// runner's own refusal path (a pre-tool hook's deny, or a person
    /// declining an approval), so the call gets the runner's denied result
    /// and never reaches the tool provider. Called after the session's
    /// script is installed and before its first send. Return false when the
    /// adapter cannot; the denied-call case is then inapplicable, by name.
    async fn deny_tool_call(
        &self,
        _session: &str,
        _arguments: &Value,
    ) -> Result<bool, HarnessError> {
        Ok(false)
    }

    /// First-send fields for `session` whose plan holds one system-text item
    /// per entry of `stamps`, each from its own scripted provider, whose
    /// `tool.catalog` system-text answer declares that entry's rich UI stamp
    /// as `system_text.rich_ui` (none for `None`). The runner must fetch the
    /// answers itself at admission, as it fetches any plan item; build the
    /// plan and the providers' answers only, never the runner's records.
    /// Called after the session's script is installed and before its first
    /// send. `Ok(None)` means the adapter cannot build such a plan; the
    /// `rich_ui_stamp` cases are then not applicable, by name. Wanted only
    /// when the runner declares `rich_ui_stamp`.
    async fn rich_ui_plan_fields(
        &self,
        _session: &str,
        _stamps: &[Option<cortexkit_role_llm_runner::rich_ui::RichUiStamp>],
    ) -> Result<Option<Map<String, Value>>, HarnessError> {
        Ok(None)
    }

    /// Change `session`'s plan through the runner's own replan path (a
    /// `session.refresh` the runner accepts, say), so the next run that
    /// starts freezes a plan whose single system-text answer declares
    /// `stamp` (none for `None`). Start no run and touch no record a run
    /// already wrote. Return false when the adapter cannot; the case that
    /// needs it is then not applicable, by name.
    async fn replan_rich_ui(
        &self,
        _session: &str,
        _stamp: Option<&cortexkit_role_llm_runner::rich_ui::RichUiStamp>,
    ) -> Result<bool, HarnessError> {
        Ok(false)
    }

    /// The attested runner principal's bare module id, exactly as recorded in
    /// `session_binding.runner`, never `reserved:<id>` or any other prefix.
    /// This is not the caller-chosen harness or the project folder. The suite
    /// uses it with the shared session-reference function to check bus headers.
    fn runner_id(&self) -> &str;

    /// Install the scripted tool provider with exactly `provides` in the
    /// daemon catalog's manifest capabilities. Return first-send fields whose
    /// plan includes `item` as that provider's system-text item, unchanged.
    /// Required only when the runner declares `system_text_default`.
    async fn prepare_system_text_default(
        &self,
        _session: &str,
        _provides: &[String],
        _item: &Value,
    ) -> Result<Map<String, Value>, HarnessError> {
        Err(HarnessError::new(
            "system-text default fixture is not implemented",
        ))
    }

    /// Install a system-text plan item with optional facts and a named preset.
    /// Required for `plans`. The provider journal must capture the actual
    /// `tool.catalog.facts` bytes separately from `system_text`, without
    /// reconstructing them from the plan. The default shares the system-text
    /// fixture; adapters may override it for a separate provider.
    async fn prepare_system_text_facts(
        &self,
        session: &str,
        provides: &[String],
        item: &Value,
    ) -> Result<Map<String, Value>, HarnessError> {
        self.prepare_system_text_default(session, provides, item)
            .await
    }

    /// Observe every provider fetch, the actual received system-text item
    /// bytes, and durable runner state. Never reconstruct these from inputs or
    /// admission replies. The snapshot must change for any admission write
    /// (including a baseline, send-id reservation or transcript record).
    async fn system_text_default_observation(
        &self,
        _handle: &Self::Handle,
        _session: &str,
    ) -> Result<SystemTextDefaultObservation, HarnessError> {
        Err(HarnessError::new(
            "system-text default observations are not implemented",
        ))
    }
    /// Apply a real compaction cut to the key's existing log, without replacing
    /// the log. False means the adapter cannot provide this observation.
    async fn cut_incarnation_log(&self, _session: &str) -> Result<bool, HarnessError> {
        Ok(false)
    }

    /// Delete and create the key's log again, retaining the key but not its
    /// identity. False means the adapter cannot provide this observation.
    async fn recreate_incarnation_log(&self, _session: &str) -> Result<bool, HarnessError> {
        Ok(false)
    }

    /// Two distinct, attested non-owner identities for caller-grant and
    /// sender/reader admission cases, including refreshes before the next run.
    /// The kit names A without flags and B with both flags in a real plan.
    /// None keeps existing adapters inapplicable until they supply routes.
    fn head_sender_identities(&self) -> Option<[PlanSenderIdentity; 2]> {
        None
    }

    /// Whether the adapter can run the session-usage fixtures. Defaults keep
    /// existing adapters inapplicable until they opt into these observations.
    fn session_usage_fixtures(&self) -> bool {
        false
    }

    /// Rewind visible history to before the latest run, without undoing spend.
    /// Return false when the host cannot produce a rewind.
    async fn rewind_session_usage(&self, _session: &str) -> Result<bool, HarnessError> {
        Ok(false)
    }

    /// Arrange a model change the host itself records, such as a person picking
    /// a model in the host's interface, without rewriting the frozen plan, and
    /// return the model that was recorded. The switch changes only the model.
    /// Keep the recorded default tier unless the new route cannot apply it;
    /// then set the default tier to standard as part of recording the switch.
    /// session.head shows the resulting model and tier immediately.
    /// None means the adapter cannot produce that observation.
    async fn switch_head_model(
        &self,
        _session: &str,
    ) -> Result<Option<cortexkit_role_llm_runner::read::HeadModel>, HarnessError> {
        Ok(None)
    }

    /// Wait until the request-rendering component holds an accepted send.
    /// A runner owning its loop may return at once. This observes staging,
    /// not delivery, and must not render a boundary just to satisfy the hook.
    async fn send_staged(&self, _session: &str, _send_id: &str) -> Result<(), HarnessError> {
        Ok(())
    }
    /// Hold the next send between acceptance and host staging. Return false
    /// only where those events coincide, making the delayed-staging case
    /// inapplicable. The subsequent send_staged call releases this hold and
    /// waits for staging, without delivering the send itself.
    async fn hold_send_staging(&self, _session: &str) -> Result<bool, HarnessError> {
        Ok(false)
    }

    /// Build admission fields whose scripted tool provider declares data:token
    /// and program:source:rich-ui/1 slots and opts into output_refs. Numbered
    /// reference cases require plan.compaction_item; without it they are
    /// inapplicable. The unnumbered case removes that item from these fields.
    /// Do not synthesize replies.
    fn output_refs_fields(&self, _session: &str) -> Result<Map<String, Value>, HarnessError> {
        Err(HarnessError::new(
            "output reference plan is not implemented",
        ))
    }

    /// Install a native held tool-result placeholder, available to the next
    /// model request. Return its tag as observed at the rendering seam. Later
    /// replace the native result through complete_output_ref_held; neither
    /// hook may alter the runner's saved reference records or serve replies.
    /// Return an error when the runner has no such event; the held-placeholder
    /// case is then not applicable.
    async fn install_output_ref_held(
        &self,
        _handle: &Self::Handle,
        _session: &str,
        _placeholder: &str,
    ) -> Result<String, HarnessError> {
        Err(HarnessError::new("held output fixture is not implemented"))
    }
    /// Replace the installed held placeholder through the runner's native
    /// late-result path. Never edit saved reference records or synthesize route
    /// replies.
    /// Return an error when the runner has no such event; the held-placeholder
    /// case is then not applicable.
    async fn complete_output_ref_held(
        &self,
        _handle: &Self::Handle,
        _session: &str,
        _answer: &str,
    ) -> Result<(), HarnessError> {
        Err(HarnessError::new("late output fixture is not implemented"))
    }

    /// Force a native history_version change that assigns different numbers while
    /// retaining the reference-bearing message. Never edit saved sets.
    /// Return an error when the runner has no such event; the writing-request
    /// case is then not applicable.
    async fn renumber_output_refs(
        &self,
        _handle: &Self::Handle,
        _session: &str,
    ) -> Result<(), HarnessError> {
        Err(HarnessError::new(
            "output reference renumbering is not implemented",
        ))
    }
    /// Configure the actual scripted provider to answer this call with a
    /// `held-reply/v1` first reply, not an unanswered request. Declare
    /// `late_results` and opt the tool into `tool-result/v1`; the runner must bind
    /// with both consumer capabilities. Do not synthesize runner status.
    async fn install_held_reply(
        &self,
        _session: &str,
        _arguments: &Value,
        _reason: &str,
    ) -> Result<(), HarnessError> {
        Err(HarnessError::new("held-reply provider is not implemented"))
    }

    /// Wait until the provider has sent the held first reply and return the
    /// call_key captured from the actual incoming call, never from run.status.
    async fn await_held_reply(&self, _arguments: &Value) -> Result<String, HarnessError> {
        Err(HarnessError::new(
            "held-reply observation is not implemented",
        ))
    }

    /// Publish a durable provider `late_results` entry for the observed call,
    /// preserving its carrier, custodian and call_key. The runner must pull
    /// and apply it normally; this hook never writes a runner tool result.
    async fn publish_late_entry(
        &self,
        _arguments: &Value,
        _entry: ScriptedLateEntry,
    ) -> Result<(), HarnessError> {
        Err(HarnessError::new(
            "late-entry publication is not implemented",
        ))
    }

    /// Configure the scripted model to accept only PNG/PDF media (besides text)
    /// when true, or only text when false. These are provider/catalog facts, not a
    /// synthesized `session.head` answer. Required with `attachments`.
    fn attachment_model_fields(
        &self,
        _session: &str,
        _accepts: bool,
    ) -> Option<Map<String, Value>> {
        None
    }

    /// Encode a tool result containing text `before-media`, the supplied PNG,
    /// and text `after-media`, in that order, in the host's parts format.
    fn attachment_tool_result(&self, _png: &[u8]) -> Option<Value> {
        None
    }

    /// Simulate a restart that loses open uploads, preserving committed media
    /// and session history. Reopen the owner's session route after restart.
    /// Required with `attachments`; existing adapters need no new hook unless
    /// they declare that group.
    async fn restart_attachments(
        &self,
        _handle: &Self::Handle,
        _session: &str,
    ) -> Result<Self::Route, HarnessError> {
        Err(HarnessError::new(
            "attachment restart fixture is not implemented",
        ))
    }
    /// Install a scripted provider declaring `ctx-status` and `ctx-wrapup`.
    /// The first replies done with `command-private-reply`; the second replies
    /// accepted, then emits `command-progress-1`, `command-progress-2`, and a
    /// done result `command-private-result`. Collision adds a second provider
    /// declaring `ctx-status`. Return the session reference and admission fields
    /// for those providers.
    /// Set up the host's command-reply interface before session admission;
    /// this setup is not a protocol operation. Every case calls this with the
    /// session's route already open (after the runner is spawned and
    /// [`LlmRunnerSubject::session_route`] has returned), so an adapter may
    /// compute the session reference from the runner instance that route
    /// bound.
    async fn prepare_user_commands(
        &self,
        _session: &str,
        _fixture: &UserCommandFixture,
    ) -> Result<(String, Map<String, Value>), HarnessError> {
        Err(HarnessError::new("user-command fixture is not implemented"))
    }

    /// External observations from the scripted provider, model and host menu.
    /// Capture actual model inputs and provider invocations, not runner state.
    async fn user_command_observation(
        &self,
        _session: &str,
    ) -> Result<UserCommandObservation, HarnessError> {
        Err(HarnessError::new(
            "user-command observations are not implemented",
        ))
    }
    /// The capability groups the runner declares, plus the harness
    /// capabilities ([`Capability::HoldToolCalls`] and, when supported,
    /// [`Capability::RetentionClock`]). Kill points come from
    /// [`Harness::declared_points`] instead. Cases requiring anything
    /// missing are skipped, with the reason.
    fn capabilities(&self) -> BTreeSet<Capability>;

    /// The owner identity cases use to start and manage their sessions.
    /// With `MessageMarks`, `NoticeWithSpeaksFor`, `NoticeKeep`, `TagRead` or `AdoptOpen`, this must be a daemon-attested
    /// plain-scope route whose principal owns the scope, so cases can mark
    /// messages with `speaks_for` or open a session without a message.
    fn owner_stamp(&self) -> RouteStamp;

    /// An identity that is not the owner of any session the owner stamp
    /// starts, for the owner-only cases.
    fn stranger_stamp(&self) -> RouteStamp;

    /// A non-owner carrier and the scope owner, admitted by the daemon under
    /// the same plain scope (no flow id or run id). `session_route` must open
    /// real routes with these stamps. Return None when the adapter cannot
    /// expose this fixture; its scoped-start, adopt-open and message-marks owner-only
    /// cases are then named inapplicable.
    fn scoped_session_start_stamps(&self) -> Option<(RouteStamp, RouteStamp)> {
        None
    }

    /// An admitted flow-scope opener and its scope owner, under the same
    /// daemon-attested flow scope. The opener may be the owner. Routes opened
    /// with these stamps must really carry the flow attribute, not a request
    /// field. Return None when the runner or adapter cannot open such sessions;
    /// frozen-plan sender cases then report inapplicable by name.
    fn flow_session_start_stamps(&self) -> Option<(RouteStamp, RouteStamp)> {
        None
    }

    /// Open the management route for `session` under `stamp`, as a real
    /// caller with that identity would. Cases use each session name once per
    /// conformance run; the runner adapter may map the name to a native
    /// session identity, but must preserve that mapping across restarts.
    async fn session_route(
        &self,
        handle: &Self::Handle,
        session: &str,
        stamp: &RouteStamp,
    ) -> Result<Self::Route, HarnessError>;

    /// Fields cases add to every `session.send` they make into
    /// `session`, beside the role's `prompt`, `send_id` and `delivery`.
    /// `first` is true for the session's first send, which carries the
    /// session's `plan` (this contract doesn't define the plan's shape, so
    /// the runner adapter builds it) and any runner parameters admission needs. The
    /// plan must make [`crate::SCRIPTED_TOOL`], served by the scripted tool
    /// provider, available to the model.
    fn send_fields(&self, session: &str, first: bool) -> Map<String, Value>;

    /// Prepare a hosted or pre-existing session with a runner-sourced policy,
    /// without a starter send or adopt. Return false if the runner cannot host
    /// this fixture; the case is then named inapplicable.
    async fn prepare_runner_retention(
        &self,
        _handle: &Self::Handle,
        _session: &str,
    ) -> Result<bool, HarnessError> {
        Ok(false)
    }

    /// Whether request dumps are unavailable for this runner. The dumps case
    /// always checks the accepted policy; it checks the flag only when true.
    fn request_dumps_unsupported(&self) -> bool {
        false
    }

    /// Hold accepted queued sends before any run starts. Return false when
    /// the adapter cannot expose an idle queue; that case is named inapplicable.
    /// The queue must remain durable and observable, not a fabricated reply.
    async fn hold_retention_queue(&self, _session: &str) -> Result<bool, HarnessError> {
        Ok(false)
    }

    /// Let the held queue run and drain. Required when hold_retention_queue
    /// returned true; release and expiry must never discard the queued sends.
    async fn release_retention_queue(&self, _session: &str) -> Result<(), HarnessError> {
        Err(HarnessError::new("held retention queue is not implemented"))
    }

    /// A daemon-attested starter under a different principal's parent scope.
    /// The second stamp is that parent principal under the same scope. Return
    /// None only when the adapter cannot create cross-principal scoped routes.
    fn retention_scope_stamps(&self) -> Option<(RouteStamp, RouteStamp)> {
        None
    }

    /// Like [`Self::send_fields`], but admitting exactly `tools` as the
    /// session's tool set: an empty list for a tool-free session, or a list
    /// naming `submit_result` for the reserved-name refusal. Every other name
    /// is [`crate::SCRIPTED_TOOL`]. Return `None` when the runner cannot
    /// admit that tool set. Required when the adapter declares
    /// [`Capability::StructuredOutput`]: `final_text` needs a tool-free run.
    fn send_fields_with_tools(
        &self,
        _session: &str,
        _first: bool,
        _tools: &[&str],
    ) -> Option<Map<String, Value>> {
        None
    }

    /// Make the scripted model route of `session` report what a catalog row
    /// would say about schema-constrained output: `Some(true)` supported for
    /// final text and tool input, `Some(false)` not supported, `None` not
    /// stated. Called before the session's first send. Required with
    /// [`Capability::StructuredOutput`].
    ///
    /// A subject whose runner can never enforce returns `Err` for `Some(true)`;
    /// the enforced cases are then inapplicable by name with that error.
    /// `Some(false)` and `None` remain required: the best-effort and other
    /// structured-output checks still run.
    /// Each such session first probes `Some(true)`, then sets the actual
    /// support value. A refused probe requires reason `host_unsupported`;
    /// a successful probe requires the case's exact catalog or route reason.
    async fn set_structured_output_support(
        &self,
        _session: &str,
        _supported: Option<bool>,
    ) -> Result<(), HarnessError> {
        Err(HarnessError::new(
            "structured-output route support is not implemented",
        ))
    }

    /// Every model request the scripted model received for `session`, in
    /// order, across kills and restarts. Keep the record outside the runner
    /// and its state root. Required with [`Capability::StructuredOutput`],
    /// [`Capability::NoTools`], [`Capability::MaxOutputTokens`],
    /// [`Capability::Temperature`] and
    /// [`Capability::RenderedModelInput`].
    async fn model_requests(
        &self,
        _session: &str,
    ) -> Result<Vec<ModelRequestObservation>, HarnessError> {
        Err(HarnessError::new(
            "model request observation is not implemented",
        ))
    }

    /// Every raw model-request body for a session, including its forks, in
    /// receipt order across restarts. Capture at the model boundary, never
    /// reconstruct from a plan, digest or runner reply. Required by `ForkAnswer`.
    async fn raw_model_requests(
        &self,
        _session: &str,
    ) -> Result<Vec<RawModelRequestObservation>, HarnessError> {
        Err(HarnessError::new("raw_model_requests is not implemented"))
    }

    /// Optional raw-ingress fixture for the tool-set signed floor (§11.1.1).
    /// Return `Ok(None)` if unavailable; the case is not applicable, not failed.
    /// For the unique `session` label and requested `change`, perform these steps:
    ///
    /// 1. Start an isolated live session on a route where the view library reports
    ///    `signed_parts: bound`, using a scripted model with Messages-shaped JSON
    ///    requests (see [`crate::SignedToolChangeObservation`]).
    /// 2. Emit signed reasoning with unique literal text and signature markers,
    ///    plus ordinary transcript text with a separate marker. Capture a later
    ///    request carrying all markers as `before`, outside the runner's store.
    /// 3. Through the real `session.refresh` route, apply an accepted plan at a
    ///    run boundary: `Edit` changes a tool description or schema; `Reorder`
    ///    only reverses at least two tool definitions; `DeferredAddition` only
    ///    adds new `defer_loading: true` tools referenced nowhere in the request;
    ///    `ToolNote` surfaces an already-declared deferred tool by appending one
    ///    system message with a nonempty content array, keeping `tools` and every
    ///    earlier message byte-identical;
    ///    `Resume` performs the same edit as `Edit`. Keep the earlier transcript.
    /// 4. Capture the first request carrying that tool set as `after`, before
    ///    committing its model response. Do not create a compaction view that
    ///    independently strips the fixture's signed parts.
    /// 5. For `Resume`, restart the host from its durable state and render that
    ///    same pending request without new input or a committed response. Capture
    ///    its bytes as `resumed`; do not copy `after` or reserialize either body.
    ///
    /// Return raw bodies and markers only, never a synthesized verdict or a body
    /// reconstructed from configuration. The kit compares canonical tool triples,
    /// marker retention and exact resume bytes itself. Clean up the live fixture.
    /// Edits and deferred additions must record the floor and strip earlier
    /// signed reasoning; reorders and appended tool notes must preserve it.
    async fn signed_tool_change_observation(
        &self,
        _session: &str,
        _change: crate::SignedToolChange,
    ) -> Result<Option<crate::SignedToolChangeObservation>, HarnessError> {
        Ok(None)
    }

    /// The part's actual model-visible number. The "Tag numbers" chapter (A3)
    /// of cortexkit-role-compaction's CONTRACT.md owns the calculation. Capture
    /// the request's mapping from native part identity to the number shown;
    /// do not recompute it from ordinals or keys. Template and output-reference
    /// fixtures use this mapping without the removed tags group or resolve op.
    fn shown_part_number(&self, _session: &str, _mid: &str, _part: &str) -> Option<u64> {
        None
    }

    /// Insert a person's user message into an idle session, with `marker` in a
    /// nonzero stored part and as its app client_message_id. The entry may come
    /// from a plan sender marked `person: true`, so its sender may be present.
    /// Return its actual message id and part index. The entry must have no
    /// notice or speaks_for mark.
    /// Unsupported adapters leave the person-message cases inapplicable.
    async fn seed_tag_read_person(
        &self,
        _handle: &Self::Handle,
        _session: &str,
        _marker: &str,
    ) -> Result<Option<(String, u32)>, HarnessError> {
        Ok(None)
    }

    /// Fixture-only import of a historical stored tool output for the named
    /// call. The output predates the provider-result limit or was imported;
    /// it is not newly returned by a provider. Do not dispatch any tool or
    /// bypass size enforcement on a live call. Return false when unsupported.
    async fn fixture_seed_historical_tool_output(
        &self,
        _handle: &Self::Handle,
        _session: &str,
        _tool_call_id: &str,
        _content: &str,
    ) -> Result<bool, HarnessError> {
        Ok(false)
    }

    /// The fetch plan a host applies to `session` when it starts, in the
    /// shape `session.adopt` carries it (the contract does not pin a plan's
    /// shape, so the runner adapter builds it). The plan must make
    /// [`crate::SCRIPTED_TOOL`] available to the model. With `alternate`, a
    /// different plan, with a different identity, that the runner would
    /// otherwise accept: the drift cases send it to a session started with
    /// the first one. Required with [`Capability::Adopt`].
    fn host_plan(&self, _session: &str, _alternate: bool) -> Option<Map<String, Value>> {
        None
    }

    /// The scripted model to select in `harness.host_start`. Required when
    /// the live describe advertises that extension, together with `host_plan`
    /// (both original and alternate) and a plain-scope owner from `owner_stamp`.
    fn host_start_model(
        &self,
        _session: &str,
    ) -> Option<cortexkit_role_llm_runner::host_start::HostStartModel> {
        None
    }

    /// Start `session` the way a person's own app starts one, outside the
    /// role: the runner applies `plan` at host start, and no role request has
    /// reached the session yet, so it has no recorded owner. With `prompt`,
    /// the person also sends `prompt` as the session's first turn, answered
    /// by the script installed for `session`; return once the turn has
    /// started, without waiting for it to end. Pass `None` for a session
    /// with no message yet. Required with [`Capability::Adopt`].
    async fn start_host_session(
        &self,
        _handle: &Self::Handle,
        _session: &str,
        _plan: &Map<String, Value>,
        _prompt: Option<&str>,
    ) -> Result<(), HarnessError> {
        Err(HarnessError::new(
            "host-started sessions are not implemented",
        ))
    }

    /// A plan for a `session.refresh` of `session`, a session admitted with
    /// [`Self::send_fields`]' first plan. Return `None` when the runner
    /// cannot be asked the question: no plan with another tool set it would
    /// accept, or, for [`RefreshVariant::OutsideHostSet`] and
    /// [`RefreshVariant::ChangedDefinition`], a host that does not fix its
    /// tool set at start. The case is then inapplicable, by name. Required
    /// with [`Capability::SessionChange`].
    fn refresh_plan(&self, _session: &str, _variant: RefreshVariant) -> Option<RefreshPlan> {
        None
    }

    /// A valid opening plan with a `compaction_item` this runner can serve,
    /// used only by the compaction-item plan-change cases. It must make
    /// [`crate::SCRIPTED_TOOL`] available, like [`Self::send_fields`]' plan.
    /// Returning `Some` replaces that plan for these cases without enrolling
    /// other cases in compaction. With `None`, the cases fall back to the first
    /// plan from [`Self::send_fields`] and are inapplicable if it has no item.
    fn compaction_change_plan(&self, _session: &str) -> Option<Map<String, Value>> {
        None
    }

    /// A valid replacement plan naming a different `compaction_item` from
    /// the opening plan chosen by the compaction-item plan-change cases.
    /// The switch case uses this ahead of [`Self::refresh_plan`] with
    /// [`RefreshVariant::ChangedTools`]. With neither fixture supplying a
    /// different item, that case is inapplicable. Other cases do not use it.
    fn compaction_switch_plan(&self, _session: &str) -> Option<Map<String, Value>> {
        None
    }

    /// Configure a stable-prefix plan-note scenario; absence grades these
    /// optional cases not applicable. `model_requests` must observe note text
    /// at the model endpoint, not reconstruct it from this fixture.
    fn plan_notes_fixture(&self) -> Option<PlanNotesFixture> {
        None
    }

    /// Start a held initial intake of native host history before any send.
    /// Store only the first supplied text initially; `advance_history_intake`
    /// stores the second, and `finish_history_intake` stores the rest before
    /// releasing queued runs. Return false when this optional fixture is absent.
    async fn begin_history_intake(
        &self,
        _session: &str,
        _texts: &[String],
    ) -> Result<bool, HarnessError> {
        Ok(false)
    }

    /// Advance a seeded intake by one prefix entry without completing it.
    async fn advance_history_intake(&self, _session: &str) -> Result<bool, HarnessError> {
        Ok(false)
    }

    /// Complete the seeded intake, then allow accepted sends to start runs.
    async fn finish_history_intake(&self, _session: &str) -> Result<bool, HarnessError> {
        Ok(false)
    }

    /// Build first-send fields for a plan with one tool item served by
    /// [`Self::tool_provider_module`] and offering both
    /// [`crate::SCRIPTED_TOOL`] and [`crate::upfront::OTHER_TOOL`]. Apply
    /// `upfront` to that item, or omit the member for `None`; everything else,
    /// including provider params and exclusions, stays the same. The item must
    /// not offer [`crate::upfront::MISSING_TOOL`]. Return `None` when the adapter
    /// cannot create these plan sessions. With `tool_upfront`, the cases also
    /// require [`Self::model_requests`] to observe the model's actual tools.
    fn upfront_plan_fields(
        &self,
        _session: &str,
        _upfront: Option<&[String]>,
    ) -> Option<Map<String, Value>> {
        None
    }

    /// Snapshot durable runner state directly from storage, before and after
    /// upfront admission. The bytes must change for any admission write,
    /// including a baseline, send-id reservation or transcript entry; do not
    /// derive them from the request or reply. Required with `tool_upfront`
    /// when [`Self::upfront_plan_fields`] supplies a plan-session fixture.
    async fn upfront_durable_state(
        &self,
        _handle: &Self::Handle,
        _session: &str,
    ) -> Result<Vec<u8>, HarnessError> {
        Err(HarnessError::new(
            "upfront durable state observation is not implemented",
        ))
    }

    /// Like [`Self::send_fields`], but add `members` to the first send's plan
    /// beside the adapter's own members, such as `senders` for delivery-order
    /// cases. Return `None` when the runner cannot admit such a plan. Required
    /// with [`Capability::DeliveryOrder`].
    fn send_fields_with_plan(
        &self,
        _session: &str,
        _first: bool,
        _members: &Map<String, Value>,
    ) -> Option<Map<String, Value>> {
        None
    }

    /// A module other than the owner and the stranger, which the
    /// delivery-order cases name in a plan's `senders`. Required with
    /// [`Capability::DeliveryOrder`].
    fn plan_sender(&self) -> Option<PlanSenderIdentity> {
        None
    }

    /// Change only the first byte of a stored attributed text part to `!`,
    /// preserving its recorded attribution metadata. Required with
    /// `SenderAttribution`; mutate storage independently of read replies.
    async fn tamper_attribution_prefix(
        &self,
        _session: &str,
        _mid: &str,
    ) -> Result<(), HarnessError> {
        Err(HarnessError::new(
            "sender_attribution requires tamper_attribution_prefix",
        ))
    }

    /// Whether the scripted model's route supports a turn-scoped system
    /// message, so `turn_system` can be possible. When `false`, the
    /// delivery-order template case is inapplicable.
    fn turn_system_supported(&self) -> bool {
        false
    }

    /// Opt into cleared-ride cases only with a route confirmed to clear a
    /// system message at the next user-role message. Defaults to skipped.
    fn turn_system_cleared_supported(&self) -> bool {
        false
    }

    /// Configure the scripted model's confirmed or unconfirmed clearing route.
    /// With no session, select the route for the next fixture; otherwise switch
    /// that session's model/provider route through the runner's real plan path,
    /// rebuilding its prompt before its next request. Required by the opt-in
    /// cleared-ride cases; observations must still come from provider ingress.
    async fn configure_turn_system_cleared_route(
        &self,
        _session: Option<&str>,
        _confirmed: bool,
    ) -> Result<(), HarnessError> {
        Err(HarnessError::new(
            "cleared-ride route configuration is not implemented",
        ))
    }

    /// Hold `session`'s next model request at the context ceiling, as a
    /// session whose compaction view has not arrived would be held, until
    /// [`Self::end_context_ceiling_wait`]. Called before the send that
    /// reaches it. Required by [`Capability::HoldContextCeiling`].
    async fn hold_context_ceiling(&self, _session: &str) -> Result<(), HarnessError> {
        Err(HarnessError::new(
            "context-ceiling holds are not implemented",
        ))
    }

    /// End the held wait: with `view_arrived`, a view arrives and the run
    /// goes on; without, the wait's bound passes with no view and the run
    /// ends `context_ceiling`. Required by [`Capability::HoldContextCeiling`].
    async fn end_context_ceiling_wait(
        &self,
        _session: &str,
        _view_arrived: bool,
    ) -> Result<(), HarnessError> {
        Err(HarnessError::new(
            "context-ceiling holds are not implemented",
        ))
    }

    /// Change `session`'s metadata in the host, outside the role, as a
    /// person's app might (rename it, touch a timestamp), in a way that does
    /// not reach the model. Required by [`Capability::HostMetadataChange`].
    async fn change_host_metadata(
        &self,
        _handle: &Self::Handle,
        _session: &str,
    ) -> Result<(), HarnessError> {
        Err(HarnessError::new(
            "host metadata changes are not implemented",
        ))
    }

    /// Legacy fixture hook, unused by this release's cases. Return a provider
    /// result holding only an image, with `marker` in its stored bytes so a
    /// reader can locate it, or `None` if the provider cannot produce one.
    fn image_tool_result(&self, _marker: &str) -> Option<Value> {
        None
    }

    /// The finished call's stored result-part key, exactly as the runner serves
    /// it, for the call's result_ordinal entry. No payload marker is required.
    /// The default matches common call-id fields or a single-part result entry.
    /// Override for other native schemas or opaque identity mappings; `None`
    /// names part identity as unchecked.
    fn tool_result_part(
        &self,
        call: &cortexkit_role_llm_runner::tool_calls::RunToolCall,
        result: &cortexkit_role_llm_runner::read::ReadMessage,
    ) -> Option<String> {
        let parts = self.read_parts(&result.message);
        let matching: Vec<_> = parts
            .iter()
            .filter(|(_, part)| {
                ["tool_use_id", "tool_call_id", "callID", "id"]
                    .iter()
                    .any(|field| {
                        part.get(field).and_then(Value::as_str) == Some(call.tool_call_id.as_str())
                    })
                    || call.call_key.as_deref().is_some_and(|key| {
                        part.get("call_key").and_then(Value::as_str) == Some(key)
                    })
            })
            .collect();
        match matching.as_slice() {
            [(key, _)] => Some(key.clone()),
            [] if result.ordinal != call.ordinal && parts.len() == 1 => Some(parts[0].0.clone()),
            _ => None,
        }
    }

    /// An image result in the host's actual schema, with exactly `bytes` as
    /// decoded image bytes. `nested` requests a tool_result containing the
    /// image; otherwise the result is an image block. Unsupported adapters
    /// name the media-fixture cases inapplicable.
    fn media_tool_result(&self, _bytes: &[u8], _nested: bool, _scenario: &str) -> Option<Value> {
        None
    }

    /// Exact JSON pointer, relative to `message`, of the output inside the
    /// tool-result part named by `part_key` from [`Self::read_parts`]. Return it
    /// consistently for inline, descriptors and media-refs reads. Only values
    /// beneath that pointer may omit media refs when unchanged from inline;
    /// the part's envelope and all other values keep the strict comparison.
    /// Flattening adapters must supply this hook. `None` keeps the existing
    /// strict comparison everywhere; never identify a non-tool output here.
    fn media_refs_tool_output_pointer(&self, _message: &Value, _part_key: &str) -> Option<String> {
        None
    }

    /// A tool result containing `texts` as separate text payloads, preserving
    /// their order. Retained for adapter compatibility; descriptor-budget
    /// fixtures use assistant text to avoid oversized provider results.
    fn multipart_tool_result(&self, _texts: &[String]) -> Option<Value> {
        None
    }

    /// Legacy body-shape extractor using native ids or decimal array indexes.
    /// Use this default only when those exactly match the runner's served part
    /// keys. Otherwise override it using the runner's actual identity mapping;
    /// body shape alone does not establish an opaque served key. Return parts
    /// in stored order, keyed as the runner publishes them and replace_read_part
    /// addresses them.
    fn read_parts(&self, message: &Value) -> Vec<(String, Value)> {
        if let Some(parts) = message
            .get("parts")
            .or_else(|| message.get("content"))
            .and_then(Value::as_array)
        {
            parts
                .iter()
                .enumerate()
                .map(|(i, p)| {
                    (
                        p.get("id")
                            .and_then(Value::as_str)
                            .map(str::to_owned)
                            .unwrap_or_else(|| i.to_string()),
                        p.clone(),
                    )
                })
                .collect()
        } else {
            vec![("0".into(), message.clone())]
        }
    }

    /// Replace exactly the part named by `read_parts`, without changing its
    /// neighbors or the message envelope. The pruning re-read check uses this
    /// pure schema operation to compare by identity, even for byte-identical
    /// parts. Override together with `read_parts` for other native schemas.
    fn replace_read_part(&self, message: &mut Value, key: &str, replacement: Value) -> bool {
        let field = if message.get("parts").is_some() {
            "parts"
        } else {
            "content"
        };
        if let Some(parts) = message.get_mut(field).and_then(Value::as_array_mut) {
            if let Some((_, part)) = parts.iter_mut().enumerate().find(|(index, part)| {
                part.get("id")
                    .and_then(Value::as_str)
                    .map_or_else(|| index.to_string() == key, |id| id == key)
            }) {
                *part = replacement;
                return true;
            }
            false
        } else if key == "0" {
            *message = replacement;
            true
        } else {
            false
        }
    }

    /// Legacy fixture hook, unused by this release's cases. Create a host fork
    /// or clone of `parent` under `fork`, or return an error when unsupported.
    /// Compaction-protocol fork detection is outside this kit.
    async fn fork_session(
        &self,
        _handle: &Self::Handle,
        _parent: &str,
        _fork: &str,
    ) -> Result<(), HarnessError> {
        Err(HarnessError::new("forks are not implemented"))
    }

    /// Write the host's native marker, without installing a compaction provider.
    /// Older active-path entries remain readable from the host's own store.
    async fn compact_in_host(
        &self,
        _handle: &Self::Handle,
        _session: &str,
    ) -> Result<(), HarnessError> {
        Err(HarnessError::new("host compaction is not implemented"))
    }

    /// Legacy fixture hook, unused by this release's cases. Read the model reply
    /// containing `marker` directly from the host store, outside the runner, or
    /// return `None` when no such reply exists. Return an error if unsupported.
    async fn host_stored_text(
        &self,
        _handle: &Self::Handle,
        _session: &str,
        _marker: &str,
    ) -> Result<Option<String>, HarnessError> {
        Err(HarnessError::new(
            "reading the host's store is not implemented",
        ))
    }

    /// End an invocation at the host's session cutoff, just before or just
    /// after forwarding. The adapter holds the invocation at that boundary;
    /// it must not synthesize the runner's answer or status.
    async fn cut_off_user_command(
        &self,
        _handle: &Self::Handle,
        _session: &str,
        _after_forward: bool,
    ) -> Result<(), HarnessError> {
        Err(HarnessError::new("cutting off commands is not implemented"))
    }

    /// The module id the runner records as `dispatched_to` for a call to
    /// the scripted tool provider.
    fn tool_provider_module(&self) -> String;

    /// Install `script` as the model for `session` before its first send.
    /// The runner adapter must preserve the script across kills and restarts.
    /// The scripted model answers a model request of the session with
    /// `script.turns[n]`, where `n` is the number of assistant messages in
    /// the history the request carries. Cases supply enough turns for every
    /// model request they cause.
    /// A native marker may hide earlier assistant entries from model input.
    /// In that case select turns by emitted assistant answers, preserving
    /// the script cursor across held calls and restarts. No compaction
    /// provider fixture is installed by this kit.
    async fn install_script(&self, session: &str, script: Script) -> Result<(), HarnessError>;

    /// Hold the next actual model answer for this session until explicitly
    /// released. Called before the send. Required when describe lists
    /// `run_cancel.at: step_boundary`; do not pause the runner before it sends
    /// the model request, since cancellation must arrive during a step. Also
    /// required by the session_retention active-run and pending-release cases.
    async fn hold_next_model_step(&self, _session: &str) -> Result<(), HarnessError> {
        Err(HarnessError::new("held model step is not implemented"))
    }

    /// Resolve after the scripted model receives the held request, not merely
    /// after installing a script or accepting a send.
    async fn await_model_step(&self, _session: &str) -> Result<(), HarnessError> {
        Err(HarnessError::new(
            "held model observation is not implemented",
        ))
    }

    /// Release the held model answer to the runner, even if it has cancelled.
    async fn release_model_step(&self, _session: &str) -> Result<(), HarnessError> {
        Err(HarnessError::new("held model release is not implemented"))
    }

    /// Count requests actually received by the scripted model for this session.
    /// Keep the count outside the runner and its state root; do not infer it
    /// from transcript messages or configured answers, which can hide an extra
    /// model step whose answer was discarded after cancellation. Required by
    /// session_retention queue and pending-release cases to prove queued sends ran.
    async fn model_invocations(&self, _session: &str) -> Result<usize, HarnessError> {
        Err(HarnessError::new(
            "model invocation observation is not implemented",
        ))
    }

    /// Install one native persistent message with no recoverable run or episode
    /// in the existing session. Return its ordinal and a unique marker stored
    /// in its body, derived from fixture construction rather than `session.read`.
    /// Do not synthesize management replies or invent a run for this record.
    /// Return `CannotHoldUnattributable` only if every native write path names
    /// a run, so the runner truly cannot store such a message. The default
    /// `NotSupported` fails the case; it is not a partial conformance pass.
    async fn install_attribution_fixture(
        &self,
        _handle: &Self::Handle,
        _session: &str,
    ) -> Result<AttributionFixture, HarnessError> {
        Ok(AttributionFixture::NotSupported)
    }

    /// Seed `messages` finished messages after the session's existing run,
    /// through the runner's own store, then a native marker leaving only `window`
    /// entries in model input. All active-path entries remain readable. The
    /// runner treats these messages as ones it wrote itself. Return an error
    /// when unsupported; the cost case is then not applicable, never passed.
    async fn seed_session_history(
        &self,
        _handle: &Self::Handle,
        _session: &str,
        _messages: u64,
        _window: u64,
    ) -> Result<(), HarnessError> {
        Err(HarnessError::new(
            "seeding session history is not implemented",
        ))
    }

    /// Seed `count` native entries before the first runner send. They join the
    /// same one-based active-path count as later sends. With `cut`, write an
    /// optional native marker that limits model input, not reads. Return an
    /// error when unsupported; cases needing the fixture are not applicable by name.
    async fn seed_host_history(
        &self,
        _handle: &Self::Handle,
        _session: &str,
        _count: u64,
        _cut: bool,
    ) -> Result<(), HarnessError> {
        Err(HarnessError::new("seeding host history is not implemented"))
    }

    /// Put into `session`'s stored history, after its newest message, a
    /// finished call of [`crate::SCRIPTED_TOOL`] with `call`'s id and
    /// arguments and a short text result, the way history stored before the
    /// runner kept tool-call times holds it: nothing records when the call
    /// started or finished. `session` has a finished run behind it and no
    /// run active. The runner must serve the call as one it replays from
    /// that history. Return an error when the adapter cannot: the case that
    /// needs it is then inapplicable, by name.
    async fn seed_untimed_tool_call(
        &self,
        _handle: &Self::Handle,
        _session: &str,
        _call: &ScriptedToolCall,
    ) -> Result<(), HarnessError> {
        Err(HarnessError::new(
            "seeding a tool call without times is not implemented",
        ))
    }

    /// Revert `session` the way its host reverts (an undo), removing the
    /// message `mid` and every later message, while no run is active. The
    /// runner moves the session to a new history_version. Return an error when the
    /// adapter cannot revert: the cases that need one are then inapplicable,
    /// by name.
    async fn revert_session(
        &self,
        _handle: &Self::Handle,
        _session: &str,
        _mid: &str,
    ) -> Result<(), HarnessError> {
        Err(HarnessError::new("reverting a session is not implemented"))
    }

    /// Current Unix epoch milliseconds of the clock also governing delivery
    /// expiry. Paired with `advance_retention_clock` for deterministic retry
    /// deadline checks; `None` makes those checks inapplicable.
    fn delivery_clock_now_ms(&self) -> Option<u64> {
        None
    }

    /// Whether scripted assistant messages record host finish stamps that can
    /// be exposed as `completed_at`. False makes the completion display-fact
    /// case inapplicable by name, without skipping the other display facts.
    fn reports_read_completed_at(&self) -> bool {
        false
    }

    /// Store exactly `created_at_ms` (Unix epoch milliseconds) as the host's
    /// recorded creation time for the entry `mid` in `session`, independently of
    /// the read route and without changing its write or completion times. Return
    /// `Ok(true)` once seeded, or `Ok(false)` if the adapter can't seed that host
    /// fact; the creation-time case is then inapplicable, never a pass.
    async fn seed_host_created_at(
        &self,
        _session: &str,
        _mid: &str,
        _created_at_ms: u64,
    ) -> Result<bool, HarnessError> {
        Ok(false)
    }

    /// Seed per-call display metadata independently of the read route. The
    /// calls already exist in `session`; use the supplied order and preserve
    /// absent times/outcomes. Return false when the adapter cannot arrange
    /// stored metadata, so display-fact checks are inapplicable, not passes.
    async fn seed_read_tool_times(
        &self,
        _session: &str,
        _times: &[cortexkit_role_llm_runner::read::ToolTime],
    ) -> Result<bool, HarnessError> {
        Ok(false)
    }

    /// Advance the session's retention clock by `milliseconds`, completing
    /// expiry and any due deletion work before returning, including letting
    /// declared log readers durably read through the due content. Durable activity
    /// timestamps, expiry checks, and deletion deadlines must all use this
    /// same clock. Declare [`Capability::RetentionClock`] only when the
    /// adapter can provide this behavior; otherwise short waits use real time.
    async fn advance_retention_clock(
        &self,
        _session: &str,
        _milliseconds: u64,
    ) -> Result<(), HarnessError> {
        Err(HarnessError::new(
            "controlled retention clock is not implemented",
        ))
    }

    /// Advance the fork idle clock and finish every expiry due in the span
    /// before returning. Held model calls remain in flight during the advance.
    async fn advance_fork_clock(
        &self,
        _session: &str,
        _milliseconds: u64,
    ) -> Result<(), HarnessError> {
        Err(HarnessError::new("advance_fork_clock is not implemented"))
    }

    /// How many times the scripted tool provider was called with
    /// `arguments`, counted across every kill and restart in the run.
    ///
    /// The count must be kept outside the runner's process and outside its
    /// state root: in the adapter's provider process, or on disk somewhere a kill
    /// and restart of the runner cannot touch. A count the runner holds is
    /// reset by every kill, and the at-most-once crash checks would then pass
    /// whatever the runner does.
    async fn tool_invocations(&self, arguments: &Value) -> usize;

    /// Decode a not-executed result for this model call id from a real
    /// `session.read` message body. Override only to locate the native call id
    /// and result content; never infer results from scripts or terminal state.
    /// There is no shared body layout; the default recognizes no results and
    /// therefore cannot pass the input-cap tool-result case.
    fn not_executed_tool_result(
        &self,
        _message: &Value,
        _tool_call_id: &str,
    ) -> Option<ToolResultObservation> {
        None
    }

    /// Locate this call's actual stored result, including the model-visible
    /// error text and measured size fields, in a native read message.
    fn oversized_tool_result(
        &self,
        _message: &Value,
        _tool_call_id: &str,
    ) -> Option<ToolResultLimitObservation> {
        None
    }

    /// Resolve once the scripted tool provider holds the call with
    /// `arguments`. Required by [`Capability::HoldToolCalls`].
    async fn await_tool_call(&self, arguments: &Value) -> Result<(), HarnessError>;

    /// Let the held call with `arguments` answer its scripted result.
    /// Required by [`Capability::HoldToolCalls`].
    async fn release_tool_call(&self, arguments: &Value) -> Result<(), HarnessError>;

    /// Inspect the runner's durable deletion-completion report and its
    /// runner-held transcript/derived stores for this session. Required for
    /// retention cases. Do not infer completion from an `expired` read: a
    /// tombstone hides content before deletion is finished. Return true only
    /// once deletion has actually finished, including after a restart.
    async fn retention_deletion_finished(
        &self,
        _handle: &Self::Handle,
        _session: &str,
    ) -> Result<bool, HarnessError> {
        Err(HarnessError::new(
            "retention deletion inspection is not implemented",
        ))
    }

    /// Classify every advertised non-role op for the retention leak check.
    /// Return `Some` with all request variants needed to export/list/read
    /// content for `session` (including all listing pages), or `None` only
    /// for ops that cannot serve stored session content. Export/list schemas
    /// are runner-specific, so cases cannot construct these requests.
    /// An unclassified advertised op fails the check, never silently skips it.
    fn retention_probe_params(
        &self,
        _session: &str,
        method: &str,
    ) -> Result<Option<Vec<Value>>, HarnessError> {
        Err(HarnessError::new(format!(
            "unclassified retention probe op: {method}"
        )))
    }

    /// The serializer profile (`cortexkit_role_llm_runner::serializer_profile`)
    /// of `session`'s stored bodies, as `session.history` serves them raw, or
    /// `None` when they follow no profile this role defines. The display
    /// case comparing a served row with the profile's projection of the same
    /// entry is then inapplicable, by name.
    fn serializer_profile(&self, _session: &str) -> Option<String> {
        None
    }

    /// Make the scripted model stream the first `parts` parts of its next
    /// answer for `session`, each whole, and then hold the rest of the
    /// answer until [`Self::release_model_step`]; [`Self::await_model_step`]
    /// resolves once those parts are streamed. Called before the send. The
    /// live-event cases that attach while a step is in flight are
    /// inapplicable, by name, when this answers an error.
    async fn hold_next_model_stream(
        &self,
        _session: &str,
        _parts: usize,
    ) -> Result<(), HarnessError> {
        Err(HarnessError::new(
            "holding a model answer mid-stream is not implemented",
        ))
    }

    /// Change the real model selection so the session's next and subsequent
    /// requests go to `model` (1 to 256 printable ASCII bytes, without spaces),
    /// as the runner's own model switch would. Before the first send this sets
    /// the starting model. Observe the subsequent requests rather than
    /// synthesizing management replies.
    async fn switch_model(&self, _session: &str, _model: &str) -> Result<(), HarnessError> {
        Err(HarnessError::new("switching models is not implemented"))
    }

    /// Make the final options of `session`'s next model requests carry the
    /// prompt-cache lifetime `milliseconds` (`None`: carry none), so the
    /// runner observes that lifetime in its own request, as it would from
    /// a provider adapter that sets one.
    async fn set_observed_cache_lifetime(
        &self,
        _session: &str,
        _milliseconds: Option<u64>,
    ) -> Result<(), HarnessError> {
        Err(HarnessError::new(
            "an observed prompt-cache lifetime is not implemented",
        ))
    }

    /// Install the supplied cache prices in the runner's scripted model fixture
    /// before the session's first send. Return the prices actually used, read
    /// back from the fixture, not an echo of the arguments. The fixture must
    /// support idle warming with an explicit lifetime supplied through
    /// `set_observed_cache_lifetime`; `cache_policy` alone does not promise that
    /// support. Return `None` when this fixture is unavailable. Scripted cache
    /// usage is returned on every request, including repeated request bytes,
    /// without consuming a real assistant turn for a cache touch.
    async fn configure_keep_warm_prices(
        &self,
        _handle: &Self::Handle,
        _session: &str,
        _prices: &crate::KeepWarmPrices,
    ) -> Result<Option<crate::KeepWarmPrices>, HarnessError> {
        Ok(None)
    }

    /// Move the clock governing keep-warm scheduling, processing due timers in
    /// chronological order and settling provider requests and their usage before
    /// returning. Advancing time must not itself send a real request. The same
    /// clock timestamps the provider journal. Return `None` when unavailable;
    /// release builds must not expose a controllable scheduler clock.
    async fn advance_warm_clock(
        &self,
        _handle: &Self::Handle,
        _by: std::time::Duration,
    ) -> Result<Option<()>, HarnessError> {
        Ok(None)
    }

    /// Requests actually received by the scripted provider for this session,
    /// starting at the zero-based `cursor`, and the current scheduler time.
    /// The cursor advances by the number of returned requests. Keep this journal
    /// outside runner state. Record exact request and cache-prefix bytes plus
    /// actual response usage; never infer requests from transcript entries or
    /// label them warm/real. Settle in-flight responses before returning. Return
    /// `None` when the independent provider journal is unavailable.
    async fn keep_warm_requests(
        &self,
        _session: &str,
        _cursor: u64,
    ) -> Result<Option<crate::KeepWarmRequests>, HarnessError> {
        Ok(None)
    }

    /// Wait a short while between polls that check whether a run has ended.
    async fn pause(&self);
}

/// A native fixture's pruning oracle, configured independently of maintenance.
/// The kit matches `original` against actual parts from `read_parts` and checks
/// the exact `served` value, including the format's required fields.
#[derive(Clone, Debug, PartialEq)]
pub struct ArchivePruningPart {
    pub kind: String,
    pub original: Value,
    pub served: Value,
}

/// External observations for the preset-less system-text admission cases.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
#[non_exhaustive]
pub struct SystemTextDefaultObservation {
    /// One entry per actual fetch: exact JSON facts bytes, or `None` when the
    /// request has no facts member. Used by the system-text facts cases.
    pub provider_facts: Vec<Option<Vec<u8>>>,
    /// All provider fetches, including requests without system text.
    pub provider_calls: usize,
    pub provider_items: Vec<Vec<u8>>,
    pub durable_state: Vec<u8>,
}

/// Configuration for scripted commands, interactivity and replies outside model history.
#[derive(Clone, Debug)]
pub struct UserCommandFixture {
    pub collision: bool,
    pub interactive: bool,
    pub host_replies_outside_history: bool,
}

#[derive(Clone, Debug, Default)]
pub struct UserCommandObservation {
    /// Each actual request received by the provider, including retries.
    pub provider_calls: Vec<Value>,
    /// Complete requests captured at the model boundary, across all turns.
    pub model_inputs: Vec<Value>,
    /// Names actually registered in the host menu, without a slash.
    pub host_commands: Vec<String>,
}
