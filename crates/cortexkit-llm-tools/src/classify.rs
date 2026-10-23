//! One classifier item, with the role's questions and answers unchanged.

use cortexkit_role_classifier::{errors::Refusal, question::check_question, request::is_state};
pub use cortexkit_role_classifier::{question::Question, reply::Answer, request::Item};
use indexmap::IndexMap;
use serde::{Deserialize, Serialize};
use serde_json::{Number, Value};

use crate::{non_negative, Usage, ValidationError};

pub const TAG: &str = "llm.classify/v1";

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Request {
    pub model: String,
    pub questions: IndexMap<String, Question>,
    pub item: Item,
    /// Required per-call ceiling; the caller supplies its remaining budget.
    pub max_cost_usd: Number,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Result {
    pub answers: IndexMap<String, Answer>,
    pub usage: Usage,
    pub cost_usd: Number,
}

/// Catalog-specific limits and model kind still require the role's admission checks.
pub fn validate_request(request: &Request) -> std::result::Result<(), ValidationError> {
    if request.model.is_empty() {
        return Err(ValidationError::EmptyModel);
    }
    if request.questions.is_empty() {
        return Err(ValidationError::Role(Refusal::invalid_params(
            "questions",
            "at least one question is required",
        )));
    }
    for (id, question) in &request.questions {
        check_question(id, question).map_err(ValidationError::Role)?;
    }
    if !is_state(&request.item.state) {
        return Err(ValidationError::Role(Refusal::invalid_params(
            "item.state",
            "state is a string, object or array",
        )));
    }
    if let Some(images) = &request.item.images {
        for (i, image) in images.iter().enumerate() {
            cortexkit_model_batch::image::check_image(image).map_err(|message| {
                ValidationError::Role(Refusal::invalid_params(
                    format!("item.images[{i}]"),
                    message,
                ))
            })?;
        }
    }
    non_negative(&request.max_cost_usd, "max_cost_usd")?;
    Ok(())
}

pub fn validate_result(result: &Result) -> std::result::Result<(), ValidationError> {
    non_negative(&result.cost_usd, "cost_usd")?;
    result.usage.validate()
}

pub fn input_schema() -> Value {
    crate::schema::classify_input()
}
pub fn result_schema() -> Value {
    crate::schema::role_result(false)
}
