//! The providers' shared question shape, and the images an item may carry.
//!
//! Both starting providers (Cloudflare Workers AI `@cf/cloudflare/clef` and
//! TypeSafe `jev-latest`) take the same question object, and a runner
//! forwards each question to the provider as the caller wrote it. The field
//! names are the providers' own; see `CONTRACT.md` for the sources.

use std::fmt;

use indexmap::IndexMap;
use serde::{Deserialize, Deserializer, Serialize, Serializer};
use serde_json::Value;

use crate::errors::Refusal;

/// Question type names.
pub mod types {
    /// A yes/no question; the answer is the probability that the answer is
    /// yes, from 0 (no) to 1 (yes).
    pub const NOUL: &str = "noul";
    /// Pick one of the named options.
    pub const CHOICE: &str = "choice";
    /// Place the item on an ordered scale of levels, lowest first.
    pub const SCORE: &str = "score";
}

/// Fewest options a `choice` question may have.
pub const CHOICE_MIN_OPTIONS: usize = 2;
/// Most options a `choice` question may have.
pub const CHOICE_MAX_OPTIONS: usize = 255;
/// Fewest levels a `score` question may have.
pub const SCORE_MIN_LEVELS: usize = 2;
/// Most levels a `score` question may have.
pub const SCORE_MAX_LEVELS: usize = 10;
/// Longest question id, in bytes. Ids are caller-chosen, `[a-z0-9_-]`.
pub const QUESTION_ID_MAX_LEN: usize = 64;
/// The two keys of a `noul` question's optional criteria.
pub const NOUL_CRITERIA_KEYS: [&str; 2] = ["true", "false"];

/// A question type. Decodes open: a value other than [`types`] is kept in
/// [`QuestionType::Other`]. The open variant is for decoding a reply, where
/// a provider's new type passes through as given; in a request,
/// [`check_question`] refuses it `invalid_params` naming
/// `questions.<id>.type`, because a runner can neither validate nor map a
/// type it does not know.
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub enum QuestionType {
    Noul,
    Choice,
    Score,
    Other(String),
}

impl QuestionType {
    pub fn parse(name: &str) -> Self {
        match name {
            types::NOUL => Self::Noul,
            types::CHOICE => Self::Choice,
            types::SCORE => Self::Score,
            other => Self::Other(other.to_owned()),
        }
    }

    pub fn as_str(&self) -> &str {
        match self {
            Self::Noul => types::NOUL,
            Self::Choice => types::CHOICE,
            Self::Score => types::SCORE,
            Self::Other(other) => other,
        }
    }
}

impl fmt::Display for QuestionType {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

impl Serialize for QuestionType {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        serializer.serialize_str(self.as_str())
    }
}

impl<'de> Deserialize<'de> for QuestionType {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        Ok(Self::parse(&String::deserialize(deserializer)?))
    }
}

/// A question's `criteria`, as the providers document it per type:
///
/// - `score`: an array of level descriptions, lowest first, indexed from 0
///   ([`Criteria::Levels`]). The request has no levels or legend field; the
///   answer's `legend` is keyed by the level index.
/// - `choice`: an object from option id to its description, a string,
///   object, array or null ([`Criteria::Options`]).
/// - `noul`: optionally `{"true": string, "false": string}`, also an
///   [`Criteria::Options`].
///
/// The options keep their order, so a decoded question re-encodes as sent.
#[derive(Clone, Debug, PartialEq, Deserialize, Serialize)]
#[serde(untagged)]
pub enum Criteria {
    Levels(Vec<Value>),
    Options(IndexMap<String, Value>),
}

/// One question: `{type, instructions, criteria?}`.
///
/// Non-exhaustive so later optional members are additive: use
/// [`Question::noul`], [`Question::choice`] or [`Question::score`] and the
/// `with_*` setters, or decode one.
#[derive(Clone, Debug, PartialEq, Deserialize, Serialize)]
#[non_exhaustive]
pub struct Question {
    #[serde(rename = "type")]
    pub kind: QuestionType,
    /// A non-empty string, or an object or array.
    pub instructions: Value,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub criteria: Option<Criteria>,
}

