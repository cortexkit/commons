use cortexkit_role_llm_runner::{
    capabilities,
    describe::{check_describe, RoleDescribe},
    errors, SYSTEM_TEXT_DEFAULT_PROVIDER_CAPABILITY,
};
use serde_json::{json, Value};

#[test]
fn system_text_default_vectors_and_open_codes() {
    let vector: Value = serde_json::from_str(include_str!(
        "../test-vectors/llm-runner-v1/system-text-default.json"
    ))
    .unwrap();
    let describe = check_describe(&vector["describe"]).unwrap();
    assert!(describe.system_text_default());
    assert!(!RoleDescribe::new(vec![], "old").system_text_default());
    let mut incomplete = vector["describe"].clone();
    incomplete["majors"][0]["ops"] = json!(["role.describe", "session.baseline"]);
    assert!(check_describe(&incomplete).is_err());
    assert!(describe.declares(capabilities::SYSTEM_TEXT_DEFAULT));
    let refusal = &vector["refusal"];
    assert_eq!(refusal["code"], errors::SYSTEM_TEXT_PRESET_REQUIRED);
    let detail: errors::SystemTextPresetRequiredDetail =
        serde_json::from_value(refusal["detail"].clone()).unwrap();
    assert_eq!(detail.provider, "example.system-text");
    assert_eq!(
        serde_json::to_value(errors::SystemTextPresetRequiredDetail::new(
            "example.system-text"
        ))
        .unwrap(),
        refusal["detail"]
    );
    assert!(errors::CODES.contains(&errors::SYSTEM_TEXT_PRESET_REQUIRED));
    assert!(!errors::is_retryable(errors::SYSTEM_TEXT_PRESET_REQUIRED));
    assert!(!errors::CODES.contains(&"future_refusal"));
    assert!(!errors::is_retryable("future_refusal"));
}

#[test]
fn system_text_default_shared_capability_name_matches_provider() {
    assert_eq!(
        SYSTEM_TEXT_DEFAULT_PROVIDER_CAPABILITY,
        cortexkit_role_tool_provider::SYSTEM_TEXT_DEFAULT
    );
    assert_eq!(
        SYSTEM_TEXT_DEFAULT_PROVIDER_CAPABILITY,
        "system-text-default/v1"
    );
}
