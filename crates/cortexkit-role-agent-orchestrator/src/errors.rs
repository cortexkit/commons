//! Refusal bodies, distinct from transport failures and successful replies.

use serde::{Deserialize, Serialize};
use serde_json::Value;

open_string!(Code {
    CallerNotModule => "caller_not_module",
    NotOrchestratorSession => "not_orchestrator_session",
    SessionEnded => "session_ended",
    TooManyOpen => "too_many_open",
    KeyReused => "key_reused",
    SyncRequired => "sync_required",
    UnknownKey => "unknown_key",
    InvalidParams => "invalid_params",
});

/// All v1 refusal codes.
pub const CODES: &[&str] = &[
    "caller_not_module",
    "not_orchestrator_session",
    "session_ended",
    "too_many_open",
    "key_reused",
    "sync_required",
    "unknown_key",
    "invalid_params",
];

/// Every refusal is terminal, including unknown codes: the caller falls back
/// rather than retrying the unchanged request.
pub fn is_terminal(_code: &str) -> bool {
    true
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
