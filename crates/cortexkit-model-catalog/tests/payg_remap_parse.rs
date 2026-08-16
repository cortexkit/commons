use cortexkit_model_catalog::{
    is_all_zero, CostSchedule, PaygModelId, PaygProviderRuleKind, PaygRemapDoc, PaygRemapEntry,
    PaygRemapParseError,
};

#[test]
fn all_none_schedule_is_unpriced_not_zero() {
    assert!(!is_all_zero(&CostSchedule::default()));
}

#[test]
fn all_zero_schedule_is_zero() {
    let schedule = CostSchedule {
        input: Some(0),
        output: Some(0),
        ..CostSchedule::default()
    };

    assert!(is_all_zero(&schedule));
}

#[test]
fn parses_minimal_schema_one_document() {
    let doc = PaygRemapDoc::parse(
        r#"{
            "schema": 1,
            "counterfactual": "same_platform_list",
            "providers": {},
            "entries": {
                "plan/model": {
                    "kind": "not_sold_per_token",
                    "reason": "plan_only_no_published_rate",
                    "source": "https://vendor.example/pricing",
                    "observed": "2026-08-13"
                }
            }
        }"#,
    )
    .unwrap();

    let id = PaygModelId::parse("plan/model").unwrap();
    assert_eq!(id.provider(), "plan");
    assert_eq!(id.model(), "model");
    assert!(matches!(
        doc.entries.get(&id),
        Some(PaygRemapEntry::NotSoldPerToken(_))
    ));
    assert!(doc.providers.is_empty());
}

#[test]
fn rejects_document_level_parse_guards() {
    let cases = [
        (
            r#"{ "schema": 2, "counterfactual": "same_platform_list", "providers": {}, "entries": {} }"#,
            "unknown schema",
        ),
        (
            r#"{ "schema": 1, "counterfactual": "different", "providers": {}, "entries": {} }"#,
            "counterfactual mismatch",
        ),
        (
            r#"{ "schema": 1, "counterfactual": "same_platform_list", "providers": {}, "entries": { "model": { "kind": "not_sold_per_token", "reason": "plan", "source": "https://vendor.example", "observed": "2026-08-13" } } }"#,
            "malformed id",
        ),
    ];

    for (json, name) in cases {
        let error = PaygRemapDoc::parse(json).unwrap_err();
        match name {
            "unknown schema" => assert!(matches!(
                error,
                PaygRemapParseError::UnknownSchema { schema: 2 }
            )),
            "counterfactual mismatch" => assert!(matches!(
                error,
                PaygRemapParseError::CounterfactualMismatch { .. }
            )),
            "malformed id" => assert!(matches!(error, PaygRemapParseError::MalformedId { .. })),
            _ => unreachable!(),
        }
    }
}

#[test]
fn parses_provider_rule_and_resolve_target_without_lookup_fallback() {
    let doc = PaygRemapDoc::parse(
        r#"{
            "schema": 1,
            "counterfactual": "same_platform_list",
            "providers": {
                "google": {
                    "kind": "zeros_are_not_prices",
                    "id_prefix": "antigravity-",
                    "source": "https://vendor.example/rules",
                    "observed": "2026-08-13"
                }
            },
            "entries": {
                "reseller/model": {
                    "kind": "resolves_to",
                    "target": "origin/model",
                    "because": "origin api rate",
                    "source": "https://vendor.example/pricing",
                    "observed": "2026-08-13"
                }
            }
        }"#,
    )
    .unwrap();

    assert!(matches!(
        doc.providers.get("google").map(|rule| &rule.kind),
        Some(PaygProviderRuleKind::ZerosAreNotPrices)
    ));
    let reseller = PaygModelId::parse("reseller/model").unwrap();
    let origin = PaygModelId::parse("origin/model").unwrap();
    assert_ne!(reseller, origin);
    let nested = PaygModelId::parse("provider/path/with/slashes").unwrap();
    assert_eq!(nested.provider(), "provider");
    assert_eq!(nested.model(), "path/with/slashes");
    match doc.entries.get(&reseller).unwrap() {
        PaygRemapEntry::ResolvesTo(entry) => assert_eq!(entry.target, origin),
        other => panic!("expected resolves_to, got {other:?}"),
    }
}

#[test]
fn rejects_malformed_provider_qualified_ids() {
    for id in ["model", "/model", "provider/"] {
        assert!(matches!(
            PaygModelId::parse(id),
            Err(PaygRemapParseError::MalformedId { .. })
        ));
    }
}

