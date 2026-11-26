use cortexkit_role_routing::declarations::{
    parse_consumer_declarations, ConsumerDeclaration, ConsumerDeclarations, DeclarationError,
    DemandDefaults, RunnerDeclaration,
};
use serde_json::{json, Value};

fn slot(runner: Value) -> Value {
    json!({"runner": runner})
}

fn invalid(value: Value, field: &str) {
    assert_eq!(
        parse_consumer_declarations(&value),
        Err(DeclarationError::InvalidField {
            field: field.into()
        }),
        "{value}"
    );
}

fn round_trip(wire: &str) -> ConsumerDeclarations {
    let parsed = parse_consumer_declarations(&serde_json::from_str(wire).unwrap()).unwrap();
    parsed.validate().unwrap();
    let decoded: ConsumerDeclarations = serde_json::from_str(wire).unwrap();
    assert_eq!(parsed, decoded);
    assert_eq!(serde_json::to_string(&decoded).unwrap(), wire);
    parsed
}

#[test]
fn pure_runner_accepts_empty_declaration_and_defaults() {
    let parsed = round_trip(r#"{"runner":{}}"#);
    assert!(parsed.consumers.is_empty());
    let runner = parsed.runner.unwrap();
    assert!(!runner.follows_account_choice);
    assert!(runner.tool_capabilities.is_empty());
    assert!(runner.starts_sessions());
}

#[test]
fn consumers_and_runner_share_the_slot() {
    let parsed = round_trip(
        r#"{"consumers":[{"name":"example","eq":42}],"runner":{"follows_account_choice":true,"tool_capabilities":["code.read/v1"]}}"#,
    );
    assert_eq!(parsed.consumers[0].defaults.eq, Some(42));
    assert!(parsed.runner.unwrap().follows_account_choice);
}

#[test]
fn consumers_only_slots_keep_legacy_bytes_including_empty_lists() {
    for wire in [
        r#"{"consumers":[]}"#,
        r#"{"consumers":[{"name":"example","eq":42,"tools":"required","tool_capabilities":["code.read/v1"]}]}"#,
        "{}",
    ] {
        round_trip(wire);
    }
}

#[test]
fn slot_constructor_and_builders_validate_without_struct_literals() {
    let slot = ConsumerDeclarations::new()
        .with_consumers(vec![ConsumerDeclaration {
            name: "example".into(),
            defaults: DemandDefaults::default(),
        }])
        .with_runner(RunnerDeclaration::new());
    slot.validate().unwrap();
    assert_eq!(slot.consumers.len(), 1);
    assert!(slot.runner.is_some());
    assert_eq!(
        serde_json::to_string(&ConsumerDeclarations::new()).unwrap(),
        "{}"
    );
}

#[test]
fn absent_account_choice_is_not_serialized_as_false() {
    let parsed = parse_consumer_declarations(&slot(json!({}))).unwrap();
    let encoded = serde_json::to_value(&parsed).unwrap();
    assert!(encoded["runner"].get("follows_account_choice").is_none());
    assert!(!parsed.runner.unwrap().follows_account_choice);
}

#[test]
fn deleted_runner_fields_refuse_the_whole_declaration() {
    for member in ["pairs", "standing_limits", "requires_wire_api"] {
        let raw = json!({"consumers": [{"name": "example"}], "runner": {member: null}});
        let expected = DeclarationError::UnknownMember {
            field: format!("runner.{member}"),
        };
        assert_eq!(parse_consumer_declarations(&raw), Err(expected.clone()));
        let typed: ConsumerDeclarations = serde_json::from_value(raw).unwrap();
        assert_eq!(typed.validate(), Err(expected));
    }
}

#[test]
fn deleted_nested_fields_are_retained_but_ignored() {
    let facts =
        json!({"pairs": false, "explicit_providers": 42, "exclude": "-", "starts_sessions": false});
    let parsed = parse_consumer_declarations(&slot(
        json!({"harnesses": ["alpha"], "harness_limits": {"alpha": facts}}),
    ))
    .unwrap();
    parsed.validate().unwrap();
    let runner = parsed.runner.unwrap();
    assert_eq!(runner.harness_limits.as_ref().unwrap()["alpha"], facts);
    let typed = runner
        .typed_harness_limits_serving(&[cortexkit_role_routing::capabilities::RUNNER_HARNESS_FACTS])
        .unwrap()
        .unwrap();
    for field in ["pairs", "explicit_providers", "exclude"] {
        assert_eq!(typed["alpha"].unknown[field], facts[field]);
    }
    let resolved = runner.limits_for_harness(Some("alpha")).unwrap();
    assert!(resolved.unknown.is_empty());
    assert_eq!(resolved.starts_sessions, Some(false));
}

#[test]
fn unknown_members_are_rejected_at_every_declaration_level() {
    for (value, field) in [
        (json!({"extra": false}), "extra"),
        (
            json!({"consumers": [{"name": "example", "extra": false}]}),
            "consumers[0].extra",
        ),
        (slot(json!({"extra": false})), "runner.extra"),
    ] {
        let expected = DeclarationError::UnknownMember {
            field: field.into(),
        };
        assert_eq!(parse_consumer_declarations(&value), Err(expected.clone()));
        let decoded: ConsumerDeclarations = serde_json::from_value(value).unwrap();
        assert_eq!(decoded.validate(), Err(expected));
    }
}

#[test]
fn duplicate_runner_capabilities_are_rejected_by_path() {
    invalid(
        slot(json!({"tool_capabilities": ["code.read/v1", "code.read/v1"]})),
        "runner.tool_capabilities[1]",
    );
}

#[test]
fn runner_capabilities_require_valid_names_and_array_shape() {
    invalid(
        slot(json!({"tool_capabilities": "code.read/v1"})),
        "runner.tool_capabilities",
    );
    for capability in [
        json!(""),
        json!("bad capability"),
        json!("unknown/v1"),
        json!("Bad:code.read/v1"),
        json!("example:code.read/v01"),
        json!(1),
    ] {
        invalid(
            slot(json!({"tool_capabilities": [capability]})),
            "runner.tool_capabilities[0]",
        );
    }
    assert!(parse_consumer_declarations(&slot(json!({"tool_capabilities": []}))).is_ok());
    assert!(parse_consumer_declarations(&slot(
        json!({"tool_capabilities": ["example:new.capability/v2"]})
    ))
    .is_ok());
}

#[test]
fn runner_tool_capabilities_allow_64_entries_but_not_65() {
    let capabilities: Vec<_> = (0..65)
        .map(|i| format!("example:tool{i}.read/v1"))
        .collect();
    assert!(parse_consumer_declarations(&slot(json!({
        "tool_capabilities": &capabilities[..64]
    })))
    .is_ok());
    invalid(
        slot(json!({"tool_capabilities": capabilities})),
        "runner.tool_capabilities",
    );
}

#[test]
fn optional_runner_members_read_null_as_absent() {
    let parsed = parse_consumer_declarations(&json!({"runner": null})).unwrap();
    assert!(parsed.runner.is_none());
    assert_eq!(serde_json::to_value(parsed).unwrap(), json!({}));
    let parsed = parse_consumer_declarations(&slot(json!({
        "follows_account_choice": null,
        "tool_capabilities": null
    })))
    .unwrap();
    assert_eq!(serde_json::to_value(parsed).unwrap(), slot(json!({})));
}
