//! What the runner adapter supplies beyond its harness: the capabilities
//! it declares, how to reach a session's management route, and a scripted
//! model, tool and compaction providers.

use std::collections::BTreeSet;

use async_trait::async_trait;
use cortexkit_role_harness::{Harness, HarnessError, RouteStamp};
use cortexkit_role_llm_runner::capabilities as groups;
use serde_json::{Map, Value};

/// A capability a conformance case requires of the runner or its harness.
/// A case whose requirement the runner adapter does not declare is reported as
/// skipped with the missing names, never as passed.
///
/// Non-exhaustive: the role adds groups over time, so a `match` outside this
/// crate keeps a wildcard arm.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
#[non_exhaustive]
pub enum Capability {
    /// Preset-less system-text admission checked against the daemon catalog.
    SystemTextDefault,
    /// Chunked uploads, send references and session-scoped media reads.
    Attachments,
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
    /// The `model_view` group: live half-open replacement/insertion paging,
    /// and the canonical view used by the compaction crash checks.
    ModelView,
    /// The `steer` delivery mode of `session.send`.
    Steer,
    /// The `queue` delivery mode of `session.send`. Cases start every
    /// session it writes with a `queue` send (the mode an absent `delivery`
    /// means), so every case that needs a written session requires it.
    Queue,
    /// The `interrupt` delivery mode of `session.send`.
    Interrupt,
    /// The `compaction` group: scripted Setup, step answers, refusals,
    /// deadlines, request fences and durable replay.
    Compaction,
    /// The `tag_checkpoint` group: the runner honours a compaction
    /// provider's Setup checkpoint `{tag_checkpoint, cut}`.
    TagCheckpoint,
    /// The `session_change` group: `session.refresh`,
    /// `session.refresh_policy` and `session.flush_prefix`. Its cases need
    /// [`LlmRunnerSubject::refresh_plan`] and
    /// [`LlmRunnerSubject::model_requests`].
    SessionChange,
    /// Whole-session retention with limits from `role.describe.retention`.
    Retention,
    /// The durable session input-token cap on `session.send.budget`.
    InputTokenCap,
    /// Positive model-response limit in `stop_when.max_steps`.
    MaxSteps,
    /// Positive per-response output cap in `generation.max_output_tokens`.
    MaxOutputTokens,
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
    /// Runner-owned part tags on `session.read` and `session.resolve`.
    Tags,
    /// `run.tool_calls`: a run's tool calls as a paged list.
    RunToolCalls,
    /// Per-run token usage (`usage`) on `run.result` and `run.status`.
    RunUsage,
    /// Durable session spend on `session.head`.
    SessionUsage,
    /// `session.send` with `delivery_order`, plan-named senders and plan
    /// templates.
    DeliveryOrder,
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
    /// The `client_message_id` group: `client_message_id` on
    /// `session.send`, shown on the user message's display row.
    ClientMessageId,
    /// The `run_cause` group: `send_id` on the live `run_started` event,
    /// naming the send that started the run, absent only for a turn the
    /// host started with no send.
    RunCause,
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
    /// The role groups represented by this enum, in the order of
    /// `cortexkit_role_llm_runner::capabilities::GROUPS`.
    pub const GROUPS: &'static [Capability] = &[
        Self::SystemTextDefault,
        Self::Attachments,
        Self::TranscriptReads,
        Self::HostHistory,
        Self::RunOps,
        Self::RunStatus,
        Self::RunCancel,
        Self::Retract,
        Self::DispatchAttribution,
        Self::Streaming,
        Self::ModelView,
        Self::Steer,
        Self::Queue,
        Self::Interrupt,
        Self::Compaction,
        Self::TagCheckpoint,
        Self::SessionChange,
        Self::Retention,
        Self::InputTokenCap,
        Self::MaxSteps,
        Self::MaxOutputTokens,
        Self::NoTools,
        Self::TurnDirectives,
        Self::InterruptReason,
        Self::StructuredOutput,
        Self::Adopt,
        Self::Tags,
        Self::RunToolCalls,
        Self::RunUsage,
        Self::SessionUsage,
        Self::DeliveryOrder,
        Self::SendStatus,
        Self::UserCommands,
        Self::HistoryDisplay,
        Self::LiveEvents,
        Self::ClientMessageId,
        Self::RunCause,
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
            Self::SystemTextDefault => groups::SYSTEM_TEXT_DEFAULT,
            Self::Attachments => groups::ATTACHMENTS,
            Self::UserCommands => groups::USER_COMMANDS,
            Self::TranscriptReads => groups::TRANSCRIPT_READS,
            Self::HostHistory => groups::HOST_HISTORY,
            Self::RunOps => groups::RUN_OPS,
            Self::RunStatus => groups::RUN_STATUS,
            Self::RunCancel => groups::RUN_CANCEL,
            Self::Retract => groups::RETRACT,
            Self::DispatchAttribution => groups::DISPATCH_ATTRIBUTION,
            Self::Streaming => groups::STREAMING,
            Self::ModelView => groups::MODEL_VIEW,
            Self::Steer => groups::STEER,
            Self::Queue => groups::QUEUE,
            Self::Interrupt => groups::INTERRUPT,
            Self::Compaction => groups::COMPACTION,
            Self::TagCheckpoint => groups::TAG_CHECKPOINT,
            Self::SessionChange => groups::SESSION_CHANGE,
            Self::Retention => groups::RETENTION,
            Self::InputTokenCap => groups::INPUT_TOKEN_CAP,
            Self::MaxSteps => groups::MAX_STEPS,
            Self::MaxOutputTokens => groups::MAX_OUTPUT_TOKENS,
            Self::NoTools => groups::NO_TOOLS,
            Self::TurnDirectives => groups::TURN_DIRECTIVES,
            Self::InterruptReason => groups::INTERRUPT_REASON,
            Self::StructuredOutput => groups::STRUCTURED_OUTPUT,
            Self::Adopt => groups::ADOPT,
            Self::Tags => groups::TAGS,
            Self::RunToolCalls => groups::RUN_TOOL_CALLS,
            Self::RunUsage => groups::RUN_USAGE,
            Self::SessionUsage => groups::SESSION_USAGE,
            Self::DeliveryOrder => groups::DELIVERY_ORDER,
            Self::SendStatus => groups::SEND_STATUS,
            Self::HistoryDisplay => groups::HISTORY_DISPLAY,
            Self::LiveEvents => groups::LIVE_EVENTS,
            Self::ClientMessageId => groups::CLIENT_MESSAGE_ID,
            Self::RunCause => groups::RUN_CAUSE,
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
    /// Report exactly these counts, or omit input usage entirely for `None`.
    /// Adapters must not fill in missing usage with an estimate or zero.
    /// For output-cap cases, the scripted provider honours the maximum it
    /// actually receives, truncating the response and reporting its actual
    /// output usage rather than the full, untruncated scripted count.
    pub usage: Option<ScriptedUsage>,
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
    /// The output-token maximum actually sent to the model, if any.
    pub max_output_tokens: Option<u64>,
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
}

