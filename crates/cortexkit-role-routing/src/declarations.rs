//! Consumer defaults and runner matching declarations from the catalog, never a call.

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
#[derive(Clone, Debug, Default, PartialEq, Eq)]
#[non_exhaustive]
pub struct ConsumerDeclarations {
    /// Consumer names and demand defaults; absence decodes to an empty list.
    pub consumers: Vec<ConsumerDeclaration>,
    pub runner: Option<RunnerDeclaration>,
    pub unknown: BTreeMap<String, Value>,
    // An explicit empty consumer list is an existing wire shape. Keep its
    // presence so older slots retain their bytes while runner-only slots omit it.
    consumers_present: bool,
}

impl ConsumerDeclarations {
    /// Start an empty slot, omitting both optional members on the wire.
    pub fn new() -> Self {
        Self::default()
    }

    /// Include consumers, preserving an explicitly empty list on the wire.
    pub fn with_consumers(mut self, consumers: Vec<ConsumerDeclaration>) -> Self {
        self.consumers = consumers;
        self.consumers_present = true;
        self
    }

    /// Include the runner module's matching inputs.
    pub fn with_runner(mut self, runner: RunnerDeclaration) -> Self {
        self.runner = Some(runner);
        self
    }

    /// Validate the whole declaration, including values built without serde.
    pub fn validate(&self) -> Result<(), DeclarationError> {
        parse_consumer_declarations(&serde_json::to_value(self).expect("JSON declaration"))?;
        Ok(())
    }
}

impl Serialize for ConsumerDeclarations {
    fn serialize<S: serde::Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        #[derive(Serialize)]
        struct Slot<'a> {
            #[serde(skip_serializing_if = "Option::is_none")]
            consumers: Option<&'a Vec<ConsumerDeclaration>>,
            #[serde(skip_serializing_if = "Option::is_none")]
            runner: &'a Option<RunnerDeclaration>,
            #[serde(flatten)]
            unknown: &'a BTreeMap<String, Value>,
        }

        Slot {
            consumers: (self.consumers_present || !self.consumers.is_empty())
                .then_some(&self.consumers),
            runner: &self.runner,
            unknown: &self.unknown,
        }
        .serialize(serializer)
    }
}

impl<'de> Deserialize<'de> for ConsumerDeclarations {
    fn deserialize<D: serde::Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        #[derive(Deserialize)]
        struct Slot {
            consumers: Option<Vec<ConsumerDeclaration>>,
            runner: Option<RunnerDeclaration>,
            #[serde(flatten)]
            unknown: BTreeMap<String, Value>,
        }

        let slot = Slot::deserialize(deserializer)?;
        Ok(Self {
            consumers_present: slot.consumers.is_some(),
            consumers: slot.consumers.unwrap_or_default(),
            runner: slot.runner,
            unknown: slot.unknown,
        })
    }
}

/// One open-set wire API and authentication method a runner module can drive.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[non_exhaustive]
pub struct RunnerPair {
    pub wire_api: String,
    pub auth_method: String,
    #[serde(flatten, skip_serializing_if = "BTreeMap::is_empty")]
    pub unknown: BTreeMap<String, Value>,
}

impl RunnerPair {
    pub fn new(wire_api: impl Into<String>, auth_method: impl Into<String>) -> Self {
        Self {
            wire_api: wire_api.into(),
            auth_method: auth_method.into(),
            unknown: BTreeMap::new(),
        }
    }
}

/// Provider restrictions applied to every credential a runner module may use.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[non_exhaustive]
pub struct StandingLimits {
    /// Absence means no provider limit; a present list must be non-empty.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub explicit_providers: Option<Vec<String>>,
    #[serde(flatten, skip_serializing_if = "BTreeMap::is_empty")]
    pub unknown: BTreeMap<String, Value>,
}

/// Matching inputs supplied by a runner module, not a credential or a route.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[non_exhaustive]
pub struct RunnerDeclaration {
    pub pairs: Vec<RunnerPair>,
    /// When true, the runner calls `route.account_for` and uses its answer.
    #[serde(
        default,
        deserialize_with = "null_default",
        skip_serializing_if = "std::ops::Not::not"
    )]
    pub follows_account_choice: bool,
    #[serde(
        default,
        deserialize_with = "null_default",
        skip_serializing_if = "Vec::is_empty"
    )]
    pub tool_capabilities: Vec<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub standing_limits: Option<StandingLimits>,
    /// Exact programs this runner hosts sessions in, independently of its pairs.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub harnesses: Option<Vec<String>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub default_harness: Option<String>,
    /// False excludes the runner from selection, not from account operations.
    #[serde(
        default = "default_starts_sessions",
        deserialize_with = "null_starts_sessions",
        skip_serializing_if = "is_true"
    )]
    pub starts_sessions: bool,
    #[serde(flatten, skip_serializing_if = "BTreeMap::is_empty")]
    pub unknown: BTreeMap<String, Value>,
}

