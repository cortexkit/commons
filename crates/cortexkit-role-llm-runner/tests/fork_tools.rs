use cortexkit_role_llm_runner::{
    capabilities,
    fork_answer::{self, ForkAskReply, ForkAskRequest, ForkFinish, FramingPlacement},
};
use serde_json::{json, Value};

fn samples() -> Value {
    serde_json::from_str(include_str!(
        "../test-vectors/llm-runner-v1/fork-tools.json"
    ))
    .unwrap()
}

#[test]
fn fork_tools_shape_gate_identity_and_tool_permissions() {
    let samples = samples();
    let ask: ForkAskRequest = serde_json::from_value(samples["ask"].clone()).unwrap();
    assert_eq!(serde_json::to_value(&ask).unwrap(), samples["ask"]);
    ask.check().unwrap();
    ask.check_tools(&[capabilities::FORK_TOOLS]).unwrap();
    assert_eq!(ask.check_tools::<&str>(&[]), Err("allowed_tools"));
    let mut frame_only = ask.clone();
    frame_only.allowed_tools = None;
    assert_eq!(frame_only.check_tools::<&str>(&[]), Err("framing"));
    assert!(ask.same_question(&ask.clone()));
    for change in samples["identity_changes"].as_array().unwrap() {
        let mut changed = samples["ask"].clone();
        if let Some(remove) = change["remove"].as_str() {
            changed.as_object_mut().unwrap().remove(remove);
        } else {
            changed
                .as_object_mut()
                .unwrap()
                .extend(change.as_object().unwrap().clone());
        }
        let changed: ForkAskRequest = serde_json::from_value(changed).unwrap();
        assert!(!ask.same_question(&changed));
    }
    for call in samples["calls"].as_array().unwrap() {
        let allowed = ask.permits_tool_on_route(
            call["name"].as_str().unwrap(),
            call["read_only"].as_bool().unwrap(),
            call.get("routing_argument").is_some(),
        );
        assert_eq!(allowed, call.get("dispatch").is_some());
        if !allowed {
            assert_eq!(call["result"], fork_answer::TOOL_UNAVAILABLE);
        }
    }
    let mut no_tools = ask.clone();
    no_tools.allowed_tools = None;
    assert!(!no_tools.permits_tool("read", true));
    for invalid in [json!(null), json!(1), json!([1])] {
        let mut value = samples["ask"].clone();
        value["allowed_tools"] = invalid;
        assert!(serde_json::from_value::<ForkAskRequest>(value).is_err());
    }
    for invalid in [
        vec!["".to_owned()],
        vec!["read".into(), "read".into()],
        (0..33).map(|i| i.to_string()).collect(),
    ] {
        let mut value = ask.clone();
        value.allowed_tools = Some(invalid);
        assert_eq!(value.check(), Err("allowed_tools"));
    }
    let mut empty = ask.clone();
    empty.allowed_tools = Some(vec![]);
    empty.check().unwrap();
    for text in ["".to_owned(), "é".repeat(2049)] {
        let mut value = ask.clone();
        value.framing = Some(text);
        assert_eq!(value.check(), Err("framing"));
    }
    let mut limit = ask.clone();
    limit.framing = Some("é".repeat(2048));
    limit.check().unwrap();
    for invalid in [json!(null), json!(1)] {
        let mut value = samples["ask"].clone();
        value["framing"] = invalid;
        assert!(serde_json::from_value::<ForkAskRequest>(value).is_err());
    }
}

#[test]
fn fork_tools_placements_and_step_limit_samples_round_trip() {
    let samples = samples();
    let ask: ForkAskRequest = serde_json::from_value(samples["ask"].clone()).unwrap();
    for sample in samples["placements"].as_array().unwrap() {
        let placement: FramingPlacement =
            serde_json::from_value(sample["placement"].clone()).unwrap();
        assert_eq!(json!(ask.question_messages(placement)), sample["suffix"]);
    }
    assert!(serde_json::from_value::<FramingPlacement>(json!("other")).is_err());
    assert_eq!(fork_answer::MAX_MODEL_REQUESTS, 8);
    assert_eq!(
        samples["cap"]["last_request_append"],
        fork_answer::ANSWER_NOW
    );
    let reply: ForkAskReply = serde_json::from_value(samples["cap"]["reply"].clone()).unwrap();
    assert_eq!(reply.finish, ForkFinish::StepLimit);
    assert_eq!(
        serde_json::to_value(&reply).unwrap(),
        samples["cap"]["reply"]
    );
    assert_eq!(
        ForkFinish::parse("future"),
        ForkFinish::Unknown("future".into())
    );
}

#[test]
fn fork_tools_describe_requires_fork_answer_and_closed_placement_map() {
    use cortexkit_role_llm_runner::{
        describe::{self, Major, RoleDescribe},
        ops, PROVIDES, REQUIRED_OPS,
    };
    let operations = REQUIRED_OPS
        .iter()
        .copied()
        .chain([
            ops::SESSION_FORK_PREPARE,
            ops::SESSION_FORK_ASK,
            ops::SESSION_FORK_END,
        ])
        .map(str::to_owned)
        .collect();
    let desc = RoleDescribe::new(vec![Major::new(PROVIDES, operations, "alpha")], "test")
        .with_capabilities(vec![
            capabilities::FORK_ANSWER.into(),
            capabilities::FORK_TOOLS.into(),
        ])
        .with_session_capabilities_from("baseline")
        .with_fork_idle_s(30)
        .with_fork_tools([("provider".into(), FramingPlacement::System)].into());
    let valid = serde_json::to_value(desc).unwrap();
    describe::check_describe(&valid).unwrap();
    let mut undeclared_null = valid.clone();
    undeclared_null["capabilities"] = json!([capabilities::FORK_ANSWER]);
    undeclared_null["fork_tools"] = Value::Null;
    assert!(describe::check_describe(&undeclared_null).is_err());
    for change in [
        json!({"capabilities":["fork_tools"], "fork_idle_s":null}),
        json!({"fork_tools":null}),
        json!({"fork_tools":{"provider":"other"}}),
    ] {
        let mut value = valid.clone();
        value
            .as_object_mut()
            .unwrap()
            .extend(change.as_object().unwrap().clone());
        assert!(describe::check_describe(&value).is_err());
    }
}
