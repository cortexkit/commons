//! Execute the frozen PAYG remap parse-gate fixture against the public parser.
//!
//! `tests/golden/payg-parse-vectors.json` names the invalid documents each guard must
//! refuse. Change it only with a format-contract change and fresh mutation evidence:
//! a passing suite alone does not prove a parser guard remains load-bearing.

use cortexkit_model_catalog::{PaygModelId, PaygRemapDoc, PaygRemapEntry, PaygRemapParseError};
use serde::Deserialize;

const VECTORS: &str = include_str!("golden/payg-parse-vectors.json");

#[derive(Debug, Deserialize)]
struct VectorFile {
    vectors: Vec<Vector>,
}

#[derive(Debug, Deserialize)]
struct PositiveVectorFile {
    #[serde(default)]
    positive_vectors: Vec<PositiveVector>,
}

#[derive(Debug, Deserialize)]
struct Vector {
    name: String,
    input_json: String,
    expect_error: ExpectedError,
}

#[derive(Debug, Deserialize)]
struct PositiveVector {
    name: String,
    input_json: String,
    id: String,
    entry_kind: String,
    provider: Option<String>,
    provider_kind: Option<String>,
    source: String,
    observed: String,
    effective_from: Option<String>,
}

#[derive(Debug, Deserialize)]
struct RawPositiveVectorFile {
    #[serde(default)]
    positive_vectors: Vec<RawPositiveVector>,
}

#[derive(Debug, Deserialize)]
struct RawPositiveVector {
    name: String,
    entry_kind: Option<String>,
    source: Option<String>,
    observed: Option<String>,
}

#[derive(Debug, Deserialize)]
#[serde(tag = "variant")]
enum ExpectedError {
    UnknownSchema {
        schema: u64,
    },
    CounterfactualMismatch {
        expected: String,
        found: String,
    },
    UnknownKind {
        id: String,
        kind: String,
    },
    MalformedId {
        id: String,
    },
    MissingProvenance {
        id: String,
        field: String,
    },
    SelfReferentialTarget {
        id: String,
    },
    ChainedTarget {
        id: String,
        target: String,
    },
    ZeroOverride {
        id: String,
    },
    InexactRate {
        id: String,
        field: String,
        value: String,
    },
    NegativeRate {
        id: String,
        field: String,
        value: String,
    },
    ContextBandNotRepresentable {
        id: String,
        message: String,
    },
    InvalidIdPrefix {
        id: String,
    },
    InvalidEffectiveFrom {
        id: String,
        value: String,
    },
    UnexpectedField {
        id: String,
        field: String,
    },
    DuplicateEntry {
        id: String,
    },
    DuplicateIdPrefix {
        id_prefix: String,
    },
}

#[test]
fn parse_gate_rejects_every_golden_vector_with_its_exact_error() {
    let file: VectorFile = serde_json::from_str(VECTORS).expect("parse PAYG parse vectors");
    assert_eq!(
        file.vectors.len(),
        34,
        "one vector for each non-structural parse guard"
    );

    for vector in file.vectors {
        let error = match PaygRemapDoc::parse(&vector.input_json) {
            Ok(doc) => panic!("{} unexpectedly parsed: {doc:?}", vector.name),
            Err(error) => error,
        };
        assert_expected_error(&vector.name, error, vector.expect_error);
    }
}

#[test]
fn parse_gate_accepts_every_positive_golden_vector() {
    let file: PositiveVectorFile = serde_json::from_str(VECTORS).expect("parse PAYG parse vectors");

    assert_eq!(
        file.positive_vectors.len(),
        3,
        "one unset optional field plus entry and provider time-banded declarations"
    );
    for vector in file.positive_vectors {
        let doc = PaygRemapDoc::parse(&vector.input_json)
            .unwrap_or_else(|error| panic!("{} unexpectedly refused: {error}", vector.name));
        let id = PaygModelId::parse(&vector.id).expect("golden vector must use a valid id");
        match (vector.entry_kind.as_str(), &doc.entries[&id]) {
            ("not_sold_per_token", PaygRemapEntry::NotSoldPerToken(entry)) => {
                assert_eq!(entry.source, vector.source, "{}: source", vector.name);
                assert_eq!(entry.observed, vector.observed, "{}: observed", vector.name);
                assert_eq!(
                    entry.effective_from, vector.effective_from,
                    "{}: effective_from",
                    vector.name
                );
            }
            ("rate_time_banded", PaygRemapEntry::RateTimeBanded(entry)) => {
                assert_eq!(entry.source, vector.source, "{}: source", vector.name);
                assert_eq!(entry.observed, vector.observed, "{}: observed", vector.name);
                assert_eq!(
                    entry.effective_from, vector.effective_from,
                    "{}: effective_from",
                    vector.name
                );
            }
            (kind, entry) => panic!("{} must parse a {kind} entry, got {entry:?}", vector.name),
        }

        match (vector.provider.as_deref(), vector.provider_kind.as_deref()) {
            (Some(provider), Some(expected_kind)) => {
                let rule = &doc.providers[provider];
                match expected_kind {
                    "rate_time_banded" => assert!(matches!(
                        rule.kind,
                        cortexkit_model_catalog::PaygProviderRuleKind::RateTimeBanded
                    )),
                    kind => panic!("{} has unknown provider kind {kind}", vector.name),
                }
                assert_eq!(rule.source, vector.source, "{}: source", vector.name);
                assert_eq!(rule.observed, vector.observed, "{}: observed", vector.name);
                assert_eq!(
                    rule.effective_from, vector.effective_from,
                    "{}: effective_from",
                    vector.name
                );
            }
            (None, None) => {}
            _ => panic!(
                "{} must declare both provider and provider_kind",
                vector.name
            ),
        }
    }
}

