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

/// Why a provisional display id is not a valid `live_id`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub enum LiveIdError {
    /// Ids must contain between 1 and 128 bytes.
    Length,
    /// Every byte must be printable ASCII, excluding space (0x21–0x7E).
    Character,
}

impl std::fmt::Display for LiveIdError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(match self {
            Self::Length => "live_id must contain 1–128 bytes",
            Self::Character => "live_id must contain only ASCII bytes 0x21–0x7E",
        })
    }
}

impl std::error::Error for LiveIdError {}

/// Validate the wire form of an opaque, runner-minted provisional display id.
/// Session-wide uniqueness and separation from stored ids are the runner's
/// responsibility; a single id cannot establish those properties.
pub fn validate_live_id(live_id: &str) -> Result<(), LiveIdError> {
    if live_id.is_empty() || live_id.len() > 128 {
        return Err(LiveIdError::Length);
    }
    if !live_id.bytes().all(|byte| (0x21..=0x7e).contains(&byte)) {
        return Err(LiveIdError::Character);
    }
    Ok(())
}

/// The event types, as `type` spells them.
pub mod event_types {
    pub const SESSION_REFRESH_FAILED: &str = "session_refresh_failed";
    pub const MESSAGE_STARTED: &str = "message_started";
    pub const MESSAGE_FINISHED: &str = "message_finished";
    pub const TOOL_STARTED: &str = "tool_started";
    pub const TOOL_FINISHED: &str = "tool_finished";
    pub const NOTICE_DELIVERED: &str = "notice_delivered";
    #[deprecated(since = "0.8.11", note = "renamed to NOTICE_DELIVERED")]
    pub const RIDE_DELIVERED: &str = NOTICE_DELIVERED;
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
/// `tool_started.arguments` as they bound a display row, and, under the
/// separate `live_ids` group, the provisional-id opt-in (false by default).
/// Strict, like [`SubscribeRequest`]: a runner without the matching group
/// refuses `limits` or `live_ids` as an unknown field, so a caller sends each
/// only to a runner declaring that group.
///
/// Non-exhaustive: build one with [`LiveSubscribeRequest::new`],
/// [`LiveSubscribeRequest::with_limits`] and
/// [`LiveSubscribeRequest::with_live_ids`], or decode one.
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
    /// Opt in to provisional display ids. Send `true` only to a runner
    /// declaring `live_ids`: older runners refuse the unknown request field.
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub live_ids: bool,
}

impl LiveSubscribeRequest {
    pub fn new(from: SubscribeFrom) -> Self {
        Self {
            from: Some(from),
            limits: None,
            live_ids: false,
        }
    }

    pub fn with_limits(mut self, limits: Limits) -> Self {
        self.limits = Some(limits);
        self
    }

    pub fn with_live_ids(mut self, live_ids: bool) -> Self {
        self.live_ids = live_ids;
        self
    }

    /// The attach point the request selects.
    pub fn attach_point(&self) -> SubscribeFrom {
        self.from.clone().unwrap_or(SubscribeFrom::Live)
    }

    /// The base [`SubscribeRequest`]: only `from`, without either extension
    /// field.
    pub fn base(&self) -> SubscribeRequest {
        SubscribeRequest {
            from: self.from.clone(),
        }
    }
}

/// `message_started` and `message_finished`: the message's `mid`, the one
/// its stored entry carries. Only `message_started` may also name the
/// provisional `live_id` whose display words the reader rekeys to `mid`.
#[derive(Clone, Debug, PartialEq, Eq, Deserialize, Serialize)]
#[non_exhaustive]
pub struct MessageEvent {
    pub mid: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub live_id: Option<String>,
}

impl MessageEvent {
    pub fn new(mid: impl Into<String>) -> Self {
        Self {
            mid: mid.into(),
            live_id: None,
        }
    }

    /// Link a `message_started` to its pre-storage display words.
    pub fn with_live_id(mut self, live_id: impl Into<String>) -> Self {
        self.live_id = Some(live_id.into());
        self
    }
}

