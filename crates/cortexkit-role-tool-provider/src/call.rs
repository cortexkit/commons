//! The body of a tool-call request: subc-protocol's `ToolCallRequest`, with
//! its top-level `call_key`, plus the top-level `schema_pin`.
//!
//! `call_key` and its validator come from subc-protocol 0.26.0 and are
//! re-exported unchanged.
//!
//! TODO(subc-protocol 0.27.0): [`PinnedToolCallRequest`], [`SCHEMA_PIN_FIELD`],
//! [`SchemaPinBoundError`] and [`validate_schema_pin`] are a LOCAL MIRROR of
//! what subc-protocol 0.27.0 adds: `ToolCallRequest.schema_pin:
//! Option<String>`, top-level and omitted when `None`, bounded by the same
//! rule as `call_key`. When 0.27.0 publishes, bump the dependency, replace
//! [`PinnedToolCallRequest`] with `subc_protocol::tool_call::ToolCallRequest`
//! and re-export the validator from there; the wire bytes do not change.
//! The pin's CONTENT is this role's, not subc-protocol's, and stays here:
//! see [`SchemaPin`].

use serde::{Deserialize, Serialize};
use serde_json::Value;
use subc_protocol::ErrorBody;

pub use subc_protocol::tool_call::{
    validate_call_key, CallKeyError, ToolCallRequest, CALL_KEY_FIELD, CALL_KEY_MAX_LEN,
};

use crate::errors;

/// MIRROR of subc-protocol 0.27.0: the wire name of the schema pin, and the
/// `field` an `invalid_request` names when the pin is malformed.
pub const SCHEMA_PIN_FIELD: &str = "schema_pin";

/// MIRROR of subc-protocol 0.27.0: why a `schema_pin` fails the shared bound
/// (1 to 256 bytes, each 0x21 to 0x7E), the same bound as `call_key`.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum SchemaPinBoundError {
    Empty,
    TooLong { length: usize },
    InvalidCharacter { index: usize },
}

impl std::fmt::Display for SchemaPinBoundError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Empty => write!(f, "{SCHEMA_PIN_FIELD} must not be empty"),
            Self::TooLong { length } => write!(
                f,
                "{SCHEMA_PIN_FIELD} is {length} bytes; at most {CALL_KEY_MAX_LEN} are allowed"
            ),
            Self::InvalidCharacter { index } => write!(
                f,
                "{SCHEMA_PIN_FIELD} has a character at byte {index} outside printable ASCII \
                 (0x21 to 0x7E; space is not allowed)"
            ),
        }
    }
}

impl std::error::Error for SchemaPinBoundError {}

/// MIRROR of subc-protocol 0.27.0: check a `schema_pin` against the same
/// bound as `call_key`.
pub fn validate_schema_pin(pin: &str) -> Result<(), SchemaPinBoundError> {
    validate_call_key(pin).map_err(|error| match error {
        CallKeyError::Empty => SchemaPinBoundError::Empty,
        CallKeyError::TooLong { length } => SchemaPinBoundError::TooLong { length },
        CallKeyError::InvalidCharacter { index } => SchemaPinBoundError::InvalidCharacter { index },
    })
}

/// MIRROR of subc-protocol 0.27.0's `ToolCallRequest`: the 0.26.0 type plus
/// the top-level `schema_pin`, omitted when absent. Decoding stays tolerant
/// of members it does not know, as the 0.26.0 type is.
#[derive(Clone, Debug, Default, Deserialize, PartialEq, Serialize)]
pub struct PinnedToolCallRequest {
    #[serde(flatten)]
    pub request: ToolCallRequest,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub schema_pin: Option<String>,
}

impl PinnedToolCallRequest {
    /// A call with only a name and arguments.
    pub fn new(name: impl Into<String>, arguments: Value) -> Self {
        Self {
            request: ToolCallRequest::new(name, arguments),
            schema_pin: None,
        }
    }

    pub fn with_call_key(mut self, key: impl Into<String>) -> Self {
        self.request.call_key = Some(key.into());
        self
    }

    pub fn with_schema_pin(mut self, pin: &SchemaPin) -> Self {
        self.schema_pin = Some(pin.encode().expect("pin within bounds"));
        self
    }
}

/// The first number of every schema pin this role defines.
pub const SCHEMA_PIN_PREFIX: &str = "tp1:";

/// What a call's `schema_pin` says: the tool, the catalog `generation` the
/// runner froze, and the tool's `semantics` version.
///
/// Canonical encoding (`tp1`):
///
/// ```text
/// tp1:<pct(tool)>:<pct(generation)>:<semantics>
/// ```
///
/// - `pct` keeps the RFC 3986 unreserved bytes (`A-Z a-z 0-9 - . _ ~`) and
///   writes every other byte of the UTF-8 string as `%` plus two UPPERCASE hex
///   digits. So `:` inside a component is always `%3A` and the three `:`
///   separators are unambiguous.
/// - `semantics` is the decimal integer, no sign, no leading zeros.
/// - `tool` and `generation` are non-empty.
///
/// Exactly one string encodes each pin: [`SchemaPin::parse`] refuses anything
/// [`SchemaPin::encode`] would not produce (lowercase hex, an escaped
/// unreserved byte, a leading zero). The whole string must also pass
/// [`validate_schema_pin`], so a pin longer than 256 bytes cannot be encoded.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SchemaPin {
    pub tool: String,
    pub generation: String,
    pub semantics: u64,
}

