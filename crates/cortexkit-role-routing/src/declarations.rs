//! Consumer defaults and runner matching declarations from the catalog, never a call.

use std::collections::{BTreeMap, BTreeSet};

use serde::{Deserialize, Serialize};
use serde_json::{json, Value};

use crate::select::{parse_requirements, Modality, ToolsRequirement, REQUIREMENT_FIELDS};

/// Optional declaration defaults using selection demand names and types.
#[derive(Clone, Debug, Default, Serialize)]
pub struct DemandDefaults {
    /// Serde presence metadata for strict legacy validation, never a wire member.
    #[doc(hidden)]
    #[serde(skip)]
    pub iq_present: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub iq: Option<u8>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub ladder: Option<Vec<String>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub placement: Option<crate::placement::Placement>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub selectors: Option<Vec<String>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub kind: Option<crate::placement::Kind>,
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

impl<'de> Deserialize<'de> for DemandDefaults {
    fn deserialize<D: serde::Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        fn take<T: serde::de::DeserializeOwned, E: serde::de::Error>(
            fields: &mut BTreeMap<String, Value>,
            name: &str,
        ) -> Result<Option<T>, E> {
            fields
                .remove(name)
                .filter(|value| !value.is_null())
                .map(serde_json::from_value)
                .transpose()
                .map_err(E::custom)
        }
        let mut fields = BTreeMap::<String, Value>::deserialize(deserializer)?;
        Ok(Self {
            iq_present: fields.contains_key("iq"),
            iq: take(&mut fields, "iq")?,
            eq: take(&mut fields, "eq")?,
            ladder: take(&mut fields, "ladder")?,
            placement: take(&mut fields, "placement")?,
            selectors: take(&mut fields, "selectors")?,
            kind: take(&mut fields, "kind")?,
            context_window: take(&mut fields, "context_window")?,
            max_output_tokens: take(&mut fields, "max_output_tokens")?,
            tools: take(&mut fields, "tools")?,
            tool_capabilities: take(&mut fields, "tool_capabilities")?,
            modalities: take(&mut fields, "modalities")?,
            structured_output: take(&mut fields, "structured_output")?,
            unknown: fields,
        })
    }
}

impl PartialEq for DemandDefaults {
    fn eq(&self, other: &Self) -> bool {
        self.iq == other.iq
            && self.eq == other.eq
            && self.ladder == other.ladder
            && self.placement == other.placement
            && self.selectors == other.selectors
            && self.kind == other.kind
            && self.context_window == other.context_window
            && self.max_output_tokens == other.max_output_tokens
            && self.tools == other.tools
            && self.tool_capabilities == other.tool_capabilities
            && self.modalities == other.modalities
            && self.structured_output == other.structured_output
            && self.unknown == other.unknown
    }
}
impl Eq for DemandDefaults {}

impl DemandDefaults {
    pub fn new() -> Self {
        Self::default()
    }
    pub fn with_iq(mut self, iq: u8) -> Self {
        self.iq_present = true;
        self.iq = Some(iq);
        self
    }
    pub fn with_ladder(mut self, ladder: Vec<String>) -> Self {
        self.ladder = Some(ladder);
        self
    }
    pub fn with_placement(mut self, placement: crate::placement::Placement) -> Self {
        self.placement = Some(placement);
        self
    }
    pub fn with_selectors(mut self, selectors: Vec<String>) -> Self {
        self.selectors = Some(selectors);
        self
    }
    pub fn with_kind(mut self, kind: crate::placement::Kind) -> Self {
        self.kind = Some(kind);
        self
    }
}

/// One consumer owned by the module whose catalog entry carries it.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct ConsumerDeclaration {
    pub name: String,
    #[serde(flatten)]
    pub defaults: DemandDefaults,
}

impl ConsumerDeclaration {
    pub fn new(name: impl Into<String>) -> Self {
        Self {
            name: name.into(),
            defaults: DemandDefaults::default(),
        }
    }
    pub fn with_defaults(mut self, defaults: DemandDefaults) -> Self {
        self.defaults = defaults;
        self
    }
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

