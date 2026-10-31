//! Provider-independent request levels and truthful served-tier reporting.

use std::collections::{BTreeMap, BTreeSet};

use serde::{Deserialize, Serialize};

/// A requested level. Provider-specific request values are a runner's mapping.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
#[non_exhaustive]
pub enum ServiceTier {
    #[default]
    Standard,
    Fast,
    Ultrafast,
}

impl ServiceTier {
    /// Every request level this release can apply, from standard to ultrafast.
    pub const ALL: &'static [Self] = &[Self::Standard, Self::Fast, Self::Ultrafast];

    pub fn parse(value: &str) -> Option<Self> {
        match value {
            "standard" => Some(Self::Standard),
            "fast" => Some(Self::Fast),
            "ultrafast" => Some(Self::Ultrafast),
            _ => None,
        }
    }
    pub fn is_standard(&self) -> bool {
        *self == Self::Standard
    }

    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Standard => "standard",
            Self::Fast => "fast",
            Self::Ultrafast => "ultrafast",
        }
    }
}

/// Whether an optional level is absent or standard. This is not a send
/// serialization policy: explicit standard must stay on the wire.
pub fn is_standard_or_absent(tier: &Option<ServiceTier>) -> bool {
    tier.is_none_or(|tier| tier.is_standard())
}

/// Whether the wire lets a runner confirm the provider's served tier.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
#[non_exhaustive]
pub enum Served {
    Reported,
    Unknown,
}

/// Non-standard levels a runner can apply on one wire API. Standard is implied.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[non_exhaustive]
pub struct Support {
    pub levels: Vec<ServiceTier>,
    pub served: Served,
}

impl Support {
    pub fn new(levels: Vec<ServiceTier>, served: Served) -> Self {
        Self { levels, served }
    }
}

/// Keys use routing 0.5.4's wire API spellings, not provider or model names.
pub type ServiceTiers = BTreeMap<String, Support>;

/// Routing's wire API spellings, pinned by a cross-crate drift test in routing.
pub const WIRE_APIS: &[&str] = &[
    "anthropic.messages",
    "openai.responses",
    "openai.chat_completions",
    "openai.chatgpt_responses",
    "google.generate_content",
    "google.code_assist",
    "aws.bedrock_converse",
];

/// Validate a declared map, including the non-empty, duplicate-free levels.
pub fn check_service_tiers(tiers: &ServiceTiers) -> Result<(), &'static str> {
    for (wire, support) in tiers {
        if !WIRE_APIS.contains(&wire.as_str()) {
            return Err("unknown_wire_api");
        }
        if support.levels.is_empty() {
            return Err("empty_levels");
        }
        let mut seen = BTreeSet::new();
        for level in &support.levels {
            if level.is_standard() {
                return Err("standard_implied");
            }
            if !seen.insert(level) {
                return Err("duplicate_level");
            }
        }
    }
    Ok(())
}

/// The reason in an `invalid_params` detail whose field is `service_tier`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub enum TierProblem {
    UnsupportedTier,
    RunTierFixed,
}

impl TierProblem {
    pub fn reason(&self) -> &'static str {
        match self {
            Self::UnsupportedTier => "unsupported_tier",
            Self::RunTierFixed => "run_tier_fixed",
        }
    }

    pub fn detail(&self) -> serde_json::Value {
        serde_json::json!({"field": "service_tier", "reason": self.reason()})
    }
}

/// Read the raw request member before generic decoding so an unknown level is
/// refused as `invalid_params` with this problem's field and reason detail.
pub fn parse_request(
    value: Option<&serde_json::Value>,
) -> Result<Option<ServiceTier>, TierProblem> {
    match value {
        None | Some(serde_json::Value::Null) => Ok(None),
        Some(value) => value
            .as_str()
            .and_then(ServiceTier::parse)
            .map(Some)
            .ok_or(TierProblem::UnsupportedTier),
    }
}

