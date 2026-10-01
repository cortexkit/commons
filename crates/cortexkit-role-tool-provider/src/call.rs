//! The body of a tool-call request: subc-protocol's `ToolCallRequest`, with
//! its top-level `call_key` and `schema_pin`.
//!
//! The request type, both fields' wire names and bounds, and their shape
//! validators come from `subc_protocol::tool_call` and are re-exported
//! unchanged, so a provider and a consumer that both depend on subc-protocol
//! pass the same type across. What a `schema_pin` HOLDS is this role's, not
//! subc-protocol's, and is defined here: see [`SchemaPin`].

use subc_protocol::ErrorBody;

pub use subc_protocol::tool_call::{
    validate_call_key, validate_schema_pin, CallKeyError, OpaqueFieldError, ToolCallRequest,
    CALL_KEY_FIELD, CALL_KEY_MAX_LEN, SCHEMA_PIN_FIELD, SCHEMA_PIN_MAX_LEN,
};

use crate::{catalog::is_schema_digest, errors};

/// Builder helpers for [`ToolCallRequest`], which subc-protocol defines with
/// only a `new` (a call with only a name and arguments). A trait because the
/// type is foreign to this crate: `ToolCallRequest::new(..).with_call_key(..)`
/// works once the trait is in scope.
pub trait ToolCallRequestExt: Sized {
    /// Set the top-level `call_key`. The key is not checked here; a provider
    /// checks it with [`check_call`].
    fn with_call_key(self, key: impl Into<String>) -> Self;

    /// Set the top-level `schema_pin` to the canonical `tp1` encoding of
    /// `pin`.
    ///
    /// # Panics
    ///
    /// If `pin` cannot be encoded (see [`SchemaPin::encode`]).
    fn with_schema_pin(self, pin: &SchemaPin) -> Self;
}

impl ToolCallRequestExt for ToolCallRequest {
    fn with_call_key(mut self, key: impl Into<String>) -> Self {
        self.call_key = Some(key.into());
        self
    }

    fn with_schema_pin(mut self, pin: &SchemaPin) -> Self {
        self.schema_pin = Some(pin.encode().expect("pin within bounds"));
        self
    }
}

/// The first number of every schema pin this role defines.
pub const SCHEMA_PIN_PREFIX: &str = "tp1:";

/// What a call's `schema_pin` says: the tool, its `schema_digest` and its
/// `semantics` version, as the runner froze them from the catalog.
///
/// The pin holds the argument schema's structure and the behaviour version,
/// never the whole catalog or any description text, so a provider deploy that
/// only rewrites descriptions keeps every pin valid.
///
/// Canonical encoding (`tp1`):
///
/// ```text
/// tp1:<pct(tool)>:<schema_digest>:<semantics>
/// ```
///
/// - `pct` keeps the RFC 3986 unreserved bytes (`A-Z a-z 0-9 - . _ ~`) and
///   writes every other byte of the UTF-8 string as `%` plus two UPPERCASE hex
///   digits. So `:` inside the tool name is always `%3A` and the three `:`
///   separators are unambiguous.
/// - `schema_digest` is the tool's digest as the catalog gives it: 64
///   lowercase hex characters.
/// - `semantics` is the decimal integer, no sign, no leading zeros.
/// - `tool` is non-empty.
///
/// Exactly one string encodes each pin: [`SchemaPin::parse`] refuses anything
/// [`SchemaPin::encode`] would not produce (lowercase percent hex, an escaped
/// unreserved byte, an uppercase digest, a leading zero). The whole string
/// must also pass [`validate_schema_pin`].
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SchemaPin {
    pub tool: String,
    pub schema_digest: String,
    pub semantics: u64,
}

