//! Single-item LLM tool contracts. No provider calls, routing, grants or billing.
//! See `CONTRACT.md` for the serving module's obligations beyond validation.

pub mod classify;
pub mod complete;
pub mod errors;
pub mod extract;
mod schema;
#[cfg(feature = "test-vectors")]
pub mod vectors;

use serde::{Deserialize, Serialize};
use serde_json::Number;

/// The only invocation modes advertised by these tools, never direct agent tools.
pub const MODES: &[&str] = &["codemode", "flow"];

/// Provider measurements only. Unknown counts are omitted, not fabricated as zero.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Usage {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub input_tokens: Option<u64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub cached_input_tokens: Option<u64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub cache_write_tokens: Option<u64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub output_tokens: Option<u64>,
    /// Included in output_tokens, not added to it.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub reasoning_tokens: Option<u64>,
}

impl Usage {
    pub fn validate(&self) -> Result<(), ValidationError> {
        if let Some(reasoning) = self.reasoning_tokens {
            match self.output_tokens {
                Some(output) if reasoning <= output => {}
                _ => return Err(ValidationError::ReasoningNotInOutput),
            }
        }
        Ok(())
    }
}

/// Named structural/semantic validation failures, not a second provider vocabulary.
#[derive(Clone, Debug, PartialEq)]
pub enum ValidationError {
    DemandOutOfRange {
        field: &'static str,
    },
    EmptyPrompt,
    EmptyModel,
    ZeroMaxOutputTokens,
    OutputSchemaTooLarge {
        size: usize,
        limit: usize,
    },
    NegativeCost {
        field: &'static str,
    },
    OutputCount,
    OutputModeMismatch,
    ReasoningNotInOutput,
    /// The original role refusal, including its code and field path.
    Role(cortexkit_model_batch::errors::Refusal),
}

impl std::fmt::Display for ValidationError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{self:?}")
    }
}

impl std::error::Error for ValidationError {}

pub(crate) fn non_negative(value: &Number, field: &'static str) -> Result<(), ValidationError> {
    if value.as_f64().is_some_and(|n| n < 0.0) {
        Err(ValidationError::NegativeCost { field })
    } else {
        Ok(())
    }
}
