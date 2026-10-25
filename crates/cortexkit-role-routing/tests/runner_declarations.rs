use cortexkit_role_routing::declarations::{
    parse_consumer_declarations, ConsumerDeclaration, ConsumerDeclarations, DeclarationError,
    DemandDefaults, RunnerDeclaration, RunnerPair,
};
use serde_json::{json, Value};

fn pair() -> Value {
    json!({"wire_api": "future.api-v2", "auth_method": "custom_auth"})
}

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
fn pure_runner_accepts_open_set_tokens_and_defaults() {
    let parsed = round_trip(
        r#"{"runner":{"pairs":[{"wire_api":"future.api-v2","auth_method":"custom_auth"}]}}"#,
    );
    assert!(parsed.consumers.is_empty());
    let runner = parsed.runner.unwrap();
    assert!(!runner.follows_account_choice);
    assert!(runner.tool_capabilities.is_empty());
    assert!(runner.standing_limits.is_none());
    assert_eq!(
        runner.pairs[0],
        RunnerPair::new("future.api-v2", "custom_auth")
    );
}

#[test]
fn consumers_and_runner_share_the_slot() {
    let parsed = round_trip(
        r#"{"consumers":[{"name":"example","eq":42}],"runner":{"pairs":[{"wire_api":"chat","auth_method":"api-key"}],"follows_account_choice":true,"tool_capabilities":["code.read/v1"],"standing_limits":{"explicit_providers":["example-provider"]}}}"#,
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
        .with_runner(RunnerDeclaration::new(vec![RunnerPair::new("chat", "key")]));
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
    let parsed = parse_consumer_declarations(&slot(json!({"pairs": [pair()]}))).unwrap();
    let encoded = serde_json::to_value(&parsed).unwrap();
    assert!(encoded["runner"].get("follows_account_choice").is_none());
    assert!(!parsed.runner.unwrap().follows_account_choice);
}

#[test]
fn runner_pair_shapes_are_rejected_by_path() {
    for (runner, field) in [
        (json!([]), "runner"),
        (json!({}), "runner.pairs"),
        (json!({"pairs": null}), "runner.pairs"),
        (json!({"pairs": []}), "runner.pairs"),
        (json!({"pairs": [null]}), "runner.pairs[0]"),
        (json!({"pairs": [{}]}), "runner.pairs[0].wire_api"),
        (
            json!({"pairs": [{"wire_api": "chat"}]}),
            "runner.pairs[0].auth_method",
        ),
        (
            json!({"pairs": [pair()], "follows_account_choice": "true"}),
            "runner.follows_account_choice",
        ),
    ] {
        invalid(slot(runner), field);
    }
}

#[test]
fn duplicate_runner_pairs_are_rejected_by_path() {
    let value = slot(json!({"pairs": [pair(), pair()]}));
    invalid(value.clone(), "runner.pairs[1]");
    let decoded: ConsumerDeclarations = serde_json::from_value(value).unwrap();
    assert_eq!(
        decoded.validate(),
        Err(DeclarationError::InvalidField {
            field: "runner.pairs[1]".into()
        })
    );
    assert!(parse_consumer_declarations(&slot(json!({"pairs": [
        {"wire_api": "chat", "auth_method": "key"},
        {"wire_api": "chat", "auth_method": "oauth"},
        {"wire_api": "responses", "auth_method": "key"}
    ]})))
    .is_ok());
}

#[test]
fn runner_pair_tokens_are_validated_by_path() {
    for token in [
        json!(""),
        json!("1chat"),
        json!("Chat"),
        json!("chat/v1"),
        json!("é"),
        json!("a".repeat(65)),
        json!(1),
    ] {
        for field in ["wire_api", "auth_method"] {
            let mut value = pair();
            value[field] = token.clone();
            invalid(
                slot(json!({"pairs": [value]})),
                &format!("runner.pairs[0].{field}"),
            );
        }
    }
    assert!(parse_consumer_declarations(&slot(json!({"pairs": [{
        "wire_api": "a".repeat(64), "auth_method": "z0._-"
    }]})))
    .is_ok());
}

#[test]
fn unknown_members_are_rejected_at_every_declaration_level() {
    for (value, field) in [
        (json!({"extra": false}), "extra"),
        (
            json!({"consumers": [{"name": "example", "extra": false}]}),
            "consumers[0].extra",
        ),
        (
            slot(json!({"pairs": [pair()], "extra": false})),
            "runner.extra",
        ),
        (
            slot(json!({"pairs": [{"wire_api": "chat", "auth_method": "key", "extra": false}]})),
            "runner.pairs[0].extra",
        ),
        (
            slot(json!({"pairs": [pair()], "standing_limits": {"extra": false}})),
            "runner.standing_limits.extra",
        ),
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
        slot(json!({"pairs": [pair()], "tool_capabilities": ["code.read/v1", "code.read/v1"]})),
        "runner.tool_capabilities[1]",
    );
}

#[test]
fn runner_capabilities_require_valid_names_and_array_shape() {
    invalid(
        slot(json!({"pairs": [pair()], "tool_capabilities": "code.read/v1"})),
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
            slot(json!({"pairs": [pair()], "tool_capabilities": [capability]})),
            "runner.tool_capabilities[0]",
        );
    }
    assert!(parse_consumer_declarations(&slot(
        json!({"pairs": [pair()], "tool_capabilities": []})
    ))
    .is_ok());
    assert!(parse_consumer_declarations(&slot(
        json!({"pairs": [pair()], "tool_capabilities": ["example:new.capability/v2"]})
    ))
    .is_ok());
}

