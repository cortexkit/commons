//! The model-visible size measure and limit, against the shared vectors.

use cortexkit_role_tool_provider::{
    result::{
        legacy_model_visible_bytes, model_visible_bytes, ContentPart, ToolResult,
        MAX_MODEL_VISIBLE_BYTES,
    },
    vectors,
};
use serde_json::Value;

fn vectors() -> Value {
    serde_json::from_str(vectors::MODEL_VISIBLE_SIZE_JSON).unwrap()
}

#[test]
fn the_limit_is_the_vectors_limit() {
    assert_eq!(MAX_MODEL_VISIBLE_BYTES, 65_536);
    assert_eq!(vectors()["limit"], MAX_MODEL_VISIBLE_BYTES);
}

#[test]
fn tool_result_v1_replies_count_their_model_visible_bytes() {
    for case in vectors()["results"].as_array().unwrap() {
        let result: ToolResult = serde_json::from_value(case["result"].clone()).unwrap();
        assert_eq!(
            model_visible_bytes(&result) as u64,
            case["bytes"].as_u64().unwrap(),
            "{}",
            case["name"]
        );
    }
}

#[test]
fn legacy_bodies_count_text_or_compact_json() {
    for case in vectors()["legacy"].as_array().unwrap() {
        assert_eq!(
            legacy_model_visible_bytes(&case["body"]) as u64,
            case["bytes"].as_u64().unwrap(),
            "{}",
            case["name"]
        );
    }
}

#[test]
fn boundary_cases_are_over_exactly_when_past_the_limit() {
    for case in vectors()["boundary"].as_array().unwrap() {
        let text = case["text_unit"]
            .as_str()
            .unwrap()
            .repeat(case["repeat"].as_u64().unwrap() as usize);
        let bytes = model_visible_bytes(&ToolResult::new(vec![ContentPart::text(text)]));
        assert_eq!(
            bytes as u64,
            case["bytes"].as_u64().unwrap(),
            "{}",
            case["name"]
        );
        assert_eq!(
            bytes > MAX_MODEL_VISIBLE_BYTES,
            case["over"].as_bool().unwrap(),
            "{}",
            case["name"]
        );
    }
}
