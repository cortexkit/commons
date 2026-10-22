//! `extract.run` requests and their validators.
//!
//! A runner validates a request completely before it records anything or
//! calls a provider ([`admit`] runs every step in the contract's order):
//! [`check_request_bytes`] checks its size, [`parse_request`] decodes the
//! text naming the first malformed field, [`check_catalog_row`] the model's
//! catalog row, [`check_request`] every limit the model's `role.describe`
//! entry states, [`check_provider_bodies`](crate::provider::check_provider_bodies)
//! each provider request's size, and [`check_reply_size`] the reply's.
//! Each refusal carries its code and `detail.field`.

use indexmap::IndexMap;
use serde::{Deserialize, Serialize};
use serde_json::{Number, Value};

use cortexkit_model_batch::{
    admission::{
        bounded, check_batch_id, check_catalog_row as check_row, check_max_cost_usd,
        check_request_size,
    },
    image::check_images,
};

pub use cortexkit_model_batch::admission::{
    effective_ceiling, is_batch_id, CatalogRow, BATCH_ID_MAX_LEN,
};

use crate::{
    describe::{Limits, ModelEntry, RoleDescribe},
    errors::Refusal,
    field::{check_field, Field},
    image::Image,
};

/// The catalog `kind` of an extraction model.
pub const EXTRACTION_KIND: &str = "extraction";

/// The largest reply frame the daemon carries from a module to a client, in
/// bytes (4 MiB).
pub const REPLY_FRAME_BYTES: u64 = 4 * 1024 * 1024;

/// The reply-size estimate's allowance for the reply's own members, beyond
/// the JSON-escaped `batch_id` and `model` it counts exactly: the member
/// names, the batch `usage` and `cost_usd`, and punctuation.
pub const REPLY_BATCH_ALLOWANCE: u64 = 512;

/// The reply-size estimate's allowance for each item, beyond its answers
/// and echoed state text: `index`, `truncated`, `usage` with every count,
/// punctuation, and an item error in place of the answers.
pub const REPLY_ITEM_ALLOWANCE: u64 = 1024;

/// The member names an `extract.run` request may carry.
pub const REQUEST_MEMBERS: &[&str] = &[
    "batch_id",
    "model",
    "fields",
    "items",
    "options",
    "max_cost_usd",
];

/// The member names an item may carry.
pub const ITEM_MEMBERS: &[&str] = &["state", "images", "reference_date"];

/// The member names `options` may carry.
pub const OPTION_MEMBERS: &[&str] = &["quantiles", "state_text"];

/// The `extract.run` request.
///
/// Non-exhaustive so later optional members are additive: use
/// [`ExtractRequest::new`] and the `with_*` setters, or decode one.
#[derive(Clone, Debug, PartialEq, Deserialize, Serialize)]
#[non_exhaustive]
pub struct ExtractRequest {
    /// The idempotency key: 1 to [`BATCH_ID_MAX_LEN`] printable ASCII
    /// characters.
    pub batch_id: String,
    /// The catalog id, `<provider>/<model>`. Never substituted.
    pub model: String,
    /// The fields, by caller-chosen id, in the order the caller wrote them.
    /// The role never reorders them.
    pub fields: IndexMap<String, Field>,
    /// The items, answered in this order.
    pub items: Vec<Item>,
    /// Batch options, applied to every item.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub options: Option<Options>,
    /// The batch's spend ceiling. Not part of the body identity; a lower
    /// value on a re-send becomes the batch's recorded ceiling (see
    /// [`effective_ceiling`]).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub max_cost_usd: Option<Number>,
}

impl ExtractRequest {
    pub fn new(
        batch_id: impl Into<String>,
        model: impl Into<String>,
        fields: IndexMap<String, Field>,
        items: Vec<Item>,
    ) -> Self {
        Self {
            batch_id: batch_id.into(),
            model: model.into(),
            fields,
            items,
            options: None,
            max_cost_usd: None,
        }
    }

