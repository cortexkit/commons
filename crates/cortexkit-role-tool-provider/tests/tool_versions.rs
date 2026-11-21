use cortexkit_role_tool_provider::{
    call::{
        check_versioned_call, SchemaPin, ToolCallRequest, ToolMajorPin, VersionedSchemaPin,
        SCHEMA_PIN_MAX_LEN,
    },
    catalog::{schema_digest, CatalogAnswer, ToolVersion},
    errors,
    views::{FieldKind, RecordSchema, ViewChange, ViewManifest, ViewTool},
};
use serde::Deserialize;
use serde_json::{json, Value};

fn fixture() -> Value {
    serde_json::from_str(include_str!(
        "../test-vectors/tool-provider-v1/tool-versions.json"
    ))
    .unwrap()
}

#[test]
fn tool_versions_catalog_validates_and_legacy_readers_ignore_version() {
    #[derive(Deserialize)]
    struct LegacyTool {
        name: String,
        semantics: u64,
    }
    let vector = fixture();
    let catalog: CatalogAnswer = serde_json::from_value(vector["catalog"].clone()).unwrap();
    let tool = &catalog.tools[0];
    assert_eq!(tool.version, Some(ToolVersion::new(2, 1, 0)));
    assert_eq!(
        tool.schema_digest,
        schema_digest(&tool.input_schema).unwrap()
    );
    assert_eq!(tool.schema_pin().encode().unwrap(), vector["current_pin"]);
    assert_eq!(serde_json::to_value(catalog).unwrap(), vector["catalog"]);
    let legacy: LegacyTool = serde_json::from_value(vector["catalog"]["tools"][0].clone()).unwrap();
    assert_eq!((legacy.name.as_str(), legacy.semantics), ("echo", 8));
    for version in vector["invalid_versions"].as_array().unwrap() {
        let mut malformed = vector["catalog"].clone();
        malformed["tools"][0]["version"] = version.clone();
        let message = serde_json::from_value::<CatalogAnswer>(malformed)
            .unwrap_err()
            .to_string();
        assert!(
            message.contains("echo") && message.contains("version"),
            "{version}: {message}"
        );
    }
    let max = "18446744073709551615.18446744073709551615.18446744073709551615";
    assert_eq!(
        max.parse::<ToolVersion>().unwrap(),
        ToolVersion::new(u64::MAX, u64::MAX, u64::MAX)
    );
}

#[test]
fn tool_versions_pins_and_refusal_vectors() {
    let vector = fixture();
    for case in vector["admission"].as_array().unwrap() {
        let request: ToolCallRequest = serde_json::from_value(case["request"].clone()).unwrap();
        let expected = request.schema_pin.as_deref().unwrap();
        let pin = check_versioned_call(&request);
        if case["outcome"] == "invalid_request" {
            let refusal = pin.unwrap_err();
            assert_eq!(refusal.code, case["outcome"]);
            assert_eq!(refusal.detail.unwrap(), case["detail"]);
        } else {
            assert_eq!(pin.unwrap().unwrap().encode().unwrap(), expected);
            if case["outcome"] == "tool_major_changed" {
                let refusal = errors::tool_major_changed(
                    &request.name,
                    expected,
                    vector["current_pin"].as_str().unwrap(),
                );
                assert_eq!(serde_json::to_value(refusal).unwrap(), case["refusal"]);
            }
        }
    }
}

#[test]
fn tool_versions_pin_bound_matches_legacy_with_the_largest_major() {
    let digest = "a".repeat(64);
    let tool = format!("{}é:", "t".repeat(SCHEMA_PIN_MAX_LEN - 90 - 9));
    let pin = ToolMajorPin::new(&tool, &digest, u64::MAX);
    let wire = pin.encode().unwrap();
    assert_eq!(wire.len(), SCHEMA_PIN_MAX_LEN);
    assert!(wire.ends_with(":18446744073709551615"));
    assert!(wire.contains("%C3%A9%3A:"));
    assert_eq!(
        VersionedSchemaPin::parse(&wire),
        Ok(VersionedSchemaPin::Tp2(pin))
    );
    assert_eq!(
        SchemaPin::new(&tool, &digest, u64::MAX)
            .encode()
            .unwrap()
            .len(),
        wire.len()
    );
    assert!(ToolMajorPin::new(format!("{tool}x"), digest, u64::MAX)
        .encode()
        .is_err());
}

#[test]
fn tool_versions_views_grow_and_keep_larger_schema_after_rollback() {
    let sample = fixture();
    let tool: ViewTool = serde_json::from_value(sample["view_tool"].clone()).unwrap();
    assert_eq!(serde_json::to_value(&tool).unwrap(), sample["view_tool"]);
    let pin = tool.schema_pin().unwrap();
    assert_eq!(pin, sample["current_pin"]);
    let old_tool = ViewTool::new_versioned(
        &tool.name,
        tool.input_schema.clone(),
        ToolVersion::new(2, 1, 0),
    );
    let schema = RecordSchema::new("echo", 1, [("status".to_owned(), FieldKind::Code)].into());
    let held = ViewManifest::new(schema, vec![], json!({})).with_tool(old_tool);
    assert_eq!(held.check_pin(&pin), Ok(true));
    let mut grown = held.clone().with_tool(tool).with_template_revision(1);
    grown.record_schema.revision = 2;
    grown
        .record_schema
        .completion_fields
        .insert("tail".into(), FieldKind::TextLines);
    assert_eq!(grown.check_pin(&pin), Ok(true));
    assert_eq!(grown.compare_with_kept(&held), ViewChange::NewerTemplate);
    assert_eq!(held.compare_with_kept(&grown), ViewChange::Stale);
    let mut wrong_kind = grown.clone();
    wrong_kind
        .record_schema
        .fields
        .insert("status".into(), FieldKind::Integer);
    assert_eq!(wrong_kind.compare_with_kept(&held), ViewChange::Conflict);
    assert_eq!(
        grown
            .clone()
            .with_template_revision(0)
            .compare_with_kept(&held),
        ViewChange::Conflict
    );
    let mut description = held.clone();
    description.tool.as_mut().unwrap().input_schema["description"] =
        json!("Echo the supplied text");
    assert_eq!(description.check_pin(&pin), Ok(true));
    assert_eq!(description.compare_with_kept(&held), ViewChange::Conflict);
    assert_eq!(
        description
            .with_template_revision(1)
            .compare_with_kept(&held),
        ViewChange::NewerTemplate
    );
    let mut without_tool = held;
    without_tool.tool = None;
    assert!(without_tool.check_pin(&pin).is_err());
}
