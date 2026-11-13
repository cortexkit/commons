use cortexkit_role_routing::{
    attempt::{AttemptOutcome, AttemptStatus},
    capabilities::{self, CONSUMER_DECLARATIONS, REPORT_ATTEMPT_OUTCOMES},
    declarations::{
        parse_consumer_declarations, parse_consumer_declarations_serving, DemandDefaults,
    },
    placement::{admits_selector, owns_consumer_name, HarnessCapability, Kind, Placement},
    price::{Price, RatesPerMtok},
    report::{parse_report_request, parse_report_request_serving, Outcome, ReportRequest},
    select::{
        check_answer, check_answer_serving, check_answer_serving_with_harnesses,
        parse_select_request, parse_select_request_serving, parse_select_request_with_defaults,
        Requirements, RouteRef, RunnerRef, SelectAnswer, SelectRequest,
    },
};
use serde_json::{json, Value};
use sha2::{Digest, Sha256};

fn samples() -> Value {
    serde_json::from_str(include_str!(
        "../test-vectors/routing-v1/consumers-and-placement.json"
    ))
    .unwrap()
}

#[test]
fn declaration_defaults_are_opt_in_explicit_zero_wins() {
    let defaults = DemandDefaults::new().with_iq(40);
    let absent = json!({"consumer":"example.summary.child","requirements":{}});
    assert!(parse_select_request(&absent).is_err());
    assert!(parse_select_request_with_defaults(&absent, &[] as &[&str], &defaults).is_err());
    assert_eq!(
        parse_select_request_with_defaults(&absent, &[CONSUMER_DECLARATIONS], &defaults)
            .unwrap()
            .requirements
            .iq,
        40
    );
    let explicit = json!({"consumer":"example.summary.child","requirements":{"iq":0}});
    assert_eq!(
        parse_select_request_with_defaults(&explicit, &[CONSUMER_DECLARATIONS], &defaults)
            .unwrap()
            .requirements
            .iq,
        0
    );
    assert!(parse_select_request_with_defaults(
        &absent,
        &[CONSUMER_DECLARATIONS],
        &DemandDefaults::new()
    )
    .is_err());
    for (field, invalid) in [
        ("iq", json!(101)),
        ("ladder", json!(["not-a-pin"])),
        ("placement", json!("elsewhere")),
        ("selectors", json!([""])),
        ("kind", json!(42)),
    ] {
        let mut slot = json!({"consumers":[{"name":"example.summary"}]});
        slot["consumers"][0][field] = invalid;
        assert!(
            parse_consumer_declarations_serving(&slot, &[CONSUMER_DECLARATIONS]).is_err(),
            "{field}"
        );
    }
    let defaults = DemandDefaults::new()
        .with_iq(40)
        .with_ladder(vec!["p/m:fast".into()])
        .with_placement(Placement::module("runner"))
        .with_selectors(vec!["selector".into()])
        .with_kind(Kind::Search);
    assert_eq!(
        serde_json::to_value(defaults).unwrap()["placement"],
        json!({"module":"runner"})
    );
}

#[test]
fn typed_declaration_validation_preserves_legacy_presence_and_allows_declared_iq() {
    use cortexkit_role_routing::ConsumerDeclarations;
    for iq in [json!(null), json!(40)] {
        let decoded: ConsumerDeclarations =
            serde_json::from_value(json!({"consumers":[{"name":"example.task","iq":iq}]})).unwrap();
        assert!(decoded.validate().is_err());
        decoded.validate_serving(&[CONSUMER_DECLARATIONS]).unwrap();
        if iq.is_null() {
            assert_eq!(decoded.consumers[0].defaults, DemandDefaults::new());
            assert_eq!(
                serde_json::to_value(&decoded).unwrap(),
                json!({"consumers":[{"name":"example.task"}]})
            );
        }
    }
}

#[test]
fn module_roots_and_attested_selector_admission() {
    for (name, expected) in [
        ("example", true),
        ("example.summary", true),
        ("example_summary", false),
        ("examples.summary", false),
        ("other.summary", false),
    ] {
        assert_eq!(owns_consumer_name("example", name), expected);
    }
    let selectors = vec!["selector".into()];
    assert!(admits_selector("owner", &selectors, "owner", None));
    assert!(admits_selector("owner", &selectors, "selector", None));
    assert!(admits_selector("owner", &selectors, "other", Some("owner")));
    assert!(!admits_selector(
        "owner",
        &selectors,
        "other",
        Some("selector")
    ));
    assert!(!admits_selector("owner", &[], "selector", None));
}

