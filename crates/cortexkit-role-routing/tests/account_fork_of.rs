use cortexkit_role_routing::{
    account::{parse_account_for_request_serving, AccountForRequest, ForkOf},
    capabilities::ACCOUNT_FORK_OF,
};
use serde_json::{json, Value};

fn request() -> Value {
    json!({
        "provider": "example-subscription", "model": "model-c",
        "conversation_key": "fork", "admission_id": "fork-admission",
        "fork_of": {"admission_id": "parent-admission"}
    })
}

#[test]
fn fork_of_is_optional_strict_and_capability_gated() {
    let value = request();
    let parse = |value: &Value| parse_account_for_request_serving(value, &[ACCOUNT_FORK_OF]);
    let parsed = parse(&value).unwrap();
    assert_eq!(parsed.fork_of, Some(ForkOf::new("parent-admission")));
    let refusal = parse_account_for_request_serving::<&str>(&value, &[]).unwrap_err();
    assert_eq!(refusal.code.as_str(), "invalid_request");
    assert_eq!(refusal.detail["field"], "fork_of");
    for parent in [
        json!({}),
        json!({"admission_id": ""}),
        json!({"admission_id": "a".repeat(129)}),
        json!({"admission_id": 3}),
        json!({"admission_id": "parent", "extra": true}),
        json!("parent"),
    ] {
        let mut malformed = value.clone();
        malformed["fork_of"] = parent;
        let refusal = parse(&malformed).unwrap_err();
        assert_eq!(refusal.code.as_str(), "invalid_request");
        assert_eq!(refusal.detail["field"], "fork_of");
    }
    let mut boundary = value;
    boundary["fork_of"]["admission_id"] = json!("é".repeat(128));
    assert!(parse(&boundary).is_ok());
    boundary["fork_of"] = Value::Null;
    assert_eq!(parse(&boundary).unwrap().fork_of, None);
    assert!(serde_json::from_value::<ForkOf>(json!({
        "admission_id": "parent", "extra": true
    }))
    .is_err());
}

#[test]
fn fork_of_refuses_exclusions_and_every_non_admit_mode() {
    for (field, value) in [
        ("requested_account", json!({"account_id": "a"})),
        ("warm_credential", json!("credential")),
        ("parent_conversation_key", json!("conversation")),
        ("candidates", json!({"view_digest": "v", "accounts": []})),
        ("mode", json!("renew")),
        ("mode", json!("reassign")),
        ("mode", json!("future-mode")),
    ] {
        let mut value_request = request();
        value_request[field] = value;
        let refusal =
            parse_account_for_request_serving(&value_request, &[ACCOUNT_FORK_OF]).unwrap_err();
        assert_eq!(refusal.code.as_str(), "invalid_request");
        assert_eq!(refusal.detail["field"], "fork_of", "{field}");
    }
    let mut explicit = request();
    explicit["mode"] = json!("admit");
    assert!(parse_account_for_request_serving(&explicit, &[ACCOUNT_FORK_OF]).is_ok());
    let mut continuation = request();
    continuation["mode"] = json!("continue");
    let describe = cortexkit_role_routing::describe::RoleDescribe::new(vec![], false)
        .with_capability(cortexkit_role_routing::capabilities::ACCOUNT_CONTINUE)
        .with_capability(ACCOUNT_FORK_OF);
    let refusal =
        cortexkit_role_routing::account_continue::parse_continue_request(&continuation, &describe)
            .unwrap_err();
    assert_eq!(refusal.code.as_str(), "invalid_request");
    assert_eq!(refusal.detail["field"], "fork_of");
}

#[test]
fn fork_of_participates_in_same_body_and_legacy_wire_is_unchanged() {
    assert_eq!(ACCOUNT_FORK_OF, "account_fork_of");
    assert!(cortexkit_role_routing::capabilities::KNOWN.contains(&ACCOUNT_FORK_OF));
    let bare = AccountForRequest::new("provider", "model", "conversation", "admission");
    assert!(serde_json::to_value(&bare)
        .unwrap()
        .get("fork_of")
        .is_none());
    let fork = bare.clone().with_fork_of(ForkOf::new("parent"));
    assert!(!bare.same_body(&fork));
    assert!(fork.same_body(&fork.clone()));
    assert!(!fork.same_body(&bare.with_fork_of(ForkOf::new("other-parent"))));
}
