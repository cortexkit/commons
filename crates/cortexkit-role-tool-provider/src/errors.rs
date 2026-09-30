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

/// A call to a tool the provider does not serve at all.
pub const UNKNOWN_TOOL: &str = "unknown_tool";

/// A `tool.withdraw` caller may not withdraw the call it addressed: the call
/// is under a different scope than the route's stamp. A route error, never a
/// withdraw answer; terminal for the caller.
pub const WITHDRAW_NOT_PERMITTED: &str = "withdraw_not_permitted";

/// The scope's owner withdrew without `arguments.carrier`, and it holds no
/// call of its own under that key. A route error; terminal.
pub const WITHDRAW_CARRIER_REQUIRED: &str = "withdraw_carrier_required";

/// A caller other than the scope's owner named a carrier other than itself.
/// A route error; terminal.
pub const WITHDRAW_CARRIER_MISMATCH: &str = "withdraw_carrier_mismatch";

/// `arguments.scope` differs from the route's stamped scope. A route error;
/// terminal.
pub const WITHDRAW_SCOPE_MISMATCH: &str = "withdraw_scope_mismatch";

/// subc's route refusal while the target module reloads. Transient.
pub const MODULE_RELOADING: &str = subc_protocol::error_codes::MODULE_RELOADING;

/// subc's route refusal while the scope's owner has not yet synced the scope
/// after a daemon restart. Transient.
pub const SCOPE_NOT_SYNCED: &str = "scope_not_synced";

/// Whether a route error on a role request is transient, so the caller
/// retries the same request: `scope_not_synced`, or any code subc itself
/// retries (`module_reloading`, `module_warming`, `target_unavailable`,
/// `module_timeout`). Every other code is terminal for that request.
pub fn is_transient(code: &str) -> bool {
    code == SCOPE_NOT_SYNCED || subc_protocol::error_codes::is_retryable_route_open(code)
}

/// An `unknown_tool` refusal for `tool`.
pub fn unknown_tool(tool: &str) -> ErrorBody {
    ErrorBody::new(UNKNOWN_TOOL, format!("no tool named {tool}"))
        .with_detail(json!({ "tool": tool }))
}

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

fn detail_str<'a>(error: &'a ErrorBody, key: &str) -> Option<&'a str> {
    error
        .detail
        .as_ref()
        .and_then(|d| d.get(key))
        .and_then(Value::as_str)
}