fn default_starts_sessions() -> bool {
    true
}

fn null_starts_sessions<'de, D: serde::Deserializer<'de>>(
    deserializer: D,
) -> Result<bool, D::Error> {
    Ok(Option::<bool>::deserialize(deserializer)?.unwrap_or(true))
}

fn is_true(value: &bool) -> bool {
    *value
}

impl Default for RunnerDeclaration {
    fn default() -> Self {
        Self {
            pairs: Vec::new(),
            follows_account_choice: false,
            tool_capabilities: Vec::new(),
            standing_limits: None,
            harnesses: None,
            default_harness: None,
            starts_sessions: true,
            unknown: BTreeMap::new(),
        }
    }
}

impl RunnerDeclaration {
    pub fn new(pairs: Vec<RunnerPair>) -> Self {
        Self {
            pairs,
            ..Self::default()
        }
    }

    pub fn with_harnesses(mut self, harnesses: Vec<String>) -> Self {
        self.harnesses = Some(harnesses);
        self
    }

    pub fn with_default_harness(mut self, harness: impl Into<String>) -> Self {
        self.default_harness = Some(harness.into());
        self
    }

    pub fn with_starts_sessions(mut self, starts_sessions: bool) -> Self {
        self.starts_sessions = starts_sessions;
        self
    }

    pub fn harnesses(&self) -> Option<&[String]> {
        self.harnesses.as_deref()
    }

    pub fn default_harness(&self) -> Option<&str> {
        self.default_harness.as_deref()
    }

    pub fn starts_sessions(&self) -> bool {
        self.starts_sessions
    }
}

fn null_default<'de, D, T>(deserializer: D) -> Result<T, D::Error>
where
    D: serde::Deserializer<'de>,
    T: Deserialize<'de> + Default,
{
    Ok(Option::<T>::deserialize(deserializer)?.unwrap_or_default())
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
    if let Some(field) = object
        .keys()
        .find(|key| !["consumers", "runner"].contains(&key.as_str()))
    {
        return Err(DeclarationError::UnknownMember {
            field: field.clone(),
        });
    }
    let consumers = match object.get("consumers") {
        Some(value) => value
            .as_array()
            .ok_or_else(|| invalid("consumers".into()))?
            .as_slice(),
        None => &[],
    };
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
    if let Some(runner) = object.get("runner").filter(|value| !value.is_null()) {
        validate_runner(runner)?;
    }
    serde_json::from_value(value.clone()).map_err(|_| invalid("$".into()))
}

fn declaration_object<'a>(
    value: &'a Value,
    path: &str,
    allowed: &[&str],
) -> Result<&'a serde_json::Map<String, Value>, DeclarationError> {
    let object = value
        .as_object()
        .ok_or_else(|| DeclarationError::InvalidField { field: path.into() })?;
    if let Some(field) = object.keys().find(|key| !allowed.contains(&key.as_str())) {
        return Err(DeclarationError::UnknownMember {
            field: format!("{path}.{field}"),
        });
    }
    Ok(object)
}

pub(crate) fn is_runner_token(value: &str) -> bool {
    !value.is_empty()
        && value.len() <= 64
        && value.as_bytes()[0].is_ascii_lowercase()
        && value.bytes().all(|byte| {
            byte.is_ascii_lowercase() || byte.is_ascii_digit() || b"._-".contains(&byte)
        })
}

fn validate_unique_list(
    value: &Value,
    path: &str,
    nonempty: bool,
    valid: fn(&str) -> bool,
) -> Result<(), DeclarationError> {
    let invalid = |field| DeclarationError::InvalidField { field };
    let values = value.as_array().ok_or_else(|| invalid(path.into()))?;
    if (nonempty && values.is_empty()) || values.len() > 64 {
        return Err(invalid(path.into()));
    }
    let mut seen = BTreeSet::new();
    for (index, value) in values.iter().enumerate() {
        let field = format!("{path}[{index}]");
        let value = value.as_str().ok_or_else(|| invalid(field.clone()))?;
        if !valid(value) || !seen.insert(value) {
            return Err(invalid(field));
        }
    }
    Ok(())
}

