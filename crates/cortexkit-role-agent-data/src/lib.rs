//! Wire contract of `agent-data/v1`, the capability entorhinal calls to
//! move an agent's data from one Mac to another.
//!
//! An agent's data lives in several modules. To move an agent, entorhinal
//! asks each module that holds part of it, a participant, to stop the data
//! at a cut ([`ops::PREPARE`]), ship it through engram's session door
//! ([`ops::EXPORT`]), and bring it up on the other Mac ([`ops::IMPORT`],
//! [`ops::ACTIVATE`]); then the source gives it up ([`ops::RELEASE`]) or
//! either side undoes it ([`ops::ABORT`]). A participant claims the
//! capability by listing [`PROVIDES`] in its manifest's
//! `capabilities.provides` and answers every op in [`REQUIRED_OPS`], only
//! for the caller [`ADMITTED_CALLER`].
//!
//! The words this crate's docs use for the contract's parts (entorhinal,
//! engram, participant, move, cut, the door, driver, source and receiver,
//! this Mac, step, external data, AGENT_SYNC) are defined in the "Terms"
//! section of `CONTRACT.md`, next to this crate's `Cargo.toml`.
//!
//! `CONTRACT.md`, next to this crate's `Cargo.toml`, is the contract. The
//! types here are its wire shapes. Requests and replies are strict at the
//! top level and in every sub-object the contract types; the values a
//! participant passes through opaquely (`describe.detail`, an item's
//! `format`, an external entry's `check`) are tolerant.
//!
//! With the `test-vectors` feature, [`vectors`] exposes the contract's test
//! vectors, so a participant's own tests can check the same bytes.

#![forbid(unsafe_code)]

pub mod abort;
pub mod activate;
pub mod busy;
pub mod describe;
pub mod diverged;
pub mod errors;
pub mod export;
pub mod idempotency;
pub mod import;
pub mod prepare;
pub mod refs;
pub mod release;
pub mod request;
pub mod status;
pub mod step;
#[cfg(feature = "test-vectors")]
pub mod vectors;

use serde::{Deserialize, Serialize};

/// The capability name.
pub const ROLE: &str = "agent-data";

/// The only major version of the contract this crate defines.
pub const VERSION: &str = "v1";

/// The string a participant lists in its manifest's
/// `capabilities.provides`.
pub const PROVIDES: &str = "agent-data/v1";

/// The `service_id` a participant serves these ops on. Entorhinal opens a
/// route with target `InternalService { module_id: <participant>, service_id:
/// "agent-data" }`, and the participant declares a matching
/// `internal_service` role in its manifest's `provides`, listing the ten op
/// names (`REQUIRED_OPS`) and `agent_facing: false`. The daemon refuses a
/// route to a service the module didn't declare.
pub const SERVICE_ID: &str = "agent-data";

/// The only caller a participant serves, as the daemon stamps it on the
/// route bind. Any other caller, `direct` included, is refused
/// `caller_forbidden`.
pub const ADMITTED_CALLER: &str = "reserved:entorhinal";

pub use refs::MAX_REFS;

/// Names of the ops, as the method of the request that carries them.
pub mod ops {
    /// Reports what this participant holds. A read: it changes no state.
    pub const DESCRIBE: &str = "agent_data.describe";
    /// Reports whether each ref's data can be cut now. A read: it changes
    /// no state.
    pub const BUSY: &str = "agent_data.busy";
    /// Reports the step a `cutId` has reached on this participant. A read:
    /// it changes no state.
    pub const STATUS: &str = "agent_data.status";
    /// On the source, decides where the cut ends and freezes the data
    /// where it can. Moves the step from `none` to `prepared`.
    pub const PREPARE: &str = "agent_data.prepare";
    /// On the source, admits the cut's bytes through engram's session door
    /// (`session.admit`). Moves the step from `prepared` to `exported`.
    pub const EXPORT: &str = "agent_data.export";
    /// On the receiver, compares this Mac's copy with the cut. A read: it
    /// changes no state.
    pub const DIVERGED: &str = "agent_data.diverged";
    /// On the receiver, reads the cut into staging and checks its digests.
    /// Moves the step from `none` to `imported`.
    pub const IMPORT: &str = "agent_data.import";
    /// On the receiver, makes the staged data live. Moves the step from
    /// `imported` to `activated`.
    pub const ACTIVATE: &str = "agent_data.activate";
    /// On either side, undoes the move on this participant. Moves any
    /// earlier step to `aborted`.
    pub const ABORT: &str = "agent_data.abort";
    /// On the source, turns the freeze into a tombstone naming the new
    /// holder. Moves the step from `exported` to `released`.
    pub const RELEASE: &str = "agent_data.release";
}

