//! Non-llm `route.select` requests and answers, dispatched by the request kind.
//!
//! The llm types stay separate: an engine answer must never make the llm
//! answer's route, runner or token limits optional.

use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};
use serde_json::{Number, Value};

use crate::{errors::Refusal, members::Members, select::RouteRef, select_engine as engine};

/// Advertise only the members the router actually reads, under §5.1's rule.
pub const REQUIREMENT_FIELDS: &[&str] = &[
    "language",
    "voice",
    "format",
    "duration_ms",
    "input_bytes",
    "reference_formats",
    "bias",
    "size",
    "resolution",
    "quality",
    "latency_class",
    "metric",
    "output_duration_ms",
    "aspect_ratio",
    "audio",
    "metric_limit",
];

/// A hard evaluation threshold. Exactly one finite bound must be present.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
#[non_exhaustive]
pub struct MetricLimit {
    pub metric: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub at_most: Option<Number>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub at_least: Option<Number>,
}

impl MetricLimit {
    pub fn at_most(metric: impl Into<String>, bound: Number) -> Self {
        Self {
            metric: metric.into(),
            at_most: Some(bound),
            at_least: None,
        }
    }

    pub fn at_least(metric: impl Into<String>, bound: Number) -> Self {
        Self {
            metric: metric.into(),
            at_most: None,
            at_least: Some(bound),
        }
    }

    pub fn check(&self) -> Result<(), Refusal> {
        if self.at_most.is_some() == self.at_least.is_some()
            || self
                .at_most
                .as_ref()
                .or(self.at_least.as_ref())
                .and_then(Number::as_f64)
                .is_none_or(|n| !n.is_finite())
        {
            return Err(Refusal::invalid_requirements(
                "metric_limit",
                "expected exactly one finite bound",
            ));
        }
        Ok(())
    }

    /// Missing measurements never satisfy a hard requirement.
    pub fn admits(&self, score: Option<f64>) -> bool {
        self.check().is_ok()
            && score.is_some_and(|score| {
                score.is_finite()
                    && self
                        .at_most
                        .as_ref()
                        .is_none_or(|n| score <= n.as_f64().unwrap())
                    && self
                        .at_least
                        .as_ref()
                        .is_none_or(|n| score >= n.as_f64().unwrap())
            })
    }
}

/// One wire object, reusing the legacy engine members without changing them.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[non_exhaustive]
pub struct MediaRequirements {
    #[serde(flatten)]
    pub engine: engine::EngineRequirements,
    /// Relative catalog grade demand, not an engine accuracy score.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub iq: Option<u8>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub output_duration_ms: Option<u64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub aspect_ratio: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub audio: Option<bool>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub metric_limit: Option<MetricLimit>,
}

impl MediaRequirements {
    pub fn new(engine: engine::EngineRequirements) -> Self {
        Self {
            engine,
            ..Self::default()
        }
    }
    pub fn with_iq(mut self, level: u8) -> Self {
        self.iq = Some(level);
        self
    }
}

/// The non-llm decoding of `route.select`; not a second operation.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
#[non_exhaustive]
pub struct SelectMediaRequest {
    pub consumer: String,
    pub kind: engine::Kind,
    pub requirements: MediaRequirements,
    pub privacy: engine::Privacy,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub caller_local: Option<Vec<engine::Engine>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub pin: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub exclude: Option<Vec<String>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub exclude_providers: Option<Vec<String>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub eligible_routes: Option<Vec<RouteRef>>,
}

impl SelectMediaRequest {
    pub fn new(
        consumer: impl Into<String>,
        kind: engine::Kind,
        requirements: MediaRequirements,
        privacy: engine::Privacy,
    ) -> Self {
        Self {
            consumer: consumer.into(),
            kind,
            requirements,
            privacy,
            caller_local: None,
            pin: None,
            exclude: None,
            exclude_providers: None,
            eligible_routes: None,
        }
    }

    /// Reuse the legacy engine matching and location checks, not its ranking.
    pub fn engine_request(&self) -> engine::SelectEngineRequest {
        let mut request = engine::SelectEngineRequest::new(
            &self.consumer,
            self.kind.clone(),
            self.requirements.engine.clone(),
            self.privacy.clone(),
        );
        request.caller_local = self.caller_local.clone();
        request
    }
}

