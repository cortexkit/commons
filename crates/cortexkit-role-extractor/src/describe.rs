//! `role.describe`: the module's limits and every model it serves.
//!
//! Resolved from the runner's catalog plus its own routing pins, so a
//! caller validates a manifest at install time against what this runner
//! will actually do, and can name the outside host its items are sent to.

use std::collections::BTreeSet;

use serde::{Deserialize, Serialize};
use serde_json::Number;

pub use cortexkit_model_batch::image::{ImageLimits, ImageReferenceSupport};

use crate::{field::FieldType, PROVIDES};

/// The answer to `role.describe`: `{role, limits, models, image_reference?}`.
#[derive(Clone, Debug, PartialEq, Deserialize, Serialize)]
#[non_exhaustive]
pub struct RoleDescribe {
    /// Always [`PROVIDES`], `extractor/v1`.
    pub role: String,
    pub limits: Limits,
    pub models: Vec<ModelEntry>,
    /// Present when the module resolves image references (`CONTRACT.md`
    /// §3); absent when it refuses them as unsupported images.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub image_reference: Option<ImageReferenceSupport>,
}

impl RoleDescribe {
    pub fn new(limits: Limits, models: Vec<ModelEntry>) -> Self {
        Self {
            role: PROVIDES.to_owned(),
            limits,
            models,
            image_reference: None,
        }
    }

    /// Declare that the module resolves image references.
    pub fn with_image_reference(mut self, support: ImageReferenceSupport) -> Self {
        self.image_reference = Some(support);
        self
    }

    /// Whether the module declares `image_reference`: a caller sends an
    /// image reference only when this is true.
    pub fn declares_image_reference(&self) -> bool {
        self.image_reference.is_some()
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
    /// Largest `extract.run` request body, in bytes.
    pub max_request_bytes: u64,
    /// Longest field `instructions`, in bytes. Clairic accepted 20,000
    /// characters; its own bound above that is not measured.
    pub max_instruction_bytes: u64,
}

impl Limits {
    pub fn new(max_items: u64, max_request_bytes: u64, max_instruction_bytes: u64) -> Self {
        Self {
            max_items,
            max_request_bytes,
            max_instruction_bytes,
        }
    }
}

/// One served model.
#[derive(Clone, Debug, PartialEq, Deserialize, Serialize)]
#[non_exhaustive]
pub struct ModelEntry {
    /// The catalog id, `<provider>/<model>`.
    pub model: String,
    /// Most items in one batch, never above `limits.max_items`.
    pub max_items: u64,
    /// Most fields in one batch: the module's cap (the provider states
    /// none).
    pub max_fields: u64,
    /// The types `extract.run` accepts for this model, exactly.
    pub field_types: Vec<FieldType>,
    /// The provider's per-pass token budget; fields beyond it run in more
    /// passes.
    pub pass_tokens: u64,
    /// The module's bound on one answer's encoded size, in bytes. The
    /// module never forwards a larger answer (`answer_too_large`), so the
    /// reply-size estimate (§3) is a real upper bound.
    pub max_answer_bytes: u64,
    /// The model's total context, in tokens; absent where the provider
    /// states no total. Absent is not zero.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub context_tokens: Option<u64>,
    /// The model's image limits per item; `null` when it takes no images.
    /// Always encoded, never omitted.
    pub images: Option<ImageLimits>,
    /// The catalog price; absent when the model is unpriced.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub price: Option<Price>,
    /// The provider host item text is sent to.
    pub egress_host: String,
}

impl ModelEntry {
    pub fn new(
        model: impl Into<String>,
        max_items: u64,
        max_fields: u64,
        field_types: Vec<FieldType>,
        pass_tokens: u64,
        max_answer_bytes: u64,
        egress_host: impl Into<String>,
    ) -> Self {
        Self {
            model: model.into(),
            max_items,
            max_fields,
            field_types,
            pass_tokens,
            max_answer_bytes,
            context_tokens: None,
            images: None,
            price: None,
            egress_host: egress_host.into(),
        }
    }

    pub fn with_context_tokens(mut self, context_tokens: u64) -> Self {
        self.context_tokens = Some(context_tokens);
        self
    }

    pub fn with_images(mut self, images: ImageLimits) -> Self {
        self.images = Some(images);
        self
    }

    pub fn with_price(mut self, price: Price) -> Self {
        self.price = Some(price);
        self
    }

    /// Whether the catalog prices input tokens for this model. Output is
    /// stated free for the starting provider, and a cost is input-priced.
    pub fn is_priced(&self) -> bool {
        self.price
            .as_ref()
            .is_some_and(|price| price.input_per_mtok_usd.is_some())
    }
}

/// A catalog price, in USD per million tokens. A direction the catalog
/// does not price is absent; a direction it states as free is `0`.
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
    /// `role` is not `extractor/v1`.
    WrongRole(String),
    /// The answer lists no model.
    NoModels,
    /// A model is listed twice.
    DuplicateModel(String),
    /// A model states a zero limit or context, an empty, unknown or
    /// repeated field type, image limits it cannot meet, no egress host, or
    /// more items than the module allows. Names the model and member.
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
    for (member, value) in [
        ("max_items", describe.limits.max_items),
        ("max_request_bytes", describe.limits.max_request_bytes),
        (
            "max_instruction_bytes",
            describe.limits.max_instruction_bytes,
        ),
    ] {
        if value == 0 {
            return Err(DescribeProblem::BadLimits(member));
        }
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
        if entry.max_items == 0 || entry.max_items > describe.limits.max_items {
            return bad("max_items");
        }
        if entry.max_fields == 0 {
            return bad("max_fields");
        }
        let mut types = BTreeSet::new();
        if entry.field_types.is_empty()
            || !entry
                .field_types
                .iter()
                .all(|kind| kind.is_known() && types.insert(kind.as_str()))
        {
            return bad("field_types");
        }
        if entry.pass_tokens == 0 {
            return bad("pass_tokens");
        }
        if entry.max_answer_bytes == 0 {
            return bad("max_answer_bytes");
        }
        if entry.context_tokens == Some(0) {
            return bad("context_tokens");
        }
        if let Some(images) = &entry.images {
            let formats_known = !images.formats.is_empty()
                && images.formats.iter().all(|format| {
                    cortexkit_model_batch::image::content_types::ALL.contains(&format.as_str())
                });
            if images.max == 0
                || images.max_bytes_each == 0
                || images.max_bytes_total < images.max_bytes_each
                || !formats_known
            {
                return bad("images");
            }
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

    #[test]
    fn the_image_reference_group_is_stated_only_when_declared() {
        let describe = RoleDescribe::new(
            Limits::new(10, 1 << 20, 100),
            vec![ModelEntry::new(
                "m/x",
                1,
                1,
                vec![FieldType::Text],
                8192,
                100,
                "host.invalid",
            )],
        );
        let plain = serde_json::to_string(&describe).unwrap();
        assert!(!plain.contains("image_reference"), "{plain}");
        let decoded: RoleDescribe = serde_json::from_str(&plain).unwrap();
        assert!(!decoded.declares_image_reference());
        let declared = describe.with_image_reference(ImageReferenceSupport::new());
        let text = serde_json::to_string(&declared).unwrap();
        assert!(text.ends_with(r#","image_reference":{}}"#), "{text}");
        assert_eq!(check_describe(&declared), Ok(()));
        let decoded: RoleDescribe = serde_json::from_str(&text).unwrap();
        assert!(decoded.declares_image_reference());
    }
}
