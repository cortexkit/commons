use cortexkit_role_step_transform_provider::{
    answer::{ApprovalAsk, LateExecution, OnExpiry},
    describe::{check_describe, DescribeRequest, Major, RoleDescribe},
    grant::{plan_user_grants, UserGrant, USER_GRANTS_FIELD},
    hook::{HookCall, Subject},
    ops,
    subscription::{
        DeclareRequest, DeclaredSubscription, Hook, OnUnavailable, Op, Phase, Subscription,
    },
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
                    blocks: vec!["hi".into()],
                    mark: None,
                    delivery: None,
                },
            )
            .with_lineage("l")
            .with_item(Some("head".into()), Map::new()),
        ),
        json!({"method": "transform.hook", "params": {
            "session": "s", "lineage_id": "l", "preset": "head",
            "params": {}, "hook": "pre_user", "blocks": ["hi"]
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
        &ApprovalAsk::new(
            "Run?",
            1_790_000_723_000,
            OnExpiry::Deny,
            true,
            LateExecution::NotifyOnly,
        )
        .with_options(vec!["run".into(), "skip".into()]),
        json!({"prompt": "Run?", "expires_at_ms": 1_790_000_723_000u64, "on_expiry": "deny",
            "material_damage": true, "late_execution": "notify_only", "options": ["run", "skip"]}),
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
        "majors": [{"version": "step-transform-provider/v1",
            "ops": ["role.describe", "transform.declare", "transform.hook"],
            "stability": "alpha"}],
        "implementation_version": "1.0.0", "capabilities": [],
        "runner_groups": ["transcript_reads"]
    });
    exact(&describe, wire.clone());
    let decoded = check_describe(&wire).unwrap();
    assert_eq!(decoded.unmet_runner_groups(&[]), ["transcript_reads"]);
    assert!(decoded
        .unmet_runner_groups(&["transcript_reads".into()])
        .is_empty());
}

#[test]
fn user_grants_round_trip_in_their_own_field() {
    let grants = vec![UserGrant::new(
        "acme-trim",
        Hook::PostTool,
        vec!["read".into()],
    )];
    let wire = json!([{"module": "acme-trim", "hook": "post_tool", "tools": ["read"]}]);
    exact(&grants, wire.clone());
    let mut plan = Map::new();
    plan.insert(USER_GRANTS_FIELD.into(), wire);
    assert_eq!(plan_user_grants(&plan).unwrap(), grants);
    assert!(grants[0].allows_post_tool_replace("acme-trim", "read"));
}

#[test]
fn a_grant_in_an_ordinary_plan_field_is_not_a_grant() {
    let grant = json!([{"module": "acme-trim", "hook": "post_tool", "tools": ["read"]}]);
    let plan: Map<String, Value> = serde_json::from_value(json!({
        "step_transform_items": [{"provider": "acme-trim", "preset": "default",
            "params": {"user_grants": grant.clone()},
            "subscriptions": [{"hook": "post_tool", "ops": ["replace"],
                "on_unavailable": "pass", "budget_ms": 100, "user_grants": grant.clone()}]}],
        "composition": {"user_grants": grant.clone()},
        "grants": grant,
    }))
    .unwrap();
    assert!(plan_user_grants(&plan).unwrap().is_empty());
}
