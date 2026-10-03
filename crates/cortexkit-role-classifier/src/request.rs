//! `classify.run` requests and their validators.
//!
//! A runner validates a request completely before it records anything or
//! calls a provider: [`parse_request`] decodes the request text naming the
//! first malformed field, [`check_request_bytes`] checks its size,
//! [`check_catalog_row`] the model's catalog row, and [`check_request`]
//! every limit the model's `role.describe` entry states. Each refusal
//! carries its code and `detail.field`.

use indexmap::IndexMap;
use serde::{Deserialize, Serialize};
use serde_json::{Number, Value};

use crate::{
    describe::{Limits, ModelEntry},
    errors::Refusal,
    question::{bounded, check_image, check_question, Image, Question, IMAGES_MAX_TOTAL_BYTES},
};

/// Longest `batch_id`, in bytes.
pub const BATCH_ID_MAX_LEN: usize = 128;

/// The catalog `kind` of a classifier model.
pub const CLASSIFIER_KIND: &str = "classifier";

/// The `classify.run` request.
///
/// Non-exhaustive so later optional members are additive: use
/// [`ClassifyRequest::new`] and the `with_*` setters, or decode one.
#[derive(Clone, Debug, PartialEq, Deserialize, Serialize)]
#[non_exhaustive]
pub struct ClassifyRequest {
    /// The idempotency key: 1 to [`BATCH_ID_MAX_LEN`] printable ASCII
    /// characters.
    pub batch_id: String,
    /// The catalog id, `<provider>/<model>`. Never substituted.
    pub model: String,
    /// The questions, by caller-chosen id, in the order the caller wrote
    /// them.
    pub questions: IndexMap<String, Question>,
    /// The items, answered in this order.
    pub items: Vec<Item>,
    /// The batch's spend ceiling. Not part of the body identity; see
    /// [`effective_ceiling`].
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub max_cost_usd: Option<Number>,
}

impl ClassifyRequest {
    pub fn new(
        batch_id: impl Into<String>,
        model: impl Into<String>,
        questions: IndexMap<String, Question>,
        items: Vec<Item>,
    ) -> Self {
        Self {
            batch_id: batch_id.into(),
            model: model.into(),
            questions,
            items,
            max_cost_usd: None,
        }
    }

    pub fn with_max_cost_usd(mut self, max_cost_usd: Number) -> Self {
        self.max_cost_usd = Some(max_cost_usd);
        self
    }
}

/// One item: `{state, images?}`.
#[derive(Clone, Debug, PartialEq, Deserialize, Serialize)]
#[non_exhaustive]
pub struct Item {
    /// A string, an object or an array, forwarded to the provider verbatim.
    /// Null, numbers and booleans are refused.
    pub state: Value,
    /// Only where the model's catalog row allows images.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub images: Option<Vec<Image>>,
}

impl Item {
    pub fn new(state: impl Into<Value>) -> Self {
        Self {
            state: state.into(),
            images: None,
        }
    }

    pub fn with_images(mut self, images: Vec<Image>) -> Self {
        self.images = Some(images);
        self
    }
}

/// Whether `batch_id` is 1 to [`BATCH_ID_MAX_LEN`] printable ASCII
/// characters (space through tilde).
pub fn is_batch_id(batch_id: &str) -> bool {
    !batch_id.is_empty()
        && batch_id.len() <= BATCH_ID_MAX_LEN
        && batch_id.bytes().all(|b| (0x20..=0x7e).contains(&b))
}

/// Whether `state` is a kind the role forwards: a string, an object or an
/// array.
pub fn is_state(state: &Value) -> bool {
    matches!(state, Value::String(_) | Value::Object(_) | Value::Array(_))
}

