//! The provider side for Cloudflare Workers AI Clairic: what a runner sends
//! for one item, the reply envelope it reads, and how provider errors map
//! to item codes. `CONTRACT.md` §8 and §9 state each shape and where it
//! comes from: Cloudflare's documentation, and a live probe of
//! Clairic of 2026-10-22 for the reply envelope and the answers.
//!
//! One provider call carries one item with every field of the batch, passed
//! through key by key as the caller wrote them, and the batch options under
//! the provider's own names.

use indexmap::IndexMap;
use serde::{
    de::{Error as _, IgnoredAny},
    Deserialize, Deserializer, Serialize,
};
use serde_json::{value::RawValue, Value};

pub use cortexkit_model_batch::status::{
    item_code_for_status, workers_ai_codes, workers_ai_item_code,
};

use crate::{
    errors::{ItemError, Refusal, PROVIDER_ERROR},
    field::Field,
    image::{DecodedImage, Image, InlineImage},
    reply::{Answer, ItemResult, Usage},
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
    /// Forwarded as the caller wrote them, except that the provider never
    /// sees a reference: a module that resolved an item's references sends
    /// their bytes instead ([`ProviderRequest::with_resolved_images`]).
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

    /// This request with the item's images replaced by their bytes, in
    /// order: what a module that resolved the item's references
    /// (`resolve_item_images`) sends. Inline images keep their bytes, sent
    /// in the canonical object form.
    pub fn with_resolved_images(mut self, resolved: &[DecodedImage]) -> Self {
        if self.images.is_some() {
            self.images = Some(
                resolved
                    .iter()
                    .map(|image| Image::Inline(InlineImage::from_decoded(image)))
                    .collect(),
            );
        }
        self
    }
}

/// Refuse item `index` when its provider request `body` would be larger
/// than the provider accepts (`max_body_bytes`), before any provider call:
/// `invalid_params` naming `items[index]` and the limit. A module that
/// resolves image references runs this on the request with the resolved
/// images in it ([`ProviderRequest::with_resolved_images`]).
pub fn check_provider_body(
    index: usize,
    body: &ProviderRequest,
    max_body_bytes: u64,
) -> Result<(), Refusal> {
    let size = serde_json::to_vec(body).map_or(u64::MAX, |bytes| bytes.len() as u64);
    if size > max_body_bytes {
        return Err(Refusal::over_limit(
            format!("items[{index}]"),
            max_body_bytes,
            format!("the provider request for this item is {size} bytes; at most {max_body_bytes}"),
        ));
    }
    Ok(())
}

