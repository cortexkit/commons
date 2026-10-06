use cortexkit_role_compaction_provider::{
    describe::DescribeRequest,
    ops,
    ready::{CompactionReady, ReadyReply},
    setup::SetupRequest,
    status::{Estimate, MessageRef, StepStatus},
    OpRequest,
};
use serde::{de::DeserializeOwned, Serialize};
use serde_json::{json, Map, Value};

fn exact<T: Serialize + DeserializeOwned>(value: &T, expected: Value) {
    assert_eq!(serde_json::to_value(value).unwrap(), expected);
    let decoded: T = serde_json::from_value(expected.clone()).unwrap();
    assert_eq!(serde_json::to_value(decoded).unwrap(), expected);
}

#[test]
fn op_envelopes_and_empty_answers_round_trip() {
    exact(
        &OpRequest::new(ops::ROLE_DESCRIBE, DescribeRequest::new()),
        json!({"method": "role.describe", "params": {}}),
    );
    exact(
        &OpRequest::new(
            ops::COMPACTION_SETUP,
            SetupRequest::new("s", "r0", Map::new(), "m", 1),
        ),
        json!({"method": "compaction.setup", "params": {
            "session": "s", "request_id": "r0", "params": {},
            "composition": {}, "model": "m", "now": 1
        }}),
    );
    exact(
        &OpRequest::new(
            ops::COMPACTION_STEP,
            StepStatus::new(
                "s",
                "r1",
                "l",
                "step",
                "user_turn",
                "m",
                Estimate::new(3),
                1,
            ),
        ),
        json!({"method": "compaction.step", "params": {
            "session": "s", "request_id": "r1", "lineage_id": "l",
            "step_id": "step", "step_kind": "user_turn", "model": "m",
            "estimate": {"request_tokens": 3}, "messages": [], "now": 1
        }}),
    );
    exact(
        &OpRequest::new(ops::COMPACTION_READY, CompactionReady::new("s", "r1")),
        json!({"method": "compaction.ready", "params": {"session": "s", "request_id": "r1"}}),
    );
    exact(&ReadyReply::new(), json!({}));
}

#[test]
fn setup_setters_preserve_every_optional_field() {
    let request = SetupRequest::new("s", "r0", Map::new(), "m", 1)
        .with_item(Some("head".into()), Map::new())
        .with_lineage("l")
        .with_model_details(Some("high".into()), Some(200), Some(10))
        .with_newest(MessageRef {
            ordinal: 2,
            mid: "m2".into(),
        });
    exact(
        &request,
        json!({
            "session": "s", "request_id": "r0", "params": {}, "preset": "head",
            "composition": {}, "model": "m", "now": 1, "lineage_id": "l",
            "variant": "high", "context_window": 200, "output_limit": 10,
            "newest": {"ordinal": 2, "mid": "m2"}
        }),
    );
}
