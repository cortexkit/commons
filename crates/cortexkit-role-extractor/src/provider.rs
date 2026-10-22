//! The provider side for Cloudflare Workers AI Clairic: what a runner sends
//! for one item, the reply envelope it reads, and how provider errors map
//! to item codes. `CONTRACT.md` §9 cites the documentation each shape comes
//! from, and §12 lists what it leaves unmeasured.
//!
//! One provider call carries one item with every field of the batch, passed
//! through key by key as the caller wrote them, and the batch options under
//! the provider's own names.

use indexmap::IndexMap;
use serde::{Deserialize, Serialize};
use serde_json::Value;

pub use cortexkit_model_batch::status::{
    item_code_for_status, workers_ai_codes, workers_ai_item_code,
};

use crate::{
    errors::Refusal,
    field::Field,
    image::Image,
    reply::{Answer, Usage},
    request::ExtractRequest,
};

/// Catalog ids of the starting provider's models.
pub mod catalog_ids {
    pub const CLAIRIC: &str = "cloudflare-workers-ai/@cf/cloudflare/clairic";
}

/// The `model` value the provider expects in its request body.
pub mod models {
    pub const CLAIRIC: &str = "@cf/cloudflare/clairic";
}

/// Provider hosts a `role.describe` names as `egress_host`.
pub mod hosts {
    pub const CLOUDFLARE: &str = "api.cloudflare.com";
}

/// Clairic's limits, from Cloudflare's documentation.
pub mod clairic_limits {
    /// The token budget of one pass; fields beyond it run in more passes.
    pub const PASS_TOKENS: u64 = 8192;
    /// Largest request body, in bytes (16 MiB).
    pub const MAX_BODY_BYTES: u64 = 16 * 1024 * 1024;
    /// Largest image, in decoded bytes (4 MiB).
    pub const MAX_IMAGE_BYTES: u64 = 4 * 1024 * 1024;
    /// Largest total of an item's images, in decoded bytes (8 MiB).
    pub const MAX_IMAGES_TOTAL_BYTES: u64 = 8 * 1024 * 1024;
}

/// The URL a runner posts one item to, for the Cloudflare account
/// `account_id`, with `Authorization: Bearer <scoped Workers AI token>`.
pub fn clairic_endpoint(account_id: &str) -> String {
    format!(
        "https://{}/client/v4/accounts/{account_id}/ai/run/{}",
        hosts::CLOUDFLARE,
        models::CLAIRIC
    )
}

/// The provider's batch options: `{include_quantiles, return_state_text}`,
/// both always sent.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Deserialize, Serialize)]
#[non_exhaustive]
pub struct ProviderOptions {
    /// The batch's `options.quantiles`.
    pub include_quantiles: bool,
    /// The batch's `options.state_text`.
    pub return_state_text: bool,
}

/// The provider request for one item: `{model, state, images?,
/// reference_date?, fields, options}`.
#[derive(Clone, Debug, PartialEq, Deserialize, Serialize)]
#[non_exhaustive]
pub struct ProviderRequest {
    /// The provider's own model value, [`models::CLAIRIC`].
    pub model: String,
    pub state: String,
    /// Forwarded as the caller wrote them.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub images: Option<Vec<Image>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub reference_date: Option<String>,
    /// The batch's fields, key by key, in the caller's order.
    pub fields: IndexMap<String, Field>,
    pub options: ProviderOptions,
}

impl ProviderRequest {
    /// The provider request for item `index` of `request`, sent to
    /// `provider_model`. `None` when there is no such item.
    pub fn for_item(
        request: &ExtractRequest,
        index: usize,
        provider_model: impl Into<String>,
    ) -> Option<Self> {
        let item = request.items.get(index)?;
        let options = request.effective_options();
        Some(Self {
            model: provider_model.into(),
            state: item.state.clone(),
            images: item.images.clone(),
            reference_date: item.reference_date.clone(),
            fields: request.fields.clone(),
            options: ProviderOptions {
                include_quantiles: options.quantiles,
                return_state_text: options.state_text,
            },
        })
    }
}

/// Refuse a batch any of whose provider requests would be larger than the
/// provider accepts (`max_body_bytes`; Clairic's is
/// [`clairic_limits::MAX_BODY_BYTES`]), before any provider call:
/// `invalid_params` naming `items[i]` and the limit.
pub fn check_provider_bodies(request: &ExtractRequest, max_body_bytes: u64) -> Result<(), Refusal> {
    for index in 0..request.items.len() {
        let body = ProviderRequest::for_item(request, index, models::CLAIRIC)
            .map(|body| serde_json::to_vec(&body).map_or(u64::MAX, |bytes| bytes.len() as u64))
            .unwrap_or(0);
        if body > max_body_bytes {
            return Err(Refusal::over_limit(
                format!("items[{index}]"),
                max_body_bytes,
                format!(
                    "the provider request for this item is {body} bytes; at most {max_body_bytes}"
                ),
            ));
        }
    }
    Ok(())
}

