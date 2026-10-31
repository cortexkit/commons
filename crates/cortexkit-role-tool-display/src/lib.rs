//! Provider-authored `tool-display/v1` data for transcript readers.
//!
//! The normative contract is in `CONTRACT.md`. These types perform no routing,
//! persistence, template interpretation or transcript joining.

#![forbid(unsafe_code)]

pub mod schema;
#[cfg(feature = "test-vectors")]
pub mod vectors;

use serde::{Deserialize, Deserializer, Serialize, Serializer};
use serde_json::{json, Map, Value};
use subc_protocol::{
    tool_call::{validate_call_key, validate_schema_pin},
    ErrorBody,
};

pub use schema::{FieldKind, RecordSchema};
pub use subc_protocol::Principal;

pub const CONTRACT: &str = "tool-display/v1";
/// Declared by the module serving app-reader operations, not display storage.
pub const APP_SERVER_CAPABILITY: &str = "app-server/v1";
/// Independent promise to serve display writes and fetch provider views.
pub const TOOL_DISPLAY_CAPABILITY: &str = "tool-display/v1";
pub const DISPLAY_WRITE: &str = "display.write";
/// The `role.describe` member containing [`DisplayLimits`].
pub const DESCRIBE_GROUP: &str = "tool_display";
pub const MAX_RECORD_BYTES: u64 = 16 * 1024;
pub const MAX_COMPLETION_BYTES: u64 = 16 * 1024;
pub const MAX_DETAIL_BYTES: u64 = 3 * 1024 * 1024;
pub const MAX_WRITE_FRAME_BYTES: u64 = 4 * 1024 * 1024;
pub const INVALID_PARAMS: &str = "invalid_params";
pub const DISPLAY_WRITE_CONFLICT: &str = "display_write_conflict";
pub const DISPLAY_TOO_LARGE: &str = "display_too_large";
pub const DISPLAY_WRITER_UNVERIFIED: &str = "display_writer_unverified";
/// The existing retryable route refusal used while the display server is not ready.
pub const NOT_READY: &str = subc_protocol::error_codes::MODULE_WARMING;

/// Readers retain unknown kinds; a server admits only the three defined kinds.
#[derive(Clone, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub enum DisplayKind {
    Record,
    Detail,
    Completion,
    Unknown(String),
}

impl DisplayKind {
    pub fn as_str(&self) -> &str {
        match self {
            Self::Record => "record",
            Self::Detail => "detail",
            Self::Completion => "completion",
            Self::Unknown(value) => value,
        }
    }
}

impl Serialize for DisplayKind {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        serializer.serialize_str(self.as_str())
    }
}

impl<'de> Deserialize<'de> for DisplayKind {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        Ok(match String::deserialize(deserializer)?.as_str() {
            "record" => Self::Record,
            "detail" => Self::Detail,
            "completion" => Self::Completion,
            value => Self::Unknown(value.to_owned()),
        })
    }
}

/// Params only: the display server stores every kind with the route's attested
/// writer, never a writer claimed in a field.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[non_exhaustive]
pub struct DisplayWrite {
    pub runner: Principal,
    pub call_key: String,
    pub kind: DisplayKind,
    pub schema_pin: String,
    pub body: Map<String, Value>,
}

impl DisplayWrite {
    pub fn new(
        runner: Principal,
        call_key: impl Into<String>,
        kind: DisplayKind,
        schema_pin: impl Into<String>,
        body: Map<String, Value>,
    ) -> Self {
        Self {
            runner,
            call_key: call_key.into(),
            kind,
            schema_pin: schema_pin.into(),
            body,
        }
    }
}

/// Served limits in `role.describe.tool_display`. Clients use these, not ceilings.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[non_exhaustive]
pub struct DisplayLimits {
    pub max_record_bytes: u64,
    pub max_detail_bytes: u64,
    pub max_completion_bytes: u64,
    pub loading_window_ms: u64,
}

impl DisplayLimits {
    pub fn new(
        max_record_bytes: u64,
        max_detail_bytes: u64,
        max_completion_bytes: u64,
        loading_window_ms: u64,
    ) -> Self {
        Self {
            max_record_bytes,
            max_detail_bytes,
            max_completion_bytes,
            loading_window_ms,
        }
    }

    pub fn validate(&self) -> Result<(), InvalidParams> {
        for (field, value, ceiling) in [
            ("max_record_bytes", self.max_record_bytes, MAX_RECORD_BYTES),
            ("max_detail_bytes", self.max_detail_bytes, MAX_DETAIL_BYTES),
            (
                "max_completion_bytes",
                self.max_completion_bytes,
                MAX_COMPLETION_BYTES,
            ),
        ] {
            if value > ceiling {
                return Err(InvalidParams::new(field));
            }
        }
        Ok(())
    }