/// Why a string is not a canonical `tp1` schema pin.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum SchemaPinError {
    Bound(SchemaPinBoundError),
    /// The pin does not start with [`SCHEMA_PIN_PREFIX`].
    UnknownScheme,
    /// The pin does not have exactly three components.
    Shape,
    EmptyComponent,
    /// A component is not percent-encoded canonically, or does not decode to
    /// UTF-8.
    Encoding,
    /// `semantics` is not a canonical decimal `u64`.
    Semantics,
}

impl std::fmt::Display for SchemaPinError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Bound(error) => error.fmt(f),
            Self::UnknownScheme => {
                write!(f, "{SCHEMA_PIN_FIELD} does not start {SCHEMA_PIN_PREFIX}")
            }
            Self::Shape => write!(
                f,
                "{SCHEMA_PIN_FIELD} must have tool, generation and semantics"
            ),
            Self::EmptyComponent => write!(f, "{SCHEMA_PIN_FIELD} has an empty component"),
            Self::Encoding => write!(f, "{SCHEMA_PIN_FIELD} is not canonically percent-encoded"),
            Self::Semantics => write!(f, "{SCHEMA_PIN_FIELD} semantics is not a canonical integer"),
        }
    }
}

impl std::error::Error for SchemaPinError {}

fn is_unreserved(byte: u8) -> bool {
    byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'.' | b'_' | b'~')
}

fn pct_encode(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    for byte in text.bytes() {
        if is_unreserved(byte) {
            out.push(byte as char);
        } else {
            out.push_str(&format!("%{byte:02X}"));
        }
    }
    out
}

fn pct_decode(text: &str) -> Result<String, SchemaPinError> {
    let bytes = text.as_bytes();
    let mut out = Vec::with_capacity(bytes.len());
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] == b'%' {
            let hex = bytes.get(i + 1..i + 3).ok_or(SchemaPinError::Encoding)?;
            let upper = |b: u8| b.is_ascii_digit() || (b'A'..=b'F').contains(&b);
            if !hex.iter().all(|b| upper(*b)) {
                return Err(SchemaPinError::Encoding);
            }
            let byte = u8::from_str_radix(std::str::from_utf8(hex).unwrap(), 16)
                .map_err(|_| SchemaPinError::Encoding)?;
            if is_unreserved(byte) {
                return Err(SchemaPinError::Encoding);
            }
            out.push(byte);
            i += 3;
        } else if is_unreserved(bytes[i]) {
            out.push(bytes[i]);
            i += 1;
        } else {
            return Err(SchemaPinError::Encoding);
        }
    }
    String::from_utf8(out).map_err(|_| SchemaPinError::Encoding)
}

impl SchemaPin {
    pub fn new(tool: impl Into<String>, generation: impl Into<String>, semantics: u64) -> Self {
        Self {
            tool: tool.into(),
            generation: generation.into(),
            semantics,
        }
    }

    /// The canonical `tp1` string, or why it cannot be sent.
    pub fn encode(&self) -> Result<String, SchemaPinError> {
        if self.tool.is_empty() || self.generation.is_empty() {
            return Err(SchemaPinError::EmptyComponent);
        }
        let pin = format!(
            "{SCHEMA_PIN_PREFIX}{}:{}:{}",
            pct_encode(&self.tool),
            pct_encode(&self.generation),
            self.semantics
        );
        validate_schema_pin(&pin).map_err(SchemaPinError::Bound)?;
        Ok(pin)
    }

    /// Parse a canonical `tp1` pin.
    pub fn parse(pin: &str) -> Result<Self, SchemaPinError> {
        validate_schema_pin(pin).map_err(SchemaPinError::Bound)?;
        let rest = pin
            .strip_prefix(SCHEMA_PIN_PREFIX)
            .ok_or(SchemaPinError::UnknownScheme)?;
        let parts: Vec<&str> = rest.split(':').collect();
        let [tool, generation, semantics] = parts.as_slice() else {
            return Err(SchemaPinError::Shape);
        };
        if tool.is_empty() || generation.is_empty() {
            return Err(SchemaPinError::EmptyComponent);
        }
        if semantics.is_empty()
            || !semantics.bytes().all(|b| b.is_ascii_digit())
            || (semantics.len() > 1 && semantics.starts_with('0'))
        {
            return Err(SchemaPinError::Semantics);
        }
        let semantics = semantics
            .parse::<u64>()
            .map_err(|_| SchemaPinError::Semantics)?;
        Ok(Self {
            tool: pct_decode(tool)?,
            generation: pct_decode(generation)?,
            semantics,
        })
    }
}

