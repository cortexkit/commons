use cortexkit_role_step_transform_provider::{
    errors::{ErrorBody, INVALID_PARAMS, TRANSIENT},
    hook::{DescendsFrom, HookCall, HookProblem, Subject, DEFAULT_HOOK_CAP_BYTES},
};
use serde_json::{json, Value};

fn host_vectors() -> Value {
    serde_json::from_str(include_str!(
        "../../../test-vectors/step-transform-provider-v1/host-runner.json"
    ))
    .unwrap()
}

fn hook() -> HookCall {
    HookCall::new(
        "s",
        "host",
        Subject::PreUser {
            blocks: vec![],
            mark: None,
            delivery: None,
        },
    )
}

#[test]
fn host_hook_fields_round_trip() {
    let vectors = host_vectors();
    for case in vectors["requests"].as_array().unwrap() {
        let call: HookCall = serde_json::from_value(case["request"].clone()).unwrap();
        assert_eq!(call.check_host_fields(), Ok(()), "{}", case["name"]);
        assert_eq!(serde_json::to_value(&call).unwrap(), case["request"]);
        let again: HookCall = serde_json::from_slice(&serde_json::to_vec(&call).unwrap()).unwrap();
        assert_eq!(again, call);
    }
    let call = hook()
        .with_lineage("l")
        .with_subject_identity("m", 0)
        .with_descends_from(DescendsFrom::new("old", 0));
    for message in [
        json!({"own_schema": true}),
        json!([1, "x"]),
        json!(7),
        Value::Null,
    ] {
        let call = call.clone().with_message(message.clone());
        let again: HookCall = serde_json::from_slice(&serde_json::to_vec(&call).unwrap()).unwrap();
        assert_eq!(again.message, Some(message));
        assert_eq!(again, call);
        assert_eq!(again.check_host_fields(), Ok(()));
    }
    assert_eq!(call.subject_mid.as_deref(), Some("m"));
    assert_eq!(call.subject_ordinal, Some(0));
    assert_eq!(call.descends_from.unwrap(), DescendsFrom::new("old", 0));
}

#[test]
fn hook_subject_pairing_is_validated() {
    for case in host_vectors()["invalid_pairs"].as_array().unwrap() {
        // These shapes decode, but must be refused before any ingestion.
        let call: HookCall = serde_json::from_value(case["request"].clone()).unwrap();
        let problem = call
            .check_host_fields()
            .expect_err(case["name"].as_str().unwrap());
        assert_eq!(problem.field(), case["field"].as_str().unwrap());
        let refusal = ErrorBody::new(INVALID_PARAMS, "invalid subject")
            .with_detail(json!({"field": problem.field()}));
        assert_eq!(refusal.code, "invalid_params");
    }
    assert_eq!(hook().check_host_fields(), Ok(()));
    assert_eq!(
        hook().with_subject_identity("m", 0).check_host_fields(),
        Ok(())
    );
}

#[test]
fn legacy_hook_bytes_unchanged() {
    let legacy: Value = serde_json::from_str(include_str!(
        "../../../test-vectors/step-transform-provider-v1/hook-requests.json"
    ))
    .unwrap();
    for case in legacy["requests"].as_array().unwrap() {
        let call: HookCall = serde_json::from_value(case["request"].clone()).unwrap();
        assert!(call.subject_mid.is_none());
        assert!(call.subject_ordinal.is_none());
        assert!(call.message.is_none());
        assert!(call.descends_from.is_none());
        assert_eq!(call.check_host_fields(), Ok(()));
        // Compare compact bytes in JSON object-key order against the unchanged
        // vectors; no expected value is derived from the new serializer.
        assert_eq!(
            serde_json::to_vec(&serde_json::to_value(&call).unwrap()).unwrap(),
            serde_json::to_vec(&case["request"]).unwrap(),
            "{}",
            case["name"]
        );
    }
    assert_eq!(
        serde_json::to_string(&hook()).unwrap(),
        r#"{"session":"s","harness":"host","params":{},"hook":"pre_user","blocks":[]}"#
    );
}

#[test]
fn hook_message_counts_toward_request_cap() {
    assert_eq!(DEFAULT_HOOK_CAP_BYTES, 4_194_304);
    let call = hook().with_subject_identity("m", 0).with_message(json!(""));
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
    assert_eq!(HookProblem::RequestTooLarge.field(), "message");
    assert_eq!(
        hook().check_message_size(0),
        Ok(()),
        "no new cap on legacy hooks"
    );
}

#[test]
fn hook_history_refusals_round_trip_and_ignore_unrelated_hints() {
    for case in host_vectors()["refusals"].as_array().unwrap() {
        let refusal: ErrorBody = serde_json::from_value(case["refusal"].clone()).unwrap();
        assert_eq!(serde_json::to_value(&refusal).unwrap(), case["refusal"]);
        if refusal.code == TRANSIENT {
            assert_eq!(refusal.history_gap_from(), Some(1));
        } else {
            assert_eq!(refusal.history_gap_from(), None);
        }
    }
    for code in [INVALID_PARAMS, "not_subscribed", "unknown"] {
        let refusal =
            ErrorBody::new(code, "no history hint").with_detail(json!({"history_gap_from": 0}));
        assert_eq!(refusal.history_gap_from(), None);
    }
    assert_eq!(ErrorBody::new(TRANSIENT, "busy").history_gap_from(), None);
}

#[test]
fn host_fields_keep_u64_precision_and_lenient_decoding() {
    let call = hook()
        .with_subject_identity("m", u64::MAX)
        .with_descends_from(DescendsFrom::new("old", u64::MAX));
    let mut wire = serde_json::to_value(&call).unwrap();
    wire["unknown"] = json!(true);
    wire["descends_from"]["unknown"] = json!(true);
    let decoded: HookCall = serde_json::from_value(wire).unwrap();
    assert_eq!(decoded, call);
    for case in host_vectors()["undecodable"].as_array().unwrap() {
        assert!(
            serde_json::from_value::<HookCall>(case["request"].clone()).is_err(),
            "{}",
            case["name"]
        );
    }
}