/// `tool_started`: a call of message `mid` went to its tool. `arguments` is
/// the call's arguments as JSON text, cut by the subscription's `limits`
/// like a display row's.
///
/// `started_at` is the time the runner recorded when the call started. It is
/// `None`, and left out of the event, only when the runner recorded no time
/// for the call, such as a call replayed from history stored before the
/// runner kept times. A runner never puts an invented time there (now, zero,
/// or the message's time), and a reader never makes one up for it.
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
    /// Milliseconds since the Unix epoch; `None` when no time was recorded.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub started_at: Option<u64>,
    /// As on the display row's tool call ([`crate::display::ToolCallPart::call_key`]),
    /// on a runner declaring `call_identity`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub call_key: Option<String>,
    /// As on the display row's tool call ([`crate::display::ToolCallPart::schema_pin`]),
    /// on a runner declaring `call_identity`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub schema_pin: Option<String>,
}

impl ToolStarted {
    /// `started_at` is the recorded start time, or `None` when the runner
    /// recorded none; never a substitute.
    pub fn new(
        mid: impl Into<String>,
        tool_call_id: impl Into<String>,
        name: impl Into<String>,
        arguments: &Value,
        limits: Option<&Limits>,
        started_at: Option<u64>,
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
            call_key: None,
            schema_pin: None,
        }
    }

    /// Name the call's key, once the call is prepared or dispatched.
    pub fn with_call_key(mut self, call_key: impl Into<String>) -> Self {
        self.call_key = Some(call_key.into());
        self
    }

    /// Name the pin of the definition the call was dispatched against.
    pub fn with_schema_pin(mut self, schema_pin: impl Into<String>) -> Self {
        self.schema_pin = Some(schema_pin.into());
        self
    }
}

/// `tool_finished`: the call of message `mid` ended with `outcome`
/// ([`tool_outcomes`]). `output_bytes` is the byte length of its result's
/// content uncut, as a display row's `tool_result.content` holds it.
///
/// `finished_at` follows [`ToolStarted::started_at`]'s rule: the recorded
/// time, or `None` and left out when the runner recorded none.
#[derive(Clone, Debug, PartialEq, Eq, Deserialize, Serialize)]
#[non_exhaustive]
pub struct ToolFinished {
    pub mid: String,
    pub tool_call_id: String,
    pub outcome: String,
    pub output_bytes: u64,
    /// Milliseconds since the Unix epoch; `None` when no time was recorded.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub finished_at: Option<u64>,
    /// As on [`ToolStarted::call_key`].
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub call_key: Option<String>,
    /// As on [`ToolStarted::schema_pin`].
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub schema_pin: Option<String>,
}

impl ToolFinished {
    /// `finished_at` is the recorded finish time, or `None` when the runner
    /// recorded none; never a substitute.
    pub fn new(
        mid: impl Into<String>,
        tool_call_id: impl Into<String>,
        outcome: impl Into<String>,
        output_bytes: u64,
        finished_at: Option<u64>,
    ) -> Self {
        Self {
            mid: mid.into(),
            tool_call_id: tool_call_id.into(),
            outcome: outcome.into(),
            output_bytes,
            finished_at,
            call_key: None,
            schema_pin: None,
        }
    }

    /// Name the call's key, once the call is prepared or dispatched.
    pub fn with_call_key(mut self, call_key: impl Into<String>) -> Self {
        self.call_key = Some(call_key.into());
        self
    }

    /// Name the pin of the definition the call was dispatched against.
    pub fn with_schema_pin(mut self, schema_pin: impl Into<String>) -> Self {
        self.schema_pin = Some(schema_pin.into());
        self
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
    /// On a runner declaring `rich_ui_stamp`: the rich UI stamp the run's
    /// frozen plan carries, which every assistant row the run writes repeats
    /// (CONTRACT §9.17). Absent there means the run is unstamped; on any
    /// other runner absence says nothing.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub rich_ui: Option<crate::rich_ui::RichUiStamp>,
}

impl RunStarted {
    pub fn new(run_id: impl Into<String>) -> Self {
        Self {
            run_id: run_id.into(),
            send_id: None,
            rich_ui: None,
        }
    }

