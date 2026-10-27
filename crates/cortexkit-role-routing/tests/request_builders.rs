use std::{collections::BTreeMap, fmt::Debug};

use cortexkit_role_routing::{
    account::{
        parse_account_for_request, AccountForRequest, AccountMode, CandidateAccount, Candidates,
        ExpectedPin, RequestedAccount,
    },
    capabilities::{SELECT_EXCLUDE_PROVIDERS, SELECT_HARNESS, SELECT_HOLD_CONTROL, SELECT_PREFER},
    errors::Refusal,
    observe::{parse_account_observe_request, AccountObserveRequest, ObserveKind},
    report::{parse_report_request, Eligibility, EligibilityScope, Outcome, ReportRequest, Usage},
    select::{
        parse_select_request_serving, ConcurrencySlot, PinFallback, Requirements, RouteRef,
        RunnerAuth, RunnerRef, SelectRequest,
    },
};
use serde::{de::DeserializeOwned, Serialize};
use serde_json::{json, Value};

fn round_trip<T>(request: T, expected: Value, parse: fn(&Value) -> Result<T, Refusal>)
where
    T: Serialize + DeserializeOwned + PartialEq + Debug,
{
    assert_eq!(serde_json::to_value(&request).unwrap(), expected);
    assert_eq!(
        serde_json::from_value::<T>(expected.clone()).unwrap(),
        request
    );
    assert_eq!(parse(&expected).unwrap(), request);
}

fn parse_with_hold_control(params: &Value) -> Result<SelectRequest, Refusal> {
    parse_select_request_serving(
        params,
        &[SELECT_HOLD_CONTROL, SELECT_PREFER, SELECT_EXCLUDE_PROVIDERS],
    )
}

#[test]
fn select_builds_and_round_trips_from_outside_the_crate() {
    let select = SelectRequest::new("example_summary", Requirements::iq(50))
        .with_pin("example-provider/model-a:fast")
        .with_pin_fallback(PinFallback::Route)
        .with_exclude(vec!["example-provider/model-b".into()])
        .with_exclude_providers(vec!["example-subscription".into()])
        .with_runners(vec!["runner-one".into()])
        .with_affinity("conversation-1")
        .with_correlation_id("work-1")
        .with_concurrency_slots(vec![ConcurrencySlot {
            key: "runner-one".into(),
            running: 1,
            limit: 2,
            queued: None,
        }])
        .with_detail(true)
        .with_hold_ok(false)
        .with_hold_wait_max_s(120)
        .with_prefer(vec!["example-provider/model-b".into()]);
    assert!(!select.hold_ok());
    assert!(select.excludes_provider("example-subscription"));
    assert!(!select.excludes_provider("example-provider"));
    round_trip(
        select,
        json!({
            "consumer": "example_summary", "requirements": {"iq": 50},
            "pin": "example-provider/model-a:fast", "pin_fallback": "route", "exclude": ["example-provider/model-b"],
            "exclude_providers": ["example-subscription"],
            "runners": ["runner-one"], "affinity": "conversation-1",
            "correlation_id": "work-1",
            "concurrency_slots": [{"key": "runner-one", "running": 1, "limit": 2}],
            "detail": true, "hold_ok": false, "hold_wait_max_s": 120,
            "prefer": ["example-provider/model-b"]
        }),
        parse_with_hold_control,
    );
    assert!(SelectRequest::new("c", Requirements::iq(1)).hold_ok());
    round_trip(
        SelectRequest::new("c", Requirements::iq(1)).with_harness("beta"),
        json!({"consumer": "c", "requirements": {"iq": 1}, "harness": "beta"}),
        |params| parse_select_request_serving(params, &[SELECT_HARNESS]),
    );

    let runner = RunnerRef::new("runner-one", RouteRef::new("p", "m"))
        .with_auth(RunnerAuth {
            pool: Some("keys".into()),
            credential_id: None,
        })
        .with_harness("example-harness");
    assert_eq!(
        serde_json::to_value(&runner).unwrap(),
        json!({"module": "runner-one", "model": {"provider": "p", "model": "m"},
               "auth": {"pool": "keys"}, "harness": "example-harness"})
    );
    assert_eq!(
        serde_json::to_value(RunnerRef::new("runner-one", RouteRef::new("p", "m"))).unwrap(),
        json!({"module": "runner-one", "model": {"provider": "p", "model": "m"}})
    );
}

