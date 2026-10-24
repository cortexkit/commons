use cortexkit_role_tool_provider::{
    catalog::{schema_digest, CatalogAnswer, CatalogTool},
    result::{validate, ContentPart, ImageContent, ResultFormat, ToolResult, MAX_RESULT_BYTES},
    vectors,
};
use serde_json::{json, Value};
use sha2::{Digest, Sha256};

fn vectors() -> Value {
    serde_json::from_str(vectors::TOOL_RESULT_V1_JSON).unwrap()
}

fn check_vector(group: &str, name: &str) {
    let vectors = vectors();
    let case = vectors[group]
        .as_array()
        .unwrap()
        .iter()
        .find(|case| case["name"] == name)
        .unwrap();
    let mut body = case["body"].clone();
    if let Some(expansion) = case.get("expand_text") {
        let index = expansion["part"].as_u64().unwrap() as usize;
        let text = body["content"][index]["text"]
            .as_str()
            .unwrap()
            .repeat(expansion["repeat"].as_u64().unwrap() as usize);
        body["content"][index]["text"] = json!(text);
    }
    let bytes = serde_json::to_vec(&body).unwrap();
    let decoded = serde_json::from_slice::<ToolResult>(&bytes);
    if group == "decode_invalid" {
        let error = decoded.expect_err(name).to_string();
        assert!(
            error.contains(case["member"].as_str().unwrap()),
            "{name}: {error}"
        );
        return;
    }
    let result = decoded.unwrap();
    if group == "valid" {
        assert_eq!(validate(&result), Ok(()), "{name}");
        assert_eq!(
            serde_json::to_value(&result).unwrap(),
            body,
            "{name}: order and fields must round-trip"
        );
    } else {
        let problem = validate(&result).expect_err(name);
        assert_eq!(problem.member, case["member"], "{name}");
        assert_eq!(problem.reason, case["reason"], "{name}");
        assert_eq!(
            problem.to_string(),
            format!("{}: {}", problem.member, problem.reason)
        );
    }
}