#[test]
fn harness_facts_are_gated_and_never_widen_runner_starts() {
    use cortexkit_role_routing::declarations::{HarnessLimits, RunnerDeclaration, RunnerPair};
    let runner = RunnerDeclaration::new(vec![RunnerPair::new("messages", "api_key")])
        .with_harnesses(vec!["alpha".into()])
        .with_starts_sessions(false)
        .with_harness_limits(
            [(
                "alpha".into(),
                HarnessLimits::new()
                    .with_starts_sessions(true)
                    .with_capabilities(vec![HarnessCapability::OneOff])
                    .with_exclude(vec!["p/m".into()]),
            )]
            .into(),
        );
    assert!(runner
        .typed_harness_limits_serving::<&str>(&[])
        .unwrap()
        .is_err());
    assert!(runner
        .typed_harness_limits_serving(&[capabilities::RUNNER_HARNESS_FACTS])
        .unwrap()
        .is_ok());
    assert_eq!(
        runner
            .limits_for_harness(Some("alpha"))
            .unwrap()
            .starts_sessions,
        Some(false)
    );
    for bad in ["", "-", "p/"] {
        let runner = runner.clone().with_harness_limits(
            [(
                "alpha".into(),
                HarnessLimits::new().with_exclude(vec![bad.into()]),
            )]
            .into(),
        );
        assert!(runner.typed_harness_limits().unwrap().is_err());
    }
}

#[test]
fn select_groups_refuse_undeclared_and_subset_never_expands() {
    for (field, value, group) in [
        ("kind", json!("search"), capabilities::SELECT_KIND),
        (
            "eligible_routes",
            json!([]),
            capabilities::SELECT_ELIGIBLE_ROUTES,
        ),
    ] {
        let mut params = json!({"consumer":"example","requirements":{"iq":50}});
        params[field] = value;
        assert_eq!(
            parse_select_request(&params).unwrap_err().detail["field"],
            field
        );
        assert!(parse_select_request_serving(&params, &[group]).is_ok());
    }
    let mut request =
        parse_select_request_serving(&samples()["request"], capabilities::KNOWN).unwrap();
    let answer: SelectAnswer = serde_json::from_value(samples()["answers"][0].clone()).unwrap();
    request.eligible_routes = Some(vec![]);
    assert!(check_answer(&request, &answer).is_err());
    let mut no_harness = answer.clone();
    no_harness.runner.harness = None;
    assert!(check_answer_serving_with_harnesses(
        &SelectRequest::new("example", Requirements::iq(50)),
        &no_harness,
        &[capabilities::DECISION_HARNESS],
        Some(&["alpha".into()]),
    )
    .is_err());
    assert_eq!(
        SelectRequest::new("example", Requirements::iq(50)).kind(),
        "llm"
    );
    let future: Kind = serde_json::from_value(json!("future_kind")).unwrap();
    assert_eq!(future.as_str(), "future_kind");
    let capability: HarnessCapability = serde_json::from_value(json!("future_task")).unwrap();
    assert_eq!(capability.as_str(), "future_task");
}

fn harness_answer() -> SelectAnswer {
    let route = RouteRef::new("example-provider", "model-a");
    SelectAnswer::new(
        "decision",
        route.clone(),
        RunnerRef::new("runner-one", route),
        100_000,
        10_000,
        60_000,
        "Selected an available route.",
    )
}

#[test]
fn decision_harness_requires_a_declared_program_and_checks_membership() {
    let request = SelectRequest::new("example", Requirements::iq(50));
    let mut answer = harness_answer();
    let declared = vec!["alpha".into(), "beta".into()];
    let groups = &[capabilities::DECISION_HARNESS];
    assert_eq!(
        check_answer_serving_with_harnesses(&request, &answer, groups, Some(&declared)),
        Err("decision_harness requires runner.harness when the runner declares harnesses".into())
    );
    for harness in ["alpha", "beta"] {
        answer.runner.harness = Some(harness.into());
        check_answer_serving_with_harnesses(&request, &answer, groups, Some(&declared)).unwrap();
    }
    for harness in ["unknown", "runner-one", "", "Bad token"] {
        answer.runner.harness = Some(harness.into());
        assert_eq!(
            check_answer_serving_with_harnesses(&request, &answer, groups, Some(&declared)),
            Err("runner.harness must name one of the runner's declared harnesses".into())
        );
    }
    answer.runner.harness = Some("Bad token".into());
    assert!(check_answer_serving_with_harnesses(
        &request,
        &answer,
        groups,
        Some(&["Bad token".into()])
    )
    .is_err());
    answer.runner.harness = None;
    check_answer_serving_with_harnesses(&request, &answer, &[] as &[&str], Some(&declared))
        .unwrap();
}