impl Question {
    pub fn new(kind: QuestionType, instructions: impl Into<Value>) -> Self {
        Self {
            kind,
            instructions: instructions.into(),
            criteria: None,
        }
    }

    /// A `noul` question with no criteria.
    pub fn noul(instructions: impl Into<Value>) -> Self {
        Self::new(QuestionType::Noul, instructions)
    }

    /// A `choice` question over `options`, in order.
    pub fn choice(instructions: impl Into<Value>, options: IndexMap<String, Value>) -> Self {
        Self::new(QuestionType::Choice, instructions).with_criteria(Criteria::Options(options))
    }

    /// A `score` question over `levels`, lowest first.
    pub fn score(instructions: impl Into<Value>, levels: Vec<Value>) -> Self {
        Self::new(QuestionType::Score, instructions).with_criteria(Criteria::Levels(levels))
    }

    pub fn with_criteria(mut self, criteria: Criteria) -> Self {
        self.criteria = Some(criteria);
        self
    }
}

/// Whether `id` is a valid question id: 1 to [`QUESTION_ID_MAX_LEN`]
/// characters of `[a-z0-9_-]`.
pub fn is_question_id(id: &str) -> bool {
    !id.is_empty()
        && id.len() <= QUESTION_ID_MAX_LEN
        && id
            .bytes()
            .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'_' || b == b'-')
}

/// Check one question against the role's limits. A refusal names
/// `questions.<id>` (the id), `questions.<id>.type` (a type this crate does
/// not name, checked before anything else about the question, since the
/// rest depends on the type), `questions.<id>.instructions` or
/// `questions.<id>.criteria`.
pub fn check_question(id: &str, question: &Question) -> Result<(), Refusal> {
    let at = |member: &str| format!("questions.{id}.{member}");
    if !is_question_id(id) {
        return Err(Refusal::invalid_params(
            format!("questions.{id}"),
            "a question id is 1-64 characters of [a-z0-9_-]",
        ));
    }
    let unknown_type = |name: &str| {
        Refusal::invalid_params(
            at("type"),
            format!("unknown question type {name}; expected noul, choice or score"),
        )
    };
    if let QuestionType::Other(name) = &question.kind {
        return Err(unknown_type(name));
    }
    let instructions_ok = match &question.instructions {
        Value::String(text) => !text.is_empty(),
        Value::Object(_) | Value::Array(_) => true,
        _ => false,
    };
    if !instructions_ok {
        return Err(Refusal::invalid_params(
            at("instructions"),
            "instructions is a non-empty string, an object or an array",
        ));
    }
    match (&question.kind, &question.criteria) {
        (QuestionType::Noul, None) => Ok(()),
        (QuestionType::Noul, Some(Criteria::Options(options)))
            if options.len() == NOUL_CRITERIA_KEYS.len()
                && NOUL_CRITERIA_KEYS
                    .iter()
                    .all(|key| options.get(*key).is_some_and(Value::is_string)) =>
        {
            Ok(())
        }
        (QuestionType::Noul, Some(_)) => Err(Refusal::invalid_params(
            at("criteria"),
            "noul criteria, when present, is {\"true\": string, \"false\": string}",
        )),
        (QuestionType::Choice, Some(Criteria::Options(options))) => bounded(
            options.len(),
            CHOICE_MIN_OPTIONS,
            CHOICE_MAX_OPTIONS,
            &at("criteria"),
            "choice options",
        ),
        (QuestionType::Choice, _) => Err(Refusal::invalid_params(
            at("criteria"),
            "choice criteria is an object from option id to description",
        )),
        (QuestionType::Score, Some(Criteria::Levels(levels))) => bounded(
            levels.len(),
            SCORE_MIN_LEVELS,
            SCORE_MAX_LEVELS,
            &at("criteria"),
            "score levels",
        ),
        (QuestionType::Score, _) => Err(Refusal::invalid_params(
            at("criteria"),
            "score criteria is an array of level descriptions, lowest first",
        )),
        (QuestionType::Other(name), _) => Err(unknown_type(name)),
    }
}

