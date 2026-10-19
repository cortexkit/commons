use cortexkit_role_tool_provider::{
    catalog::{check_modes, serves_call_mode, CatalogAnswer, CatalogTool},
    vectors, CallMode, CALL_MODES_CAPABILITY,
};
use serde_json::{json, Value};
use sha2::{Digest, Sha256};

fn vectors() -> Value {
    serde_json::from_str(vectors::CALL_MODES_JSON).unwrap()
}

// Catalog digests are provider-defined. These vectors pin one scheme: SHA-256
// over the encoded catalog's JCS content, excluding catalog_digest itself.
fn content_digest(mut catalog: Value) -> String {
    catalog.as_object_mut().unwrap().remove("catalog_digest");
    let hash = Sha256::digest(serde_jcs::to_vec(&catalog).unwrap());
    hash.iter().map(|byte| format!("{byte:02x}")).collect()
}

#[test]
fn classic_default_omission_preserves_catalog_digest() {
    let case = &vectors()["digests"][0];
    let original: Value = serde_json::from_str(vectors::CATALOG_ANSWERS_JSON).unwrap();
    let mut original = original["full"].clone();
    // CatalogTool ignores unknown extensions, including future_member;
    // compare the modeled catalog content rather than that extension.
    original["tools"][1]
        .as_object_mut()
        .unwrap()
        .remove("future_member");
    assert_eq!(original, case["catalog"]);
    let absent: CatalogAnswer = serde_json::from_value(case["catalog"].clone()).unwrap();
    assert!(absent.tools.iter().all(|tool| tool.modes.is_none()));
    let mut explicit = absent.clone();
    for tool in &mut explicit.tools {
        // A classic-only list assigned directly to the field, bypassing
        // `with_modes`, must still be omitted on the wire.
        tool.modes = Some(vec![CallMode::Classic]);
    }
    for catalog in [absent, explicit] {
        let wire = serde_json::to_value(catalog).unwrap();
        assert_eq!(content_digest(wire.clone()), case["content_digest"]);
        assert_eq!(wire, case["catalog"]);
    }
}

#[test]
fn explicit_classic_decodes_to_absence_and_encodes_to_the_same_digest() {
    let case = &vectors()["digests"][0];
    let mut wire = case["catalog"].clone();
    for tool in wire["tools"].as_array_mut().unwrap() {
        tool["modes"] = json!(["classic"]);
    }
    let decoded: CatalogAnswer = serde_json::from_value(wire).unwrap();
    assert!(decoded.tools.iter().all(|tool| tool.modes.is_none()));
    let encoded = serde_json::to_value(decoded).unwrap();
    assert_eq!(content_digest(encoded.clone()), case["content_digest"]);
    assert_eq!(encoded, case["catalog"]);
}

#[test]
fn catalogs_with_modes_have_pinned_digests() {
    let vectors = vectors();
    for case in vectors["digests"].as_array().unwrap().iter().skip(1) {
        let catalog: CatalogAnswer = serde_json::from_value(case["catalog"].clone()).unwrap();
        for tool in &catalog.tools {
            assert_eq!(check_modes(tool), Ok(()), "{}", case["name"]);
        }
        let encoded = serde_json::to_value(catalog).unwrap();
        assert_eq!(
            content_digest(encoded.clone()),
            case["content_digest"],
            "{}",
            case["name"]
        );
        assert_eq!(encoded, case["catalog"], "{}", case["name"]);
    }
}

#[test]
fn providers_validate_nonempty_unique_known_modes() {
    for modes in [
        None,
        Some(vec![CallMode::Classic]),
        Some(vec![CallMode::Codemode]),
        Some(vec![CallMode::Flow, CallMode::Classic, CallMode::Codemode]),
    ] {
        let mut tool = CatalogTool::new("example", "unused", 1, json!({"type": "object"}));
        tool.modes = modes;
        assert_eq!(check_modes(&tool), Ok(()));
    }
    let vectors = vectors();
    for case in vectors["invalid"].as_array().unwrap() {
        let tool = CatalogTool::new("example", "unused", 1, json!({"type": "object"}))
            .with_modes(serde_json::from_value(case["modes"].clone()).unwrap());
        let problem = check_modes(&tool).expect_err(case["name"].as_str().unwrap());
        assert_eq!(problem.tool, "example");
        assert_eq!(problem.member, "modes");
        assert_eq!(problem.reason, case["reason"]);
        assert!(problem.to_string().contains("example modes:"));
    }
}

#[test]
fn each_known_mode_is_served_only_when_listed() {
    let base = CatalogTool::new("example", "unused", 1, json!({"type": "object"}));
    assert!(serves_call_mode(&base, &CallMode::Classic));
    assert!(!serves_call_mode(&base, &CallMode::Codemode));
    assert!(!serves_call_mode(&base, &CallMode::Flow));
    for mode in [CallMode::Classic, CallMode::Codemode, CallMode::Flow] {
        let tool = base.clone().with_modes(vec![mode.clone()]);
        for bind in [CallMode::Classic, CallMode::Codemode, CallMode::Flow] {
            assert_eq!(serves_call_mode(&tool, &bind), mode == bind);
        }
    }
    let empty = base.with_modes(vec![]);
    assert!(!serves_call_mode(&empty, &CallMode::Classic));
}

#[test]
fn unknown_modes_are_preserved_but_never_served() {
    for name in [
        "future-mode",
        "Classic",
        "",
        "mode\nwith\"escapes",
        "classic",
    ] {
        let unknown = CallMode::Unknown(name.to_owned());
        let tool = CatalogTool::new("example", "unused", 1, json!({"type": "object"}))
            .with_modes(vec![unknown.clone()]);
        assert!(!serves_call_mode(&tool, &unknown));
        assert!(!serves_call_mode(&CatalogTool::default(), &unknown));
        assert!(check_modes(&tool).is_err());
        if name != "classic" {
            let encoded = serde_json::to_value(&tool).unwrap();
            assert_eq!(encoded["modes"], json!([name]));
            let decoded: CatalogTool = serde_json::from_value(encoded.clone()).unwrap();
            assert_eq!(decoded.modes, Some(vec![unknown]));
            assert_eq!(serde_json::to_value(decoded).unwrap(), encoded);
        }
    }
}

#[test]
fn mode_builder_and_protocol_reexports_agree() {
    assert_eq!(CALL_MODES_CAPABILITY, "call-modes/v1");
    let mode: subc_protocol::call_mode::CallMode = CallMode::Flow;
    let tool =
        CatalogTool::new("example", "unused", 1, json!({"type": "object"})).with_modes(vec![mode]);
    assert_eq!(
        serde_json::to_value(tool).unwrap()["modes"],
        json!(["flow"])
    );
    let default = CatalogTool::default().with_modes(vec![CallMode::Classic]);
    assert!(default.modes.is_none());
}
