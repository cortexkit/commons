use serde::{Deserialize, Serialize};
use serde_json::{Map, Value};

use crate::{text, ValidationError};

pub const FACT_MALFORMED: &str = "fact_malformed";
pub const FACT_CONFLICT: &str = "fact_conflict";
pub const FACT_RANGE: &str = "fact_range";
pub const UNITS: &[&str] = &["count", "tokens", "bytes", "duration", "ratio"];
pub const STATES: &[&str] = &[
    "ready",
    "building",
    "paused",
    "stale",
    "unavailable",
    "failed",
    "off",
];
pub const TONES: &[&str] = &["neutral", "info", "success", "warning", "danger"];
pub const MAX_FACTS: usize = 16;

/// An unlabelled fact for a table cell. Strings stay open on decode; validation
/// rejects unfamiliar vocabulary so apps can show a neutral placeholder.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[non_exhaustive]
pub struct FactValue {
    #[serde(
        default,
        deserialize_with = "crate::present",
        skip_serializing_if = "Option::is_none"
    )]
    pub value: Option<f64>,
    #[serde(
        default,
        deserialize_with = "crate::present",
        skip_serializing_if = "Option::is_none"
    )]
    pub unit: Option<String>,
    #[serde(
        default,
        deserialize_with = "crate::present",
        skip_serializing_if = "Option::is_none"
    )]
    pub noun: Option<String>,
    #[serde(
        default,
        deserialize_with = "crate::present",
        skip_serializing_if = "Option::is_none"
    )]
    pub approx: Option<bool>,
    #[serde(
        default,
        deserialize_with = "crate::present",
        skip_serializing_if = "Option::is_none"
    )]
    pub state: Option<String>,
    #[serde(
        default,
        deserialize_with = "crate::present",
        skip_serializing_if = "Option::is_none"
    )]
    pub progress: Option<f64>,
    #[serde(
        default,
        deserialize_with = "crate::present",
        skip_serializing_if = "Option::is_none"
    )]
    pub tone: Option<String>,
    /// Retained only to report `fact_malformed`, never a producer extension.
    #[serde(flatten)]
    pub extra: Map<String, Value>,
}

impl FactValue {
    pub fn number(value: f64) -> Self {
        Self {
            value: Some(value),
            ..Self::default()
        }
    }
    pub fn state(state: impl Into<String>) -> Self {
        Self {
            state: Some(state.into()),
            ..Self::default()
        }
    }

    pub fn check(&self) -> Result<(), ValidationError> {
        if let Some(field) = self.extra.keys().next() {
            return Err(fact_error(FACT_MALFORMED, field));
        }
        for (field, value, allowed) in [
            ("unit", &self.unit, UNITS),
            ("state", &self.state, STATES),
            ("tone", &self.tone, TONES),
        ] {
            if value
                .as_ref()
                .is_some_and(|v| !allowed.contains(&v.as_str()))
            {
                return Err(fact_error(FACT_MALFORMED, field));
            }
        }
        if self.value.is_some() == self.state.is_some() {
            return Err(fact_error(FACT_CONFLICT, "value"));
        }
        if self.state.is_some() {
            for (field, present) in [
                ("unit", self.unit.is_some()),
                ("noun", self.noun.is_some()),
                ("approx", self.approx.is_some()),
                ("tone", self.tone.is_some()),
            ] {
                if present {
                    return Err(fact_error(FACT_CONFLICT, field));
                }
            }
        }
        if self.progress.is_some() && self.state.as_deref() != Some("building") {
            return Err(fact_error(FACT_CONFLICT, "progress"));
        }
        if self.noun.is_some() && self.unit.as_deref().unwrap_or("count") != "count" {
            return Err(fact_error(FACT_CONFLICT, "noun"));
        }
        if self.value.is_some_and(|v| !v.is_finite()) {
            return Err(fact_error(FACT_RANGE, "value"));
        }
        if self
            .progress
            .is_some_and(|v| !v.is_finite() || !(0.0..=1.0).contains(&v))
        {
            return Err(fact_error(FACT_RANGE, "progress"));
        }
        if self.noun.as_ref().is_some_and(|v| !text(v, 24)) {
            return Err(fact_error(FACT_RANGE, "noun"));
        }
        Ok(())
    }
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[non_exhaustive]
pub struct Fact {
    pub label: String,
    #[serde(flatten)]
    pub data: FactValue,
}

impl Fact {
    pub fn new(label: impl Into<String>, data: FactValue) -> Self {
        Self {
            label: label.into(),
            data,
        }
    }

    pub fn check(&self) -> Result<(), ValidationError> {
        if !text(&self.label, 64) {
            return Err(fact_error(FACT_RANGE, "label"));
        }
        self.data.check()
    }
}

fn fact_error(code: &str, field: &str) -> ValidationError {
    ValidationError::classified(code, field)
}

/// Validate raw JSON before typed decoding to classify unknown fields, missing
/// labels and mistyped values with the app library's diagnostic names.
pub fn validate_fact(body: &Value) -> Result<(), ValidationError> {
    validate_raw(body, true)
}

pub fn validate_fact_value(body: &Value) -> Result<(), ValidationError> {
    validate_raw(body, false)
}

fn validate_raw(body: &Value, labelled: bool) -> Result<(), ValidationError> {
    let object = body
        .as_object()
        .ok_or_else(|| fact_error(FACT_MALFORMED, "fact"))?;
    for field in object.keys() {
        if !matches!(
            field.as_str(),
            "value" | "unit" | "noun" | "approx" | "state" | "progress" | "tone"
        ) && !(labelled && field == "label")
        {
            return Err(fact_error(FACT_MALFORMED, field));
        }
    }
    if labelled {
        let label = object
            .get("label")
            .and_then(Value::as_str)
            .ok_or_else(|| fact_error(FACT_MALFORMED, "label"))?;
        if !text(label, 64) {
            return Err(fact_error(FACT_RANGE, "label"));
        }
    }
    // Presence, including null, must not disappear through Option decoding.
    for (field, value) in object {
        let valid_type = match field.as_str() {
            "value" | "progress" => value.is_number(),
            "approx" => value.is_boolean(),
            _ => value.is_string(),
        };
        if !valid_type {
            return Err(fact_error(FACT_MALFORMED, field));
        }
    }
    let mut value = object.clone();
    value.remove("label");
    let fact: FactValue = serde_json::from_value(Value::Object(value))
        .map_err(|_| fact_error(FACT_MALFORMED, "fact"))?;
    fact.check()
}

pub(crate) fn check_facts(facts: &Option<Vec<Fact>>) -> Result<(), ValidationError> {
    if let Some(facts) = facts {
        crate::require(facts.len() <= MAX_FACTS, "facts")?;
        for (index, fact) in facts.iter().enumerate() {
            fact.check().map_err(|e| e.at(&format!("facts.{index}")))?;
        }
    }
    Ok(())
}
