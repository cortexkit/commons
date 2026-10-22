//! Live events on `session.subscribe` (the `live_events` group,
//! CONTRACT.md §7.1).
//!
//! A runner that declares the group puts a typed `event: {type, ...}` on its
//! stream frames, beside its own members:
//!
//! - durable `control` frames carry the message, tool and run lifecycle
//!   (`message_started`, `message_finished`, `tool_started`,
//!   `tool_finished`, `run_started`, `run_finished`, `run_paused`), each
//!   mapped onto a record the runner already writes durably, so they replay
//!   after a reconnect;
//! - best-effort `display` frames carry coalesced deltas of the step in
//!   flight (`text_delta`, `reasoning_delta`, `tool_input_delta`) and the
//!   stream's own signals (`display_gap`, `display_reset`).
//!
//! Every frame of one stream carries `seq`, starting at [`FIRST_SEQ`] and
//! rising by one per frame across both lanes. A jump in `seq`, or a
//! `display_gap`, means frames were lost: the reader drops its live rows and
//! attaches again. A reader decodes an event type it does not know as
//! `display_reset` ([`LiveEvent`]'s lenient decoding).

use serde::{Deserialize, Deserializer, Serialize};
use serde_json::Value;

use crate::display::Limits;
use crate::subscribe::{SubscribeEvent, SubscribeFrom, SubscribeRequest};

/// The `seq` of a stream's first frame.
pub const FIRST_SEQ: u64 = 1;

/// The frame member carrying the stream position.
pub const SEQ_FIELD: &str = "seq";

/// The frame member carrying the typed event.
pub const EVENT_FIELD: &str = "event";

/// The event types, as `type` spells them.
pub mod event_types {
    pub const MESSAGE_STARTED: &str = "message_started";
    pub const MESSAGE_FINISHED: &str = "message_finished";
    pub const TOOL_STARTED: &str = "tool_started";
    pub const TOOL_FINISHED: &str = "tool_finished";
    pub const RUN_STARTED: &str = "run_started";
    pub const RUN_FINISHED: &str = "run_finished";
    pub const RUN_PAUSED: &str = "run_paused";
    pub const TEXT_DELTA: &str = "text_delta";
    pub const REASONING_DELTA: &str = "reasoning_delta";
    pub const TOOL_INPUT_DELTA: &str = "tool_input_delta";
    pub const DISPLAY_GAP: &str = "display_gap";
    pub const DISPLAY_RESET: &str = "display_reset";
}

/// Known `tool_finished.outcome` values. The member is an open string.
pub mod tool_outcomes {
    pub const OK: &str = "ok";
    pub const ERROR: &str = "error";
    /// A caller asked for the call to stop.
    pub const CANCELLED: &str = "cancelled";
    /// A restart cut the call, and it was never sent again.
    pub const INTERRUPTED: &str = "interrupted";
    /// An approval or a hook refused the call.
    pub const DENIED: &str = "denied";
}

/// Known `run_finished.reason` and `run_paused.reason` values. The member is
/// an open string.
pub mod run_reasons {
    pub const COMPLETED: &str = "completed";
    pub const CANCELLED: &str = "cancelled";
    pub const INTERRUPTED: &str = "interrupted";
    pub const MAX_STEPS: &str = "max_steps";
    pub const ERROR: &str = "error";
    pub const BUDGET_EXHAUSTED: &str = "budget_exhausted";
    pub const CONTEXT_CEILING: &str = "context_ceiling";
}

/// The `session.subscribe` request a runner declaring `live_events` reads:
/// [`SubscribeRequest`]'s `from`, plus the optional `limits` that bound
/// `tool_started.arguments` as they bound a display row. Strict, like
/// [`SubscribeRequest`]: a runner without the group refuses `limits` as an
/// unknown field, so a caller sends it only to a runner declaring the group.
///
/// Non-exhaustive: build one with [`LiveSubscribeRequest::new`] and
/// [`LiveSubscribeRequest::with_limits`], or decode one.
#[derive(Clone, Debug, Default, PartialEq, Eq, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
#[non_exhaustive]
pub struct LiveSubscribeRequest {
    /// Absent means [`SubscribeFrom::Live`].
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub from: Option<SubscribeFrom>,
    /// Absent means whole arguments.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub limits: Option<Limits>,
}