/// Every op a participant answers. All ten are required: a participant can
/// be the source of one move and the receiver of the next.
pub const REQUIRED_OPS: &[&str] = &[
    ops::DESCRIBE,
    ops::BUSY,
    ops::STATUS,
    ops::PREPARE,
    ops::EXPORT,
    ops::DIVERGED,
    ops::IMPORT,
    ops::ACTIVATE,
    ops::ABORT,
    ops::RELEASE,
];

/// One of the ten ops.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Op {
    Describe,
    Busy,
    Status,
    Prepare,
    Export,
    Diverged,
    Import,
    Activate,
    Abort,
    Release,
}

impl Op {
    pub const ALL: [Op; 10] = [
        Op::Describe,
        Op::Busy,
        Op::Status,
        Op::Prepare,
        Op::Export,
        Op::Diverged,
        Op::Import,
        Op::Activate,
        Op::Abort,
        Op::Release,
    ];

    /// The op's method name.
    pub fn method(self) -> &'static str {
        match self {
            Self::Describe => ops::DESCRIBE,
            Self::Busy => ops::BUSY,
            Self::Status => ops::STATUS,
            Self::Prepare => ops::PREPARE,
            Self::Export => ops::EXPORT,
            Self::Diverged => ops::DIVERGED,
            Self::Import => ops::IMPORT,
            Self::Activate => ops::ACTIVATE,
            Self::Abort => ops::ABORT,
            Self::Release => ops::RELEASE,
        }
    }

    /// The op with this method name, if it is one of the ten.
    pub fn from_method(method: &str) -> Option<Self> {
        Self::ALL.into_iter().find(|op| op.method() == method)
    }

    /// Whether the op changes state. These six are keyed by `(cutId, op)`
    /// for idempotency; the four reads (`describe`, `busy`, `status`,
    /// `diverged`) are not.
    pub fn changes_state(self) -> bool {
        matches!(
            self,
            Self::Prepare
                | Self::Export
                | Self::Import
                | Self::Activate
                | Self::Abort
                | Self::Release
        )
    }

    /// Whether the request carries a `cutId`: every op except `describe`
    /// and `busy`.
    pub fn carries_cut_id(self) -> bool {
        !matches!(self, Self::Describe | Self::Busy)
    }
}

/// The reply `{}` of `activate` and `abort`. Strict: any member
/// is refused.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
#[non_exhaustive]
pub struct EmptyReply {}

impl EmptyReply {
    pub fn new() -> Self {
        Self {}
    }
}

/// Whether a route caller, as the daemon stamped it, may call any op of
/// this contract. Decided on the stamped principal only, never on anything
/// in the request body.
pub fn caller_admitted(stamped_principal: &str) -> bool {
    stamped_principal == ADMITTED_CALLER
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn required_ops_are_every_op_in_order() {
        let methods: Vec<&str> = Op::ALL.iter().map(|op| op.method()).collect();
        assert_eq!(methods, REQUIRED_OPS);
        for op in Op::ALL {
            assert_eq!(Op::from_method(op.method()), Some(op));
        }
        assert_eq!(Op::from_method("agent_data.resume"), None);
    }

    #[test]
    fn reads_and_writes() {
        let writes: Vec<Op> = Op::ALL
            .into_iter()
            .filter(|op| op.changes_state())
            .collect();
        assert_eq!(
            writes,
            [
                Op::Prepare,
                Op::Export,
                Op::Import,
                Op::Activate,
                Op::Abort,
                Op::Release
            ]
        );
    }

    #[test]
    fn only_entorhinal_is_admitted() {
        assert!(caller_admitted("reserved:entorhinal"));
        for other in [
            "direct",
            "reserved:broca",
            "session:ses_1",
            "",
            "entorhinal",
        ] {
            assert!(!caller_admitted(other), "{other}");
        }
    }
}
