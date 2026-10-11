//! Opt-in `content/v1` tool reply bodies, shared by providers and consumers.

use serde::{Deserialize, Deserializer, Serialize, Serializer};
use serde_json::{Map, Value};
use std::collections::HashSet;

/// A serialized reply must be strictly smaller than this bound. The daemon's
/// 4 MiB connection limit needs headroom for framing and other traffic.
pub const MAX_RESULT_BYTES: usize = 3 * 1024 * 1024;

/// Open per-tool result-format vocabulary. Unknown strings are preserved.
#[derive(Clone, Debug, PartialEq, Eq, Deserialize, Serialize)]
#[serde(from = "String", into = "String")]
#[non_exhaustive]
pub enum ResultFormat {
    ContentV1,
    Unknown(String),
}

impl ResultFormat {
    pub fn as_str(&self) -> &str {
        match self {
            Self::ContentV1 => "content/v1",
            Self::Unknown(value) => value,
        }
    }
}

impl From<String> for ResultFormat {
    fn from(value: String) -> Self {
        match value.as_str() {
            "content/v1" => Self::ContentV1,
            _ => Self::Unknown(value),
        }
    }
}

impl From<ResultFormat> for String {
    fn from(value: ResultFormat) -> Self {
        value.as_str().to_owned()
    }
}

/// A `content/v1` reply, using MCP's CallToolResult member names.
///
/// Consumers build model input from `content` only, never from
/// `structuredContent`, which is for programs. Content order is significant;
/// keep every part, rendering unknown types as a one-line unsupported-content
/// note rather than silently dropping them. Validate before sending and after
/// receiving with [`validate`].
#[derive(Clone, Debug, PartialEq, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
#[non_exhaustive]
pub struct ToolResult {
    pub content: Vec<ContentPart>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub structured_content: Option<Map<String, Value>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub is_error: Option<bool>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub error_code: Option<String>,
}

impl ToolResult {
    pub fn new(content: Vec<ContentPart>) -> Self {
        Self {
            content,
            structured_content: None,
            is_error: None,
            error_code: None,
        }
    }

    pub fn with_structured_content(mut self, content: Map<String, Value>) -> Self {
        self.structured_content = Some(content);
        self
    }

    /// Mark an error with the provider's stable snake_case code.
    pub fn with_error(mut self, code: impl Into<String>) -> Self {
        self.is_error = Some(true);
        self.error_code = Some(code.into());
        self
    }
}

/// One ordered model-facing content part. Unknown types retain their raw JSON.
#[derive(Clone, Debug, PartialEq)]
#[non_exhaustive]
pub enum ContentPart {
    /// Text the model sees verbatim.
    Text {
        text: String,
    },
    Image(ImageContent),
    Unknown(Value),
}

impl ContentPart {
    pub fn text(text: impl Into<String>) -> Self {
        Self::Text { text: text.into() }
    }

    pub fn image(image: ImageContent) -> Self {
        Self::Image(image)
    }
}

/// An image with raw standard base64 data, never a `data:` URL.
#[derive(Clone, Debug, PartialEq, Eq, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
#[non_exhaustive]
pub struct ImageContent {
    pub mime_type: String,
    pub data: String,
    /// Positive pixel dimensions.
    pub width: u64,
    pub height: u64,
    /// Provider wording for a consumer that cannot deliver the image to its model.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub alt: Option<String>,
    /// Opaque provider identifier, unique within the reply when present.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub id: Option<String>,
}

impl ImageContent {
    pub fn new(
        mime_type: impl Into<String>,
        data: impl Into<String>,
        width: u64,
        height: u64,
    ) -> Self {
        Self {
            mime_type: mime_type.into(),
            data: data.into(),
            width,
            height,
            alt: None,
            id: None,
        }
    }

    pub fn with_alt(mut self, alt: impl Into<String>) -> Self {
        self.alt = Some(alt.into());
        self
    }

    pub fn with_id(mut self, id: impl Into<String>) -> Self {
        self.id = Some(id.into());
        self
    }
}

#[derive(Serialize)]
#[serde(tag = "type", rename_all = "lowercase")]
enum PartRef<'a> {
    Text { text: &'a str },
    Image(&'a ImageContent),
}

impl Serialize for ContentPart {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        match self {
            Self::Text { text } => PartRef::Text { text }.serialize(serializer),
            Self::Image(image) => PartRef::Image(image).serialize(serializer),
            Self::Unknown(value) => value.serialize(serializer),
        }
    }
}