pub(crate) fn deserialize_request<'de, D: serde::Deserializer<'de>>(
    deserializer: D,
) -> Result<Option<ServiceTier>, D::Error> {
    let value = Option::<serde_json::Value>::deserialize(deserializer)?;
    parse_request(value.as_ref())
        .map_err(|problem| serde::de::Error::custom(format!("service_tier: {}", problem.reason())))
}

/// Producer-side validation only: record readers retain unknown applied values.
pub fn check_applied(value: Option<&str>) -> Result<(), TierProblem> {
    match value {
        None | Some("fast" | "ultrafast") => Ok(()),
        Some(_) => Err(TierProblem::UnsupportedTier),
    }
}

/// Check before dispatch. `running` is the fixed tier of a run this send
/// steers or queues into; an absent send tier inherits that run's tier.
pub fn check_send(
    requested: Option<ServiceTier>,
    running: Option<ServiceTier>,
    wire: &str,
    declared: Option<&ServiceTiers>,
) -> Result<ServiceTier, TierProblem> {
    if let (Some(requested), Some(running)) = (requested, running) {
        if requested != running {
            return Err(TierProblem::RunTierFixed);
        }
    }
    let tier = requested.or(running).unwrap_or_default();
    if !tier.is_standard()
        && !declared
            .and_then(|tiers| tiers.get(wire))
            .is_some_and(|support| support.levels.contains(&tier))
    {
        return Err(TierProblem::UnsupportedTier);
    }
    Ok(tier)
}

/// Owned metadata for one step, separate from the token-only, Copy StepUsage.
/// `service_tier_served` comes only from a provider response on a reported wire:
/// never copy the applied tier into it. Map known provider values back to the
/// abstract level and preserve an unmapped response value verbatim.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[non_exhaustive]
pub struct ServiceTierUsage {
    /// An open record string. Producers use ServiceTier::as_str and check().
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub service_tier_applied: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub service_tier_served: Option<String>,
}

impl ServiceTierUsage {
    pub fn new(applied: ServiceTier) -> Self {
        Self {
            service_tier_applied: (!applied.is_standard()).then(|| applied.as_str().into()),
            service_tier_served: None,
        }
    }

    /// Check only when producing metadata, never when reading a stored record.
    pub fn check(&self) -> Result<(), TierProblem> {
        check_applied(self.service_tier_applied.as_deref())
    }

    /// Only call with a value the provider stated on a `reported` wire.
    pub fn with_served(mut self, served: impl Into<String>) -> Self {
        self.service_tier_served = Some(served.into());
        self
    }
}

/// Durable aggregation state that keeps a constant number of tier strings,
/// independent of the number of steps, and examines only the new step. It is
/// not a role wire record. Persist it alongside a run's token totals so a
/// restart cannot forget a disagreement.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[non_exhaustive]
pub struct RunServiceTierUsage {
    steps: u64,
    applied: Option<String>,
    served: Option<String>,
    conflicted: bool,
}

impl RunServiceTierUsage {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn steps(&self) -> u64 {
        self.steps
    }

    /// A different or unknown applied level refuses without changing the aggregate.
    pub fn record_step(&mut self, step: &ServiceTierUsage) -> Result<(), TierProblem> {
        step.check()?;
        let applied = step.service_tier_applied.as_deref().unwrap_or("standard");
        if self
            .applied
            .as_deref()
            .is_some_and(|fixed| fixed != applied)
        {
            return Err(TierProblem::RunTierFixed);
        }
        self.applied = Some(applied.into());
        self.steps = self.steps.saturating_add(1);
        if let Some(served) = &step.service_tier_served {
            if !self.conflicted {
                match &self.served {
                    None => self.served = Some(served.clone()),
                    Some(first) if first != served => {
                        self.served = None;
                        self.conflicted = true;
                    }
                    Some(_) => {}
                }
            }
        }
        Ok(())
    }

    /// Missing reports do not vote; disagreement leaves served absent forever.
    pub fn usage(&self) -> ServiceTierUsage {
        ServiceTierUsage {
            service_tier_applied: self.applied.clone().filter(|tier| tier != "standard"),
            service_tier_served: self.served.clone().filter(|_| !self.conflicted),
        }
    }
}
