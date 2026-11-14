use cortexkit_role_routing::{
    account::{parse_account_for_request_serving, AccountForRequest, AccountMode, ExpectedPin},
    account_continue::{parse_continue_request, ContinueRequest, ExhaustedAccount},
    capabilities::{self, ACCOUNT_HEAD_TURN},
    describe::RoleDescribe,
};
use serde_json::{json, Value};

fn describe() -> RoleDescribe {
    RoleDescribe::new(vec![], false)
        .with_capability(ACCOUNT_HEAD_TURN)
        .with_capability(capabilities::ACCOUNT_CONTINUE)
}

fn continuation() -> ContinueRequest {
    ContinueRequest::new(
        "run",
        "admission",
        "conversation",
        "decision",
        ExhaustedAccount::new("credential", "account", "five_hour", 100),
        "selection",
    )
}

#[test]
fn account_head_turn_round_trips_in_every_mode_and_false_is_absent() {
    let describe = describe();
    assert!(describe.serves_account_head_turn());
    assert!(capabilities::KNOWN.contains(&ACCOUNT_HEAD_TURN));
    assert!(!RoleDescribe::new(vec![], false).serves_account_head_turn());
    for mode in [
        AccountMode::Admit,
        AccountMode::Renew,
        AccountMode::Reassign,
    ] {
        let bare = AccountForRequest::new("example", "model", "conversation", "admission")
            .with_mode(mode)
            .with_expected_pin(ExpectedPin {
                id: "pin".into(),
                generation: 1,
            });
        let head = bare.clone().with_head_turn(true);
        let wire = serde_json::to_value(&head).unwrap();
        assert_eq!(wire["head_turn"], true);
        assert_eq!(
            serde_json::from_value::<AccountForRequest>(wire.clone()).unwrap(),
            head
        );
        assert_eq!(
            parse_account_for_request_serving(&wire, &describe.capabilities).unwrap(),
            head
        );
        assert!(!bare.same_body(&head));
        let mut explicit_false = serde_json::to_value(&bare).unwrap();
        explicit_false["head_turn"] = json!(false);
        assert_eq!(
            serde_json::from_value::<AccountForRequest>(explicit_false.clone()).unwrap(),
            bare
        );
        assert!(bare.same_body(
            &parse_account_for_request_serving(&explicit_false, &describe.capabilities).unwrap()
        ));
        assert_eq!(
            serde_json::to_value(bare.clone().with_head_turn(false))
                .unwrap()
                .get("head_turn"),
            None
        );
    }
    let bare = continuation();
    let head = bare.clone().with_head_turn(true);
    let wire = serde_json::to_value(&head).unwrap();
    assert_eq!(wire["head_turn"], true);
    assert_eq!(
        serde_json::from_value::<ContinueRequest>(wire.clone()).unwrap(),
        head
    );
    assert_eq!(parse_continue_request(&wire, &describe).unwrap(), head);
    assert!(!bare.same_question(&head));
    let mut explicit_false = serde_json::to_value(&bare).unwrap();
    explicit_false["head_turn"] = json!(false);
    assert_eq!(
        serde_json::from_value::<ContinueRequest>(explicit_false.clone()).unwrap(),
        bare
    );
    assert!(bare.same_question(&parse_continue_request(&explicit_false, &describe).unwrap()));
    assert_eq!(
        serde_json::to_value(bare.with_head_turn(false))
            .unwrap()
            .get("head_turn"),
        None
    );
}

#[test]
fn account_head_turn_decode_is_boolean_and_requires_the_group() {
    let describe = describe();
    let legacy = RoleDescribe::new(vec![], false).with_capability(capabilities::ACCOUNT_CONTINUE);
    for continuing in [false, true] {
        let mut wire = if continuing {
            serde_json::to_value(continuation()).unwrap()
        } else {
            serde_json::to_value(AccountForRequest::new(
                "example",
                "model",
                "conversation",
                "admission",
            ))
            .unwrap()
        };
        let parse = |wire: &Value, describe: &RoleDescribe| {
            if continuing {
                parse_continue_request(wire, describe).map(|r| r.head_turn)
            } else {
                parse_account_for_request_serving(wire, &describe.capabilities).map(|r| r.head_turn)
            }
        };
        assert!(!parse(&wire, &describe).unwrap());
        for valid in [true, false] {
            wire["head_turn"] = json!(valid);
            assert_eq!(parse(&wire, &describe).unwrap(), valid);
            let refusal = parse(&wire, &legacy).unwrap_err();
            assert_eq!(refusal.code.as_str(), "invalid_request");
            assert_eq!(refusal.detail["field"], "head_turn");
        }
        for invalid in [json!("true"), json!(1), json!({}), json!([])] {
            wire["head_turn"] = invalid;
            let refusal = parse(&wire, &describe).unwrap_err();
            assert_eq!(refusal.code.as_str(), "invalid_request");
            assert_eq!(refusal.detail["field"], "head_turn");
            if continuing {
                assert!(serde_json::from_value::<ContinueRequest>(wire.clone()).is_err());
            } else {
                assert!(serde_json::from_value::<AccountForRequest>(wire.clone()).is_err());
            }
        }
    }
}
