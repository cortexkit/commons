//! `role.describe`: the module's limits and every model it serves.
//!
//! Resolved from the runner's catalog plus its own routing pins, so a
//! caller validates a manifest at install time against what this runner
//! will actually do, and can name the outside host its items are sent to.

use std::collections::BTreeSet;

use serde::{Deserialize, Serialize};
use serde_json::Number;

use crate::PROVIDES;

/// The answer to `role.describe`: `{role, limits, models}`.
#[derive(Clone, Debug, PartialEq, Deserialize, Serialize)]
#[non_exhaustive]
pub struct RoleDescribe {
    /// Always [`PROVIDES`], `classifier/v1`.
    pub role: String,
    pub limits: Limits,
    pub models: Vec<ModelEntry>,
}

impl RoleDescribe {
    pub fn new(limits: Limits, models: Vec<ModelEntry>) -> Self {
        Self {
            role: PROVIDES.to_owned(),
            limits,
            models,
        }
    }

    /// The entry for `model`, if this runner serves it.
    pub fn model(&self, model: &str) -> Option<&ModelEntry> {
        self.models.iter().find(|entry| entry.model == model)
    }
}

/// The module's own limits, across every model.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Deserialize, Serialize)]
#[non_exhaustive]
pub struct Limits {
    /// Most items in one batch.
    pub max_items: u64,
    /// Largest `classify.run` request body, in bytes.
    pub max_request_bytes: u64,
}

impl Limits {
    pub fn new(max_items: u64, max_request_bytes: u64) -> Self {
        Self {
            max_items,
            max_request_bytes,
        }
    }
}

/// One served model.
#[derive(Clone, Debug, PartialEq, Deserialize, Serialize)]
#[non_exhaustive]
pub struct ModelEntry {
    /// The catalog id, `<provider>/<model>`.
    pub model: String,
    /// Most questions in one batch: the tighter of the catalog's and the
    /// module's limits.
    pub max_questions: u64,
    /// Most items in one batch: the tighter of the catalog's and the
    /// module's limits, so never above `limits.max_items`.
    pub max_items: u64,
    /// The model's context, in tokens, from the catalog. The provider
    /// truncates `state` beyond it without saying so.
    pub context_tokens: u64,
    /// The model's image support; `null` when it takes no images. Always
    /// encoded, never omitted, so "no images" is stated rather than implied.
    pub images: Option<ImageSupport>,
    /// The catalog price; absent when the model is unpriced.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub price: Option<Price>,
    /// The provider host item text is sent to, e.g. `api.typesafe.ai`.
    pub egress_host: String,
}

impl ModelEntry {
    pub fn new(
        model: impl Into<String>,
        max_questions: u64,
        max_items: u64,
        context_tokens: u64,
        egress_host: impl Into<String>,
    ) -> Self {
        Self {
            model: model.into(),
            max_questions,
            max_items,
            context_tokens,
            images: None,
            price: None,
            egress_host: egress_host.into(),
        }
    }

    pub fn with_images(mut self, images: ImageSupport) -> Self {
        self.images = Some(images);
        self
    }

    pub fn with_price(mut self, price: Price) -> Self {
        self.price = Some(price);
        self
    }

    /// Whether the model has a price for at least one token direction.
    pub fn is_priced(&self) -> bool {
        self.price.as_ref().is_some_and(|price| {
            price.input_per_mtok_usd.is_some() || price.output_per_mtok_usd.is_some()
        })
    }
}

/// A model's image support: at most `max` images per item.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Deserialize, Serialize)]
#[non_exhaustive]
pub struct ImageSupport {
    pub max: u64,
}

impl ImageSupport {
    pub fn new(max: u64) -> Self {
        Self { max }
    }
}

/// A catalog price, in USD per million tokens. A direction the catalog does
/// not price is absent, never zero.
#[derive(Clone, Debug, Default, PartialEq, Deserialize, Serialize)]
#[non_exhaustive]
pub struct Price {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub input_per_mtok_usd: Option<Number>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub output_per_mtok_usd: Option<Number>,
}