macro_rules! vector_tests {
    ($list:ident, $group:literal, $($name:ident),+ $(,)?) => {
        const $list: &[&str] = &[$(stringify!($name)),+];
        $(#[test] fn $name() { check_vector($group, stringify!($name)); })+
    };
}

vector_tests!(
    VALID,
    "valid",
    text_only,
    text_two_images_in_order,
    error_with_code,
    structured_content,
    unknown_part,
    images_without_text
);
vector_tests!(
    INVALID,
    "invalid",
    empty_content,
    structured_only,
    error_missing_code,
    code_without_error,
    code_with_false_error,
    code_not_snake_case,
    data_url,
    bad_base64,
    bad_mime,
    zero_width,
    zero_height,
    duplicate_image_id,
    oversize
);
vector_tests!(
    DECODE_INVALID,
    "decode_invalid",
    missing_content,
    structured_not_object,
    missing_image_dimension,
    missing_text,
    missing_type,
    nonstring_type
);

#[test]
fn every_vector_has_a_named_decode_and_validate_test() {
    let vectors = vectors();
    for (group, names) in [
        ("valid", VALID),
        ("invalid", INVALID),
        ("decode_invalid", DECODE_INVALID),
    ] {
        let actual: Vec<_> = vectors[group]
            .as_array()
            .unwrap()
            .iter()
            .map(|case| case["name"].as_str().unwrap())
            .collect();
        assert_eq!(actual, names);
    }
}

#[test]
fn builders_use_mcp_wire_names_and_keep_optional_members_absent() {
    let plain = ToolResult::new(vec![ContentPart::text("verbatim")]);
    assert_eq!(
        serde_json::to_value(&plain).unwrap(),
        json!({
            "content": [{"type": "text", "text": "verbatim"}]
        })
    );
    let image = ImageContent::new("image/png", "AA==", 12, 13)
        .with_alt("Provider wording")
        .with_id("opaque-id");
    let result = ToolResult::new(vec![ContentPart::text("before"), ContentPart::image(image)])
        .with_structured_content(json!({"program": 42}).as_object().unwrap().clone())
        .with_error("image_unavailable");
    let wire = serde_json::to_value(&result).unwrap();
    assert_eq!(
        wire,
        json!({
            "content": [
                {"type": "text", "text": "before"},
                {"type": "image", "mimeType": "image/png", "data": "AA==", "width": 12, "height": 13,
                 "alt": "Provider wording", "id": "opaque-id"}
            ],
            "structuredContent": {"program": 42}, "isError": true, "errorCode": "image_unavailable"
        })
    );
    assert_eq!(validate(&result), Ok(()));
    assert_eq!(serde_json::from_value::<ToolResult>(wire).unwrap(), result);
}

#[test]
fn base64_alphabet_padding_and_trailing_bits_are_checked() {
    for data in ["AA==", "AAA=", "AAAA", "+/8=", "aGVsbG8="] {
        let result = ToolResult::new(vec![ContentPart::image(ImageContent::new(
            "image/png",
            data,
            1,
            1,
        ))]);
        assert_eq!(validate(&result), Ok(()), "{data}");
    }
    for data in [
        "", "A", "AA", "AAA", "A===", "====", "AA=A", "AB==", "AAB=", "AA-_", "AA==\n", "éé",
    ] {
        let result = ToolResult::new(vec![ContentPart::image(ImageContent::new(
            "image/png",
            data,
            1,
            1,
        ))]);
        let problem = validate(&result).expect_err(data);
        assert_eq!(problem.member, "content[0].data");
    }
}

#[test]
fn snake_case_error_codes_are_nonempty_provider_names() {
    for code in ["file_not_found", "error", "http_404", "v2_failed"] {
        assert_eq!(
            validate(&ToolResult::new(vec![ContentPart::text("error")]).with_error(code)),
            Ok(())
        );
    }
    for code in [
        "",
        "_error",
        "error_",
        "two__words",
        "Bad",
        "bad-code",
        "bad code",
        "9error",
        "échec",
    ] {
        let error = validate(&ToolResult::new(vec![ContentPart::text("error")]).with_error(code))
            .expect_err(code);
        assert_eq!(error.member, "errorCode");
    }
}

#[test]
fn serialized_size_is_strict_and_counts_all_fields_and_utf8_bytes() {
    let mut result = ToolResult::new(vec![ContentPart::text("")]);
    let overhead = serde_json::to_vec(&result).unwrap().len();
    result.content[0] = ContentPart::text("x".repeat(MAX_RESULT_BYTES - overhead - 1));
    assert_eq!(
        serde_json::to_vec(&result).unwrap().len(),
        MAX_RESULT_BYTES - 1
    );
    assert_eq!(validate(&result), Ok(()));
    result.content[0] = ContentPart::text("x".repeat(MAX_RESULT_BYTES - overhead));
    assert_eq!(serde_json::to_vec(&result).unwrap().len(), MAX_RESULT_BYTES);
    assert_eq!(validate(&result).unwrap_err().member, "reply");
    let structured = ToolResult::new(vec![ContentPart::text("small")]).with_structured_content(
        json!({"large": "é".repeat(MAX_RESULT_BYTES / 2)})
            .as_object()
            .unwrap()
            .clone(),
    );
    assert_eq!(validate(&structured).unwrap_err().member, "reply");
    let unknown = ToolResult::new(vec![ContentPart::Unknown(
        json!({"type": "future", "large": "x".repeat(MAX_RESULT_BYTES)}),
    )]);
    assert_eq!(validate(&unknown).unwrap_err().member, "reply");
}

#[test]
fn unknown_parts_cannot_bypass_known_part_validation_when_constructed() {
    for value in [
        json!(null),
        json!({"type": 7}),
        json!({"type": "text"}),
        json!({"type": "image"}),
    ] {
        let error = validate(&ToolResult::new(vec![ContentPart::Unknown(value)])).unwrap_err();
        assert_eq!(error.member, "content[0].type");
    }
}

#[test]
fn absent_result_format_preserves_existing_catalog_bytes_and_pinned_digests() {
    let vectors: Value = serde_json::from_str(vectors::CALL_MODES_JSON).unwrap();
    let original: Value = serde_json::from_str(vectors::CATALOG_ANSWERS_JSON).unwrap();
    let mut original = original["full"].clone();
    // Unknown catalog extensions are intentionally not modeled by CatalogTool.
    original["tools"][1]
        .as_object_mut()
        .unwrap()
        .remove("future_member");
    assert_eq!(original, vectors["digests"][0]["catalog"]);
    for case in vectors["digests"].as_array().unwrap() {
        let catalog: CatalogAnswer = serde_json::from_value(case["catalog"].clone()).unwrap();
        assert!(catalog
            .tools
            .iter()
            .all(|tool| tool.result_format.is_none()));
        let encoded = serde_json::to_value(catalog).unwrap();
        assert_eq!(
            serde_json::to_vec(&encoded).unwrap(),
            serde_json::to_vec(&case["catalog"]).unwrap()
        );
        let mut content = encoded;
        content.as_object_mut().unwrap().remove("catalog_digest");
        let hash = Sha256::digest(serde_jcs::to_vec(&content).unwrap());
        let digest: String = hash.iter().map(|byte| format!("{byte:02x}")).collect();
        assert_eq!(digest, case["content_digest"]);
    }
}

#[test]
fn result_formats_are_open_and_do_not_change_the_schema_digest() {
    let schema = json!({"type": "object"});
    let digest = schema_digest(&schema).unwrap();
    let tool = CatalogTool::new("example", digest.clone(), 1, schema)
        .with_result_format(ResultFormat::ToolResultV1);
    let wire = serde_json::to_value(&tool).unwrap();
    assert_eq!(wire["result_format"], "tool-result/v1");
    assert_eq!(schema_digest(&tool.input_schema).unwrap(), digest);
    assert_eq!(serde_json::from_value::<CatalogTool>(wire).unwrap(), tool);
    let unknown = tool.with_result_format(ResultFormat::Unknown("future/v9".into()));
    let wire = serde_json::to_value(&unknown).unwrap();
    let decoded: CatalogTool = serde_json::from_value(wire.clone()).unwrap();
    assert_eq!(
        decoded.result_format,
        Some(ResultFormat::Unknown("future/v9".into()))
    );
    assert_eq!(
        decoded.result_format.as_ref().unwrap().as_str(),
        "future/v9"
    );
    assert_eq!(serde_json::to_value(decoded).unwrap(), wire);
}
