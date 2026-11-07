use cortexkit_role_tool_provider::{
    call::ToolCallRequest,
    call_yield::*,
    consumer_reads_call_yield_v1,
    result::{ContentPart, HeldReply, ToolResult},
    scope::ScopeIdentity,
    vectors,
};
use serde_json::{json, Value};

#[test]
fn call_yield_catalog_tag_is_distinct_from_route_capability() {
    use cortexkit_role_tool_provider::{check_capability_tag, CALL_YIELD_TOOL_TAG, CALL_YIELD_V1};
    let samples: Value = serde_json::from_str(vectors::CALL_YIELD_JSON).unwrap();
    assert_eq!(
        samples["opt_in"]["tool_capabilities"],
        json!([CALL_YIELD_TOOL_TAG])
    );
    assert!(samples["opt_in"]["provided_capabilities"]
        .as_array()
        .unwrap()
        .contains(&json!(CALL_YIELD_V1)));
    assert!(samples["opt_in"]["consumer_capabilities"]
        .as_array()
        .unwrap()
        .contains(&json!(CALL_YIELD_V1)));
    assert!(check_capability_tag(CALL_YIELD_TOOL_TAG).is_ok());
    assert!(check_capability_tag(CALL_YIELD_V1).is_err());
}

#[test]
fn call_yield_requests_answers_and_held_vectors() {
    let vectors: Value = serde_json::from_str(vectors::CALL_YIELD_JSON).unwrap();
    for case in vectors["requests"].as_array().unwrap() {
        let request: ToolCallRequest = serde_json::from_value(case["request"].clone()).unwrap();
        let parsed = parse_yield_request(&request);
        assert_eq!(parsed.is_ok(), case["valid"].as_bool().unwrap(), "{case}");
        if let Some(field) = case["field"].as_str() {
            assert_eq!(parsed.unwrap_err().detail.unwrap()["field"], field);
        }
    }
    for answer in vectors["answers"].as_array().unwrap() {
        let decoded: YieldAnswer = serde_json::from_value(answer.clone()).unwrap();
        assert_eq!(serde_json::to_value(decoded).unwrap(), *answer);
    }
    let held = HeldReply::new("running").with_yielded();
    assert_eq!(
        serde_json::to_value(&held).unwrap(),
        json!({"reason":"running","yielded":true})
    );
    assert!(held.validate().is_ok());
    assert!(HeldReply::new("consent").with_yielded().validate().is_err());
    let result: ToolResult = serde_json::from_value(vectors["held"].clone()).unwrap();
    assert_eq!(result.held.unwrap(), held);
    let ordinary = ToolResult::new(vec![ContentPart::text("done")]);
    assert!(serde_json::to_value(ordinary)
        .unwrap()
        .get("held")
        .is_none());
}

#[test]
fn call_yield_requires_all_bind_declarations() {
    let names = ["tool-result/v1", "held-reply/v1", "call-yield/v1"];
    for mask in 0..8 {
        let capabilities: Vec<String> = names
            .iter()
            .enumerate()
            .filter(|(bit, _)| mask & (1 << bit) != 0)
            .map(|(_, name)| (*name).to_owned())
            .collect();
        assert_eq!(consumer_reads_call_yield_v1(Some(&capabilities)), mask == 7);
    }
    assert!(!consumer_reads_call_yield_v1(None));
}

#[test]
fn call_yield_never_resolves_another_carrier() {
    assert!(record_is_open_on_route("carrier", "carrier", None, None));
    assert!(!record_is_open_on_route("other", "carrier", None, None));
    let scope: ScopeIdentity =
        serde_json::from_value(json!({"owner":"owner","ref":"scope","scope_epoch":1})).unwrap();
    assert!(!record_is_open_on_route(
        "carrier",
        "carrier",
        Some(&scope),
        None
    ));
}
