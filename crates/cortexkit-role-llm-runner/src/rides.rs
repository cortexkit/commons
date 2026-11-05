//! Rendering for sends delivered alongside a tool result.
//!
//! A ride is a runner record, not a transcript entry. Its carrying result is
//! identified by `call_key`; the host's stored result is never modified.

/// A send delivered with a carrying tool result, in delivery order.
#[derive(Clone, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub struct Ride {
    pub send_id: String,
    pub sender: String,
    pub call_key: String,
    pub text: String,
}

impl Ride {
    pub fn new(
        send_id: impl Into<String>,
        sender: impl Into<String>,
        call_key: impl Into<String>,
        text: impl Into<String>,
    ) -> Self {
        Self {
            send_id: send_id.into(),
            sender: sender.into(),
            call_key: call_key.into(),
            text: text.into(),
        }
    }
}

/// Append each ride's text, prefixed by exactly two newlines, to the output.
///
/// Neither output nor ride text is trimmed, escaped, or otherwise rewritten.
/// Callers supply only rides whose carrying output renders in the applied view.
/// System-message rendering uses the existing provider `turn_system` rules,
/// not this helper.
pub fn append(output: &str, rides: &[Ride]) -> String {
    let mut rendered = output.to_owned();
    for ride in rides {
        rendered.push_str("\n\n");
        rendered.push_str(&ride.text);
    }
    rendered
}

/// A tool-result content item: text, or an unchanged host-specific item.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ContentItem<T> {
    Text(String),
    Opaque(T),
}

/// Append rides to a result's content list without moving non-text items.
///
/// If the last item is text, append exactly `"\n\n"` plus each ride's text
/// to it, in delivery order. Otherwise push one text item containing those
/// same bytes, including the leading two newlines. An empty ride list changes
/// nothing. A single text value or an error string is the one-text-item case.
pub fn append_to_content<T>(content: &mut Vec<ContentItem<T>>, rides: &[Ride]) {
    if rides.is_empty() {
        return;
    }
    if let Some(ContentItem::Text(text)) = content.last_mut() {
        *text = append(text, rides);
    } else {
        content.push(ContentItem::Text(append("", rides)));
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::Value;

    fn rides(case: &Value) -> Vec<Ride> {
        case["rides"]
            .as_array()
            .unwrap()
            .iter()
            .map(|text| Ride::new("send", "sender", "call", text.as_str().unwrap()))
            .collect()
    }

    fn content(value: &Value) -> Vec<ContentItem<Value>> {
        value
            .as_array()
            .unwrap()
            .iter()
            .map(|item| match item.get("text").and_then(Value::as_str) {
                Some(text) => ContentItem::Text(text.to_owned()),
                None => ContentItem::Opaque(item.clone()),
            })
            .collect()
    }

    #[test]
    fn rides_append_bytes_vectors() {
        let vectors: Value = serde_json::from_str(include_str!(
            "../test-vectors/llm-runner-v1/result-rides.json"
        ))
        .unwrap();
        for case in vectors["strings"].as_array().unwrap() {
            assert_eq!(
                append(case["output"].as_str().unwrap(), &rides(case)),
                case["expected"].as_str().unwrap(),
                "{}",
                case["name"]
            );
        }
        for case in vectors["content"].as_array().unwrap() {
            let mut actual = content(&case["input"]);
            append_to_content(&mut actual, &rides(case));
            assert_eq!(actual, content(&case["expected"]), "{}", case["name"]);
        }
    }

    #[test]
    fn notices_reader_fact_and_live_event_round_trip() {
        let fact = serde_json::json!({
            "part": 2, "call_key": "call-1", "send_id": "send-1", "sender": "example-notifier",
            "text": "note", "delivered_as": "tool_result"
        });
        let decoded: crate::read::NoticeFact = serde_json::from_value(fact.clone()).unwrap();
        assert_eq!(decoded.part, 2);
        assert_eq!(
            decoded,
            crate::read::NoticeFact::new(2, "send-1", "example-notifier", "note")
                .with_call_key("call-1")
        );
        assert_eq!(serde_json::to_value(decoded).unwrap(), fact);
        let event = serde_json::json!({
            "type": "notice_delivered", "mid": "message-1", "part": 2,
            "call_key": "call-1", "send_id": "send-1", "sender": "example-notifier", "text": "note"
        });
        let decoded = crate::live::LiveEvent::decode_strict(&event).unwrap();
        assert_eq!(decoded.type_name(), "notice_delivered");
        assert_eq!(
            decoded,
            crate::live::LiveEvent::NoticeDelivered(
                crate::live::NoticeDelivered::new(
                    "message-1",
                    2,
                    "call-1",
                    "send-1",
                    "example-notifier"
                )
                .with_text("note")
            )
        );
        assert_eq!(serde_json::to_value(decoded).unwrap(), event);
    }

    #[test]
    fn notices_require_new_fields_but_delivery_mode_is_open() {
        let mut fact = serde_json::json!({"part":0,"call_key":"call-1","send_id":"send-1",
            "sender":"example-notifier","text":"note","delivered_as":"future_mode"});
        let decoded: crate::read::NoticeFact = serde_json::from_value(fact.clone()).unwrap();
        assert_eq!(decoded.delivered_as, "future_mode");
        for field in ["call_key", "delivered_as"] {
            let mut missing = fact.clone();
            missing.as_object_mut().unwrap().remove(field);
            assert!(serde_json::from_value::<crate::read::NoticeFact>(missing).is_err());
        }
        fact["type"] = "notice_delivered".into();
        fact["mid"] = "message-1".into();
        fact.as_object_mut().unwrap().remove("text");
        assert!(crate::live::LiveEvent::decode_strict(&fact).is_err());
        fact["text"] = "note".into();
        fact["type"] = "ride_delivered".into();
        assert!(crate::live::LiveEvent::decode_strict(&fact).is_err());
    }
}
