use cortexkit_role_llm_runner::{
    delivery_order::{self, Mode},
    errors,
    send::Delivery,
    session_change,
};
use serde_json::{json, Map, Value};

fn plan(value: Value) -> Map<String, Value> {
    value.as_object().unwrap().clone()
}

#[test]
fn accepted_pending_senders_replace_effective_admission_before_model_input_applies() {
    let effective = plan(json!({"senders": []}));
    let added = plan(json!({"senders": [{
        "module": "sender-b", "idle_start": true, "interrupt": true,
        "templates": [{"id": "notice", "text": "Ready", "slots": {}}]
    }]}));
    let before = delivery_order::effective_senders(&effective, None).unwrap();
    assert_eq!(
        delivery_order::admit("sender-b", Some("owner"), &before, &[Mode::IdleStart]),
        Err((errors::SCOPE_OWNER_MISMATCH, None))
    );
    let now = delivery_order::effective_senders(&effective, Some(&added)).unwrap();
    assert!(
        now.iter().any(|sender| sender.module == "sender-b"),
        "the latest accepted pending sender list must govern admission before model input applies"
    );
    assert_eq!(now[0].templates[0].text, "Ready");
    assert!(delivery_order::admit("sender-b", Some("owner"), &now, &[Mode::IdleStart]).is_ok());
    assert!(
        delivery_order::admit_delivery("sender-b", Some("owner"), &now, Delivery::Interrupt)
            .is_ok()
    );

    let narrowed = plan(json!({"senders": [{"module": "sender-b"}]}));
    let now = delivery_order::effective_senders(&effective, Some(&narrowed)).unwrap();
    assert!(now[0].templates.is_empty());
    for delivery in [Delivery::Queue, Delivery::Interrupt] {
        assert_eq!(
            delivery_order::admit_delivery("sender-b", Some("owner"), &now, delivery),
            Err((errors::INVALID_PARAMS, Some("delivery")))
        );
    }
    let removed = plan(json!({}));
    assert!(delivery_order::effective_senders(&added, Some(&removed))
        .unwrap()
        .is_empty());
    assert!(std::ptr::eq(
        session_change::admission_plan(&added, None),
        &added
    ));
    assert!(std::ptr::eq(
        session_change::admission_plan(&added, Some(&removed)),
        &removed
    ));
    // Applying the latest accepted refresh changes no admission policy.
    assert_eq!(
        delivery_order::effective_senders(&effective, Some(&narrowed)).unwrap(),
        delivery_order::effective_senders(&narrowed, None).unwrap()
    );
}
