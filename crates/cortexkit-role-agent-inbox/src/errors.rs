//! Refusal bodies, separate from successful operation replies. Capability
//! absence (`no_implementer`) is discovery failure, never a refusal body.

use serde::{Deserialize, Serialize};
use serde_json::Value;

open_string!(Code {
    ScopeNotYours => "scope_not_yours",
    ScopeUnknown => "scope_unknown",
    ScopeEnded => "scope_ended",
    IdempotencyConflict => "idempotency_conflict",
    ContentTooLarge => "content_too_large",
    InboxFull => "inbox_full",
    TemplateUnknown => "template_unknown",
    TemplateNotAllowed => "template_not_allowed",
    SinkTargetNotOwner => "sink_target_not_owner",
    SinkTargetNotInAudience => "sink_target_not_in_audience",
    InvalidParams => "invalid_params",
});

/// All v1 refusal codes. Only `inbox_full` is retryable.
pub const CODES: &[&str] = &[
    "scope_not_yours",
    "scope_unknown",
    "scope_ended",
    "idempotency_conflict",
    "content_too_large",
    "inbox_full",
    "template_unknown",
    "template_not_allowed",
    "sink_target_not_owner",
    "sink_target_not_in_audience",
    "invalid_params",
];

/// Unknown refusals are terminal: an unchanged retry has no known remedy.
pub fn is_terminal(code: &str) -> bool {
    code != "inbox_full"
}

pub fn is_retryable(code: &str) -> bool {
    !is_terminal(code)
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[non_exhaustive]
pub struct Refusal {
    pub code: Code,
    pub message: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub detail: Option<Value>,
}

impl Refusal {
    pub fn new(code: Code, message: impl Into<String>) -> Self {
        Self {
            code,
            message: message.into(),
            detail: None,
        }
    }

    pub fn with_detail(mut self, detail: Value) -> Self {
        self.detail = Some(detail);
        self
    }

    pub fn is_terminal(&self) -> bool {
        is_terminal(self.code.as_str())
    }
}
