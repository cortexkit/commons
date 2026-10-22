use cortexkit_role_compaction_provider::{
    describe::{check_describe, DescribeRequest, Major, RoleDescribe},
    errors::RefuseCode,
    lineage::MessageRef,
    ops,
    ride::{RideCause, RideRequest},
    setup::{SetupAnswer, SetupRequest},
    view::ViewGetRequest,
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
fn op_envelopes_round_trip() {
    exact(
        &OpRequest::new(ops::ROLE_DESCRIBE, DescribeRequest::new()),
        json!({"method": "role.describe", "params": {}}),
    );
    exact(
        &OpRequest::new(
            ops::COMPACTION_SETUP,
            SetupRequest::new("s", "broca", "r0", Map::new(), "m", 1),
        ),
        json!({"method": "compaction.setup", "params": {
            "session": "s", "harness": "broca", "request_id": "r0", "params": {},
            "composition": {}, "model": "m", "now": 1
        }}),
    );
    let session_ref = "0123456789abcdef0123456789abcdef";
    exact(
        &OpRequest::new(ops::VIEW_GET, ViewGetRequest::new(session_ref, 3)),
        json!({"method": "view.get", "params": {"session_ref": session_ref, "view_version": 3}}),
    );
    exact(
        &OpRequest::new(
            ops::COMPACTION_RIDE,
            RideRequest::new(session_ref, RideCause::TtlExpired, 1500),
        ),
        json!({"method": "compaction.ride", "params": {
            "session_ref": session_ref, "cause": "ttl_expired", "deadline_ms": 1500
        }}),
    );
}

#[test]
fn setup_setters_preserve_every_optional_field() {
    let request = SetupRequest::new("s", "broca", "r0", Map::new(), "m", 1)
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
            "session": "s", "harness": "broca", "request_id": "r0", "params": {}, "preset": "head",
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
            "ops": ["role.describe", "compaction.setup", "view.get", "compaction.ride"],
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
fn setup_refuse_carries_provider_code_and_no_retryable() {
    let wire = json!({"answer": "refuse", "request_id": "r1", "code": "provider_busy",
        "reason": "busy", "provider_code": "mc:historian_running"});
    let setup: SetupAnswer = serde_json::from_value(wire.clone()).unwrap();
    assert_eq!(
        setup,
        SetupAnswer::refuse("r1", RefuseCode::ProviderBusy, "busy")
            .with_provider_code("mc:historian_running")
    );
    assert_eq!(setup.refuse_code(), Some(&RefuseCode::ProviderBusy));
    assert_eq!(setup.refuse_reason(), Some("busy"));
    assert_eq!(setup.provider_code(), Some("mc:historian_running"));
    assert!(!setup.is_ready() && setup.initial().is_none());
    exact(&setup, wire);
    assert_eq!(setup.retryable(), Some(true));
}
