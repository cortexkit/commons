use cortexkit_role_tool_provider::{
    consumer_reads_held_update_v1,
    late_results::{AckRequest, EntryKind, LateEntry, LateResultsReply},
    result::{is_person_wait, HeldReply, ToolResult},
    vectors, HELD_UPDATE_CAPABILITY,
};
use serde::{Deserialize, Serialize};
use serde_json::{json, Map, Value};
use std::collections::HashSet;

fn vectors() -> Value {
    serde_json::from_str(vectors::HELD_UPDATE_V1_JSON).unwrap()
}

fn answer<'a>(entries: &'a [LateEntry], call_key: &str) -> Option<&'a LateEntry> {
    entries.iter().find(|entry| {
        entry.call_key == call_key
            && matches!(
                entry.kind(),
                EntryKind::Result | EntryKind::NotStarted | EntryKind::Expired
            )
    })
}

#[test]
fn held_updates_do_not_answer_the_call() {
    let vectors = vectors();
    let sequence = &vectors["sequence"];
    let first: ToolResult = serde_json::from_value(sequence["first_reply"].clone()).unwrap();
    assert_eq!(first.held().unwrap().reason, "consent");
    let reply: LateResultsReply = serde_json::from_value(sequence["reply"].clone()).unwrap();
    assert_eq!(reply.entries.len(), 3);
    assert_eq!(reply.entries[0].kind(), EntryKind::Held);
    assert_eq!(reply.entries[1].kind(), EntryKind::Held);
    assert_eq!(reply.entries[2].kind(), EntryKind::Result);
    let call_key = &reply.entries[0].call_key;
    assert!(reply
        .entries
        .iter()
        .all(|entry| &entry.call_key == call_key));
    assert!(answer(&reply.entries[..2], call_key).is_none());
    assert_eq!(
        answer(&reply.entries, call_key).unwrap().event_id,
        sequence["answer_event_id"]
    );
    assert!(answer(&reply.entries, "different-call").is_none());
    let ids: HashSet<_> = reply.entries.iter().map(|entry| &entry.event_id).collect();
    assert_eq!(ids.len(), 3);

    for (index, entry) in reply.entries[..2].iter().enumerate() {
        let held = entry.held().unwrap().unwrap();
        assert_eq!(is_person_wait(&held.reason), sequence["person_wait"][index]);
        assert_eq!(entry.settled_at, sequence["effective_at"][index]);
        assert!(entry.result.is_none());
        assert!(entry.outcome.is_none());
        assert!(!entry.reduced);
        assert!(entry.validate().is_ok());
    }
    assert!(reply.entries[0].held().unwrap().unwrap().detail.is_some());
    assert!(reply.entries[1].held().unwrap().unwrap().detail.is_none());
    assert!(reply.entries[2].held().unwrap().is_none());
    assert_eq!(serde_json::to_value(&reply).unwrap(), sequence["reply"]);

    let replay: LateResultsReply = serde_json::from_value(sequence["reply"].clone()).unwrap();
    for (original, replayed) in reply.entries.iter().zip(&replay.entries) {
        assert_eq!(
            original.intake_key("example:provider"),
            replayed.intake_key("example:provider")
        );
    }
    let ack: AckRequest = serde_json::from_value(sequence["ack"].clone()).unwrap();
    assert_eq!(ack.through, reply.cursor);
}

#[test]
fn every_held_update_vector_is_validated() {
    let vectors = vectors();
    let valid = vectors["valid"].as_array().unwrap();
    let invalid = vectors["invalid"].as_array().unwrap();
    assert_eq!(valid.len(), 2);
    assert_eq!(invalid.len(), 12);
    for case in valid {
        let entry: LateEntry = serde_json::from_value(case["entry"].clone()).unwrap();
        assert_eq!(entry.kind(), EntryKind::Held, "{}", case["name"]);
        assert!(entry.validate().is_ok(), "{}", case["name"]);
        assert_eq!(serde_json::to_value(&entry).unwrap(), case["entry"]);
    }
    let unknown: LateEntry = serde_json::from_value(valid[0]["entry"].clone()).unwrap();
    assert_eq!(unknown.held().unwrap().unwrap().reason, "provider_step_v2");
    assert!(!is_person_wait(&unknown.held().unwrap().unwrap().reason));
    for case in invalid {
        assert!(
            serde_json::from_value::<LateEntry>(case["entry"].clone()).is_err(),
            "{}",
            case["name"]
        );
    }
}

#[test]
fn held_update_gate_requires_all_three_bind_declarations_and_a_held_first_reply() {
    assert_eq!(HELD_UPDATE_CAPABILITY, "held-update/v1");
    let vectors = vectors();
    let cases = vectors["gating"].as_array().unwrap();
    assert_eq!(cases.len(), 12);
    for case in cases {
        let caps: Option<Vec<String>> =
            serde_json::from_value(case["consumer_capabilities"].clone()).unwrap();
        let may_send = case["first_reply_held"].as_bool().unwrap()
            && consumer_reads_held_update_v1(caps.as_deref());
        assert_eq!(may_send, case["may_send"], "{}", case["name"]);
    }
}

#[test]
fn cursor_order_can_return_to_person_wait_and_any_terminal_kind_ends_the_call() {
    let vectors = vectors();
    let mut reply: LateResultsReply =
        serde_json::from_value(vectors["sequence"]["reply"].clone()).unwrap();
    let consent_again: LateEntry =
        serde_json::from_value(vectors["valid"][1]["entry"].clone()).unwrap();
    reply.entries.insert(2, consent_again);
    let latest_person_wait: Vec<_> = reply.entries[..3]
        .iter()
        .map(|entry| is_person_wait(&entry.held().unwrap().unwrap().reason))
        .collect();
    assert_eq!(latest_person_wait, [true, false, true]);
    assert!(answer(&reply.entries[..3], "build:7").is_none());
    for terminal in ["result", "not_started", "expired"] {
        reply.entries[3].kind = terminal.into();
        assert_eq!(answer(&reply.entries, "build:7"), Some(&reply.entries[3]));
    }
}

