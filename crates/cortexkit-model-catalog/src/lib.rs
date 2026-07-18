//! Shared representation of the models.dev model catalog: types + parsing,
//! deliberately NO bundled data.
//!
//! Two CortexKit consumers read the catalog for different reasons — broca for
//! capabilities/limits on the serving path, astrocyte for pricing — and the
//! fleet rule is that they must parse the SAME shape so a catalog schema
//! drift cannot make them disagree silently. This crate is that shape.
//! Consumers bring their own snapshot bytes and own their derived stores.
//!
//! Money discipline: models.dev publishes dollar-per-million-token rates as
//! JSON decimal numbers. This crate converts them ONCE, at the parse
//! boundary, into exact integer NANODOLLARS per million tokens
//! ([`RateNanosPerMtok`]) via decimal string scaling — no float ever reaches
//! a consumer's money path. A rate the decimal cannot represent exactly in
//! nanodollars is a parse error, never a rounded guess. `None` = "no
//! published rate", which is NOT zero (free) — consumers must distinguish
//! them.

use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};
use serde_json::Value;

/// Integer nanodollars per million tokens. $3/M tokens = 3_000_000_000.
pub type RateNanosPerMtok = i64;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum CatalogParseError {
    Json(String),
    /// A cost number that cannot scale exactly to integer nanodollars
    /// (more than 9 fractional digits, out of range, or not a plain decimal).
    InexactRate {
        provider: String,
        model: String,
        field: &'static str,
        value: String,
    },
    /// A NEGATIVE rate. No catalog publishes one; a corrupted snapshot must
    /// fail loud here rather than flow into consumers' signed money paths.
    NegativeRate {
        provider: String,
        model: String,
        field: &'static str,
        value: String,
    },
}

impl std::fmt::Display for CatalogParseError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            CatalogParseError::Json(e) => write!(f, "catalog json: {e}"),
            CatalogParseError::InexactRate { provider, model, field, value } => write!(
                f,
                "catalog rate {provider}/{model}.{field} = {value} cannot scale exactly to nanodollars"
            ),
            CatalogParseError::NegativeRate { provider, model, field, value } => {
                write!(f, "catalog rate {provider}/{model}.{field} = {value} is negative")
            }
        }
    }
}

impl std::error::Error for CatalogParseError {}

/// The parsed catalog: provider id → provider entry.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct CatalogDoc {
    pub providers: BTreeMap<String, ProviderEntry>,
}

/// One provider, with the fields both consumers rely on. Unmodeled catalog
/// fields are preserved verbatim in `raw` (ingestion-only passthrough) so a
/// schema addition never silently drops data.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct ProviderEntry {
    pub id: String,
    pub name: Option<String>,
    pub api: Option<String>,
    pub npm: Option<String>,
    pub models: BTreeMap<String, ModelEntry>,
    pub raw: Value,
}

/// One model offered by a provider.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct ModelEntry {
    /// The wire model id (what goes in a request's `model` field).
    pub id: String,
    pub display_name: Option<String>,
    pub family: Option<String>,
    pub capabilities: Capabilities,
    pub limits: Limits,
    pub cost: CostSchedule,
    pub release_date: Option<String>,
    pub status: Option<String>,
    pub raw: Value,
}

/// Capability flags (mirrors the catalog's booleans).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
pub struct Capabilities {
    #[serde(default)]
    pub attachment: bool,
    #[serde(default)]
    pub reasoning: bool,
    #[serde(default)]
    pub temperature: bool,
    #[serde(default)]
    pub tool_call: bool,
}

/// Token limits (mirrors the catalog).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
pub struct Limits {
    #[serde(default)]
    pub context: Option<u64>,
    #[serde(default)]
    pub max_input: Option<u64>,
    #[serde(default)]
    pub max_output: Option<u64>,
}