    pub fn max_bytes(&self, kind: &DisplayKind) -> Result<u64, InvalidParams> {
        match kind {
            DisplayKind::Record => Ok(self.max_record_bytes),
            DisplayKind::Detail => Ok(self.max_detail_bytes),
            DisplayKind::Completion => Ok(self.max_completion_bytes),
            DisplayKind::Unknown(_) => Err(InvalidParams::new("kind")),
        }
    }
}

/// A validator failure naming the invalid field, without echoing its value.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[non_exhaustive]
pub struct InvalidParams {
    pub field: String,
}

impl InvalidParams {
    pub fn new(field: impl Into<String>) -> Self {
        Self {
            field: field.into(),
        }
    }
    pub fn refusal(&self) -> ErrorBody {
        ErrorBody::new(INVALID_PARAMS, "invalid display params")
            .with_detail(json!({"field": self.field}))
    }
}

impl std::fmt::Display for InvalidParams {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "invalid display field: {}", self.field)
    }
}
impl std::error::Error for InvalidParams {}

/// RFC 8785 JCS bytes, used both for byte limits and idempotency comparisons.
pub fn canonical_body(body: &Map<String, Value>) -> Result<Vec<u8>, serde_json::Error> {
    serde_jcs::to_vec(body)
}

/// An unambiguous JCS tuple key: the writer the daemon attested on the
/// `display.write` route, then the runner, call key and kind. The writer comes
/// from the route, never from the params, so a write from another module can't
/// claim this key first and block the real provider's write. The schema pin is
/// deliberately absent.
pub fn idempotency_key(
    writer: &Principal,
    write: &DisplayWrite,
) -> Result<Vec<u8>, serde_json::Error> {
    serde_jcs::to_vec(&(writer, &write.runner, &write.call_key, &write.kind))
}

/// Check routing-independent params and limits before storing anything.
/// Return the bytes that must be stored. Record/completion schema validation is
/// separate: views are fetched off the write path and checked when joining.
pub fn validate_write(write: &DisplayWrite, limits: &DisplayLimits) -> Result<Vec<u8>, ErrorBody> {
    limits.validate().map_err(|error| error.refusal())?;
    let max_bytes = limits
        .max_bytes(&write.kind)
        .map_err(|error| error.refusal())?;
    validate_call_key(&write.call_key).map_err(|_| InvalidParams::new("call_key").refusal())?;
    validate_schema_pin(&write.schema_pin)
        .map_err(|_| InvalidParams::new("schema_pin").refusal())?;
    let bytes = canonical_body(&write.body).map_err(|_| InvalidParams::new("body").refusal())?;
    if bytes.len() as u64 > max_bytes {
        return Err(too_large(&write.kind, max_bytes));
    }
    if write.kind == DisplayKind::Detail {
        schema::validate_detail(&write.body).map_err(|error| error.refusal())?;
    }
    Ok(bytes)
}

/// A bare successful `display.write` answer.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "result", rename_all = "snake_case")]
#[non_exhaustive]
pub enum WriteReply {
    Stored,
    AlreadyStored,
}

/// Compare canonical body bytes for an already selected idempotency key.
/// The caller persists the first bytes atomically with returning `stored`.
pub fn write_outcome(previous: Option<&[u8]>, incoming: &[u8]) -> Result<WriteReply, ErrorBody> {
    match previous {
        None => Ok(WriteReply::Stored),
        Some(bytes) if bytes == incoming => Ok(WriteReply::AlreadyStored),
        Some(_) => Err(ErrorBody::new(
            DISPLAY_WRITE_CONFLICT,
            "display body differs from the first write",
        )),
    }
}

/// Admit a write only from a daemon-verified module. `writer` is the route's
/// attested opener. A `direct` or `unverified` opener can't serve a catalog, so
/// no join could accept its writes; the display server refuses them before
/// storing anything.
pub fn check_writer(writer: &Principal) -> Result<(), ErrorBody> {
    match writer {
        Principal::Reserved { .. } => Ok(()),
        _ => Err(ErrorBody::new(
            DISPLAY_WRITER_UNVERIFIED,
            "display writes come only from daemon-verified modules",
        )),
    }
}

pub fn too_large(kind: &DisplayKind, max_bytes: u64) -> ErrorBody {
    ErrorBody::new(DISPLAY_TOO_LARGE, "display body exceeds the served limit")
        .with_detail(json!({"kind": kind, "max_bytes": max_bytes}))
}

pub fn not_ready() -> ErrorBody {
    ErrorBody::new(NOT_READY, "display server is not ready")
}
