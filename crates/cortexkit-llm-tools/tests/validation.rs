use cortexkit_llm_tools::{classify, complete, extract, Usage, ValidationError};
use serde_json::{json, Value};

fn request() -> complete::Request {
    serde_json::from_value(json!({"demand":{"iq":50},"prompt":"Invent a moon.",
        "max_output_tokens":16,"max_cost_usd":0.01}))
    .unwrap()
}

fn result() -> complete::Result {
    serde_json::from_value(
        json!({"text":"Paper Moon","model":{"provider":"example","model":"tiny"},
        "usage":{},"cost_usd":0.001}),
    )
    .unwrap()
}

#[test]
fn result_rejects_both_text_and_json() {
    let mut reply = result();
    reply.json = Some(json!({"invented":"moon"}));
    assert_eq!(
        complete::validate_result(&reply),
        Err(ValidationError::OutputCount)
    );
}

#[test]
fn result_rejects_neither_text_nor_json() {
    let mut reply = result();
    reply.text = None;
    assert_eq!(
        complete::validate_result(&reply),
        Err(ValidationError::OutputCount)
    );
}

#[test]
fn usage_absent_fields_stay_absent() {
    let usage: Usage = serde_json::from_str("{}").unwrap();
    assert_eq!(serde_json::to_value(usage).unwrap(), json!({}));
    let partial: Usage = serde_json::from_value(json!({"output_tokens":7})).unwrap();
    assert_eq!(
        serde_json::to_value(partial).unwrap(),
        json!({"output_tokens":7})
    );
}

#[test]
fn measured_zero_is_not_absence() {
    let usage: Usage = serde_json::from_value(json!({"input_tokens":0,"output_tokens":0})).unwrap();
    assert_eq!(
        serde_json::to_value(usage).unwrap(),
        json!({"input_tokens":0,"output_tokens":0})
    );
}

#[test]
fn reasoning_is_part_of_reported_output() {
    for value in [
        json!({"reasoning_tokens":1}),
        json!({"output_tokens":2,"reasoning_tokens":3}),
    ] {
        let usage: Usage = serde_json::from_value(value).unwrap();
        assert_eq!(usage.validate(), Err(ValidationError::ReasoningNotInOutput));
    }
    let usage: Usage =
        serde_json::from_value(json!({"output_tokens":3,"reasoning_tokens":3})).unwrap();
    assert!(usage.validate().is_ok());
}

#[test]
fn demand_bounds_have_named_failures() {
    for (field, value) in [
        ("demand.iq", -1),
        ("demand.iq", 101),
        ("demand.eq", -1),
        ("demand.eq", 101),
    ] {
        let mut req = request();
        if field == "demand.iq" {
            req.demand.iq = value;
        } else {
            req.demand.eq = Some(value);
        }
        assert_eq!(
            complete::validate_request(&req),
            Err(ValidationError::DemandOutOfRange { field })
        );
    }
    for value in [0, 100] {
        let mut req = request();
        req.demand.iq = value;
        req.demand.eq = Some(value);
        assert!(complete::validate_request(&req).is_ok());
    }
}

#[test]
fn prompt_and_token_limit_are_required_and_nonempty() {
    let mut req = request();
    req.prompt.clear();
    assert_eq!(
        complete::validate_request(&req),
        Err(ValidationError::EmptyPrompt)
    );
    req.prompt = "Invented".into();
    req.max_output_tokens = 0;
    assert_eq!(
        complete::validate_request(&req),
        Err(ValidationError::ZeroMaxOutputTokens)
    );
}

#[test]
fn schema_limit_counts_compact_utf8_bytes() {
    let mut req = request();
    let size = complete::OUTPUT_SCHEMA_MAX_BYTES;
    // The compact object adds eight bytes around the value; é uses two UTF-8 bytes.
    req.output_schema =
        Some(serde_json::from_value(json!({"x":"é".repeat((size - 8) / 2)})).unwrap());
    assert_eq!(
        serde_json::to_vec(req.output_schema.as_ref().unwrap())
            .unwrap()
            .len(),
        size
    );
    assert!(complete::validate_request(&req).is_ok());
    req.output_schema
        .as_mut()
        .unwrap()
        .insert("x".into(), json!("é".repeat((size - 8) / 2) + "a"));
    assert_eq!(
        complete::validate_request(&req),
        Err(ValidationError::OutputSchemaTooLarge {
            size: size + 1,
            limit: size
        })
    );
}

#[test]
fn cost_ceiling_is_required_and_non_negative() {
    let mut req = request();
    req.max_cost_usd = serde_json::from_str("-0.0001").unwrap();
    assert_eq!(
        complete::validate_request(&req),
        Err(ValidationError::NegativeCost {
            field: "max_cost_usd"
        })
    );
    req.max_cost_usd = 0.into();
    assert!(complete::validate_request(&req).is_ok());
    let mut value = serde_json::to_value(&req).unwrap();
    value.as_object_mut().unwrap().remove("max_cost_usd");
    assert!(serde_json::from_value::<complete::Request>(value).is_err());
}