impl Price {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn with_input_per_mtok_usd(mut self, usd: Number) -> Self {
        self.input_per_mtok_usd = Some(usd);
        self
    }

    pub fn with_output_per_mtok_usd(mut self, usd: Number) -> Self {
        self.output_per_mtok_usd = Some(usd);
        self
    }
}

/// Why a `role.describe` answer is not usable.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum DescribeProblem {
    /// `role` is not `classifier/v1`.
    WrongRole(String),
    /// The answer lists no model.
    NoModels,
    /// A model is listed twice.
    DuplicateModel(String),
    /// A model states a zero limit or context, names no egress host, or
    /// allows more items than the module does. Names the model and member.
    BadModel { model: String, member: &'static str },
    /// A module limit is zero.
    BadLimits(&'static str),
}

impl std::fmt::Display for DescribeProblem {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::WrongRole(role) => write!(f, "role is {role}, not {PROVIDES}"),
            Self::NoModels => f.write_str("models is empty"),
            Self::DuplicateModel(model) => write!(f, "{model} is listed twice"),
            Self::BadModel { model, member } => write!(f, "{model}: bad {member}"),
            Self::BadLimits(member) => write!(f, "limits.{member} is zero"),
        }
    }
}

/// Check a `role.describe` answer before relying on it.
pub fn check_describe(describe: &RoleDescribe) -> Result<(), DescribeProblem> {
    if describe.role != PROVIDES {
        return Err(DescribeProblem::WrongRole(describe.role.clone()));
    }
    if describe.limits.max_items == 0 {
        return Err(DescribeProblem::BadLimits("max_items"));
    }
    if describe.limits.max_request_bytes == 0 {
        return Err(DescribeProblem::BadLimits("max_request_bytes"));
    }
    if describe.models.is_empty() {
        return Err(DescribeProblem::NoModels);
    }
    let mut seen = BTreeSet::new();
    for entry in &describe.models {
        if !seen.insert(entry.model.as_str()) {
            return Err(DescribeProblem::DuplicateModel(entry.model.clone()));
        }
        let bad = |member| {
            Err(DescribeProblem::BadModel {
                model: entry.model.clone(),
                member,
            })
        };
        if entry.model.is_empty() {
            return bad("model");
        }
        if entry.max_questions == 0 {
            return bad("max_questions");
        }
        if entry.max_items == 0 || entry.max_items > describe.limits.max_items {
            return bad("max_items");
        }
        if entry.context_tokens == 0 {
            return bad("context_tokens");
        }
        if entry.images.is_some_and(|images| images.max == 0) {
            return bad("images");
        }
        if entry.egress_host.is_empty() {
            return bad("egress_host");
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn describe() -> RoleDescribe {
        RoleDescribe::new(
            Limits::new(100, 1 << 20),
            vec![
                ModelEntry::new("cloudflare/clef", 64, 100, 8192, "api.cloudflare.com")
                    .with_images(ImageSupport::new(4)),
                ModelEntry::new("typesafe/jev-latest", 32, 50, 8192, "api.typesafe.ai"),
            ],
        )
    }

    #[test]
    fn a_complete_answer_passes() {
        assert_eq!(check_describe(&describe()), Ok(()));
    }

    #[test]
    fn missing_images_encodes_as_null() {
        let text = serde_json::to_string(&describe().models[1]).unwrap();
        assert!(text.contains(r#""images":null"#), "{text}");
        assert!(!text.contains("price"), "{text}");
    }

    #[test]
    fn a_model_allowing_more_items_than_the_module_is_refused() {
        let mut answer = describe();
        answer.models[1].max_items = 101;
        assert_eq!(
            check_describe(&answer),
            Err(DescribeProblem::BadModel {
                model: "typesafe/jev-latest".into(),
                member: "max_items"
            })
        );
        let mut answer = describe();
        answer.models[0].context_tokens = 0;
        assert!(matches!(
            check_describe(&answer),
            Err(DescribeProblem::BadModel {
                member: "context_tokens",
                ..
            })
        ));
    }
}