/// Dispatch here only when `select_media` is advertised and `kind` is non-llm.
/// `kinds` is the router's advertised list, never inferred from the request.
pub fn parse_select_media_request<C: AsRef<str>, K: AsRef<str>>(
    value: &Value,
    capabilities: &[C],
    kinds: &[K],
) -> Result<SelectMediaRequest, Refusal> {
    let members = Members::of(value)?;
    let serves = |group: &str| capabilities.iter().any(|c| c.as_ref() == group);
    let kind: String = members.required("kind")?;
    if !serves(crate::capabilities::SELECT_MEDIA)
        || !serves(crate::capabilities::SELECT_KIND)
        || kind == "llm"
        || !kinds.iter().any(|k| k.as_ref() == kind)
    {
        return Err(members.refuse("kind", "kind is not served by select_media"));
    }
    for key in members.object.keys() {
        if ![
            "consumer",
            "kind",
            "requirements",
            "privacy",
            "caller_local",
            "pin",
            "exclude",
            "exclude_providers",
            "eligible_routes",
        ]
        .contains(&key.as_str())
        {
            return Err(members.refuse(key, "unknown member"));
        }
    }
    let mut legacy = value.clone();
    let object = legacy.as_object_mut().unwrap();
    for key in ["pin", "exclude", "exclude_providers", "eligible_routes"] {
        object.remove(key);
    }
    let requirements = object
        .get_mut("requirements")
        .and_then(Value::as_object_mut)
        .ok_or_else(|| Refusal::invalid_requirements("requirements", "expected an object"))?;
    let extra: serde_json::Map<String, Value> = [
        "iq",
        "output_duration_ms",
        "aspect_ratio",
        "audio",
        "metric_limit",
    ]
    .into_iter()
    .filter_map(|key| requirements.remove(key).map(|value| (key.into(), value)))
    .collect();
    let extra = Members {
        object: &extra,
        refuse: |field, message| Refusal::invalid_requirements(field, message),
    };
    let iq: Option<u8> = extra.optional("iq")?;
    if iq.is_some_and(|n| n > 100) {
        return Err(extra.refuse("iq", "expected 0 to 100"));
    }
    let output_duration_ms = extra.optional("output_duration_ms")?;
    let aspect_ratio: Option<String> = extra.optional("aspect_ratio")?;
    if aspect_ratio
        .as_ref()
        .is_some_and(|s| !valid_aspect_ratio(s))
    {
        return Err(extra.refuse("aspect_ratio", "expected positive integers W:H"));
    }
    let audio = extra.optional("audio")?;
    let metric_limit: Option<MetricLimit> = extra.optional("metric_limit")?;
    if let Some(limit) = &metric_limit {
        limit.check()?;
    }
    let parsed = engine::parse_select_engine_request(&legacy)?;
    let mut common = value.clone();
    let common_object = common.as_object_mut().unwrap();
    common_object.remove("privacy");
    common_object.remove("caller_local");
    common_object.insert("requirements".into(), serde_json::json!({"iq":0}));
    let common = crate::select::parse_select_request_serving(&common, capabilities)?;
    if common
        .pin
        .as_ref()
        .is_some_and(|pin| !crate::select::is_plain_route_key(pin))
    {
        return Err(Refusal::pin_unavailable(
            common.pin.unwrap(),
            crate::errors::PinUnavailableReason::Invalid,
            None,
        ));
    }
    Ok(SelectMediaRequest {
        consumer: parsed.consumer,
        kind: parsed.kind,
        requirements: MediaRequirements {
            engine: parsed.requirements,
            iq,
            output_duration_ms,
            aspect_ratio,
            audio,
            metric_limit,
        },
        privacy: parsed.privacy,
        caller_local: parsed.caller_local,
        pin: common.pin,
        exclude: common.exclude,
        exclude_providers: common.exclude_providers,
        eligible_routes: common.eligible_routes,
    })
}

fn valid_aspect_ratio(value: &str) -> bool {
    value.split_once(':').is_some_and(|(w, h)| {
        [w, h].iter().all(|s| {
            !s.is_empty() && s.bytes().all(|b| b.is_ascii_digit()) && s.bytes().any(|b| b != b'0')
        })
    })
}

/// One catalog unit rate. Keys belong to the catalog and are never normalized.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[non_exhaustive]
pub struct UnitRate {
    pub rate: Number,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub step: Option<Number>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub minimum: Option<Number>,
}