#[test]
fn classify_request_without_max_cost_usd_is_refused() {
    let mut value = json!({"model":"example/classifier",
        "questions":{"q":{"type":"noul","instructions":"Test the invented note."}},
        "item":{"state":"An imaginary moon."}});
    let schema = jsonschema::validator_for(&classify::input_schema()).unwrap();
    assert!(!schema.is_valid(&value));
    assert_eq!(
        serde_json::from_value::<classify::Request>(value.clone())
            .unwrap_err()
            .to_string(),
        "missing field `max_cost_usd`"
    );
    value["max_cost_usd"] = json!(0);
    let mut req: classify::Request = serde_json::from_value(value.clone()).unwrap();
    assert!(schema.is_valid(&value));
    assert!(classify::validate_request(&req).is_ok());
    req.max_cost_usd = (-1).into();
    assert_eq!(
        classify::validate_request(&req),
        Err(ValidationError::NegativeCost {
            field: "max_cost_usd"
        })
    );
}

#[test]
fn extract_request_without_max_cost_usd_is_refused() {
    let mut value = json!({"model":"example/extractor",
        "fields":{"color":{"type":"text"}},"item":{"state":"An imaginary blue moon."}});
    let schema = jsonschema::validator_for(&extract::input_schema()).unwrap();
    assert!(!schema.is_valid(&value));
    assert_eq!(
        serde_json::from_value::<extract::Request>(value.clone())
            .unwrap_err()
            .to_string(),
        "missing field `max_cost_usd`"
    );
    value["max_cost_usd"] = json!(0);
    let mut req: extract::Request = serde_json::from_value(value.clone()).unwrap();
    assert!(schema.is_valid(&value));
    assert!(extract::validate_request(&req).is_ok());
    req.max_cost_usd = (-1).into();
    assert_eq!(
        extract::validate_request(&req),
        Err(ValidationError::NegativeCost {
            field: "max_cost_usd"
        })
    );
}

#[test]
fn every_success_requires_a_cost() {
    let value = json!({"answers":{},"usage":{}});
    assert!(serde_json::from_value::<classify::Result>(value.clone()).is_err());
    assert!(serde_json::from_value::<extract::Result>(value).is_err());
    let mut value = serde_json::to_value(result()).unwrap();
    value.as_object_mut().unwrap().remove("cost_usd");
    assert!(serde_json::from_value::<complete::Result>(value).is_err());
}

#[test]
fn json_null_is_a_present_output() {
    let mut value = serde_json::to_value(result()).unwrap();
    value.as_object_mut().unwrap().remove("text");
    value["json"] = Value::Null;
    let reply: complete::Result = serde_json::from_value(value.clone()).unwrap();
    assert!(complete::validate_result(&reply).is_ok());
    assert_eq!(reply.json, Some(Value::Null));
    assert_eq!(serde_json::to_value(reply).unwrap(), value);
}

#[test]
fn result_branch_must_match_request_schema_presence() {
    let mut req = request();
    let mut reply = result();
    assert!(complete::validate_result_for_request(&req, &reply).is_ok());
    req.output_schema = Some(serde_json::Map::new());
    assert_eq!(
        complete::validate_result_for_request(&req, &reply),
        Err(ValidationError::OutputModeMismatch)
    );
    reply.text = None;
    reply.json = Some(json!("invented"));
    assert!(complete::validate_result_for_request(&req, &reply).is_ok());
    req.output_schema = None;
    assert_eq!(
        complete::validate_result_for_request(&req, &reply),
        Err(ValidationError::OutputModeMismatch)
    );
}

#[test]
fn validators_reuse_role_question_and_calendar_rules() {
    let req: classify::Request = serde_json::from_value(json!({"model":"example/classifier",
        "questions":{"q":{"type":"choice","instructions":"Pick","criteria":{"only":"one"}}},
        "item":{"state":"invented"},"max_cost_usd":0.01}))
    .unwrap();
    assert!(matches!(
        classify::validate_request(&req),
        Err(ValidationError::Role(_))
    ));
    let req: extract::Request = serde_json::from_value(json!({"model":"example/extractor",
        "fields":{"x":{"type":"text"}},"item":{"state":"invented","reference_date":"2028-02-30"},"max_cost_usd":0.01}))
    .unwrap();
    assert!(matches!(
        extract::validate_request(&req),
        Err(ValidationError::Role(_))
    ));
}

#[test]
fn extractor_answers_preserve_number_spelling_and_extra_members() {
    let text = r#"{"answers":{"x":{"type":"number","value":1.2300e+2,"extra":0.0100}},"usage":{},"cost_usd":0}"#;
    let reply: extract::Result = serde_json::from_str(text).unwrap();
    assert_eq!(
        reply.answers["x"].member("value").unwrap().get(),
        "1.2300e+2"
    );
    assert_eq!(reply.answers["x"].member("extra").unwrap().get(), "0.0100");
    assert_eq!(serde_json::to_string(&reply).unwrap(), text);
}

#[test]
fn tool_identity_excludes_direct_agent_mode() {
    assert_eq!(cortexkit_llm_tools::MODES, ["codemode", "flow"]);
    assert_eq!(
        [classify::TAG, extract::TAG, complete::TAG],
        ["llm.classify/v1", "llm.extract/v1", "llm.complete/v1"]
    );
}