    /// Name the rich UI stamp the run's frozen plan carries.
    pub fn with_rich_ui(mut self, rich_ui: crate::rich_ui::RichUiStamp) -> Self {
        self.rich_ui = Some(rich_ui);
        self
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
/// provider's id and may be empty. A `live_events` runner sends exactly one
/// of the stored `mid` or (with `live_ids` opt-in) a provisional `live_id`.
/// `part` is the key the stored row gives the part. A runner without
/// `live_events` can omit these keys, so they decode as optional.
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
    pub live_id: Option<String>,
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
            live_id: None,
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
    pub live_id: Option<String>,
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
            live_id: None,
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
    SessionRefreshFailed(SessionRefreshFailed),
    MessageStarted(MessageEvent),
    MessageFinished(MessageEvent),
    ToolStarted(ToolStarted),
    ToolFinished(ToolFinished),
    NoticeDelivered(NoticeDelivered),
    RunStarted(RunStarted),
    RunFinished(RunEnded),
    RunPaused(RunEnded),
    TextDelta(TextDelta),
    ReasoningDelta(TextDelta),
    ToolInputDelta(ToolInputDelta),
    DisplayGap(DisplayGap),
    /// Drop all live state for the step in flight. If the step continues or
    /// is rebuilt, its current deltas stream again; after an unstored message
    /// is abandoned, none follow.
    DisplayReset,
}

impl LiveEvent {
    /// Decode `value` strictly: an unknown `type`, or members that do not
    /// decode for a known one, is an error naming the problem.
    pub fn decode_strict(value: &Value) -> Result<Self, String> {
        let event = Self::decode_unvalidated(value)?;
        let (mid, live_id) = match &event {
            Self::MessageStarted(message) => (None, message.live_id.as_deref()),
            Self::MessageFinished(message) if message.live_id.is_some() => {
                return Err("message_finished must not carry live_id".into());
            }
            Self::TextDelta(delta) | Self::ReasoningDelta(delta) => {
                (delta.mid.as_deref(), delta.live_id.as_deref())
            }
            Self::ToolInputDelta(delta) => (delta.mid.as_deref(), delta.live_id.as_deref()),
            _ => (None, None),
        };
        if let Some(live_id) = live_id {
            validate_live_id(live_id).map_err(|e| format!("{}: {e}", event.type_name()))?;
            if mid.is_some() {
                return Err(format!(
                    "{}: mid and live_id are mutually exclusive",
                    event.type_name()
                ));
            }
        }
        Ok(event)
    }

