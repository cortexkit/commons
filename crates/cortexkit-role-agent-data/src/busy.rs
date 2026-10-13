//! `agent_data.busy {refs}`: whether each ref's data can be cut now.
//!
//! "Busy" covers anything that would change the data during a move: a turn
//! or run in flight, a queued send, an open approval card, a background
//! job. A read: it changes nothing and needs no `cutId`.

use serde::{Deserialize, Serialize};

use crate::refs::Refs;

/// Example `reason_code` values from the contract. The set is open: a
/// participant may answer with another short code, and a reader that does
/// not know one still shows `reason`.
pub mod reason_codes {
    pub const TURN_IN_FLIGHT: &str = "turn_in_flight";
    pub const RUN_ACTIVE: &str = "run_active";
    pub const SEND_QUEUED: &str = "send_queued";
    pub const APPROVAL_OPEN: &str = "approval_open";
    pub const HISTORIAN_RUNNING: &str = "historian_running";
    pub const DREAMER_RUNNING: &str = "dreamer_running";
}

/// The `busy` request: `{refs}`.
#[derive(Clone, Debug, PartialEq, Eq, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
#[non_exhaustive]
pub struct BusyRequest {
    pub refs: Refs,
}

impl BusyRequest {
    pub fn new(refs: Refs) -> Self {
        Self { refs }
    }
}

/// The `busy` reply: `{refs: [{state, reason_code?, reason?}]}`, one entry
/// per request ref, in request order.
#[derive(Clone, Debug, PartialEq, Eq, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
#[non_exhaustive]
pub struct BusyReply {
    pub refs: Vec<BusyEntry>,
}

impl BusyReply {
    pub fn new(refs: Vec<BusyEntry>) -> Self {
        Self { refs }
    }

    /// Whether this reply has one entry per ref of `request`. Entries carry
    /// no ref of their own, so a reply of any other length cannot be read.
    pub fn answers(&self, request: &BusyRequest) -> bool {
        self.refs.len() == request.refs.len()
    }

    /// Whether any ref blocks a move: `busy` or `unknown`.
    pub fn any_busy(&self) -> bool {
        self.refs.iter().any(|entry| entry.state.counts_as_busy())
    }
}

/// The `busy` answer for one request ref: its state, and optionally a
/// reason code and operator text.
#[derive(Clone, Debug, PartialEq, Eq, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
#[non_exhaustive]
pub struct BusyEntry {
    pub state: BusyState,
    /// A short code the phone can show without parsing text
    /// ([`reason_codes`]).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub reason_code: Option<String>,
    /// Short text the participant writes for the operator to read.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub reason: Option<String>,
}

impl BusyEntry {
    pub fn new(state: BusyState) -> Self {
        Self {
            state,
            reason_code: None,
            reason: None,
        }
    }

    pub fn with_reason_code(mut self, reason_code: impl Into<String>) -> Self {
        self.reason_code = Some(reason_code.into());
        self
    }

    pub fn with_reason(mut self, reason: impl Into<String>) -> Self {
        self.reason = Some(reason.into());
        self
    }
}

/// `idle`, `busy` or `unknown`.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Deserialize, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum BusyState {
    Idle,
    Busy,
    /// The participant could not tell whether the data is idle. Counts as
    /// busy: a move that cuts data still being written would lose those
    /// writes or fork the agent, so only a confirmed `idle` lets it go on.
    Unknown,
}

impl BusyState {
    /// Whether this state blocks moving the agent. `unknown` counts as
    /// busy, because a move cut while data may still be changing would lose
    /// or fork the changes.
    pub fn counts_as_busy(self) -> bool {
        self != Self::Idle
    }
}