#[test]
fn standing_limits_preserve_empty_and_explicit_provider_objects() {
    for wire in [
        r#"{"runner":{"pairs":[{"wire_api":"chat","auth_method":"key"}],"standing_limits":{}}}"#,
        r#"{"runner":{"pairs":[{"wire_api":"chat","auth_method":"key"}],"standing_limits":{"explicit_providers":["provider.one","provider-two"]}}}"#,
    ] {
        round_trip(wire);
    }
    let parsed =
        parse_consumer_declarations(&slot(json!({"pairs": [pair()], "standing_limits": {}})))
            .unwrap();
    assert!(parsed
        .runner
        .unwrap()
        .standing_limits
        .unwrap()
        .explicit_providers
        .is_none());
}

#[test]
fn standing_limit_provider_refusals_name_the_field() {
    invalid(
        slot(json!({"pairs": [pair()], "standing_limits": []})),
        "runner.standing_limits",
    );
    for (providers, field) in [
        (json!([]), "runner.standing_limits.explicit_providers"),
        (
            json!("provider"),
            "runner.standing_limits.explicit_providers",
        ),
        (
            json!(["provider", "provider"]),
            "runner.standing_limits.explicit_providers[1]",
        ),
        (
            json!(["Bad"]),
            "runner.standing_limits.explicit_providers[0]",
        ),
        (json!([""]), "runner.standing_limits.explicit_providers[0]"),
        (
            json!(["a".repeat(65)]),
            "runner.standing_limits.explicit_providers[0]",
        ),
        (json!([1]), "runner.standing_limits.explicit_providers[0]"),
    ] {
        invalid(
            slot(json!({"pairs": [pair()], "standing_limits": {"explicit_providers": providers}})),
            field,
        );
    }
}

#[test]
fn runner_lists_allow_64_entries_but_not_65() {
    let pairs: Vec<_> = (0..65)
        .map(|i| json!({"wire_api": format!("api{i}"), "auth_method": "key"}))
        .collect();
    let capabilities: Vec<_> = (0..65)
        .map(|i| format!("example:tool{i}.read/v1"))
        .collect();
    let providers: Vec<_> = (0..65).map(|i| format!("provider{i}")).collect();
    assert!(parse_consumer_declarations(&slot(json!({
        "pairs": &pairs[..64],
        "tool_capabilities": &capabilities[..64],
        "standing_limits": {"explicit_providers": &providers[..64]}
    })))
    .is_ok());
    invalid(slot(json!({"pairs": pairs})), "runner.pairs");
    invalid(
        slot(json!({"pairs": [pair()], "tool_capabilities": capabilities})),
        "runner.tool_capabilities",
    );
    invalid(
        slot(json!({"pairs": [pair()], "standing_limits": {"explicit_providers": providers}})),
        "runner.standing_limits.explicit_providers",
    );
}

#[test]
fn optional_runner_members_read_null_as_absent() {
    let parsed = parse_consumer_declarations(&json!({"runner": null})).unwrap();
    assert!(parsed.runner.is_none());
    assert_eq!(serde_json::to_value(parsed).unwrap(), json!({}));
    let parsed = parse_consumer_declarations(&slot(json!({
        "pairs": [pair()],
        "follows_account_choice": null,
        "tool_capabilities": null,
        "standing_limits": null
    })))
    .unwrap();
    assert_eq!(
        serde_json::to_value(parsed).unwrap(),
        slot(json!({"pairs": [pair()]}))
    );
    let parsed = parse_consumer_declarations(&slot(json!({
        "pairs": [pair()], "standing_limits": {"explicit_providers": null}
    })))
    .unwrap();
    assert_eq!(
        serde_json::to_value(parsed).unwrap(),
        slot(json!({"pairs": [pair()], "standing_limits": {}}))
    );
}