/// Refuse a batch any of whose provider requests would be larger than the
/// provider accepts (`max_body_bytes`; Clairic's is
/// [`clairic_limits::MAX_BODY_BYTES`]), before any provider call:
/// `invalid_params` naming `items[i]` and the limit. An image reference
/// counts here only as its handle; see [`check_provider_body`].
pub fn check_provider_bodies(request: &ExtractRequest, max_body_bytes: u64) -> Result<(), Refusal> {
    for index in 0..request.items.len() {
        if let Some(body) = ProviderRequest::for_item(request, index, models::CLAIRIC) {
            check_provider_body(index, &body, max_body_bytes)?;
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
/// never forwards the envelope.
///
/// Clairic was measured to answer a success as `{result, success, errors,
/// messages}`, with `success: true` and `errors: []`, and an error as
/// `{errors, success, result, messages}`, with `success: false` and
/// `result: {}`, not `null`. Members decode in any order; an empty `result`
/// object, or `null`, decodes as no result.
#[derive(Clone, Debug, Deserialize, Serialize)]
#[non_exhaustive]
pub struct Envelope {
    #[serde(
        default,
        deserialize_with = "result_or_empty",
        skip_serializing_if = "Option::is_none"
    )]
    pub result: Option<ProviderResult>,
    #[serde(default)]
    pub success: bool,
    #[serde(default)]
    pub errors: Vec<EnvelopeError>,
    #[serde(default)]
    pub messages: Vec<Value>,
}

/// `result` as a [`ProviderResult`], with `null` and `{}` (what Clairic
/// sends with an error) as no result. The member is read as raw text first,
/// so the answers keep the provider's number spelling.
fn result_or_empty<'de, D: Deserializer<'de>>(
    deserializer: D,
) -> Result<Option<ProviderResult>, D::Error> {
    let Some(raw) = Option::<Box<RawValue>>::deserialize(deserializer)? else {
        return Ok(None);
    };
    let members: IndexMap<String, IgnoredAny> =
        serde_json::from_str(raw.get()).map_err(D::Error::custom)?;
    if members.is_empty() {
        return Ok(None);
    }
    serde_json::from_str(raw.get())
        .map(Some)
        .map_err(D::Error::custom)
}

impl Envelope {
    /// The first error code, which [`workers_ai_item_code`] reads.
    pub fn first_error_code(&self) -> Option<u64> {
        self.errors.iter().find_map(|error| error.code)
    }
}

/// Clairic's `result`: `{model, answers, truncated, usage, state_text?}`.
/// `state_text` is present when the request asked for it
/// (`return_state_text`), and is then the request's `state` byte for byte.
/// Members this crate does not name are not forwarded.
#[derive(Clone, Debug, Deserialize, Serialize)]
#[non_exhaustive]
pub struct ProviderResult {
    /// The provider's own model name (Clairic answers `"clairic"`). Not
    /// forwarded: the reply names the catalog id.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub model: Option<String>,
    pub answers: IndexMap<String, Answer>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub truncated: Option<bool>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub usage: Option<Usage>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub state_text: Option<String>,
}

impl ProviderResult {
    /// Item `index` of the reply, answered with this result: its answers,
    /// truncation flag, state text and usage, each as the provider gave
    /// it. Spans and number text pass through untouched.
    pub fn into_item(self, index: u64) -> ItemResult {
        let mut item = ItemResult::answered(index, self.answers);
        item.truncated = self.truncated;
        item.state_text = self.state_text;
        item.usage = self.usage;
        item
    }
}

/// Read one provider call's reply, once the runner's own retries are spent:
/// its HTTP `status` and its `body` as received.
///
/// - A 2xx status whose envelope decodes with `success: true` and a
///   `result` is that result.
/// - Any other status is an item error, by Workers AI code first, then by
///   status ([`workers_ai_item_code`], `RULES.md` §15). The message names
///   the status, the first Workers AI code and that error's message, so a
///   code the table does not list can still be read. A body that does not
///   decode leaves only the status to map by.
/// - A 2xx status without a usable result (a body that does not decode,
///   `success: false`, or no `result`) is `provider_error`.
///
/// Clairic refuses a field of unknown type with 400 and code 5006, and an
/// unknown option or an empty state without images with 422 and code
/// 3030. Neither code is in the table, so each maps by its status to
/// `invalid_item`, permanent. Admission refuses all three before any call,
/// so these are backstops. A `Retry-After` header is the runner's to add
/// (`ItemError::with_retry_after_ms`); this reads the body only.
// The error is the item's own outcome, returned by value so a runner can
// store it as is; it is built once per item, so its size does not matter.
#[allow(clippy::result_large_err)]
pub fn read_reply(status: u16, body: &str) -> Result<ProviderResult, ItemError> {
    let envelope = serde_json::from_str::<Envelope>(body);
    if (200..=299).contains(&status) {
        let problem = match envelope {
            Ok(Envelope {
                success: true,
                result: Some(result),
                ..
            }) => return Ok(result),
            Ok(envelope) if !envelope.success => describe_errors(status, &envelope),
            Ok(_) => format!("HTTP {status} with no result"),
            Err(error) => format!("HTTP {status} with a reply that does not decode: {error}"),
        };
        return Err(ItemError::for_code(PROVIDER_ERROR, problem));
    }
    // An error reply whose `result` is not a result still carries its
    // `errors`: read them alone when the whole envelope does not decode.
    let envelope = envelope.or_else(|_| {
        serde_json::from_str::<ErrorsOnly>(body).map(|only| Envelope {
            result: None,
            success: false,
            errors: only.errors,
            messages: Vec::new(),
        })
    });
    let (code, message) = match &envelope {
        Ok(envelope) => (
            envelope.first_error_code(),
            describe_errors(status, envelope),
        ),
        Err(_) => (None, format!("HTTP {status}")),
    };
    let item_code = workers_ai_item_code(status, code).unwrap_or(PROVIDER_ERROR);
    Err(ItemError::for_code(item_code, message))
}

/// An error reply's `errors`, every other member ignored.
#[derive(Deserialize)]
struct ErrorsOnly {
    #[serde(default)]
    errors: Vec<EnvelopeError>,
}

/// `HTTP <status>`, then the first error's Workers AI code and message.
fn describe_errors(status: u16, envelope: &Envelope) -> String {
    let mut text = format!("HTTP {status}");
    if let Some(code) = envelope.first_error_code() {
        text.push_str(&format!(", Workers AI code {code}"));
    }
    if let Some(first) = envelope.errors.first().filter(|e| !e.message.is_empty()) {
        text.push_str(": ");
        text.push_str(&first.message);
    }
    text
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::*;
    use crate::{
        errors::{ErrorClass, INVALID_ITEM},
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

    /// Clairic's reply to a field of unknown type (live probe 08),
    /// byte for byte: `errors` first, and `result` an empty object.
    const PROBE_08: &str = r#"{"errors":[{"message":"AiError: Bad input: Error: enum colour not in text,number,date,time,datetime,duration,list,estimate,forecast at '/desk_colour/type' (663a528d-0faf-46d6-bbdc-728e4d418eff)","code":5006}],"success":false,"result":{},"messages":[]}"#;

    #[test]
    fn an_error_envelope_with_an_empty_result_decodes_in_either_order() {
        let errors_first: Envelope = serde_json::from_str(PROBE_08).unwrap();
        assert!(errors_first.result.is_none());
        assert!(!errors_first.success);
        assert_eq!(errors_first.first_error_code(), Some(5006));
        let result_first: Envelope = serde_json::from_str(
            r#"{"result":{},"success":false,"errors":[{"message":"m","code":3030}],"messages":[]}"#,
        )
        .unwrap();
        assert!(result_first.result.is_none());
        assert_eq!(result_first.first_error_code(), Some(3030));
    }

    #[test]
    fn clairic_refusal_codes_map_by_status_to_invalid_item() {
        // 5006 (unknown field type, HTTP 400) and 3030 (unknown option or
        // empty state, HTTP 422) are not in the Workers AI table, so the
        // status decides: invalid_item, permanent, never retried.
        assert_eq!(workers_ai_item_code(400, Some(5006)), Some(INVALID_ITEM));
        assert_eq!(workers_ai_item_code(422, Some(3030)), Some(INVALID_ITEM));
        let error = read_reply(400, PROBE_08).unwrap_err();
        assert_eq!(error.code, INVALID_ITEM);
        assert_eq!(error.class, ErrorClass::Permanent);
        assert!(
            error
                .message
                .starts_with("HTTP 400, Workers AI code 5006: AiError: Bad input"),
            "{}",
            error.message
        );
        // A `result` that is no result does not hide the errors.
        let odd = r#"{"errors":[{"message":"m","code":3030}],"success":false,"result":{"answers":7},"messages":[]}"#;
        assert_eq!(
            read_reply(422, odd).unwrap_err().message,
            "HTTP 422, Workers AI code 3030: m"
        );
    }

    #[test]
    fn a_success_is_its_result_and_anything_else_an_item_error() {
        let body = r#"{"result":{"model":"clairic","answers":{"code":{"type":"text","p_null":0.000014924099559721071,"span":[53,61],"guess":"QK-58213","confidence":0.9999951124191284,"value":"QK-58213"}},"truncated":false,"usage":{"state_tokens":31,"image_tokens":0,"schema_tokens":122,"input_tokens":153,"output_tokens":0,"passes":1}},"success":true,"errors":[],"messages":[]}"#;
        let item = read_reply(200, body).unwrap().into_item(3);
        assert_eq!(item.index, 3);
        let answer = &item.answers.as_ref().unwrap()["code"];
        assert_eq!(answer.span.as_deref().map(RawValue::get), Some("[53,61]"));
        assert_eq!(
            answer.p_null.as_deref().map(RawValue::get),
            Some("0.000014924099559721071")
        );
        assert_eq!(item.truncated, Some(false));
        assert_eq!(item.usage.as_ref().unwrap().input_tokens, Some(153));
        for (status, body) in [
            (
                200,
                r#"{"result":{},"success":true,"errors":[],"messages":[]}"#,
            ),
            (200, "not json"),
            (
                200,
                r#"{"result":{},"success":false,"errors":[{"code":5004,"message":"m"}],"messages":[]}"#,
            ),
            (500, ""),
        ] {
            let error = read_reply(status, body).unwrap_err();
            assert_eq!(error.code, PROVIDER_ERROR, "{status} {body}");
        }
        assert_eq!(
            read_reply(429, r#"{"errors":[{"code":3036,"message":"m"}],"success":false,"result":{},"messages":[]}"#)
                .unwrap_err()
                .code,
            crate::errors::ACCOUNT_LIMIT_EXHAUSTED
        );
    }
}