impl LiveSubscribeRequest {
    pub fn new(from: SubscribeFrom) -> Self {
        Self {
            from: Some(from),
            limits: None,
        }
    }

    pub fn with_limits(mut self, limits: Limits) -> Self {
        self.limits = Some(limits);
        self
    }

    /// The attach point the request selects.
    pub fn attach_point(&self) -> SubscribeFrom {
        self.from.clone().unwrap_or(SubscribeFrom::Live)
    }

    /// The same request without `limits`, as a runner without the group
    /// reads it.
    pub fn base(&self) -> SubscribeRequest {
        SubscribeRequest {
            from: self.from.clone(),
        }
    }
}

/// `message_started` and `message_finished`: the message's `mid`, the one
/// its stored entry carries.
#[derive(Clone, Debug, PartialEq, Eq, Deserialize, Serialize)]
#[non_exhaustive]
pub struct MessageEvent {
    pub mid: String,
}

impl MessageEvent {
    pub fn new(mid: impl Into<String>) -> Self {
        Self { mid: mid.into() }
    }
}

/// `tool_started`: a call of message `mid` went to its tool. `arguments` is
/// the call's arguments as JSON text, cut by the subscription's `limits`
/// like a display row's.
#[derive(Clone, Debug, PartialEq, Eq, Deserialize, Serialize)]
#[non_exhaustive]
pub struct ToolStarted {
    pub mid: String,
    /// The model's id for the call, as the stored row carries it.
    pub tool_call_id: String,
    pub name: String,
    pub arguments: String,
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub arguments_truncated: bool,
    /// Milliseconds since the Unix epoch.
    pub started_at: u64,
}

impl ToolStarted {
    pub fn new(
        mid: impl Into<String>,
        tool_call_id: impl Into<String>,
        name: impl Into<String>,
        arguments: &Value,
        limits: Option<&Limits>,
        started_at: u64,
    ) -> Self {
        let text = serde_json::to_string(arguments).expect("JSON values serialize");
        let (arguments, arguments_truncated) = match limits {
            Some(limits) => crate::display::cut(&text, limits.arguments_bytes),
            None => (text, false),
        };
        Self {
            mid: mid.into(),
            tool_call_id: tool_call_id.into(),
            name: name.into(),
            arguments,
            arguments_truncated,
            started_at,
        }
    }
}

/// `tool_finished`: the call of message `mid` ended with `outcome`
/// ([`tool_outcomes`]). `output_bytes` is the byte length of its result's
/// content uncut, as a display row's `tool_result.content` holds it.
#[derive(Clone, Debug, PartialEq, Eq, Deserialize, Serialize)]
#[non_exhaustive]
pub struct ToolFinished {
    pub mid: String,
    pub tool_call_id: String,
    pub outcome: String,
    pub output_bytes: u64,
    /// Milliseconds since the Unix epoch.
    pub finished_at: u64,
}

impl ToolFinished {
    pub fn new(
        mid: impl Into<String>,
        tool_call_id: impl Into<String>,
        outcome: impl Into<String>,
        output_bytes: u64,
        finished_at: u64,
    ) -> Self {
        Self {
            mid: mid.into(),
            tool_call_id: tool_call_id.into(),
            outcome: outcome.into(),
            output_bytes,
            finished_at,
        }
    }
}

/// `run_started`. On a runner declaring `run_cause`
/// ([`crate::capabilities::RUN_CAUSE`]), `send_id` names the send that
/// started the run: for a queued send, that send's own `send_id`, never an
/// id the runner made for its queue; for an interrupt, the interrupting
/// send, never the one it interrupted. It is left out only for a turn the
/// host started with no send (a person typing in the host's own app on an
/// adopted session), and is never an empty string or a guess. On a runner
/// without the group an absent `send_id` says nothing about the cause.
#[derive(Clone, Debug, PartialEq, Eq, Deserialize, Serialize)]
#[non_exhaustive]
pub struct RunStarted {
    pub run_id: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub send_id: Option<String>,
}