    fn decode_unvalidated(value: &Value) -> Result<Self, String> {
        use event_types as t;
        let kind = value
            .get("type")
            .and_then(Value::as_str)
            .ok_or("an event without a type")?;
        fn typed<T: serde::de::DeserializeOwned>(kind: &str, value: &Value) -> Result<T, String> {
            serde_json::from_value(value.clone()).map_err(|e| format!("{kind}: {e}"))
        }
        Ok(match kind {
            t::SESSION_REFRESH_FAILED => Self::SessionRefreshFailed(typed(kind, value)?),
            t::MESSAGE_STARTED => Self::MessageStarted(typed(kind, value)?),
            t::MESSAGE_FINISHED => Self::MessageFinished(typed(kind, value)?),
            t::TOOL_STARTED => Self::ToolStarted(typed(kind, value)?),
            t::TOOL_FINISHED => Self::ToolFinished(typed(kind, value)?),
            t::NOTICE_DELIVERED => Self::NoticeDelivered(typed(kind, value)?),
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
            Self::SessionRefreshFailed(_) => t::SESSION_REFRESH_FAILED,
            Self::MessageStarted(_) => t::MESSAGE_STARTED,
            Self::MessageFinished(_) => t::MESSAGE_FINISHED,
            Self::ToolStarted(_) => t::TOOL_STARTED,
            Self::ToolFinished(_) => t::TOOL_FINISHED,
            Self::NoticeDelivered(_) => t::NOTICE_DELIVERED,
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

/// A notice became durable beside an existing tool-result part. This event
/// creates no transcript entry and does not change `history_version`.
#[derive(Clone, Debug, PartialEq, Eq, Deserialize, Serialize)]
#[non_exhaustive]
pub struct NoticeDelivered {
    pub mid: String,
    pub part: u32,
    pub call_key: String,
    pub send_id: String,
    pub sender: String,
    pub text: String,
}

impl NoticeDelivered {
    /// Construct a delivery event. Set the notice text with [`Self::with_text`]
    /// before serving it.
    pub fn new(
        mid: impl Into<String>,
        part: u32,
        call_key: impl Into<String>,
        send_id: impl Into<String>,
        sender: impl Into<String>,
    ) -> Self {
        Self {
            mid: mid.into(),
            part,
            call_key: call_key.into(),
            send_id: send_id.into(),
            sender: sender.into(),
            text: String::new(),
        }
    }

    pub fn with_text(mut self, text: impl Into<String>) -> Self {
        self.text = text.into();
        self
    }
}

#[deprecated(since = "0.8.11", note = "renamed to NoticeDelivered")]
pub type RideDelivered = NoticeDelivered;

/// Owner-only durable control notice; `session.baseline.pending_failure`
/// remains the recovery record when the owner misses the live notice.
#[derive(Clone, Debug, PartialEq, Eq, Deserialize, Serialize)]
#[non_exhaustive]
pub struct SessionRefreshFailed {
    pub session: String,
    pub generation: u64,
    pub reason: String,
}

impl SessionRefreshFailed {
    pub fn new(session: impl Into<String>, generation: u64, reason: impl Into<String>) -> Self {
        Self {
            session: session.into(),
            generation,
            reason: reason.into(),
        }
    }
}

impl<'de> Deserialize<'de> for LiveEvent {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        let value = Value::deserialize(deserializer)?;
        Ok(Self::decode_unvalidated(&value).unwrap_or(Self::DisplayReset))
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
    fn live_ids_absent_preserve_wire_bytes() {
        assert_eq!(
            serde_json::to_string(&LiveEvent::TextDelta(TextDelta::new("", "hi", "m1", "p1")))
                .unwrap(),
            r#"{"type":"text_delta","message_id":"","delta":"hi","mid":"m1","part":"p1"}"#
        );
        assert_eq!(
            serde_json::to_string(&LiveEvent::MessageStarted(MessageEvent::new("m1"))).unwrap(),
            r#"{"type":"message_started","mid":"m1"}"#
        );
        assert_eq!(
            serde_json::to_string(
                &LiveSubscribeRequest::new(SubscribeFrom::Start).with_live_ids(false)
            )
            .unwrap(),
            r#"{"from":"start"}"#
        );
        let request: LiveSubscribeRequest = serde_json::from_str("{}").unwrap();
        assert!(!request.live_ids);
        assert_eq!(
            serde_json::to_string(&request.with_live_ids(true)).unwrap(),
            r#"{"live_ids":true}"#
        );
    }

    #[test]
    fn live_ids_validate_form_and_strict_events_only() {
        assert_eq!(validate_live_id(""), Err(LiveIdError::Length));
        assert_eq!(validate_live_id(&"x".repeat(129)), Err(LiveIdError::Length));
        for id in [" ", "a\nb", "\x7f", "é"] {
            assert_eq!(validate_live_id(id), Err(LiveIdError::Character));
        }
        for id in ["!".to_owned(), "~".repeat(128)] {
            assert_eq!(validate_live_id(&id), Ok(()));
        }
        for kind in [
            "text_delta",
            "reasoning_delta",
            "tool_input_delta",
            "message_started",
        ] {
            let mut wire = serde_json::json!({"type":kind, "live_id":"live-1", "delta":"hi", "tool_call_id":"c1", "part":"p1"});
            if kind == "message_started" {
                wire["mid"] = "m1".into();
            }
            let decoded = LiveEvent::decode_strict(&wire).unwrap();
            assert_eq!(serde_json::to_value(decoded).unwrap()["live_id"], "live-1");
            wire["live_id"] = "bad id".into();
            assert!(LiveEvent::decode_strict(&wire)
                .unwrap_err()
                .contains("live_id"));
            // Lenient readers still decode known shapes without validating ids.
            let lenient: LiveEvent = serde_json::from_value(wire).unwrap();
            assert_ne!(lenient, LiveEvent::DisplayReset);
        }
        assert!(LiveEvent::decode_strict(&serde_json::json!({
            "type":"text_delta", "delta":"hi", "mid":"m1", "live_id":"live-1"
        }))
        .is_err());
        assert!(LiveEvent::decode_strict(&serde_json::json!({
            "type":"message_finished", "mid":"m1", "live_id":"live-1"
        }))
        .is_err());
    }

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

    /// Each `tool_times` case names a `tool_started` and a `tool_finished`
    /// vector and the times a reader gets from them: the recorded number, or
    /// `None` when the event leaves the member out. The bytes themselves are
    /// pinned by the `events` round trip above.
    #[test]
    fn tool_times_are_recorded_or_left_out() {
        let file = vectors::load("live-events.json");
        let events = vectors::cases(&file, "events");
        let event = |name: &str| {
            let case = events
                .iter()
                .find(|c| c["name"] == name)
                .unwrap_or_else(|| panic!("no event vector named {name}"));
            LiveEvent::decode_strict(&case["event"]).unwrap_or_else(|e| panic!("{name}: {e}"))
        };
        for case in vectors::cases(&file, "tool_times") {
            let name = case["name"].as_str().unwrap();
            let LiveEvent::ToolStarted(started) = event(case["started"].as_str().unwrap()) else {
                panic!("{name}: not a tool_started event");
            };
            let LiveEvent::ToolFinished(finished) = event(case["finished"].as_str().unwrap())
            else {
                panic!("{name}: not a tool_finished event");
            };
            assert_eq!(started.started_at, case["started_at"].as_u64(), "{name}");
            assert_eq!(finished.finished_at, case["finished_at"].as_u64(), "{name}");
            let arguments: Value = serde_json::from_str(&started.arguments).unwrap();
            let built = ToolStarted::new(
                &started.mid,
                &started.tool_call_id,
                &started.name,
                &arguments,
                None,
                case["started_at"].as_u64(),
            );
            assert_eq!(built, started, "{name}");
            let built = ToolFinished::new(
                &finished.mid,
                &finished.tool_call_id,
                &finished.outcome,
                finished.output_bytes,
                case["finished_at"].as_u64(),
            );
            assert_eq!(built, finished, "{name}");
        }
    }

    /// Each `call_identity` event decodes strictly to the members its case
    /// names and round-trips, and the setters rebuild it from the event
    /// without them; an event without them keeps the bytes it had before
    /// the group.
    #[test]
    fn call_identity_events_carry_their_members_or_leave_them_out() {
        let file = vectors::load("call-identity.json");
        for case in vectors::cases(&file, "events") {
            let name = case["name"].as_str().unwrap();
            let event =
                LiveEvent::decode_strict(&case["event"]).unwrap_or_else(|e| panic!("{name}: {e}"));
            assert_eq!(
                serde_json::to_value(&event).unwrap(),
                case["event"],
                "{name}"
            );
            let (call_key, schema_pin, rebuilt) = match &event {
                LiveEvent::ToolStarted(started) => {
                    let arguments: Value = serde_json::from_str(&started.arguments).unwrap();
                    let mut built = ToolStarted::new(
                        &started.mid,
                        &started.tool_call_id,
                        &started.name,
                        &arguments,
                        None,
                        started.started_at,
                    );
                    if let Some(key) = &started.call_key {
                        built = built.with_call_key(key);
                    }
                    if let Some(pin) = &started.schema_pin {
                        built = built.with_schema_pin(pin);
                    }
                    (
                        &started.call_key,
                        &started.schema_pin,
                        LiveEvent::ToolStarted(built),
                    )
                }
                LiveEvent::ToolFinished(finished) => {
                    let mut built = ToolFinished::new(
                        &finished.mid,
                        &finished.tool_call_id,
                        &finished.outcome,
                        finished.output_bytes,
                        finished.finished_at,
                    );
                    if let Some(key) = &finished.call_key {
                        built = built.with_call_key(key);
                    }
                    if let Some(pin) = &finished.schema_pin {
                        built = built.with_schema_pin(pin);
                    }
                    (
                        &finished.call_key,
                        &finished.schema_pin,
                        LiveEvent::ToolFinished(built),
                    )
                }
                other => panic!("{name}: not a tool event: {other:?}"),
            };
            assert_eq!(call_key.as_deref(), case["call_key"].as_str(), "{name}");
            assert_eq!(schema_pin.as_deref(), case["schema_pin"].as_str(), "{name}");
            assert_eq!(rebuilt, event, "{name}");
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
        // A legacy delta, without `mid` and `part`, still decodes.
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
            Some(5),
        );
        assert_eq!(
            (cut.arguments.as_str(), cut.arguments_truncated),
            ("{\"t\":\"é", true)
        );
    }
}

#[cfg(test)]
mod refresh_failure_tests {
    use super::*;

    #[test]
    fn session_refresh_failed_control_event_round_trips() {
        let event = LiveEvent::SessionRefreshFailed(SessionRefreshFailed::new(
            "session-a",
            4,
            "host_start_failed",
        ));
        let wire = serde_json::json!({"type":"session_refresh_failed", "session":"session-a",
            "generation":4, "reason":"host_start_failed"});
        assert_eq!(serde_json::to_value(&event).unwrap(), wire);
        assert_eq!(LiveEvent::decode_strict(&wire).unwrap(), event);
        assert_eq!(event.type_name(), event_types::SESSION_REFRESH_FAILED);
        assert_eq!(serde_json::from_value::<LiveEvent>(wire).unwrap(), event);
        assert!(LiveEvent::decode_strict(&serde_json::json!({
            "type":"session_refresh_failed", "session":"session-a", "generation":4
        }))
        .is_err());
    }
}
