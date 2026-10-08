use cortexkit_role_step_transform_provider::{
    errors::{ErrorBody, INVALID_PARAMS},
    hook::{DescendsFrom, HookCall, Subject},
    subscription::{DeclareRequest, Hook, UnservedSubject},
    OpRequest,
};
use serde::{Deserialize, Serialize};
use serde_json::{json, Map, Value};

fn vectors() -> Value {
    serde_json::from_str(include_str!(
        "../../../test-vectors/step-transform-provider-v1/host-runner-lane.json"
    ))
    .unwrap()
}

fn cases<'a>(vectors: &'a Value, key: &str) -> &'a [Value] {
    let cases = vectors[key].as_array().expect(key);
    assert!(!cases.is_empty(), "{key} must exercise at least one case");
    cases
}

fn bytes(value: &Value) -> Vec<u8> {
    // Compact JSON in object-key order: whitespace and key placement in the
    // source file are not part of the wire contract, but omission and values are.
    serde_json::to_vec(value).unwrap()
}

fn round_trip<T>(case: &Value)
where
    T: serde::de::DeserializeOwned + Serialize + PartialEq + std::fmt::Debug,
{
    let request: T = serde_json::from_value(case["request"].clone()).unwrap();
    let encoded = serde_json::to_value(&request).unwrap();
    assert_eq!(bytes(&encoded), bytes(&case["request"]), "{}", case["name"]);
    assert_eq!(
        serde_json::from_value::<T>(encoded).unwrap(),
        request,
        "{}",
        case["name"]
    );
}

#[test]
fn lane_requests_round_trip() {
    let vectors = vectors();
    for key in ["requests", "checks", "admission"] {
        for case in cases(&vectors, key) {
            round_trip::<HookCall>(case);
        }
    }
    for case in cases(&vectors, "plan_value_refusals") {
        round_trip::<OpRequest<DeclareRequest>>(case);
    }
}

// Frozen pre-host-lane shape. Reusing HookCall here would test the new reader
// against itself and could not prove that an older reader ignores extensions.
#[derive(Debug, PartialEq, Deserialize, Serialize)]
struct LegacyHookCall {
    session: String,
    harness: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    lineage_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    subject_mid: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    subject_ordinal: Option<u64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    message: Option<Value>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    descends_from: Option<DescendsFrom>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    preset: Option<String>,
    #[serde(default)]
    params: Map<String, Value>,
    #[serde(flatten)]
    subject: Subject,
}

#[test]
fn lane_legacy_reader_ignores_extensions() {
    let vectors = vectors();
    for key in ["requests", "checks", "admission"] {
        for case in cases(&vectors, key) {
            let mut stripped = case["request"].clone();
            for field in [
                "served_through_ordinal",
                "unserved_subjects",
                "subject_part",
                "pass_complete",
            ] {
                stripped.as_object_mut().unwrap().remove(field);
            }
            let legacy: LegacyHookCall = serde_json::from_value(case["request"].clone()).unwrap();
            let expected: LegacyHookCall = serde_json::from_value(stripped.clone()).unwrap();
            assert_eq!(legacy, expected, "{}", case["name"]);
            assert_eq!(
                bytes(&serde_json::to_value(legacy).unwrap()),
                bytes(&stripped)
            );
        }
    }
}

#[test]
fn lane_subject_part_validation() {
    let vectors = vectors();
    let mut refusals = 0;
    for case in cases(&vectors, "checks") {
        let call: HookCall = serde_json::from_value(case["request"].clone()).unwrap();
        assert_eq!(call.check_host_fields(), Ok(()), "{}", case["name"]);
        let field = case["refusal"]["detail"]["field"]
            .as_str()
            .filter(|field| *field == "subject_part" || field.starts_with("unserved_subjects["));
        if let Some(field) = field {
            let problem = call.validate().expect_err(case["name"].as_str().unwrap());
            assert_eq!(problem.field(), field, "{}", case["name"]);
            let refusal = ErrorBody::new(INVALID_PARAMS, format!("invalid {field}"))
                .with_detail(json!({"field": problem.field()}));
            assert_eq!(serde_json::to_value(refusal).unwrap(), case["refusal"]);
            refusals += 1;
        } else {
            // Watermarks, burning and held-history effects belong to provider
            // conformance; only the request-local shape is checked here.
            assert_eq!(call.validate(), Ok(()), "{}", case["name"]);
        }
        if let Some(size) = case["subject_part_bytes"].as_u64() {
            assert_eq!(call.subject_part.as_ref().unwrap().len() as u64, size);
        }
    }
    assert_eq!(
        refusals, 3,
        "exercise empty, oversized and burn-entry parts"
    );

    // Check the actual index, not a hard-coded first-entry refusal, and an
    // empty burn-entry part, which is not present in the hook vectors.
    let base: HookCall =
        serde_json::from_value(cases(&vectors, "requests")[0]["request"].clone()).unwrap();
    let valid = UnservedSubject::new("m", Hook::PostTool).with_subject_part("é".repeat(128));
    assert_eq!(
        base.clone()
            .with_unserved_subjects(vec![valid.clone()])
            .validate(),
        Ok(())
    );
    for part in [String::new(), format!("{}x", "é".repeat(128))] {
        let call = base.clone().with_unserved_subjects(vec![
            valid.clone(),
            UnservedSubject::new("other", Hook::PostTool).with_subject_part(part),
        ]);
        assert_eq!(
            call.validate().unwrap_err().field(),
            "unserved_subjects[1].subject_part"
        );
    }
}