    /// Validate for a reader that does not declare the new consumer group.
    /// Even a decoded `iq: null` is refused as an unknown legacy member.
    pub fn validate(&self) -> Result<(), DeclarationError> {
        self.validate_serving::<&str>(&[])
    }

    /// Validate typed declarations for the reader's advertised groups. With
    /// `consumer_declarations`, `iq: null` is absent and valid iq defaults apply.
    pub fn validate_serving<C: AsRef<str>>(
        &self,
        capabilities: &[C],
    ) -> Result<(), DeclarationError> {
        let mut value = serde_json::to_value(self).expect("JSON declaration");
        for (index, consumer) in self.consumers.iter().enumerate() {
            if consumer.defaults.iq_present && consumer.defaults.iq.is_none() {
                value["consumers"][index]["iq"] = Value::Null;
            }
        }
        parse_consumer_declarations_serving(&value, capabilities)?;
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
    /// Override the runner's endpoint requirement for this wire/auth pair.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub requires_wire_api: Option<bool>,
    #[serde(flatten, skip_serializing_if = "BTreeMap::is_empty")]
    pub unknown: BTreeMap<String, Value>,
}

impl RunnerPair {
    pub fn new(wire_api: impl Into<String>, auth_method: impl Into<String>) -> Self {
        Self {
            wire_api: wire_api.into(),
            auth_method: auth_method.into(),
            requires_wire_api: None,
            unknown: BTreeMap::new(),
        }
    }

    pub fn with_requires_wire_api(mut self, requires_wire_api: bool) -> Self {
        self.requires_wire_api = Some(requires_wire_api);
        self
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

/// Optional restrictions for one harness; absent fields inherit runner values.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[non_exhaustive]
pub struct HarnessLimits {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub starts_sessions: Option<bool>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub capabilities: Option<Vec<crate::placement::HarnessCapability>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub exclude: Option<Vec<String>>,
    #[serde(
        default,
        with = "harness_pairs",
        skip_serializing_if = "Option::is_none"
    )]
    pub pairs: Option<Vec<RunnerPair>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub explicit_providers: Option<Vec<String>>,
    #[serde(flatten, skip_serializing_if = "BTreeMap::is_empty")]
    pub unknown: BTreeMap<String, Value>,
}

// Harness pairs share the public pair type but only narrow identities. Keep
// requirement members as unknown data here, including values that are not bools.
mod harness_pairs {
    use super::*;

    #[derive(Serialize, Deserialize)]
    struct Pair {
        wire_api: String,
        auth_method: String,
        #[serde(flatten)]
        unknown: BTreeMap<String, Value>,
    }

    pub fn deserialize<'de, D: serde::Deserializer<'de>>(
        deserializer: D,
    ) -> Result<Option<Vec<RunnerPair>>, D::Error> {
        Ok(
            Option::<Vec<Pair>>::deserialize(deserializer)?.map(|pairs| {
                pairs
                    .into_iter()
                    .map(|pair| RunnerPair {
                        wire_api: pair.wire_api,
                        auth_method: pair.auth_method,
                        requires_wire_api: None,
                        unknown: pair.unknown,
                    })
                    .collect()
            }),
        )
    }

    pub fn serialize<S: serde::Serializer>(
        pairs: &Option<Vec<RunnerPair>>,
        serializer: S,
    ) -> Result<S::Ok, S::Error> {
        pairs
            .as_ref()
            .map(|pairs| {
                pairs
                    .iter()
                    .map(|pair| Pair {
                        wire_api: pair.wire_api.clone(),
                        auth_method: pair.auth_method.clone(),
                        unknown: pair.unknown.clone(),
                    })
                    .collect::<Vec<_>>()
            })
            .serialize(serializer)
    }
}

impl HarnessLimits {
    pub fn with_starts_sessions(mut self, starts: bool) -> Self {
        self.starts_sessions = Some(starts);
        self
    }
    pub fn with_capabilities(
        mut self,
        capabilities: Vec<crate::placement::HarnessCapability>,
    ) -> Self {
        self.capabilities = Some(capabilities);
        self
    }
    pub fn with_exclude(mut self, exclude: Vec<String>) -> Self {
        self.exclude = Some(exclude);
        self
    }
    pub fn new() -> Self {
        Self::default()
    }