#[test]
fn requests_build_and_round_trip_from_outside_the_crate() {
    let account = AccountForRequest::new("example-subscription", "model-c", "child", "admission-1")
        .with_affinity_model("model-d")
        .with_mode(AccountMode::Renew)
        .with_expected_pin(ExpectedPin {
            id: "pin-1".into(),
            generation: 3,
        })
        .with_initial_input_bytes(512)
        .with_initial_input_estimated(true)
        .with_candidates(Candidates {
            view_digest: "view-1".into(),
            accounts: vec![CandidateAccount {
                account_id: "account-1".into(),
                credential_id: "credential-1".into(),
            }],
        })
        .with_requested_account(RequestedAccount {
            account_id: Some("account-1".into()),
            credential_id: Some("credential-1".into()),
        })
        .with_hold_wait_max_s(30)
        .with_decision_id("decision-1")
        .with_parent_conversation_key("parent")
        .with_pool("model.pool_1-x");
    round_trip(
        account,
        json!({
            "provider": "example-subscription", "model": "model-c",
            "conversation_key": "child", "admission_id": "admission-1",
            "affinity_model": "model-d", "mode": "renew",
            "expected_pin": {"id": "pin-1", "generation": 3},
            "initial_input_bytes": 512, "initial_input_estimated": true,
            "candidates": {"view_digest": "view-1", "accounts": [{
                "account_id": "account-1", "credential_id": "credential-1"
            }]},
            "requested_account": {"account_id": "account-1", "credential_id": "credential-1"},
            "hold_wait_max_s": 30, "decision_id": "decision-1",
            "parent_conversation_key": "parent", "pool": "model.pool_1-x"
        }),
        parse_account_for_request,
    );

    let observation = AccountObserveRequest::new("event-1", ObserveKind::Attempt)
        .with_admission_id("admission-1")
        .with_run_id("run-1")
        .with_served_account_id("account-1")
        .with_credential_id("credential-1")
        .with_view_digest("view-1")
        .with_observed_at_ms(1000)
        .with_upstream_origin("https://provider.example")
        .with_headers(BTreeMap::from([(
            "x-ratelimit-remaining".into(),
            "4".into(),
        )]))
        .with_limit_window("short")
        .with_resets_at_ms(2000)
        .with_resets_at_source("headers")
        .with_pool("model.pool_1-x");
    round_trip(
        observation,
        json!({
            "event_id": "event-1", "kind": "attempt",
            "admission_id": "admission-1", "run_id": "run-1",
            "served_account_id": "account-1", "credential_id": "credential-1",
            "view_digest": "view-1", "observed_at_ms": 1000,
            "upstream_origin": "https://provider.example",
            "headers": {"x-ratelimit-remaining": "4"}, "limit_window": "short",
            "resets_at_ms": 2000, "resets_at_source": "headers", "pool": "model.pool_1-x"
        }),
        parse_account_observe_request,
    );

    let exhaustion = ReportRequest::new("decision-1", Outcome::Refused)
        .with_runner_code("account_limit_exhausted")
        .with_credential_id("credential-1")
        .with_reset_at_ms(2000)
        .with_usage(
            Usage::new()
                .with_input_tokens(100)
                .with_output_tokens(20)
                .with_duration_ms(1500)
                .with_first_token_ms(300)
                .with_cost_usd(0.5),
        )
        .with_pool("model.pool_1-x")
        .with_request_input_tokens(150);
    round_trip(
        exhaustion,
        json!({
            "decision_id": "decision-1", "outcome": "refused",
            "runner_code": "account_limit_exhausted", "credential_id": "credential-1",
            "reset_at_ms": 2000,
            "usage": {"input_tokens": 100, "output_tokens": 20, "duration_ms": 1500, "first_token_ms": 300, "cost_usd": 0.5},
            "pool": "model.pool_1-x", "request_input_tokens": 150
        }),
        parse_report_request,
    );

    let eligibility =
        ReportRequest::new("decision-2", Outcome::Refused).with_eligibility(Eligibility {
            scope: EligibilityScope::Model,
            code: "model_not_included".into(),
        });
    round_trip(
        eligibility,
        json!({
            "decision_id": "decision-2", "outcome": "refused",
            "eligibility": {"scope": "model", "code": "model_not_included"}
        }),
        parse_report_request,
    );
}
