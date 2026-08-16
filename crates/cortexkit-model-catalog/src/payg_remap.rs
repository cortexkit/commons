use std::collections::BTreeMap;

use serde_json::Value;

use crate::{dollars_to_nanos, CostSchedule, CostTier, RateNanosPerMtok};

const COUNTERFACTUAL: &str = "same_platform_list";

/// An exact provider-qualified model identifier used by PAYG remap documents.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct PaygModelId(String);

impl PaygModelId {
    pub fn parse(id: &str) -> Result<Self, PaygRemapParseError> {
        let Some((provider, model)) = id.split_once('/') else {
            return Err(PaygRemapParseError::MalformedId { id: id.into() });
        };
        if provider.is_empty() || model.is_empty() {
            return Err(PaygRemapParseError::MalformedId { id: id.into() });
        }
        Ok(Self(id.into()))
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }

    pub fn provider(&self) -> &str {
        self.0
            .split_once('/')
            .map(|(provider, _)| provider)
            .expect("PaygModelId is validated by parse")
    }

    pub fn model(&self) -> &str {
        self.0
            .split_once('/')
            .map(|(_, model)| model)
            .expect("PaygModelId is validated by parse")
    }
}

/// One complete, parsed PAYG remap document.
///
/// ```rust,compile_fail
/// use cortexkit_model_catalog::PaygRemapDoc;
///
/// let parsed = PaygRemapDoc::parse("{}");
/// let _ = parsed.unwrap_or_default();
/// ```
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PaygRemapDoc {
    pub schema: u64,
    pub providers: BTreeMap<String, PaygProviderRule>,
    pub entries: BTreeMap<PaygModelId, PaygRemapEntry>,
}

impl PaygRemapDoc {
    pub fn parse(json: &str) -> Result<Self, PaygRemapParseError> {
        let root: Value = serde_json::from_str(json)
            .map_err(|error| PaygRemapParseError::Json(error.to_string()))?;
        let root = root
            .as_object()
            .ok_or_else(|| PaygRemapParseError::Json("top level is not an object".into()))?;

        let schema = root
            .get("schema")
            .and_then(Value::as_u64)
            .ok_or_else(|| PaygRemapParseError::Json("schema is not an unsigned integer".into()))?;
        if schema != 1 {
            return Err(PaygRemapParseError::UnknownSchema { schema });
        }

        let found = root
            .get("counterfactual")
            .and_then(Value::as_str)
            .map_or_else(|| "<missing>".into(), Into::into);
        if found != COUNTERFACTUAL {
            return Err(PaygRemapParseError::CounterfactualMismatch {
                expected: COUNTERFACTUAL,
                found,
            });
        }

        let providers = parse_provider_rules(
            root.get("providers")
                .ok_or_else(|| PaygRemapParseError::Json("providers is missing".into()))?,
        )?;
        let entries = parse_entries(
            root.get("entries")
                .ok_or_else(|| PaygRemapParseError::Json("entries is missing".into()))?,
        )?;

        for (id, entry) in &entries {
            if let PaygRemapEntry::ResolvesTo(resolve) = entry {
                if resolve.target == *id {
                    return Err(PaygRemapParseError::SelfReferentialTarget {
                        id: id.as_str().into(),
                    });
                }
                if entries.contains_key(&resolve.target) {
                    return Err(PaygRemapParseError::ChainedTarget {
                        id: id.as_str().into(),
                        target: resolve.target.as_str().into(),
                    });
                }
            }
        }

        Ok(Self {
            schema,
            providers,
            entries,
        })
    }
}

/// The only provider-wide PAYG refusal rule.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PaygProviderRuleKind {
    ZerosAreNotPrices,
}

/// A provider-scoped PAYG refusal rule.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PaygProviderRule {
    pub kind: PaygProviderRuleKind,
    pub id_prefix: Option<String>,
    pub source: String,
    pub observed: String,
}

/// A specific PAYG remap declaration.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PaygRemapEntry {
    ResolvesTo(ResolvesToEntry),
    OverridesUnpriced(OverridesUnpricedEntry),
    NotSoldPerToken(NotSoldPerTokenEntry),
}

