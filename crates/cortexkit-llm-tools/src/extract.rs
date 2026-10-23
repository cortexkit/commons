//! One extractor item, preserving the role's fields, options and raw answers.

use cortexkit_role_extractor::{
    errors::Refusal,
    field::{check_field, types, FieldType},
    request::is_calendar_date,
};
pub use cortexkit_role_extractor::{
    field::Field,
    reply::Answer,
    request::{Item, Options},
};
use indexmap::IndexMap;
use serde::{Deserialize, Serialize};
use serde_json::{Number, Value};

use crate::{non_negative, Usage, ValidationError};

pub const TAG: &str = "llm.extract/v1";

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Request {
    pub model: String,
    pub fields: IndexMap<String, Field>,
    pub item: Item,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub options: Option<Options>,
    /// Required per-call ceiling; the caller supplies its remaining budget.
    pub max_cost_usd: Number,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Result {
    pub answers: IndexMap<String, Answer>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub truncated: Option<bool>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub state_text: Option<String>,
    pub usage: Usage,
    pub cost_usd: Number,
}

/// Checks role invariants independent of a catalog row. Serving modules must also
/// apply the selected model's limits, supported field types and image policy.
pub fn validate_request(request: &Request) -> std::result::Result<(), ValidationError> {
    if request.model.is_empty() {
        return Err(ValidationError::EmptyModel);
    }
    if request.fields.is_empty() {
        return Err(ValidationError::Role(Refusal::invalid_params(
            "fields",
            "at least one field is required",
        )));
    }
    let served: Vec<_> = types::ALL
        .iter()
        .map(|name| FieldType::parse(name))
        .collect();
    for (id, field) in &request.fields {
        check_field(id, field, &served, u64::MAX).map_err(ValidationError::Role)?;
    }
    let has_images = request
        .item
        .images
        .as_ref()
        .is_some_and(|images| !images.is_empty());
    if request.item.state.is_empty() && !has_images {
        return Err(ValidationError::Role(Refusal::invalid_params(
            "item.state",
            "state is non-empty unless images are present",
        )));
    }
    if let Some(images) = &request.item.images {
        if images.is_empty() {
            return Err(ValidationError::Role(Refusal::invalid_params(
                "item.images",
                "images is non-empty when present",
            )));
        }
        for (i, image) in images.iter().enumerate() {
            cortexkit_model_batch::image::check_image(image).map_err(|message| {
                ValidationError::Role(Refusal::invalid_params(
                    format!("item.images[{i}]"),
                    message,
                ))
            })?;
        }
    }
    if request
        .item
        .reference_date
        .as_ref()
        .is_some_and(|date| !is_calendar_date(date))
    {
        return Err(ValidationError::Role(Refusal::invalid_params(
            "item.reference_date",
            "expected a real YYYY-MM-DD date",
        )));
    }
    non_negative(&request.max_cost_usd, "max_cost_usd")?;
    Ok(())
}

pub fn validate_result(result: &Result) -> std::result::Result<(), ValidationError> {
    non_negative(&result.cost_usd, "cost_usd")?;
    result.usage.validate()
}

pub fn input_schema() -> Value {
    crate::schema::extract_input()
}
pub fn result_schema() -> Value {
    crate::schema::role_result(true)
}
