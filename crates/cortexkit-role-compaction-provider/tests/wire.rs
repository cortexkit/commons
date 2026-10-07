use cortexkit_role_compaction_provider::{
    answer::StepAnswer,
    describe::{check_describe, DescribeRequest, Major, RoleDescribe},
    errors::RefuseCode,
    ops,
    ready::{CompactionReady, ReadyReply},
    setup::{SetupAnswer, SetupRequest},
    status::{Estimate, MessageRef, StepStatus},
    OpRequest, PROVIDES, REQUIRED_OPS,
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

#[test]
fn runner_groups_round_trip_and_name_the_unmet_ones() {
    let describe = RoleDescribe::new(
        vec![Major::new(
            PROVIDES,
            REQUIRED_OPS.iter().map(|op| op.to_string()).collect(),
            "alpha",
        )],
        "1.0.0",
    )
    .with_runner_groups(vec!["transcript_reads".into()]);
    let wire = json!({
        "majors": [{"version": "compaction-provider/v1",
            "ops": ["role.describe", "compaction.setup", "compaction.step"],
            "stability": "alpha"}],
        "implementation_version": "1.0.0", "capabilities": [],
        "runner_groups": ["transcript_reads"]
    });
    exact(&describe, wire.clone());
    let decoded = check_describe(&wire).unwrap();
    assert_eq!(
        decoded.unmet_runner_groups(&["compaction".into()]),
        ["transcript_reads"]
    );
    assert!(decoded
        .unmet_runner_groups(&["compaction".into(), "transcript_reads".into()])
        .is_empty());
}

#[test]
fn refuse_carries_provider_code_and_no_retryable() {
    let wire = json!({"answer": "refuse", "request_id": "r1", "code": "provider_busy",
        "reason": "busy", "provider_code": "mc:historian_running"});
    let step: StepAnswer = serde_json::from_value(wire.clone()).unwrap();
    assert_eq!(
        step,
        StepAnswer::Refuse {
            request_id: "r1".into(),
            code: RefuseCode::ProviderBusy,
            reason: "busy".into(),
            provider_code: Some("mc:historian_running".into()),
        }
    );
    exact(&step, wire.clone());
    let setup: SetupAnswer = serde_json::from_value(wire.clone()).unwrap();
    exact(&setup, wire);
    assert_eq!(step.retryable(), Some(true));
}