/// A declaration that points at one terminal catalog schedule.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ResolvesToEntry {
    pub target: PaygModelId,
    pub because: String,
    pub source: String,
    pub observed: String,
}

/// A declaration that supplies a sourced schedule absent from the catalog.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct OverridesUnpricedEntry {
    pub cost: CostSchedule,
    pub source: String,
    pub observed: String,
}

/// A declaration that the platform has no per-token rate for this identifier.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NotSoldPerTokenEntry {
    pub reason: String,
    pub source: String,
    pub observed: String,
}

/// A PAYG remap-document parse failure.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PaygRemapParseError {
    Json(String),
    UnknownSchema {
        schema: u64,
    },
    CounterfactualMismatch {
        expected: &'static str,
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
        field: &'static str,
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
        field: &'static str,
        value: String,
    },
    NegativeRate {
        id: String,
        field: &'static str,
        value: String,
    },
    ContextBandNotRepresentable {
        id: String,
    },
    InvalidIdPrefix {
        id: String,
    },
}

impl std::fmt::Display for PaygRemapParseError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Json(error) => write!(f, "PAYG remap json: {error}"),
            Self::UnknownSchema { schema } => write!(f, "unknown PAYG remap schema {schema}"),
            Self::CounterfactualMismatch { expected, found } => {
                write!(
                    f,
                    "PAYG remap counterfactual is {found:?}, expected {expected:?}"
                )
            }
            Self::UnknownKind { id, kind } => {
                write!(f, "unknown PAYG remap kind {kind:?} for {id}")
            }
            Self::MalformedId { id } => write!(f, "malformed PAYG remap id {id:?}"),
            Self::MissingProvenance { id, field } => {
                write!(f, "PAYG remap declaration {id} is missing {field}")
            }
            Self::SelfReferentialTarget { id } => {
                write!(f, "PAYG remap declaration {id} resolves to itself")
            }
            Self::ChainedTarget { id, target } => {
                write!(
                    f,
                    "PAYG remap declaration {id} resolves through entry {target}"
                )
            }
            Self::ZeroOverride { id } => {
                write!(f, "PAYG remap override {id} does not supply a positive rate")
            }
            Self::InexactRate { id, field, value } => write!(
                f,
                "PAYG remap rate {id}.{field} = {value} cannot scale exactly to nanodollars"
            ),
            Self::NegativeRate { id, field, value } => {
                write!(f, "PAYG remap rate {id}.{field} = {value} is negative")
            }
            Self::ContextBandNotRepresentable { id } => write!(
                f,
                "PAYG remap override {id} uses context_over_200k; express that band through a tiers entry"
            ),
            Self::InvalidIdPrefix { id } => {
                write!(f, "PAYG provider rule {id} has a non-string id_prefix")
            }
        }
    }
}

impl std::error::Error for PaygRemapParseError {}

fn parse_provider_rules(
    value: &Value,
) -> Result<BTreeMap<String, PaygProviderRule>, PaygRemapParseError> {
    let rules = value
        .as_object()
        .ok_or_else(|| PaygRemapParseError::Json("providers is not an object".into()))?;
    let mut parsed = BTreeMap::new();
    for (id, value) in rules {
        let rule = value.as_object().ok_or_else(|| {
            PaygRemapParseError::Json(format!("provider rule {id} is not an object"))
        })?;
        let kind = required_string(rule, id, "kind")?;
        let kind = match kind.as_str() {
            "zeros_are_not_prices" => PaygProviderRuleKind::ZerosAreNotPrices,
            _ => {
                return Err(PaygRemapParseError::UnknownKind {
                    id: id.clone(),
                    kind,
                });
            }
        };
        let id_prefix = match rule.get("id_prefix") {
            None | Some(Value::Null) => None,
            Some(Value::String(prefix)) => Some(prefix.clone()),
            Some(_) => return Err(PaygRemapParseError::InvalidIdPrefix { id: id.clone() }),
        };
        parsed.insert(
            id.clone(),
            PaygProviderRule {
                kind,
                id_prefix,
                source: required_provenance(rule, id, "source")?,
                observed: required_provenance(rule, id, "observed")?,
            },
        );
    }
    Ok(parsed)
}

