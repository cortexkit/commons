use cortexkit_role_llm_runner::{
    capabilities,
    describe::{check_describe, RoleDescribe},
    send::SendRequest,
    service_tier::{
        check_send, check_service_tiers, Served, ServiceTier, ServiceTierUsage, ServiceTiers,
        Support, TierProblem,
    },
    usage::{RunUsage, StepUsage},
    REQUIRED_OPS,
};
use serde_json::{json, Value};

fn describe() -> Value {
    let mut ops = REQUIRED_OPS.to_vec();
    ops.push(cortexkit_role_llm_runner::ops::SESSION_SEND);
    json!({"majors": [{"version": "llm-runner/v1", "stability": "stable", "ops": ops}],
        "implementation_version": "example", "capabilities": [], "session_capabilities_from": "baseline"})
}

fn tiers() -> ServiceTiers {
    [(
        "openai.responses".into(),
        Support::new(
            vec![ServiceTier::Fast, ServiceTier::Ultrafast],
            Served::Reported,
        ),
    )]
    .into()
}

#[test]
fn service_tier_unknown_requests_are_refused_by_field_and_reason() {
    use cortexkit_role_llm_runner::service_tier::parse_request;
    let raw = json!({"prompt":"hi","send_id":"unknown","service_tier":"future-tier"});
    let problem = parse_request(raw.get("service_tier")).unwrap_err();
    assert_eq!(
        problem.detail(),
        json!({"field":"service_tier","reason":"unsupported_tier"})
    );
    let error = serde_json::from_value::<SendRequest>(raw)
        .unwrap_err()
        .to_string();
    assert!(
        error.contains("service_tier") && error.contains("unsupported_tier"),
        "{error}"
    );
}

#[test]
fn service_tier_record_readers_are_open_but_producer_checks_are_closed() {
    for tier in ["future-tier", "standard", ""] {
        let raw = json!({"service_tier_applied":tier,"service_tier_served":"provider-special"});
        let step: ServiceTierUsage = serde_json::from_value(raw.clone()).unwrap();
        assert_eq!(serde_json::to_value(&step).unwrap(), raw);
        assert_eq!(step.check(), Err(TierProblem::UnsupportedTier));
        let mut run = RunUsage::new();
        run.service_tier_applied = Some(tier.into());
        assert_eq!(run.check_service_tier(), Err(TierProblem::UnsupportedTier));
    }
    for tier in [
        ServiceTier::Fast,
        ServiceTier::Ultrafast,
        ServiceTier::Standard,
    ] {
        assert_eq!(ServiceTierUsage::new(tier).check(), Ok(()));
    }
}

#[test]
fn service_tier_samples_decode_and_preserve_meaning() {
    let samples: Value =
        serde_json::from_str(include_str!("../test-vectors/service-tier-v1/cases.json")).unwrap();
    let mut raw = describe();
    raw["capabilities"] = json!([capabilities::SERVICE_TIER]);
    for valid in samples["describe_valid"].as_array().unwrap() {
        raw["service_tiers"] = valid.clone();
        assert!(check_describe(&raw).is_ok(), "{valid}");
    }
    for invalid in samples["describe_refused"].as_array().unwrap() {
        raw["service_tiers"] = invalid.clone();
        assert!(check_describe(&raw).is_err(), "{invalid}");
    }
    for send in samples["sends"].as_array().unwrap() {
        let request: SendRequest = serde_json::from_value(send.clone()).unwrap();
        assert_eq!(serde_json::to_value(request).unwrap(), *send);
    }
    for usage in samples["usage"].as_array().unwrap() {
        let decoded: ServiceTierUsage = serde_json::from_value(usage.clone()).unwrap();
        assert_eq!(serde_json::to_value(decoded).unwrap(), *usage);
    }
    for (index, problem) in [TierProblem::UnsupportedTier, TierProblem::RunTierFixed]
        .iter()
        .enumerate()
    {
        assert_eq!(samples["refusals"][index]["detail"], problem.detail());
    }
}