#[test]
fn positive_vectors_must_declare_expected_fields() {
    let file: RawPositiveVectorFile =
        serde_json::from_str(VECTORS).expect("parse raw PAYG parse vectors");

    for vector in file.positive_vectors {
        assert!(
            vector.entry_kind.is_some(),
            "{} must declare entry_kind",
            vector.name
        );
        assert!(
            vector.source.is_some(),
            "{} must declare source",
            vector.name
        );
        assert!(
            vector.observed.is_some(),
            "{} must declare observed",
            vector.name
        );
    }
}

#[test]
fn malformed_effective_from_is_refused() {
    let error = PaygRemapDoc::parse(
        r#"{
            "schema": 1,
            "counterfactual": "same_platform_list",
            "providers": {},
            "entries": {
                "p/m": {
                    "kind": "not_sold_per_token",
                    "reason": "plan_only",
                    "source": "https://vendor.example/pricing",
                    "observed": "2026-08-13",
                    "effective_from": "2026-8-13"
                }
            }
        }"#,
    )
    .expect_err("an effective date outside YYYY-MM-DD must be refused");

    assert_eq!(
        error.to_string(),
        "PAYG remap declaration p/m has invalid effective_from \"2026-8-13\""
    );
}

#[test]
fn entry_effective_from_round_trips() {
    let doc = PaygRemapDoc::parse(
        r#"{
            "schema": 1,
            "counterfactual": "same_platform_list",
            "providers": {
                "p": {
                    "kind": "zeros_are_not_prices",
                    "source": "https://vendor.example/pricing",
                    "observed": "2026-08-13",
                    "effective_from": "2026-09-01"
                }
            },
            "entries": {
                "p/with-effective-date": {
                    "kind": "not_sold_per_token",
                    "reason": "plan_only",
                    "source": "https://vendor.example/pricing",
                    "observed": "2026-08-13",
                    "effective_from": "2026-09-01"
                }
            }
        }"#,
    )
    .expect("a shaped effective date must parse");

    assert_eq!(
        doc.providers["p"].effective_from.as_deref(),
        Some("2026-09-01")
    );
    let id = PaygModelId::parse("p/with-effective-date").expect("valid test id");
    let PaygRemapEntry::NotSoldPerToken(entry) = &doc.entries[&id] else {
        panic!("expected not_sold_per_token entry");
    };
    assert_eq!(entry.effective_from.as_deref(), Some("2026-09-01"));
}

#[test]
fn entry_without_effective_from_stays_absent() {
    let doc = PaygRemapDoc::parse(
        r#"{
            "schema": 1,
            "counterfactual": "same_platform_list",
            "providers": {},
            "entries": {
                "p/without-effective-date": {
                    "kind": "not_sold_per_token",
                    "reason": "plan_only",
                    "source": "https://vendor.example/pricing",
                    "observed": "2026-08-13"
                }
            }
        }"#,
    )
    .expect("an entry without effective_from must remain valid");

    let id = PaygModelId::parse("p/without-effective-date").expect("valid test id");
    let PaygRemapEntry::NotSoldPerToken(entry) = &doc.entries[&id] else {
        panic!("expected not_sold_per_token entry");
    };
    assert_eq!(entry.observed, "2026-08-13");
    assert_eq!(entry.effective_from, None);
}

#[test]
fn override_with_a_real_rate_beside_zero_is_not_all_zero() {
    let doc = PaygRemapDoc::parse(
        r#"{
            "schema": 1,
            "counterfactual": "same_platform_list",
            "providers": {},
            "entries": {
                "p/m": {
                    "kind": "overrides_unpriced",
                    "cost": { "input": 0, "output": 0, "reasoning": 1 },
                    "source": "https://vendor.example/pricing",
                    "observed": "2026-08-13"
                }
            }
        }"#,
    )
    .expect("a mixed override has a real rate and must parse");

    assert_eq!(doc.entries.len(), 1);
}

// `payg_remap_parse::parses_provider_rule_and_resolve_target_without_lookup_fallback`
// owns the target-operand mutation: its terminal target is absent from entries, so
// `entries.contains_key(id)` must not turn a valid declaration into ChainedTarget.