/// Per-token-class rates in exact integer nanodollars per million tokens.
///
/// `None` = the catalog did not state a rate — NOT the same as zero (free).
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct CostSchedule {
    pub input: Option<RateNanosPerMtok>,
    pub output: Option<RateNanosPerMtok>,
    pub cache_read: Option<RateNanosPerMtok>,
    pub cache_write: Option<RateNanosPerMtok>,
    pub reasoning: Option<RateNanosPerMtok>,
    pub input_audio: Option<RateNanosPerMtok>,
    pub output_audio: Option<RateNanosPerMtok>,
    /// Context-size pricing tiers, ascending by `min_context`. Empty = flat.
    pub tiers: Vec<CostTier>,
}

/// One context-size pricing tier: rates that apply at/above `min_context`.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct CostTier {
    pub min_context: u64,
    pub input: Option<RateNanosPerMtok>,
    pub output: Option<RateNanosPerMtok>,
    pub cache_read: Option<RateNanosPerMtok>,
    pub cache_write: Option<RateNanosPerMtok>,
}

impl CatalogDoc {
    /// Parse a models.dev snapshot (the top-level `{ provider_id: {...} }`
    /// document). Unknown fields are preserved in `raw`, never dropped.
    pub fn parse(json: &str) -> Result<Self, CatalogParseError> {
        let root: Value =
            serde_json::from_str(json).map_err(|e| CatalogParseError::Json(e.to_string()))?;
        let obj = root
            .as_object()
            .ok_or_else(|| CatalogParseError::Json("top level is not an object".into()))?;
        let mut providers = BTreeMap::new();
        for (provider_id, entry) in obj {
            providers.insert(provider_id.clone(), parse_provider(provider_id, entry)?);
        }
        Ok(Self { providers })
    }

    pub fn model(
        &self,
        provider_id: &str,
        model_id: &str,
    ) -> Option<(&ProviderEntry, &ModelEntry)> {
        let provider = self.providers.get(provider_id)?;
        let model = provider.models.get(model_id)?;
        Some((provider, model))
    }
}

fn parse_provider(id: &str, entry: &Value) -> Result<ProviderEntry, CatalogParseError> {
    let mut models = BTreeMap::new();
    if let Some(model_map) = entry.get("models").and_then(Value::as_object) {
        for (model_id, model_entry) in model_map {
            models.insert(model_id.clone(), parse_model(id, model_id, model_entry)?);
        }
    }
    Ok(ProviderEntry {
        id: id.to_string(),
        name: entry.get("name").and_then(Value::as_str).map(String::from),
        api: entry.get("api").and_then(Value::as_str).map(String::from),
        npm: entry.get("npm").and_then(Value::as_str).map(String::from),
        models,
        raw: entry.clone(),
    })
}

fn parse_model(provider: &str, id: &str, entry: &Value) -> Result<ModelEntry, CatalogParseError> {
    let capabilities = Capabilities {
        attachment: flag(entry, "attachment"),
        reasoning: flag(entry, "reasoning"),
        temperature: flag(entry, "temperature"),
        tool_call: flag(entry, "tool_call"),
    };
    let limits = entry
        .get("limit")
        .map(|l| Limits {
            context: l.get("context").and_then(Value::as_u64),
            max_input: l.get("input").and_then(Value::as_u64),
            max_output: l.get("output").and_then(Value::as_u64),
        })
        .unwrap_or_default();
    let cost = match entry.get("cost") {
        None => CostSchedule::default(),
        Some(c) => parse_cost(provider, id, c)?,
    };
    Ok(ModelEntry {
        id: id.to_string(),
        display_name: entry.get("name").and_then(Value::as_str).map(String::from),
        family: entry
            .get("family")
            .and_then(Value::as_str)
            .map(String::from),
        capabilities,
        limits,
        cost,
        release_date: entry
            .get("release_date")
            .and_then(Value::as_str)
            .map(String::from),
        status: entry
            .get("status")
            .and_then(Value::as_str)
            .map(String::from),
        raw: entry.clone(),
    })
}

fn flag(entry: &Value, key: &str) -> bool {
    entry.get(key).and_then(Value::as_bool).unwrap_or(false)
}

