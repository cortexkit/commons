//! The provider side: what a runner sends each provider for one item, and
//! what the provider answers. Both starting providers share one shape
//! (Cloudflare calls clef "fully Jev-API compatible"); `CONTRACT.md` cites
//! the documentation each shape comes from.
//!
//! One provider call carries one item's `state` with every question of the
//! batch. The runner maps the batch's catalog id to the provider's own model
//! value ([`models`]) and forwards `state`, the questions and the images as
//! the caller wrote them.

use indexmap::IndexMap;
use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::{
    errors::{INVALID_ITEM, PROVIDER_ERROR, RATE_LIMITED},
    question::{Image, Question},
    reply::{Answer, Usage},
};

/// Catalog ids of the starting providers' models.
pub mod catalog_ids {
    pub const CLEF: &str = "cloudflare/clef";
    pub const CLEF_FLASH: &str = "cloudflare/clef-flash";
    pub const JEV_LATEST: &str = "typesafe/jev-latest";
}

/// The `model` value each provider expects in its request body.
pub mod models {
    /// Cloudflare Workers AI `@cf/cloudflare/clef`.
    pub const CLEF: &str = "clef";
    /// Cloudflare Workers AI's faster clef variant.
    pub const CLEF_FLASH: &str = "clef-flash";
    /// TypeSafe's current model.
    pub const JEV_LATEST: &str = "jev-latest";
}

/// Provider hosts a `role.describe` names as `egress_host`.
pub mod hosts {
    pub const CLOUDFLARE: &str = "api.cloudflare.com";
    pub const TYPESAFE: &str = "api.typesafe.ai";
}

/// TypeSafe's classification endpoint.
pub const TYPESAFE_ENDPOINT: &str = "https://api.typesafe.ai/v1/systemone";

/// clef's limits on one request, from Cloudflare's documentation.
pub mod clef_limits {
    /// Fewest questions in one request.
    pub const MIN_QUESTIONS: usize = 1;
    /// Most questions in one request.
    pub const MAX_QUESTIONS: usize = 64;
    /// Longest question id; ids use letters, digits, `_`, `.` and `-`.
    pub const MAX_QUESTION_ID_LEN: usize = 100;
    /// Most images in one request.
    pub const MAX_IMAGES: usize = 4;
    /// Largest request body, in bytes (13 MiB).
    pub const MAX_BODY_BYTES: usize = 13 * 1024 * 1024;
}

/// The provider request for one item: `{model, state, questions, images?}`.
/// `images` is clef only; TypeSafe takes text only.
#[derive(Clone, Debug, PartialEq, Deserialize, Serialize)]
#[non_exhaustive]
pub struct ProviderRequest {
    /// The provider's own model value, one of [`models`].
    pub model: String,
    /// A string, or structured data (an object or array). The provider
    /// silently truncates a long state.
    pub state: Value,
    pub questions: IndexMap<String, Question>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub images: Option<Vec<Image>>,
}

impl ProviderRequest {
    pub fn new(
        model: impl Into<String>,
        state: impl Into<Value>,
        questions: IndexMap<String, Question>,
    ) -> Self {
        Self {
            model: model.into(),
            state: state.into(),
            questions,
            images: None,
        }
    }

    pub fn with_images(mut self, images: Vec<Image>) -> Self {
        self.images = Some(images);
        self
    }
}

/// The provider response for one item: `{model, answers, usage}`.
#[derive(Clone, Debug, PartialEq, Deserialize, Serialize)]
#[non_exhaustive]
pub struct ProviderResponse {
    pub model: String,
    pub answers: IndexMap<String, Answer>,
    /// Absent when the provider reported none; a count it left out stays
    /// absent.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub usage: Option<Usage>,
}

impl ProviderResponse {
    pub fn new(model: impl Into<String>, answers: IndexMap<String, Answer>) -> Self {
        Self {
            model: model.into(),
            answers,
            usage: None,
        }
    }

    pub fn with_usage(mut self, usage: Usage) -> Self {
        self.usage = Some(usage);
        self
    }
}

/// The per-item error code for a provider's HTTP status, once the runner's
/// own retries are spent. Neither provider documents an error body, so the
/// status is all the mapping reads:
///
/// - 429 (clef's 3036 account-limited and 3040 out-of-capacity, TypeSafe's
///   rate limit) and 529 (TypeSafe's Overloaded): `rate_limited`;
/// - 408 (clef's timeout) and every 5xx: `provider_error`;
/// - every other 4xx (clef's 400, 403, 404 and 413; TypeSafe's 401 and
///   422): `invalid_item`.
///
/// `None` for a status that is not an error. Neither provider documents a
/// content-refusal status, so nothing here maps to `provider_refused`.
pub fn item_code_for_status(status: u16) -> Option<&'static str> {
    match status {
        429 | 529 => Some(RATE_LIMITED),
        408 | 500..=599 => Some(PROVIDER_ERROR),
        400..=499 => Some(INVALID_ITEM),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn documented_statuses_map_to_item_codes() {
        for status in [429, 529] {
            assert_eq!(item_code_for_status(status), Some(RATE_LIMITED));
        }
        for status in [408, 500, 502, 503] {
            assert_eq!(item_code_for_status(status), Some(PROVIDER_ERROR));
        }
        for status in [400, 401, 403, 404, 413, 422] {
            assert_eq!(item_code_for_status(status), Some(INVALID_ITEM));
        }
        assert_eq!(item_code_for_status(200), None);
    }
}
