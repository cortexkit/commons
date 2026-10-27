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
/// The three input counts are disjoint. Their sum, each where present, is the
/// reported part of total input; an omitted component remains unknown, not zero.
/// Serving modules subtract cache counts included in a provider's
/// prompt or input count and count duplicate names for the same writes once.
/// Consumers price the three fields separately, never subtracting one from another.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Usage {
    /// Input neither read from nor written to the prompt cache; excludes
    /// `cached_input_tokens` and `cache_write_tokens`.
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

/// Check a tool item's images, naming `item.images[i]`. A reference
/// `{handle}` is checked by its handle's grammar when `references` is set,
/// for a provider that resolves references; otherwise it is refused as an
/// unsupported image, as the role refuses it on a module that does not
/// declare `image_reference`. Every other image gets the role's image check.
pub(crate) fn check_item_images(
    images: &[cortexkit_model_batch::image::Image],
    references: bool,
) -> Result<(), ValidationError> {
    for (i, image) in images.iter().enumerate() {
        let checked = match image.reference() {
            Some(reference) if references => reference.check(),
            _ => cortexkit_model_batch::image::check_image(image),
        };
        checked.map_err(|message| {
            ValidationError::Role(cortexkit_model_batch::errors::Refusal::invalid_params(
                format!("item.images[{i}]"),
                message,
            ))
        })?;
    }
    Ok(())
}
