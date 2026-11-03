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
    for key in ["page", "default_page", "call_identity_page"] {
        let page: ReadPage = serde_json::from_value(facts[key].clone()).unwrap();
        assert_eq!(serde_json::to_value(page).unwrap(), facts[key]);
    }
    // The sample page for a runner without call_identity carries neither field.
    for entry in facts["page"]["messages"].as_array().unwrap() {
        for time in entry["tool_times"].as_array().into_iter().flatten() {
            assert!(time.get("call_key").is_none() && time.get("schema_pin").is_none());
        }
    }
    // A call prepared and then denied keeps its call_key but has no
    // schema_pin, because it was never dispatched.
    let denied = &facts["call_identity_page"]["messages"][0]["tool_times"][1];
    assert!(denied.get("call_key").is_some() && denied.get("schema_pin").is_none());
    let built = ToolTime::new("call-1")
        .with_started_at(100)
        .with_finished_at(125)
        .with_call_key("ck-1")
        .with_schema_pin(
            "tp1:read_file:0f1e2d3c4b5a69788796a5b4c3d2e1f00f1e2d3c4b5a69788796a5b4c3d2e1f0:1",
        );
    assert_eq!(
        serde_json::to_value(built).unwrap(),
        facts["call_identity_page"]["messages"][0]["tool_times"][0]
    );
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

#[test]
fn completed_at_and_reasoning_summaries_round_trip_without_changing_message_bytes() {
    use cortexkit_role_llm_runner::{
        display,
        read::{ReadPage, ReasoningSummary},
    };
    let facts: serde_json::Value = serde_json::from_str(include_str!(
        "../test-vectors/llm-runner-v1/read-display-facts.json"
    ))
    .unwrap();
    let plain: ReadPage = serde_json::from_value(facts["assistant_page"].clone()).unwrap();
    let plain_bytes = serde_json::to_vec(&plain).unwrap();
    assert_eq!(
        serde_json::to_value(&plain).unwrap(),
        facts["assistant_page"]
    );
    for (key, member) in [
        ("completed_page", "completed_at"),
        ("reasoning_summaries_page", "reasoning_summaries"),
    ] {
        let page: ReadPage = serde_json::from_value(facts[key].clone()).unwrap();
        assert_eq!(serde_json::to_value(&page).unwrap(), facts[key]);
        assert_eq!(page.messages[0].message, plain.messages[0].message);
        let mut stripped = serde_json::to_value(&page).unwrap();
        stripped["messages"][0]
            .as_object_mut()
            .unwrap()
            .remove(member);
        let stripped: ReadPage = serde_json::from_value(stripped).unwrap();
        assert_eq!(serde_json::to_vec(&stripped).unwrap(), plain_bytes);
    }
    let entry = plain.messages[0].clone().with_completed_at(200);
    assert_eq!(
        serde_json::to_value(&entry).unwrap(),
        facts["completed_page"]["messages"][0]
    );
    let summary = ReasoningSummary::new(1, "An invented summary.");
    let wire = serde_json::to_value(&summary).unwrap();
    let decoded: ReasoningSummary = serde_json::from_value(wire.clone()).unwrap();
    assert_eq!(decoded, summary);
    assert_eq!(
        wire,
        serde_json::json!({"part":1,"text":"An invented summary."})
    );
    assert_eq!(
        display::reasoning_summary(&plain.messages[0].message["content"][1]).as_deref(),
        Some(summary.text.as_str())
    );
    let entry = plain.messages[0]
        .clone()
        .with_reasoning_summaries(vec![summary]);
    assert_eq!(
        serde_json::to_value(&entry).unwrap(),
        facts["reasoning_summaries_page"]["messages"][0]
    );
}
