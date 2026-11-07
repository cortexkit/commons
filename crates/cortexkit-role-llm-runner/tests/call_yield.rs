use cortexkit_role_llm_runner::{
    call_yield::*, capabilities, delivery_order::PlanSender, send::SendRequest,
    session_change::RefreshRequest, vectors,
};
use serde_json::{json, Value};

#[test]
fn call_yield_dependencies_are_refused_by_name() {
    use cortexkit_role_llm_runner::{
        describe::{check_describe, DescribeProblem},
        PROVIDES, REQUIRED_OPS,
    };
    let raw = json!({"majors":[{"version":PROVIDES,"ops":REQUIRED_OPS,"stability":"stable"}],"implementation_version":"test","capabilities":["call_yield"],"session_capabilities_from":"baseline"});
    let errors = check_describe(&raw).unwrap_err();
    for name in [
        "call_yield_without_plans",
        "call_yield_without_delivery_order",
        "call_yield_without_notices",
        "call_yield_without_message_marks",
    ] {
        assert!(
            errors.contains(&DescribeProblem::Undecodable(name.into())),
            "{name}: {errors:?}"
        );
    }
}

#[test]
fn call_yield_person_steer_vectors() {
    let vectors: Value = serde_json::from_str(vectors::CALL_YIELD_JSON).unwrap();
    let senders: Vec<PlanSender> = serde_json::from_value(vectors["senders"].clone()).unwrap();
    for case in vectors["triggers"].as_array().unwrap() {
        assert_eq!(
            is_person_steer(
                &senders,
                case["sender"].as_str().unwrap(),
                &case["delivery"],
                case["notice"].as_bool().unwrap_or(false),
                case.get("speaks_for").is_some()
            ),
            case["expected"].as_bool().unwrap(),
            "{case}"
        );
    }
    let record: YieldedCall = serde_json::from_value(vectors["yielded"].clone()).unwrap();
    assert_eq!(record, YieldedCall::new("runner", "call-1"));
    assert_eq!(serde_json::to_value(record).unwrap(), vectors["yielded"]);
}

#[test]
fn call_yield_person_is_strictly_group_gated() {
    for person in [json!(true), json!(false), json!(null), json!("true")] {
        let plan = json!({"senders":[{"module":"app-gateway","person":person}]})
            .as_object()
            .unwrap()
            .clone();
        assert_eq!(
            check_plan(&plan, &[] as &[&str]),
            Err("plan.senders[0].person".into())
        );
        assert_eq!(
            check_plan(&plan, &[capabilities::CALL_YIELD]).is_ok(),
            person.is_boolean()
        );
        let send: SendRequest =
            serde_json::from_value(json!({"send_id":"send-1","prompt":"hello","plan":plan}))
                .unwrap();
        assert_eq!(
            send.check_call_yield(&[] as &[&str]),
            Err("plan.senders[0].person".into())
        );
        let refresh = RefreshRequest::new(plan.clone(), "next_run", 1);
        assert!(refresh.check_call_yield(&[] as &[&str]).is_err());
    }
    assert_eq!(
        serde_json::to_value(PlanSender::new("peer")).unwrap(),
        json!({"module":"peer"})
    );
    assert!(serde_json::from_value::<PlanSender>(json!({"module":"peer","unknown":true})).is_err());
    assert_eq!(
        serde_json::to_value(PlanSender::new("app-gateway").with_person()).unwrap(),
        json!({"module":"app-gateway","person":true})
    );
}
