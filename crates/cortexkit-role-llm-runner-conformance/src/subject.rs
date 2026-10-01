//! What a runner under test supplies beyond its harness: the capabilities
//! it declares, how to reach a session's management route, and a scripted
//! model and tool provider.

use std::collections::BTreeSet;

use async_trait::async_trait;
use cortexkit_role_harness::{Harness, HarnessError, RouteStamp};
use cortexkit_role_llm_runner::capabilities as groups;
use serde_json::{Map, Value};

/// A capability a conformance case requires of the runner or its harness.
/// A case whose requirement the subject does not declare is reported as
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
    /// The `model_view` group. No case the suite implements yet requires
    /// it.
    ModelView,
    /// The `steer` delivery mode of `session.send`.
    Steer,
    /// The `queue` delivery mode of `session.send`. The suite starts every
    /// session it writes with a `queue` send (the mode an absent `delivery`
    /// means), so every case that needs a written session requires it.
    Queue,
    /// The `interrupt` delivery mode of `session.send`.
    Interrupt,
    /// The `compaction` group. No case the suite implements yet requires
    /// it.
    Compaction,
    /// The `session_change` group. No case the suite implements yet
    /// requires it.
    SessionChange,
    /// The subject's scripted tool provider can hold a call it received
    /// until the suite releases it. See
    /// [`LlmRunnerSubject::await_tool_call`] and
    /// [`LlmRunnerSubject::release_tool_call`].
    HoldToolCalls,
    /// The harness can kill the runner at this point of
    /// `cortexkit_role_llm_runner::points`. Never listed by
    /// [`LlmRunnerSubject::capabilities`]: the runner derives it from
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
/// order the model gives them. The subject adapts it to whatever its
/// runner's model provider needs.
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
    /// The model's id for the call. Display only: the suite reuses one id
    /// across turns on purpose.
    pub tool_call_id: String,
    /// Always [`crate::SCRIPTED_TOOL`].
    pub tool: String,
    /// The call's arguments. Unique within a suite run, so the scripted
    /// tool provider can find the call's result by them.
    pub arguments: Value,
    /// What the scripted tool provider answers the call with.
    pub result: Value,
    /// Hold the call in the tool provider until the suite releases it.
    /// Only set when the subject declares [`Capability::HoldToolCalls`].
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

/// A live runner under test: its harness plus what the suite cannot know
/// about it. Every request the suite makes still goes over the runner's
/// real management route; nothing here answers on the runner's behalf.
///
/// The suite opens every route with [`LlmRunnerSubject::session_route`],
/// because every role op rides a session-bound route. It never calls
/// [`Harness::route`].
#[async_trait]
pub trait LlmRunnerSubject: Harness {
    /// The capability groups the runner declares, plus the harness
    /// capabilities ([`Capability::HoldToolCalls`]). Kill points come from
    /// [`Harness::declared_points`] instead. Cases requiring anything
    /// missing are skipped, with the reason.
    fn capabilities(&self) -> BTreeSet<Capability>;

    /// The identity the suite drives every session as: the session's owner.
    fn owner_stamp(&self) -> RouteStamp;

    /// An identity that is not the owner of any session the owner stamp
    /// starts, for the owner-only cases.
    fn stranger_stamp(&self) -> RouteStamp;

    /// Open the management route bound to the session the suite calls
    /// `session`, under `stamp`, as a real caller with that identity would.
    /// The suite uses a name once per run; the subject may map it to
    /// whatever session identity its runner uses.
    async fn session_route(
        &self,
        handle: &Self::Handle,
        session: &str,
        stamp: &RouteStamp,
    ) -> Result<Self::Route, HarnessError>;

    /// Fields the suite adds to every `session.send` it makes into
    /// `session`, beside the role's `prompt`, `send_id` and `delivery`.
    /// `first` is true for the session's first send, which carries the
    /// session's `plan` (whose shape the role has not pinned yet, so the
    /// subject builds it) and any runner parameters admission needs. The
    /// plan must make [`crate::SCRIPTED_TOOL`], served by the scripted tool
    /// provider, available to the model.
    fn send_fields(&self, session: &str, first: bool) -> Map<String, Value>;

    /// The module id the runner records as `dispatched_to` for a call to
    /// the scripted tool provider.
    fn tool_provider_module(&self) -> String;

    /// Make `script` the model of `session`, before the suite's first send
    /// into it. It stays in force across kills and restarts. The subject's
    /// model answers a model request of the session with
    /// `script.turns[n]`, where `n` is the number of assistant messages in
    /// the history the request carries; the suite never lets a session ask
    /// for more turns than its script holds.
    async fn install_script(&self, session: &str, script: Script) -> Result<(), HarnessError>;

    /// How many times the scripted tool provider was called with
    /// `arguments`, counted across every kill and restart in the run.
    ///
    /// The count must be kept outside the runner's process and outside its
    /// state root: in the suite's own process, or on disk somewhere a kill
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

    /// Wait a short while. The suite calls this between polls while it
    /// waits for a run to end.
    async fn pause(&self);
}
