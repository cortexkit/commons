use cortexkit_role_routing::{
    account::{parse_account_for_request, AccountForRequest, CONVERSATION_KEY_MAX_CHARS},
    errors::RefusalCode,
    observe::parse_account_observe_request,
    report::{parse_report_request, EligibilityScope},
};
use serde_json::{json, Value};

#[test]
fn pool_changes_the_admission_body() {
    let bare = AccountForRequest::new("p", "m", "c", "a");
    let mut one = bare.clone();
    one.pool = Some("one".into());
    let mut two = one.clone();
    two.pool = Some("two".into());
    assert!(!bare.same_body(&one));
    assert!(!one.same_body(&two));
    assert!(one.same_body(&one.clone()));
}

#[test]
fn parent_changes_the_admission_body() {
    let bare = AccountForRequest::new("p", "m", "c", "a");
    let mut one = bare.clone();
    one.parent_conversation_key = Some("one".into());
    let mut two = one.clone();
    two.parent_conversation_key = Some("two".into());
    assert!(!bare.same_body(&one));
    assert!(!one.same_body(&two));
    assert!(one.same_body(&one.clone()));
}

#[test]
fn parent_uses_conversation_character_bounds() {
    let base = json!({"provider": "p", "model": "m", "conversation_key": "c", "admission_id": "a"});
    for length in [1, CONVERSATION_KEY_MAX_CHARS] {
        let mut body = base.clone();
        body["parent_conversation_key"] = json!("界".repeat(length));
        assert_eq!(
            parse_account_for_request(&body)
                .unwrap()
                .parent_conversation_key,
            Some("界".repeat(length))
        );
    }
    for invalid in [
        json!(""),
        json!("x".repeat(CONVERSATION_KEY_MAX_CHARS + 1)),
        json!(false),
    ] {
        let mut body = base.clone();
        body["parent_conversation_key"] = invalid;
        let refusal = parse_account_for_request(&body).unwrap_err();
        assert_eq!(refusal.code, RefusalCode::InvalidRequest);
        assert_eq!(refusal.detail["field"], "parent_conversation_key");
    }
}

#[test]
fn pool_grammar_is_shared_by_every_op() {
    for pool in [
        json!("a"),
        json!("a".repeat(64)),
        json!("0123456789._-abcdefghijklmnopqrstuvwxyz"),
        Value::Null,
    ] {
        assert_eq!(pool_results(pool), [None, None, None]);
    }
    for pool in [
        json!(""),
        json!("a".repeat(65)),
        json!("A"),
        json!("é"),
        json!("a/b"),
        json!("a b"),
        json!("a\nb"),
        json!(42),
        json!([]),
    ] {
        assert_eq!(
            pool_results(pool),
            [
                Some("pool".into()),
                Some("pool".into()),
                Some("pool".into())
            ]
        );
    }
}

fn pool_results(pool: Value) -> [Option<String>; 3] {
    let refusals = [
        parse_account_for_request(&json!({"provider": "p", "model": "m", "conversation_key": "c", "admission_id": "a", "pool": pool})).err(),
        parse_account_observe_request(&json!({"event_id": "e", "kind": "attempt", "pool": pool})).err(),
        parse_report_request(&json!({"decision_id": "d", "outcome": "completed", "pool": pool})).err(),
    ];
    refusals.map(|refusal| {
        refusal.map(|refusal| {
            assert_eq!(refusal.code, RefusalCode::InvalidRequest);
            refusal.detail["field"].as_str().unwrap().to_owned()
        })
    })
}

#[test]
fn eligibility_scope_is_open() {
    let body = json!({"decision_id": "d", "outcome": "refused", "eligibility": {"scope": "future", "code": "provider_code"}});
    let request = parse_report_request(&body).unwrap();
    assert_eq!(
        request.eligibility.as_ref().unwrap().scope,
        EligibilityScope::Other("future".into())
    );
    assert_eq!(serde_json::to_value(request).unwrap(), body);
}

#[test]
fn eligibility_conflicts_name_eligibility() {
    for code in [
        "account_limit_exhausted",
        "account_pool_exhausted",
        "quota_exhausted",
        "credential_billing_exhausted",
    ] {
        let refusal = parse_report_request(&json!({"decision_id": "d", "outcome": "refused", "runner_code": code, "eligibility": {"scope": "model", "code": "not_included"}})).unwrap_err();
        assert_eq!(refusal.code, RefusalCode::InvalidRequest);
        assert_eq!(refusal.detail["field"], "eligibility");
    }
    let refusal = parse_report_request(&json!({"decision_id": "d", "outcome": "refused", "reset_at_ms": 0, "eligibility": {"scope": "account", "code": "disabled"}})).unwrap_err();
    assert_eq!(refusal.code, RefusalCode::InvalidRequest);
    assert_eq!(refusal.detail["field"], "eligibility");
    assert!(parse_report_request(&json!({"decision_id": "d", "outcome": "refused", "runner_code": "model_unavailable", "reset_at_ms": null, "eligibility": {"scope": "model", "code": "not_included"}})).is_ok());
}

#[test]
fn report_members_are_part_of_the_body() {
    let base = json!({"decision_id": "d", "outcome": "refused"});
    for (field, one, two) in [
        ("pool", json!("one"), json!("two")),
        (
            "eligibility",
            json!({"scope": "model", "code": "one"}),
            json!({"scope": "model", "code": "two"}),
        ),
        (
            "eligibility",
            json!({"scope": "model", "code": "one"}),
            json!({"scope": "account", "code": "one"}),
        ),
        ("request_input_tokens", json!(0), json!(1)),
    ] {
        let mut a = base.clone();
        let mut b = base.clone();
        a[field] = one;
        b[field] = two;
        let a = parse_report_request(&a).unwrap();
        let b = parse_report_request(&b).unwrap();
        assert_ne!(a, b, "{field}");
        assert_ne!(parse_report_request(&base).unwrap(), a, "{field}");
    }
}
