use cortexkit_role_tool_provider::{call::ToolCallRequest, errors, stop_session::*, vectors};
use serde_json::{json, Value};

#[test]
fn stop_session_vectors_and_route_only_identity() {
    let vectors: Value = serde_json::from_str(vectors::STOP_SESSION_JSON).unwrap();
    let route = SessionKey::new("runner", "app", "session");
    assert_ne!(route, SessionKey::new("other-principal", "app", "session"));
    assert_ne!(route, SessionKey::new("runner", "other-app", "session"));
    for case in vectors["requests"].as_array().unwrap() {
        let request: ToolCallRequest = serde_json::from_value(case["request"].clone()).unwrap();
        let result = check_request(&request, case["bound"].as_bool().unwrap().then_some(&route));
        assert_eq!(result.is_ok(), case["valid"].as_bool().unwrap(), "{case}");
        if let Some(field) = case["field"].as_str() {
            assert_eq!(
                errors::invalid_request_field(&result.unwrap_err()),
                Some(field)
            );
        }
    }
    let reply: StopSessionReply = serde_json::from_value(vectors["reply"].clone()).unwrap();
    reply.validate().unwrap();
    assert_eq!(serde_json::to_value(reply).unwrap(), vectors["reply"]);
}

#[test]
fn stop_session_bounds_and_open_outcomes() {
    for (id, expected) in [
        (" ".to_owned(), true),
        ("x".repeat(128), true),
        ("x".repeat(129), false),
        ("é".into(), false),
        ("\u{7f}".into(), false),
    ] {
        assert_eq!(valid_request_id(&id), expected);
    }
    let item = StopItem::new("future_kind", "☃".repeat(120), "future_outcome");
    StopSessionReply::new(vec![item.clone(); 64])
        .with_more(1)
        .validate()
        .unwrap();
    assert!(StopSessionReply::new(vec![item; 65])
        .with_more(1)
        .validate()
        .is_err());
    assert!(
        StopSessionReply::new(vec![StopItem::new("call", "x".repeat(121), "stopped")])
            .validate()
            .is_err()
    );
    assert!(
        StopSessionReply::new(vec![StopItem::new("call", "work", "could_not_stop")])
            .validate()
            .is_err()
    );
    StopSessionReply::new(vec![
        StopItem::new("call", "work", "could_not_stop").with_reason("unreachable")
    ])
    .validate()
    .unwrap();
    assert_eq!(
        serde_json::to_value(StopSessionArguments::new("stop-1").into_request()).unwrap(),
        json!({"name":"tool.stop_session","arguments":{"request_id":"stop-1"}})
    );
}