#[test]
fn decision_harness_omits_program_for_runner_without_declarations() {
    let request = SelectRequest::new("example", Requirements::iq(50));
    let groups = &[capabilities::DECISION_HARNESS];
    for declared in [None, Some([].as_slice())] {
        let mut answer = harness_answer();
        check_answer_serving_with_harnesses(&request, &answer, groups, declared).unwrap();
        for invented in ["runner-one", "invented-program"] {
            answer.runner.harness = Some(invented.into());
            assert_eq!(
                check_answer_serving_with_harnesses(&request, &answer, groups, declared),
                Err("runner.harness is forbidden: runner declares no harnesses".into())
            );
        }
    }
}

#[test]
fn legacy_serving_check_leaves_harness_presence_to_declaration_aware_check() {
    let mut request = SelectRequest::new("example", Requirements::iq(50));
    let mut answer = harness_answer();
    let groups = &[capabilities::DECISION_HARNESS];
    check_answer_serving(&request, &answer, groups).unwrap();
    request.runners = Some(vec!["runner-two".into()]);
    assert!(check_answer_serving(&request, &answer, groups).is_err());
    request.runners = None;
    answer.price = Some(Price::metered(RatesPerMtok::new(3.into(), 15.into())));
    assert_eq!(
        check_answer_serving(&request, &answer, groups),
        Err("price requires select_price".into())
    );
    check_answer_serving_with_harnesses(
        &request,
        &answer,
        &[capabilities::DECISION_HARNESS, capabilities::SELECT_PRICE],
        None,
    )
    .unwrap();
}

#[test]
fn price_rates_are_value_estimates_and_ttl_never_falls_back() {
    let rates = RatesPerMtok::new(3.into(), 15.into())
        .with_cache_write(4.into())
        .with_cache_read(1.into())
        .with_cache_write_ttl([("1h".into(), 6.into())].into());
    assert_eq!(rates.cache_write_rate(Some("1h")), Some(&6.into()));
    assert_eq!(rates.cache_write_rate(Some("12h")), None);
    assert_eq!(rates.cache_write_rate(None), Some(&4.into()));
    let mut metered = Price::metered(rates.clone()).with_estimated_task_usd(1.into());
    metered.check().unwrap();
    metered.multiplier = Some(20.into());
    assert_eq!(metered.check(), Err("price.multiplier"));
    let mut subscription = Price::subscription_equivalent(rates, 20.into());
    subscription.check().unwrap();
    subscription.multiplier = Some(0.into());
    assert!(subscription.check().is_err());
    subscription.multiplier = Some(20.into());
    subscription.rates_per_mtok.output = (-1).into();
    assert!(subscription.check().is_err());
}

#[test]
fn attempt_set_mapping_retry_and_unknown_are_safe() {
    for (status, legacy, penalty) in [
        (AttemptStatus::Served, Outcome::Completed, false),
        (AttemptStatus::ServedEmpty, Outcome::Completed, false),
        (AttemptStatus::Quota, Outcome::Failed, true),
        (AttemptStatus::Credential, Outcome::Failed, true),
        (AttemptStatus::Unavailable, Outcome::Failed, true),
        (AttemptStatus::Rejected, Outcome::Cancelled, false),
    ] {
        let attempt = AttemptOutcome::new(status);
        assert_eq!(attempt.legacy_outcome(), legacy);
        assert_eq!(attempt.counts_against_route(), penalty);
        let report =
            ReportRequest::new("decision", Outcome::Completed).with_attempt_outcome(attempt);
        assert_eq!(
            parse_report_request_serving(
                &serde_json::to_value(&report).unwrap(),
                &[REPORT_ATTEMPT_OUTCOMES]
            )
            .unwrap(),
            report
        );
    }
    let throttled = AttemptOutcome::throttled(0);
    throttled.check(&Outcome::Failed).unwrap();
    let mut rejected = AttemptOutcome::new(AttemptStatus::Rejected);
    rejected.retry_after_ms = Some(1);
    assert_eq!(
        rejected.check(&Outcome::Cancelled).unwrap_err().detail["field"],
        "attempt_outcome.retry_after_ms"
    );
    let unknown =
        json!({"decision_id":"decision","outcome":"failed","attempt_outcome":{"status":"future"}});
    let decoded = parse_report_request_serving(&unknown, &[REPORT_ATTEMPT_OUTCOMES]).unwrap();
    assert!(!decoded.counts_against_route());
    assert!(!decoded.attempt_outcome.unwrap().status.is_known());
    assert!(AttemptOutcome::new(AttemptStatus::Rejected)
        .check(&Outcome::Failed)
        .is_err());
    let invalid = json!({"decision_id":"decision","outcome":"failed","attempt_outcome":{"status":"throttled","retry_after_ms":-1}});
    assert_eq!(
        parse_report_request_serving(&invalid, &[REPORT_ATTEMPT_OUTCOMES])
            .unwrap_err()
            .detail["field"],
        "attempt_outcome.retry_after_ms"
    );
}