    pub fn with_options(mut self, options: Options) -> Self {
        self.options = Some(options);
        self
    }

    pub fn with_max_cost_usd(mut self, max_cost_usd: Number) -> Self {
        self.max_cost_usd = Some(max_cost_usd);
        self
    }

    /// The options in force: absent members are `false`.
    pub fn effective_options(&self) -> EffectiveOptions {
        let options = self.options.unwrap_or_default();
        EffectiveOptions {
            quantiles: options.quantiles.unwrap_or(false),
            state_text: options.state_text.unwrap_or(false),
        }
    }
}

/// One item: `{state, images?, reference_date?}`.
#[derive(Clone, Debug, PartialEq, Deserialize, Serialize)]
#[non_exhaustive]
pub struct Item {
    /// The text to extract from. Empty only when the item carries images.
    pub state: String,
    /// One or more images, where the model takes them.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub images: Option<Vec<Image>>,
    /// `YYYY-MM-DD`, a real calendar date: anchors dates written without a
    /// year, for this item.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub reference_date: Option<String>,
}

impl Item {
    pub fn new(state: impl Into<String>) -> Self {
        Self {
            state: state.into(),
            images: None,
            reference_date: None,
        }
    }

    pub fn with_images(mut self, images: Vec<Image>) -> Self {
        self.images = Some(images);
        self
    }

    pub fn with_reference_date(mut self, reference_date: impl Into<String>) -> Self {
        self.reference_date = Some(reference_date.into());
        self
    }
}

/// Batch options as sent: `{quantiles?, state_text?}`.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Deserialize, Serialize)]
#[non_exhaustive]
pub struct Options {
    /// Ask for quantiles on estimate and forecast answers. Default false.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub quantiles: Option<bool>,
    /// Echo the provider's state text in each answered item. Default false.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub state_text: Option<bool>,
}

impl Options {
    pub fn new(quantiles: bool, state_text: bool) -> Self {
        Self {
            quantiles: Some(quantiles),
            state_text: Some(state_text),
        }
    }
}

/// The options in force, each member stated; what batch identity hashes,
/// so an absent option and the same option sent as `false` are one batch.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Deserialize, Serialize)]
pub struct EffectiveOptions {
    pub quantiles: bool,
    pub state_text: bool,
}

/// Whether `date` is `YYYY-MM-DD` naming a real day of the Gregorian
/// calendar, years 0001 to 9999.
pub fn is_calendar_date(date: &str) -> bool {
    let bytes = date.as_bytes();
    if bytes.len() != 10 || bytes[4] != b'-' || bytes[7] != b'-' {
        return false;
    }
    let number = |range: std::ops::Range<usize>| -> Option<u32> {
        let part = &date[range];
        part.bytes()
            .all(|b| b.is_ascii_digit())
            .then(|| part.parse().ok())
            .flatten()
    };
    let (Some(year), Some(month), Some(day)) = (number(0..4), number(5..7), number(8..10)) else {
        return false;
    };
    let leap = (year % 4 == 0 && year % 100 != 0) || year % 400 == 0;
    let days = match month {
        1 | 3 | 5 | 7 | 8 | 10 | 12 => 31,
        4 | 6 | 9 | 11 => 30,
        2 if leap => 29,
        2 => 28,
        _ => return false,
    };
    year >= 1 && (1..=days).contains(&day)
}

