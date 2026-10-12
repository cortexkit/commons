use cortexkit_role_step_transform_provider::{
    claude_code::{
        is_valid_capability_identifier, ClaudeCodeCapabilities, ClaudeCodeLane, CAPABILITY,
        PRINCIPAL,
    },
    describe::RoleDescribe,
    hook::HookCall,
    subscription::{Declaration, DeclareRequest},
};
use serde::{de::DeserializeOwned, Serialize};
use serde_json::{json, Value};

fn vectors() -> Value {
    serde_json::from_str(include_str!(
        "../../../test-vectors/step-transform-provider-v1/claude-code.json"
    ))
    .unwrap()
}
fn bundle() -> ClaudeCodeCapabilities {
    serde_json::from_value(vectors()["bundle"].clone()).unwrap()
}
fn round_trip<T: DeserializeOwned + Serialize>(wire: &Value) -> T {
    let decoded: T = serde_json::from_value(wire.clone()).unwrap();
    assert_eq!(serde_json::to_value(&decoded).unwrap(), *wire);
    decoded
}
fn with_bundle(mut wire: Value) -> Value {
    wire["claude_code"] = vectors()["bundle"].clone();
    wire
}

#[test]
fn capabilities_round_trip_in_describe_and_declare() {
    round_trip::<ClaudeCodeCapabilities>(&vectors()["bundle"]);
    let describe: RoleDescribe = round_trip(&with_bundle(vectors()["describe_base"].clone()));
    let request: DeclareRequest =
        round_trip(&with_bundle(vectors()["declare_request_base"].clone()));
    let answer: Declaration = round_trip(&with_bundle(vectors()["declaration_base"].clone()));
    assert_eq!(describe.claude_code, Some(bundle()));
    assert_eq!(request.claude_code, answer.claude_code);
    assert_eq!(bundle().validate(), Ok(()));
    assert_eq!(bundle().capability, CAPABILITY);
}

#[test]
fn attested_and_declared_applies_and_freezes_connection_bundle() {
    let request = DeclareRequest::new(Default::default()).with_claude_code(bundle());
    let lane = request.admit_claude_code(PRINCIPAL, "connection1").unwrap();
    assert_eq!(
        lane.selected_for(PRINCIPAL, "connection1"),
        Ok(Some(&bundle()))
    );
    assert_eq!(
        lane.selected_for(PRINCIPAL, "connection2")
            .unwrap_err()
            .field,
        "claude_code"
    );
    assert_eq!(
        lane.selected_for("reserved:other", "connection1")
            .unwrap_err()
            .field,
        "claude_code"
    );
    assert_eq!(lane.check_frozen(Some(&bundle())), Ok(()));
    let mut changed = bundle();
    changed.epochs.insert("codec".into(), 999);
    assert_eq!(
        lane.check_frozen(Some(&changed)).unwrap_err().field,
        "claude_code"
    );
    assert_eq!(lane.check_frozen(None).unwrap_err().field, "claude_code");
}

#[test]
fn declared_without_attestation_refuses_by_name() {
    let request = DeclareRequest::new(Default::default()).with_claude_code(bundle());
    let refusal = request
        .admit_claude_code("reserved:other", "connection1")
        .unwrap_err();
    assert_eq!(refusal.code(), "invalid_params");
    assert_eq!(refusal.field, "claude_code");
    let cases = vectors();
    let case = &cases["admission"][1];
    assert_eq!(case["field"], refusal.field);
    assert_eq!(case["code"], refusal.code());
}

#[test]
fn attested_without_declaration_and_ordinary_absence_behave_as_today() {
    for principal in [PRINCIPAL, "reserved:broca"] {
        let request = DeclareRequest::new(Default::default());
        let lane = request.admit_claude_code(principal, "connection1").unwrap();
        assert_eq!(lane.selected_for(principal, "connection1"), Ok(None));
        assert_eq!(
            serde_json::to_value(&request).unwrap(),
            json!({"params": {}})
        );
        assert_eq!(lane.check_frozen(None), Ok(()));
        assert_eq!(
            lane.check_frozen(Some(&bundle())).unwrap_err().field,
            "claude_code"
        );
    }
    round_trip::<RoleDescribe>(&vectors()["describe_base"]);
    round_trip::<Declaration>(&vectors()["declaration_base"]);
}

#[test]
fn capability_identifiers_follow_daemon_grammar_and_epochs_are_fields() {
    for case in vectors()["identifier_cases"].as_array().unwrap() {
        assert_eq!(
            is_valid_capability_identifier(case["id"].as_str().unwrap()),
            case["valid"].as_bool().unwrap(),
            "{}",
            case["id"]
        );
    }
    assert!(!is_valid_capability_identifier(&format!(
        "{}/v1",
        "a".repeat(65)
    )));
    assert!(is_valid_capability_identifier(&format!(
        "{}/v1",
        "a".repeat(64)
    )));
    for id in [
        "claude-code-step/v01",
        "claude-code-step/v1/epoch3",
        "claude-code-step-profile3/v1",
    ] {
        let mut bundle = bundle();
        bundle.capability = id.into();
        assert_eq!(bundle.validate().unwrap_err().field, "claude_code");
    }
    let mut bundle = bundle();
    bundle.capabilities = vec!["bad_name/v1".into()];
    assert_eq!(bundle.validate().unwrap_err().field, "claude_code");
    bundle.capabilities.clear();
    bundle.bootstrap_receipt_format = "receipt/epoch3/v1".into();
    assert_eq!(bundle.validate().unwrap_err().field, "claude_code");
}

#[test]
fn capability_caps_and_admission_claims_refuse_when_inconsistent() {
    for field in [
        "principal",
        "harness",
        "observation",
        "exact_endpoint",
        "digest",
        "profile_generation",
        "provider_generation",
    ] {
        let mut wire = vectors()["bundle"].clone();
        wire[field] = json!("");
        let bundle: ClaudeCodeCapabilities = serde_json::from_value(wire).unwrap();
        assert_eq!(
            bundle.validate().unwrap_err().field,
            "claude_code",
            "{field}"
        );
    }
    for field in [
        "ingress_envelope_bytes",
        "hook_envelope_bytes",
        "single_message_bytes",
        "view_page_bytes",
    ] {
        let mut wire = vectors()["bundle"].clone();
        wire["caps"][field] = json!(0);
        let bundle: ClaudeCodeCapabilities = serde_json::from_value(wire).unwrap();
        assert_eq!(
            bundle.validate().unwrap_err().field,
            "claude_code",
            "{field}"
        );
    }
}

#[test]
fn hook_single_message_caps_refuse_by_name_only_on_selected_lane() {
    let lane = ClaudeCodeLane::admit(PRINCIPAL, "connection1", Some(bundle())).unwrap();
    let wire = json!({"session": "s", "harness": "claude-code", "params": {}, "hook": "pre_user", "blocks": ["x"], "message": {"content": "x".repeat(bundle().caps.single_message_bytes)}});
    let hook: HookCall = serde_json::from_value(wire).unwrap();
    assert_eq!(
        hook.validate_claude_code(&lane, PRINCIPAL, "connection1")
            .unwrap_err()
            .field,
        "message"
    );
    let off = ClaudeCodeLane::admit(PRINCIPAL, "connection1", None).unwrap();
    assert_eq!(
        hook.validate_claude_code(&off, PRINCIPAL, "connection1"),
        Ok(())
    );
}
