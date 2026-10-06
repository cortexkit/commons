use cortexkit_role_step_transform_provider::{
    answer::ApprovalAsk,
    describe::DescribeRequest,
    hook::{HookCall, Subject},
    ops,
    subscription::{
        DeclareRequest, DeclaredSubscription, Hook, OnUnavailable, Op, Phase, Subscription,
    },
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
fn op_envelopes_round_trip() {
    exact(
        &OpRequest::new(ops::ROLE_DESCRIBE, DescribeRequest::new()),
        json!({"method": "role.describe", "params": {}}),
    );
    exact(
        &OpRequest::new(
            ops::TRANSFORM_DECLARE,
            DeclareRequest::new(Map::new())
                .with_preset("head")
                .with_composition(Map::new()),
        ),
        json!({"method": "transform.declare", "params": {
            "preset": "head", "params": {}, "composition": {}
        }}),
    );
    exact(
        &OpRequest::new(
            ops::TRANSFORM_HOOK,
            HookCall::new(
                "s",
                Subject::PreUser {
                    text: "hi".into(),
                    mark: None,
                    delivery: None,
                },
            )
            .with_lineage("l")
            .with_item(Some("head".into()), Map::new()),
        ),
        json!({"method": "transform.hook", "params": {
            "session": "s", "lineage_id": "l", "preset": "head",
            "params": {}, "hook": "pre_user", "text": "hi"
        }}),
    );
}

#[test]
fn subscription_and_approval_builders_match_wire_shapes() {
    let declared = DeclaredSubscription::new(Hook::PreTool, vec![], 1000)
        .with_phase(Phase::Validate)
        .with_tools(vec!["bash".into()])
        .with_on_unavailable(OnUnavailable::Refuse);
    let planned = Subscription::new(Hook::PreTool, vec![], OnUnavailable::Refuse, 1000)
        .with_phase(Phase::Validate)
        .with_tools(vec!["bash".into()]);
    let wire = json!({"hook": "pre_tool", "phase": "validate", "tools": ["bash"],
        "ops": [], "on_unavailable": "refuse", "budget_ms": 1000});
    exact(&declared, wire.clone());
    exact(&planned, wire);
    exact(
        &DeclaredSubscription::new(Hook::PreUser, vec![Op::Append], 100),
        json!({"hook": "pre_user", "ops": ["append"], "budget_ms": 100}),
    );
    exact(
        &ApprovalAsk::new("Run?", 600, "deny", true, "notify_only")
            .with_options(vec!["run".into(), "skip".into()]),
        json!({"prompt": "Run?", "expiry_ms": 600, "on_expiry": "deny", "material_damage": true,
            "late_execution": "notify_only", "options": ["run", "skip"]}),
    );
}