#[test]
fn describe_requires_and_validates_the_service_tier_map() {
    let mut raw = describe();
    raw["capabilities"] = json!([capabilities::SERVICE_TIER]);
    assert!(check_describe(&raw).is_err());
    raw["service_tiers"] =
        json!({"openai.responses": {"levels": ["fast", "ultrafast"], "served": "reported"}});
    assert!(check_describe(&raw).is_ok());
    for invalid in [
        json!({"openai.responses": {"levels": [], "served": "reported"}}),
        json!({"openai.responses": {"levels": ["standard"], "served": "reported"}}),
        json!({"openai.responses": {"levels": ["fast", "fast"], "served": "unknown"}}),
        json!({"openai.responses": {"levels": ["rapid"], "served": "reported"}}),
        json!({"openai.responses": {"levels": ["fast"], "served": "guessed"}}),
        json!({"responses": {"levels": ["fast"], "served": "reported"}}),
    ] {
        raw["service_tiers"] = invalid.clone();
        assert!(check_describe(&raw).is_err(), "{invalid}");
    }
    assert!(check_service_tiers(&tiers()).is_ok());
}

#[test]
fn legacy_describe_send_and_usage_bytes_are_unchanged() {
    let minimal: RoleDescribe =
        serde_json::from_value(json!({"majors":[],"implementation_version":"example"})).unwrap();
    assert_eq!(
        serde_json::to_string(&minimal).unwrap(),
        r#"{"majors":[],"implementation_version":"example","capabilities":[]}"#
    );
    let send = SendRequest::new("hi", "s");
    let bytes = r#"{"prompt":"hi","send_id":"s"}"#;
    assert_eq!(serde_json::to_string(&send).unwrap(), bytes);
    assert_eq!(
        serde_json::to_string(&send.with_service_tier(ServiceTier::Standard)).unwrap(),
        r#"{"prompt":"hi","send_id":"s","service_tier":"standard"}"#
    );
    assert_eq!(
        serde_json::to_string(&RunUsage::new()).unwrap(),
        r#"{"steps":0,"unreported_steps":0,"by_model":[]}"#
    );
    assert_eq!(
        serde_json::to_string(&ServiceTierUsage::default()).unwrap(),
        "{}"
    );
}

#[test]
fn explicit_standard_send_survives_encoding_and_is_refused_into_fast_run() {
    use cortexkit_role_llm_runner::send::Delivery;
    let absent = SendRequest::new("steer", "absent").with_delivery(Delivery::Steer);
    let wire = serde_json::to_value(&absent).unwrap();
    assert!(wire.get("service_tier").is_none());
    let decoded: SendRequest = serde_json::from_value(wire).unwrap();
    assert_eq!(
        decoded.check_service_tier(Some(ServiceTier::Fast), "openai.responses", Some(&tiers())),
        Ok(ServiceTier::Fast)
    );

    let explicit = SendRequest::new("steer", "standard")
        .with_delivery(Delivery::Steer)
        .with_service_tier(ServiceTier::Standard);
    let wire = serde_json::to_value(&explicit).unwrap();
    assert_eq!(wire["service_tier"], "standard");
    let decoded: SendRequest = serde_json::from_value(wire).unwrap();
    let refusal = decoded
        .check_service_tier(Some(ServiceTier::Fast), "openai.responses", Some(&tiers()))
        .unwrap_err();
    assert_eq!(
        refusal.detail(),
        json!({"field":"service_tier","reason":"run_tier_fixed"})
    );
}