/// Decode an `extract.run` request from its JSON text, refusing a malformed
/// one `invalid_params` naming the first malformed field. Decoding from the
/// text keeps the order of the fields and of each field's options.
///
/// Every object of the request is closed: a member the contract does not
/// name is refused naming its path (`<member>`, `items[i].<member>`,
/// `options.<member>`; a field's unknown option is [`check_field`]'s), so
/// a misspelt member is never silently dropped. The refusal names `params`
/// only when no narrower path applies: the text is not JSON, or not an
/// object.
pub fn parse_request(text: &str) -> Result<ExtractRequest, Refusal> {
    let value: Value = serde_json::from_str(text)
        .map_err(|e| Refusal::invalid_params("params", format!("not JSON: {e}")))?;
    let Value::Object(params) = &value else {
        return Err(Refusal::invalid_params("params", "params is an object"));
    };
    let malformed = |field: String, what: &str| Refusal::invalid_params(field, what.to_owned());
    for (field, what) in [
        ("batch_id", "a string"),
        ("model", "a string"),
        ("fields", "an object"),
        ("items", "an array"),
    ] {
        let ok = matches!(
            (params.get(field), what),
            (Some(Value::String(_)), "a string")
                | (Some(Value::Object(_)), "an object")
                | (Some(Value::Array(_)), "an array")
        );
        if !ok {
            return Err(malformed(field.to_owned(), &format!("{field} is {what}")));
        }
    }
    if let Some(member) = params
        .keys()
        .find(|key| !REQUEST_MEMBERS.contains(&key.as_str()))
    {
        return Err(malformed(
            member.clone(),
            &format!("{member} is not a member of an extract.run request"),
        ));
    }
    match params.get("max_cost_usd") {
        None | Some(Value::Null) | Some(Value::Number(_)) => {}
        Some(_) => return Err(malformed("max_cost_usd".into(), "max_cost_usd is a number")),
    }
    match params.get("options") {
        None | Some(Value::Null) => {}
        Some(Value::Object(options)) => {
            for (name, value) in options {
                if !OPTION_MEMBERS.contains(&name.as_str()) {
                    return Err(malformed(
                        format!("options.{name}"),
                        "options has quantiles and state_text only",
                    ));
                }
                if !matches!(value, Value::Bool(_) | Value::Null) {
                    return Err(malformed(
                        format!("options.{name}"),
                        "an option is a boolean",
                    ));
                }
            }
        }
        Some(_) => return Err(malformed("options".into(), "options is an object")),
    }
    for (id, field) in params["fields"].as_object().into_iter().flatten() {
        let Value::Object(field) = field else {
            return Err(malformed(format!("fields.{id}"), "a field is an object"));
        };
        if !field.get("type").is_some_and(Value::is_string) {
            return Err(malformed(format!("fields.{id}.type"), "type is a string"));
        }
        if !matches!(field.get("instructions"), None | Some(Value::String(_))) {
            return Err(malformed(
                format!("fields.{id}.instructions"),
                "instructions is a string",
            ));
        }
    }
    for (i, item) in params["items"].as_array().into_iter().flatten().enumerate() {
        let Value::Object(item) = item else {
            return Err(malformed(format!("items[{i}]"), "an item is an object"));
        };
        if !item.get("state").is_some_and(Value::is_string) {
            return Err(malformed(format!("items[{i}].state"), "state is a string"));
        }
        if let Some(member) = item
            .keys()
            .find(|key| !ITEM_MEMBERS.contains(&key.as_str()))
        {
            return Err(malformed(
                format!("items[{i}].{member}"),
                "an item has state, images and reference_date only",
            ));
        }
        match item.get("images") {
            None | Some(Value::Null) => {}
            Some(Value::Array(images)) => {
                for (j, image) in images.iter().enumerate() {
                    if serde_json::from_value::<Image>(image.clone()).is_err() {
                        return Err(malformed(
                            format!("items[{i}].images[{j}]"),
                            "an image is a data: URL or {content_type, base64}",
                        ));
                    }
                }
            }
            Some(_) => {
                return Err(malformed(
                    format!("items[{i}].images"),
                    "images is an array",
                ))
            }
        }
        if !matches!(
            item.get("reference_date"),
            None | Some(Value::Null) | Some(Value::String(_))
        ) {
            return Err(malformed(
                format!("items[{i}].reference_date"),
                "reference_date is a YYYY-MM-DD string",
            ));
        }
    }
    // Every member was checked above, so this decode does not fail for any
    // reason a narrower path could name; `params` covers what is left.
    serde_json::from_str(text).map_err(|e| Refusal::invalid_params("params", e.to_string()))
}

