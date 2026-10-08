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
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum Capability {
    /// The `transcript_reads` group: `session.read` and `session.head`.
    TranscriptReads,
    /// The `run_ops` group: `run.result` and run attribution.
    RunOps,
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
        Self::DispatchAttribution,
        Self::Streaming,
        Self::ModelView,
        Self::Steer,
        Self::Queue,
        Self::Interrupt,
        Self::Compaction,
        Self::SessionChange,
        Self::Retention,
    ];

    /// The name the report prints. A group prints as the role spells it.
    pub fn name(self) -> String {
        match self.group() {
            Some(group) => group.to_owned(),
            None => match self {
                Self::HoldToolCalls => "hold_tool_calls".to_owned(),
                Self::KillAt(point) => format!("kill_at:{point}"),
                _ => unreachable!("every other capability is a group"),
            },
        }
    }

    /// The role's name for a capability group, or `None` for a harness
    /// capability.
    pub fn group(self) -> Option<&'static str> {
        Some(match self {
            Self::TranscriptReads => groups::TRANSCRIPT_READS,
            Self::RunOps => groups::RUN_OPS,
            Self::DispatchAttribution => groups::DISPATCH_ATTRIBUTION,
            Self::Streaming => groups::STREAMING,
            Self::ModelView => groups::MODEL_VIEW,
            Self::Steer => groups::STEER,
            Self::Queue => groups::QUEUE,
            Self::Interrupt => groups::INTERRUPT,
            Self::Compaction => groups::COMPACTION,
            Self::SessionChange => groups::SESSION_CHANGE,
            Self::Retention => groups::RETENTION,
            Self::HoldToolCalls | Self::KillAt(_) => return None,
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
pub struct ScriptedTurn {
    pub parts: Vec<ScriptedPart>,
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
    /// Always [`crate::SCRIPTED_TOOL`].
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
    /// The turn's text parts, in order.
    pub fn text_parts(&self) -> impl Iterator<Item = &str> {
        self.parts.iter().filter_map(|part| match part {
            ScriptedPart::Text(text) => Some(text.as_str()),
            _ => None,
        })
    }
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
    /// The capability groups the runner declares, plus the harness
    /// capabilities ([`Capability::HoldToolCalls`]). Kill points come from
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
    /// session's `plan` (whose shape the role has not pinned yet, so the
    /// runner adapter builds it) and any runner parameters admission needs. The
    /// plan must make [`crate::SCRIPTED_TOOL`], served by the scripted tool
    /// provider, available to the model.
    fn send_fields(&self, session: &str, first: bool) -> Map<String, Value>;

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

    /// Install the scripted compaction provider and controlled clock for
    /// `session`. Initial installation must precede admission, and the
    /// session plan must name that provider. Replacing an installed script
    /// changes the provider's configured answers, not its captured calls,
    /// clock or the runner's persistent records.
    /// Translate replacement text to the runner's message schema; preserve
    /// ranges, ids, versions and request ids verbatim. Configure the runner's
    /// timers and token estimator from [`crate::CompactionScript`].
    /// Required when the runner adapter declares `compaction`; cases do not
    /// call this method without that capability.
    async fn install_compaction_script(
        &self,
        _session: &str,
        _script: crate::CompactionScript,
    ) -> Result<(), HarnessError> {
        Err(HarnessError::new("scripted compaction is not implemented"))
    }

    /// Return provider calls observed at the scripted provider and canonical
    /// inputs captured from requests received by the scripted model. Keep
    /// both observations outside the runner process and its persistent state
    /// directory so a runner crash cannot erase them. Do not reconstruct
    /// observations from configured answers or `session.read` responses.
    async fn compaction_observation(
        &self,
        _session: &str,
    ) -> Result<crate::CompactionObservation, HarnessError> {
        Err(HarnessError::new(
            "compaction observation is not implemented",
        ))
    }

    /// Advance the injected runner/provider clock by `milliseconds` and
    /// complete all work due at the resulting instant, including timer
    /// handlers and durable terminal records, before returning. Time must
    /// advance only through this method. The clock and scripted provider must
    /// survive runner crashes. Advance the runner's actual timers; advancing
    /// only adapter timestamps while runner timers use another clock is not
    /// acceptable for deadline and cap checks.
    async fn advance_compaction_clock(
        &self,
        _session: &str,
        _milliseconds: u64,
    ) -> Result<(), HarnessError> {
        Err(HarnessError::new(
            "controlled compaction clock is not implemented",
        ))
    }

    /// Deliver `answer` for `calls[call_index]` returned by
    /// [`Self::compaction_observation`]; the zero-based call list includes
    /// Setup. Deliver late, stale and duplicate answers to the runner's
    /// answer-disposition path, even after timeout; the runner adapter must
    /// not filter them. `request_id: None` echoes the indexed call's id;
    /// `Some(id)` must send `id` verbatim so cases can check the fence.
    async fn answer_compaction(
        &self,
        _session: &str,
        _call_index: usize,
        _request_id: Option<String>,
        _answer: crate::CompactionAnswer,
    ) -> Result<(), HarnessError> {
        Err(HarnessError::new(
            "compaction answer delivery is not implemented",
        ))
    }

    /// Release exactly one held model call for `session`. Compaction scripts
    /// must hold every model answer, letting cases inspect the canonical
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