#[test]
fn held_constructor_accessor_and_validation_share_the_reply_rules() {
    let detail = json!({"opaque": {"nested": [true, 42, null]}});
    let held = HeldReply::new("consent").with_detail(detail.as_object().unwrap().clone());
    let entry = LateEntry::new_held(
        "example:owner",
        "s",
        1,
        "example:runner",
        "k",
        "e",
        1,
        held.clone(),
    )
    .with_invocation_id("onward-1");
    assert_eq!(entry.held().unwrap(), Some(held));
    assert!(entry.validate().is_ok());
    assert_eq!(
        serde_json::from_value::<LateEntry>(serde_json::to_value(&entry).unwrap()).unwrap(),
        entry
    );
    for reason in ["", "Bad", "bad-code", "two__words", "9reason", "échec"] {
        let mut malformed = entry.clone();
        malformed
            .extra
            .insert("held".into(), json!({"reason": reason}));
        assert!(malformed.validate().is_err(), "{reason}");
    }
    for detail in [
        json!(null),
        json!(true),
        json!(42),
        json!("text"),
        json!([]),
    ] {
        let mut malformed = entry.clone();
        malformed.extra.insert(
            "held".into(),
            json!({"reason": "consent", "detail": detail}),
        );
        assert!(malformed.validate().is_err(), "{detail}");
    }
    for malformed in [
        entry.clone().with_result(json!(null)),
        entry.clone().with_outcome("ok"),
        entry.clone().with_reduced(true),
        entry.clone().with_extra(Map::new()),
    ] {
        assert!(malformed.validate().is_err());
    }
    let value = serde_json::to_value(&entry).unwrap();
    for member in [
        "owner",
        "ref",
        "scope_epoch",
        "custodian",
        "call_key",
        "event_id",
        "settled_at",
    ] {
        let mut malformed = value.clone();
        malformed.as_object_mut().unwrap().remove(member);
        assert!(
            serde_json::from_value::<LateEntry>(malformed).is_err(),
            "{member}"
        );
    }
}

// Frozen 0.13.2 wire shape and kind dispatch. In particular, held is an
// uninterpreted extension member, not a typed field in the older decoder.
#[derive(Deserialize, Serialize)]
struct Entry0132 {
    kind: String,
    owner: String,
    #[serde(rename = "ref")]
    scope_ref: String,
    scope_epoch: u64,
    custodian: String,
    call_key: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    invocation_id: Option<String>,
    event_id: String,
    settled_at: u64,
    #[serde(default)]
    reduced: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    result: Option<Value>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    outcome: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    reason: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    text: Option<String>,
    #[serde(flatten)]
    extra: Map<String, Value>,
}

#[derive(Debug, PartialEq)]
enum Kind0132 {
    Result,
    Progress,
    NotStarted,
    Expired,
    Unknown(String),
}

impl Entry0132 {
    fn kind(&self) -> Kind0132 {
        match self.kind.as_str() {
            "result" => Kind0132::Result,
            "progress" => Kind0132::Progress,
            "not_started" => Kind0132::NotStarted,
            "expired" => Kind0132::Expired,
            other => Kind0132::Unknown(other.to_owned()),
        }
    }
}

#[test]
fn an_older_consumer_decodes_held_as_unknown_and_keeps_the_payload() {
    let vectors = vectors();
    let entries = vectors["sequence"]["reply"]["entries"]
        .as_array()
        .unwrap()
        .iter()
        .filter(|entry| entry["kind"] == "held")
        .chain(
            vectors["valid"]
                .as_array()
                .unwrap()
                .iter()
                .map(|case| &case["entry"]),
        );
    for value in entries {
        let old: Entry0132 = serde_json::from_value(value.clone()).unwrap();
        assert_eq!(old.kind(), Kind0132::Unknown("held".into()));
        assert_eq!(old.extra["held"], value["held"]);
        assert_eq!(serde_json::to_value(old).unwrap(), *value);
    }
}

#[test]
fn other_kinds_still_decode_like_0132_even_with_a_held_extension() {
    let vectors: Value = serde_json::from_str(vectors::LATE_RESULTS_JSON).unwrap();
    let entries = vectors["reply"]["entries"]
        .as_array()
        .unwrap()
        .iter()
        .chain(
            vectors["unknown_kinds"]
                .as_array()
                .unwrap()
                .iter()
                .map(|case| &case["entry"]),
        );
    for value in entries {
        let mut value = value.clone();
        value["held"] = json!({"reason": "bad-reason", "detail": null});
        let old: Entry0132 = serde_json::from_value(value.clone()).unwrap();
        let current: LateEntry = serde_json::from_value(value.clone()).unwrap();
        assert_eq!(serde_json::to_value(old).unwrap(), value);
        assert_eq!(serde_json::to_value(&current).unwrap(), value);
        assert!(current.held().unwrap().is_none());
        assert!(current.validate().is_ok());
    }
    let duplicate = r#"{"kind":"result","owner":"example:owner","ref":"s","scope_epoch":1,"custodian":"example:runner","call_key":"k","event_id":"e","settled_at":1,"result":null,"result":{}}"#;
    assert!(serde_json::from_str::<Entry0132>(duplicate).is_err());
    assert!(serde_json::from_str::<LateEntry>(duplicate).is_err());
}
