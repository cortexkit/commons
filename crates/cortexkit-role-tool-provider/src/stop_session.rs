//! Session-bound stop requests and bounded display replies.

use crate::{call::ToolCallRequest, errors, ops, result::is_snake_case};
use serde::{Deserialize, Serialize};
use subc_protocol::ErrorBody;

pub const MAX_ITEMS: usize = 64;
pub const MAX_LABEL_CHARS: usize = 120;

/// Body addressing is forbidden: the session and its runner come from the bind.
#[derive(Clone, Debug, PartialEq, Eq, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
#[non_exhaustive]
pub struct StopSessionArguments {
    pub request_id: String,
}

/// The provider's work key, copied only from the requesting route's bind.
#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord)]
#[non_exhaustive]
pub struct SessionKey {
    pub principal: String,
    pub harness: String,
    pub session: String,
}
impl SessionKey {
    pub fn new(
        principal: impl Into<String>,
        harness: impl Into<String>,
        session: impl Into<String>,
    ) -> Self {
        Self {
            principal: principal.into(),
            harness: harness.into(),
            session: session.into(),
        }
    }
}

impl StopSessionArguments {
    pub fn new(request_id: impl Into<String>) -> Self {
        Self {
            request_id: request_id.into(),
        }
    }
    pub fn into_request(self) -> ToolCallRequest {
        ToolCallRequest::new(
            ops::TOOL_STOP_SESSION,
            serde_json::to_value(self).expect("stop arguments serialize"),
        )
    }
}

/// Shared request-id grammar, also used by the runner's stop-all operation.
pub fn valid_request_id(id: &str) -> bool {
    (1..=128).contains(&id.len()) && id.bytes().all(|byte| (0x20..=0x7e).contains(&byte))
}

/// Check the request against the requesting route's attested work key. Use
/// that exact key for lookup; another principal's identical pair is unrelated.
/// Malformed requests are terminal errors naming their field.
pub fn check_request(
    request: &ToolCallRequest,
    bound_session: Option<&SessionKey>,
) -> Result<StopSessionArguments, ErrorBody> {
    if bound_session.is_none() {
        return Err(errors::invalid_request(
            "route",
            "a session-bound route is required",
        ));
    }
    if request.call_key.is_some() {
        return Err(errors::invalid_request(
            "call_key",
            "stop_session is not a held call",
        ));
    }
    let object = request
        .arguments
        .as_object()
        .ok_or_else(|| errors::invalid_request("arguments", "must be an object"))?;
    for key in object.keys() {
        if key != "request_id" {
            return Err(errors::invalid_request(
                &format!("arguments.{key}"),
                "unknown member",
            ));
        }
    }
    let args: StopSessionArguments = serde_json::from_value(request.arguments.clone())
        .map_err(|e| errors::invalid_request("arguments.request_id", e.to_string()))?;
    if !valid_request_id(&args.request_id) {
        return Err(errors::invalid_request(
            "arguments.request_id",
            "must be 1–128 printable ASCII characters",
        ));
    }
    Ok(args)
}

/// Open outcome vocabulary; consumers preserve outcomes they do not know.
pub mod outcomes {
    pub const STOPPED: &str = "stopped";
    pub const PAUSED: &str = "paused";
    pub const ALREADY_FINISHED: &str = "already_finished";
    pub const COULD_NOT_STOP: &str = "could_not_stop";
}

#[derive(Clone, Debug, PartialEq, Eq, Deserialize, Serialize)]
#[non_exhaustive]
pub struct StopItem {
    pub kind: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub id: Option<String>,
    /// Short safe display text, never a command line or a secret.
    pub label: String,
    pub outcome: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub reason: Option<String>,
}
impl StopItem {
    pub fn new(
        kind: impl Into<String>,
        label: impl Into<String>,
        outcome: impl Into<String>,
    ) -> Self {
        Self {
            kind: kind.into(),
            id: None,
            label: label.into(),
            outcome: outcome.into(),
            reason: None,
        }
    }
    pub fn with_id(mut self, id: impl Into<String>) -> Self {
        self.id = Some(id.into());
        self
    }
    pub fn with_reason(mut self, reason: impl Into<String>) -> Self {
        self.reason = Some(reason.into());
        self
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Deserialize, Serialize)]
#[non_exhaustive]
pub struct StopSessionReply {
    pub items: Vec<StopItem>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub more: Option<u64>,
}
impl StopSessionReply {
    pub fn new(items: Vec<StopItem>) -> Self {
        Self { items, more: None }
    }
    pub fn with_more(mut self, more: u64) -> Self {
        self.more = Some(more);
        self
    }
    /// Check bounds and the open kind grammar. Display safety still belongs
    /// to the provider: a length check cannot recognize a secret or command.
    pub fn validate(&self) -> Result<(), String> {
        if self.items.len() > MAX_ITEMS {
            return Err("items: at most 64 items; put the remaining count in more".into());
        }
        for (index, item) in self.items.iter().enumerate() {
            if !is_snake_case(&item.kind) {
                return Err(format!("items[{index}].kind: expected snake_case"));
            }
            if item.label.chars().count() > MAX_LABEL_CHARS {
                return Err(format!("items[{index}].label: exceeds 120 characters"));
            }
            if item.outcome == outcomes::COULD_NOT_STOP
                && item.reason.as_ref().is_none_or(String::is_empty)
            {
                return Err(format!(
                    "items[{index}].reason: required for could_not_stop"
                ));
            }
        }
        Ok(())
    }
}