fn parse_entries(
    value: &Value,
) -> Result<BTreeMap<PaygModelId, PaygRemapEntry>, PaygRemapParseError> {
    let entries = value
        .as_object()
        .ok_or_else(|| PaygRemapParseError::Json("entries is not an object".into()))?;
    let mut parsed = BTreeMap::new();
    for (raw_id, value) in entries {
        let id = PaygModelId::parse(raw_id)?;
        let entry = value.as_object().ok_or_else(|| {
            PaygRemapParseError::Json(format!("PAYG remap entry {raw_id} is not an object"))
        })?;
        let source = required_provenance(entry, raw_id, "source")?;
        let observed = required_provenance(entry, raw_id, "observed")?;
        let kind = required_string(entry, raw_id, "kind")?;
        let entry = match kind.as_str() {
            "resolves_to" => PaygRemapEntry::ResolvesTo(ResolvesToEntry {
                target: PaygModelId::parse(&required_string(entry, raw_id, "target")?)?,
                because: required_string(entry, raw_id, "because")?,
                source,
                observed,
            }),
            "overrides_unpriced" => PaygRemapEntry::OverridesUnpriced(OverridesUnpricedEntry {
                cost: parse_override_cost(
                    raw_id,
                    entry
                        .get("cost")
                        .ok_or_else(|| missing_required_field(raw_id, "cost"))?,
                )?,
                source,
                observed,
            }),
            "not_sold_per_token" => PaygRemapEntry::NotSoldPerToken(NotSoldPerTokenEntry {
                reason: required_string(entry, raw_id, "reason")?,
                source,
                observed,
            }),
            _ => {
                return Err(PaygRemapParseError::UnknownKind {
                    id: raw_id.clone(),
                    kind,
                });
            }
        };
        parsed.insert(id, entry);
    }
    Ok(parsed)
}

fn required_provenance(
    entry: &serde_json::Map<String, Value>,
    id: &str,
    field: &'static str,
) -> Result<String, PaygRemapParseError> {
    entry
        .get(field)
        .and_then(Value::as_str)
        .filter(|value| !value.is_empty())
        .map(Into::into)
        .ok_or_else(|| PaygRemapParseError::MissingProvenance {
            id: id.into(),
            field,
        })
}

fn required_string(
    entry: &serde_json::Map<String, Value>,
    id: &str,
    field: &str,
) -> Result<String, PaygRemapParseError> {
    entry
        .get(field)
        .and_then(Value::as_str)
        .filter(|value| !value.is_empty())
        .map(Into::into)
        .ok_or_else(|| missing_required_field(id, field))
}

fn missing_required_field(id: &str, field: &str) -> PaygRemapParseError {
    PaygRemapParseError::Json(format!("PAYG remap declaration {id} is missing {field}"))
}

fn parse_override_cost(id: &str, value: &Value) -> Result<CostSchedule, PaygRemapParseError> {
    let cost = value.as_object().ok_or_else(|| {
        PaygRemapParseError::Json(format!(
            "PAYG remap override cost for {id} is not an object"
        ))
    })?;
    for field in cost.keys() {
        if field == "context_over_200k" {
            return Err(PaygRemapParseError::ContextBandNotRepresentable { id: id.into() });
        }
        if !matches!(
            field.as_str(),
            "input"
                | "output"
                | "cache_read"
                | "cache_write"
                | "reasoning"
                | "input_audio"
                | "output_audio"
                | "tiers"
        ) {
            return Err(PaygRemapParseError::Json(format!(
                "PAYG remap override cost for {id} has unknown field {field}"
            )));
        }
    }

    let schedule = CostSchedule {
        input: parse_rate(cost, id, "input")?,
        output: parse_rate(cost, id, "output")?,
        cache_read: parse_rate(cost, id, "cache_read")?,
        cache_write: parse_rate(cost, id, "cache_write")?,
        reasoning: parse_rate(cost, id, "reasoning")?,
        input_audio: parse_rate(cost, id, "input_audio")?,
        output_audio: parse_rate(cost, id, "output_audio")?,
        tiers: parse_tiers(cost, id)?,
    };
    if !has_positive_rate(&schedule) {
        return Err(PaygRemapParseError::ZeroOverride { id: id.into() });
    }
    Ok(schedule)
}

