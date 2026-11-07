use cortexkit_role_llm_runner::{delivery_order::PlanSender, stop_all::*, vectors};
use serde_json::{json, Value};

#[test]
fn stop_all_dependency_is_refused_by_name() {
    use cortexkit_role_llm_runner::{
        describe::{check_describe, DescribeProblem},
        PROVIDES, REQUIRED_OPS,
    };
    let raw = json!({"majors":[{"version":PROVIDES,"ops":REQUIRED_OPS,"stability":"stable"}],"implementation_version":"test","capabilities":["stop_all"],"session_capabilities_from":"baseline"});
    assert!(check_describe(&raw)
        .unwrap_err()
        .contains(&DescribeProblem::Undecodable(
            "stop_all_without_run_cancel_by_interrupt".into()
        )));
}

#[test]
fn stop_all_vectors_preserve_provider_items_and_strict_addressing() {
    let vectors: Value = serde_json::from_str(vectors::STOP_ALL_JSON).unwrap();
    for case in vectors["requests"].as_array().unwrap() {
        let result = check_request(case["params"].as_object().unwrap());
        assert_eq!(result.is_ok(), case["valid"].as_bool().unwrap(), "{case}");
        if let Some(field) = case["field"].as_str() {
            assert_eq!(result.unwrap_err(), field);
        }
    }
    let reply: StopAllReply = serde_json::from_value(vectors["reply"].clone()).unwrap();
    assert_eq!(serde_json::to_value(reply).unwrap(), vectors["reply"]);
    assert_eq!(
        serde_json::to_value(StopAllRequest::new("stop-1")).unwrap(),
        json!({"request_id":"stop-1"})
    );
    for (id, expected) in [
        (" ".to_owned(), true),
        ("x".repeat(128), true),
        ("x".repeat(129), false),
        ("é".into(), false),
    ] {
        assert_eq!(
            check_request(json!({"request_id":id}).as_object().unwrap()).is_ok(),
            expected
        );
    }
}

#[test]
fn stop_all_uses_current_interrupt_grant_or_owner() {
    let mut senders = vec![PlanSender::new("app-gateway").with_interrupt()];
    assert!(caller_is_admitted(false, "app-gateway", &senders));
    assert!(!caller_is_admitted(false, "peer", &senders));
    senders[0].interrupt = false;
    assert!(!caller_is_admitted(false, "app-gateway", &senders));
    assert!(caller_is_admitted(true, "owner", &senders));
}