impl RunStarted {
    pub fn new(run_id: impl Into<String>) -> Self {
        Self {
            run_id: run_id.into(),
            send_id: None,
        }
    }

    /// Name the send that started the run.
    pub fn with_send_id(mut self, send_id: impl Into<String>) -> Self {
        self.send_id = Some(send_id.into());
        self
    }
}

/// `run_finished` (the run ended) and `run_paused` (the run stopped and can
/// resume under the same `run_id`), with `reason` ([`run_reasons`]).
#[derive(Clone, Debug, PartialEq, Eq, Deserialize, Serialize)]
#[non_exhaustive]
pub struct RunEnded {
    pub run_id: String,
    pub reason: String,
}

impl RunEnded {
    pub fn new(run_id: impl Into<String>, reason: impl Into<String>) -> Self {
        Self {
            run_id: run_id.into(),
            reason: reason.into(),
        }
    }
}

/// `text_delta` and `reasoning_delta`: a coalesced chunk of a text or
/// reasoning part of the step in flight. `message_id` is the model
/// provider's id and may be empty. `mid` is the message's final `mid`,
/// known from its first event; `part` is the key the stored row gives the
/// part. A runner without `live_events` sends neither, so both decode as
/// optional.
#[derive(Clone, Debug, PartialEq, Eq, Deserialize, Serialize)]
#[non_exhaustive]
pub struct TextDelta {
    #[serde(default)]
    pub message_id: String,
    pub delta: String,
    /// On `text_delta` only: text that belongs to a tool call.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub tool_call_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub mid: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub part: Option<String>,
}

impl TextDelta {
    pub fn new(
        message_id: impl Into<String>,
        delta: impl Into<String>,
        mid: impl Into<String>,
        part: impl Into<String>,
    ) -> Self {
        Self {
            message_id: message_id.into(),
            delta: delta.into(),
            tool_call_id: None,
            mid: Some(mid.into()),
            part: Some(part.into()),
        }
    }
}

/// `tool_input_delta`: a coalesced chunk of a tool call's arguments text.
#[derive(Clone, Debug, PartialEq, Eq, Deserialize, Serialize)]
#[non_exhaustive]
pub struct ToolInputDelta {
    pub tool_call_id: String,
    pub delta: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub mid: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub part: Option<String>,
}

impl ToolInputDelta {
    pub fn new(
        tool_call_id: impl Into<String>,
        delta: impl Into<String>,
        mid: impl Into<String>,
        part: impl Into<String>,
    ) -> Self {
        Self {
            tool_call_id: tool_call_id.into(),
            delta: delta.into(),
            mid: Some(mid.into()),
            part: Some(part.into()),
        }
    }
}

/// `display_gap`: the runner dropped `dropped` display frames.
#[derive(Clone, Debug, PartialEq, Eq, Deserialize, Serialize)]
#[non_exhaustive]
pub struct DisplayGap {
    pub dropped: u64,
}

impl DisplayGap {
    pub fn new(dropped: u64) -> Self {
        Self { dropped }
    }
}

/// A typed stream event. Decoded leniently: an event whose `type` this
/// crate does not know, or a known type whose members do not decode, is
/// [`LiveEvent::DisplayReset`], so a reader drops its live state rather
/// than show something it cannot place. [`LiveEvent::decode_strict`] is the
/// strict decoding a conformance check uses.
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
#[serde(tag = "type", rename_all = "snake_case")]
#[non_exhaustive]
pub enum LiveEvent {
    MessageStarted(MessageEvent),
    MessageFinished(MessageEvent),
    ToolStarted(ToolStarted),
    ToolFinished(ToolFinished),
    RunStarted(RunStarted),
    RunFinished(RunEnded),
    RunPaused(RunEnded),
    TextDelta(TextDelta),
    ReasoningDelta(TextDelta),
    ToolInputDelta(ToolInputDelta),
    DisplayGap(DisplayGap),
    /// Drop all live state for the step in flight; its deltas stream again.
    DisplayReset,
}