/// Check the request body's size against the module's limit, naming
/// `params` and the limit.
pub fn check_request_bytes(len: usize, limits: &Limits) -> Result<(), Refusal> {
    check_request_size(len, limits.max_request_bytes)
}

/// Check the model's catalog row: `model_unknown` when there is none, and
/// `model_not_extractor` when its `kind` is not [`EXTRACTION_KIND`],
/// including a row that states no kind (then without `detail.kind`). The
/// model is never substituted.
pub fn check_catalog_row(model: &str, row: CatalogRow<'_>) -> Result<(), Refusal> {
    check_row(model, row, EXTRACTION_KIND, |model, kind| {
        Refusal::model_not_extractor(model, kind)
    })
}

/// Check every limit of `request` against the model's `role.describe` entry
/// and the module's limits, in this order, returning the first refusal:
/// the batch id; the field count and each field ([`check_field`]); the item
/// count; each item's state, images (shared image rules) and
/// `reference_date`; and `max_cost_usd`.
pub fn check_request(
    request: &ExtractRequest,
    model: &ModelEntry,
    limits: &Limits,
) -> Result<(), Refusal> {
    check_batch_id(&request.batch_id)?;
    bounded(
        request.fields.len(),
        1,
        model.max_fields as usize,
        "fields",
        "fields",
    )?;
    for (id, field) in &request.fields {
        check_field(id, field, &model.field_types, limits.max_instruction_bytes)?;
    }
    bounded(
        request.items.len(),
        1,
        model.max_items as usize,
        "items",
        "items",
    )?;
    for (i, item) in request.items.iter().enumerate() {
        let has_images = item
            .images
            .as_ref()
            .is_some_and(|images| !images.is_empty());
        if item.state.is_empty() && !has_images {
            return Err(Refusal::invalid_params(
                format!("items[{i}].state"),
                "state is non-empty unless the item carries images",
            ));
        }
        if let Some(images) = &item.images {
            let field = format!("items[{i}].images");
            if images.is_empty() {
                return Err(Refusal::over_limit(
                    field,
                    1,
                    "images, when present, holds at least one image",
                ));
            }
            check_images(&model.model, &field, images, model.images.as_ref())?;
        }
        if let Some(date) = &item.reference_date {
            if !is_calendar_date(date) {
                return Err(Refusal::invalid_params(
                    format!("items[{i}].reference_date"),
                    format!("{date} is not a YYYY-MM-DD calendar date"),
                ));
            }
        }
    }
    check_max_cost_usd(request.max_cost_usd.as_ref())
}

/// The length of `text` as a JSON string, quotes and escapes included.
fn json_len(text: &str) -> u64 {
    serde_json::to_string(text).map_or(u64::MAX, |encoded| encoded.len() as u64)
}

/// The reply-size estimate for `request` on `model`: an upper bound on the
/// encoded `extract.run` reply, given that the module never forwards an
/// answer larger than `model.max_answer_bytes`.
///
/// ```text
/// REPLY_BATCH_ALLOWANCE + json(batch_id) + json(model)
///   + per item: REPLY_ITEM_ALLOWANCE
///               + json(state), when options.state_text
///               + per field: json(id) + 1 + max_answer_bytes
/// ```
///
/// `json(s)` is the JSON-escaped length of `s`, quotes included; the `1`
/// is the colon after an answer's key. The echoed state text is counted
/// as the caller's `state`: Clairic echoes it byte for byte (`CONTRACT.md`
/// §5). Saturates rather than wrapping.
pub fn estimated_reply_bytes(request: &ExtractRequest, model: &ModelEntry) -> u64 {
    let per_answers: u64 = request
        .fields
        .keys()
        .map(|id| {
            json_len(id)
                .saturating_add(1)
                .saturating_add(model.max_answer_bytes)
        })
        .fold(0, u64::saturating_add);
    let state_text = request.effective_options().state_text;
    let items = request
        .items
        .iter()
        .map(|item| {
            let echoed = if state_text { json_len(&item.state) } else { 0 };
            REPLY_ITEM_ALLOWANCE
                .saturating_add(echoed)
                .saturating_add(per_answers)
        })
        .fold(0, u64::saturating_add);
    REPLY_BATCH_ALLOWANCE
        .saturating_add(json_len(&request.batch_id))
        .saturating_add(json_len(&request.model))
        .saturating_add(items)
}

