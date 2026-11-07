//! Session cache requests and provider-independent shape checks.
//!
//! A missing request leaves the cache lifetime to the runner and disables
//! warming. Zone recognition and provider mappings belong to the runner.

use serde::{Deserialize, Serialize};

/// Requested explicit cache lifetime, mapped to the route's supported choices.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
#[non_exhaustive]
pub enum Ttl {
    None,
    Short,
    Long,
}

/// Warming is opt-in; an omitted mode never incurs warming costs.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
#[non_exhaustive]
pub enum KeepWarm {
    #[default]
    Off,
    Hours,
    Always,
    Auto,
}

/// A half-open local-time window. Crossing midnight is allowed.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[non_exhaustive]
pub struct Hours {
    pub from: String,
    pub to: String,
    /// An IANA zone name recognized by the runner.
    pub tz: String,
}

impl Hours {
    pub fn new(from: impl Into<String>, to: impl Into<String>, tz: impl Into<String>) -> Self {
        Self {
            from: from.into(),
            to: to.into(),
            tz: tz.into(),
        }
    }
}

/// A request, not a requirement that the route support explicit caching or warms.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[non_exhaustive]
pub struct Cache {
    /// Opaque provider routing hint; callers must not put secrets or private data here.
    #[serde(
        default,
        skip_serializing_if = "Option::is_none",
        deserialize_with = "deserialize_key"
    )]
    pub key: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub ttl: Option<Ttl>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub keep_warm: Option<KeepWarm>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub hours: Option<Hours>,
}

impl Cache {
    pub fn new() -> Self {
        Self::default()
    }

    /// Resolve an omitted mode without enabling warming by default.
    pub fn keep_warm(&self) -> KeepWarm {
        self.keep_warm.unwrap_or_default()
    }
}

/// An `invalid_params` refusal body, ready for the runner's error envelope.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[non_exhaustive]
pub struct ErrorBody {
    pub code: String,
    pub message: String,
    pub detail: serde_json::Value,
}

impl ErrorBody {
    pub fn invalid_params(field: impl Into<String>, message: impl Into<String>) -> Self {
        Self {
            code: crate::errors::INVALID_PARAMS.into(),
            message: message.into(),
            detail: serde_json::json!({ "field": field.into() }),
        }
    }
}

/// Check the provider-independent shape of a plan's cache request.
///
/// Refusals name `plan.cache.…`. A creating send uses the same rules with
/// `cache.…` fields instead. The runner must also validate the IANA zone before
/// acceptance; this crate deliberately carries no time-zone database.
pub fn check(cache: &Cache) -> Result<(), ErrorBody> {
    check_at(cache, "plan.cache")
}

/// Check a creating send's cache request, using its field names in refusals.
pub fn check_send(cache: &Cache) -> Result<(), ErrorBody> {
    check_at(cache, "cache")
}

/// Decode and shape-check a plan member before admission or refresh.
/// Call only for a runner declaring `cache_policy`; unknown plan members on
/// older runners keep their existing interpretation.
pub fn check_plan(
    plan: &serde_json::Map<String, serde_json::Value>,
) -> Result<Option<Cache>, ErrorBody> {
    let Some(value) = plan.get("cache") else {
        return Ok(None);
    };
    let cache = decode_at(value, "plan.cache")?;
    check(&cache)?;
    Ok(Some(cache))
}

/// Decode and shape-check a creating send's raw cache policy.
/// Use this before typed decoding to preserve exact refusal fields for bad key types.
pub fn check_send_value(value: &serde_json::Value) -> Result<Cache, ErrorBody> {
    let cache = decode_at(value, "cache")?;
    check_send(&cache)?;
    Ok(cache)
}

fn decode_at(value: &serde_json::Value, prefix: &str) -> Result<Cache, ErrorBody> {
    if let Some(key) = value.get("key") {
        check_key(key.as_str(), prefix)?;
    }
    serde_json::from_value(value.clone())
        .map_err(|_| ErrorBody::invalid_params(prefix, "expected a cache policy object"))
}

fn deserialize_key<'de, D: serde::Deserializer<'de>>(
    deserializer: D,
) -> Result<Option<String>, D::Error> {
    let value = serde_json::Value::deserialize(deserializer)?;
    let key = value.as_str();
    // A present null must not become an omitted routing hint during typed decoding.
    check_key(key, "cache")
        .map_err(|error| serde::de::Error::custom(format!("cache.key: {}", error.message)))?;
    Ok(key.map(str::to_owned))
}