fn parse_cost(
    provider: &str,
    model: &str,
    cost: &Value,
) -> Result<CostSchedule, CatalogParseError> {
    let convert = |field: &'static str, v: &Value| -> Result<RateNanosPerMtok, CatalogParseError> {
        let nanos = dollars_to_nanos(v).map_err(|value| CatalogParseError::InexactRate {
            provider: provider.to_string(),
            model: model.to_string(),
            field,
            value,
        })?;
        if nanos < 0 {
            return Err(CatalogParseError::NegativeRate {
                provider: provider.to_string(),
                model: model.to_string(),
                field,
                value: v.to_string(),
            });
        }
        Ok(nanos)
    };
    let rate = |field: &'static str| -> Result<Option<RateNanosPerMtok>, CatalogParseError> {
        match cost.get(field) {
            None | Some(Value::Null) => Ok(None),
            Some(v) => convert(field, v).map(Some),
        }
    };
    let mut tiers = Vec::new();
    if let Some(list) = cost.get("tiers").and_then(Value::as_array) {
        for tier in list {
            let trate =
                |field: &'static str| -> Result<Option<RateNanosPerMtok>, CatalogParseError> {
                    match tier.get(field) {
                        None | Some(Value::Null) => Ok(None),
                        Some(v) => convert(field, v).map(Some),
                    }
                };
            tiers.push(CostTier {
                min_context: tier
                    .get("context_over")
                    .or_else(|| tier.get("min_context"))
                    .and_then(Value::as_u64)
                    .unwrap_or(0),
                input: trate("input")?,
                output: trate("output")?,
                cache_read: trate("cache_read")?,
                cache_write: trate("cache_write")?,
            });
        }
        tiers.sort_by_key(|t| t.min_context);
    }
    Ok(CostSchedule {
        input: rate("input")?,
        output: rate("output")?,
        cache_read: rate("cache_read")?,
        cache_write: rate("cache_write")?,
        reasoning: rate("reasoning")?,
        input_audio: rate("input_audio")?,
        output_audio: rate("output_audio")?,
        tiers,
    })
}

/// Convert a catalog dollar rate (JSON number) to exact integer nanodollars
/// via DECIMAL STRING scaling — floats never do money arithmetic.
///
/// The JSON number's shortest-roundtrip decimal form is scaled by 10^9
/// exactly. Precision beyond nanodollars ROUNDS HALF-EVEN at this boundary:
/// real catalogs carry upstream float artifacts (models.dev publishes rates
/// like `0.8299999999999998` for an intended `0.83`), and the rounding error
/// is below the money resolution (< 0.5 nanodollar per million tokens).
/// The one dangerous case stays a loud error: a NONZERO rate that would
/// round to ZERO (e.g. `1e-10`) is rejected — rounding it would fabricate a
/// free model, the exact silent-$0 the fleet's money rules ban.
fn dollars_to_nanos(v: &Value) -> Result<RateNanosPerMtok, String> {
    let n = v.as_number().ok_or_else(|| v.to_string())?;
    decimal_str_to_nanos(&n.to_string()).ok_or_else(|| n.to_string())
}

