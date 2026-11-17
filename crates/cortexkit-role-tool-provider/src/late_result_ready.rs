//! Bus wake-up hints for held results fetched and acknowledged through `late_results`.

use serde::{Deserialize, Serialize};

use crate::call::validate_call_key;

pub const LATE_RESULT_READY_EVENT: &str = "late_result_ready";
pub const LATE_RESULT_READY_VERSION: u64 = 1;

/// Inline hint naming a durably settled held call, without its result body.
///
/// Serialize with `serde_json::to_vec` for the compact producer payload. Field
/// order is part of the wire contract. Readers ignore unknown members and keep
/// outcomes open; validate decoded or constructed values before using them.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[non_exhaustive]
pub struct LateResultReady {
    pub call_key: String,
    pub outcome: String,
}

impl LateResultReady {
    pub fn new(call_key: impl Into<String>, outcome: impl Into<String>) -> Self {
        Self {
            call_key: call_key.into(),
            outcome: outcome.into(),
        }
    }

    /// Check the existing call-key bounds and require a non-empty, open outcome.
    pub fn validate(&self) -> Result<(), serde_json::Error> {
        validate_call_key(&self.call_key)
            .map_err(<serde_json::Error as serde::de::Error>::custom)?;
        if self.outcome.is_empty() {
            return Err(<serde_json::Error as serde::de::Error>::custom(
                "outcome: must be non-empty",
            ));
        }
        Ok(())
    }
}

/// Build `Nats-Msg-Id` using the settlement's stable event id and its reader.
/// Reuse both inputs when retrying or replaying the same settlement.
pub fn late_result_ready_msg_id(event_id: &str, reader: &str) -> String {
    format!("{event_id}:{reader}")
}

/// Derive the subject's reader suffix only from a reserved module custodian.
/// Invalid principals get no notice; they must not broaden a bus subject.
pub fn late_result_ready_reader(custodian: &str) -> Option<&str> {
    let reader = custodian.strip_prefix("reserved:")?;
    cortexkit_bus_naming::validate_token(cortexkit_bus_naming::TokenKind::ModuleId, reader).ok()?;
    Some(reader)
}
