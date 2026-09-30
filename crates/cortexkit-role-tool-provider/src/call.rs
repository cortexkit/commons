//! The body of a tool-call request, with its top-level `call_key`.
//!
//! TODO(subc-protocol 0.26.0): everything in this module except
//! [`check_call_key`] is a LOCAL MIRROR of what subc-protocol 0.26.0 adds to
//! `subc_protocol::tool_call`: the `ToolCallRequest.call_key` field,
//! `CALL_KEY_FIELD`, `CALL_KEY_MAX_LEN`, `CallKeyError` and
//! `validate_call_key`. 0.26.0 is not published yet, so this crate depends on
//! 0.25.2 and adds only that field and its validator on top of the 0.25.2
//! type. When 0.26.0 publishes, bump the dependency, replace
//! [`KeyedToolCallRequest`] with `subc_protocol::tool_call::ToolCallRequest`
//! and re-export the validator from there; the wire bytes do not change.
//! Swap before this crate is merged anywhere it ships.

use serde::{Deserialize, Serialize};
use serde_json::Value;
use subc_protocol::ErrorBody;

pub use subc_protocol::tool_call::ToolCallRequest;

use crate::errors;

/// MIRROR of subc-protocol 0.26.0: the wire name of the call key, and the
/// `field` an `invalid_request` names when the key is malformed.
pub const CALL_KEY_FIELD: &str = "call_key";

/// MIRROR of subc-protocol 0.26.0: the longest accepted `call_key`, in bytes
/// (every accepted byte is one ASCII character).
pub const CALL_KEY_MAX_LEN: usize = 256;

/// MIRROR of subc-protocol 0.26.0: why a `call_key` was refused.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum CallKeyError {
    /// The key was the empty string. An absent key is `None`, never `""`.
    Empty,
    /// The key was longer than [`CALL_KEY_MAX_LEN`] bytes.
    TooLong { length: usize },
    /// The byte at `index` is not printable, non-space ASCII.
    InvalidCharacter { index: usize },
}

impl CallKeyError {
    /// The request field the error is about, always [`CALL_KEY_FIELD`].
    pub fn field(&self) -> &'static str {
        CALL_KEY_FIELD
    }
}

impl std::fmt::Display for CallKeyError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Empty => write!(f, "{CALL_KEY_FIELD} must not be empty"),
            Self::TooLong { length } => write!(
                f,
                "{CALL_KEY_FIELD} is {length} bytes; at most {CALL_KEY_MAX_LEN} are allowed"
            ),
            Self::InvalidCharacter { index } => write!(
                f,
                "{CALL_KEY_FIELD} has a character at byte {index} outside printable ASCII \
                 (0x21 to 0x7E; space is not allowed)"
            ),
        }
    }
}

impl std::error::Error for CallKeyError {}

/// MIRROR of subc-protocol 0.26.0: check a `call_key` is 1 to
/// [`CALL_KEY_MAX_LEN`] characters, each printable ASCII from 0x21 to 0x7E.
///
/// Space is refused because providers compare keys byte for byte and write
/// them into logs, where a leading or trailing space is invisible: two keys
/// that differ only by one would read as the same key and act as different
/// ones.
pub fn validate_call_key(key: &str) -> Result<(), CallKeyError> {
    if key.is_empty() {
        return Err(CallKeyError::Empty);
    }
    if key.len() > CALL_KEY_MAX_LEN {
        return Err(CallKeyError::TooLong { length: key.len() });
    }
    if let Some(index) = key.bytes().position(|byte| !(0x21..=0x7e).contains(&byte)) {
        return Err(CallKeyError::InvalidCharacter { index });
    }
    Ok(())
}

/// MIRROR of subc-protocol 0.26.0's `ToolCallRequest`: the 0.25.2 type plus
/// the top-level `call_key`, a sibling of `name` and `arguments` that is
/// omitted when absent.
///
/// Decoding stays tolerant of unknown members, as the 0.25.2 type is: a
/// provider never refuses a call because a newer consumer added a key.
#[derive(Clone, Debug, Default, Deserialize, PartialEq, Serialize)]
pub struct KeyedToolCallRequest {
    #[serde(flatten)]
    pub request: ToolCallRequest,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub call_key: Option<String>,
}

impl KeyedToolCallRequest {
    /// A call with only a name and arguments: no `tool_call_id`, no
    /// `progress_token` and no `call_key`.
    pub fn new(name: impl Into<String>, arguments: Value) -> Self {
        Self {
            request: ToolCallRequest::new(name, arguments),
            call_key: None,
        }
    }

    pub fn with_call_key(mut self, key: impl Into<String>) -> Self {
        self.call_key = Some(key.into());
        self
    }
}

/// The provider's check of a call's key: a present key that fails
/// [`validate_call_key`] is refused as `invalid_request {field: "call_key"}`.
/// An absent key is not an error; such a call executes, but can be neither
/// deduplicated nor withdrawn.
pub fn check_call_key(request: &KeyedToolCallRequest) -> Result<(), ErrorBody> {
    match request.call_key.as_deref() {
        None => Ok(()),
        Some(key) => validate_call_key(key)
            .map_err(|error| errors::invalid_request(CALL_KEY_FIELD, error.to_string())),
    }
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::*;
    use crate::vectors;

    #[test]
    fn call_key_vectors_validate_as_recorded() {
        let vectors = vectors::load("call-key.json");
        let valid = vectors["valid"].as_array().unwrap();
        let invalid = vectors["invalid"].as_array().unwrap();
        assert!(!valid.is_empty() && !invalid.is_empty());
        for case in valid {
            let key = case["key"].as_str().unwrap();
            assert_eq!(validate_call_key(key), Ok(()), "{key:?} should be valid");
        }
        for case in invalid {
            let key = case["key"].as_str().unwrap();
            let error = validate_call_key(key).expect_err(key);
            let expected = case["error"].as_str().unwrap();
            let kind = match error {
                CallKeyError::Empty => "empty",
                CallKeyError::TooLong { .. } => "too_long",
                CallKeyError::InvalidCharacter { .. } => "invalid_character",
            };
            assert_eq!(kind, expected, "{key:?}");
        }
    }

    #[test]
    fn call_key_is_a_top_level_sibling_omitted_when_absent() {
        let keyed = KeyedToolCallRequest::new("echo", json!({"x": 1})).with_call_key("k:1");
        assert_eq!(
            serde_json::to_value(&keyed).unwrap(),
            json!({"name": "echo", "arguments": {"x": 1}, "call_key": "k:1"})
        );
        let plain = KeyedToolCallRequest::new("echo", json!({}));
        assert_eq!(
            serde_json::to_value(&plain).unwrap(),
            json!({"name": "echo", "arguments": {}})
        );
        let decoded: KeyedToolCallRequest = serde_json::from_value(json!({
            "name": "echo", "arguments": {}, "tool_call_id": "call_0",
            "call_key": "k:2", "newer_member": true
        }))
        .unwrap();
        assert_eq!(decoded.call_key.as_deref(), Some("k:2"));
        assert_eq!(decoded.request.tool_call_id.as_deref(), Some("call_0"));
    }

    #[test]
    fn a_malformed_key_is_invalid_request_naming_call_key() {
        let request = KeyedToolCallRequest::new("echo", json!({})).with_call_key("has space");
        let error = check_call_key(&request).unwrap_err();
        assert_eq!(errors::invalid_request_field(&error), Some(CALL_KEY_FIELD));
        assert_eq!(
            check_call_key(&KeyedToolCallRequest::new("echo", json!({}))),
            Ok(())
        );
    }
}
