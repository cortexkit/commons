use cortexkit_role_llm_runner::{
    capabilities,
    describe::{check_describe, problem_names, DescribeProblem},
    ops,
    send::{AccountRoute, SendRequest},
    PROVIDES, REQUIRED_OPS,
};
use serde_json::json;

#[test]
fn account_route_requires_routed_accounts_by_name() {
    let mut body = json!({
        "majors": [{"version": PROVIDES, "ops": REQUIRED_OPS}],
        "implementation_version": "fixture",
        "capabilities": [capabilities::ACCOUNT_ROUTE],
        "session_capabilities_from": "admission"
    });
    assert!(check_describe(&body)
        .unwrap_err()
        .contains(&DescribeProblem::Undecodable(
            problem_names::ACCOUNT_ROUTE_WITHOUT_ROUTED_ACCOUNTS.into()
        )));
    body["capabilities"] = json!([capabilities::ACCOUNT_ROUTE, capabilities::ROUTED_ACCOUNTS]);
    body["routed_accounts"] = json!(["apikey"]);
    body["majors"][0]["ops"] = json!([REQUIRED_OPS, &[ops::SESSION_SEND]].concat());
    assert!(check_describe(&body).is_ok());
    assert_eq!(
        capabilities::group_ops(capabilities::ACCOUNT_ROUTE),
        Some(&[ops::SESSION_SEND][..])
    );
    body["majors"][0]["ops"] = json!(REQUIRED_OPS);
    assert!(check_describe(&body)
        .unwrap_err()
        .contains(&DescribeProblem::GroupIncomplete {
            capability: capabilities::ACCOUNT_ROUTE,
            op: ops::SESSION_SEND,
        }));
}

#[test]
fn account_route_is_typed_and_round_trips() {
    let wire = json!({"prompt":"p", "send_id":"s",
        "route_decision_id":"decision", "account_route":{"provider":"pool", "model":"route-model"}});
    let request: SendRequest = serde_json::from_value(wire.clone()).unwrap();
    let route = AccountRoute::new("pool", "route-model");
    assert_eq!(request.account_route, Some(route.clone()));
    assert!(!request.runner_params.contains_key("account_route"));
    assert_eq!(serde_json::to_value(request).unwrap(), wire);
    assert_eq!(
        SendRequest::new("p", "s")
            .with_account_route(route.clone())
            .account_route,
        Some(route)
    );
}

#[test]
fn account_route_absence_preserves_request_bytes() {
    let bytes = br#"{"prompt":"p","send_id":"s"}"#;
    let request: SendRequest = serde_json::from_slice(bytes).unwrap();
    assert_eq!(request.account_route, None);
    assert_eq!(serde_json::to_vec(&request).unwrap(), bytes);
    assert_eq!(
        serde_json::to_vec(&SendRequest::new("p", "s")).unwrap(),
        bytes
    );
}

#[test]
fn account_route_rejects_malformed_values_by_field() {
    for value in [
        json!(null),
        json!(true),
        json!("pool"),
        json!([]),
        json!({}),
        json!({"provider":"pool"}),
        json!({"model":"m"}),
        json!({"provider":1,"model":"m"}),
        json!({"provider":"pool","model":false}),
        json!({"provider":"","model":"m"}),
        json!({"provider":"pool","model":""}),
        json!({"provider":"pool","model":"m","extra":true}),
    ] {
        let error = serde_json::from_value::<SendRequest>(
            json!({"prompt":"p","send_id":"s","account_route":value}),
        )
        .unwrap_err();
        assert!(
            error.to_string().contains("account_route"),
            "{value}: {error}"
        );
    }
    for route in [AccountRoute::new("", "m"), AccountRoute::new("pool", "")] {
        assert_eq!(
            SendRequest::new("p", "s")
                .with_account_route(route)
                .check_account_route(None),
            Err("account_route")
        );
    }
}

#[test]
fn account_route_names_follow_routing_nonempty_strings() {
    for name in [
        "vendor/model:variant",
        "Mixed Case",
        "模型",
        &"a".repeat(1024),
    ] {
        let wire = json!({"provider":name,"model":name});
        let route: AccountRoute = serde_json::from_value(wire.clone()).unwrap();
        assert_eq!(route.check(), Ok(()));
        assert_eq!(serde_json::to_value(route).unwrap(), wire);
    }
}

#[test]
fn account_route_is_fixed_per_run_and_omission_inherits() {
    let route = AccountRoute::new("pool", "route-model");
    let send = SendRequest::new("p", "s").with_account_route(route.clone());
    assert_eq!(send.check_account_route(None), Ok(()));
    assert_eq!(send.check_account_route(Some(Some(&route))), Ok(()));
    assert_eq!(send.check_account_route(Some(None)), Err("account_route"));
    for different in [
        AccountRoute::new("other", "route-model"),
        AccountRoute::new("pool", "other"),
    ] {
        assert_eq!(
            send.check_account_route(Some(Some(&different))),
            Err("account_route")
        );
    }
    let omitted = SendRequest::new("join", "j");
    assert_eq!(omitted.check_account_route(Some(Some(&route))), Ok(()));
    assert_eq!(omitted.check_account_route(Some(None)), Ok(()));
}
