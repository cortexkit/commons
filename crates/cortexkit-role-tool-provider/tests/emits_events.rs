//! Per-tool `emits_events`: the vectors in `emits-events.json`, and proof
//! that catalogs which never declare it keep their exact bytes and digests.

use cortexkit_role_tool_provider::{
    catalog::{schema_digest, CatalogAnswer, CatalogTool},
    vectors,
};
use serde_json::{json, Value};
use sha2::{Digest, Sha256};

fn vectors() -> Value {
    serde_json::from_str(vectors::EMITS_EVENTS_JSON).unwrap()
}

fn cases(group: &str) -> Vec<Value> {
    vectors()[group].as_array().unwrap().clone()
}

#[test]
fn absent_emits_events_preserves_existing_catalog_bytes_and_digests() {
    let answers: Value = serde_json::from_str(vectors::CATALOG_ANSWERS_JSON).unwrap();
    let modes: Value = serde_json::from_str(vectors::CALL_MODES_JSON).unwrap();
    let mut full = answers["full"].clone();
    // Unknown catalog extensions are intentionally not modeled by CatalogTool,
    // so this one would not survive a decode and re-encode.
    full["tools"][1]
        .as_object_mut()
        .unwrap()
        .remove("future_member");
    let mut catalogs = vec![("catalog-answers full".to_owned(), full, None)];
    for case in modes["digests"].as_array().unwrap() {
        catalogs.push((
            format!("call-modes {}", case["name"]),
            case["catalog"].clone(),
            Some(case["content_digest"].as_str().unwrap().to_owned()),
        ));
    }

    for (name, original, content_digest) in catalogs {
        assert!(
            !serde_json::to_string(&original)
                .unwrap()
                .contains("emits_events"),
            "{name}: the vector itself must not declare emits_events"
        );
        let catalog: CatalogAnswer = serde_json::from_value(original.clone()).unwrap();
        assert!(
            catalog.tools.iter().all(|tool| tool.emits_events.is_none()),
            "{name}"
        );
        // The struct's own encoding must not name the member at all, not even
        // as `null`.
        let direct = String::from_utf8(serde_json::to_vec(&catalog).unwrap()).unwrap();
        assert!(!direct.contains("emits_events"), "{name}: {direct}");
        let encoded = serde_json::to_value(&catalog).unwrap();
        assert_eq!(
            serde_json::to_vec(&encoded).unwrap(),
            serde_json::to_vec(&original).unwrap(),
            "{name}: re-encoding changed the catalog bytes"
        );
        if let Some(expected) = content_digest {
            let mut content = encoded;
            content.as_object_mut().unwrap().remove("catalog_digest");
            let hash = Sha256::digest(serde_jcs::to_vec(&content).unwrap());
            let digest: String = hash.iter().map(|byte| format!("{byte:02x}")).collect();
            assert_eq!(digest, expected, "{name}: content digest changed");
        }
    }
}

#[test]
fn declared_emits_events_vectors_decode_and_round_trip() {
    let declared = cases("declared");
    assert_eq!(declared.len(), 3);
    for case in declared {
        let name = &case["name"];
        let tool: CatalogTool = serde_json::from_value(case["tool"].clone()).unwrap();
        let expected: Option<bool> = serde_json::from_value(case["emits_events"].clone()).unwrap();
        assert_eq!(tool.emits_events, expected, "{name}");

        // The builder produces the same tool as decoding the recorded entry.
        let mut built = CatalogTool::new(
            tool.name.clone(),
            tool.schema_digest.clone(),
            tool.semantics,
            tool.input_schema.clone(),
        );
        if let Some(emits_events) = expected {
            built = built.with_emits_events(emits_events);
        }
        assert_eq!(built, tool, "{name}");

        assert_eq!(serde_json::to_value(&tool).unwrap(), case["tool"], "{name}");
    }
}

#[test]
fn explicit_null_emits_events_decodes_as_unknown_and_encodes_absent() {
    for case in cases("absent_on_encode") {
        let name = &case["name"];
        let tool: CatalogTool = serde_json::from_value(case["tool"].clone()).unwrap();
        assert_eq!(tool.emits_events, None, "{name}");
        let mut expected = case["tool"].clone();
        expected.as_object_mut().unwrap().remove("emits_events");
        assert_eq!(serde_json::to_value(&tool).unwrap(), expected, "{name}");
    }
}

#[test]
fn non_boolean_emits_events_does_not_decode() {
    let refused = cases("refused");
    assert!(!refused.is_empty());
    for case in refused {
        let name = &case["name"];
        let error = serde_json::from_value::<CatalogTool>(case["tool"].clone())
            .expect_err(name.as_str().unwrap())
            .to_string();
        assert!(error.contains("expected a boolean"), "{name}: {error}");
    }
}

#[test]
fn emits_events_is_catalog_content_not_schema_structure() {
    let schema = json!({"type": "object"});
    let digest = schema_digest(&schema).unwrap();
    let tool = CatalogTool::new("example", digest.clone(), 1, schema).with_emits_events(false);
    let wire = serde_json::to_value(&tool).unwrap();
    assert_eq!(wire["emits_events"], json!(false));
    assert_eq!(schema_digest(&tool.input_schema).unwrap(), digest);
    assert_eq!(serde_json::from_value::<CatalogTool>(wire).unwrap(), tool);
}
