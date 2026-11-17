//! Inline bus wake-up hints for changes to keyed sends.

use serde::{Deserialize, Serialize};

pub const SEND_STATE_EVENT: &str = "send_state";
pub const SEND_STATE_VERSION: u64 = 1;

/// A hint to re-read `session.send_status` through the runner owning the send.
///
/// Serialize with `serde_json::to_vec` for the compact JCS body: fields are in
/// sorted key order. Readers ignore unknown members and keep `state` open.
/// Validate decoded or constructed values before using them.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[non_exhaustive]
pub struct SendStateNotice {
    pub send_id: String,
    pub seq: u64,
    pub session_ref: String,
    pub state: String,
    pub v: u64,
}

impl SendStateNotice {
    pub fn new(
        send_id: impl Into<String>,
        seq: u64,
        session_ref: impl Into<String>,
        state: impl Into<String>,
    ) -> Self {
        Self {
            send_id: send_id.into(),
            seq,
            session_ref: session_ref.into(),
            state: state.into(),
            v: SEND_STATE_VERSION,
        }
    }

    /// Check identifier bounds, version, open state and the serialized byte cap.
    pub fn validate(&self) -> Result<(), serde_json::Error> {
        let invalid = <serde_json::Error as serde::de::Error>::custom;
        if self.send_id.is_empty() {
            return Err(invalid("send_id: must be non-empty"));
        }
        if self.session_ref.len() != 32
            || !self
                .session_ref
                .bytes()
                .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
        {
            return Err(invalid("session_ref: must be 32 lowercase hex characters"));
        }
        if self.v != SEND_STATE_VERSION {
            return Err(invalid("v: must be 1"));
        }
        if self.seq > (1_u64 << 53) - 1 {
            return Err(invalid("seq: must be no larger than 2^53 - 1"));
        }
        if self.state.is_empty() {
            return Err(invalid("state: must be non-empty"));
        }
        if serde_json::to_vec(self)?.len() > 512 {
            return Err(invalid("body: must be at most 512 bytes"));
        }
        Ok(())
    }
}

/// Build `Nats-Msg-Id`; reuse the same sequence when republishing a notice.
pub fn send_state_msg_id(session_ref: &str, send_id: &str, seq: u64) -> String {
    format!("{session_ref}:{send_id}:{seq}")
}
