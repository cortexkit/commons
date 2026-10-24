//! Consumer names and demand defaults from the daemon catalog, never a call.

use std::collections::{BTreeMap, BTreeSet};

use serde::{Deserialize, Serialize};
use serde_json::{json, Value};

use crate::select::{parse_requirements, Modality, ToolsRequirement, REQUIREMENT_FIELDS};

/// Optional declaration defaults using selection demand names and types,
/// except `iq`, which only requests and operator configuration may set.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct DemandDefaults {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub eq: Option<u8>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub context_window: Option<u64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub max_output_tokens: Option<u64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub tools: Option<ToolsRequirement>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub tool_capabilities: Option<Vec<String>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub modalities: Option<Vec<Modality>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub structured_output: Option<bool>,
    /// Unknown members are retained so validation can reject them by name.
    #[serde(flatten, skip_serializing_if = "BTreeMap::is_empty")]
    pub unknown: BTreeMap<String, Value>,
}

/// One consumer owned by the module whose catalog entry carries it.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct ConsumerDeclaration {
    pub name: String,
    #[serde(flatten)]
    pub defaults: DemandDefaults,
}

/// The object in a module's `role_declarations["routing/v1"]` catalog slot.
/// Decode alone does not validate; use [`parse_consumer_declarations`] or
/// [`Self::validate`] before applying any names or defaults.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct ConsumerDeclarations {
    pub consumers: Vec<ConsumerDeclaration>,
    #[serde(flatten, skip_serializing_if = "BTreeMap::is_empty")]
    pub unknown: BTreeMap<String, Value>,
}

impl ConsumerDeclarations {
    /// Validate the whole declaration, including values built without serde.
    pub fn validate(&self) -> Result<(), DeclarationError> {
        parse_consumer_declarations(&serde_json::to_value(self).expect("JSON declaration"))?;
        Ok(())
    }
}

/// A declaration is rejected as a whole; a path identifies the offending member.
#[derive(Clone, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub enum DeclarationError {
    InvalidField { field: String },
    UnknownMember { field: String },
    InvalidName { index: usize, name: String },
    DuplicateName { name: String },
}

impl std::fmt::Display for DeclarationError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::InvalidField { field } => write!(f, "invalid declaration field {field}"),
            Self::UnknownMember { field } => write!(f, "unknown declaration member {field}"),
            Self::InvalidName { index, name } => {
                write!(f, "invalid consumer name {name:?} at consumers[{index}]")
            }
            Self::DuplicateName { name } => write!(f, "duplicate consumer name {name:?}"),
        }
    }
}

impl std::error::Error for DeclarationError {}

