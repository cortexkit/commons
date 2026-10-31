use cortexkit_role_llm_runner::{
    capabilities,
    delivery_order::{DeliveryOrder, Mode, PlanSender, TemplateUse},
    read::{ReadMessage, ReadRequest, ToolTime},
};
use serde_json::json;

#[test]
fn default_read_request_and_entry_keep_exact_bytes() {
    assert_eq!(serde_json::to_string(&ReadRequest::tail()).unwrap(), "{}");
    assert_eq!(
        serde_json::to_string(&ReadRequest::tail().with_include_display_facts(false)).unwrap(),
        "{}"
    );
    let entry = ReadMessage::new(0, "m0", json!({"content":"hello"}));
    assert_eq!(
        serde_json::to_string(&entry).unwrap(),
        r#"{"ordinal":0,"mid":"m0","message":{"content":"hello"}}"#
    );
    assert_eq!(
        serde_json::to_string(&PlanSender::new("sender")).unwrap(),
        r#"{"module":"sender"}"#
    );
}

#[test]
fn display_facts_are_opt_in_and_beside_unchanged_body() {
    let request = ReadRequest::tail().with_include_display_facts(true);
    assert_eq!(
        serde_json::to_value(&request).unwrap(),
        json!({"include_display_facts":true})
    );
    assert_eq!(
        request.check_display_facts(&[] as &[&str]),
        Err("include_display_facts")
    );
    assert_eq!(
        request.check_display_facts(&[capabilities::READ_DISPLAY_FACTS]),
        Ok(())
    );
    assert!(serde_json::from_value::<ReadRequest>(json!({"include_display_facts":null})).is_err());
    assert!(serde_json::from_value::<ReadRequest>(json!({"include_display_facts":"yes"})).is_err());
    let body =
        json!({"content":[{"type":"tool_use","id":"call-1"},{"type":"tool_use","id":"call-2"}]});
    let entry = ReadMessage::new(1, "m1", body.clone()).with_tool_times(vec![
        ToolTime::new("call-1")
            .with_started_at(10)
            .with_finished_at(25),
        ToolTime::new("call-2").with_unanswered("future_reason"),
    ]);
    let wire = serde_json::to_value(entry).unwrap();
    assert_eq!(wire["message"], body);
    assert_eq!(
        wire["tool_times"],
        json!([
            {"tool_call_id":"call-1","started_at":10,"finished_at":25},
            {"tool_call_id":"call-2","unanswered":"future_reason"}
        ])
    );
    let decoded: ReadMessage = serde_json::from_value(wire).unwrap();
    assert_eq!(
        decoded.tool_times[1].unanswered.as_deref(),
        Some("future_reason")
    );
    assert_eq!(
        serde_json::to_value(ToolTime::new("untimed")).unwrap(),
        json!({"tool_call_id":"untimed"})
    );
    let user =
        ReadMessage::new(0, "m0", json!({"content":"hello"})).with_client_message_id("phone_1");
    assert_eq!(
        serde_json::to_value(user).unwrap()["client_message_id"],
        "phone_1"
    );
}

#[test]
fn delivery_deadline_retry_matches_without_changing_content_comparison() {
    let original = DeliveryOrder::new(vec![Mode::ToolResult])
        .with_dedupe_key("key")
        .with_expires_at_ms(10);
    let retry = DeliveryOrder::new(vec![Mode::ToolResult])
        .with_dedupe_key("key")
        .with_expires_at_ms(50);
    assert!(retry.same_payload(&original));
    assert_eq!(original.expires_at_ms, Some(10));
    assert!(!DeliveryOrder::new(vec![Mode::Steer])
        .with_dedupe_key("key")
        .same_payload(&original));
    assert!(!DeliveryOrder::new(vec![Mode::ToolResult])
        .with_dedupe_key("other")
        .same_payload(&original));
    assert!(!DeliveryOrder::new(vec![Mode::ToolResult])
        .with_dedupe_key("key")
        .with_template(TemplateUse::new("notice", "digest", Default::default()))
        .same_payload(&original));
}

#[test]
fn samples_round_trip_and_pin_unchanged_message_values() {
    use cortexkit_role_llm_runner::{read::ReadPage, send::SendRequest, sender_succession};
    let facts: serde_json::Value = serde_json::from_str(include_str!(
        "../test-vectors/llm-runner-v1/read-display-facts.json"
    ))
    .unwrap();
    for key in ["request", "default_request"] {
        let request: ReadRequest = serde_json::from_value(facts[key].clone()).unwrap();
        assert_eq!(serde_json::to_value(request).unwrap(), facts[key]);
    }
    for key in ["page", "default_page"] {
        let page: ReadPage = serde_json::from_value(facts[key].clone()).unwrap();
        assert_eq!(serde_json::to_value(page).unwrap(), facts[key]);
    }
    for (default, decorated) in facts["default_page"]["messages"]
        .as_array()
        .unwrap()
        .iter()
        .zip(facts["page"]["messages"].as_array().unwrap())
    {
        assert_eq!(default["message"], decorated["message"]);
        assert!(default.get("client_message_id").is_none());
        assert!(default.get("tool_times").is_none());
    }
    let succession: serde_json::Value = serde_json::from_str(include_str!(
        "../test-vectors/llm-runner-v1/sender-succession.json"
    ))
    .unwrap();
    let plan = succession["plan"].as_object().unwrap();
    assert_eq!(
        sender_succession::check_plan(plan, &[capabilities::SENDER_SUCCESSION]),
        Ok(())
    );
    for key in ["predecessor_send", "successor_retry", "different_payload"] {
        let request: SendRequest = serde_json::from_value(succession[key].clone()).unwrap();
        assert_eq!(serde_json::to_value(request).unwrap(), succession[key]);
    }
}