/// `count` within `min..=max`, or an over-limit refusal naming `field` and
/// the bound it broke.
pub(crate) fn bounded(
    count: usize,
    min: usize,
    max: usize,
    field: &str,
    what: &str,
) -> Result<(), Refusal> {
    if count < min {
        Err(Refusal::over_limit(
            field,
            min as u64,
            format!("{count} {what}; at least {min}"),
        ))
    } else if count > max {
        Err(Refusal::over_limit(
            field,
            max as u64,
            format!("{count} {what}; at most {max}"),
        ))
    } else {
        Ok(())
    }
}

// ---- images ---------------------------------------------------------------

/// Image content types a provider accepts in the object form.
pub mod content_types {
    pub const PNG: &str = "image/png";
    pub const JPEG: &str = "image/jpeg";
    pub const WEBP: &str = "image/webp";
    pub const ALL: &[&str] = &[PNG, JPEG, WEBP];
}

/// Largest image, in decoded bytes (4 MiB).
pub const IMAGE_MAX_BYTES: usize = 4 * 1024 * 1024;
/// Largest total of an item's images, in decoded bytes (8 MiB).
pub const IMAGES_MAX_TOTAL_BYTES: usize = 8 * 1024 * 1024;
/// Largest image, in pixels (16 megapixels). The role crate does not decode
/// images, so this is a runner's check, not [`check_image`]'s.
pub const IMAGE_MAX_PIXELS: u64 = 16_000_000;

/// An image in its object form: `{content_type, base64}`.
#[derive(Clone, Debug, PartialEq, Eq, Deserialize, Serialize)]
pub struct InlineImage {
    /// One of [`content_types::ALL`].
    pub content_type: String,
    pub base64: String,
}

impl InlineImage {
    pub fn new(content_type: impl Into<String>, base64: impl Into<String>) -> Self {
        Self {
            content_type: content_type.into(),
            base64: base64.into(),
        }
    }
}

/// An image: a `data:` URL string, or `{content_type, base64}`. Remote URLs
/// are never accepted.
#[derive(Clone, Debug, PartialEq, Eq, Deserialize, Serialize)]
#[serde(untagged)]
pub enum Image {
    DataUrl(String),
    Inline(InlineImage),
}

impl Image {
    /// The image's size in decoded bytes, estimated from its base64 length.
    pub fn approx_bytes(&self) -> usize {
        let payload = match self {
            Self::DataUrl(url) => url.split_once(',').map_or("", |(_, data)| data),
            Self::Inline(inline) => &inline.base64,
        };
        payload.trim_end_matches('=').len() * 3 / 4
    }
}

