//! `agent_data.diverged {refs, cutId}`: on the receiver, before import,
//! compare this Mac's copy with the cut.
//!
//! A ref this participant doesn't hold yet answers `at_cut`: that is a
//! first-time import. `advanced` (this Mac has data the cut lacks) anywhere
//! stops the move, and entorhinal asks the operator which copy to keep;
//! `unknown` blocks too and is never read as `at_cut`. May be refused
//! `external_not_present`. A read: it changes no state.

use serde::{Deserialize, Serialize};

/// The `diverged` request: `{refs, cutId}`.
pub type DivergedRequest = crate::request::CutRequest;

/// The `diverged` reply: `{refs: [{state}]}`, one entry per request ref, in
/// request order.
#[derive(Clone, Debug, PartialEq, Eq, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
#[non_exhaustive]
pub struct DivergedReply {
    pub refs: Vec<DivergedEntry>,
}

impl DivergedReply {
    pub fn new(refs: Vec<DivergedEntry>) -> Self {
        Self { refs }
    }

    /// Whether this reply has one entry per ref of `request`. Entries carry
    /// no ref of their own, so a reply of any other length cannot be read.
    pub fn answers(&self, request: &DivergedRequest) -> bool {
        self.refs.len() == request.refs.len()
    }

    /// Whether the move may go on to `import`: every ref is `at_cut`.
    pub fn all_at_cut(&self) -> bool {
        self.refs
            .iter()
            .all(|entry| entry.state == DivergedState::AtCut)
    }
}

/// One ref's answer.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
#[non_exhaustive]
pub struct DivergedEntry {
    pub state: DivergedState,
}

impl DivergedEntry {
    pub fn new(state: DivergedState) -> Self {
        Self { state }
    }
}

/// `at_cut`, `advanced` or `unknown`.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Deserialize, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum DivergedState {
    /// This Mac's copy is a prefix of the cut, or absent.
    AtCut,
    /// This Mac has data past where the cut starts that the cut doesn't
    /// contain, including host data past an external `check`.
    Advanced,
    /// Blocks the move; never read as `at_cut`.
    Unknown,
}
