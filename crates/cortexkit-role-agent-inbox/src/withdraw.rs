//! Withdrawal uses the authenticated route's namespace, never a sender name
//! supplied in params. Only a pending item can be withdrawn; once a scheduler
//! has claimed it for delivery, withdraw answers `already_delivered`.

use serde::{Deserialize, Serialize};

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct IdempotencyKey {
    pub idempotency_key: String,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ReplaceKey {
    pub replace_key: String,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(untagged)]
pub enum WithdrawRequest {
    Idempotency(IdempotencyKey),
    Replace(ReplaceKey),
}

impl WithdrawRequest {
    pub fn idempotency(key: impl Into<String>) -> Self {
        Self::Idempotency(IdempotencyKey {
            idempotency_key: key.into(),
        })
    }
    pub fn replace(key: impl Into<String>) -> Self {
        Self::Replace(ReplaceKey {
            replace_key: key.into(),
        })
    }
}

open_string!(WithdrawState {
    Withdrawn => "withdrawn", AlreadyDelivered => "already_delivered", NotFound => "not_found",
});

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[non_exhaustive]
pub struct WithdrawReply {
    pub state: WithdrawState,
}

impl WithdrawReply {
    pub fn new(state: WithdrawState) -> Self {
        Self { state }
    }
}
