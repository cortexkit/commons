//! Error codes a tool provider answers with, carried as the body of an
//! `ERROR` frame (`subc_protocol::ErrorBody`).
//!
//! Codes are open strings on the wire: a consumer that meets a code it does
//! not know treats it as a terminal refusal of that one request.

use serde_json::{json, Value};
use subc_protocol::ErrorBody;

/// The request body is malformed. `detail.field` names the offending field,
/// for example `call_key`.
pub const INVALID_REQUEST: &str = "invalid_request";

/// A call to a tool that is disabled. `detail.tool` names it. A call to a
/// tool disabled after the session froze its catalog gets this code, saying
/// it was disabled since the session started.
pub const TOOL_DISABLED: &str = "tool_disabled";

/// The provider can no longer honour the call's pinned `schema_digest`.
/// Detail: `{tool, expected, current}`. Permanent for the session; never
/// re-dispatched.
pub const TOOL_SCHEMA_CHANGED: &str = "tool_schema_changed";

/// The provider can no longer honour the call's pinned semantics version.
/// Detail: `{tool, expected, current}`. Handled exactly like
/// [`TOOL_SCHEMA_CHANGED`].
pub const TOOL_SEMANTICS_CHANGED: &str = "tool_semantics_changed";

/// The tool cannot run. Detail: `{tool, reason}`, where `reason` is
/// `retired` (permanent for the session) or `under_review` (this call only).
/// Never re-dispatched.
pub const TOOL_UNAVAILABLE: &str = "tool_unavailable";

/// A tool a provider restart withdrew (a lost OS permission, say) was called.
/// Refuses that call only; never re-dispatched.
pub const CAPABILITY_NOT_ADMITTED: &str = "capability_not_admitted";

/// A `tool.withdraw` caller is neither the call's carrier nor the scope's
/// owner. A route error, never a withdraw answer.
pub const WITHDRAW_NOT_PERMITTED: &str = "withdraw_not_permitted";

/// An `invalid_request` error naming `field`.
pub fn invalid_request(field: &str, message: impl Into<String>) -> ErrorBody {
    ErrorBody::new(INVALID_REQUEST, message).with_detail(json!({ "field": field }))
}

/// The field an `invalid_request` error names, or `None` for any other error.
pub fn invalid_request_field(error: &ErrorBody) -> Option<&str> {
    if error.code != INVALID_REQUEST {
        return None;
    }
    detail_str(error, "field")
}

/// A `tool_disabled` refusal for `tool`.
pub fn tool_disabled(tool: &str) -> ErrorBody {
    ErrorBody::new(
        TOOL_DISABLED,
        format!("{tool} is disabled since this session started"),
    )
    .with_detail(json!({ "tool": tool }))
}

/// The tool a `tool_disabled` refusal names, or `None` for any other error.
pub fn disabled_tool(error: &ErrorBody) -> Option<&str> {
    if error.code != TOOL_DISABLED {
        return None;
    }
    detail_str(error, "tool")
}

/// A `withdraw_not_permitted` route error.
pub fn withdraw_not_permitted(message: impl Into<String>) -> ErrorBody {
    ErrorBody::new(WITHDRAW_NOT_PERMITTED, message)
}

fn detail_str<'a>(error: &'a ErrorBody, key: &str) -> Option<&'a str> {
    error
        .detail
        .as_ref()
        .and_then(|d| d.get(key))
        .and_then(Value::as_str)
}