/// The provider's check of a call's top-level members: a present but
/// malformed `call_key` is refused as `invalid_request {field: "call_key"}`,
/// and a present `schema_pin` that is not a canonical pin for this very tool
/// as `invalid_request {field: "schema_pin"}`. Absent members are not errors:
/// a call without a key executes but can be neither deduplicated nor
/// withdrawn, and a call without a pin is not checked against a schema.
///
/// Returns the parsed pin, which the provider then compares with its current
/// catalog (refusing `tool_schema_changed` or `tool_semantics_changed`).
pub fn check_call(request: &PinnedToolCallRequest) -> Result<Option<SchemaPin>, ErrorBody> {
    if let Some(key) = request.request.call_key.as_deref() {
        validate_call_key(key)
            .map_err(|error| errors::invalid_request(CALL_KEY_FIELD, error.to_string()))?;
    }
    let Some(pin) = request.schema_pin.as_deref() else {
        return Ok(None);
    };
    let pin = SchemaPin::parse(pin)
        .map_err(|error| errors::invalid_request(SCHEMA_PIN_FIELD, error.to_string()))?;
    if pin.tool != request.request.name {
        return Err(errors::invalid_request(
            SCHEMA_PIN_FIELD,
            format!(
                "the pin names {}, the call {}",
                pin.tool, request.request.name
            ),
        ));
    }
    Ok(Some(pin))
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::*;
    use crate::vectors;

    #[test]
    fn call_key_vectors_agree_with_the_published_validator() {
        let vectors = vectors::load("call-key.json");
        let valid = vectors["valid"].as_array().unwrap();
        let invalid = vectors["invalid"].as_array().unwrap();
        assert!(!valid.is_empty() && !invalid.is_empty());
        for case in valid {
            let key = case["key"].as_str().unwrap();
            assert_eq!(validate_call_key(key), Ok(()), "{key:?} should be valid");
            assert_eq!(validate_schema_pin(key), Ok(()), "same bound for pins");
        }
        for case in invalid {
            let key = case["key"].as_str().unwrap();
            let kind = match validate_call_key(key).expect_err(key) {
                CallKeyError::Empty => "empty",
                CallKeyError::TooLong { .. } => "too_long",
                CallKeyError::InvalidCharacter { .. } => "invalid_character",
            };
            assert_eq!(kind, case["error"].as_str().unwrap(), "{key:?}");
            assert!(validate_schema_pin(key).is_err(), "same bound for pins");
        }
    }

    #[test]
    fn schema_pin_vectors_encode_and_parse_as_recorded() {
        let vectors = vectors::load("schema-pin.json");
        for case in vectors["canonical"].as_array().unwrap() {
            let pin = SchemaPin::new(
                case["tool"].as_str().unwrap(),
                case["generation"].as_str().unwrap(),
                case["semantics"].as_u64().unwrap(),
            );
            let wire = case["pin"].as_str().unwrap();
            assert_eq!(pin.encode().as_deref(), Ok(wire), "{}", case["name"]);
            assert_eq!(SchemaPin::parse(wire), Ok(pin), "{}", case["name"]);
        }
        for case in vectors["refused"].as_array().unwrap() {
            assert!(
                SchemaPin::parse(case["pin"].as_str().unwrap()).is_err(),
                "{}",
                case["name"]
            );
        }
    }

    #[test]
    fn pin_and_key_are_top_level_siblings_omitted_when_absent() {
        let call = PinnedToolCallRequest::new("echo", json!({"x": 1}))
            .with_call_key("k:1")
            .with_schema_pin(&SchemaPin::new("echo", "g7", 2));
        assert_eq!(
            serde_json::to_value(&call).unwrap(),
            json!({"name": "echo", "arguments": {"x": 1}, "call_key": "k:1", "schema_pin": "tp1:echo:g7:2"})
        );
        assert_eq!(
            serde_json::to_value(PinnedToolCallRequest::new("echo", json!({}))).unwrap(),
            json!({"name": "echo", "arguments": {}})
        );
    }

    #[test]
    fn check_call_names_the_offending_field() {
        let bad_key = PinnedToolCallRequest::new("echo", json!({})).with_call_key("has space");
        let error = check_call(&bad_key).unwrap_err();
        assert_eq!(errors::invalid_request_field(&error), Some(CALL_KEY_FIELD));

        let mut other_tool = PinnedToolCallRequest::new("echo", json!({}));
        other_tool.schema_pin = Some("tp1:grep:g:1".into());
        let error = check_call(&other_tool).unwrap_err();
        assert_eq!(
            errors::invalid_request_field(&error),
            Some(SCHEMA_PIN_FIELD)
        );

        let good = PinnedToolCallRequest::new("echo", json!({}))
            .with_schema_pin(&SchemaPin::new("echo", "g", 0));
        assert_eq!(check_call(&good), Ok(Some(SchemaPin::new("echo", "g", 0))));
    }
}
