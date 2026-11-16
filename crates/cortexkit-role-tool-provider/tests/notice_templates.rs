use cortexkit_role_tool_provider::{
    catalog::{check_notice_templates_request, CatalogAnswer, CatalogRequest},
    errors::invalid_request_field,
    vectors::CATALOG_NOTICE_TEMPLATES_JSON,
};
use serde_json::{json, Value};

fn answer(templates: Value) -> Value {
    json!({"generation":"g1", "catalog_digest":"d1", "tools":[], "notice_templates":templates})
}

fn refuses(templates: Value, field: &str) {
    let error = serde_json::from_value::<CatalogAnswer>(answer(templates)).unwrap_err();
    assert!(
        error.to_string().starts_with(&format!("{field}:")),
        "{error}"
    );
}

#[test]
fn notice_templates_request_and_answer_are_opt_in() {
    let request = CatalogRequest::new(Default::default());
    assert!(serde_json::to_value(&request)
        .unwrap()
        .get("notice_templates")
        .is_none());
    let request = request.with_notice_templates(true).with_preset("review");
    let decoded: CatalogRequest =
        serde_json::from_value(serde_json::to_value(&request).unwrap()).unwrap();
    assert_eq!(decoded, request);
    assert_eq!(decoded.notice_templates, Some(true));
    let disabled: CatalogRequest =
        serde_json::from_value(json!({"notice_templates":false})).unwrap();
    assert_eq!(disabled.notice_templates, Some(false));
    let answer = CatalogAnswer::new("g1", "d1");
    assert!(serde_json::to_value(&answer)
        .unwrap()
        .get("notice_templates")
        .is_none());
    let answer = answer.with_notice_templates(vec![]);
    let decoded: CatalogAnswer =
        serde_json::from_value(serde_json::to_value(&answer).unwrap()).unwrap();
    assert_eq!(decoded.notice_templates, Some(vec![]));
}

#[test]
fn notice_templates_refuses_too_many() {
    refuses(
        json!((0..17)
            .map(|id| json!({"id":id.to_string()}))
            .collect::<Vec<_>>()),
        "notice_templates",
    );
}

#[test]
fn notice_templates_refuses_non_object() {
    refuses(json!(["not an object"]), "notice_templates[0]");
}

#[test]
fn notice_templates_refuses_invalid_id() {
    for template in [
        json!({}),
        json!({"id":""}),
        json!({"id":null}),
        json!({"id":1}),
    ] {
        refuses(json!([template]), "notice_templates[0].id");
    }
}

#[test]
fn notice_templates_refuses_duplicate_id() {
    refuses(
        json!([{"id":"same"}, {"id":"same"}]),
        "notice_templates[1].id",
    );
}

#[test]
fn notice_templates_refuses_oversized_element() {
    let template = json!({"id":"a", "text":"é".repeat(2048)});
    assert!(serde_json::to_vec(&template).unwrap().len() > 4096);
    refuses(json!([template]), "notice_templates[0]");
}

#[test]
fn notice_templates_accepts_bounds_and_defers_runner_validation() {
    let mut template = json!({"id":"a", "text":""});
    let overhead = serde_json::to_vec(&template).unwrap().len();
    template["text"] = json!("x".repeat(4096 - overhead));
    assert_eq!(serde_json::to_vec(&template).unwrap().len(), 4096);
    let mut templates = (0..15)
        .map(|id| json!({"id":id.to_string()}))
        .collect::<Vec<_>>();
    templates.push(template);
    let decoded: CatalogAnswer = serde_json::from_value(answer(json!(templates))).unwrap();
    assert_eq!(decoded.notice_templates, Some(templates));
}

#[test]
fn notice_templates_refuses_non_list() {
    for invalid in [Value::Null, json!({}), json!(true)] {
        refuses(invalid, "notice_templates");
    }
}

#[test]
fn notice_templates_vectors_decode_and_refuse_named_fields() {
    let vectors: Value = serde_json::from_str(CATALOG_NOTICE_TEMPLATES_JSON).unwrap();
    for case in vectors["requests"].as_array().unwrap() {
        let request: CatalogRequest = serde_json::from_value(case["request"].clone()).unwrap();
        let provides: Vec<String> = serde_json::from_value(case["provides"].clone()).unwrap();
        check_notice_templates_request(&request, &provides).unwrap();
        assert_eq!(serde_json::to_value(request).unwrap(), case["request"]);
    }
    for case in vectors["answers"].as_array().unwrap() {
        let answer: CatalogAnswer = serde_json::from_value(case["answer"].clone()).unwrap();
        assert_eq!(serde_json::to_value(answer).unwrap(), case["answer"]);
    }
    for case in vectors["refused_answers"].as_array().unwrap() {
        let error = serde_json::from_value::<CatalogAnswer>(case["answer"].clone()).unwrap_err();
        assert!(
            error
                .to_string()
                .starts_with(&format!("{}:", case["field"].as_str().unwrap())),
            "{}: {error}",
            case["name"]
        );
    }
    for case in vectors["refused_requests"].as_array().unwrap() {
        let request: CatalogRequest = serde_json::from_value(case["request"].clone()).unwrap();
        let provides: Vec<String> = serde_json::from_value(case["provides"].clone()).unwrap();
        let expected: subc_protocol::ErrorBody =
            serde_json::from_value(case["error"].clone()).unwrap();
        let error = check_notice_templates_request(&request, &provides).unwrap_err();
        assert_eq!(error.code, expected.code);
        assert_eq!(
            invalid_request_field(&error),
            invalid_request_field(&expected)
        );
    }
    check_notice_templates_request(&CatalogRequest::default(), &[]).unwrap();
}