/// Refuse a batch whose estimated reply would not fit the daemon's reply
/// frame ([`REPLY_FRAME_BYTES`]), before any provider call:
/// `invalid_params` naming `items`, with `detail.limit` the frame size.
pub fn check_reply_size(request: &ExtractRequest, model: &ModelEntry) -> Result<(), Refusal> {
    let estimate = estimated_reply_bytes(request, model);
    if estimate > REPLY_FRAME_BYTES {
        return Err(Refusal::over_limit(
            "items",
            REPLY_FRAME_BYTES,
            format!(
                "the reply is estimated at {estimate} bytes; a reply frame holds at most {REPLY_FRAME_BYTES}"
            ),
        ));
    }
    Ok(())
}

/// Every admission check, in the contract's order (§6.7): the request size,
/// the decode, the catalog row (`row_of` reads the runner's catalog), the
/// model's `role.describe` entry and limits, each provider request's size
/// against `max_provider_body_bytes`, and the reply size. Returns the
/// decoded request when every check passes. A model whose row is an
/// extraction model but which `describe` does not list is `model_unknown`:
/// this module does not serve it.
pub fn admit<'a>(
    text: &str,
    describe: &RoleDescribe,
    row_of: impl FnOnce(&str) -> CatalogRow<'a>,
    max_provider_body_bytes: u64,
) -> Result<ExtractRequest, Refusal> {
    check_request_bytes(text.len(), &describe.limits)?;
    let request = parse_request(text)?;
    check_catalog_row(&request.model, row_of(&request.model))?;
    let model = describe
        .model(&request.model)
        .ok_or_else(|| Refusal::model_unknown(&request.model))?;
    check_request(&request, model, &describe.limits)?;
    crate::provider::check_provider_bodies(&request, max_provider_body_bytes)?;
    check_reply_size(&request, model)?;
    Ok(request)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{
        errors::{INVALID_PARAMS, MODEL_NOT_EXTRACTOR, MODEL_UNKNOWN},
        field::{types, FieldType},
        image::InlineImage,
    };

    fn model() -> ModelEntry {
        ModelEntry::new(
            "cloudflare-workers-ai/@cf/cloudflare/clairic",
            3,
            2,
            types::ALL.iter().map(|t| FieldType::parse(t)).collect(),
            8192,
            1000,
            "api.cloudflare.com",
        )
        .with_images(crate::describe::ImageLimits::workers_ai(2))
    }

    fn limits() -> Limits {
        Limits::new(10, 1 << 20, 100)
    }

    fn request(items: Vec<Item>) -> ExtractRequest {
        ExtractRequest::new(
            "b-1",
            "cloudflare-workers-ai/@cf/cloudflare/clairic",
            [("total".to_owned(), Field::new(FieldType::Number))].into(),
            items,
        )
    }

    fn field(result: Result<(), Refusal>) -> String {
        let refusal = result.unwrap_err();
        assert_eq!(refusal.code, INVALID_PARAMS);
        refusal.detail.field.unwrap()
    }

    /// The fields and states of the three requests Clairic refused in
    /// a live probe of 2026-10-22 (probes 08, 09 and 11), as role
    /// requests: each is refused at admission, before any provider call.
    #[test]
    fn what_clairic_refuses_is_refused_at_admission() {
        let po = "Purchase order PO-7731 from Brightwell Interiors, dated 2026-05-04. Three standing desks at $419.95 each and two task chairs at $189.50 each, delivered to the Leeds office on 2026-05-18. Payment terms: net 45. Contact: Imogen Hart.";
        let describe = RoleDescribe::new(Limits::new(10, 1 << 20, 100), vec![model()]);
        let admit_one = |fields: &str, state: &str| {
            let text = format!(
                r#"{{"batch_id":"b","model":"cloudflare-workers-ai/@cf/cloudflare/clairic","fields":{fields},"items":[{{"state":{}}}]}}"#,
                serde_json::to_string(state).unwrap()
            );
            admit(
                &text,
                &describe,
                |_| CatalogRow::Kind("extraction"),
                u64::MAX,
            )
            .unwrap_err()
        };
        // Probe 08: HTTP 400, Workers AI code 5006.
        let unknown_type = admit_one(
            r#"{"desk_colour":{"type":"colour","instructions":"Colour of the standing desks"}}"#,
            po,
        );
        assert_eq!(unknown_type.code, INVALID_PARAMS);
        assert_eq!(unknown_type.field(), Some("fields.desk_colour.type"));
        // Probe 09: HTTP 422, code 3030; the provider refuses an unknown
        // option rather than ignoring it.
        let unknown_option = admit_one(
            r#"{"desk_price":{"type":"number","decimalz":2,"instructions":"Price of one standing desk in US dollars"}}"#,
            po,
        );
        assert_eq!(unknown_option.field(), Some("fields.desk_price.decimalz"));
        // Probe 11: HTTP 422, code 3030.
        let empty_state = admit_one(
            r#"{"contact_name":{"type":"text","instructions":"Name of the order's contact"}}"#,
            "",
        );
        assert_eq!(empty_state.field(), Some("items[0].state"));
    }

    #[test]
    fn calendar_dates() {
        for good in [
            "2026-02-28",
            "2024-02-29",
            "2000-02-29",
            "0001-01-01",
            "9999-12-31",
        ] {
            assert!(is_calendar_date(good), "{good}");
        }
        for bad in [
            "2026-02-30",
            "2023-02-29",
            "1900-02-29",
            "2026-13-01",
            "2026-00-10",
            "0000-01-01",
            "2026-1-01",
            "26-01-01",
            "2026/01/01",
            "+026-01-01",
        ] {
            assert!(!is_calendar_date(bad), "{bad}");
        }
    }

    #[test]
    fn states_dates_and_images_are_refused_naming_their_index() {
        let empty = request(vec![Item::new("ok"), Item::new("")]);
        assert_eq!(
            field(check_request(&empty, &model(), &limits())),
            "items[1].state"
        );
        let pixel = Image::Inline(InlineImage::new("image/png", "AAAA"));
        let image_only = request(vec![Item::new("").with_images(vec![pixel.clone()])]);
        assert!(check_request(&image_only, &model(), &limits()).is_ok());
        let none = request(vec![Item::new("").with_images(vec![])]);
        assert_eq!(
            field(check_request(&none, &model(), &limits())),
            "items[0].state"
        );
        let empty_list = request(vec![Item::new("s").with_images(vec![])]);
        assert_eq!(
            field(check_request(&empty_list, &model(), &limits())),
            "items[0].images"
        );
        let date = request(vec![Item::new("s").with_reference_date("2026-02-30")]);
        assert_eq!(
            field(check_request(&date, &model(), &limits())),
            "items[0].reference_date"
        );
        let bad = Image::Inline(InlineImage::new("image/png", "not base64!"));
        let malformed = request(vec![Item::new("s").with_images(vec![pixel, bad])]);
        assert_eq!(
            field(check_request(&malformed, &model(), &limits())),
            "items[0].images[1]"
        );
    }

    #[test]
    fn parse_names_the_member_and_closes_every_object() {
        let refusal =
            parse_request(r#"{"batch_id":"b","model":"m","fields":{},"items":[{"state":1}]}"#)
                .unwrap_err();
        assert_eq!(refusal.field(), Some("items[0].state"));
        let refusal =
            parse_request(r#"{"batch_id":"b","model":"m","fields":{},"items":[],"max_cost":1}"#)
                .unwrap_err();
        assert_eq!(refusal.field(), Some("max_cost"));
        let refusal = parse_request(
            r#"{"batch_id":"b","model":"m","fields":{},"items":[{"state":"s","reference":"2026-01-01"}]}"#,
        )
        .unwrap_err();
        assert_eq!(refusal.field(), Some("items[0].reference"));
        let refusal = parse_request(
            r#"{"batch_id":"b","model":"m","fields":{},"items":[],"options":{"quantile":true}}"#,
        )
        .unwrap_err();
        assert_eq!(refusal.field(), Some("options.quantile"));
        for whole in ["", "[]", "7"] {
            assert_eq!(parse_request(whole).unwrap_err().field(), Some("params"));
        }
        let parsed = parse_request(
            r#"{"batch_id":"b","model":"m","fields":{"z":{"type":"text"},"a":{"type":"number","decimals":2}},"items":[{"state":"s"}]}"#,
        )
        .unwrap();
        assert_eq!(parsed.fields.keys().collect::<Vec<_>>(), ["z", "a"]);
    }

    #[test]
    fn counts_are_refused_with_their_limit() {
        let items = (0..4).map(|i| Item::new(format!("s{i}"))).collect();
        let refusal = check_request(&request(items), &model(), &limits()).unwrap_err();
        assert_eq!(refusal.field(), Some("items"));
        assert_eq!(refusal.detail.limit, Some(3));
        let mut many = request(vec![Item::new("s")]);
        for id in ["a", "b"] {
            many.fields.insert(id.into(), Field::new(FieldType::Text));
        }
        let refusal = check_request(&many, &model(), &limits()).unwrap_err();
        assert_eq!(refusal.field(), Some("fields"));
        assert_eq!(refusal.detail.limit, Some(2));
    }

    #[test]
    fn catalog_rows_are_three_cases() {
        assert!(check_catalog_row("m", CatalogRow::Kind("extraction")).is_ok());
        assert_eq!(
            check_catalog_row("m", CatalogRow::Absent).unwrap_err().code,
            MODEL_UNKNOWN
        );
        let kindless = check_catalog_row("m", CatalogRow::Kindless).unwrap_err();
        assert_eq!(kindless.code, MODEL_NOT_EXTRACTOR);
        assert_eq!(kindless.detail.kind, None);
        let chat = check_catalog_row("m", CatalogRow::Kind("chat")).unwrap_err();
        assert_eq!(chat.detail.kind.as_deref(), Some("chat"));
    }

    #[test]
    fn the_reply_estimate_counts_echoed_state_and_answers() {
        let mut one = request(vec![Item::new("a\"b")]);
        let base = REPLY_BATCH_ALLOWANCE + 5 + 46 + REPLY_ITEM_ALLOWANCE + (7 + 1 + 1000);
        assert_eq!(estimated_reply_bytes(&one, &model()), base);
        one = one.with_options(Options::new(false, true));
        // "a\"b" encodes as 6 bytes: two quotes, a, backslash, quote, b.
        assert_eq!(estimated_reply_bytes(&one, &model()), base + 6);
        let mut tight = model();
        tight.max_answer_bytes = REPLY_FRAME_BYTES;
        let refusal = check_reply_size(&one, &tight).unwrap_err();
        assert_eq!(refusal.field(), Some("items"));
        assert_eq!(refusal.detail.limit, Some(REPLY_FRAME_BYTES));
    }
}
