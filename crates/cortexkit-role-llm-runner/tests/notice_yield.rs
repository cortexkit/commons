use cortexkit_role_llm_runner::{
    call_yield, capabilities,
    delivery_order::PlanSender,
    describe::{check_describe, DescribeProblem},
    send::{Delivery, SendRequest},
    session_change::{RefreshRequest, RelaunchRefreshRequest},
    vectors, PROVIDES, REQUIRED_OPS,
};
use serde_json::{json, Value};

#[test]
fn notice_yield_requires_call_yield_by_name() {
    let raw = json!({"majors":[{"version":PROVIDES,"ops":REQUIRED_OPS,"stability":"stable"}],"implementation_version":"test","capabilities":["notice_yield"],"session_capabilities_from":"baseline"});
    assert!(check_describe(&raw)
        .unwrap_err()
        .contains(&DescribeProblem::Undecodable(
            "notice_yield_without_call_yield".into()
        )));
}

#[test]
fn notice_yield_plan_request_vectors_and_refresh_helpers() {
    let vectors: Value = serde_json::from_str(vectors::NOTICE_YIELD_JSON).unwrap();
    for case in vectors["plans"].as_array().unwrap() {
        let groups: Vec<String> = serde_json::from_value(case["groups"].clone()).unwrap();
        let plan = case["plan"].as_object().unwrap();
        let owner = case["owner"].as_str();
        let expected = case["field"].as_str().map_or(Ok(()), |s| Err(s.into()));
        assert_eq!(
            call_yield::check_plan_for_owner(plan, &groups, owner),
            expected,
            "{case}"
        );
        let send = SendRequest::new("hello", "s").with_plan(plan.clone());
        assert_eq!(
            send.check_notice_yield(&groups, owner, "core", &[])
                .map(|_| ()),
            expected,
            "{case}"
        );
        let refresh = RefreshRequest::new(plan.clone(), "next_run", 1);
        assert_eq!(
            refresh.check_notice_yield(&groups, owner),
            expected,
            "{case}"
        );
        let relaunch = RelaunchRefreshRequest::from(refresh);
        assert_eq!(
            relaunch.check_notice_yield(&groups, owner),
            expected,
            "{case}"
        );
    }
    for invalid in [json!("true"), json!(0), json!({})] {
        let plan = json!({"senders":[{"module":"core","yields":invalid}]})
            .as_object()
            .unwrap()
            .clone();
        assert_eq!(
            call_yield::check_plan(&plan, &[capabilities::NOTICE_YIELD]),
            Err("plan.senders[0].yields".into())
        );
    }
    assert_eq!(
        serde_json::to_value(PlanSender::new("core").with_yields()).unwrap(),
        json!({"module":"core","yields":true})
    );
    assert_eq!(
        serde_json::to_value(PlanSender::new("core")).unwrap(),
        json!({"module":"core"})
    );
    assert!(serde_json::from_value::<PlanSender>(
        json!({"module":"core","yields":true,"unknown":true})
    )
    .is_err());
}

#[test]
fn notice_yield_send_request_vectors() {
    let vectors: Value = serde_json::from_str(vectors::NOTICE_YIELD_JSON).unwrap();
    let senders: Vec<PlanSender> = serde_json::from_value(vectors["senders"].clone()).unwrap();
    for case in vectors["sends"].as_array().unwrap() {
        let groups: Vec<String> = serde_json::from_value(case["groups"].clone()).unwrap();
        let send: SendRequest = serde_json::from_value(case["request"].clone()).unwrap();
        let expected = case["field"].as_str().map_or_else(
            || Ok(case["trigger"].as_bool().unwrap()),
            |field| Err(field.into()),
        );
        assert_eq!(
            send.check_notice_yield(
                &groups,
                Some("core"),
                case["sender"].as_str().unwrap(),
                &senders
            ),
            expected,
            "{case}"
        );
        if case["request"]["yield"] == false {
            assert!(serde_json::to_value(send).unwrap().get("yield").is_none());
        } else {
            assert_eq!(serde_json::to_value(send).unwrap(), case["request"]);
        }
    }
    let send = SendRequest::new("urgent", "s")
        .with_delivery(Delivery::Steer)
        .with_yield(true);
    assert_eq!(
        send.check_notice_yield(
            &[capabilities::NOTICE_YIELD],
            Some("core"),
            "core",
            &senders
        ),
        Ok(true)
    );
    assert!(serde_json::to_value(send.with_yield(false))
        .unwrap()
        .get("yield")
        .is_none());
}