#[test]
fn sends_are_fixed_per_run_and_never_downgraded() {
    for level in [
        ServiceTier::Standard,
        ServiceTier::Fast,
        ServiceTier::Ultrafast,
    ] {
        assert_eq!(
            check_send(Some(level), None, "openai.responses", Some(&tiers())),
            Ok(level)
        );
    }
    assert_eq!(
        check_send(None, None, "other", None),
        Ok(ServiceTier::Standard)
    );
    assert_eq!(
        check_send(Some(ServiceTier::Fast), None, "openai.responses", None),
        Err(TierProblem::UnsupportedTier)
    );
    assert_eq!(
        check_send(
            Some(ServiceTier::Ultrafast),
            None,
            "anthropic.messages",
            Some(&tiers())
        ),
        Err(TierProblem::UnsupportedTier)
    );
    assert_eq!(
        check_send(
            Some(ServiceTier::Standard),
            Some(ServiceTier::Fast),
            "openai.responses",
            Some(&tiers())
        ),
        Err(TierProblem::RunTierFixed)
    );
    assert_eq!(
        check_send(
            None,
            Some(ServiceTier::Fast),
            "openai.responses",
            Some(&tiers())
        ),
        Ok(ServiceTier::Fast)
    );
    assert_eq!(
        TierProblem::UnsupportedTier.detail(),
        json!({"field":"service_tier", "reason":"unsupported_tier"})
    );
    assert_eq!(
        TierProblem::RunTierFixed.detail(),
        json!({"field":"service_tier", "reason":"run_tier_fixed"})
    );
    let wire = json!({"prompt":"hi","send_id":"s","service_tier":"ultrafast"});
    let request: SendRequest = serde_json::from_value(wire.clone()).unwrap();
    assert_eq!(serde_json::to_value(request).unwrap(), wire);
}

#[test]
fn run_tier_metadata_keeps_only_unanimous_reported_served_values() {
    use cortexkit_role_llm_runner::service_tier::RunServiceTierUsage;
    let mut run = RunUsage::new();
    let mut tiers = RunServiceTierUsage::new();
    let applied = ServiceTierUsage::new(ServiceTier::Fast);
    run.record_step_with_service_tier(
        "p",
        "m",
        None,
        StepUsage::unreported(),
        &applied,
        &mut tiers,
    )
    .unwrap();
    assert_eq!(
        run.service_tier_applied.as_deref(),
        Some(ServiceTier::Fast.as_str())
    );
    assert_eq!(
        run.service_tier_served, None,
        "applied must never be echoed as served"
    );
    for _ in 0..2 {
        run.record_step_with_service_tier(
            "p",
            "m",
            None,
            StepUsage::unreported(),
            &applied.clone().with_served("fast"),
            &mut tiers,
        )
        .unwrap();
        assert_eq!(run.service_tier_served.as_deref(), Some("fast"));
    }
    run.record_step_with_service_tier(
        "p",
        "m",
        None,
        StepUsage::unreported(),
        &applied.clone().with_served("provider-special"),
        &mut tiers,
    )
    .unwrap();
    assert_eq!(run.service_tier_served, None);
    let checkpoint = serde_json::to_value((&run, &tiers)).unwrap();
    let (mut restored, mut restored_tiers): (RunUsage, RunServiceTierUsage) =
        serde_json::from_value(checkpoint).unwrap();
    restored
        .record_step_with_service_tier(
            "p",
            "m",
            None,
            StepUsage::unreported(),
            &applied.with_served("fast"),
            &mut restored_tiers,
        )
        .unwrap();
    assert_eq!(
        restored.service_tier_served, None,
        "a restart must not forget disagreement"
    );
    assert_eq!(
        restored.record_step_with_service_tier(
            "p",
            "m",
            None,
            StepUsage::unreported(),
            &ServiceTierUsage::new(ServiceTier::Standard),
            &mut restored_tiers
        ),
        Err("run_tier_fixed")
    );
    assert_eq!(restored.steps, 5);
    assert_eq!(restored_tiers.steps(), 5);
    assert_eq!(
        restored.record_step_with_service_tier(
            "p",
            "m",
            None,
            StepUsage::unreported(),
            &ServiceTierUsage::new(ServiceTier::Fast),
            &mut RunServiceTierUsage::new()
        ),
        Err("step_tiers_incomplete")
    );
}
