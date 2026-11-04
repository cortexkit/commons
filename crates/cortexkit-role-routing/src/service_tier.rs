//! Strict request levels. Records carry strings so newer levels still decode.

use serde::{Deserialize, Serialize};

/// Abstract request levels, ordered from standard to ultrafast for fallback.
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
    /// Every request level this release can select, from standard to ultrafast.
    pub const ALL: &'static [Self] = &[Self::Standard, Self::Fast, Self::Ultrafast];

    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Standard => "standard",
            Self::Fast => "fast",
            Self::Ultrafast => "ultrafast",
        }
    }

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
}

/// Read a raw request member before generic decoding, so an unknown level
/// receives `unsupported_tier` naming `service_tier`, not a decode error.
pub fn parse_request(
    value: Option<&serde_json::Value>,
) -> Result<Option<ServiceTier>, &'static str> {
    match value {
        None | Some(serde_json::Value::Null) => Ok(None),
        Some(value) => value
            .as_str()
            .and_then(ServiceTier::parse)
            .map(Some)
            .ok_or("unsupported_tier"),
    }
}

pub(crate) fn parse_member(
    value: Option<&serde_json::Value>,
) -> Result<Option<ServiceTier>, crate::errors::Refusal> {
    parse_request(value).map_err(|reason| {
        let mut refusal = crate::errors::Refusal::invalid_request("service_tier", reason);
        refusal.detail.insert("reason".into(), reason.into());
        refusal
    })
}

/// Eligible levels in descending order, never upgrading the request.
pub fn fallback(requested: ServiceTier) -> impl Iterator<Item = ServiceTier> {
    ServiceTier::ALL
        .iter()
        .rev()
        .copied()
        .filter(move |tier| *tier <= requested)
}

/// Producer-side tier check used by `select::check_answer` and available to
/// account-answer producers. It checks only that the chosen tier is present
/// when one was requested and does not exceed the request, so it allows a
/// step-down. A decision-bound `route.account_for` answer must instead return
/// exactly the decision's chosen non-standard tier (§5.5): pass that tier as
/// the effective request here and also require equality.
pub fn check_chosen(requested: Option<ServiceTier>, chosen: Option<&str>) -> bool {
    !(requested.is_some() && chosen.is_none())
        && chosen.is_none_or(|tier| {
            ServiceTier::parse(tier).is_some_and(|tier| tier <= requested.unwrap_or_default())
        })
}

pub(crate) fn deserialize_request<'de, D: serde::Deserializer<'de>>(
    deserializer: D,
) -> Result<Option<ServiceTier>, D::Error> {
    let value = Option::<serde_json::Value>::deserialize(deserializer)?;
    parse_request(value.as_ref())
        .map_err(|reason| serde::de::Error::custom(format!("service_tier: {reason}")))
}

/// Producer check only. Record readers retain unknown applied levels.
pub fn check_applied(value: Option<&str>) -> Result<(), &'static str> {
    match value {
        None | Some("fast" | "ultrafast") => Ok(()),
        Some(_) => Err("unsupported_tier"),
    }
}