impl<'de> Deserialize<'de> for ContentPart {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        let value = Value::deserialize(deserializer)?;
        match value.get("type").and_then(Value::as_str) {
            Some("text") => {
                #[derive(Deserialize)]
                struct Text {
                    text: String,
                }
                serde_json::from_value::<Text>(value)
                    .map(|part| Self::Text { text: part.text })
                    .map_err(serde::de::Error::custom)
            }
            Some("image") => serde_json::from_value(value)
                .map(Self::Image)
                .map_err(serde::de::Error::custom),
            Some(_) => Ok(Self::Unknown(value)),
            None => Err(serde::de::Error::custom(
                "content part type must be a string",
            )),
        }
    }
}

/// The member path and reason for a malformed `content/v1` reply.
#[derive(Clone, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub struct ResultProblem {
    pub member: String,
    pub reason: &'static str,
}

impl std::fmt::Display for ResultProblem {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}: {}", self.member, self.reason)
    }
}

impl std::error::Error for ResultProblem {}

fn problem(member: impl Into<String>, reason: &'static str) -> ResultProblem {
    ResultProblem {
        member: member.into(),
        reason,
    }
}

/// Validate a decoded or constructed result before sending or on receipt.
/// Providers exceeding the size bound downsize or refuse by name, never truncate.
pub fn validate(result: &ToolResult) -> Result<(), ResultProblem> {
    if result.content.is_empty() {
        return Err(problem("content", "must contain at least one part"));
    }
    match (result.is_error, result.error_code.as_deref()) {
        (Some(true), None) => return Err(problem("errorCode", "required when isError is true")),
        (Some(true), Some(code)) if !is_snake_case(code) => {
            return Err(problem("errorCode", "must be a nonempty snake_case code"));
        }
        (Some(true), Some(_)) | (_, None) => {}
        (_, Some(_)) => return Err(problem("errorCode", "requires isError to be true")),
    }
    let mut ids = HashSet::new();
    for (index, part) in result.content.iter().enumerate() {
        let member = |name: &str| format!("content[{index}].{name}");
        match part {
            ContentPart::Text { .. } => {}
            ContentPart::Image(image) => {
                if image.data.starts_with("data:") {
                    return Err(problem(member("data"), "must not have a data: URL prefix"));
                }
                if !is_standard_base64(&image.data) {
                    return Err(problem(member("data"), "must be raw standard base64"));
                }
                if !["image/png", "image/jpeg", "image/webp", "image/gif"]
                    .contains(&image.mime_type.as_str())
                {
                    return Err(problem(
                        member("mimeType"),
                        "must be image/png, image/jpeg, image/webp or image/gif",
                    ));
                }
                if image.width == 0 {
                    return Err(problem(member("width"), "must be positive pixels"));
                }
                if image.height == 0 {
                    return Err(problem(member("height"), "must be positive pixels"));
                }
                if let Some(id) = &image.id {
                    if !ids.insert(id) {
                        return Err(problem(member("id"), "must be unique within the reply"));
                    }
                }
            }
            ContentPart::Unknown(value) => match value.get("type").and_then(Value::as_str) {
                Some(kind) if kind != "text" && kind != "image" => {}
                _ => {
                    return Err(problem(
                        member("type"),
                        "Unknown must carry an unknown string type",
                    ))
                }
            },
        }
    }
    let bytes =
        serde_json::to_vec(result).map_err(|_| problem("reply", "could not serialize reply"))?;
    if bytes.len() >= MAX_RESULT_BYTES {
        return Err(problem("reply", "serialized reply must be under 3 MiB"));
    }
    Ok(())
}

fn is_snake_case(code: &str) -> bool {
    code.as_bytes().first().is_some_and(u8::is_ascii_lowercase)
        && code.split('_').all(|word| {
            !word.is_empty()
                && word
                    .bytes()
                    .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit())
        })
}

// Check the standard alphabet, padding and unused trailing bits without
// allocating a decoded image. Standard base64 uses complete four-byte groups.
fn is_standard_base64(data: &str) -> bool {
    fn sextet(b: u8) -> Option<u8> {
        match b {
            b'A'..=b'Z' => Some(b - b'A'),
            b'a'..=b'z' => Some(b - b'a' + 26),
            b'0'..=b'9' => Some(b - b'0' + 52),
            b'+' => Some(62),
            b'/' => Some(63),
            _ => None,
        }
    }
    let bytes = data.as_bytes();
    if !bytes.len().is_multiple_of(4) {
        return false;
    }
    let padding = bytes.iter().rev().take_while(|&&b| b == b'=').count();
    if padding > 2 {
        return false;
    }
    let body = &bytes[..bytes.len() - padding];
    if !body.iter().all(|&b| sextet(b).is_some()) {
        return false;
    }
    match padding {
        1 => body
            .last()
            .and_then(|&b| sextet(b))
            .is_some_and(|b| b & 3 == 0),
        2 => body
            .last()
            .and_then(|&b| sextet(b))
            .is_some_and(|b| b & 15 == 0),
        _ => true,
    }
}
