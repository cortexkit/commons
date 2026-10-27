use cortexkit_role_routing::{
    account::{parse_account_for_request, AccountForRequest, CONVERSATION_KEY_MAX_CHARS},
    capabilities::{SELECT_HOLD_CONTROL, SELECT_PREFER},
    describe::RoleDescribe,
    errors::RefusalCode,
    observe::parse_account_observe_request,
    report::{parse_report_request, EligibilityScope, MAX_USAGE_COUNT},
    select::{parse_select_request, parse_select_request_serving, PinFallback, PinOutcome},
};
use serde_json::{json, Value};

fn select_with(members: Value) -> Value {
    let mut params = json!({"consumer": "example_summary", "requirements": {"iq": 50}});
    params
        .as_object_mut()
        .unwrap()
        .extend(members.as_object().unwrap().clone());
    params
}

#[test]
fn hold_members_decode_each_value_with_the_group() {
    let group = [SELECT_HOLD_CONTROL];
    for (members, hold_ok, cap) in [
        (json!({}), true, None),
        (json!({"hold_ok": true}), true, None),
        (json!({"hold_ok": false}), false, None),
        (json!({"hold_wait_max_s": 0}), true, Some(0)),
        (json!({"hold_wait_max_s": 300}), true, Some(300)),
        (json!({"hold_wait_max_s": u64::MAX}), true, Some(u64::MAX)),
        (
            json!({"hold_ok": true, "hold_wait_max_s": 60}),
            true,
            Some(60),
        ),
        // Valid together: the answer a caller that takes no hold falls back
        // to may still hold, and the cap applies to that hold.
        (
            json!({"hold_ok": false, "hold_wait_max_s": 60}),
            false,
            Some(60),
        ),
        (
            json!({"hold_ok": false, "hold_wait_max_s": 0}),
            false,
            Some(0),
        ),
        (
            json!({"hold_ok": null, "hold_wait_max_s": null}),
            true,
            None,
        ),
    ] {
        let request = parse_select_request_serving(&select_with(members.clone()), &group)
            .unwrap_or_else(|refusal| panic!("{members}: {refusal}"));
        assert_eq!(request.hold_ok(), hold_ok, "{members}");
        assert_eq!(request.hold_wait_max_s, cap, "{members}");
    }
    for (members, field) in [
        (json!({"hold_ok": 0}), "hold_ok"),
        (json!({"hold_ok": "true"}), "hold_ok"),
        (json!({"hold_wait_max_s": -1}), "hold_wait_max_s"),
        (json!({"hold_wait_max_s": 0.5}), "hold_wait_max_s"),
        (json!({"hold_wait_max_s": true}), "hold_wait_max_s"),
    ] {
        let refusal = parse_select_request_serving(&select_with(members.clone()), &group)
            .expect_err(&members.to_string());
        assert_eq!(refusal.code, RefusalCode::InvalidRequest, "{members}");
        assert_eq!(refusal.detail["field"], field, "{members}");
    }
}

#[test]
fn hold_members_are_refused_by_name_without_the_group() {
    let describe = RoleDescribe::new(vec![], false);
    assert!(!describe.serves(SELECT_HOLD_CONTROL));
    for (members, field) in [
        (json!({"hold_ok": true}), "hold_ok"),
        (json!({"hold_ok": false}), "hold_ok"),
        (json!({"hold_wait_max_s": 0}), "hold_wait_max_s"),
        (json!({"hold_ok": false, "hold_wait_max_s": 9}), "hold_ok"),
    ] {
        let params = select_with(members.clone());
        for refusal in [
            parse_select_request(&params).expect_err("no group"),
            parse_select_request_serving(&params, &describe.capabilities[..])
                .expect_err("no group"),
            parse_select_request_serving(&params, &["some_other_group"]).expect_err("other"),
        ] {
            assert_eq!(refusal.code, RefusalCode::InvalidRequest, "{members}");
            assert_eq!(refusal.detail["field"], field, "{members}");
        }
    }
    let declared = describe.with_capability(SELECT_HOLD_CONTROL);
    assert!(parse_select_request_serving(
        &select_with(json!({"hold_ok": false})),
        &declared.capabilities[..]
    )
    .is_ok());
    assert!(parse_select_request(&select_with(json!({"hold_ok": null}))).is_ok());
}

#[test]
fn prefer_members_are_refused_by_name_without_the_group() {
    let describe = RoleDescribe::new(vec![], false);
    assert!(!describe.serves(SELECT_PREFER));
    let params = select_with(json!({"prefer": ["example-provider/model-a"]}));
    for refusal in [
        parse_select_request(&params).expect_err("no group"),
        parse_select_request_serving(&params, &describe.capabilities[..]).expect_err("no group"),
        parse_select_request_serving(&params, &["some_other_group"]).expect_err("other"),
    ] {
        assert_eq!(refusal.code, RefusalCode::InvalidRequest);
        assert_eq!(refusal.detail["field"], "prefer");
    }
    let declared = describe.with_capability(SELECT_PREFER);
    assert!(parse_select_request_serving(
        &select_with(json!({"prefer": ["example-provider/model-a"]})),
        &declared.capabilities[..]
    )
    .is_ok());
    assert!(parse_select_request(&select_with(json!({"prefer": null}))).is_ok());
}

