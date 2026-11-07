use std::{collections::BTreeSet, fs, path::Path};

use cortexkit_role_tool_display::{
    canonical_body, check_writer, idempotency_key, not_ready, schema::*, validate_write,
    write_outcome, DisplayKind, DisplayLimits, DisplayWrite, Principal, RecordSchema, WriteReply,
    DISPLAY_WRITER_UNVERIFIED, MAX_COMPLETION_BYTES, MAX_DETAIL_BYTES, MAX_RECORD_BYTES,
    MAX_WRITE_FRAME_BYTES,
};
use serde_json::{json, Map, Value};

fn vector(name: &str) -> Value {
    let path = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("test-vectors/tool-display-v1")
        .join(name);
    serde_json::from_str(&fs::read_to_string(path).unwrap()).unwrap()
}

fn object(value: &Value) -> &Map<String, Value> {
    value.as_object().unwrap()
}

#[test]
fn every_json_vector_is_exercised() {
    let directory = Path::new(env!("CARGO_MANIFEST_DIR")).join("test-vectors/tool-display-v1");
    let actual: BTreeSet<_> = fs::read_dir(directory)
        .unwrap()
        .map(|entry| entry.unwrap().file_name().into_string().unwrap())
        .filter(|name| name.ends_with(".json"))
        .collect();
    let expected = [
        "canonical.json",
        "field-kinds.json",
        "records.json",
        "details.json",
        "writes.json",
        "views.json",
        "view-tools.json",
        "wakes.json",
    ]
    .into_iter()
    .map(str::to_owned)
    .collect();
    assert_eq!(actual, expected);
}

#[test]
fn canonical_vectors_have_independent_exact_expected_bytes() {
    let fixture = vector("canonical.json");
    for input in fixture["inputs"].as_array().unwrap() {
        let body: Map<String, Value> = serde_json::from_str(input.as_str().unwrap()).unwrap();
        assert_eq!(
            canonical_body(&body).unwrap(),
            fixture["canonical"].as_str().unwrap().as_bytes()
        );
    }
}

#[test]
fn field_kind_vectors_validate_every_scalar_kind() {
    let fixture = vector("field-kinds.json");
    let mut kinds = BTreeSet::new();
    for case in fixture.as_array().unwrap() {
        let kind: FieldKind = serde_json::from_value(case["kind"].clone()).unwrap();
        kinds.insert(case["kind"].as_str().unwrap());
        let expected = case["valid"].as_bool().unwrap();
        assert_eq!(kind.validate_scalar(&case["value"]), expected, "{case}");
        assert_eq!(kind.validate(&case["value"]), expected, "{case}");
        assert!(!kind.validate(&Value::Null));
        assert!(!kind.validate(&json!({"field":"value"})));
    }
    assert_eq!(kinds.len(), 9);
}

#[test]
fn record_and_completion_vectors_enforce_flat_declared_fields() {
    let fixture = vector("records.json");
    let schema: RecordSchema = serde_json::from_value(fixture["schema"].clone()).unwrap();
    for case in fixture["cases"].as_array().unwrap() {
        let result = if case["kind"] == "completion" {
            validate_completion(object(&case["body"]), &schema)
        } else {
            validate_record(object(&case["body"]), &schema)
        };
        assert_eq!(
            result.is_ok(),
            case["valid"].as_bool().unwrap(),
            "{}: {result:?}",
            case["name"]
        );
        if let Err(error) = result {
            assert_eq!(error.field, case["field"].as_str().unwrap());
        }
    }
}

#[test]
fn detail_vectors_validate_cut_metadata() {
    for case in vector("details.json").as_array().unwrap() {
        let result = validate_detail(object(&case["body"]));
        assert_eq!(result.is_ok(), case["valid"].as_bool().unwrap(), "{case}");
        if let Err(error) = result {
            assert_eq!(error.field, case["field"].as_str().unwrap());
        }
    }
}