#[test]
fn rejects_invalid_entries_and_zero_overrides() {
    let cases = [
        (
            r#"{ "schema": 1, "counterfactual": "same_platform_list", "providers": {}, "entries": { "p/m": { "kind": "unknown", "source": "https://vendor.example", "observed": "2026-08-13" } } }"#,
            "unknown kind",
        ),
        (
            r#"{ "schema": 1, "counterfactual": "same_platform_list", "providers": {}, "entries": { "p/m": { "kind": "not_sold_per_token", "reason": "plan", "observed": "2026-08-13" } } }"#,
            "missing provenance",
        ),
        (
            r#"{ "schema": 1, "counterfactual": "same_platform_list", "providers": {}, "entries": { "p/m": { "kind": "resolves_to", "target": "p/m", "because": "self", "source": "https://vendor.example", "observed": "2026-08-13" } } }"#,
            "self target",
        ),
        (
            r#"{ "schema": 1, "counterfactual": "same_platform_list", "providers": {}, "entries": { "p/m": { "kind": "overrides_unpriced", "cost": { "input": 0 }, "source": "https://vendor.example", "observed": "2026-08-13" } } }"#,
            "zero override",
        ),
    ];

    for (json, name) in cases {
        let error = PaygRemapDoc::parse(json).unwrap_err();
        match name {
            "unknown kind" => assert!(matches!(error, PaygRemapParseError::UnknownKind { .. })),
            "missing provenance" => assert!(matches!(
                error,
                PaygRemapParseError::MissingProvenance { .. }
            )),
            "self target" => assert!(matches!(
                error,
                PaygRemapParseError::SelfReferentialTarget { .. }
            )),
            "zero override" => assert!(matches!(error, PaygRemapParseError::ZeroOverride { .. })),
            _ => unreachable!(),
        }
    }
}

#[test]
fn rejects_all_remaining_parse_guards() {
    let cases = [
        (
            r#"{ "schema": 1, "counterfactual": "same_platform_list", "providers": { "p": { "kind": "zeros_are_not_prices", "observed": "2026-08-13" } }, "entries": {} }"#,
            "provider provenance",
        ),
        (
            r#"{ "schema": 1, "counterfactual": "same_platform_list", "providers": { "p": { "kind": "unknown", "source": "https://vendor.example", "observed": "2026-08-13" } }, "entries": {} }"#,
            "provider kind",
        ),
        (
            r#"{ "schema": 1, "counterfactual": "same_platform_list", "providers": {}, "entries": { "p/a": { "kind": "resolves_to", "target": "p/b", "because": "a", "source": "https://vendor.example", "observed": "2026-08-13" }, "p/b": { "kind": "not_sold_per_token", "reason": "plan", "source": "https://vendor.example", "observed": "2026-08-13" } } }"#,
            "chained target",
        ),
        (
            r#"{ "schema": 1, "counterfactual": "same_platform_list", "providers": {}, "entries": { "p/m": { "kind": "overrides_unpriced", "cost": { "input": 1e-10 }, "source": "https://vendor.example", "observed": "2026-08-13" } } }"#,
            "inexact rate",
        ),
        (
            r#"{ "schema": 1, "counterfactual": "same_platform_list", "providers": {}, "entries": { "p/m": { "kind": "overrides_unpriced", "cost": { "output": -1 }, "source": "https://vendor.example", "observed": "2026-08-13" } } }"#,
            "negative rate",
        ),
    ];

    for (json, name) in cases {
        let error = PaygRemapDoc::parse(json).unwrap_err();
        match name {
            "provider provenance" => assert!(matches!(
                error,
                PaygRemapParseError::MissingProvenance { ref id, field: "source" } if id == "p"
            )),
            "provider kind" => assert!(matches!(error, PaygRemapParseError::UnknownKind { .. })),
            "chained target" => assert!(matches!(error, PaygRemapParseError::ChainedTarget { .. })),
            "inexact rate" => assert!(matches!(error, PaygRemapParseError::InexactRate { .. })),
            "negative rate" => assert!(matches!(error, PaygRemapParseError::NegativeRate { .. })),
            _ => unreachable!(),
        }
    }
}

#[test]
fn parses_nonzero_override_schedule_and_rejects_unrepresentable_rate_blocks() {
    let doc = PaygRemapDoc::parse(
        r#"{
            "schema": 1,
            "counterfactual": "same_platform_list",
            "providers": {},
            "entries": {
                "plan/model": {
                    "kind": "overrides_unpriced",
                    "cost": {
                        "input": 0.5,
                        "output": 3,
                        "reasoning": 1,
                        "tiers": [
                            { "tier": { "type": "context", "size": 256000 }, "input": 2, "output": 6 },
                            { "tier": { "type": "context", "size": 128000 }, "input": 1, "output": 4 }
                        ]
                    },
                    "source": "https://vendor.example/pricing",
                    "observed": "2026-08-13"
                }
            }
        }"#,
    )
    .unwrap();
    let id = PaygModelId::parse("plan/model").unwrap();
    match doc.entries.get(&id).unwrap() {
        PaygRemapEntry::OverridesUnpriced(entry) => {
            assert_eq!(entry.cost.input, Some(500_000_000));
            assert_eq!(entry.cost.output, Some(3_000_000_000));
            assert_eq!(entry.cost.reasoning, Some(1_000_000_000));
            assert_eq!(entry.cost.tiers[0].min_context, 128_000);
            assert_eq!(entry.cost.tiers[1].min_context, 256_000);
        }
        other => panic!("expected overrides_unpriced, got {other:?}"),
    }

    let error = PaygRemapDoc::parse(
        r#"{ "schema": 1, "counterfactual": "same_platform_list", "providers": {}, "entries": { "p/m": { "kind": "overrides_unpriced", "cost": { "input": 1, "context_over_200k": { "input": 2 } }, "source": "https://vendor.example", "observed": "2026-08-13" } } }"#,
    )
    .unwrap_err();
    assert!(matches!(
        error,
        PaygRemapParseError::ContextBandNotRepresentable { ref id } if id == "p/m"
    ));
}