fn parse_tiers(
    cost: &serde_json::Map<String, Value>,
    id: &str,
) -> Result<Vec<CostTier>, PaygRemapParseError> {
    let Some(tiers) = cost.get("tiers") else {
        return Ok(Vec::new());
    };
    let tiers = tiers.as_array().ok_or_else(|| {
        PaygRemapParseError::Json(format!(
            "PAYG remap override tiers for {id} is not an array"
        ))
    })?;
    let mut parsed = Vec::with_capacity(tiers.len());
    for tier in tiers {
        let tier = tier.as_object().ok_or_else(|| {
            PaygRemapParseError::Json(format!(
                "PAYG remap override tier for {id} is not an object"
            ))
        })?;
        for field in tier.keys() {
            if !matches!(
                field.as_str(),
                "input" | "output" | "cache_read" | "cache_write" | "tier"
            ) {
                return Err(PaygRemapParseError::Json(format!(
                    "PAYG remap override tier for {id} has unknown field {field}"
                )));
            }
        }
        let dimension = tier.get("tier").and_then(Value::as_object).ok_or_else(|| {
            PaygRemapParseError::Json(format!("PAYG remap override tier for {id} lacks tier"))
        })?;
        if dimension.get("type").and_then(Value::as_str) != Some("context") {
            return Err(PaygRemapParseError::Json(format!(
                "PAYG remap override tier for {id} is not a context tier"
            )));
        }
        let min_context = dimension
            .get("size")
            .and_then(Value::as_u64)
            .ok_or_else(|| {
                PaygRemapParseError::Json(format!(
                    "PAYG remap override tier for {id} lacks tier.size"
                ))
            })?;
        parsed.push(CostTier {
            min_context,
            input: parse_rate(tier, id, "input")?,
            output: parse_rate(tier, id, "output")?,
            cache_read: parse_rate(tier, id, "cache_read")?,
            cache_write: parse_rate(tier, id, "cache_write")?,
        });
    }
    parsed.sort_by_key(|tier| tier.min_context);
    Ok(parsed)
}

fn parse_rate(
    fields: &serde_json::Map<String, Value>,
    id: &str,
    field: &'static str,
) -> Result<Option<RateNanosPerMtok>, PaygRemapParseError> {
    let Some(value) = fields.get(field) else {
        return Ok(None);
    };
    if value.is_null() {
        return Ok(None);
    }
    let rate = dollars_to_nanos(value).map_err(|value| PaygRemapParseError::InexactRate {
        id: id.into(),
        field,
        value,
    })?;
    if rate < 0 {
        return Err(PaygRemapParseError::NegativeRate {
            id: id.into(),
            field,
            value: value.to_string(),
        });
    }
    Ok(Some(rate))
}

/// §5.3's ALL-ZERO predicate for one parsed cost schedule.
///
/// At least one of `input` or `output` must be `Some(0)`, every present rate must be
/// `Some(0)`, and every tier rate must be zero. An all-`None` schedule is unpriced, not
/// zero; the leading `input`/`output` condition preserves that distinction.
pub fn is_all_zero(cost: &CostSchedule) -> bool {
    let direct = [
        cost.input,
        cost.output,
        cost.cache_read,
        cost.cache_write,
        cost.reasoning,
        cost.input_audio,
        cost.output_audio,
    ];
    (cost.input == Some(0) || cost.output == Some(0))
        && direct.into_iter().flatten().all(|rate| rate == 0)
        && cost.tiers.iter().all(|tier| {
            [tier.input, tier.output, tier.cache_read, tier.cache_write]
                .into_iter()
                .flatten()
                .all(|rate| rate == 0)
        })
}

fn has_positive_rate(cost: &CostSchedule) -> bool {
    let direct = [
        cost.input,
        cost.output,
        cost.cache_read,
        cost.cache_write,
        cost.reasoning,
        cost.input_audio,
        cost.output_audio,
    ];
    direct.into_iter().flatten().any(|rate| rate > 0)
        || cost.tiers.iter().any(|tier| {
            [tier.input, tier.output, tier.cache_read, tier.cache_write]
                .into_iter()
                .flatten()
                .any(|rate| rate > 0)
        })
}
