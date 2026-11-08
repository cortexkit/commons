use cortexkit_role_llm_runner::live::{LiveEvent, NoticeDelivered};
use serde_json::json;

#[test]
fn notice_delivered_forms_round_trip_and_omit_absent_form() {
    let event = NoticeDelivered::new("message-a", 0, "call-a", "send-a", "example-sender")
        .with_text("A reminder.");
    let legacy = json!({
        "type": "notice_delivered", "mid": "message-a", "part": 0,
        "call_key": "call-a", "send_id": "send-a", "sender": "example-sender",
        "text": "A reminder."
    });
    assert_eq!(
        serde_json::to_value(LiveEvent::NoticeDelivered(event.clone())).unwrap(),
        legacy
    );
    for form in ["turn_system", "future_form"] {
        let event = LiveEvent::NoticeDelivered(event.clone().with_delivered_as(form));
        let mut expected = legacy.clone();
        expected["delivered_as"] = json!(form);
        assert_eq!(serde_json::to_value(&event).unwrap(), expected);
        assert_eq!(LiveEvent::decode_strict(&expected).unwrap(), event);
    }
}

#[test]
fn notice_delivered_legacy_event_without_form_still_decodes() {
    let wire = json!({
        "type": "notice_delivered", "mid": "message-a", "part": 0,
        "call_key": "call-a", "send_id": "send-a", "sender": "example-sender",
        "text": "A reminder.", "future_member": true
    });
    let LiveEvent::NoticeDelivered(event) = LiveEvent::decode_strict(&wire).unwrap() else {
        panic!("legacy notice_delivered did not decode as its event type");
    };
    assert_eq!(event.delivered_as, None);
    assert_eq!(event.text, "A reminder.");
    assert!(serde_json::to_value(event)
        .unwrap()
        .get("delivered_as")
        .is_none());
}
