use cortexkit_role_compaction::{
    notice::{Event, NoticeAnswer},
    part::{Attachment, AttachmentItem, OutputFormat, Part, PartBody, Seal, ToolStatus, Transport},
    read::{Detail, Role, Select},
    ride::{RideAnswer, RideCause},
    rules::{EditCost, RuleBreak, SignedParts},
    setup::{ModelRef, SetupAnswer},
    view::{ApplyViewAnswer, EditOp, InsertAt, ViewRefusal},
    wire::{self, Refusal},
};
use serde_json::{json, Value};

#[test]
fn later_string_values_are_retained_verbatim() {
    macro_rules! check {
        ($ty:ident) => {
            let decoded: $ty = serde_json::from_value(json!("future_value")).unwrap();
            assert!(matches!(&decoded, $ty::Unknown(name) if name == "future_value"));
            assert_eq!(decoded.as_str(), "future_value");
            assert_eq!(serde_json::to_value(decoded).unwrap(), json!("future_value"));
        };
    }
    check!(RideCause);
    check!(Seal);
    check!(ToolStatus);
    check!(OutputFormat);
    check!(Transport);
    check!(Role);
    check!(SignedParts);
    check!(EditCost);
    let refusal: Refusal =
        serde_json::from_value(json!({"code":"future_value","message":"invented"})).unwrap();
    assert_eq!(refusal.code, "future_value");
}

#[test]
fn later_tagged_values_retain_their_name_and_payload() {
    macro_rules! check {
        ($ty:ident, $tag:literal) => {
            let raw = json!({$tag: "future_value", "payload": {"x": [1, true, "invented"]}});
            let decoded: $ty = serde_json::from_value(raw.clone()).unwrap();
            assert!(matches!(&decoded, $ty::Unknown { name, .. } if name == "future_value"));
            assert_eq!(serde_json::to_value(decoded).unwrap(), raw);
        };
    }
    check!(SetupAnswer, "answer");
    check!(NoticeAnswer, "answer");
    check!(RideAnswer, "answer");
    check!(ApplyViewAnswer, "answer");
    check!(Event, "event");
    check!(PartBody, "kind");
    check!(AttachmentItem, "kind");
    check!(Select, "by");
    check!(RuleBreak, "rule");
    check!(EditOp, "op");
    check!(ViewRefusal, "code");

    let part_json = json!({"id":"part-future","digest":"invented-checksum","tag":7,"kind":"future_value","payload":{"x":1}});
    let part: Part = serde_json::from_value(part_json.clone()).unwrap();
    assert!(matches!(&part.body, PartBody::Unknown { name, .. } if name == "future_value"));
    assert_eq!(serde_json::to_value(part.compact()).unwrap(), part_json);
    let attachment_json = json!({"id":"attachment-future","kind":"future_value","payload":{"x":1}});
    let attachment: Attachment = serde_json::from_value(attachment_json.clone()).unwrap();
    assert_eq!(serde_json::to_value(attachment).unwrap(), attachment_json);
}

#[test]
fn malformed_known_values_are_not_mistaken_for_later_variants() {
    macro_rules! rejects {
        ($ty:ty, $value:expr) => {
            assert!(
                serde_json::from_value::<$ty>($value).is_err(),
                stringify!($ty)
            );
        };
    }
    rejects!(PartBody, json!({"kind":"text"}));
    rejects!(AttachmentItem, json!({"kind":"file","mime":5}));
    rejects!(Event, json!({"event":"transcript_changed"}));
    rejects!(Select, json!({"by":"mids"}));
    rejects!(EditOp, json!({"op":"replace","mid":"message-a"}));
    rejects!(RuleBreak, json!({"rule":"signed_part_invalid"}));
    rejects!(ViewRefusal, json!({"code":"stale_version","version":1}));
    rejects!(RideAnswer, json!({"answer":"view"}));
    rejects!(ApplyViewAnswer, json!({"answer":"refused"}));
    rejects!(NoticeAnswer, json!({"answer":true}));
    rejects!(SetupAnswer, json!({}));
    rejects!(InsertAt, json!({"before":5}));
    rejects!(Detail, json!("future_value"));
    let refusal = wire::params::<PartBody>(json!({"kind":"text"})).unwrap_err();
    assert_eq!(refusal.code, "invalid_params");
}

#[test]
fn anchors_and_unknown_fields_are_forward_compatible() {
    for raw in [
        json!("future_anchor"),
        json!({"after":"message-a"}),
        json!({"after":"message-a","count":2}),
    ] {
        let decoded: InsertAt = serde_json::from_value(raw.clone()).unwrap();
        assert!(matches!(&decoded, InsertAt::Unknown(_)));
        assert_eq!(serde_json::to_value(decoded).unwrap(), raw);
    }
    // W1.4: a field added beside `before` is ignored, not a new anchor.
    let before: InsertAt =
        serde_json::from_value(json!({"before":"message-a","future_field":true})).unwrap();
    assert_eq!(before, InsertAt::Before("message-a".to_owned()));
    let model: ModelRef = wire::params(
        json!({"provider":"example-provider","model":"example-model","future_field":true}),
    )
    .unwrap();
    assert_eq!(model, ModelRef::new("example-provider", "example-model"));
    let event: Event =
        serde_json::from_value(json!({"event":"marker_written","version":1,"future_field":true}))
            .unwrap();
    assert_eq!(event, Event::MarkerWritten { version: 1 });
    let wire = wire::encode("example.method", &json!({}));
    let mut object: Value = serde_json::from_slice(&wire).unwrap();
    object["future_field"] = json!(true);
    assert_eq!(
        wire::decode(&serde_json::to_vec(&object).unwrap()).unwrap(),
        ("example.method".to_owned(), json!({}))
    );
}