impl ModelRequestObservation {
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
        Self { parts, usage: None }
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
/// Compaction terms are defined in the crate's Contract vocabulary section.
#[async_trait]
pub trait LlmRunnerSubject: Harness {
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

    /// Two distinct, attested non-owner identities for the caller-grant case.
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

    /// Record a model switch through the host's normal switch path, without
    /// rewriting the frozen plan, and return the selection that was recorded.
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

    /// Build admission fields with a tag-rendering plan whose scripted tool
    /// provider declares data:token and program:source:rich-ui/1 slots, and
    /// whose provider item opts into output_refs. Do not synthesize replies.
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

    /// Force a native lineage change that assigns different numbers while
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
    fn owner_stamp(&self) -> RouteStamp;

    /// An identity that is not the owner of any session the owner stamp
    /// starts, for the owner-only cases.
    fn stranger_stamp(&self) -> RouteStamp;

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
    /// [`Capability::NoTools`], [`Capability::MaxOutputTokens`] and
    /// [`Capability::RenderedModelInput`].
    async fn model_requests(
        &self,
        _session: &str,
    ) -> Result<Vec<ModelRequestObservation>, HarnessError> {
        Err(HarnessError::new(
            "model request observation is not implemented",
        ))
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

    /// Like [`Self::send_fields`], but with `members` added to the first
    /// send's plan beside whatever the adapter puts there (`render_tags` for
    /// the tags cases, `senders` for the delivery-order cases). Return
    /// `None` when the runner cannot admit such a plan. Required with
    /// [`Capability::DeliveryOrder`], and by the tags case that renders tags.
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

    /// Whether the scripted model's route supports a turn-scoped system
    /// message, so `turn_system` can be possible. When `false`, the
    /// delivery-order template case is inapplicable.
    fn turn_system_supported(&self) -> bool {
        false
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

    /// A scripted tool result that the adapter's tool provider answers as a
    /// tool result holding only an image (no text), with `marker` somewhere
    /// in the result's bytes as `session.read` returns them (an image's
    /// name, say), so a case can find its entry. `None` when the provider
    /// cannot answer one; the tags case counting an image-only result then
    /// checks the rest of its rules and names the one it could not.
    fn image_tool_result(&self, _marker: &str) -> Option<Value> {
        None
    }

    /// An image result in the host's actual schema, with exactly `bytes` as
    /// decoded image bytes. `nested` requests a tool_result containing the
    /// image; otherwise the result is an image block. Unsupported adapters
    /// name the media-fixture cases inapplicable.
    fn media_tool_result(&self, _bytes: &[u8], _nested: bool, _scenario: &str) -> Option<Value> {
        None
    }

    /// A tool result containing `texts` as separate text payloads, preserving
    /// their order. Used to test descriptor selection when individually
    /// inline-sized text payloads together exceed the per-message budget.
    fn multipart_tool_result(&self, _texts: &[String]) -> Option<Value> {
        None
    }

    /// Top-level parts of a read message in message order, keyed exactly as
    /// tags and replace_part address them. Override for a host's schema;
    /// these defaults cover content-block and parts-array schemas.
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

    /// Make session `fork` a fork or clone of the managed session `parent`
    /// the way the host makes one (OpenCode's fork copies the parent's
    /// parts, marker checkpoint included), so that the runner can know it
    /// was forked. `fork`'s script is installed and its route open, and no
    /// role request has reached it. Return an error when the adapter cannot
    /// create a fork the runner can detect: the fork case is then
    /// inapplicable, since a runner refuses only forks it can know about.
    async fn fork_session(
        &self,
        _handle: &Self::Handle,
        _parent: &str,
        _fork: &str,
    ) -> Result<(), HarnessError> {
        Err(HarnessError::new("forks are not implemented"))
    }

    /// Compact `session` the way its host compacts natively (OpenCode's own
    /// compaction), so that the runner writes its checkpoint key on the
    /// marker. Return an error when the host cannot: the lost-key tags case
    /// is then inapplicable.
    async fn compact_in_host(
        &self,
        _handle: &Self::Handle,
        _session: &str,
    ) -> Result<(), HarnessError> {
        Err(HarnessError::new("host compaction is not implemented"))
    }

    /// Drop the checkpoint key from `session`'s newest compaction marker the
    /// way the host loses it (OpenCode's part-update route drops unknown
    /// keys), leaving the marker in effect. Return an error when the adapter
    /// cannot: the lost-key tags case is then inapplicable.
    async fn drop_marker_key(
        &self,
        _handle: &Self::Handle,
        _session: &str,
    ) -> Result<(), HarnessError> {
        Err(HarnessError::new(
            "dropping a marker key is not implemented",
        ))
    }

    /// The text of the model reply holding `marker`, as the host's own store
    /// keeps it, read outside the runner (an in-process host's save seam has
    /// run on it); `None` when the store holds no such reply. Return an
    /// error when the adapter cannot read the host's store: the save-seam
    /// tags case is then inapplicable.
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

    /// Whether the runner's host runs standalone Magic Context in-process,
    /// so a runner that has never served a session hands it to standalone on
    /// `standalone_session_not_migrated` instead of failing the run. The
    /// refusal case checks that code only when this is `false`.
    fn standalone_in_process(&self) -> bool {
        false
    }

    /// Make the next run call Setup again, as a provider reconnect would,
    /// without changing the history or whether the runner has already served it.
    /// Without this hook the case cannot check that a later
    /// `standalone_session_not_migrated` fails rather than handing off.
    async fn require_setup_again(
        &self,
        _handle: &Self::Handle,
        _session: &str,
    ) -> Result<(), HarnessError> {
        Err(HarnessError::new("repeating Setup is not implemented"))
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
    /// Compaction may hide assistant messages from the model view; for a
    /// session with a compaction script, select turns by emitted assistant
    /// answers instead, keeping that cursor across held calls and restarts.
    async fn install_script(&self, session: &str, script: Script) -> Result<(), HarnessError>;

    /// Hold the next actual model answer for this session until explicitly
    /// released. Called before the send. Required when describe lists
    /// `run_cancel.at: step_boundary`; do not pause the runner before it sends
    /// the model request, since cancellation must arrive during a step.
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
    /// model step whose answer was discarded after cancellation.
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

    /// Install the scripted compaction provider for `session`, answering the
    /// runner's Setup call with `setup`. Initial installation must precede
    /// admission, and the session plan must name that provider. Installing
    /// again changes the provider's configured answer, not its captured
    /// calls or the runner's persistent records. Translate replacement text
    /// to the runner's message schema; preserve ranges, ids and versions
    /// verbatim. The provider must hold every model answer until
    /// [`Self::release_compaction_model`]. Required when the runner adapter
    /// declares `compaction`; cases do not call this method without that
    /// capability.
    async fn install_compaction_setup(
        &self,
        _session: &str,
        _setup: crate::CompactionSetup,
    ) -> Result<(), HarnessError> {
        Err(HarnessError::new("scripted compaction is not implemented"))
    }

    /// Return Setup calls observed at the scripted provider and canonical
    /// inputs captured at the scripted model. For each Setup call, capture
    /// `harness` and `session` from the body and the actual `harness`,
    /// `session` and `project_root` from the route that received it; route
    /// metadata must not be reconstructed from the body. A shared project
    /// route may have a generic harness and an empty session. Keep both
    /// observations outside the runner process and persistent state directory,
    /// and do not reconstruct them from configured answers or `session.read`
    /// responses. `CompactionCall` pairs the body and route identities so the
    /// case can distinguish a single-session bind from a shared route.
    async fn compaction_observation(
        &self,
        _session: &str,
    ) -> Result<crate::CompactionObservation, HarnessError> {
        Err(HarnessError::new(
            "compaction observation is not implemented",
        ))
    }

    /// Publish `view` from the scripted compaction provider of `session`, as
    /// a provider publishes one: durably, then announced with `view_ready`.
    /// Translate replacement text to the runner's message schema, compute
    /// the view digest, and keep ranges, ordinals, `mid`s and the cut
    /// verbatim. Return the view's version: one above every version the
    /// provider published for the session. The runner fetches, checks and
    /// applies it at its next step boundary, as it would any view. Return an
    /// error when the adapter cannot publish views: the cases that need one
    /// are then inapplicable, by name.
    async fn publish_compaction_view(
        &self,
        _session: &str,
        _view: crate::ScriptedView,
    ) -> Result<u64, HarnessError> {
        Err(HarnessError::new(
            "publishing compaction views is not implemented",
        ))
    }

    /// Write `messages` finished messages into `session`'s history through
    /// the runner's own store, then a cut, so the session's current lineage
    /// holds only the newest `window` of them, as a session that ran for
    /// months and compacted along the way would. `session` already has a
    /// run behind it; the seeded messages follow it. The runner must treat
    /// them exactly as messages it wrote itself. Return an error when the
    /// adapter cannot seed: the cost case is then inapplicable, never
    /// passed.
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

    /// Give `session` `count` messages of history in the host before the
    /// runner numbers anything, as when the runner adopts a session its host
    /// already holds: the runner has no ordinals for them, its window starts
    /// at ordinal 0 after them, and `session.history` serves them from the
    /// host's own storage. With `cut`, the host's history ends in a cut the
    /// host made before the runner adopted the session, folding the newest
    /// of those messages: the runner's first lineage carries that cut, with
    /// `first_ordinal` 0, and the runner has no lineage at it. Called before
    /// the session's first send. Return an error when the adapter cannot:
    /// the cases that need it are then inapplicable, by name.
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
    /// runner moves the session to a new lineage. Return an error when the
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

    /// Hold the next Setup call the runner makes for `session` at the
    /// scripted provider, unanswered, until
    /// [`Self::release_compaction_setup`]. Return an error when the adapter
    /// cannot: the case that needs it is then inapplicable, by name.
    async fn hold_next_compaction_setup(&self, _session: &str) -> Result<(), HarnessError> {
        Err(HarnessError::new("holding a Setup call is not implemented"))
    }

    /// Resolve once the scripted provider holds the Setup call
    /// [`Self::hold_next_compaction_setup`] asked it to hold.
    async fn await_compaction_setup_held(&self, _session: &str) -> Result<(), HarnessError> {
        Err(HarnessError::new("holding a Setup call is not implemented"))
    }

    /// Answer the held Setup call with the session's installed answer.
    async fn release_compaction_setup(&self, _session: &str) -> Result<(), HarnessError> {
        Err(HarnessError::new("holding a Setup call is not implemented"))
    }

    /// Whether the runner refuses reads `transient` while a tag-rendering
    /// run waits for a Setup answer without `descends_from`, as the contract
    /// lets it. Only such a runner could hold reads during a cut's Setup, so
    /// the case checking it never does is inapplicable to any other.
    fn holds_reads_during_setup(&self) -> bool {
        false
    }

    /// Advance the session's retention clock by `milliseconds`, completing
    /// expiry and any due deletion work before returning. Durable activity
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

    /// Release exactly one held model call for `session`. A session with a
    /// scripted compaction provider holds every model answer, letting cases inspect the canonical
    /// input and durable records before the answer lets the runner prepare
    /// another model request.
    async fn release_compaction_model(&self, _session: &str) -> Result<(), HarnessError> {
        Err(HarnessError::new(
            "held compaction model is not implemented",
        ))
    }

    /// Read the initial CompactionMessage from the runner's persistent store.
    /// Do not substitute an in-memory cache, the configured provider answer,
    /// or a `session.read` model page. Return `None` if no initial answer is
    /// durably recorded. Cases call this method while the first model call
    /// is held, to check that Setup was persistent before the model request.
    async fn durable_compaction_setup(
        &self,
        _handle: &Self::Handle,
        _session: &str,
    ) -> Result<Option<crate::CompactionMessage>, HarnessError> {
        Err(HarnessError::new(
            "durable Setup inspection is not implemented",
        ))
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

    /// Move `session`'s prompt-cache clock forward by `milliseconds`: the
    /// runner's next model request finds that much more time passed since
    /// its previous request reached the cache. The `cache_ttl` cases are
    /// inapplicable, by name, when this answers an error.
    async fn advance_prompt_cache_clock(
        &self,
        _session: &str,
        _milliseconds: u64,
    ) -> Result<(), HarnessError> {
        Err(HarnessError::new("a prompt-cache clock is not implemented"))
    }

    /// Make `session`'s model requests from the next one on go to `model`
    /// (1 to 256 bytes of printable ASCII, no space), as the runner's own
    /// model switch would; the `model` header of the runner's compaction
    /// advance notices then names it. Before the session's first send this
    /// sets the model the session starts with. A switch after that is a
    /// model change, and the scripted compaction provider answers its
    /// `model_change` ride without a `cache_ttl` entry, so the runner holds
    /// no entry for the new model, as when it missed the ride.
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

    /// The `ride_wait_max_ms` the runner's latest Setup request for
    /// `session` carried, as the scripted compaction provider received it
    /// (compaction-provider §4): `None` when the request carried none,
    /// which means the runner does not honour provider-set ride waits. The
    /// `ride_wait_` cases are inapplicable, by name, when this answers
    /// `None` or an error.
    async fn observed_ride_wait_max_ms(&self, _session: &str) -> Result<Option<u64>, HarnessError> {
        Err(HarnessError::new(
            "observing a Setup request's ride_wait_max_ms is not implemented",
        ))
    }

    /// Make the scripted compaction provider of `session` answer the
    /// session's next rewrite ride as `ride` says: after
    /// `ride.answer_after_ms` milliseconds from the ride's arrival, with its
    /// view or `nothing`; an answer later than the runner's deadline for the
    /// ride is never seen, and the ride times out. Only that one ride
    /// follows the script; every
    /// other ride is answered as the scripted provider answers it without
    /// one. The `ride_wait_` cases are inapplicable, by name, when this
    /// answers an error.
    async fn script_next_ride(
        &self,
        _session: &str,
        _ride: crate::compaction::ScriptedRide,
    ) -> Result<(), HarnessError> {
        Err(HarnessError::new(
            "scripting a ride's answer is not implemented",
        ))
    }

    /// Wait a short while between polls that check whether a run has ended.
    async fn pause(&self);
}

/// External observations for the preset-less system-text admission cases.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
#[non_exhaustive]
pub struct SystemTextDefaultObservation {
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