/// One entry of the Workers AI envelope's `errors`.
#[derive(Clone, Debug, PartialEq, Deserialize, Serialize)]
#[non_exhaustive]
pub struct EnvelopeError {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub code: Option<u64>,
    #[serde(default)]
    pub message: String,
}

/// The Workers AI REST envelope, `{result, success, errors, messages}`. A
/// runner reads `result` on success and `errors[].code` on failure, and
/// never forwards the envelope. Every member decodes leniently: whether
/// Clairic uses the envelope exactly like this is not yet measured.
#[derive(Clone, Debug, Deserialize, Serialize)]
#[non_exhaustive]
pub struct Envelope {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub result: Option<ProviderResult>,
    #[serde(default)]
    pub success: bool,
    #[serde(default)]
    pub errors: Vec<EnvelopeError>,
    #[serde(default)]
    pub messages: Vec<Value>,
}

impl Envelope {
    /// The first error code, which [`workers_ai_item_code`] reads.
    pub fn first_error_code(&self) -> Option<u64> {
        self.errors.iter().find_map(|error| error.code)
    }
}

/// Clairic's `result`: `{answers, truncated, state_text?, usage}`. Members
/// this crate does not name are not forwarded.
#[derive(Clone, Debug, Deserialize, Serialize)]
#[non_exhaustive]
pub struct ProviderResult {
    pub answers: IndexMap<String, Answer>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub truncated: Option<bool>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub state_text: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub usage: Option<Usage>,
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::*;
    use crate::{
        field::FieldType,
        request::{Item, Options},
    };

    #[test]
    fn an_item_maps_to_one_provider_request_with_every_field() {
        let request = ExtractRequest::new(
            "b",
            catalog_ids::CLAIRIC,
            [
                ("vendor".to_owned(), Field::new(FieldType::Text)),
                (
                    "total".to_owned(),
                    Field::new(FieldType::Number).with_option("decimals", 2),
                ),
            ]
            .into(),
            vec![
                Item::new("a"),
                Item::new("Invoice 7").with_reference_date("2026-03-01"),
            ],
        )
        .with_options(Options::new(true, false));
        let body = ProviderRequest::for_item(&request, 1, models::CLAIRIC).unwrap();
        assert_eq!(
            serde_json::to_value(&body).unwrap(),
            json!({
                "model": "@cf/cloudflare/clairic",
                "state": "Invoice 7",
                "reference_date": "2026-03-01",
                "fields": {"vendor": {"type": "text"}, "total": {"type": "number", "decimals": 2}},
                "options": {"include_quantiles": true, "return_state_text": false}
            })
        );
        assert!(ProviderRequest::for_item(&request, 2, models::CLAIRIC).is_none());
        assert_eq!(
            clairic_endpoint("acct"),
            "https://api.cloudflare.com/client/v4/accounts/acct/ai/run/@cf/cloudflare/clairic"
        );
    }

    #[test]
    fn an_oversize_provider_body_is_refused_naming_its_item() {
        let request = ExtractRequest::new(
            "b",
            catalog_ids::CLAIRIC,
            [("vendor".to_owned(), Field::new(FieldType::Text))].into(),
            vec![Item::new("short"), Item::new("x".repeat(200))],
        );
        let refusal = check_provider_bodies(&request, 200).unwrap_err();
        assert_eq!(refusal.field(), Some("items[1]"));
        assert_eq!(refusal.detail.limit, Some(200));
        assert!(check_provider_bodies(&request, 1000).is_ok());
    }

    #[test]
    fn the_envelope_decodes_leniently() {
        let failed: Envelope = serde_json::from_str(
            r#"{"result":null,"success":false,"errors":[{"code":3036,"message":"daily limit"}],"messages":[]}"#,
        )
        .unwrap();
        assert_eq!(failed.first_error_code(), Some(3036));
        assert_eq!(
            workers_ai_item_code(429, failed.first_error_code()),
            Some(crate::errors::ACCOUNT_LIMIT_EXHAUSTED)
        );
        let bare: Envelope = serde_json::from_str("{}").unwrap();
        assert_eq!(bare.first_error_code(), None);
    }
}
