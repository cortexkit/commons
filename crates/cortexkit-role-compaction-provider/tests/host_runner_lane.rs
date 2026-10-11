use cortexkit_role_compaction_provider::{
    errors::{ErrorBody, INVALID_PARAMS},
    setup::SetupRequest,
    status::{
        AppliedRef, DescendsFrom, Estimate, MessageRef, NotAppliedRef, PrefixRebuild,
        StatusMessage, StepStatus, UnservedSubject, Usage,
    },
    OpRequest,
};
use cortexkit_role_step_transform_provider::subscription::Hook;
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};

fn vectors() -> Value {
    serde_json::from_str(include_str!(
        "../../../test-vectors/compaction-provider-v1/host-runner-lane.json"
    ))
    .unwrap()
}

fn cases<'a>(vectors: &'a Value, key: &str) -> &'a [Value] {
    let cases = vectors[key].as_array().expect(key);
    assert!(!cases.is_empty(), "{key} must exercise at least one case");
    cases
}

fn bytes(value: &Value) -> Vec<u8> {
    // Compare compact bytes in object-key order, independently of source-file
    // whitespace and the order in which serde visits struct fields.
    serde_json::to_vec(value).unwrap()
}

fn status_wire(request: &Value) -> Value {
    let mut wire = request.clone();
    // The admission vectors deliberately include a body `params` hint. It is
    // not a StepStatus member and cannot confer authority; lenient readers must
    // drop it. All actual status members must survive byte-for-byte.
    wire.as_object_mut().unwrap().remove("params");
    wire
}

#[test]
fn lane_requests_round_trip() {
    let vectors = vectors();
    for key in ["requests", "checks", "admission"] {
        for case in cases(&vectors, key) {
            let status: StepStatus = serde_json::from_value(case["request"].clone()).unwrap();
            let encoded = serde_json::to_value(&status).unwrap();
            assert_eq!(
                bytes(&encoded),
                bytes(&status_wire(&case["request"])),
                "{}",
                case["name"]
            );
            assert_eq!(
                serde_json::from_value::<StepStatus>(encoded).unwrap(),
                status
            );
        }
    }
    for case in cases(&vectors, "plan_value_refusals") {
        let request: OpRequest<SetupRequest> =
            serde_json::from_value(case["request"].clone()).unwrap();
        let encoded = serde_json::to_value(&request).unwrap();
        assert_eq!(bytes(&encoded), bytes(&case["request"]), "{}", case["name"]);
        assert_eq!(
            serde_json::from_value::<OpRequest<SetupRequest>>(encoded).unwrap(),
            request
        );
    }
}

// Frozen pre-host-lane status shape. Do not substitute StepStatus: an old
// reader must ignore the extensions without parsing them or relying on them.
#[derive(Debug, PartialEq, Deserialize, Serialize)]
struct LegacyStepStatus {
    session: String,
    harness: String,
    request_id: String,
    lineage_id: String,
    step_id: String,
    step_kind: String,
    model: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    variant: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    context_window: Option<u64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    output_limit: Option<u64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    previous_usage: Option<Usage>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    previous_provider_code: Option<String>,
    estimate: Estimate,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    prefix_rebuilding: Option<PrefixRebuild>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    newest: Option<MessageRef>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    last_applied: Option<AppliedRef>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    last_not_applied: Option<NotAppliedRef>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    after_ordinal: Option<u64>,
    messages: Vec<StatusMessage>,
    #[serde(default, skip_serializing_if = "is_false")]
    more: bool,
    now: u64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    descends_from: Option<DescendsFrom>,
}

fn is_false(value: &bool) -> bool {
    !*value
}