    pub fn with_pairs(mut self, pairs: Vec<RunnerPair>) -> Self {
        self.pairs = Some(pairs);
        self
    }

    pub fn with_explicit_providers(mut self, providers: Vec<String>) -> Self {
        self.explicit_providers = Some(providers);
        self
    }
}

/// A refused harness-limits member does not invalidate its runner declaration.
#[derive(Clone, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub enum HarnessLimitsError {
    InvalidField { field: String },
    MissingHarnesses,
    UnknownHarness { harness: String },
    EmptyPairs { harness: String },
    PairOutsideRunner { harness: String, index: usize },
    ProviderOutsideRunner { harness: String, index: usize },
}

impl std::fmt::Display for HarnessLimitsError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::InvalidField { field } => write!(f, "invalid declaration field {field}"),
            Self::MissingHarnesses => write!(f, "runner.harness_limits requires runner.harnesses"),
            Self::UnknownHarness { harness } => {
                write!(f, "runner.harness_limits.{harness} is not in runner.harnesses")
            }
            Self::EmptyPairs { harness } => {
                write!(f, "runner.harness_limits.{harness}.pairs is empty")
            }
            Self::PairOutsideRunner { harness, index } => write!(
                f,
                "runner.harness_limits.{harness}.pairs[{index}] is outside runner.pairs"
            ),
            Self::ProviderOutsideRunner { harness, index } => write!(
                f,
                "runner.harness_limits.{harness}.explicit_providers[{index}] is outside runner.standing_limits.explicit_providers"
            ),
        }
    }
}

