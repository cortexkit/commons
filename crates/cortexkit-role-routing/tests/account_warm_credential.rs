use cortexkit_role_routing::{
    account::{parse_account_for_request, parse_account_for_request_serving, AccountForRequest},
    account_continue::parse_continue_request,
    capabilities::{self, ACCOUNT_WARM_CREDENTIAL},
    describe::RoleDescribe,
};
use serde_json::{json, Value};

fn body() -> Value {
    json!({"provider":"example", "model":"model", "conversation_key":"conversation", "admission_id":"admission"})
}

#[test]
fn warm_credential_builder_round_trips_and_uses_public_opaque_id_rules() {
    let credential = "vault:oauth:account/界";
    let request = AccountForRequest::new("example", "model", "conversation", "admission")
        .with_warm_credential(credential);
    let wire = serde_json::to_value(&request).unwrap();
    assert_eq!(wire["warm_credential"], credential);
    assert_eq!(
        parse_account_for_request_serving(&wire, &[ACCOUNT_WARM_CREDENTIAL]).unwrap(),
        request
    );
    assert!(capabilities::KNOWN.contains(&ACCOUNT_WARM_CREDENTIAL));
    for invalid in [json!(""), json!(42), json!({}), json!([]), json!(false)] {
        let mut params = body();
        params["warm_credential"] = invalid;
        let refusal =
            parse_account_for_request_serving(&params, &[ACCOUNT_WARM_CREDENTIAL]).unwrap_err();
        assert_eq!(refusal.code.as_str(), "invalid_request");
        assert_eq!(refusal.detail["field"], "warm_credential");
    }
}

#[test]
fn warm_credential_requires_capability_and_admit_mode() {
    let mut params = body();
    params["warm_credential"] = json!("credential-2");
    assert_eq!(
        parse_account_for_request(&params).unwrap_err().detail["field"],
        "warm_credential"
    );
    for mode in ["renew", "reassign", "continue", "future"] {
        params["mode"] = json!(mode);
        params["expected_pin"] = json!({"id":"pin", "generation":1});
        let refusal =
            parse_account_for_request_serving(&params, &[ACCOUNT_WARM_CREDENTIAL]).unwrap_err();
        assert_eq!(refusal.code.as_str(), "invalid_request");
        assert_eq!(refusal.detail["field"], "warm_credential", "{mode}");
    }
    let describe = RoleDescribe::new(vec![], true)
        .with_capability(capabilities::ACCOUNT_CONTINUE)
        .with_capability(ACCOUNT_WARM_CREDENTIAL);
    params["mode"] = json!("continue");
    assert_eq!(
        parse_continue_request(&params, &describe)
            .unwrap_err()
            .detail["field"],
        "warm_credential"
    );
}

#[test]
fn warm_credential_changes_body_but_omission_preserves_legacy_fingerprint_bytes() {
    let bare = parse_account_for_request(&body()).unwrap();
    let legacy = r#"{"provider":"example","model":"model","affinity_model":"model","conversation_key":"conversation","admission_id":"admission","mode":"admit","initial_input_bytes":0,"initial_input_estimated":false,"hold_wait_max_s":900}"#;
    // These canonical bytes predate the hint: hash inputs must not gain a null member.
    assert_eq!(serde_json::to_string(&bare.normalized()).unwrap(), legacy);
    let mut null = body();
    null["warm_credential"] = Value::Null;
    assert_eq!(parse_account_for_request(&null).unwrap(), bare);
    let one = bare.clone().with_warm_credential("credential-1");
    let two = bare.clone().with_warm_credential("credential-2");
    assert!(!bare.same_body(&one));
    assert!(!one.same_body(&two));
    assert!(one.same_body(&one.clone()));
    let mut explicit_admit = serde_json::to_value(&one).unwrap();
    explicit_admit["mode"] = json!("admit");
    assert!(one.same_body(
        &parse_account_for_request_serving(&explicit_admit, &[ACCOUNT_WARM_CREDENTIAL]).unwrap()
    ));
}
