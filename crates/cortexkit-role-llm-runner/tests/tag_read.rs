use cortexkit_role_llm_runner::{
    capabilities,
    describe::{check_describe, DescribeProblem},
    errors,
    read::{TagReadAnswer, TagReadRequest},
    scope_admission::ScopeKind,
};
use serde_json::{json, Value};

fn vector() -> Value {
    serde_json::from_str(include_str!("../test-vectors/llm-runner-v1/tag-read.json")).unwrap()
}

#[test]
fn tag_read_vectors_pin_requests_answers_and_named_refusals() {
    let vector = vector();
    let request = TagReadRequest::decode(&vector["request"]).unwrap();
    assert_eq!(request, TagReadRequest::new(7, "call-before-undo"));
    assert_eq!(serde_json::to_value(request).unwrap(), vector["request"]);
    let answer: TagReadAnswer = serde_json::from_value(vector["answer"].clone()).unwrap();
    assert_eq!(answer.part, 1);
    assert_eq!(answer.entry.mid, "original-message");
    assert_eq!(
        answer.entry.client_message_id.as_deref(),
        Some("app-message-7")
    );
    assert_eq!(serde_json::to_value(answer).unwrap(), vector["answer"]);
    for invalid in vector["invalid"].as_array().unwrap() {
        assert_eq!(
            TagReadRequest::decode(&invalid["request"]).unwrap_err(),
            invalid["field"]
        );
        assert!(serde_json::from_value::<TagReadRequest>(invalid["request"].clone()).is_err());
    }
    for refusal in vector["refusals"].as_array().unwrap() {
        assert!(errors::CODES.contains(&refusal["code"].as_str().unwrap()));
    }
}

#[test]
fn tag_read_owner_is_attested_and_flow_or_run_never_counts() {
    let request = TagReadRequest::new(7, "call-before-undo");
    assert_eq!(request.check_declared(false), Err("tag"));
    assert_eq!(request.check_declared(true), Ok(()));
    assert_eq!(
        request.check_owner(Some("owner"), "owner", ScopeKind::Plain),
        Ok(())
    );
    for (owner, sender, kind) in [
        (Some("owner"), "reader", ScopeKind::Plain),
        (None, "owner", ScopeKind::Plain),
        (Some("owner"), "owner", ScopeKind::Flow { flow_id: "flow" }),
        (Some("owner"), "owner", ScopeKind::Run { run_id: "run" }),
    ] {
        assert_eq!(
            request.check_owner(owner, sender, kind),
            Err(errors::SCOPE_OWNER_MISMATCH)
        );
    }
}

#[test]
fn tag_read_rejects_all_nonselector_members_even_false_flags() {
    for field in [
        "media",
        "view",
        "lineage_id",
        "include_originals",
        "include_rich_ui",
        "sender",
    ] {
        let mut request = json!({"tag":7,"call_key":"call-before-undo"});
        request[field] = json!(false);
        assert_eq!(TagReadRequest::decode(&request).unwrap_err(), field);
    }
}

#[test]
fn tag_read_describe_requires_all_fact_groups_by_name() {
    let mut value = json!({
        "majors":[{"version":"llm-runner/v1", "stability":"alpha", "ops":cortexkit_role_llm_runner::REQUIRED_OPS}],
        "implementation_version":"test", "session_capabilities_from":"baseline",
        "capabilities":[capabilities::TAG_READ]
    });
    let problems = check_describe(&value).unwrap_err();
    for name in [
        "tag_read_without_message_senders",
        "tag_read_without_message_marks",
        "tag_read_without_client_message_id",
        "tag_read_without_read_display_facts",
    ] {
        assert!(
            problems.contains(&DescribeProblem::Undecodable(name.into())),
            "{problems:?}"
        );
    }
    value["capabilities"] = json!([
        capabilities::TAG_READ,
        capabilities::CLIENT_MESSAGE_ID,
        capabilities::MESSAGE_SENDERS,
        capabilities::MESSAGE_MARKS,
        capabilities::HISTORY_DISPLAY,
        capabilities::LIVE_EVENTS,
        capabilities::READ_DISPLAY_FACTS
    ]);
    value["majors"][0]["ops"] = json!(cortexkit_role_llm_runner::capabilities::GROUPS
        .iter()
        .filter(|(group, _)| value["capabilities"]
            .as_array()
            .unwrap()
            .iter()
            .any(|g| g == group))
        .flat_map(|(_, ops)| ops.iter().copied())
        .chain(cortexkit_role_llm_runner::REQUIRED_OPS.iter().copied())
        .collect::<Vec<_>>());
    assert!(
        check_describe(&value).is_ok(),
        "{:?}",
        check_describe(&value)
    );
    for (group, name) in [
        (
            capabilities::MESSAGE_SENDERS,
            "tag_read_without_message_senders",
        ),
        (
            capabilities::MESSAGE_MARKS,
            "tag_read_without_message_marks",
        ),
        (
            capabilities::CLIENT_MESSAGE_ID,
            "tag_read_without_client_message_id",
        ),
        (
            capabilities::READ_DISPLAY_FACTS,
            "tag_read_without_read_display_facts",
        ),
    ] {
        let mut missing = value.clone();
        missing["capabilities"]
            .as_array_mut()
            .unwrap()
            .retain(|item| item != group);
        let problems = check_describe(&missing).unwrap_err();
        let tag_problems: Vec<_> = problems.iter().filter(|problem| {
            matches!(problem, DescribeProblem::Undecodable(name) if name.starts_with("tag_read_without_"))
        }).cloned().collect();
        assert_eq!(
            tag_problems,
            vec![DescribeProblem::Undecodable(name.into())]
        );
    }
}