fn decimal_str_to_nanos(s: &str) -> Option<RateNanosPerMtok> {
    // Split off an exponent (serde prints e.g. 1e-7 for tiny rates).
    let (mantissa, exp) = match s.find(['e', 'E']) {
        Some(idx) => {
            let exp: i32 = s[idx + 1..].parse().ok()?;
            (&s[..idx], exp)
        }
        None => (s, 0),
    };
    let negative = mantissa.starts_with('-');
    let mantissa = mantissa.trim_start_matches(['-', '+']);
    let (int_part, frac_part) = match mantissa.find('.') {
        Some(idx) => (&mantissa[..idx], &mantissa[idx + 1..]),
        None => (mantissa, ""),
    };
    if int_part.is_empty() && frac_part.is_empty() {
        return None;
    }
    if !int_part.chars().all(|c| c.is_ascii_digit())
        || !frac_part.chars().all(|c| c.is_ascii_digit())
    {
        return None;
    }
    // digits = int_part + frac_part, decimal point sits after int_part.len(),
    // then shift by exp. Effective fractional digits = frac.len() - exp.
    let digits: String = format!("{int_part}{frac_part}");
    let digits_trimmed = digits.trim_start_matches('0');
    let value: i128 = if digits_trimmed.is_empty() {
        0
    } else {
        digits_trimmed.parse().ok()?
    };
    // value × 10^(exp - frac_len) dollars → nanos = value × 10^(9 + exp - frac_len)
    let shift = 9 + exp - frac_part.len() as i32;
    let scaled = if shift >= 0 {
        value.checked_mul(10i128.checked_pow(shift as u32)?)?
    } else {
        // Sub-nanodollar digits: round half-even at the money resolution.
        // Upstream float artifacts ("0.8299999999999998") land here; the
        // error is < 0.5 nano per Mtok. A NONZERO value rounding to ZERO is
        // refused — that would fabricate a free model from a real price.
        let divisor = 10i128.checked_pow((-shift) as u32)?;
        let quot = value / divisor;
        let rem = value % divisor;
        let rounded = match (rem * 2).cmp(&divisor) {
            std::cmp::Ordering::Greater => quot + 1,
            std::cmp::Ordering::Less => quot,
            std::cmp::Ordering::Equal => {
                if quot % 2 == 0 {
                    quot
                } else {
                    quot + 1
                }
            }
        };
        if rounded == 0 && value != 0 {
            return None; // nonzero price must never become $0
        }
        rounded
    };
    let scaled = if negative { -scaled } else { scaled };
    RateNanosPerMtok::try_from(scaled).ok()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn decimal_scaling_is_exact() {
        assert_eq!(decimal_str_to_nanos("3"), Some(3_000_000_000));
        assert_eq!(decimal_str_to_nanos("3.75"), Some(3_750_000_000));
        assert_eq!(decimal_str_to_nanos("0.3"), Some(300_000_000));
        // The classic float trap: 0.1 + 0.2 style artifacts cannot arise —
        // "0.07" scales as digits, not as a binary float.
        assert_eq!(decimal_str_to_nanos("0.07"), Some(70_000_000));
        assert_eq!(decimal_str_to_nanos("0"), Some(0));
        // Exponent forms (serde prints tiny rates this way).
        assert_eq!(decimal_str_to_nanos("1e-7"), Some(100));
        assert_eq!(decimal_str_to_nanos("2.5e-3"), Some(2_500_000));
        // A nonzero rate that would round to ZERO stays an ERROR — rounding
        // it would fabricate a free model from a real price.
        assert_eq!(decimal_str_to_nanos("1e-10"), None);
        assert_eq!(decimal_str_to_nanos("0.0000000001"), None);
    }

    #[test]
    fn upstream_float_artifacts_round_half_even() {
        // Real models.dev data: IEEE-754 shortest-roundtrip artifacts from
        // the upstream pipeline. The intended decimal is recovered exactly.
        assert_eq!(
            decimal_str_to_nanos("0.8299999999999998"),
            Some(830_000_000)
        );
        assert_eq!(
            decimal_str_to_nanos("1.7999999999999998"),
            Some(1_800_000_000)
        );
        assert_eq!(
            decimal_str_to_nanos("0.49299999999999994"),
            Some(493_000_000)
        );
        // Half-even at the boundary digit: 10 fractional digits ...5 exact.
        assert_eq!(decimal_str_to_nanos("0.0000000015"), Some(2)); // 1.5 → 2 (even)
        assert_eq!(decimal_str_to_nanos("0.0000000025"), Some(2)); // 2.5 → 2 (even)
                                                                   // True zero stays zero (zero is not "rounded to zero").
        assert_eq!(decimal_str_to_nanos("0.0000000000"), Some(0));
    }

    fn snapshot() -> &'static str {
        r#"{
            "anthropic": {
                "name": "Anthropic",
                "api": "https://api.anthropic.com",
                "models": {
                    "claude-test-4": {
                        "name": "Claude Test 4",
                        "reasoning": true,
                        "tool_call": true,
                        "limit": { "context": 200000, "output": 64000 },
                        "cost": {
                            "input": 3, "output": 15,
                            "cache_read": 0.3, "cache_write": 3.75
                        }
                    },
                    "claude-unpriced": { "name": "No cost row" }
                }
            },
            "somehost": {
                "models": {
                    "tiered": {
                        "cost": {
                            "input": 1.25, "output": 10,
                            "tiers": [
                                { "context_over": 200000, "input": 2.5, "output": 15 }
                            ]
                        }
                    }
                }
            }
        }"#
    }

    #[test]
    fn parses_providers_models_rates() {
        let doc = CatalogDoc::parse(snapshot()).unwrap();
        let (provider, model) = doc.model("anthropic", "claude-test-4").unwrap();
        assert_eq!(provider.name.as_deref(), Some("Anthropic"));
        assert!(model.capabilities.reasoning);
        assert_eq!(model.limits.context, Some(200_000));
        assert_eq!(model.cost.input, Some(3_000_000_000));
        assert_eq!(model.cost.output, Some(15_000_000_000));
        assert_eq!(model.cost.cache_read, Some(300_000_000));
        assert_eq!(model.cost.cache_write, Some(3_750_000_000));
        // No published reasoning rate: None, NOT zero.
        assert_eq!(model.cost.reasoning, None);
    }

    #[test]
    fn missing_cost_block_is_all_none_not_zero() {
        let doc = CatalogDoc::parse(snapshot()).unwrap();
        let (_, model) = doc.model("anthropic", "claude-unpriced").unwrap();
        assert_eq!(model.cost, CostSchedule::default());
        assert_eq!(model.cost.input, None, "no rate is None, never $0");
    }

    #[test]
    fn tiers_parse_sorted() {
        let doc = CatalogDoc::parse(snapshot()).unwrap();
        let (_, model) = doc.model("somehost", "tiered").unwrap();
        assert_eq!(model.cost.tiers.len(), 1);
        assert_eq!(model.cost.tiers[0].min_context, 200_000);
        assert_eq!(model.cost.tiers[0].input, Some(2_500_000_000));
    }

    #[test]
    fn raw_passthrough_preserves_unmodeled_fields() {
        let doc =
            CatalogDoc::parse(r#"{ "p": { "future_field": {"x": 1}, "models": {} } }"#).unwrap();
        let provider = doc.providers.get("p").unwrap();
        assert_eq!(provider.raw.get("future_field").unwrap()["x"], 1);
    }

    #[test]
    fn negative_rate_is_a_loud_error() {
        // A corrupted snapshot's negative price must fail parse, not flow
        // silently into consumers' signed money paths.
        let err =
            CatalogDoc::parse(r#"{ "p": { "models": { "m": { "cost": { "output": -15 } } } } }"#)
                .unwrap_err();
        match err {
            CatalogParseError::NegativeRate {
                provider,
                model,
                field,
                ..
            } => {
                assert_eq!(
                    (provider.as_str(), model.as_str(), field),
                    ("p", "m", "output")
                );
            }
            other => panic!("expected NegativeRate, got {other:?}"),
        }
        // Tier rates are guarded by the same gate.
        let err = CatalogDoc::parse(
            r#"{ "p": { "models": { "m": { "cost": { "tiers": [ { "context_over": 1, "input": -1 } ] } } } } }"#,
        )
        .unwrap_err();
        assert!(matches!(err, CatalogParseError::NegativeRate { .. }));
    }

    #[test]
    fn inexact_rate_is_a_loud_error() {
        let err =
            CatalogDoc::parse(r#"{ "p": { "models": { "m": { "cost": { "input": 1e-10 } } } } }"#)
                .unwrap_err();
        match err {
            CatalogParseError::InexactRate {
                provider,
                model,
                field,
                ..
            } => {
                assert_eq!(
                    (provider.as_str(), model.as_str(), field),
                    ("p", "m", "input")
                );
            }
            other => panic!("expected InexactRate, got {other:?}"),
        }
    }
}