/// Decode a `classify.run` request from its JSON text, refusing a malformed
/// one `invalid_params` naming the first malformed field. Decoding from the
/// text keeps the order of the questions and of each choice's options.
pub fn parse_request(text: &str) -> Result<ClassifyRequest, Refusal> {
    let value: Value = serde_json::from_str(text)
        .map_err(|e| Refusal::invalid_params("params", format!("not JSON: {e}")))?;
    let Value::Object(params) = &value else {
        return Err(Refusal::invalid_params("params", "params is an object"));
    };
    let malformed = |field: String, what: &str| Refusal::invalid_params(field, what.to_owned());
    for (field, what) in [
        ("batch_id", "a string"),
        ("model", "a string"),
        ("questions", "an object"),
        ("items", "an array"),
    ] {
        let ok = match (params.get(field), what) {
            (Some(Value::String(_)), "a string") => true,
            (Some(Value::Object(_)), "an object") => true,
            (Some(Value::Array(_)), "an array") => true,
            _ => false,
        };
        if !ok {
            return Err(malformed(field.to_owned(), &format!("{field} is {what}")));
        }
    }
    match params.get("max_cost_usd") {
        None | Some(Value::Null) | Some(Value::Number(_)) => {}
        Some(_) => return Err(malformed("max_cost_usd".into(), "max_cost_usd is a number")),
    }
    for (id, question) in params["questions"].as_object().into_iter().flatten() {
        let Value::Object(question) = question else {
            return Err(malformed(
                format!("questions.{id}"),
                "a question is an object",
            ));
        };
        if !question.get("type").is_some_and(Value::is_string) {
            return Err(malformed(
                format!("questions.{id}.type"),
                "type is a string",
            ));
        }
        if !question.contains_key("instructions") {
            return Err(malformed(
                format!("questions.{id}.instructions"),
                "instructions is required",
            ));
        }
        if !matches!(
            question.get("criteria"),
            None | Some(Value::Null) | Some(Value::Array(_)) | Some(Value::Object(_))
        ) {
            return Err(malformed(
                format!("questions.{id}.criteria"),
                "criteria is an array or an object",
            ));
        }
    }
    for (i, item) in params["items"].as_array().into_iter().flatten().enumerate() {
        let Value::Object(item) = item else {
            return Err(malformed(format!("items[{i}]"), "an item is an object"));
        };
        if !item.get("state").is_some_and(is_state) {
            return Err(malformed(
                format!("items[{i}].state"),
                "state is a string, an object or an array",
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
    }
    serde_json::from_str(text).map_err(|e| Refusal::invalid_params("params", e.to_string()))
}

/// Check the request body's size against the module's limit. The refusal
/// names `params` and the limit.
pub fn check_request_bytes(len: usize, limits: &Limits) -> Result<(), Refusal> {
    if len as u64 > limits.max_request_bytes {
        Err(Refusal::over_limit(
            "params",
            limits.max_request_bytes,
            format!(
                "the request is {len} bytes; at most {}",
                limits.max_request_bytes
            ),
        ))
    } else {
        Ok(())
    }
}

/// Check the model's catalog row: `None` when the model is not in the
/// catalog (`model_unknown`), else its `kind`, which must be
/// [`CLASSIFIER_KIND`] (`model_not_classifier`). The model is never
/// substituted.
pub fn check_catalog_row(model: &str, kind: Option<&str>) -> Result<(), Refusal> {
    match kind {
        None => Err(Refusal::model_unknown(model)),
        Some(CLASSIFIER_KIND) => Ok(()),
        Some(kind) => Err(Refusal::model_not_classifier(model, kind)),
    }
}

/// Check every limit of `request` against the model's `role.describe` entry:
/// the batch id, question count and each question, item count, each item's
/// state and images, and `max_cost_usd`. Returns the first refusal.
pub fn check_request(request: &ClassifyRequest, model: &ModelEntry) -> Result<(), Refusal> {
    if !is_batch_id(&request.batch_id) {
        return Err(Refusal::invalid_params(
            "batch_id",
            "batch_id is 1-128 printable ASCII characters",
        ));
    }
    bounded(
        request.questions.len(),
        1,
        model.max_questions as usize,
        "questions",
        "questions",
    )?;
    for (id, question) in &request.questions {
        check_question(id, question)?;
    }
    bounded(
        request.items.len(),
        1,
        model.max_items as usize,
        "items",
        "items",
    )?;
    for (i, item) in request.items.iter().enumerate() {
        if !is_state(&item.state) {
            return Err(Refusal::invalid_params(
                format!("items[{i}].state"),
                "state is a string, an object or an array",
            ));
        }
        let Some(images) = &item.images else {
            continue;
        };
        let field = format!("items[{i}].images");
        let max = model.images.map_or(0, |support| support.max);
        if images.len() as u64 > max {
            return Err(Refusal::over_limit(
                field,
                max,
                if model.images.is_none() {
                    format!("{} takes no images", model.model)
                } else {
                    format!("{} images; at most {max}", images.len())
                },
            ));
        }
        for (j, image) in images.iter().enumerate() {
            check_image(image)
                .map_err(|why| Refusal::invalid_params(format!("{field}[{j}]"), why))?;
        }
        let total: usize = images.iter().map(Image::approx_bytes).sum();
        if total > IMAGES_MAX_TOTAL_BYTES {
            return Err(Refusal::over_limit(
                field,
                IMAGES_MAX_TOTAL_BYTES as u64,
                format!("{total} image bytes; at most {IMAGES_MAX_TOTAL_BYTES}"),
            ));
        }
    }
    if let Some(max_cost_usd) = &request.max_cost_usd {
        if max_cost_usd.as_f64().is_none_or(|usd| usd < 0.0) {
            return Err(Refusal::invalid_params(
                "max_cost_usd",
                "max_cost_usd is a number of USD, zero or more",
            ));
        }
    }
    Ok(())
}

/// The ceiling a call works under: the lower of the ceiling recorded at the
/// batch's first admission and the one this call carries. A caller can
/// tighten a batch's ceiling on a re-send, never loosen it. `None` when
/// neither names one.
pub fn effective_ceiling(recorded: Option<f64>, sent: Option<f64>) -> Option<f64> {
    match (recorded, sent) {
        (Some(recorded), Some(sent)) => Some(recorded.min(sent)),
        (one, None) | (None, one) => one,
    }
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::*;
    use crate::{
        describe::ImageSupport,
        errors::{INVALID_PARAMS, MODEL_NOT_CLASSIFIER, MODEL_UNKNOWN},
        question::InlineImage,
    };

    fn model() -> ModelEntry {
        ModelEntry::new("cloudflare/clef", 2, 3, 8192, "api.cloudflare.com")
            .with_images(ImageSupport::new(4))
    }

    fn request(items: Vec<Item>) -> ClassifyRequest {
        ClassifyRequest::new(
            "b-1",
            "cloudflare/clef",
            [("urgent".to_owned(), Question::noul("Is it urgent?"))].into(),
            items,
        )
    }

    fn field(result: Result<(), Refusal>) -> String {
        let refusal = result.unwrap_err();
        assert_eq!(refusal.code, INVALID_PARAMS);
        refusal.detail.field.unwrap()
    }

    #[test]
    fn a_state_of_null_number_or_boolean_is_refused_naming_its_index() {
        for bad in [json!(null), json!(3), json!(true)] {
            let items = vec![Item::new("ok"), Item::new(json!({"a": 1})), Item::new(bad)];
            assert_eq!(
                field(check_request(&request(items), &model())),
                "items[2].state"
            );
        }
        let items = vec![
            Item::new("s"),
            Item::new(json!({"a": 1})),
            Item::new(json!([1])),
        ];
        assert!(check_request(&request(items), &model()).is_ok());
        let refusal = parse_request(
            r#"{"batch_id":"b","model":"m","questions":{},"items":[{"state":"s"},{"state":null}]}"#,
        )
        .unwrap_err();
        assert_eq!(refusal.field(), Some("items[1].state"));
    }

    #[test]
    fn counts_are_refused_with_their_limit() {
        let items = (0..4).map(|i| Item::new(format!("s{i}"))).collect();
        let refusal = check_request(&request(items), &model()).unwrap_err();
        assert_eq!(refusal.field(), Some("items"));
        assert_eq!(refusal.detail.limit, Some(3));
        let mut three = request(vec![Item::new("s")]);
        for id in ["a", "b"] {
            three.questions.insert(id.into(), Question::noul("i"));
        }
        let refusal = check_request(&three, &model()).unwrap_err();
        assert_eq!(refusal.field(), Some("questions"));
        assert_eq!(refusal.detail.limit, Some(2));
        assert_eq!(field(check_request(&request(vec![]), &model())), "items");
    }

    #[test]
    fn images_are_refused_where_the_model_takes_none_or_too_many() {
        let image = Image::Inline(InlineImage::new("image/png", "AAAA"));
        let item = Item::new("s").with_images(vec![image.clone(); 5]);
        let refusal = check_request(&request(vec![item]), &model()).unwrap_err();
        assert_eq!(refusal.field(), Some("items[0].images"));
        assert_eq!(refusal.detail.limit, Some(4));
        let text_only = ModelEntry::new("typesafe/jev-latest", 2, 3, 8192, "api.typesafe.ai");
        let item = Item::new("s").with_images(vec![image]);
        let refusal = check_request(&request(vec![item]), &text_only).unwrap_err();
        assert_eq!(refusal.field(), Some("items[0].images"));
        assert_eq!(refusal.detail.limit, Some(0));
        let item = Item::new("s").with_images(vec![Image::DataUrl("https://x/y.png".into())]);
        assert_eq!(
            field(check_request(&request(vec![item]), &model())),
            "items[0].images[0]"
        );
    }

    #[test]
    fn batch_id_and_max_cost_are_checked() {
        let mut bad = request(vec![Item::new("s")]);
        bad.batch_id = "tab\there".into();
        assert_eq!(field(check_request(&bad, &model())), "batch_id");
        bad.batch_id = "x".repeat(129);
        assert_eq!(field(check_request(&bad, &model())), "batch_id");
        let negative =
            request(vec![Item::new("s")]).with_max_cost_usd(Number::from_f64(-1.0).unwrap());
        assert_eq!(field(check_request(&negative, &model())), "max_cost_usd");
    }

    #[test]
    fn parse_names_the_malformed_field_and_keeps_question_order() {
        let refusal = parse_request(r#"{"model":"m","questions":{},"items":[]}"#).unwrap_err();
        assert_eq!(refusal.field(), Some("batch_id"));
        let refusal = parse_request(
            r#"{"batch_id":"b","model":"m","questions":{"q":{"instructions":"i"}},"items":[]}"#,
        )
        .unwrap_err();
        assert_eq!(refusal.field(), Some("questions.q.type"));
        let parsed = parse_request(
            r#"{"batch_id":"b","model":"m","questions":{"z":{"type":"noul","instructions":"i"},"a":{"type":"noul","instructions":"i"}},"items":[{"state":"s"}]}"#,
        )
        .unwrap();
        assert_eq!(parsed.questions.keys().collect::<Vec<_>>(), ["z", "a"]);
    }

    #[test]
    fn catalog_rows_are_pinned() {
        assert!(check_catalog_row("m", Some("classifier")).is_ok());
        assert_eq!(
            check_catalog_row("m", None).unwrap_err().code,
            MODEL_UNKNOWN
        );
        let refusal = check_catalog_row("m", Some("chat")).unwrap_err();
        assert_eq!(refusal.code, MODEL_NOT_CLASSIFIER);
        assert_eq!(refusal.detail.kind.as_deref(), Some("chat"));
    }

    #[test]
    fn request_bytes_are_bounded() {
        let limits = Limits::new(10, 100);
        assert!(check_request_bytes(100, &limits).is_ok());
        let refusal = check_request_bytes(101, &limits).unwrap_err();
        assert_eq!(refusal.field(), Some("params"));
        assert_eq!(refusal.detail.limit, Some(100));
    }

    #[test]
    fn the_ceiling_tightens_and_never_loosens() {
        assert_eq!(effective_ceiling(Some(1.0), Some(5.0)), Some(1.0));
        assert_eq!(effective_ceiling(Some(1.0), Some(0.5)), Some(0.5));
        assert_eq!(effective_ceiling(None, Some(0.5)), Some(0.5));
        assert_eq!(effective_ceiling(Some(1.0), None), Some(1.0));
        assert_eq!(effective_ceiling(None, None), None);
    }
}