#[test]
fn throttled_without_provider_wait_decodes_and_validates() {
    let wire = json!({"decision_id":"decision", "outcome":"failed", "attempt_outcome":{"status":"throttled"}});
    let decoded: ReportRequest = serde_json::from_value(wire.clone()).unwrap();
    let attempt = decoded.attempt_outcome.as_ref().unwrap();
    assert_eq!(attempt.status, AttemptStatus::Throttled);
    assert_eq!(attempt.retry_after_ms, None);
    attempt.check(&Outcome::Failed).unwrap();
    attempt.check_producer().unwrap();
    assert_eq!(
        parse_report_request_serving(&wire, &[REPORT_ATTEMPT_OUTCOMES]).unwrap(),
        decoded
    );
    assert_eq!(serde_json::to_value(decoded).unwrap(), wire);
    AttemptOutcome::new(AttemptStatus::Throttled)
        .check(&Outcome::Failed)
        .unwrap();
}

#[test]
fn provider_wait_is_strict_when_present() {
    for retry in [json!(-1), json!(0.5), json!("10")] {
        let wire = json!({"decision_id":"decision", "outcome":"failed", "attempt_outcome":{"status":"throttled", "retry_after_ms":retry}});
        assert_eq!(
            parse_report_request_serving(&wire, &[REPORT_ATTEMPT_OUTCOMES])
                .unwrap_err()
                .detail["field"],
            "attempt_outcome.retry_after_ms"
        );
    }
    for (status, legacy) in [
        ("served", "completed"),
        ("served_empty", "completed"),
        ("quota", "failed"),
        ("credential", "failed"),
        ("unavailable", "failed"),
        ("rejected", "cancelled"),
        ("future", "cancelled"),
    ] {
        let wire = json!({"decision_id":"decision", "outcome":legacy, "attempt_outcome":{"status":status, "retry_after_ms":0}});
        assert_eq!(
            parse_report_request_serving(&wire, &[REPORT_ATTEMPT_OUTCOMES])
                .unwrap_err()
                .detail["field"],
            "attempt_outcome.retry_after_ms"
        );
    }
}

// The expected bytes come from pre-extension vector bodies, not from a second
// invocation of the new serializer. Hashing those bytes pins fingerprint inputs.
fn unchanged<T: serde::Serialize>(decoded: T, expected: &Value) {
    let encoded = serde_json::to_vec(&serde_json::to_value(decoded).unwrap()).unwrap();
    let old = serde_json::to_vec(expected).unwrap();
    assert_eq!(encoded, old);
    assert_eq!(Sha256::digest(&encoded), Sha256::digest(&old));
}

#[test]
fn omitted_members_preserve_today_sample_body_and_fingerprint_bytes() {
    for text in [include_str!("../test-vectors/routing-v1/route-select.json")] {
        let vectors: Value = serde_json::from_str(text).unwrap();
        for case in vectors["requests"].as_array().unwrap() {
            unchanged(
                parse_select_request(&case["params"]).unwrap(),
                case.get("encodes_as").unwrap_or(&case["params"]),
            );
        }
        for case in vectors["answers"].as_array().unwrap() {
            let body = &case["answer"];
            unchanged(
                serde_json::from_value::<SelectAnswer>(body.clone()).unwrap(),
                body,
            );
        }
    }
    let vectors: Value =
        serde_json::from_str(include_str!("../test-vectors/routing-v1/route-report.json")).unwrap();
    for case in vectors["requests"].as_array().unwrap() {
        unchanged(
            parse_report_request(&case["params"]).unwrap(),
            case.get("encodes_as").unwrap_or(&case["params"]),
        );
    }
    let declarations: Value = serde_json::from_str(include_str!(
        "../test-vectors/routing-v1/consumer-declarations.json"
    ))
    .unwrap();
    for case in declarations["valid"].as_array().unwrap() {
        let decoded = parse_consumer_declarations(&case["declaration"]).unwrap();
        let bytes = serde_json::to_string(&decoded).unwrap();
        assert_eq!(bytes, case["wire"].as_str().unwrap());
        assert_eq!(
            Sha256::digest(bytes.as_bytes()),
            Sha256::digest(case["wire"].as_str().unwrap().as_bytes())
        );
    }
}
