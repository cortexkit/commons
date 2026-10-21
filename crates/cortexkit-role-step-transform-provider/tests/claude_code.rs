use cortexkit_role_step_transform_provider::{
    claude_code::{
        is_valid_capability_identifier, ClaudeCodeCapabilities, ClaudeCodeLane, CAPABILITY,
        COMPUTED_TAGGER_EPOCH, PRINCIPAL,
    },
    describe::RoleDescribe,
    hook::HookCall,
    subscription::{Declaration, DeclareRequest},
};
use serde::{de::DeserializeOwned, Serialize};
use serde_json::{json, Value};

fn vectors() -> Value {
    serde_json::from_str(include_str!(
        "../test-vectors/step-transform-provider-v1/claude-code.json"
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
    let wire = json!({"session": "s", "harness": "claude-code", "params": {}, "hook": "pre_user", "blocks": ["x"], "raw_message": "{\"content\":\"x\"}", "message": {"content": "x".repeat(bundle().caps.single_message_bytes)}});
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

#[test]
fn capability_identifier_grammar_matches_daemon() {
    // This independent checker is the daemon's HELLO admission gate. Compare
    // both acceptance and rejection boundaries whenever its dependency changes.
    let mut cases = vec![
        ("a/v1".to_owned(), true),
        ("claude-code-step/v1".to_owned(), true),
        ("a1-b2/v2".to_owned(), true),
        ("a/v4294967295".to_owned(), true),
        ("a/v4294967296".to_owned(), false),
        ("a/v18446744073709551616".to_owned(), false),
        ("a/v0".to_owned(), false),
        ("a/v1alpha1".to_owned(), false),
        ("a/v01".to_owned(), false),
        ("a/v".to_owned(), false),
        ("a/v-1".to_owned(), false),
        ("a/v+1".to_owned(), false),
        ("claude.code/v1".to_owned(), false),
        ("-a/v1".to_owned(), false),
        ("a-/v1".to_owned(), false),
        ("a--b/v1".to_owned(), false),
        ("a_b/v1".to_owned(), false),
        ("A/v1".to_owned(), false),
        ("1a/v1".to_owned(), false),
        ("é/v1".to_owned(), false),
        ("a/v1/epoch3".to_owned(), false),
        ("a/v1/v2".to_owned(), false),
        ("/v1".to_owned(), false),
        ("a".to_owned(), false),
        ("".to_owned(), false),
        (" a/v1".to_owned(), false),
        ("a/v1 ".to_owned(), false),
        ("a/v1\n".to_owned(), false),
    ];
    cases.push((format!("{}/v1", "a".repeat(64)), true));
    cases.push((format!("{}/v1", "a".repeat(65)), false));
    for (identifier, expected) in cases {
        let local = is_valid_capability_identifier(&identifier);
        let daemon = subc_protocol::manifest::is_valid_capability_identifier(&identifier);
        assert_eq!(local, daemon, "checker drift for {identifier:?}");
        assert_eq!(
            daemon, expected,
            "unexpected daemon grammar for {identifier:?}"
        );
    }
}

fn native_vectors() -> Value {
    serde_json::from_str(include_str!(
        "../test-vectors/step-transform-provider-v1/claude-code-native-ingress.json"
    ))
    .unwrap()
}

#[test]
fn hook_raw_message_round_trips_exact_native_bytes() {
    assert_eq!(native_vectors()["hooks"].as_array().unwrap().len(), 3);
    for case in native_vectors()["hooks"].as_array().unwrap() {
        let hook: HookCall = round_trip(&case["request"]);
        let encoded = serde_json::to_vec(&hook).unwrap();
        let decoded: HookCall = serde_json::from_slice(&encoded).unwrap();
        assert_eq!(
            decoded.raw_message.as_ref().unwrap().as_bytes(),
            case["request"]["raw_message"].as_str().unwrap().as_bytes()
        );
        let lane = ClaudeCodeLane::admit(PRINCIPAL, "connection1", Some(bundle())).unwrap();
        assert_eq!(
            decoded.validate_claude_code(&lane, PRINCIPAL, "connection1"),
            Ok(())
        );
    }
}

#[test]
fn hook_raw_message_required_and_json_valid_only_on_admitted_lane() {
    let hook: HookCall =
        serde_json::from_value(native_vectors()["hooks"][0]["request"].clone()).unwrap();
    for raw in [None, Some(String::new()), Some("not-json".into())] {
        let mut hook = hook.clone();
        hook.raw_message = raw;
        let lane = ClaudeCodeLane::admit(PRINCIPAL, "connection1", Some(bundle())).unwrap();
        let refusal = hook
            .validate_claude_code(&lane, PRINCIPAL, "connection1")
            .unwrap_err();
        assert_eq!(refusal.code(), "invalid_params");
        assert_eq!(refusal.field, "raw_message");
        let off = ClaudeCodeLane::admit(PRINCIPAL, "connection1", None).unwrap();
        assert_eq!(
            hook.validate_claude_code(&off, PRINCIPAL, "connection1"),
            Ok(())
        );
    }
}

#[test]
fn hook_raw_sidecar_counts_toward_single_message_cap() {
    let mut hook: HookCall =
        serde_json::from_value(native_vectors()["hooks"][0]["request"].clone()).unwrap();
    hook.raw_message = Some(format!(
        "{}{}",
        " ".repeat(bundle().caps.single_message_bytes),
        hook.raw_message.unwrap()
    ));
    let lane = ClaudeCodeLane::admit(PRINCIPAL, "connection1", Some(bundle())).unwrap();
    assert_eq!(
        hook.validate_claude_code(&lane, PRINCIPAL, "connection1")
            .unwrap_err()
            .field,
        "message"
    );
}

fn assert_epoch_required(epoch: &str) {
    let mut selected = bundle();
    selected.epochs.remove(epoch);
    let refusal = ClaudeCodeLane::admit(PRINCIPAL, "connection1", Some(selected)).unwrap_err();
    assert_eq!(refusal.field, "claude_code");
    assert_eq!(refusal.code(), "invalid_params");
}

/// The fixture advertises the computed tagger, and an older tagger epoch is
/// still admitted: the epoch is advertised data, not an admission gate.
#[test]
fn the_fixture_advertises_the_computed_tagger_epoch() {
    assert_eq!(bundle().epochs.get("tagger"), Some(&COMPUTED_TAGGER_EPOCH));
    let mut older = bundle();
    older.epochs.insert("tagger".into(), 1);
    assert_eq!(older.validate(), Ok(()));
}

#[test]
fn capability_requires_tagger_epoch() {
    assert_epoch_required("tagger");
}

#[test]
fn capability_requires_memory_epoch() {
    assert_epoch_required("memory");
}

#[test]
fn capability_requires_compartment_epoch() {
    assert_epoch_required("compartment");
}