#[test]
fn prefer_entries_are_validated_by_name() {
    let group = [SELECT_PREFER];
    let valid = parse_select_request_serving(
        &select_with(json!({"prefer": ["example-provider/model-a"]})),
        &group,
    )
    .unwrap();
    assert_eq!(valid.prefer.unwrap(), ["example-provider/model-a"]);
    for (members, field) in [
        (json!({"prefer": ["model-a"]}), "prefer"),
        (
            json!({"prefer": ["example-provider/model-a:fast"]}),
            "prefer",
        ),
        (
            json!({"prefer": ["example-provider/model-a", "example-provider/model-a"]}),
            "prefer",
        ),
        (json!({"prefer": "example-provider/model-a"}), "prefer"),
    ] {
        let refusal = parse_select_request_serving(&select_with(members.clone()), &group)
            .expect_err(&members.to_string());
        assert_eq!(refusal.code, RefusalCode::InvalidRequest);
        assert_eq!(refusal.detail["field"], field);
    }
    let many: Vec<serde_json::Value> = (0..17).map(|n| json!(format!("p/m{n}"))).collect();
    let refusal = parse_select_request_serving(&select_with(json!({"prefer": many})), &group)
        .expect_err("too many");
    assert_eq!(refusal.detail["field"], "prefer");
}

#[test]
fn pin_fallback_other_values_and_missing_pin_are_refused() {
    let with_pin = |fallback: serde_json::Value| {
        let mut params = select_with(json!({"pin": "example-provider/model-a"}));
        params["pin_fallback"] = fallback;
        params
    };
    let request = parse_select_request(&with_pin(json!("route"))).unwrap();
    assert_eq!(request.pin_fallback.unwrap(), PinFallback::Route);
    for params in [
        with_pin(json!("retry")),
        with_pin(json!(7)),
        select_with(json!({"pin_fallback": "route"})),
    ] {
        let refusal = parse_select_request(&params).expect_err(&params.to_string());
        assert_eq!(refusal.code, RefusalCode::InvalidRequest);
        assert_eq!(refusal.detail["field"], "pin_fallback");
    }
    assert!(parse_select_request(&select_with(json!({"pin_fallback": null}))).is_ok());
    assert_eq!(
        parse_select_request(&with_pin(json!("route"))).unwrap(),
        parse_select_request_serving(&with_pin(json!("route")), &[SELECT_PREFER]).unwrap()
    );
}

#[test]
fn usage_counts_are_bounded_and_strict() {
    let base = || json!({"decision_id": "d", "outcome": "completed"});
    let with_usage = |usage: serde_json::Value| {
        let mut params = base();
        params["usage"] = usage;
        params
    };
    let full = json!({"input_tokens": 1, "cached_input_tokens": 2, "cache_write_tokens": 3, "output_tokens": 4, "reasoning_tokens": 1, "duration_ms": 10, "first_token_ms": 2, "cost_usd": 0.5});
    let request = parse_report_request(&with_usage(full.clone())).unwrap();
    assert_eq!(serde_json::to_value(request.usage.unwrap()).unwrap(), full);
    assert!(parse_report_request(&with_usage(json!({}))).is_ok());
    for usage in [
        json!({"input_tokens": -1}),
        json!({"output_tokens": 1.5}),
        json!({"input_tokens": MAX_USAGE_COUNT + 1}),
        json!({"cost_usd": -0.5}),
        json!({"bogus": 1}),
    ] {
        let refusal =
            parse_report_request(&with_usage(usage.clone())).expect_err(&usage.to_string());
        assert_eq!(refusal.code, RefusalCode::InvalidRequest);
        assert_eq!(refusal.detail["field"], "usage");
    }
    assert_eq!(PinOutcome::Pinned.as_str(), "pinned");
    assert_eq!(PinOutcome::FellBack.as_str(), "fell_back");
}

#[test]
fn describe_capabilities_are_left_out_when_empty() {
    let describe = RoleDescribe::new(vec![], false);
    assert!(serde_json::to_value(&describe)
        .unwrap()
        .get("capabilities")
        .is_none());
    let declared = describe
        .with_capability(SELECT_HOLD_CONTROL)
        .with_capability(SELECT_HOLD_CONTROL);
    assert_eq!(declared.capabilities, [SELECT_HOLD_CONTROL]);
    assert_eq!(
        serde_json::to_value(&declared).unwrap()["capabilities"],
        json!(["select_hold_control"])
    );
    let mut future = serde_json::to_value(&declared).unwrap();
    future["capabilities"] = json!(["select_hold_control", "future_group"]);
    let decoded: RoleDescribe = serde_json::from_value(future).unwrap();
    assert!(decoded.serves("future_group"));
}

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
        (
            "usage",
            json!({"input_tokens": 100}),
            json!({"input_tokens": 101}),
        ),
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
