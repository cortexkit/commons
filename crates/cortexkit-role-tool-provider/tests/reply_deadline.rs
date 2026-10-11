use cortexkit_role_tool_provider::{
    call::SchemaPin,
    catalog::{
        check_reply, reply_deadline_ms, schema_digest, CatalogAnswer, CatalogTool, HoldArgument,
        Reply,
    },
};
use serde_json::{json, Value};
use sha2::{Digest, Sha256};

fn vectors() -> Value {
    serde_json::from_str(include_str!(
        "../../../test-vectors/tool-provider-v1/reply-deadline.json"
    ))
    .unwrap()
}

fn assert_deadline(case: &Value, vectors: &Value) {
    let reply: Option<Reply> = serde_json::from_value(case["reply"].clone()).unwrap();
    let fallback = case
        .get("runner_fallback_ms")
        .unwrap_or(&vectors["runner_fallback_ms"]);
    assert_eq!(
        reply_deadline_ms(
            reply.as_ref(),
            &case["arguments"],
            fallback.as_u64().unwrap()
        ),
        case["expected_ms"].as_u64().unwrap(),
        "{}: reply={}, arguments={}",
        case["name"],
        case["reply"],
        case["arguments"]
    );
}

fn is_floor_case(case: &Value) -> bool {
    matches!(
        case["name"].as_str().unwrap(),
        "fallback_floor_max_5000" | "fallback_floor_after_hold" | "fallback_above_max"
    )
}

#[test]
fn reply_deadline_vectors_hold() {
    let vectors = vectors();
    for case in vectors["deadlines"].as_array().unwrap() {
        if !is_floor_case(case) && case["name"] != "string_hold" {
            assert_deadline(case, &vectors);
        }
    }
}

#[test]
fn reply_deadline_fallback_floor_vectors_hold() {
    let vectors = vectors();
    let cases: Vec<_> = vectors["deadlines"]
        .as_array()
        .unwrap()
        .iter()
        .filter(|case| is_floor_case(case))
        .collect();
    assert_eq!(cases.len(), 3);
    for case in cases {
        assert_deadline(case, &vectors);
    }
}

#[test]
fn reply_deadline_string_hold_vector_uses_default() {
    let vectors = vectors();
    let case = vectors["deadlines"]
        .as_array()
        .unwrap()
        .iter()
        .find(|case| case["name"] == "string_hold")
        .unwrap();
    assert_deadline(case, &vectors);
}

fn vector_tool(case: &Value, vectors: &Value) -> CatalogTool {
    let mut tool = CatalogTool::new(
        format!("vector-tool-{}", case["name"].as_str().unwrap()),
        "unused-for-reply-validation",
        1,
        case.get("input_schema")
            .unwrap_or(&vectors["input_schema"])
            .clone(),
    );
    tool.reply = serde_json::from_value(case["reply"].clone()).unwrap();
    tool
}

#[test]
fn reply_validation_vectors_hold_and_name_tool_and_member() {
    let vectors = vectors();
    for case in vectors["valid"].as_array().unwrap() {
        let tool = vector_tool(case, &vectors);
        assert_eq!(check_reply(&tool), Ok(()), "{}", case["name"]);
    }
    for case in vectors["invalid"].as_array().unwrap() {
        let tool = vector_tool(case, &vectors);
        let problem = check_reply(&tool).expect_err(case["name"].as_str().unwrap());
        assert_eq!(problem.tool, tool.name);
        assert_eq!(problem.member, case["member"].as_str().unwrap());
        assert!(problem.to_string().contains(&tool.name));
        assert!(problem.to_string().contains(problem.member));
    }
    for case in vectors["malformed"].as_array().unwrap() {
        let problem = serde_json::from_value::<Reply>(case["reply"].clone())
            .expect_err(case["name"].as_str().unwrap());
        if let Some(member) = case["member"].as_str() {
            assert!(problem.to_string().contains(member), "{problem}");
        }
    }
}

fn existing_catalog() -> Value {
    let vectors: Value = serde_json::from_str(include_str!(
        "../../../test-vectors/tool-provider-v1/catalog-answers.json"
    ))
    .unwrap();
    let mut full = vectors["full"].clone();
    // The vector's unknown extension was already ignored by the old decoder.
    // Compare the known catalog content, not preservation of unknown fields.
    full["tools"][1]
        .as_object_mut()
        .unwrap()
        .remove("future_member");
    full
}

// catalog_digest is provider-defined. These tests use SHA-256 over JCS content
// as one provider's digest scheme, excluding the digest's own output field.
fn catalog_content_digest(mut content: Value) -> Vec<u8> {
    content.as_object_mut().unwrap().remove("catalog_digest");
    Sha256::digest(serde_jcs::to_vec(&content).unwrap()).to_vec()
}

#[test]
fn absent_reply_preserves_existing_catalog_bytes_and_digest() {
    let before = existing_catalog();
    let catalog: CatalogAnswer = serde_json::from_value(before.clone()).unwrap();
    assert!(catalog.tools.iter().all(|tool| tool.reply.is_none()));
    let after = serde_json::to_value(&catalog).unwrap();
    assert_eq!(
        serde_jcs::to_vec(&after).unwrap(),
        serde_jcs::to_vec(&before).unwrap()
    );
    assert_eq!(
        catalog_content_digest(after.clone()),
        catalog_content_digest(before.clone())
    );
    assert_eq!(
        serde_json::to_vec(&after["catalog_digest"]).unwrap(),
        serde_json::to_vec(&before["catalog_digest"]).unwrap()
    );
}

#[test]
fn reply_changes_catalog_digest_but_not_schema_digest_or_pin() {
    let before = existing_catalog();
    let mut catalog: CatalogAnswer = serde_json::from_value(before.clone()).unwrap();
    let pin = |tool: &CatalogTool| {
        SchemaPin::new(
            &tool.name,
            schema_digest(&tool.input_schema).unwrap(),
            tool.semantics,
        )
        .encode()
        .unwrap()
    };
    let before_pin = pin(&catalog.tools[0]);
    let before_schema_digest = schema_digest(&catalog.tools[0].input_schema).unwrap();
    catalog.tools[0] = catalog.tools[0].clone().with_reply(Reply::new(600000));
    check_reply(&catalog.tools[0]).unwrap();
    let after = serde_json::to_value(&catalog).unwrap();
    assert_ne!(
        catalog_content_digest(after),
        catalog_content_digest(before)
    );
    assert_eq!(
        schema_digest(&catalog.tools[0].input_schema).unwrap(),
        before_schema_digest
    );
    assert_eq!(catalog.tools[0].schema_digest, before_schema_digest);
    assert_eq!(pin(&catalog.tools[0]), before_pin);
}

#[test]
fn reply_builders_preserve_wire_members() {
    let reply = Reply::new(1830000);
    assert_eq!(
        serde_json::to_value(&reply).unwrap(),
        json!({"max_ms": 1830000})
    );
    let reply = reply.with_hold_argument(HoldArgument::new("timeoutMs", 1800000, 30000));
    let schema = vectors()["input_schema"].clone();
    let tool =
        CatalogTool::new("hold", schema_digest(&schema).unwrap(), 1, schema).with_reply(reply);
    check_reply(&tool).unwrap();
    assert_eq!(
        serde_json::to_value(&tool).unwrap()["reply"],
        json!({
            "max_ms": 1830000,
            "hold_argument": {"name": "timeoutMs", "default_ms": 1800000, "grace_ms": 30000}
        })
    );
    assert_eq!(
        serde_json::from_value::<CatalogTool>(serde_json::to_value(&tool).unwrap()).unwrap(),
        tool
    );
}