fn check_key(key: Option<&str>, prefix: &str) -> Result<(), ErrorBody> {
    if key.is_some_and(|key| {
        (1..=128).contains(&key.len()) && key.bytes().all(|byte| (0x21..=0x7e).contains(&byte))
    }) {
        Ok(())
    } else {
        Err(ErrorBody::invalid_params(
            format!("{prefix}.key"),
            "expected 1 to 128 bytes of printable ASCII without spaces",
        ))
    }
}

pub(crate) fn deserialize_optional<'de, D: serde::Deserializer<'de>>(
    deserializer: D,
) -> Result<Option<Cache>, D::Error> {
    Cache::deserialize(deserializer)
        .map(Some)
        .map_err(|error| serde::de::Error::custom(format!("cache: {error}")))
}

fn check_at(cache: &Cache, prefix: &str) -> Result<(), ErrorBody> {
    if let Some(key) = &cache.key {
        check_key(Some(key), prefix)?;
    }
    let invalid = |field: &str, message: &str| {
        ErrorBody::invalid_params(format!("{prefix}.{field}"), message)
    };
    let mode = cache.keep_warm();
    if cache.ttl == Some(Ttl::None) && mode != KeepWarm::Off {
        return Err(invalid("keep_warm", "ttl none requires keep_warm off"));
    }
    if (mode == KeepWarm::Hours) != cache.hours.is_some() {
        return Err(invalid(
            "hours",
            "hours is required only for keep_warm hours",
        ));
    }
    if let Some(hours) = &cache.hours {
        if !is_time(&hours.from) {
            return Err(invalid("hours.from", "expected HH:MM in 00:00..23:59"));
        }
        if !is_time(&hours.to) || hours.from == hours.to {
            return Err(invalid(
                "hours.to",
                "expected a distinct HH:MM in 00:00..23:59",
            ));
        }
    }
    Ok(())
}

fn is_time(value: &str) -> bool {
    let bytes = value.as_bytes();
    bytes.len() == 5
        && bytes[2] == b':'
        && [bytes[0], bytes[1], bytes[3], bytes[4]]
            .iter()
            .all(u8::is_ascii_digit)
        && &bytes[..2] <= b"23"
        && &bytes[3..] <= b"59"
}

/// Actual per-step cache facts, separate from the requested policy.
/// Missing members mean unknown; `implicit: true` has no provable lifetime.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
#[non_exhaustive]
pub struct Report {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub ttl_ms: Option<u64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub implicit: Option<bool>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub warming: Option<Warming>,
}

impl Report {
    pub fn explicit(ttl_ms: u64) -> Self {
        Self {
            ttl_ms: Some(ttl_ms),
            ..Self::default()
        }
    }
    pub fn implicit() -> Self {
        Self {
            implicit: Some(true),
            ..Self::default()
        }
    }
    pub fn with_warming(mut self, warming: Warming) -> Self {
        self.warming = Some(warming);
        self
    }
    /// Producer-side consistency, without dropping usage on a reader.
    pub fn check(&self) -> Result<(), &'static str> {
        if self.implicit == Some(false) || (self.implicit == Some(true) && self.ttl_ms.is_some()) {
            return Err("cache.implicit");
        }
        if let Some(warming) = &self.warming {
            if let Some(reason) = &warming.stop_reason {
                if warming.on
                    || !matches!(
                        reason.as_str(),
                        "cost-cap"
                            | "count-cap"
                            | "outside-hours"
                            | "session-ended"
                            | "unsupported"
                    )
                {
                    return Err("cache.warming.stop_reason");
                }
            }
        }
        Ok(())
    }
}

/// Whether warming is on for the session at the recorded step.
/// An off policy never starts warming, so it needs no stop reason.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
#[non_exhaustive]
pub struct Warming {
    pub on: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub stop_reason: Option<String>,
}

impl Warming {
    pub fn new(on: bool) -> Self {
        Self {
            on,
            stop_reason: None,
        }
    }
    pub fn stopped(reason: impl Into<String>) -> Self {
        Self {
            on: false,
            stop_reason: Some(reason.into()),
        }
    }
}