fn assert_expected_error(name: &str, error: PaygRemapParseError, expected: ExpectedError) {
    match (error, expected) {
        (
            PaygRemapParseError::UnknownSchema { schema: actual },
            ExpectedError::UnknownSchema { schema: expected },
        ) => assert_eq!(actual, expected, "{name}: UnknownSchema.schema"),
        (
            PaygRemapParseError::CounterfactualMismatch {
                expected: actual_expected,
                found: actual_found,
            },
            ExpectedError::CounterfactualMismatch { expected, found },
        ) => {
            assert_eq!(
                actual_expected, expected,
                "{name}: CounterfactualMismatch.expected"
            );
            assert_eq!(actual_found, found, "{name}: CounterfactualMismatch.found");
        }
        (
            PaygRemapParseError::UnknownKind {
                id: actual_id,
                kind: actual_kind,
            },
            ExpectedError::UnknownKind { id, kind },
        ) => {
            assert_eq!(actual_id, id, "{name}: UnknownKind.id");
            assert_eq!(actual_kind, kind, "{name}: UnknownKind.kind");
        }
        (
            PaygRemapParseError::MalformedId { id: actual },
            ExpectedError::MalformedId { id: expected },
        ) => {
            assert_eq!(actual, expected, "{name}: MalformedId.id");
        }
        (
            PaygRemapParseError::MissingProvenance {
                id: actual_id,
                field: actual_field,
            },
            ExpectedError::MissingProvenance { id, field },
        ) => {
            assert_eq!(actual_id, id, "{name}: MissingProvenance.id");
            assert_eq!(actual_field, field, "{name}: MissingProvenance.field");
        }
        (
            PaygRemapParseError::SelfReferentialTarget { id: actual },
            ExpectedError::SelfReferentialTarget { id: expected },
        ) => assert_eq!(actual, expected, "{name}: SelfReferentialTarget.id"),
        (
            PaygRemapParseError::ChainedTarget {
                id: actual_id,
                target: actual_target,
            },
            ExpectedError::ChainedTarget { id, target },
        ) => {
            assert_eq!(actual_id, id, "{name}: ChainedTarget.id");
            assert_eq!(actual_target, target, "{name}: ChainedTarget.target");
        }
        (
            PaygRemapParseError::ZeroOverride { id: actual },
            ExpectedError::ZeroOverride { id: expected },
        ) => {
            assert_eq!(actual, expected, "{name}: ZeroOverride.id");
        }
        (
            PaygRemapParseError::InexactRate {
                id: actual_id,
                field: actual_field,
                value: actual_value,
            },
            ExpectedError::InexactRate { id, field, value },
        ) => {
            assert_eq!(actual_id, id, "{name}: InexactRate.id");
            assert_eq!(actual_field, field, "{name}: InexactRate.field");
            assert_eq!(actual_value, value, "{name}: InexactRate.value");
        }
        (
            PaygRemapParseError::NegativeRate {
                id: actual_id,
                field: actual_field,
                value: actual_value,
            },
            ExpectedError::NegativeRate { id, field, value },
        ) => {
            assert_eq!(actual_id, id, "{name}: NegativeRate.id");
            assert_eq!(actual_field, field, "{name}: NegativeRate.field");
            assert_eq!(actual_value, value, "{name}: NegativeRate.value");
        }
        (
            ref actual @ PaygRemapParseError::ContextBandNotRepresentable { id: ref actual_id },
            ExpectedError::ContextBandNotRepresentable { id, message },
        ) => {
            assert_eq!(actual_id, &id, "{name}: ContextBandNotRepresentable.id");
            assert_eq!(
                actual.to_string(),
                message,
                "{name}: ContextBandNotRepresentable"
            );
        }
        (
            PaygRemapParseError::InvalidIdPrefix { id: actual },
            ExpectedError::InvalidIdPrefix { id: expected },
        ) => assert_eq!(actual, expected, "{name}: InvalidIdPrefix.id"),
        (
            PaygRemapParseError::InvalidEffectiveFrom {
                id: actual_id,
                value: actual_value,
            },
            ExpectedError::InvalidEffectiveFrom { id, value },
        ) => {
            assert_eq!(actual_id, id, "{name}: InvalidEffectiveFrom.id");
            assert_eq!(actual_value, value, "{name}: InvalidEffectiveFrom.value");
        }
        (
            PaygRemapParseError::UnexpectedField {
                id: actual_id,
                field: actual_field,
            },
            ExpectedError::UnexpectedField { id, field },
        ) => {
            assert_eq!(actual_id, id, "{name}: UnexpectedField.id");
            assert_eq!(actual_field, field, "{name}: UnexpectedField.field");
        }
        (
            PaygRemapParseError::DuplicateEntry { id: actual },
            ExpectedError::DuplicateEntry { id: expected },
        ) => assert_eq!(actual, expected, "{name}: DuplicateEntry.id"),
        (
            PaygRemapParseError::DuplicateIdPrefix { id_prefix: actual },
            ExpectedError::DuplicateIdPrefix {
                id_prefix: expected,
            },
        ) => assert_eq!(actual, expected, "{name}: DuplicateIdPrefix.id_prefix"),
        (actual, expected) => panic!("{name}: expected {expected:?}, got {actual:?}"),
    }
}