impl LiveEvent {
    /// Decode `value` strictly: an unknown `type`, or members that do not
    /// decode for a known one, is an error naming the problem.
    pub fn decode_strict(value: &Value) -> Result<Self, String> {
        use event_types as t;
        let kind = value
            .get("type")
            .and_then(Value::as_str)
            .ok_or("an event without a type")?;
        fn typed<T: serde::de::DeserializeOwned>(kind: &str, value: &Value) -> Result<T, String> {
            serde_json::from_value(value.clone()).map_err(|e| format!("{kind}: {e}"))
        }
        Ok(match kind {
            t::MESSAGE_STARTED => Self::MessageStarted(typed(kind, value)?),
            t::MESSAGE_FINISHED => Self::MessageFinished(typed(kind, value)?),
            t::TOOL_STARTED => Self::ToolStarted(typed(kind, value)?),
            t::TOOL_FINISHED => Self::ToolFinished(typed(kind, value)?),
            t::RUN_STARTED => Self::RunStarted(typed(kind, value)?),
            t::RUN_FINISHED => Self::RunFinished(typed(kind, value)?),
            t::RUN_PAUSED => Self::RunPaused(typed(kind, value)?),
            t::TEXT_DELTA => Self::TextDelta(typed(kind, value)?),
            t::REASONING_DELTA => Self::ReasoningDelta(typed(kind, value)?),
            t::TOOL_INPUT_DELTA => Self::ToolInputDelta(typed(kind, value)?),
            t::DISPLAY_GAP => Self::DisplayGap(typed(kind, value)?),
            t::DISPLAY_RESET => Self::DisplayReset,
            other => return Err(format!("an unknown event type {other}")),
        })
    }

    /// The type as `type` spells it.
    pub fn type_name(&self) -> &'static str {
        use event_types as t;
        match self {
            Self::MessageStarted(_) => t::MESSAGE_STARTED,
            Self::MessageFinished(_) => t::MESSAGE_FINISHED,
            Self::ToolStarted(_) => t::TOOL_STARTED,
            Self::ToolFinished(_) => t::TOOL_FINISHED,
            Self::RunStarted(_) => t::RUN_STARTED,
            Self::RunFinished(_) => t::RUN_FINISHED,
            Self::RunPaused(_) => t::RUN_PAUSED,
            Self::TextDelta(_) => t::TEXT_DELTA,
            Self::ReasoningDelta(_) => t::REASONING_DELTA,
            Self::ToolInputDelta(_) => t::TOOL_INPUT_DELTA,
            Self::DisplayGap(_) => t::DISPLAY_GAP,
            Self::DisplayReset => t::DISPLAY_RESET,
        }
    }
}

impl<'de> Deserialize<'de> for LiveEvent {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        let value = Value::deserialize(deserializer)?;
        Ok(Self::decode_strict(&value).unwrap_or(Self::DisplayReset))
    }
}

impl SubscribeEvent {
    /// A frame on lane `kind` with no other member yet.
    pub fn new(kind: impl Into<String>) -> Self {
        Self {
            kind: kind.into(),
            cursor: None,
            body: serde_json::Map::new(),
        }
    }

    /// The frame's stream position, on a `live_events` runner's stream.
    pub fn seq(&self) -> Option<u64> {
        self.body.get(SEQ_FIELD).and_then(Value::as_u64)
    }

    /// The frame's typed event, decoded leniently; `None` for a frame
    /// without one.
    pub fn live_event(&self) -> Option<LiveEvent> {
        self.body
            .get(EVENT_FIELD)
            .map(|event| serde_json::from_value(event.clone()).expect("lenient decoding"))
    }

    pub fn with_seq(mut self, seq: u64) -> Self {
        self.body.insert(SEQ_FIELD.into(), Value::from(seq));
        self
    }

