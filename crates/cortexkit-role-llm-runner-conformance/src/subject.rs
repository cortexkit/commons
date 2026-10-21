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
    /// The `transcript_reads` group: `session.read` and `session.head`.
    TranscriptReads,
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
    /// The `session_change` group. Conformance cases do not require it.
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
    /// `session.send` with `delivery_order`, plan-named senders and plan
    /// templates.
    DeliveryOrder,
    /// `send.status`: what a re-send of a `send_id` would answer, without
    /// its body.
    SendStatus,
    /// Owner user-command relay, with replies isolated from model history.
    UserCommands,
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
    /// The harness can kill the runner at this point of
    /// `cortexkit_role_llm_runner::points`. Never listed by
    /// [`LlmRunnerSubject::capabilities`]: the conformance driver derives it from
    /// [`Harness::declared_points`].
    KillAt(&'static str),
}

impl Capability {
    /// Every capability group of the role, in the order of
    /// `cortexkit_role_llm_runner::capabilities::GROUPS`.
    pub const GROUPS: &'static [Capability] = &[
        Self::TranscriptReads,
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
        Self::DeliveryOrder,
        Self::SendStatus,
        Self::UserCommands,
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
                Self::KillAt(point) => format!("kill_at:{point}"),
                _ => unreachable!("every other capability is a group"),
            },
        }
    }

    /// The role's name for a capability group, or `None` for a harness
    /// capability.
    pub fn group(self) -> Option<&'static str> {
        Some(match self {
            Self::UserCommands => groups::USER_COMMANDS,
            Self::TranscriptReads => groups::TRANSCRIPT_READS,
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
            Self::DeliveryOrder => groups::DELIVERY_ORDER,
            Self::SendStatus => groups::SEND_STATUS,
            Self::HoldToolCalls
            | Self::HoldContextCeiling
            | Self::HostMetadataChange
            | Self::RetentionClock
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
        }
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
    /// Install a scripted provider declaring `ctx-status` and `ctx-wrapup`.
    /// The first replies done with `command-private-reply`; the second replies
    /// accepted, then emits `command-progress-1`, `command-progress-2`, and a
    /// done result `command-private-result`. Collision adds a second provider
    /// declaring `ctx-status`. Return the session reference and admission fields
    /// for those providers.
    /// Set up the host's command-reply interface before session admission;
    /// this setup is not a protocol operation.
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
    /// [`Capability::NoTools`] and [`Capability::MaxOutputTokens`].
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

    /// Wait a short while between polls that check whether a run has ended.
    async fn pause(&self);
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
