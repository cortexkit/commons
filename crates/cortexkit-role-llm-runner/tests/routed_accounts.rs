use cortexkit_role_llm_runner::{
    capabilities,
    describe::{
        check_describe, check_routed_accounts, problem_names, DescribeProblem, RoleDescribe,
    },
    PROVIDES, REQUIRED_OPS,
};
use serde_json::{json, Value};

fn describe() -> Value {
    json!({
        "majors": [{"version": PROVIDES, "ops": REQUIRED_OPS}],
        "implementation_version": "fixture",
        "capabilities": [capabilities::ROUTED_ACCOUNTS],
        "session_capabilities_from": "admission",
        "routed_accounts": ["apikey"]
    })
}

#[test]
fn routed_accounts_accept_open_tokens_and_list_boundaries() {
    for method in ["a", "apikey", "oauth", "a0._-", &"a".repeat(64)] {
        assert_eq!(check_routed_accounts(&[method.into()]), Ok(()));
    }
    let methods: Vec<_> = (0..64).map(|i| format!("method_{i}")).collect();
    assert_eq!(check_routed_accounts(&methods), Ok(()));
    let mut body = describe();
    body["routed_accounts"] = json!(methods);
    assert!(check_describe(&body).is_ok());
    assert!(capabilities::GROUPS.contains(&(capabilities::ROUTED_ACCOUNTS, &[][..])));
}

#[test]
fn routed_accounts_refuse_empty_long_and_duplicate_lists() {
    let cases = [
        (vec![], problem_names::ROUTED_ACCOUNTS_EMPTY),
        (
            (0..65).map(|i| format!("method_{i}")).collect(),
            problem_names::ROUTED_ACCOUNTS_TOO_MANY,
        ),
        (
            vec!["apikey".into(), "oauth".into(), "apikey".into()],
            problem_names::ROUTED_ACCOUNTS_DUPLICATE,
        ),
    ];
    for (methods, reason) in cases {
        assert_eq!(check_routed_accounts(&methods), Err(reason));
        let mut body = describe();
        body["routed_accounts"] = json!(methods);
        assert_eq!(
            check_describe(&body),
            Err(vec![DescribeProblem::Undecodable(reason.into())])
        );
    }
}

#[test]
fn routed_accounts_refuse_invalid_auth_method_tokens() {
    for method in [
        "",
        "A",
        "ApiKey",
        "0api",
        ".api",
        "_api",
        "-api",
        "api key",
        "api/key",
        "api\n",
        "é",
        "aé",
        &"a".repeat(65),
    ] {
        let methods = vec![method.into()];
        assert_eq!(
            check_routed_accounts(&methods),
            Err(problem_names::ROUTED_ACCOUNTS_INVALID_AUTH_METHOD),
            "{method:?}"
        );
        let mut body = describe();
        body["routed_accounts"] = json!(methods);
        assert_eq!(
            check_describe(&body),
            Err(vec![DescribeProblem::Undecodable(
                problem_names::ROUTED_ACCOUNTS_INVALID_AUTH_METHOD.into()
            )])
        );
    }
}

#[test]
fn routed_accounts_pair_metadata_with_group_and_name_wrong_wire_types() {
    let mut body = describe();
    assert!(check_describe(&body).is_ok());
    body.as_object_mut().unwrap().remove("routed_accounts");
    assert_eq!(
        check_describe(&body),
        Err(vec![DescribeProblem::Undecodable(
            problem_names::ROUTED_ACCOUNTS_MISSING.into()
        )])
    );
    body["routed_accounts"] = json!(["apikey"]);
    body["capabilities"] = json!([]);
    assert_eq!(
        check_describe(&body),
        Err(vec![DescribeProblem::Undecodable(
            problem_names::ROUTED_ACCOUNTS_UNDECLARED.into()
        )])
    );
    body["capabilities"] = json!([capabilities::ROUTED_ACCOUNTS]);
    for value in [json!("apikey"), json!([1]), json!({}), json!(true)] {
        body["routed_accounts"] = value;
        assert_eq!(
            check_describe(&body),
            Err(vec![DescribeProblem::Undecodable(
                problem_names::INVALID_ROUTED_ACCOUNTS.into()
            )])
        );
    }
}

#[test]
fn routed_accounts_absence_stays_omitted_and_builder_round_trips() {
    let mut legacy = describe();
    legacy["capabilities"] = json!([]);
    legacy.as_object_mut().unwrap().remove("routed_accounts");
    let legacy = check_describe(&legacy).unwrap();
    assert_eq!(legacy.routed_accounts, None);
    assert!(serde_json::to_value(&legacy)
        .unwrap()
        .get("routed_accounts")
        .is_none());
    let routed = legacy
        .with_capabilities(vec![capabilities::ROUTED_ACCOUNTS.into()])
        .with_routed_accounts(vec!["apikey".into(), "future-method".into()]);
    let encoded = serde_json::to_value(&routed).unwrap();
    assert_eq!(check_describe(&encoded).unwrap(), routed);
    assert_eq!(
        serde_json::from_value::<RoleDescribe>(encoded).unwrap(),
        routed
    );
}