fn validate_runner(value: &Value) -> Result<(), DeclarationError> {
    let invalid = |field| DeclarationError::InvalidField { field };
    let runner = declaration_object(
        value,
        "runner",
        &[
            "pairs",
            "follows_account_choice",
            "tool_capabilities",
            "standing_limits",
            "harnesses",
            "default_harness",
            "starts_sessions",
        ],
    )?;
    let pairs = runner
        .get("pairs")
        .and_then(Value::as_array)
        .ok_or_else(|| invalid("runner.pairs".into()))?;
    if pairs.is_empty() || pairs.len() > 64 {
        return Err(invalid("runner.pairs".into()));
    }
    let mut seen = BTreeSet::new();
    for (index, value) in pairs.iter().enumerate() {
        let path = format!("runner.pairs[{index}]");
        let pair = declaration_object(value, &path, &["wire_api", "auth_method"])?;
        let token = |name: &str| {
            pair.get(name)
                .and_then(Value::as_str)
                .filter(|value| is_runner_token(value))
                .ok_or_else(|| invalid(format!("{path}.{name}")))
        };
        let wire_api = token("wire_api")?;
        let auth_method = token("auth_method")?;
        if !seen.insert((wire_api, auth_method)) {
            return Err(invalid(path));
        }
    }
    if runner
        .get("starts_sessions")
        .is_some_and(|value| !value.is_null() && !value.is_boolean())
    {
        return Err(invalid("runner.starts_sessions".into()));
    }
    if let Some(value) = runner.get("harnesses").filter(|value| !value.is_null()) {
        validate_unique_list(value, "runner.harnesses", true, is_runner_token)?;
        if value.as_array().expect("validated array").len() > 16 {
            return Err(invalid("runner.harnesses".into()));
        }
    }
    if let Some(value) = runner
        .get("default_harness")
        .filter(|value| !value.is_null())
    {
        let harness = value
            .as_str()
            .filter(|value| is_runner_token(value))
            .ok_or_else(|| invalid("runner.default_harness".into()))?;
        if !runner
            .get("harnesses")
            .and_then(Value::as_array)
            .is_some_and(|values| values.iter().any(|value| value.as_str() == Some(harness)))
        {
            return Err(invalid("runner.default_harness".into()));
        }
    }
    if runner
        .get("follows_account_choice")
        .is_some_and(|value| !value.is_null() && !value.is_boolean())
    {
        return Err(invalid("runner.follows_account_choice".into()));
    }
    if let Some(value) = runner
        .get("tool_capabilities")
        .filter(|value| !value.is_null())
    {
        validate_unique_list(
            value,
            "runner.tool_capabilities",
            false,
            crate::capability_names::is_capability_name,
        )?;
    }
    if let Some(value) = runner
        .get("standing_limits")
        .filter(|value| !value.is_null())
    {
        let limits = declaration_object(value, "runner.standing_limits", &["explicit_providers"])?;
        if let Some(value) = limits
            .get("explicit_providers")
            .filter(|value| !value.is_null())
        {
            validate_unique_list(
                value,
                "runner.standing_limits.explicit_providers",
                true,
                is_runner_token,
            )?;
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn runner_harness_builders_and_legacy_bytes() {
        let legacy = r#"{"runner":{"pairs":[{"wire_api":"api","auth_method":"key"}]}}"#;
        let parsed = parse_consumer_declarations(&serde_json::from_str(legacy).unwrap()).unwrap();
        assert!(parsed.runner.as_ref().unwrap().starts_sessions());
        assert_eq!(serde_json::to_string(&parsed).unwrap(), legacy);
        let runner = RunnerDeclaration::new(vec![RunnerPair::new("api", "key")])
            .with_harnesses(vec!["alpha".into(), "beta".into()])
            .with_default_harness("alpha")
            .with_starts_sessions(false);
        assert_eq!(runner.harnesses().unwrap(), ["alpha", "beta"]);
        assert_eq!(runner.default_harness(), Some("alpha"));
        assert!(!runner.starts_sessions());
        ConsumerDeclarations::new()
            .with_runner(runner)
            .validate()
            .unwrap();
        let invalid =
            RunnerDeclaration::new(vec![RunnerPair::new("api", "key")]).with_harnesses(vec![]);
        assert_eq!(
            ConsumerDeclarations::new().with_runner(invalid).validate(),
            Err(DeclarationError::InvalidField {
                field: "runner.harnesses".into()
            })
        );
    }

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