    pub fn with_event(mut self, event: &LiveEvent) -> Self {
        self.body.insert(
            EVENT_FIELD.into(),
            serde_json::to_value(event).expect("events serialize"),
        );
        self
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::vectors;

    #[test]
    fn event_vectors_decode_strictly_and_round_trip() {
        let file = vectors::load("live-events.json");
        for case in vectors::cases(&file, "events") {
            let name = case["name"].as_str().unwrap();
            let event =
                LiveEvent::decode_strict(&case["event"]).unwrap_or_else(|e| panic!("{name}: {e}"));
            assert_eq!(event.type_name(), case["event"]["type"], "{name}");
            assert_eq!(
                serde_json::to_value(&event).unwrap(),
                case["event"],
                "{name}"
            );
        }
        for case in vectors::cases(&file, "frames") {
            let name = case["name"].as_str().unwrap();
            let frame: SubscribeEvent = vectors::round_trip(name, &case["frame"]);
            assert_eq!(frame.seq(), case["seq"].as_u64(), "{name}");
            assert_eq!(
                frame.live_event().map(|e| e.type_name()),
                case["type"].as_str(),
                "{name}"
            );
        }
    }

    #[test]
    fn run_started_names_its_cause_or_none() {
        let file = vectors::load("live-events.json");
        for case in vectors::cases(&file, "run_causes") {
            let name = case["name"].as_str().unwrap();
            let LiveEvent::RunStarted(started) =
                LiveEvent::decode_strict(&case["event"]).unwrap_or_else(|e| panic!("{name}: {e}"))
            else {
                panic!("{name}: not a run_started event");
            };
            assert_eq!(
                started.send_id.as_deref(),
                case["send_id"].as_str(),
                "{name}"
            );
            // A send id is never empty: absence is how "no send" is said.
            assert_ne!(started.send_id.as_deref(), Some(""), "{name}");
            let mut built = RunStarted::new(&started.run_id);
            if let Some(send_id) = &started.send_id {
                built = built.with_send_id(send_id);
            }
            assert_eq!(
                serde_json::to_value(LiveEvent::RunStarted(built)).unwrap(),
                case["event"],
                "{name}"
            );
        }
    }

    #[test]
    fn an_unknown_or_malformed_event_reads_as_display_reset() {
        let file = vectors::load("live-events.json");
        for case in vectors::cases(&file, "reset_on_read") {
            let name = case["name"].as_str().unwrap();
            assert!(LiveEvent::decode_strict(&case["event"]).is_err(), "{name}");
            let read: LiveEvent = serde_json::from_value(case["event"].clone()).unwrap();
            assert_eq!(read, LiveEvent::DisplayReset, "{name}");
        }
        // Broca's deployed delta, without `mid` and `part`, still decodes.
        let deployed: LiveEvent = serde_json::from_value(serde_json::json!({
            "type": "text_delta", "message_id": "", "delta": "hi"
        }))
        .unwrap();
        assert!(matches!(
            deployed,
            LiveEvent::TextDelta(TextDelta {
                mid: None,
                part: None,
                ..
            })
        ));
    }

    #[test]
    fn subscribe_requests_with_limits_are_strict() {
        let file = vectors::load("live-events.json");
        for case in vectors::cases(&file, "requests") {
            let name = case["name"].as_str().unwrap();
            let request: LiveSubscribeRequest = vectors::round_trip(name, &case["request"]);
            assert_eq!(
                request.limits.is_some(),
                case["limits"].as_bool().unwrap(),
                "{name}"
            );
            // A runner without the group refuses `limits` by name.
            let base = serde_json::from_value::<SubscribeRequest>(case["request"].clone());
            assert_eq!(base.is_err(), request.limits.is_some(), "{name}");
            if let Err(error) = base {
                assert!(error.to_string().contains("limits"), "{name}: {error}");
            }
        }
        for case in vectors::cases(&file, "refused_requests") {
            let name = case["name"].as_str().unwrap();
            let error = serde_json::from_value::<LiveSubscribeRequest>(case["request"].clone())
                .expect_err(name);
            let field = case["field"].as_str().unwrap();
            assert!(error.to_string().contains(field), "{name}: {error}");
        }
        let cut = ToolStarted::new(
            "m1",
            "c1",
            "echo",
            &serde_json::json!({"t": "ééé"}),
            Some(&Limits::new(9, 0, 0)),
            5,
        );
        assert_eq!(
            (cut.arguments.as_str(), cut.arguments_truncated),
            ("{\"t\":\"é", true)
        );
    }
}