/// Validate and decode a catalog declaration. No partially valid declaration
/// is returned. Demand validation is shared with `route.select`, but `iq`
/// is an unknown member here: it can never act as a declared default.
pub fn parse_consumer_declarations(
    value: &Value,
) -> Result<ConsumerDeclarations, DeclarationError> {
    let invalid = |field: String| DeclarationError::InvalidField { field };
    let object = value.as_object().ok_or_else(|| invalid("$".into()))?;
    if let Some(field) = object.keys().find(|key| key.as_str() != "consumers") {
        return Err(DeclarationError::UnknownMember {
            field: field.clone(),
        });
    }
    let consumers = object
        .get("consumers")
        .and_then(Value::as_array)
        .ok_or_else(|| invalid("consumers".into()))?;
    let mut names = BTreeSet::new();
    for (index, consumer) in consumers.iter().enumerate() {
        let path = format!("consumers[{index}]");
        let fields = consumer.as_object().ok_or_else(|| invalid(path.clone()))?;
        let name = fields
            .get("name")
            .and_then(Value::as_str)
            .ok_or_else(|| invalid(format!("{path}.name")))?;
        // Selection requests require only a non-empty string, not a new
        // character set or length limit for catalog names.
        if name.is_empty() {
            return Err(DeclarationError::InvalidName {
                index,
                name: name.into(),
            });
        }
        if !names.insert(name) {
            return Err(DeclarationError::DuplicateName { name: name.into() });
        }
        if let Some(field) = fields.keys().find(|key| {
            key.as_str() != "name"
                && (key.as_str() == "iq" || !REQUIREMENT_FIELDS.contains(&key.as_str()))
        }) {
            return Err(DeclarationError::UnknownMember {
                field: format!("{path}.{field}"),
            });
        }
        let mut demands = fields.clone();
        demands.remove("name");
        // The request validator requires iq. A temporary zero lets it check
        // the declaration's other demands without storing a declared iq.
        demands.insert("iq".into(), json!(0));
        parse_requirements(Some(&Value::Object(demands))).map_err(|refusal| {
            let field = refusal
                .detail
                .get("field")
                .and_then(Value::as_str)
                .unwrap_or("requirements");
            invalid(format!("{path}.{field}"))
        })?;
    }
    serde_json::from_value(value.clone()).map_err(|_| invalid("consumers".into()))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn defaults_are_optional_and_null_is_absent() {
        let declaration = parse_consumer_declarations(&json!({"consumers": [
            {"name": "example", "eq": null, "tools": null}
        ]}))
        .unwrap();
        assert_eq!(declaration.consumers[0].defaults, DemandDefaults::default());
        assert_eq!(
            serde_json::to_value(declaration).unwrap(),
            json!({"consumers": [{"name": "example"}]})
        );
        assert!(parse_consumer_declarations(&json!({"consumers": []})).is_ok());
    }

    #[test]
    fn declaration_key_and_name_grammar_match_selection() {
        assert_eq!(crate::ROLE_DECLARATIONS_KEY, "routing/v1");
        for name in [
            "example.curate",
            "example_summary",
            "a name with spaces",
            " ",
        ] {
            let declaration = json!({"consumers": [{"name": name}]});
            assert!(parse_consumer_declarations(&declaration).is_ok());
            let request = json!({"consumer": name, "requirements": {"iq": 0}});
            assert!(crate::select::parse_select_request(&request).is_ok());
        }
    }

    #[test]
    fn malformed_defaults_name_the_field_and_reject_the_whole_declaration() {
        for (field, value) in [
            ("eq", json!(-1)),
            ("context_window", json!(-1)),
            ("max_output_tokens", json!(1.5)),
            ("tools", json!(["code.read/v1"])),
            ("tools", json!("future")),
            ("tool_capabilities", json!([""])),
            ("modalities", json!(["future"])),
            ("structured_output", json!("true")),
        ] {
            let mut consumer = json!({"name": "bad"});
            consumer[field] = value;
            assert_eq!(
                parse_consumer_declarations(&json!({"consumers": [{"name": "valid"}, consumer]})),
                Err(DeclarationError::InvalidField {
                    field: format!("consumers[1].{field}")
                })
            );
        }
    }

    #[test]
    fn malformed_declaration_shapes_are_named() {
        for (value, field) in [
            (json!(null), "$"),
            (json!({}), "consumers"),
            (json!({"consumers": null}), "consumers"),
            (json!({"consumers": [null]}), "consumers[0]"),
            (json!({"consumers": [{}]}), "consumers[0].name"),
            (json!({"consumers": [{"name": 1}]}), "consumers[0].name"),
        ] {
            assert_eq!(
                parse_consumer_declarations(&value),
                Err(DeclarationError::InvalidField {
                    field: field.into()
                })
            );
        }
        let value = json!({"consumers": [], "extra": false});
        let decoded: ConsumerDeclarations = serde_json::from_value(value).unwrap();
        assert_eq!(
            decoded.validate(),
            Err(DeclarationError::UnknownMember {
                field: "extra".into()
            })
        );
    }

    #[test]
    fn typed_validation_keeps_unknown_members_and_checks_demands() {
        let mut declaration = ConsumerDeclarations {
            consumers: vec![ConsumerDeclaration {
                name: "example".into(),
                defaults: DemandDefaults::default(),
            }],
            ..Default::default()
        };
        declaration.consumers[0]
            .defaults
            .unknown
            .insert("extra".into(), json!(true));
        assert_eq!(
            declaration.validate(),
            Err(DeclarationError::UnknownMember {
                field: "consumers[0].extra".into()
            })
        );
        declaration.consumers[0].defaults.unknown.clear();
        declaration.consumers[0].defaults.eq = Some(101);
        assert_eq!(
            declaration.validate(),
            Err(DeclarationError::InvalidField {
                field: "consumers[0].eq".into()
            })
        );
    }

    #[test]
    fn declared_iq_is_an_unknown_member_even_when_null() {
        for iq in [json!(0), json!(70), json!(101), json!(null)] {
            let value = json!({"consumers": [{"name": "example", "iq": iq}]});
            let expected = DeclarationError::UnknownMember {
                field: "consumers[0].iq".into(),
            };
            assert_eq!(parse_consumer_declarations(&value), Err(expected.clone()));
            let decoded: ConsumerDeclarations = serde_json::from_value(value).unwrap();
            assert_eq!(decoded.validate(), Err(expected));
        }
    }
}
