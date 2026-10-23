//! Text-input, demand-routed completion; the caller never selects a model.

use serde::{Deserialize, Deserializer, Serialize};
use serde_json::{Map, Number, Value};

use crate::{non_negative, Usage, ValidationError};

pub const TAG: &str = "llm.complete/v1";
pub const OUTPUT_SCHEMA_MAX_BYTES: usize = 16 * 1024;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Demand {
    pub iq: i64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub eq: Option<i64>,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Request {
    pub demand: Demand,
    pub prompt: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub system: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub output_schema: Option<Map<String, Value>>,
    pub max_output_tokens: u64,
    /// Required; the caller supplies its remaining budget.
    pub max_cost_usd: Number,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Model {
    pub provider: String,
    pub model: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub variant: Option<String>,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Result {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub text: Option<String>,
    // JSON null is a present answer, unlike an absent json member.
    #[serde(
        default,
        skip_serializing_if = "Option::is_none",
        deserialize_with = "present_json"
    )]
    pub json: Option<Value>,
    pub model: Model,
    pub usage: Usage,
    pub cost_usd: Number,
}

fn present_json<'de, D: Deserializer<'de>>(
    deserializer: D,
) -> std::result::Result<Option<Value>, D::Error> {
    Value::deserialize(deserializer).map(Some)
}

pub fn validate_request(request: &Request) -> std::result::Result<(), ValidationError> {
    for (field, value) in [
        ("demand.iq", Some(request.demand.iq)),
        ("demand.eq", request.demand.eq),
    ] {
        if value.is_some_and(|n| !(0..=100).contains(&n)) {
            return Err(ValidationError::DemandOutOfRange { field });
        }
    }
    if request.prompt.is_empty() {
        return Err(ValidationError::EmptyPrompt);
    }
    if request.max_output_tokens == 0 {
        return Err(ValidationError::ZeroMaxOutputTokens);
    }
    non_negative(&request.max_cost_usd, "max_cost_usd")?;
    if let Some(schema) = &request.output_schema {
        let size = serde_json::to_vec(schema)
            .expect("JSON objects serialize")
            .len();
        if size > OUTPUT_SCHEMA_MAX_BYTES {
            return Err(ValidationError::OutputSchemaTooLarge {
                size,
                limit: OUTPUT_SCHEMA_MAX_BYTES,
            });
        }
    }
    Ok(())
}

pub fn validate_result(result: &Result) -> std::result::Result<(), ValidationError> {
    if result.text.is_some() == result.json.is_some() {
        return Err(ValidationError::OutputCount);
    }
    if result.model.provider.is_empty() || result.model.model.is_empty() {
        return Err(ValidationError::EmptyModel);
    }
    non_negative(&result.cost_usd, "cost_usd")?;
    result.usage.validate()
}

/// Checks the output branch against the request. The serving module must also
/// validate the JSON value against output_schema with its JSON Schema engine.
pub fn validate_result_for_request(
    request: &Request,
    result: &Result,
) -> std::result::Result<(), ValidationError> {
    validate_request(request)?;
    validate_result(result)?;
    if request.output_schema.is_some() != result.json.is_some() {
        return Err(ValidationError::OutputModeMismatch);
    }
    Ok(())
}

pub fn input_schema() -> Value {
    crate::schema::complete_input()
}
pub fn result_schema() -> Value {
    crate::schema::complete_result()
}
