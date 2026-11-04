use cortexkit_role_llm_runner::{
    capabilities,
    describe::RoleDescribe,
    head_selection::{self, PlanModel, PlanModelProblem},
    read::HeadMeta,
    send::SendRequest,
    service_tier::{Served, ServiceTier, Support},
};
use serde_json::json;

fn head_selection_description(groups: &[&str]) -> serde_json::Value {
    let mut ops = cortexkit_role_llm_runner::REQUIRED_OPS.to_vec();
    ops.push(cortexkit_role_llm_runner::ops::SESSION_HEAD);
    ops.push(cortexkit_role_llm_runner::ops::SESSION_SEND);
    json!({
        "majors": [{"version":"llm-runner/v1", "ops":ops, "stability":"alpha"}],
        "implementation_version":"test", "capabilities":groups,
        "session_capabilities_from":"baseline"
    })
}

#[test]
fn head_selection_requires_plans_by_name() {
    let problems =
        cortexkit_role_llm_runner::describe::check_describe(&head_selection_description(&[
            capabilities::HEAD_SELECTION,
        ]))
        .unwrap_err();
    assert_eq!(
        problems,
        vec![
            cortexkit_role_llm_runner::describe::DescribeProblem::Undecodable(
                "head_selection_without_plans".into()
            )
        ]
    );
}

#[test]
fn head_selection_creating_send_does_not_require_refresh() {
    let describe = head_selection_description(&[capabilities::HEAD_SELECTION, capabilities::PLANS]);
    cortexkit_role_llm_runner::describe::check_describe(&describe).unwrap();
}

#[test]
fn head_selection_omitted_and_standard_tiers_compare_equal() {
    let omitted = PlanModel::new("example", "model");
    let explicit = omitted.clone().with_service_tier(ServiceTier::Standard);
    assert_eq!(omitted, explicit);
    assert_ne!(
        omitted,
        omitted.clone().with_service_tier(ServiceTier::Fast)
    );
}

fn describe(tiers: bool) -> RoleDescribe {
    let mut describe = RoleDescribe::default();
    describe.capabilities = vec![
        capabilities::HEAD_SELECTION.into(),
        capabilities::PLANS.into(),
    ];
    if tiers {
        describe
            .capabilities
            .push(capabilities::SERVICE_TIER.into());
        describe.service_tiers = Some(
            [(
                "openai.responses".into(),
                Support::new(vec![ServiceTier::Fast], Served::Unknown),
            )]
            .into(),
        );
    }
    describe
}

#[test]
fn head_selection_strict_model_and_named_refusals() {
    for model in [
        json!(null),
        json!({"provider":"example"}),
        json!({"provider":"example","model":1}),
        json!({"provider":"example","model":"model","extra":true}),
    ] {
        let error = head_selection::check_plan(&json!({"model":model}), &describe(true), |_| {
            Some("openai.responses".into())
        })
        .unwrap_err();
        assert_eq!(error, PlanModelProblem::Malformed);
        assert_eq!(error.detail(), json!({"field":"plan.model"}));
    }
    let model = json!({"provider":"example","model":"model","service_tier":"fast"});
    let unavailable =
        head_selection::check_plan(&json!({"model":model}), &describe(true), |_| None).unwrap_err();
    assert_eq!(
        unavailable.detail(),
        json!({"field":"plan.model","reason":"unsupported_model"})
    );
    for tier in ["fast", "ultrafast", "future"] {
        let error = head_selection::check_plan(
            &json!({"model":{"provider":"example","model":"model","service_tier":tier}}),
            &describe(false),
            |_| Some("openai.responses".into()),
        )
        .unwrap_err();
        assert_eq!(
            error.detail(),
            json!({"field":"plan.model.service_tier","reason":"unsupported_tier"})
        );
    }
    let error = head_selection::check_plan(&json!({"model":model}), &describe(true), |_| {
        Some("anthropic.messages".into())
    })
    .unwrap_err();
    assert_eq!(error, PlanModelProblem::UnsupportedTier);
}

#[test]
fn head_selection_opt_in_and_standard_without_service_tier() {
    let ignored =
        head_selection::check_plan(&json!({"model":false}), &RoleDescribe::default(), |_| {
            panic!("unknown member must not resolve a route")
        });
    assert_eq!(ignored, Ok(None));
    for tier in [None, Some("standard")] {
        let mut model = json!({"provider":"example","model":"model"});
        if let Some(tier) = tier {
            model["service_tier"] = json!(tier);
        }
        assert!(
            head_selection::check_plan(&json!({"model":model}), &describe(false), |_| Some(
                "openai.responses".into()
            ))
            .unwrap()
            .is_some()
        );
    }
    assert_eq!(
        head_selection::check_plan(&json!({}), &describe(true), |_| panic!("absent member")),
        Ok(None)
    );
}

#[test]
fn head_selection_tier_precedence_and_joining_run() {
    let plan = PlanModel::new("example", "model").with_service_tier(ServiceTier::Fast);
    let resolve = |requested, running| {
        head_selection::check_send_tier(
            requested,
            running,
            Some(&plan),
            "openai.responses",
            &describe(true),
        )
    };
    assert_eq!(resolve(None, None), Ok(ServiceTier::Fast));
    assert_eq!(
        resolve(Some(ServiceTier::Standard), None),
        Ok(ServiceTier::Standard)
    );
    assert_eq!(
        resolve(None, Some(ServiceTier::Standard)),
        Ok(ServiceTier::Standard)
    );
    assert_eq!(
        resolve(Some(ServiceTier::Fast), Some(ServiceTier::Standard))
            .unwrap_err()
            .reason(),
        "run_tier_fixed"
    );
    assert_eq!(
        head_selection::check_send_tier(None, None, None, "openai.responses", &describe(false)),
        Ok(ServiceTier::Standard)
    );
}

#[test]
fn head_selection_legacy_wire_bytes_and_open_head_tier() {
    let send = r#"{"prompt":"hello","send_id":"send"}"#;
    assert_eq!(
        serde_json::to_string(&serde_json::from_str::<SendRequest>(send).unwrap()).unwrap(),
        send
    );
    let head = r#"{"history_version":0,"incarnation":"incarnation"}"#;
    let parsed: HeadMeta = serde_json::from_str(head).unwrap();
    assert_eq!(serde_json::to_string(&parsed).unwrap(), head);
    let parsed = parsed.with_service_tier("future");
    let roundtrip: HeadMeta =
        serde_json::from_value(serde_json::to_value(&parsed).unwrap()).unwrap();
    assert_eq!(roundtrip.service_tier.as_deref(), Some("future"));
    let model = PlanModel::new("example", "model");
    assert_eq!(
        serde_json::to_value(model).unwrap(),
        json!({"provider":"example","model":"model"})
    );
}
