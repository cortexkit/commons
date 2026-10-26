use cortexkit_role_tool_provider::{
    call::SchemaPin,
    catalog::{schema_digest, CatalogAnswer, CatalogTool, ToolLoad},
    vectors,
};
use serde_json::{json, Value};
use sha2::{Digest, Sha256};

fn content_digest(mut catalog: Value) -> String {
    catalog.as_object_mut().unwrap().remove("catalog_digest");
    let hash = Sha256::digest(serde_jcs::to_vec(&catalog).unwrap());
    hash.iter().map(|byte| format!("{byte:02x}")).collect()
}

fn schema_pin(tool: &CatalogTool) -> String {
    SchemaPin::new(&tool.name, &tool.schema_digest, tool.semantics)
        .encode()
        .unwrap()
}

#[test]
fn upfront_load_round_trips_and_has_a_builder() {
    let schema = json!({"type": "object"});
    let tool = CatalogTool::new("example", schema_digest(&schema).unwrap(), 1, schema)
        .with_load(ToolLoad::Upfront);
    let wire = serde_json::to_value(&tool).unwrap();
    assert_eq!(wire["load"], "upfront");
    let decoded: CatalogTool = serde_json::from_value(wire.clone()).unwrap();
    assert_eq!(decoded.load, Some(ToolLoad::Upfront));
    assert_eq!(decoded.load.as_ref().unwrap().as_str(), "upfront");
    assert_eq!(decoded, tool);
    assert_eq!(serde_json::to_value(decoded).unwrap(), wire);
}

#[test]
fn unknown_load_round_trips_unchanged() {
    let wire = json!({
        "name": "example",
        "schema_digest": "digest",
        "semantics": 1,
        "capabilities": [],
        "input_schema": {"type": "object"},
        "load": "future/v9"
    });
    let decoded: CatalogTool = serde_json::from_value(wire.clone()).unwrap();
    assert_eq!(decoded.load, Some(ToolLoad::Unknown("future/v9".into())));
    assert_eq!(decoded.load.as_ref().unwrap().as_str(), "future/v9");
    assert_eq!(serde_json::to_value(decoded).unwrap(), wire);
}

#[test]
fn absent_load_preserves_existing_catalog_bytes_and_pinned_digests() {
    let cases: Value = serde_json::from_str(vectors::CALL_MODES_JSON).unwrap();
    for case in cases["digests"].as_array().unwrap() {
        let catalog: CatalogAnswer = serde_json::from_value(case["catalog"].clone()).unwrap();
        assert!(catalog.tools.iter().all(|tool| tool.load.is_none()));
        let encoded = serde_json::to_value(catalog).unwrap();
        for tool in encoded["tools"].as_array().unwrap() {
            assert!(!tool.as_object().unwrap().contains_key("load"));
        }
        assert_eq!(
            serde_json::to_vec(&encoded).unwrap(),
            serde_json::to_vec(&case["catalog"]).unwrap()
        );
        assert_eq!(content_digest(encoded), case["content_digest"]);
    }
}

#[test]
fn present_load_changes_only_the_catalog_digest_not_the_schema_digest_or_pin() {
    let cases: Value = serde_json::from_str(vectors::CALL_MODES_JSON).unwrap();
    let base = &cases["digests"][0];
    let catalog: CatalogAnswer = serde_json::from_value(base["catalog"].clone()).unwrap();
    let original_tool = &catalog.tools[0];
    let original_pin = schema_pin(original_tool);
    let original_schema_digest = schema_digest(&original_tool.input_schema).unwrap();
    assert_eq!(original_tool.schema_digest, original_schema_digest);
    assert_eq!(
        content_digest(serde_json::to_value(&catalog).unwrap()),
        base["content_digest"]
    );

    for load in [ToolLoad::Upfront, ToolLoad::Unknown("future/v9".into())] {
        let mut changed = catalog.clone();
        changed.tools[0] = original_tool.clone().with_load(load);
        let encoded = serde_json::to_value(&changed).unwrap();
        assert_ne!(content_digest(encoded.clone()), base["content_digest"]);
        let decoded: CatalogAnswer = serde_json::from_value(encoded).unwrap();
        let tool = &decoded.tools[0];
        assert_eq!(tool.schema_digest, original_schema_digest);
        assert_eq!(
            schema_digest(&tool.input_schema).unwrap(),
            original_schema_digest
        );
        assert_eq!(schema_pin(tool), original_pin);
        assert_eq!(tool.semantics, original_tool.semantics);
    }
}
