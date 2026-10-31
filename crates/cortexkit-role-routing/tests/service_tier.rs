use cortexkit_role_routing::{
    capabilities,
    report::{parse_report_request, Outcome, ReportRequest, Usage},
    select::{
        check_answer, parse_select_request_serving, Requirements, RouteRef, RunnerRef,
        SelectAnswer, SelectRequest,
    },
    service_tier::ServiceTier,
};
use serde_json::{json, Value};

#[test]
fn service_tier_spellings_and_wire_apis_do_not_drift_from_runner() {
    use cortexkit_role_llm_runner::service_tier::{ServiceTier as RunnerTier, WIRE_APIS};
    assert_eq!(ServiceTier::ALL.len(), RunnerTier::ALL.len());
    for (&local, &runner) in ServiceTier::ALL.iter().zip(RunnerTier::ALL) {
        assert_eq!(local.as_str(), runner.as_str());
        assert_eq!(ServiceTier::parse(runner.as_str()), Some(local));
        assert_eq!(serde_json::to_value(local).unwrap(), json!(runner.as_str()));
    }
    assert_eq!(WIRE_APIS, cortexkit_role_routing::spellings::WIRE_APIS);
}

#[test]
fn service_tier_unknown_request_is_refused_by_name_not_as_a_decode_error() {
    let params =
        json!({"consumer":"summary","requirements":{"iq":50},"service_tier":"future-tier"});
    let refusal = parse_select_request_serving(&params, &[capabilities::SERVICE_TIER]).unwrap_err();
    assert_eq!(refusal.code.as_str(), "invalid_request");
    assert_eq!(
        serde_json::to_value(refusal.detail).unwrap(),
        json!({"field":"service_tier","reason":"unsupported_tier"})
    );
    let error = serde_json::from_value::<SelectRequest>(params)
        .unwrap_err()
        .to_string();
    assert!(
        error.contains("service_tier") && error.contains("unsupported_tier"),
        "{error}"
    );
}

#[test]
fn service_tier_reports_and_decisions_preserve_unknown_record_levels() {
    let raw = json!({"decision_id":"d","outcome":"completed","usage":{"service_tier_applied":"future-tier","service_tier_served":"provider-special"}});
    let report = parse_report_request(&raw).unwrap();
    assert_eq!(
        report.usage.as_ref().unwrap().check_service_tier(),
        Err("unsupported_tier")
    );
    assert_eq!(serde_json::to_value(report).unwrap(), raw);
    let route = RouteRef::new("p", "m");
    let mut raw = serde_json::to_value(SelectAnswer::new(
        "d",
        route.clone(),
        RunnerRef::new("r", route),
        100,
        10,
        1,
        "ok",
    ))
    .unwrap();
    raw["service_tier_chosen"] = json!("future-tier");
    let decision: SelectAnswer = serde_json::from_value(raw.clone()).unwrap();
    assert_eq!(serde_json::to_value(decision).unwrap(), raw);
}

#[test]
fn service_tier_select_requires_discovery_and_records_fallback() {
    let params = json!({"consumer":"summary","requirements":{"iq":50},"service_tier":"ultrafast"});
    assert!(parse_select_request_serving::<&str>(&params, &[]).is_err());
    let request = parse_select_request_serving(&params, &[capabilities::SERVICE_TIER]).unwrap();
    let route = RouteRef::new("p", "m");
    let answer = SelectAnswer::new(
        "d",
        route.clone(),
        RunnerRef::new("r", route),
        100,
        10,
        1,
        "ok",
    );
    assert!(check_answer(&request, &answer).is_err());
    for chosen in [
        ServiceTier::Ultrafast,
        ServiceTier::Fast,
        ServiceTier::Standard,
    ] {
        let answer = answer.clone().with_service_tier_chosen(chosen);
        assert!(check_answer(&request, &answer).is_ok());
        assert_eq!(
            serde_json::to_value(answer).unwrap()["service_tier_chosen"],
            json!(chosen)
        );
    }
    let standard = SelectRequest::new("summary", Requirements::iq(50));
    assert!(check_answer(
        &standard,
        &answer.with_service_tier_chosen(ServiceTier::Fast)
    )
    .is_err());
}

#[test]
fn service_tier_report_accepts_only_the_two_new_usage_members() {
    let samples: Value =
        serde_json::from_str(include_str!("../test-vectors/service-tier-v1/cases.json")).unwrap();
    for usage in samples["usage"].as_array().unwrap() {
        let params = json!({"decision_id":"d", "outcome":"completed", "usage":usage});
        let report = parse_report_request(&params).unwrap();
        assert_eq!(serde_json::to_value(report).unwrap(), params);
    }
    assert!(
        parse_report_request(&json!({"decision_id":"d","outcome":"completed",
        "usage":{"service_tier_applied":"fast","extra":true}}))
        .is_err()
    );
}

#[test]
fn service_tier_legacy_select_decision_and_report_bytes_are_pinned() {
    let select = SelectRequest::new("summary", Requirements::iq(50));
    assert_eq!(
        serde_json::to_string(&select).unwrap(),
        r#"{"consumer":"summary","requirements":{"iq":50}}"#
    );
    let route = RouteRef::new("p", "m");
    let answer = SelectAnswer::new(
        "d",
        route.clone(),
        RunnerRef::new("r", route),
        100,
        10,
        1,
        "ok",
    );
    assert_eq!(
        serde_json::to_string(&answer).unwrap(),
        r#"{"decision_id":"d","route":{"provider":"p","model":"m"},"runner":{"module":"r","model":{"provider":"p","model":"m"}},"context_window":100,"max_output_tokens":10,"expires_at_ms":1,"explanation":"ok"}"#
    );
    let report =
        ReportRequest::new("d", Outcome::Completed).with_usage(Usage::new().with_input_tokens(7));
    assert_eq!(
        serde_json::to_string(&report).unwrap(),
        r#"{"decision_id":"d","outcome":"completed","usage":{"input_tokens":7}}"#
    );
    let standard = Usage::new()
        .with_input_tokens(7)
        .with_service_tier_applied(ServiceTier::Standard);
    assert_eq!(
        serde_json::to_string(&standard).unwrap(),
        r#"{"input_tokens":7}"#
    );
}
