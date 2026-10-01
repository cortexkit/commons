//! `session.subscribe`: the session's event stream, with the snapshot
//! handoff.
//!
//! A `session.read` page and its `head` come from one snapshot. A
//! subscription from that `head` continues strictly after it, with no gap
//! and no duplicate, so a consumer can read history and then follow the
//! session without missing or repeating an event.

use serde::{Deserialize, Deserializer, Serialize, Serializer};
use serde_json::{Map, Value};

/// Where a subscription attaches.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum SubscribeFrom {
    /// Replay every durable event from the start of the lineage.
    Start,
    /// Only events after the subscription attaches.
    Live,
    /// Strictly after a position a page's `head` (or an earlier event's
    /// `cursor`) returned, passed back verbatim.
    After(Map<String, Value>),
}

/// The spellings of the named attach points.
pub mod named {
    pub const START: &str = "start";
    pub const LIVE: &str = "live";
}

impl Serialize for SubscribeFrom {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        match self {
            Self::Start => serializer.serialize_str(named::START),
            Self::Live => serializer.serialize_str(named::LIVE),
            Self::After(position) => position.serialize(serializer),
        }
    }
}

impl<'de> Deserialize<'de> for SubscribeFrom {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        match Value::deserialize(deserializer)? {
            Value::String(name) if name == named::START => Ok(Self::Start),
            Value::String(name) if name == named::LIVE => Ok(Self::Live),
            Value::Object(position) => Ok(Self::After(position)),
            other => Err(serde::de::Error::custom(format!(
                "from must be \"start\", \"live\" or a head position object; got {other}"
            ))),
        }
    }
}

/// The `session.subscribe` request. Strict on unknown fields: an absent
/// `from` means "live", so a misspelled one must be refused rather than
/// silently attach live.
#[derive(Clone, Debug, Default, PartialEq, Eq, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct SubscribeRequest {
    /// Absent means [`SubscribeFrom::Live`].
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub from: Option<SubscribeFrom>,
}

impl SubscribeRequest {
    pub fn new(from: SubscribeFrom) -> Self {
        Self { from: Some(from) }
    }

    /// Attach strictly after `head`, a page's `head` passed back verbatim.
    pub fn after_head(head: &Map<String, Value>) -> Self {
        Self::new(SubscribeFrom::After(head.clone()))
    }

    /// The attach point the request selects.
    pub fn attach_point(&self) -> SubscribeFrom {
        self.from.clone().unwrap_or(SubscribeFrom::Live)
    }
}

/// One event on the stream. `kind` may be any string, and every field other
/// than `kind` and `cursor` is kept as received.
#[derive(Clone, Debug, PartialEq, Deserialize, Serialize)]
pub struct SubscribeEvent {
    /// The lane the event rides on. [`kinds::CONTROL`] events are durable
    /// and carry a `cursor`; [`kinds::DISPLAY`] events are best-effort.
    pub kind: String,
    /// The event's durable position, on durable events: a later subscription
    /// from it continues strictly after this event.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub cursor: Option<Map<String, Value>>,
    /// The rest of the event, in the runner's schema.
    #[serde(flatten)]
    pub body: Map<String, Value>,
}

/// Event lanes.
pub mod kinds {
    /// Durable events, replayable from a cursor.
    pub const CONTROL: &str = "control";
    /// Best-effort streaming events (deltas), never replayed.
    pub const DISPLAY: &str = "display";
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::vectors;

    #[test]
    fn subscribe_vectors_decode_and_round_trip() {
        let file = vectors::load("subscribe.json");
        for case in vectors::cases(&file, "requests") {
            let name = case["name"].as_str().unwrap();
            let request: SubscribeRequest = vectors::round_trip(name, &case["request"]);
            let attach = match request.attach_point() {
                SubscribeFrom::Start => "start",
                SubscribeFrom::Live => "live",
                SubscribeFrom::After(_) => "after",
            };
            assert_eq!(attach, case["attach"].as_str().unwrap(), "{name}");
        }
        for case in vectors::cases(&file, "refused_requests") {
            let name = case["name"].as_str().unwrap();
            let error = serde_json::from_value::<SubscribeRequest>(case["request"].clone())
                .expect_err(name);
            let field = case["field"].as_str().unwrap();
            assert!(error.to_string().contains(field), "{name}: {error}");
        }
        for case in vectors::cases(&file, "events") {
            let name = case["name"].as_str().unwrap();
            let event: SubscribeEvent = vectors::round_trip(name, &case["event"]);
            assert_eq!(
                event.cursor.is_some(),
                case["durable"].as_bool().unwrap(),
                "{name}"
            );
        }
    }

    #[test]
    fn a_page_head_becomes_an_after_request() {
        let head = serde_json::json!({"seq": 41, "sub": 0});
        let request = SubscribeRequest::after_head(head.as_object().unwrap());
        assert_eq!(
            serde_json::to_value(&request).unwrap(),
            serde_json::json!({"from": head})
        );
        assert_eq!(
            SubscribeRequest::default().attach_point(),
            SubscribeFrom::Live
        );
    }
}