impl std::error::Error for HarnessLimitsError {}

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
    /// Default endpoint requirement for pairs without their own override.
    #[serde(
        default,
        deserialize_with = "null_default",
        skip_serializing_if = "std::ops::Not::not"
    )]
    pub requires_wire_api: bool,
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
    /// Raw restrictions are decoded separately so a typo cannot remove a runner.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub harness_limits: Option<Value>,
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
            requires_wire_api: false,
            tool_capabilities: Vec::new(),
            standing_limits: None,
            harnesses: None,
            harness_limits: None,
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

    pub fn with_requires_wire_api(mut self, requires_wire_api: bool) -> Self {
        self.requires_wire_api = requires_wire_api;
        self
    }

    pub fn requires_wire_api(&self) -> bool {
        self.requires_wire_api
    }

    /// Read the requirement from the matching runner pair, then the runner.
    /// Only the supplied pair's wire API and auth method are used, so a
    /// harness-limit pair cannot change the runner pair's requirement.
    pub fn requires_wire_api_for_pair(&self, pair: &RunnerPair) -> bool {
        self.pairs
            .iter()
            .find(|declared| {
                declared.wire_api == pair.wire_api && declared.auth_method == pair.auth_method
            })
            .and_then(|declared| declared.requires_wire_api)
            .unwrap_or(self.requires_wire_api())
    }

    /// Whether the matching runner pair admits a row's endpoint availability.
    /// Pair membership and other routing constraints must be checked separately.
    pub fn admits_pair_row(&self, pair: &RunnerPair, row_serves_wire_api: bool) -> bool {
        !self.requires_wire_api_for_pair(pair) || row_serves_wire_api
    }

    /// Whether this runner admits one row's endpoint availability; reads no catalog.
    ///
    /// Deprecated for pair matching: this checks only the runner-level default.
    /// Use [`Self::admits_pair_row`] to honor pair overrides. This helper retains
    /// its original behavior for callers whose runners have no pair overrides.
    pub fn admits_row(&self, row_serves_wire_api: bool) -> bool {
        !self.requires_wire_api() || row_serves_wire_api
    }

    pub fn with_harnesses(mut self, harnesses: Vec<String>) -> Self {
        self.harnesses = Some(harnesses);
        self
    }

    pub fn with_harness_limits(mut self, limits: BTreeMap<String, HarnessLimits>) -> Self {
        self.harness_limits = Some(serde_json::to_value(limits).expect("harness limits are JSON"));
        self
    }

    /// Decode and validate the entire member independently of the declaration.
    /// On error, selection must warn and use runner-level values for all harnesses.
    pub fn typed_harness_limits(
        &self,
    ) -> Option<Result<BTreeMap<String, HarnessLimits>, HarnessLimitsError>> {
        self.harness_limits.as_ref().map(|value| {
            let invalid = |field: String| HarnessLimitsError::InvalidField { field };
            let object = value
                .as_object()
                .ok_or_else(|| invalid("runner.harness_limits".into()))?;
            let harnesses = self
                .harnesses
                .as_ref()
                .ok_or(HarnessLimitsError::MissingHarnesses)?;
            let mut result = BTreeMap::new();
            for (harness, value) in object {
                if !harnesses.contains(harness) {
                    return Err(HarnessLimitsError::UnknownHarness {
                        harness: harness.clone(),
                    });
                }
                let path = format!("runner.harness_limits.{harness}");
                let fields = value.as_object().ok_or_else(|| invalid(path.clone()))?;
                let limits: HarnessLimits =
                    serde_json::from_value(value.clone()).map_err(|_| invalid(path.clone()))?;
                if limits.exclude.as_ref().is_some_and(|list| {
                    list.iter()
                        .any(|spec| !crate::route_spec::is_route_spec(spec))
                }) {
                    return Err(invalid(format!("{path}.exclude")));
                }
                if limits
                    .capabilities
                    .as_ref()
                    .is_some_and(|list| list.iter().any(|cap| !is_runner_token(cap.as_str())))
                {
                    return Err(invalid(format!("{path}.capabilities")));
                }
                if let Some(pairs) = &limits.pairs {
                    if pairs.is_empty() {
                        return Err(HarnessLimitsError::EmptyPairs {
                            harness: harness.clone(),
                        });
                    }
                    let mut seen = BTreeSet::new();
                    for (index, pair) in pairs.iter().enumerate() {
                        let field = format!("{path}.pairs[{index}]");
                        if !is_runner_token(&pair.wire_api)
                            || !is_runner_token(&pair.auth_method)
                            || !seen.insert((&pair.wire_api, &pair.auth_method))
                        {
                            return Err(invalid(field));
                        }
                        if !self.pairs.iter().any(|runner_pair| {
                            runner_pair.wire_api == pair.wire_api
                                && runner_pair.auth_method == pair.auth_method
                        }) {
                            return Err(HarnessLimitsError::PairOutsideRunner {
                                harness: harness.clone(),
                                index,
                            });
                        }
                    }
                }
                if let Some(value) = fields.get("explicit_providers").filter(|v| !v.is_null()) {
                    validate_unique_list(
                        value,
                        &format!("{path}.explicit_providers"),
                        true,
                        is_runner_token,
                    )
                    .map_err(|error| match error {
                        DeclarationError::InvalidField { field } => invalid(field),
                        _ => invalid(path.clone()),
                    })?;
                }
                if let (Some(providers), Some(runner_providers)) = (
                    &limits.explicit_providers,
                    self.standing_limits
                        .as_ref()
                        .and_then(|limits| limits.explicit_providers.as_ref()),
                ) {
                    for (index, provider) in providers.iter().enumerate() {
                        if !runner_providers.contains(provider) {
                            return Err(HarnessLimitsError::ProviderOutsideRunner {
                                harness: harness.clone(),
                                index,
                            });
                        }
                    }
                }
                result.insert(harness.clone(), limits);
            }
            Ok(result)
        })
    }

    /// Reader-side capability gate. Without the group, new facts are not
    /// applied; the normal invalid-map warning and runner-level fallback apply.
    pub fn typed_harness_limits_serving<C: AsRef<str>>(
        &self,
        capabilities: &[C],
    ) -> Option<Result<BTreeMap<String, HarnessLimits>, HarnessLimitsError>> {
        self.typed_harness_limits().map(|parsed| {
            let parsed = parsed?;
            if !capabilities
                .iter()
                .any(|cap| cap.as_ref() == crate::capabilities::RUNNER_HARNESS_FACTS)
            {
                for (harness, limits) in &parsed {
                    for (field, present) in [
                        ("starts_sessions", limits.starts_sessions.is_some()),
                        ("capabilities", limits.capabilities.is_some()),
                        ("exclude", limits.exclude.is_some()),
                    ] {
                        if present {
                            return Err(HarnessLimitsError::InvalidField {
                                field: format!("runner.harness_limits.{harness}.{field}"),
                            });
                        }
                    }
                }
            }
            Ok(parsed)
        })
    }

    /// Resolve inheritance, returning an error rather than partially applying a
    /// malformed map. Callers log that error and use runner-level values instead.
    pub fn limits_for_harness(
        &self,
        harness: Option<&str>,
    ) -> Result<HarnessLimits, HarnessLimitsError> {
        let limits = self.typed_harness_limits().transpose()?.unwrap_or_default();
        let entry = harness.and_then(|harness| limits.get(harness));
        Ok(HarnessLimits {
            starts_sessions: entry
                .and_then(|limits| limits.starts_sessions)
                .map(|starts| self.starts_sessions && starts),
            capabilities: entry.and_then(|limits| limits.capabilities.clone()),
            exclude: entry.and_then(|limits| limits.exclude.clone()),
            pairs: Some(
                entry
                    .and_then(|limits| limits.pairs.clone())
                    .unwrap_or_else(|| self.pairs.clone()),
            ),
            explicit_providers: entry
                .and_then(|limits| limits.explicit_providers.clone())
                .or_else(|| {
                    self.standing_limits
                        .as_ref()
                        .and_then(|limits| limits.explicit_providers.clone())
                }),
            ..HarnessLimits::default()
        })
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
/// is returned, except that `runner.harness_limits` is validated independently
/// by its typed accessor. New declaration members require the corresponding
/// capability in [`parse_consumer_declarations_serving`].
pub fn parse_consumer_declarations(
    value: &Value,
) -> Result<ConsumerDeclarations, DeclarationError> {
    parse_consumer_declarations_serving::<&str>(value, &[])
}

/// Parse declarations only after checking the live router's groups. Old
/// readers continue to reject new consumer defaults instead of ignoring them.
pub fn parse_consumer_declarations_serving<C: AsRef<str>>(
    value: &Value,
    capabilities: &[C],
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
        let extended = capabilities
            .iter()
            .any(|cap| cap.as_ref() == crate::capabilities::CONSUMER_DECLARATIONS);
        let extensions = ["iq", "ladder", "placement", "selectors", "kind"];
        if let Some(field) = fields.keys().find(|key| {
            key.as_str() != "name"
                && if extensions.contains(&key.as_str()) {
                    !extended
                } else {
                    !REQUIREMENT_FIELDS.contains(&key.as_str())
                }
        }) {
            return Err(DeclarationError::UnknownMember {
                field: format!("{path}.{field}"),
            });
        }
        let mut demands = fields.clone();
        demands.remove("name");
        for field in ["ladder", "placement", "selectors", "kind"] {
            demands.remove(field);
        }
        if demands.get("iq").is_none_or(Value::is_null) {
            demands.insert("iq".into(), json!(0));
        }
        parse_requirements(Some(&Value::Object(demands))).map_err(|refusal| {
            let field = refusal
                .detail
                .get("field")
                .and_then(Value::as_str)
                .unwrap_or("requirements");
            invalid(format!("{path}.{field}"))
        })?;
        let decoded: DemandDefaults = serde_json::from_value(Value::Object(
            fields
                .iter()
                .filter(|(key, _)| key.as_str() != "name")
                .map(|(k, v)| (k.clone(), v.clone()))
                .collect(),
        ))
        .map_err(|_| invalid(path.clone()))?;
        if decoded.ladder.as_ref().is_some_and(|list| {
            list.iter()
                .any(|pin| crate::select::RouteRef::parse_pin(pin).is_none())
        }) {
            return Err(invalid(format!("{path}.ladder")));
        }
        if decoded
            .selectors
            .as_ref()
            .is_some_and(|list| list.iter().any(|id| id.is_empty()))
        {
            return Err(invalid(format!("{path}.selectors")));
        }
        if decoded
            .kind
            .as_ref()
            .is_some_and(|kind| !is_runner_token(kind.as_str()))
        {
            return Err(invalid(format!("{path}.kind")));
        }
        if let Some(crate::placement::Placement::Module { module }) = &decoded.placement {
            if module.is_empty() {
                return Err(invalid(format!("{path}.placement.module")));
            }
        }
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
            "requires_wire_api",
            "tool_capabilities",
            "standing_limits",
            "harnesses",
            "harness_limits",
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
        let pair = declaration_object(
            value,
            &path,
            &["wire_api", "auth_method", "requires_wire_api"],
        )?;
        let token = |name: &str| {
            pair.get(name)
                .and_then(Value::as_str)
                .filter(|value| is_runner_token(value))
                .ok_or_else(|| invalid(format!("{path}.{name}")))
        };
        let wire_api = token("wire_api")?;
        let auth_method = token("auth_method")?;
        if pair
            .get("requires_wire_api")
            .is_some_and(|value| !value.is_null() && !value.is_boolean())
        {
            return Err(invalid(format!("{path}.requires_wire_api")));
        }
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
    if runner
        .get("requires_wire_api")
        .is_some_and(|value| !value.is_null() && !value.is_boolean())
    {
        return Err(invalid("runner.requires_wire_api".into()));
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

    fn harness_runner(limits: Value) -> RunnerDeclaration {
        parse_consumer_declarations(&json!({"runner": {
            "pairs": [{"wire_api": "example.messages", "auth_method": "oauth"},
                      {"wire_api": "example.responses", "auth_method": "apikey"}],
            "standing_limits": {"explicit_providers": ["example", "other"]},
            "harnesses": ["alpha", "beta"],
            "harness_limits": limits
        }}))
        .unwrap()
        .runner
        .unwrap()
    }

    #[test]
    fn harness_limits_parser_accepts_and_accessor_names_refusals() {
        for (value, expected) in [
            (
                json!({"typo": {}}),
                HarnessLimitsError::UnknownHarness {
                    harness: "typo".into(),
                },
            ),
            (
                json!({"alpha": {"pairs": []}}),
                HarnessLimitsError::EmptyPairs {
                    harness: "alpha".into(),
                },
            ),
            (
                json!({"alpha": {"pairs": [{"wire_api": "other.api", "auth_method": "oauth"}]}}),
                HarnessLimitsError::PairOutsideRunner {
                    harness: "alpha".into(),
                    index: 0,
                },
            ),
            (
                json!({"alpha": {"explicit_providers": ["outside"]}}),
                HarnessLimitsError::ProviderOutsideRunner {
                    harness: "alpha".into(),
                    index: 0,
                },
            ),
        ] {
            let runner = harness_runner(value.clone());
            assert_eq!(runner.harness_limits, Some(value));
            assert_eq!(runner.typed_harness_limits(), Some(Err(expected.clone())));
            assert_eq!(
                runner.limits_for_harness(Some("beta")),
                Err(expected.clone())
            );
            assert!(expected.to_string().contains("runner.harness_limits"));
        }
        let mut runner = harness_runner(json!({"alpha": {}}));
        runner.harnesses = None;
        let declaration = ConsumerDeclarations::new().with_runner(runner.clone());
        declaration.validate().unwrap();
        assert_eq!(
            runner.typed_harness_limits(),
            Some(Err(HarnessLimitsError::MissingHarnesses))
        );
    }

    #[test]
    fn harness_limits_malformed_member_is_retained_not_a_declaration_error() {
        for value in [
            json!(false),
            json!([]),
            json!({"alpha": false}),
            json!({"alpha": {"pairs": "wrong"}}),
            json!({"alpha": {"explicit_providers": []}}),
            json!({"alpha": {"explicit_providers": ["example", "example"]}}),
            json!({"alpha": {"explicit_providers": ["Bad token"]}}),
            json!({"alpha": {"pairs": [
                {"wire_api": "example.messages", "auth_method": "oauth"},
                {"wire_api": "example.messages", "auth_method": "oauth"}
            ]}}),
        ] {
            let runner = harness_runner(value.clone());
            assert_eq!(runner.harness_limits, Some(value));
            assert!(matches!(
                runner.typed_harness_limits(),
                Some(Err(HarnessLimitsError::InvalidField { .. }))
            ));
            ConsumerDeclarations::new()
                .with_runner(runner)
                .validate()
                .unwrap();
        }
    }

    #[test]
    fn harness_limits_inherit_and_only_narrow() {
        let mut runner = harness_runner(json!({"alpha": {
            "pairs": [{"wire_api": "example.messages", "auth_method": "oauth", "future": true}],
            "explicit_providers": ["example"]
        }, "beta": {}}));
        // Pair matching uses wire API and auth method, not retained unknown data.
        let alpha = runner.limits_for_harness(Some("alpha")).unwrap();
        assert_eq!(alpha.pairs.unwrap().len(), 1);
        assert_eq!(alpha.explicit_providers.unwrap(), ["example"]);
        for harness in [None, Some("beta"), Some("unlisted")] {
            let inherited = runner.limits_for_harness(harness).unwrap();
            assert_eq!(inherited.pairs.as_ref(), Some(&runner.pairs));
            assert_eq!(inherited.explicit_providers.unwrap(), ["example", "other"]);
        }
        runner.harness_limits = Some(json!({"alpha": {"pairs": null, "explicit_providers": null}}));
        assert_eq!(
            runner.limits_for_harness(Some("alpha")).unwrap().pairs,
            Some(runner.pairs.clone())
        );
        runner.standing_limits = None;
        runner.harness_limits = Some(json!({"alpha": {"explicit_providers": ["new-provider"]}}));
        assert_eq!(
            runner
                .limits_for_harness(Some("alpha"))
                .unwrap()
                .explicit_providers
                .unwrap(),
            ["new-provider"]
        );
        assert_eq!(
            runner
                .limits_for_harness(Some("beta"))
                .unwrap()
                .explicit_providers,
            None
        );
        runner.harness_limits = None;
        assert_eq!(runner.typed_harness_limits(), None);
        assert_eq!(
            runner.limits_for_harness(Some("alpha")).unwrap().pairs,
            Some(runner.pairs.clone())
        );
        assert!(serde_json::to_value(&runner)
            .unwrap()
            .get("harness_limits")
            .is_none());
    }

    #[test]
    fn harness_limits_builder_round_trips() {
        let pair = RunnerPair::new("example.messages", "oauth");
        let runner = RunnerDeclaration::new(vec![pair.clone()])
            .with_harnesses(vec!["alpha".into()])
            .with_harness_limits(BTreeMap::from([(
                "alpha".into(),
                HarnessLimits::new()
                    .with_pairs(vec![pair])
                    .with_explicit_providers(vec!["example".into()]),
            )]));
        let parsed = parse_consumer_declarations(
            &serde_json::to_value(ConsumerDeclarations::new().with_runner(runner.clone())).unwrap(),
        )
        .unwrap()
        .runner
        .unwrap();
        assert_eq!(parsed, runner);
        assert!(parsed.typed_harness_limits().unwrap().is_ok());
        assert!(harness_runner(Value::Null).harness_limits.is_none());
    }

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
