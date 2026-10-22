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
    question::{Image, Question},
    reply::{Answer, Usage},
};

/// Catalog ids of the starting providers' models: models.dev's provider key,
/// then `/`, then models.dev's model id. The first `/` splits the two, so `@`
/// and any further `/` belong to the model id. Each pins a model version,
/// never a moving alias.
pub mod catalog_ids {
    pub const CLEF: &str = "cloudflare-workers-ai/@cf/cloudflare/clef";
    pub const CLEF_FLASH: &str = "cloudflare-workers-ai/@cf/cloudflare/clef-flash";
    pub const JEV_1_13_0: &str = "typesafe/jev-1.13.0";
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

// The mapping from a provider's error to a per-item code is the shared
// model-batch rules (`cortexkit-model-batch`, its `RULES.md` §15),
// re-exported here. TypeSafe documents no error codes, so its errors map by
// status (`item_code_for_status`). clef is a Cloudflare Workers AI model, so
// its errors map by Workers AI code first (`workers_ai_item_code`), and a
// code that table does not list falls back to the status.
pub use cortexkit_model_batch::status::{
    item_code_for_status, workers_ai_codes, workers_ai_item_code,
};

#[cfg(test)]
mod tests {
    use super::*;
    use crate::errors::{
        ACCOUNT_LIMIT_EXHAUSTED, ACCOUNT_NOT_ENTITLED, AUTH_FAILED, INVALID_ITEM,
        MODEL_UNAVAILABLE, PROVIDER_ERROR, RATE_LIMITED,
    };

    #[test]
    fn documented_statuses_map_to_item_codes() {
        for status in [429, 529] {
            assert_eq!(item_code_for_status(status), Some(RATE_LIMITED));
        }
        for status in [408, 500, 502, 503] {
            assert_eq!(item_code_for_status(status), Some(PROVIDER_ERROR));
        }
        for status in [400, 413, 422, 409] {
            assert_eq!(item_code_for_status(status), Some(INVALID_ITEM));
        }
        for status in [401, 403] {
            assert_eq!(item_code_for_status(status), Some(AUTH_FAILED));
        }
        assert_eq!(item_code_for_status(404), Some(MODEL_UNAVAILABLE));
        assert_eq!(item_code_for_status(200), None);
    }

    #[test]
    fn clef_errors_map_by_workers_ai_code_first() {
        assert_eq!(
            workers_ai_item_code(429, Some(3036)),
            Some(ACCOUNT_LIMIT_EXHAUSTED)
        );
        for code in [5035, 5016, 3023] {
            assert_eq!(
                workers_ai_item_code(403, Some(code)),
                Some(ACCOUNT_NOT_ENTITLED)
            );
        }
        assert_eq!(
            workers_ai_item_code(400, Some(5007)),
            Some(MODEL_UNAVAILABLE)
        );
        assert_eq!(workers_ai_item_code(429, Some(3040)), Some(RATE_LIMITED));
        assert_eq!(workers_ai_item_code(403, None), Some(AUTH_FAILED));
    }
}