/// Check one image's form and size. The error says why it is refused.
pub fn check_image(image: &Image) -> Result<(), String> {
    match image {
        Image::DataUrl(url) => {
            let Some(rest) = url.strip_prefix("data:") else {
                return Err("an image string must be a data: URL; remote URLs are refused".into());
            };
            let Some((media, _)) = rest.split_once(',') else {
                return Err("a data: URL has no payload".into());
            };
            let content_type = media.split(';').next().unwrap_or("");
            if !content_types::ALL.contains(&content_type) {
                return Err(format!("unsupported image content type {content_type}"));
            }
        }
        Image::Inline(inline) => {
            if !content_types::ALL.contains(&inline.content_type.as_str()) {
                return Err(format!(
                    "unsupported image content type {}",
                    inline.content_type
                ));
            }
        }
    }
    if image.approx_bytes() > IMAGE_MAX_BYTES {
        return Err(format!("an image is at most {IMAGE_MAX_BYTES} bytes"));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::*;
    use crate::errors::INVALID_PARAMS;

    fn options(n: usize) -> IndexMap<String, Value> {
        (0..n).map(|i| (format!("o{i}"), json!("x"))).collect()
    }

    fn levels(n: usize) -> Vec<Value> {
        (0..n).map(|i| json!(format!("level {i}"))).collect()
    }

    #[test]
    fn choice_options_and_score_levels_are_bounded() {
        assert!(check_question("q", &Question::choice("i", options(2))).is_ok());
        assert!(check_question("q", &Question::choice("i", options(255))).is_ok());
        for n in [1, 256] {
            let refusal = check_question("q", &Question::choice("i", options(n))).unwrap_err();
            assert_eq!(refusal.code, INVALID_PARAMS);
            assert_eq!(refusal.field(), Some("questions.q.criteria"));
        }
        assert!(check_question("q", &Question::score("i", levels(2))).is_ok());
        assert!(check_question("q", &Question::score("i", levels(10))).is_ok());
        for (n, limit) in [(1, 2), (11, 10)] {
            let refusal = check_question("q", &Question::score("i", levels(n))).unwrap_err();
            assert_eq!(refusal.field(), Some("questions.q.criteria"));
            assert_eq!(refusal.detail.limit, Some(limit));
        }
    }

    #[test]
    fn ids_types_and_instructions_are_refused_by_field() {
        let refusal = check_question("Bad", &Question::noul("i")).unwrap_err();
        assert_eq!(refusal.field(), Some("questions.Bad"));
        let refusal = check_question("q", &Question::noul("")).unwrap_err();
        assert_eq!(refusal.field(), Some("questions.q.instructions"));
        let refusal =
            check_question("q", &Question::new(QuestionType::parse("rank"), "i")).unwrap_err();
        assert_eq!(refusal.field(), Some("questions.q.type"));
        // The type is refused before the instructions, which mean nothing
        // for a type the runner does not know.
        let refusal =
            check_question("q", &Question::new(QuestionType::parse("rank"), "")).unwrap_err();
        assert_eq!(refusal.field(), Some("questions.q.type"));
        assert!(check_question("q", &Question::noul(json!({"ask": "is it?"}))).is_ok());
        assert!(is_question_id(&"a".repeat(64)));
        assert!(!is_question_id(&"a".repeat(65)));
    }

    #[test]
    fn noul_criteria_is_true_and_false() {
        let good: IndexMap<String, Value> = [
            ("true".to_owned(), json!("yes")),
            ("false".to_owned(), json!("no")),
        ]
        .into();
        let question = Question::noul("i").with_criteria(Criteria::Options(good));
        assert!(check_question("q", &question).is_ok());
        let bad: IndexMap<String, Value> = [("yes".to_owned(), json!("y"))].into();
        let question = Question::noul("i").with_criteria(Criteria::Options(bad));
        assert_eq!(
            check_question("q", &question).unwrap_err().field(),
            Some("questions.q.criteria")
        );
    }

    #[test]
    fn question_type_decodes_open() {
        let question: Question =
            serde_json::from_str(r#"{"type":"rank","instructions":"i"}"#).unwrap();
        assert_eq!(question.kind, QuestionType::Other("rank".into()));
        assert_eq!(
            serde_json::to_string(&question).unwrap(),
            r#"{"type":"rank","instructions":"i"}"#
        );
    }

    #[test]
    fn remote_urls_and_unknown_types_are_refused() {
        assert!(check_image(&Image::DataUrl("https://example.com/a.png".into())).is_err());
        assert!(check_image(&Image::DataUrl("data:image/gif;base64,AAAA".into())).is_err());
        assert!(check_image(&Image::DataUrl("data:image/png;base64,AAAA".into())).is_ok());
        assert!(check_image(&Image::Inline(InlineImage::new("image/webp", "AAAA"))).is_ok());
        assert!(check_image(&Image::Inline(InlineImage::new("image/bmp", "AAAA"))).is_err());
    }
}
