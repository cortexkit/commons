//! Data and pure validators for commands a person runs from an app.
//! See CONTRACT.md for producer duties that cannot be checked from a body.

mod command;
mod facts;
mod request;
mod wire;

pub use command::*;
pub use facts::*;
pub use request::*;
pub use subc_protocol::{ErrorBody, Principal};
pub use wire::*;

use serde::{Deserialize, Serialize};
use serde_json::json;

pub const CONTRACT: &str = "user-commands/v1";
pub const PROVIDES: &str = CONTRACT;
pub const ADMITTED_CALLER: &str = "reserved:occipital";
pub const MODULE_OPS: &[&str] = &["commands.describe", "commands.preview", "commands.run"];
pub const APP_OPS: &[&str] = &["commands.list", "commands.preview", "commands.run"];
pub const INVALID_PARAMS: &str = "invalid_params";
pub const UNKNOWN_COMMAND: &str = "unknown_command";
pub const UNKNOWN_SUBJECT: &str = "unknown_subject";
pub const CALLER_NOT_ADMITTED: &str = "caller_not_admitted";

/// A field diagnostic. Fact diagnostics retain the app library's error class;
/// serving implementations map every invalid body to `invalid_params` on wire.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[non_exhaustive]
pub struct ValidationError {
    pub code: String,
    pub field: String,
}

impl ValidationError {
    pub fn new(field: impl Into<String>) -> Self {
        Self::classified(INVALID_PARAMS, field)
    }

    pub fn classified(code: impl Into<String>, field: impl Into<String>) -> Self {
        Self {
            code: code.into(),
            field: field.into(),
        }
    }

    pub fn refusal(&self) -> ErrorBody {
        ErrorBody::new(INVALID_PARAMS, "invalid command params")
            .with_detail(json!({"field": self.field}))
    }

    pub(crate) fn at(mut self, prefix: &str) -> Self {
        self.field = format!("{prefix}.{}", self.field);
        self
    }
}

impl std::fmt::Display for ValidationError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}: {}", self.code, self.field)
    }
}
impl std::error::Error for ValidationError {}

pub(crate) fn present<'de, D, T>(deserializer: D) -> Result<Option<T>, D::Error>
where
    D: serde::Deserializer<'de>,
    T: Deserialize<'de>,
{
    T::deserialize(deserializer).map(Some)
}

/// Check the daemon-attested opener, never a fact forwarded in `caller`.
/// `first_party_required` becomes true when the daemon mark is deployed.
pub fn check_admission(
    principal: &Principal,
    first_party: bool,
    first_party_required: bool,
) -> Result<(), ErrorBody> {
    if !matches!(principal, Principal::Reserved { module_id } if module_id == "occipital")
        || (first_party_required && !first_party)
    {
        return Err(ErrorBody::new(CALLER_NOT_ADMITTED, "caller not admitted"));
    }
    Ok(())
}

pub fn validate_code(value: &str) -> bool {
    (1..=64).contains(&value.len())
        && value
            .bytes()
            .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'_')
}

pub fn validate_request_id(value: &str) -> bool {
    (1..=64).contains(&value.len())
        && value
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || b == b'_' || b == b'-')
}

pub fn validate_version(value: &str) -> bool {
    (1..=128).contains(&value.len()) && value.bytes().all(|b| (0x20..=0x7e).contains(&b))
}

pub(crate) fn text(value: &str, max: usize) -> bool {
    (1..=max).contains(&value.chars().count())
}

pub(crate) fn require(valid: bool, field: &str) -> Result<(), ValidationError> {
    if valid {
        Ok(())
    } else {
        Err(ValidationError::new(field))
    }
}
