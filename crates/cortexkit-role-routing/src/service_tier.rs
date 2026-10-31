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

/// Read the raw select member before generic decoding, so an unknown level
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