#[test]
fn write_vectors_validate_outcomes_refusals_and_limits() {
    let fixture = vector("writes.json");
    assert_eq!(
        cortexkit_role_tool_display::APP_SERVER_CAPABILITY,
        fixture["capabilities"]["app_reader"]
    );
    assert_eq!(
        cortexkit_role_tool_display::TOOL_DISPLAY_CAPABILITY,
        fixture["capabilities"]["display_storage"]
    );
    assert_ne!(
        cortexkit_role_tool_display::APP_SERVER_CAPABILITY,
        cortexkit_role_tool_display::TOOL_DISPLAY_CAPABILITY
    );
    let limits: DisplayLimits = serde_json::from_value(fixture["limits"].clone()).unwrap();
    limits.validate().unwrap();
    let write: DisplayWrite = serde_json::from_value(fixture["write"].clone()).unwrap();
    validate_write(&write, &limits).unwrap();
    assert_eq!(serde_json::to_value(&write).unwrap(), fixture["write"]);
    for case in fixture["outcomes"].as_array().unwrap() {
        let previous = case["previous"]
            .as_object()
            .map(|body| canonical_body(body).unwrap());
        let incoming = canonical_body(object(&case["incoming"])).unwrap();
        match write_outcome(previous.as_deref(), &incoming) {
            Ok(reply) => {
                assert_eq!(serde_json::to_value(reply).unwrap(), case["reply"]);
                assert_eq!(
                    serde_json::from_value::<WriteReply>(case["reply"].clone()).unwrap(),
                    reply
                );
            }
            Err(error) => assert_eq!(error.code, case["code"]),
        }
    }
    for case in fixture["refusals"].as_array().unwrap() {
        let mut params = fixture["write"].clone();
        let mut limits = limits.clone();
        if let Some(limit) = case["max_record_bytes"].as_u64() {
            limits.max_record_bytes = limit;
        }
        for field in ["kind", "call_key", "schema_pin"] {
            if let Some(value) = case.get(field) {
                params[field] = value.clone();
            }
        }
        let write: DisplayWrite = serde_json::from_value(params).unwrap();
        let error = validate_write(&write, &limits).unwrap_err();
        assert_eq!(error.code, case["code"]);
        assert_eq!(error.detail.unwrap(), case["detail"]);
    }
    assert_eq!(not_ready().code, fixture["not_ready_code"]);
    assert!(subc_protocol::error_codes::is_retryable_route_open(
        &not_ready().code
    ));
}

#[test]
fn copied_views_vectors_validate_protocol_pins_and_record_schemas() {
    let fixture = vector("views.json");
    for pin in fixture["request"]["pins"].as_array().unwrap() {
        subc_protocol::tool_call::validate_schema_pin(pin.as_str().unwrap()).unwrap();
    }
    for (pin, view) in fixture["reply"]["views"].as_object().unwrap() {
        subc_protocol::tool_call::validate_schema_pin(pin).unwrap();
        if view.is_object() {
            let schema: RecordSchema =
                serde_json::from_value(view["record_schema"].clone()).unwrap();
            schema.validate().unwrap();
        } else {
            assert_eq!(view, "unknown");
        }
    }
    for case in fixture["invalid_requests"].as_array().unwrap() {
        assert!(case["pins"].as_array().unwrap().iter().any(|pin| {
            subc_protocol::tool_call::validate_schema_pin(pin.as_str().unwrap()).is_err()
        }));
    }
    for case in fixture["invalid_replies"].as_array().unwrap() {
        for view in case["views"].as_object().unwrap().values() {
            if let Some(schema) = view.get("record_schema") {
                let schema: RecordSchema = serde_json::from_value(schema.clone()).unwrap();
                assert!(schema.validate().is_err());
            }
        }
    }
}

#[test]
fn idempotency_keys_include_only_writer_runner_call_and_kind() {
    let writer = Principal::Reserved {
        module_id: "provider".into(),
    };
    let mut write: DisplayWrite =
        serde_json::from_value(vector("writes.json")["write"].clone()).unwrap();
    let key = idempotency_key(&writer, &write).unwrap();
    assert_eq!(
        key,
        br#"[{"kind":"reserved","module_id":"provider"},{"kind":"reserved","module_id":"runner"},"call:1","record"]"#
    );
    write.schema_pin = "pin:2".into();
    write.body.insert("durationMs".into(), json!(99));
    assert_eq!(idempotency_key(&writer, &write).unwrap(), key);
    // Another module writing for the same call gets its own key, so it can't
    // take the real provider's slot.
    let other = Principal::Reserved {
        module_id: "other".into(),
    };
    assert_ne!(idempotency_key(&other, &write).unwrap(), key);
    write.call_key = "call:2".into();
    assert_ne!(idempotency_key(&writer, &write).unwrap(), key);
    write.call_key = "call:1".into();
    write.runner = Principal::Direct;
    assert_ne!(idempotency_key(&writer, &write).unwrap(), key);
    write.runner = Principal::Reserved {
        module_id: "runner".into(),
    };
    write.kind = DisplayKind::Completion;
    assert_ne!(idempotency_key(&writer, &write).unwrap(), key);
}