#[test]
fn lane_legacy_reader_ignores_extensions() {
    let vectors = vectors();
    for key in ["requests", "checks", "admission"] {
        for case in cases(&vectors, key) {
            let mut stripped = status_wire(&case["request"]);
            for field in ["served_through_ordinal", "unserved_subjects"] {
                stripped.as_object_mut().unwrap().remove(field);
            }
            let legacy: LegacyStepStatus = serde_json::from_value(case["request"].clone()).unwrap();
            let expected: LegacyStepStatus = serde_json::from_value(stripped.clone()).unwrap();
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
        let status: StepStatus = serde_json::from_value(case["request"].clone()).unwrap();
        assert_eq!(status.check_messages(), Ok(()), "{}", case["name"]);
        let field = case["refusal"]["detail"]["field"]
            .as_str()
            .filter(|field| field.starts_with("unserved_subjects["));
        if let Some(field) = field {
            let problem = status.validate().expect_err(case["name"].as_str().unwrap());
            assert_eq!(problem.field(), field, "{}", case["name"]);
            let refusal = ErrorBody::new(INVALID_PARAMS, format!("invalid {field}"))
                .with_detail(json!({"field": problem.field()}));
            assert_eq!(serde_json::to_value(refusal).unwrap(), case["refusal"]);
            refusals += 1;
        } else {
            // Held-history gaps/conflicts, monotonicity and promotion require
            // provider state; sparse messages alone do not imply any gap.
            assert_eq!(status.validate(), Ok(()), "{}", case["name"]);
        }
        if let Some(size) = case["subject_part_bytes"].as_u64() {
            let part = status.unserved_subjects.as_ref().unwrap()[0]
                .subject_part
                .as_ref()
                .unwrap();
            assert_eq!(part.len() as u64, size);
        }
    }
    assert_eq!(
        refusals, 2,
        "exercise both empty and oversized burn-entry parts"
    );

    let base: StepStatus =
        serde_json::from_value(cases(&vectors, "requests")[0]["request"].clone()).unwrap();
    let valid = UnservedSubject::new("m", Hook::PostTool).with_subject_part("é".repeat(128));
    assert_eq!(
        base.clone()
            .with_unserved_subjects(vec![valid.clone()])
            .validate(),
        Ok(())
    );
    for part in [String::new(), format!("{}x", "é".repeat(128))] {
        let status = base.clone().with_unserved_subjects(vec![
            valid.clone(),
            UnservedSubject::new("other", Hook::PostTool).with_subject_part(part),
        ]);
        assert_eq!(
            status.validate().unwrap_err().field(),
            "unserved_subjects[1].subject_part"
        );
    }
}

#[test]
fn lane_runner_plan_value_refusals() {
    let vectors = vectors();
    for case in cases(&vectors, "plan_value_refusals") {
        assert_eq!(case["route"]["authenticated_principal"], "runner");
        let request: OpRequest<SetupRequest> =
            serde_json::from_value(case["request"].clone()).unwrap();
        assert_eq!(request.method, "compaction.setup");
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
        let mut wire = cases(&vectors, "plan_value_refusals")[0]["request"]["params"].clone();
        wire["params"] = params;
        let request: SetupRequest = serde_json::from_value(wire).unwrap();
        assert_eq!(request.check_runner_params(), Ok(()));
    }
}

#[test]
fn lane_setters_and_shared_strict_entry_identity() {
    let vectors = vectors();
    let base: StepStatus =
        serde_json::from_value(cases(&vectors, "requests")[0]["request"].clone()).unwrap();
    assert!(base.served_through_ordinal.is_none());
    assert!(base.unserved_subjects.is_none());
    let entry = UnservedSubject::new("m", Hook::PostTool).with_subject_part("part-a");
    // This assignment is a compile-time check that both roles share one type.
    let shared: cortexkit_role_step_transform_provider::subscription::UnservedSubject =
        entry.clone();
    assert_eq!(shared, entry);
    let status = base
        .with_served_through_ordinal(u64::MAX)
        .with_unserved_subjects(vec![entry]);
    let mut wire = serde_json::to_value(&status).unwrap();
    wire["unknown"] = json!(true);
    wire["unserved_subjects"][0]["unknown"] = json!(true);
    // These are deliberately not step fields and must never leak into answers
    // or into a re-encoded status.
    wire["subject_part"] = json!("ignored");
    wire["pass_complete"] = json!(true);
    let decoded: StepStatus = serde_json::from_value(wire.clone()).unwrap();
    assert_eq!(decoded, status);
    assert_eq!(decoded.served_through_ordinal, Some(u64::MAX));
    assert_eq!(decoded.validate(), Ok(()));
    wire["unserved_subjects"][0]["hook"] = json!("unknown_hook");
    assert!(serde_json::from_value::<StepStatus>(wire.clone()).is_err());
    wire["unserved_subjects"][0]["hook"] = json!("post_tool");
    wire["unserved_subjects"][0]
        .as_object_mut()
        .unwrap()
        .remove("subject_mid");
    assert!(serde_json::from_value::<StepStatus>(wire).is_err());
}