#[test]
fn lane_barrier_flags_are_not_acknowledgements() {
    let vectors = vectors();
    let mut barriers = 0;
    for case in cases(&vectors, "checks") {
        if let Some(scheduled) = case["historian_evaluations_scheduled"].as_u64() {
            let call: HookCall = serde_json::from_value(case["request"].clone()).unwrap();
            assert_eq!(
                u64::from(call.pass_complete == Some(true)),
                scheduled,
                "{}",
                case["name"]
            );
            assert_eq!(
                call.served_through_ordinal, None,
                "a barrier carries no acknowledgement"
            );
            barriers += 1;
        }
    }
    assert_eq!(barriers, 3, "exercise absent, true and false barriers");
}

#[test]
fn lane_runner_plan_value_refusals() {
    let vectors = vectors();
    for case in cases(&vectors, "plan_value_refusals") {
        assert_eq!(case["route"]["authenticated_principal"], "runner");
        let request: OpRequest<DeclareRequest> =
            serde_json::from_value(case["request"].clone()).unwrap();
        assert_eq!(request.method, "transform.declare");
        let problem = request
            .params
            .check_runner_params()
            .expect_err(case["name"].as_str().unwrap());
        let field = problem.field();
        let refusal = ErrorBody::new(INVALID_PARAMS, format!("invalid {field}"))
            .with_detail(json!({"field": field}));
        assert_eq!(serde_json::to_value(refusal).unwrap(), case["refusal"]);
    }
    for params in [
        json!({}),
        json!({"serializer_profile": "owned-broca"}),
        json!({"observation": "unknown"}),
    ] {
        let request: DeclareRequest = serde_json::from_value(json!({"params": params})).unwrap();
        assert_eq!(request.check_runner_params(), Ok(()));
    }
}

#[test]
fn lane_setters_and_strict_entry_identity() {
    let base = HookCall::new(
        "s",
        "opencode",
        Subject::PreUser {
            blocks: vec![],
            mark: None,
            delivery: None,
        },
    );
    assert!(base.served_through_ordinal.is_none());
    assert!(base.unserved_subjects.is_none());
    assert!(base.subject_part.is_none());
    assert!(base.pass_complete.is_none());
    let entry = UnservedSubject::new("m", Hook::PostTool).with_subject_part("é");
    let call = base
        .with_served_through_ordinal(u64::MAX)
        .with_unserved_subjects(vec![entry.clone()])
        .with_subject_part("e\u{301}")
        .with_pass_complete(false);
    let mut wire = serde_json::to_value(&call).unwrap();
    wire["unknown"] = json!(true);
    wire["unserved_subjects"][0]["unknown"] = json!(true);
    let decoded: HookCall = serde_json::from_value(wire.clone()).unwrap();
    assert_eq!(decoded, call);
    assert_eq!(decoded.served_through_ordinal, Some(u64::MAX));
    assert_eq!(decoded.pass_complete, Some(false));
    assert_ne!(
        decoded.subject_part, entry.subject_part,
        "opaque part bytes are not normalised"
    );
    assert_eq!(decoded.validate(), Ok(()));
    wire["unserved_subjects"][0]["hook"] = json!("unknown_hook");
    assert!(serde_json::from_value::<HookCall>(wire.clone()).is_err());
    wire["unserved_subjects"][0]["hook"] = json!("post_tool");
    wire["unserved_subjects"][0]
        .as_object_mut()
        .unwrap()
        .remove("subject_mid");
    assert!(serde_json::from_value::<HookCall>(wire).is_err());
}