impl UnitRate {
    pub fn new(rate: Number) -> Self {
        Self {
            rate,
            step: None,
            minimum: None,
        }
    }
    pub fn with_step(mut self, step: Number) -> Self {
        self.step = Some(step);
        self
    }
    pub fn with_minimum(mut self, minimum: Number) -> Self {
        self.minimum = Some(minimum);
        self
    }
}

/// Unit prices are estimates of value, never money spent or token prices.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[non_exhaustive]
pub struct UnitPrice {
    pub basis: crate::price::PriceBasis,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub multiplier: Option<Number>,
    pub rates: BTreeMap<String, UnitRate>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub estimated_task_usd: Option<Number>,
}

impl UnitPrice {
    pub fn metered(rates: BTreeMap<String, UnitRate>) -> Self {
        Self {
            basis: crate::price::PriceBasis::Metered,
            multiplier: None,
            rates,
            estimated_task_usd: None,
        }
    }

    /// `rates` are the catalog list rates already divided by `multiplier`.
    pub fn subscription_equivalent(rates: BTreeMap<String, UnitRate>, multiplier: Number) -> Self {
        Self {
            basis: crate::price::PriceBasis::SubscriptionEquivalent,
            multiplier: Some(multiplier),
            rates,
            estimated_task_usd: None,
        }
    }

    pub fn with_estimated_task_usd(mut self, estimate: Number) -> Self {
        self.estimated_task_usd = Some(estimate);
        self
    }

    /// An unmatched role/unit is unpriced; no other unit is an alternative.
    pub fn rate(&self, key: &str) -> Option<&UnitRate> {
        self.rates.get(key)
    }

    pub fn check(&self) -> Result<(), &'static str> {
        let mut common =
            crate::price::Price::metered(crate::price::RatesPerMtok::new(0.into(), 0.into()));
        common.basis = self.basis.clone();
        common.multiplier = self.multiplier.clone();
        common.estimated_task_usd = self.estimated_task_usd.clone();
        common.check()?;
        for rate in self.rates.values() {
            for n in [Some(&rate.rate), rate.minimum.as_ref()]
                .into_iter()
                .flatten()
            {
                if n.as_f64().is_none_or(|n| !n.is_finite() || n < 0.0) {
                    return Err("unit rate and minimum must be finite and non-negative");
                }
            }
            if rate
                .step
                .as_ref()
                .is_some_and(|n| n.as_f64().is_none_or(|n| !n.is_finite() || n <= 0.0))
            {
                return Err("unit step must be finite and positive");
            }
        }
        Ok(())
    }
}

/// Non-llm answer, chosen by the kind the caller sent, not an untagged llm union.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[non_exhaustive]
pub struct SelectMediaAnswer {
    pub decision_id: String,
    pub expires_at_ms: u64,
    pub engine: engine::SelectedEngine,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub price: Option<UnitPrice>,
}

impl SelectMediaAnswer {
    pub fn new(
        decision_id: impl Into<String>,
        expires_at_ms: u64,
        engine: engine::SelectedEngine,
    ) -> Self {
        Self {
            decision_id: decision_id.into(),
            expires_at_ms,
            engine,
            price: None,
        }
    }
}

/// Check the reused engine location/privacy rules and provider/model restrictions.
/// Catalog grades, module facts and time-based checks remain router checks.
pub fn check_answer(
    request: &SelectMediaRequest,
    answer: &SelectMediaAnswer,
) -> Result<(), Refusal> {
    engine::check_answer(
        &request.engine_request(),
        &engine::SelectEngineAnswer::new(
            &answer.decision_id,
            answer.expires_at_ms,
            answer.engine.clone(),
        ),
    )?;
    let key = format!("{}/{}", answer.engine.provider, answer.engine.model);
    if request.pin.as_ref().is_some_and(|pin| pin != &key)
        || request
            .exclude
            .as_ref()
            .is_some_and(|keys| keys.contains(&key))
        || request
            .exclude_providers
            .as_ref()
            .is_some_and(|keys| keys.contains(&answer.engine.provider))
        || request
            .eligible_routes
            .as_ref()
            .is_some_and(|routes| !routes.iter().any(|route| route.route_key() == key))
    {
        return Err(Refusal::invalid_request(
            "engine",
            "selected engine violates route policy",
        ));
    }
    if let Some(price) = &answer.price {
        price
            .check()
            .map_err(|e| Refusal::invalid_request("price", e))?;
    }
    Ok(())
}
