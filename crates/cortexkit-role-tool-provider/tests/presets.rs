//! `CatalogAnswer.presets`: the presets a provider serves, listed only in
//! the answer to a request without a preset.

use cortexkit_role_tool_provider::{
    catalog::{
        check_presets, valid_preset_name, CatalogAnswer, CatalogRequest, MAX_PRESETS,
        MAX_PRESET_NAME_BYTES,
    },
    errors, vectors,
};
use serde_json::Value;
use sha2::{Digest, Sha256};

fn cases() -> Value {
    serde_json::from_str(vectors::PRESETS_JSON).unwrap()
}

fn jcs(value: &Value) -> String {
    String::from_utf8(serde_jcs::to_vec(value).unwrap()).unwrap()
}

#[test]
fn an_empty_preset_list_is_omitted_and_keeps_old_catalog_bytes() {
    let data = cases();
    let prior: Value = serde_json::from_str(vectors::USER_COMMANDS_JSON).unwrap();
    // The absent row is the older vectors' catalog, byte for byte.
    assert_eq!(
        data["catalogs"][0]["catalog"],
        prior["catalogs"][0]["catalog"]
    );
    assert_eq!(
        data["catalogs"][0]["encoded_jcs"],
        prior["catalogs"][0]["encoded_jcs"]
    );
    assert_eq!(
        data["catalogs"][0]["content_digest"],
        prior["catalogs"][0]["content_digest"]
    );

    let answer: CatalogAnswer =
        serde_json::from_value(data["catalogs"][0]["catalog"].clone()).unwrap();
    let before = serde_json::to_vec(&answer).unwrap();
    let cleared = answer.with_presets(Vec::<String>::new());
    assert!(cleared.presets.is_empty());
    let encoded = serde_json::to_value(&cleared).unwrap();
    assert!(encoded.get("presets").is_none(), "{encoded}");
    assert_eq!(before, serde_json::to_vec(&cleared).unwrap());
    assert_eq!(jcs(&encoded), data["catalogs"][0]["encoded_jcs"]);
    assert!(serde_json::to_value(CatalogAnswer::new("g", "d"))
        .unwrap()
        .get("presets")
        .is_none());
}

#[test]
fn a_present_preset_list_is_catalog_content_covered_by_the_digest() {
    let data = cases();
    for row in data["catalogs"].as_array().unwrap() {
        let answer: CatalogAnswer = serde_json::from_value(row["catalog"].clone()).unwrap();
        check_presets(&answer, None).unwrap();
        let mut encoded = serde_json::to_value(&answer).unwrap();
        assert_eq!(jcs(&encoded), row["encoded_jcs"], "{}", row["name"]);
        encoded.as_object_mut().unwrap().remove("catalog_digest");
        assert_eq!(
            format!("{:x}", Sha256::digest(serde_jcs::to_vec(&encoded).unwrap())),
            row["content_digest"],
            "{}",
            row["name"]
        );
    }
    assert_ne!(
        data["catalogs"][0]["content_digest"],
        data["catalogs"][1]["content_digest"]
    );
    let present: CatalogAnswer =
        serde_json::from_value(data["catalogs"][1]["catalog"].clone()).unwrap();
    assert_eq!(present.presets, ["default", "review", "read_only"]);
    assert!(present.lists_preset("review"));
    assert!(!present.lists_preset("Review"));
    assert!(!present.lists_preset("missing"));
    let built = CatalogAnswer::new("g", "d").with_presets(["default", "review"]);
    assert_eq!(
        serde_json::to_value(&built).unwrap()["presets"],
        serde_json::json!(["default", "review"])
    );
}

#[test]
fn preset_list_vectors_validate_names_uniqueness_and_count() {
    let data = cases();
    for row in data["valid"].as_array().unwrap() {
        let answer = CatalogAnswer::new("g", "d")
            .with_presets(serde_json::from_value::<Vec<String>>(row["presets"].clone()).unwrap());
        check_presets(&answer, None).unwrap_or_else(|error| panic!("{row}: {error:?}"));
    }
    for row in data["invalid"].as_array().unwrap() {
        let answer = CatalogAnswer::new("g", "d")
            .with_presets(serde_json::from_value::<Vec<String>>(row["presets"].clone()).unwrap());
        let error = check_presets(&answer, row["request_preset"].as_str())
            .expect_err(row["case"].as_str().unwrap());
        assert_eq!(
            errors::invalid_request_field(&error),
            row["field"].as_str(),
            "{}",
            row["case"]
        );
        if let Some(preset) = row.get("preset") {
            assert_eq!(
                error.detail.as_ref().unwrap()["preset"],
                *preset,
                "{}",
                row["case"]
            );
        }
    }
}

#[test]
fn a_preset_list_in_a_preset_answer_is_malformed() {
    let data = cases();
    let row = &data["preset_answer"];
    let request: CatalogRequest = serde_json::from_value(row["request"].clone()).unwrap();
    let answer: CatalogAnswer = serde_json::from_value(row["answer"].clone()).unwrap();
    assert!(!answer.presets.is_empty());
    let error = check_presets(&answer, request.preset.as_deref()).unwrap_err();
    assert_eq!(errors::invalid_request_field(&error), row["field"].as_str());
    // The same list is fine in the answer to a request without a preset.
    check_presets(&answer, None).unwrap();
    // A preset answer without a list is fine.
    let mut cleared = answer.clone();
    cleared.presets.clear();
    check_presets(&cleared, Some("review")).unwrap();
}

#[test]
fn preset_name_bounds() {
    assert_eq!(MAX_PRESETS, 16);
    assert_eq!(MAX_PRESET_NAME_BYTES, 32);
    for name in ["a", "default", "read_only", "deep-dive2", "a-_9"] {
        assert!(valid_preset_name(name), "{name}");
        // Every listable name is also a valid preset on a tool call.
        subc_protocol::tool_call::validate_preset(name).unwrap();
    }
    assert!(valid_preset_name(&"a".repeat(32)));
    for name in [
        "",
        "A",
        "1a",
        "_a",
        "-a",
        "a b",
        "a.b",
        "é",
        &"a".repeat(33),
    ] {
        assert!(!valid_preset_name(name), "{name:?}");
    }
}