#[test]
fn kind_reading_is_open_but_serving_is_closed() {
    let kind: DisplayKind = serde_json::from_value(json!("future")).unwrap();
    assert!(matches!(&kind, DisplayKind::Unknown(value) if value == "future"));
    assert_eq!(serde_json::to_value(kind).unwrap(), json!("future"));
    for body in [Value::Null, json!([]), json!("text")] {
        let mut params = vector("writes.json")["write"].clone();
        params["body"] = body;
        assert!(serde_json::from_value::<DisplayWrite>(params).is_err());
    }
}

#[test]
fn scalar_and_array_boundaries_are_exact() {
    assert!(validate_code(&"a".repeat(64)));
    assert!(!validate_code(&"a".repeat(65)));
    assert!(validate_id(&"a".repeat(256)));
    assert!(!validate_id(&"a".repeat(257)));
    assert!(validate_agent_id(&"a".repeat(256)));
    assert!(!validate_agent_id(&"a".repeat(257)));
    assert!(validate_project_path(&"é".repeat(512)));
    assert!(!validate_project_path(&"é".repeat(513)));
    assert!(validate_app_id(&"a".repeat(255)));
    assert!(!validate_app_id(&"a".repeat(256)));
    assert!(FieldKind::Integer.validate(&json!(vec![1; 64])));
    assert!(!FieldKind::Integer.validate(&json!(vec![1; 65])));
    assert!(!FieldKind::Integer.validate(&json!([])));
    assert!(validate_integer(&json!(i64::MIN)));
    assert!(validate_timestamp(&json!(i64::MAX)));
    assert!(!validate_integer(&json!(1.0)));
}

#[test]
fn standard_fields_cannot_be_redeclared_and_constructors_work() {
    let mut schema = RecordSchema::new("test", 1, [("durationMs".into(), FieldKind::Code)].into());
    assert_eq!(
        schema.validate().unwrap_err().field,
        "record_schema.fields.durationMs"
    );
    schema.fields.clear();
    validate_record(
        object(&json!({"durationMs":1,"outcome":"success"})),
        &schema,
    )
    .unwrap();
    let write = DisplayWrite::new(
        Principal::Direct,
        "call:1",
        DisplayKind::Detail,
        "pin:1",
        Map::new(),
    );
    validate_write(&write, &DisplayLimits::new(0, 2, 0, 0)).unwrap();
}

#[test]
fn served_and_pinned_byte_limits_are_exact_for_each_kind() {
    assert_eq!(MAX_RECORD_BYTES, 16_384);
    assert_eq!(MAX_COMPLETION_BYTES, 16_384);
    assert_eq!(MAX_DETAIL_BYTES, 3_145_728);
    assert_eq!(MAX_WRITE_FRAME_BYTES, 4_194_304);
    let limits = DisplayLimits::new(
        MAX_RECORD_BYTES,
        MAX_DETAIL_BYTES,
        MAX_COMPLETION_BYTES,
        1000,
    );
    for (kind, ceiling) in [
        (DisplayKind::Record, MAX_RECORD_BYTES),
        (DisplayKind::Detail, MAX_DETAIL_BYTES),
        (DisplayKind::Completion, MAX_COMPLETION_BYTES),
    ] {
        let mut write = DisplayWrite::new(
            Principal::Direct,
            "call:1",
            kind,
            "pin:1",
            object(&json!({"x":"a".repeat(ceiling as usize - 8)})).clone(),
        );
        assert_eq!(
            validate_write(&write, &limits).unwrap().len() as u64,
            ceiling
        );
        let frame = json!({"name":"display.write", "arguments": write});
        assert!((serde_json::to_vec(&frame).unwrap().len() as u64 + 21) < MAX_WRITE_FRAME_BYTES);
        write
            .body
            .insert("x".into(), json!("a".repeat(ceiling as usize - 7)));
        assert_eq!(
            validate_write(&write, &limits).unwrap_err().code,
            "display_too_large"
        );
    }
    for field in [
        "max_record_bytes",
        "max_detail_bytes",
        "max_completion_bytes",
    ] {
        let mut value = serde_json::to_value(&limits).unwrap();
        value[field] = json!(value[field].as_u64().unwrap() + 1);
        let invalid: DisplayLimits = serde_json::from_value(value).unwrap();
        assert_eq!(invalid.validate().unwrap_err().field, field);
    }
}

#[test]
fn only_daemon_verified_modules_may_write() {
    assert!(check_writer(&Principal::Reserved {
        module_id: "provider".into()
    })
    .is_ok());
    for writer in [Principal::Direct, Principal::Unverified] {
        let refusal = check_writer(&writer).unwrap_err();
        assert_eq!(refusal.code, DISPLAY_WRITER_UNVERIFIED);
    }
}