/// Why a string is not a canonical `tp1` schema pin.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum SchemaPinError {
    /// The pin fails the shared opaque-field bound ([`validate_schema_pin`]).
    Bound(OpaqueFieldError),
    /// The pin does not start with [`SCHEMA_PIN_PREFIX`].
    UnknownScheme,
    /// The pin does not have exactly three components.
    Shape,
    EmptyComponent,
    /// The tool name is not percent-encoded canonically, or does not decode
    /// to UTF-8.
    Encoding,
    /// The schema digest is not 64 lowercase hex characters.
    Digest,
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
                "{SCHEMA_PIN_FIELD} must have tool, schema digest and semantics"
            ),
            Self::EmptyComponent => write!(f, "{SCHEMA_PIN_FIELD} has an empty component"),
            Self::Encoding => write!(f, "{SCHEMA_PIN_FIELD} is not canonically percent-encoded"),
            Self::Semantics => write!(f, "{SCHEMA_PIN_FIELD} semantics is not a canonical integer"),
            Self::Digest => write!(
                f,
                "{SCHEMA_PIN_FIELD} schema digest is not 64 lowercase hex characters"
            ),
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
    pub fn new(tool: impl Into<String>, schema_digest: impl Into<String>, semantics: u64) -> Self {
        Self {
            tool: tool.into(),
            schema_digest: schema_digest.into(),
            semantics,
        }
    }

    /// The canonical `tp1` string, or why it cannot be sent.
    pub fn encode(&self) -> Result<String, SchemaPinError> {
        if self.tool.is_empty() {
            return Err(SchemaPinError::EmptyComponent);
        }
        if !is_schema_digest(&self.schema_digest) {
            return Err(SchemaPinError::Digest);
        }
        let pin = format!(
            "{SCHEMA_PIN_PREFIX}{}:{}:{}",
            pct_encode(&self.tool),
            self.schema_digest,
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
        let [tool, digest, semantics] = parts.as_slice() else {
            return Err(SchemaPinError::Shape);
        };
        if tool.is_empty() {
            return Err(SchemaPinError::EmptyComponent);
        }
        if !is_schema_digest(digest) {
            return Err(SchemaPinError::Digest);
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
            schema_digest: (*digest).to_owned(),
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
pub fn check_call(request: &ToolCallRequest) -> Result<Option<SchemaPin>, ErrorBody> {
    if let Some(key) = request.call_key.as_deref() {
        validate_call_key(key)
            .map_err(|error| errors::invalid_request(CALL_KEY_FIELD, error.to_string()))?;
    }
    let Some(pin) = request.schema_pin.as_deref() else {
        return Ok(None);
    };
    let pin = SchemaPin::parse(pin)
        .map_err(|error| errors::invalid_request(SCHEMA_PIN_FIELD, error.to_string()))?;
    if pin.tool != request.name {
        return Err(errors::invalid_request(
            SCHEMA_PIN_FIELD,
            format!("the pin names {}, the call {}", pin.tool, request.name),
        ));
    }
    Ok(Some(pin))
}

#[cfg(test)]
mod tests {
    use serde_json::{json, Value};

    use super::*;
    use crate::vectors;

    const D: &str = "839469d5e28de1286cbd75382e5329eb7002d401575fc6c02c4aa17694939b3f";

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
            let kind = |error: OpaqueFieldError| match error {
                OpaqueFieldError::Empty { .. } => "empty",
                OpaqueFieldError::TooLong { .. } => "too_long",
                OpaqueFieldError::InvalidCharacter { .. } => "invalid_character",
            };
            let key_error = validate_call_key(key).expect_err(key);
            assert_eq!(key_error.field(), CALL_KEY_FIELD, "{key:?}");
            assert_eq!(kind(key_error), case["error"].as_str().unwrap(), "{key:?}");
            let pin_error = validate_schema_pin(key).expect_err("same bound for pins");
            assert_eq!(pin_error.field(), SCHEMA_PIN_FIELD, "{key:?}");
            assert_eq!(kind(pin_error), case["error"].as_str().unwrap(), "{key:?}");
        }
    }

    #[test]
    fn schema_pin_vectors_encode_and_parse_as_recorded() {
        let vectors = vectors::load("schema-pin.json");
        for case in vectors["canonical"].as_array().unwrap() {
            let pin = SchemaPin::new(
                case["tool"].as_str().unwrap(),
                case["schema_digest"].as_str().unwrap(),
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
        let call = ToolCallRequest::new("echo", json!({"x": 1}))
            .with_call_key("k:1")
            .with_schema_pin(&SchemaPin::new("echo", D, 2));
        assert_eq!(
            serde_json::to_value(&call).unwrap(),
            json!({"name": "echo", "arguments": {"x": 1}, "call_key": "k:1", "schema_pin": format!("tp1:echo:{D}:2")})
        );
        assert_eq!(
            serde_json::to_value(ToolCallRequest::new("echo", json!({}))).unwrap(),
            json!({"name": "echo", "arguments": {}})
        );
    }

    /// The pin travels in subc-protocol's own `ToolCallRequest.schema_pin`,
    /// reached through this crate's re-export: built as a struct literal (so a
    /// field added upstream stops this compiling), sent as a top-level member,
    /// decoded back into the same type, and accepted by [`check_call`].
    #[test]
    fn schema_pin_round_trips_through_the_subc_protocol_request() {
        let pin = SchemaPin::new("fs:read", D, 3);
        let wire_pin = pin.encode().unwrap();
        assert_eq!(wire_pin, format!("tp1:fs%3Aread:{D}:3"));
        let call = crate::call::ToolCallRequest {
            name: "fs:read".into(),
            arguments: json!({"path": "a"}),
            tool_call_id: None,
            progress_token: None,
            call_key: Some("k:7".into()),
            schema_pin: Some(wire_pin.clone()),
            origin: None,
        };

        let encoded = serde_json::to_vec(&call).unwrap();
        let value: Value = serde_json::from_slice(&encoded).unwrap();
        assert_eq!(value[SCHEMA_PIN_FIELD], json!(wire_pin));
        assert!(value["arguments"].get(SCHEMA_PIN_FIELD).is_none());

        let decoded: subc_protocol::tool_call::ToolCallRequest =
            serde_json::from_slice(&encoded).unwrap();
        assert_eq!(decoded, call);
        assert_eq!(decoded.schema_pin.as_deref(), Some(wire_pin.as_str()));
        assert_eq!(check_call(&decoded), Ok(Some(pin)));
    }

    #[test]
    fn check_call_names_the_offending_field() {
        let bad_key = ToolCallRequest::new("echo", json!({})).with_call_key("has space");
        let error = check_call(&bad_key).unwrap_err();
        assert_eq!(errors::invalid_request_field(&error), Some(CALL_KEY_FIELD));

        let mut other_tool = ToolCallRequest::new("echo", json!({}));
        other_tool.schema_pin = Some(format!("tp1:grep:{D}:1"));
        let error = check_call(&other_tool).unwrap_err();
        assert_eq!(
            errors::invalid_request_field(&error),
            Some(SCHEMA_PIN_FIELD)
        );

        let good =
            ToolCallRequest::new("echo", json!({})).with_schema_pin(&SchemaPin::new("echo", D, 0));
        assert_eq!(check_call(&good), Ok(Some(SchemaPin::new("echo", D, 0))));
    }
}
