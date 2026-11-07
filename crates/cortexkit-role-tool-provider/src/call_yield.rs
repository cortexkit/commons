//! Carrier-only hand-back requests. Yielding never cancels the call's work.

use crate::{
    call::ToolCallRequest, errors, ops, result::is_snake_case, scope::ScopeIdentity, withdraw,
};
use serde::{Deserialize, Serialize};
use subc_protocol::ErrorBody;

/// The documented reason for asking a waiting call to hand back.
pub const PERSON_MESSAGE: &str = "person_message";

/// Arguments of `tool.yield`; the route attests the carrier, never the body.
#[derive(Clone, Debug, PartialEq, Eq, Deserialize, Serialize)]
#[non_exhaustive]
pub struct YieldArguments {
    pub call_key: String,
    /// Open, nonempty snake_case vocabulary.
    pub reason: String,
}

impl YieldArguments {
    pub fn new(call_key: impl Into<String>, reason: impl Into<String>) -> Self {
        Self {
            call_key: call_key.into(),
            reason: reason.into(),
        }
    }

    pub fn into_request(self) -> ToolCallRequest {
        ToolCallRequest::new(
            ops::TOOL_YIELD,
            serde_json::to_value(self).expect("yield arguments serialize"),
        )
    }
}

/// Validate the envelope and target exactly as for withdraw, then the open reason.
pub fn parse_yield_request(request: &ToolCallRequest) -> Result<YieldArguments, ErrorBody> {
    withdraw::parse_withdraw_request(request)?;
    let arguments: YieldArguments = serde_json::from_value(request.arguments.clone())
        .map_err(|error| errors::invalid_request("arguments", error.to_string()))?;
    if !is_snake_case(&arguments.reason) {
        return Err(errors::invalid_request(
            "arguments.reason",
            "must be a nonempty snake_case code",
        ));
    }
    Ok(arguments)
}

/// Whether a found record belongs to the attested carrier on this route.
/// Look up only `(caller, call_key)` first. A false check answers `unknown_call`.
pub fn record_is_open_on_route(
    caller: &str,
    record_carrier: &str,
    record_scope: Option<&ScopeIdentity>,
    route_scope: Option<&ScopeIdentity>,
) -> bool {
    caller == record_carrier && withdraw::check_record_scope(record_scope, route_scope).is_ok()
}

/// Values answering a yield request, never terminal route errors.
#[derive(Clone, Debug, PartialEq, Eq, Deserialize, Serialize)]
#[serde(tag = "answer", rename_all = "snake_case")]
#[non_exhaustive]
pub enum YieldAnswer {
    /// The call's final frame will arrive within five seconds.
    Accepted,
    NotYieldable {
        #[serde(default, skip_serializing_if = "Option::is_none")]
        reason: Option<String>,
    },
    /// Includes repeats, completed calls, and calls outside this carrier/route.
    UnknownCall,
}
