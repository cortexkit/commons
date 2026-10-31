use cortexkit_role_routing::{
    account::parse_account_for_request,
    account_continue::{
        parse_continue_request, ContinueAnswer, ContinueHold, ContinueOffer, ContinueRefusal,
        ContinueRequest, ExhaustedAccount,
    },
    capabilities::ACCOUNT_CONTINUE,
    describe::RoleDescribe,
};
use serde_json::json;

fn request() -> ContinueRequest {
    ContinueRequest::new(
        "run",
        "admission",
        "conversation",
        "decision",
        ExhaustedAccount::new("credential", "account", "five_hour", 100),
        "selection",
    )
}

fn describe() -> RoleDescribe {
    RoleDescribe::new(vec![], false).with_capability(ACCOUNT_CONTINUE)
}

#[test]
fn continue_requires_opt_in_and_preserves_legacy_mode_refusal() {
    let params = serde_json::to_value(request()).unwrap();
    assert_eq!(
        parse_continue_request(&params, &describe()).unwrap(),
        request()
    );
    for refusal in [
        parse_account_for_request(&json!({"provider":"example", "model":"model", "conversation_key":"c", "admission_id":"a", "mode":"continue"})).unwrap_err(),
        parse_continue_request(&params, &RoleDescribe::new(vec![], false)).unwrap_err(),
    ] {
        assert_eq!(refusal.code.as_str(), "invalid_request");
        assert_eq!(refusal.detail["field"], "mode");
    }
}

#[test]
fn continuation_validates_required_members_and_limit_evidence() {
    let params = serde_json::to_value(request()).unwrap();
    for field in [
        "mode",
        "run_id",
        "admission_id",
        "conversation_key",
        "previous_decision_id",
        "exhausted",
        "selection_id",
    ] {
        let mut missing = params.clone();
        missing.as_object_mut().unwrap().remove(field);
        assert_eq!(
            parse_continue_request(&missing, &describe())
                .unwrap_err()
                .detail["field"],
            field
        );
    }
    for (pointer, value, field) in [
        ("/selection_id", json!(""), "selection_id"),
        ("/selection_id", json!("x".repeat(129)), "selection_id"),
        ("/admission_id", json!("x".repeat(129)), "admission_id"),
        (
            "/conversation_key",
            json!("x".repeat(257)),
            "conversation_key",
        ),
        ("/mode", json!("unknown"), "mode"),
        (
            "/exhausted/credential_id",
            json!(""),
            "exhausted.credential_id",
        ),
        ("/exhausted/account", json!(null), "exhausted.account"),
        (
            "/exhausted/limit_window",
            json!("one_day"),
            "exhausted.limit_window",
        ),
        (
            "/exhausted/observed_at_ms",
            json!(-1),
            "exhausted.observed_at_ms",
        ),
    ] {
        let mut invalid = params.clone();
        *invalid.pointer_mut(pointer).unwrap() = value;
        assert_eq!(
            parse_continue_request(&invalid, &describe())
                .unwrap_err()
                .detail["field"],
            field
        );
    }
    for (field, bad) in [
        ("pool", json!("Upper")),
        ("retry_after_s", json!(-1)),
        ("resets_at_ms", json!(1.5)),
    ] {
        let mut invalid = params.clone();
        invalid["exhausted"][field] = bad;
        assert_eq!(
            parse_continue_request(&invalid, &describe())
                .unwrap_err()
                .detail["field"],
            format!("exhausted.{field}")
        );
    }
    let mut seven_day = params;
    seven_day["exhausted"]["limit_window"] = json!("seven_day");
    seven_day["selection_id"] = json!("é".repeat(128));
    assert!(parse_continue_request(&seven_day, &describe()).is_ok());
}

#[test]
fn hold_without_until_is_unknown_and_outcomes_have_one_kind() {
    let hold = ContinueAnswer::Hold(ContinueHold::new("account_pool_exhausted", json!({})));
    assert_eq!(hold.wait_until_ms(), None);
    assert_eq!(
        serde_json::to_value(&hold).unwrap(),
        json!({"kind":"hold", "routing_code":"account_pool_exhausted", "detail":{}})
    );
    let timed = ContinueAnswer::Hold(
        ContinueHold::new("account_pool_exhausted", json!({})).with_until_ms(500),
    );
    assert_eq!(timed.wait_until_ms(), Some(500));
    for answer in [
        hold,
        timed,
        ContinueAnswer::Offer(ContinueOffer::new("credential", "account", "decision", 500)),
        ContinueAnswer::Refuse(ContinueRefusal::new(
            "account_selection_disabled",
            json!({}),
        )),
    ] {
        let value = serde_json::to_value(&answer).unwrap();
        assert_eq!(
            serde_json::from_value::<ContinueAnswer>(value).unwrap(),
            answer
        );
    }
    assert!(serde_json::from_value::<ContinueAnswer>(json!({"kind":"unknown"})).is_err());
}

#[test]
fn same_question_compares_every_parsed_binding_and_exhaustion_field() {
    let original = request();
    let params = serde_json::to_value(&original).unwrap();
    for field in [
        "run_id",
        "admission_id",
        "conversation_key",
        "previous_decision_id",
    ] {
        let mut changed = params.clone();
        changed[field] = json!("different");
        let changed = parse_continue_request(&changed, &describe()).unwrap();
        assert!(!original.same_question(&changed), "{field}");
    }
    for (field, value) in [
        ("credential_id", json!("different-credential")),
        ("account", json!("different-account")),
        ("limit_window", json!("seven_day")),
        ("pool", json!("primary")),
        ("resets_at_ms", json!(500)),
        ("retry_after_s", json!(60)),
        ("observed_at_ms", json!(101)),
    ] {
        let mut changed = params.clone();
        changed["exhausted"][field] = value;
        let changed = parse_continue_request(&changed, &describe()).unwrap();
        assert!(!original.same_question(&changed), "exhausted.{field}");
    }
    let mut equivalent = params;
    equivalent["exhausted"]["pool"] = json!(null);
    equivalent["exhausted"]["resets_at_ms"] = json!(null);
    equivalent["exhausted"]["retry_after_s"] = json!(null);
    let equivalent = parse_continue_request(&equivalent, &describe()).unwrap();
    assert!(original.same_question(&equivalent));
}
