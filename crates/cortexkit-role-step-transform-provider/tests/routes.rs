use cortexkit_role_step_transform_provider::{
    errors::{ROUTE_UNBOUND, UNATTESTED_RUNNER, UNKNOWN_ROUTE_ROLE},
    ops,
    route::{Route, SessionBinding},
    subscription::{Declaration, DeclareRequest, DeclaredSubscription, Hook, OnUnavailable, Op},
    PROVIDES,
};
use serde_json::{json, Map, Value};

fn module_route() -> Route {
    Route::new(Some("runner-a".into()), vec![PROVIDES.into()])
}

#[test]
fn route_vectors_enforce_per_call_session_binding() {
    let vectors: Value = serde_json::from_str(include_str!(
        "../test-vectors/step-transform-provider-v2/routes.json"
    ))
    .unwrap();
    for case in vectors["cases"].as_array().unwrap() {
        let mut route = module_route();
        route.scoped = case["scoped"].as_bool().unwrap();
        if case["binding"].is_object() {
            route.session_binding = Some(SessionBinding::new(
                case["binding"]["harness"].as_str().unwrap(),
                case["binding"]["session"].as_str().unwrap(),
            ));
        }
        assert_eq!(route.check_bind(), Ok(()), "{}", case["name"]);
        let result = route.check_call(case["method"].as_str().unwrap());
        match case.get("refusal") {
            Some(refusal) => assert_eq!(
                serde_json::to_value(result.unwrap_err()).unwrap(),
                *refusal,
                "{}",
                case["name"]
            ),
            None => assert_eq!(result, Ok(()), "{}", case["name"]),
        }
        assert_eq!(route.check_call(ops::ROLE_DESCRIBE), Ok(()));
        assert_eq!(route.check_call(ops::TRANSFORM_DECLARE), Ok(()));
    }
}

#[test]
fn route_admission_reuses_compaction_refusals() {
    let mut route = module_route();
    route.attested_runner = None;
    let refusal = route.check_bind().unwrap_err();
    assert_eq!(refusal.code, UNATTESTED_RUNNER);
    assert_eq!(refusal.message, "No caller module attested");
    for roles in [
        vec![],
        vec!["another-role/v1".into()],
        vec![PROVIDES.into(), "another-role/v1".into()],
    ] {
        let mut route = module_route();
        route.roles = roles;
        let refusal = route.check_bind().unwrap_err();
        assert_eq!(refusal.code, UNKNOWN_ROUTE_ROLE);
        assert_eq!(refusal.message, "Expected exactly one served role");
    }
    assert_eq!(
        module_route()
            .check_call(ops::TRANSFORM_HOOK)
            .unwrap_err()
            .code,
        ROUTE_UNBOUND
    );
}

#[test]
fn plan_composer_preflight_keeps_declaration_policies_and_hook_coverage() {
    let request = DeclareRequest::new(Map::new()).with_preset("worker");
    assert_eq!(
        serde_json::to_value(request).unwrap(),
        json!({"preset":"worker","params":{}})
    );
    let declaration = Declaration::new(vec![
        DeclaredSubscription::new(Hook::PreUser, vec![Op::Append], 120),
        DeclaredSubscription::new(Hook::PostTool, vec![Op::Prepend], 250)
            .with_on_unavailable(OnUnavailable::Pass),
    ]);
    assert!(declaration.check().is_empty());
    assert!(declaration.covered_by_step_hooks(&[Hook::PreUser, Hook::PostTool]));
    assert!(!declaration.covered_by_step_hooks(&[Hook::PreUser]));
    let policies: Vec<_> = declaration
        .subscriptions
        .iter()
        .map(|s| (s.hook.clone(), s.effective_on_unavailable(), s.budget_ms))
        .collect();
    assert_eq!(
        policies,
        [
            (Hook::PreUser, OnUnavailable::Refuse, 120),
            (Hook::PostTool, OnUnavailable::Pass, 250)
        ]
    );
}
