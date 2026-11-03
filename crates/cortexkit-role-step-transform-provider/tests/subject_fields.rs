use cortexkit_role_step_transform_provider::hook::{
    HookCall, HookProblem, Subject, DEFAULT_HOOK_CAP_BYTES,
};
use serde_json::{json, Value};

fn hook() -> HookCall {
    HookCall::new(Subject::PreUser {
        blocks: vec![],
        mark: None,
        delivery: None,
        at_ms: None,
    })
}

#[test]
fn subject_fields_round_trip() {
    let vectors: Value = serde_json::from_str(include_str!(
        "../test-vectors/step-transform-provider-v2/subject-fields.json"
    ))
    .unwrap();
    for case in vectors["requests"].as_array().unwrap() {
        let call: HookCall = serde_json::from_value(case["request"].clone()).unwrap();
        assert_eq!(call.check_subject_fields(), Ok(()), "{}", case["name"]);
        assert_eq!(serde_json::to_value(&call).unwrap(), case["request"]);
    }
    for case in vectors["undecodable"].as_array().unwrap() {
        assert!(
            serde_json::from_value::<HookCall>(case["request"].clone()).is_err(),
            "{}",
            case["name"]
        );
    }
    for message in [
        json!({"own_schema": true}),
        json!([1, "x"]),
        json!(7),
        Value::Null,
    ] {
        let call = hook()
            .with_subject_identity("m0", 1)
            .with_message(message.clone());
        let again: HookCall = serde_json::from_slice(&serde_json::to_vec(&call).unwrap()).unwrap();
        assert_eq!(again.message, Some(message));
        assert_eq!(again, call);
        assert_eq!(again.check_subject_fields(), Ok(()));
    }
}

#[test]
fn subject_pairing_and_one_based_ordinals_are_validated() {
    let vectors: Value = serde_json::from_str(include_str!(
        "../test-vectors/step-transform-provider-v2/subject-fields.json"
    ))
    .unwrap();
    for case in vectors["invalid_pairs"].as_array().unwrap() {
        let call: HookCall = serde_json::from_value(case["request"].clone()).unwrap();
        assert_eq!(
            call.check_subject_fields().unwrap_err().field(),
            case["field"].as_str().unwrap()
        );
    }
    assert_eq!(hook().check_subject_fields(), Ok(()));
    assert_eq!(
        hook().with_subject_identity("m0", 1).check_subject_fields(),
        Ok(())
    );
    assert_eq!(
        hook().with_subject_identity("m0", 0).check_subject_fields(),
        Err(HookProblem::SubjectOrdinalZero)
    );
}

#[test]
fn message_counts_toward_request_cap() {
    assert_eq!(DEFAULT_HOOK_CAP_BYTES, 3_145_728);
    let call = hook()
        .with_subject_identity("m0", 1)
        .with_message(json!(""));
    let overhead = serde_json::to_vec(&call).unwrap().len();
    let at_cap = call
        .clone()
        .with_message(json!("x".repeat(DEFAULT_HOOK_CAP_BYTES - overhead)));
    assert_eq!(
        serde_json::to_vec(&at_cap).unwrap().len(),
        DEFAULT_HOOK_CAP_BYTES
    );
    assert_eq!(at_cap.check_message_size(DEFAULT_HOOK_CAP_BYTES), Ok(()));
    let over_cap = call.with_message(json!("x".repeat(DEFAULT_HOOK_CAP_BYTES - overhead + 1)));
    assert_eq!(
        over_cap.check_message_size(DEFAULT_HOOK_CAP_BYTES),
        Err(HookProblem::RequestTooLarge)
    );
    assert_eq!(hook().check_message_size(0), Ok(()));
}

#[test]
fn message_interpretation_requires_a_supported_profile() {
    let mut call = hook()
        .with_subject_identity("m0", 1)
        .with_message(Value::Null);
    assert_eq!(
        call.check_message_profile(&["example"]),
        Err(HookProblem::SerializerProfile)
    );
    call.params
        .insert("serializer_profile".into(), json!("unknown"));
    assert_eq!(
        call.check_message_profile(&["example"]),
        Err(HookProblem::SerializerProfile)
    );
    call.params
        .insert("serializer_profile".into(), json!("example"));
    assert_eq!(call.check_message_profile(&["example"]), Ok(()));
}

#[test]
fn subject_ids_preserve_u64_precision_and_part_byte_bounds() {
    let call = hook()
        .with_subject_identity("m0", u64::MAX)
        .with_subject_part("p".repeat(256));
    let mut wire = serde_json::to_value(&call).unwrap();
    wire["unknown"] = json!(true);
    let decoded: HookCall = serde_json::from_value(wire).unwrap();
    assert_eq!(decoded, call);
    assert_eq!(decoded.validate(), Ok(()));
    assert!(hook().with_subject_part("").validate().is_err());
    assert!(hook()
        .with_subject_part("é".repeat(129))
        .validate()
        .is_err());
}

#[test]
fn runner_trace_and_prompt_provenance_survive_every_hook() {
    let vectors: Value = serde_json::from_str(include_str!(
        "../test-vectors/step-transform-provider-v2/hook-requests.json"
    ))
    .unwrap();
    for case in vectors["requests"].as_array().unwrap() {
        for trace in [Value::Null, json!({"span": [1, "opaque"]})] {
            let mut request = case["request"].clone();
            request["runner_trace"] = trace.clone();
            let call: HookCall = serde_json::from_value(request.clone()).unwrap();
            assert_eq!(call.runner_trace, Some(trace));
            assert_eq!(serde_json::to_value(call).unwrap(), request);
        }
    }
    let request = json!({"hook":"pre_user", "params":{}, "blocks":["late result"],
        "mark":{"kind":"tool_result", "from":"a-provider"}, "delivery":"queue"});
    let call: HookCall = serde_json::from_value(request.clone()).unwrap();
    assert_eq!(serde_json::to_value(call).unwrap(), request);
}
