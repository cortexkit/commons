//! `agent_data.diverged {refs, cutId}`: on the receiver, before import,
//! compare this Mac's copy with the cut.
//!
//! `at_cut` means this Mac has nothing for these refs that the cut doesn't
//! contain: for an append-only stream, this Mac's bytes are a prefix of the
//! cut's; for a row store, its newest rows match the last cut it exported
//! or imported. A ref this participant doesn't hold yet answers `at_cut`:
//! that is a first-time import. `advanced` (this Mac has data the cut lacks) anywhere
//! stops the move, and entorhinal asks the operator which copy to keep;
//! `unknown` blocks too and is never read as `at_cut`. May be refused
//! `external_not_present`, or `cut_not_ready` while a participant that
//! reads the incoming cut can't read it yet (never `unknown` for that), or
//! `incompatible_version` when it can't parse the cut's items because their
//! `format` is one this Mac can't read (never `unknown` for that either). A read: it changes no state.
//!
//! The contract terms used here (source, receiver, cut, the door, driver
//! and the rest) are defined in the "Terms" section of `CONTRACT.md`.

use serde::{de::Error as _, Deserialize, Deserializer, Serialize};

/// Maximum UTF-8 byte length of a diverged-entry reason.
pub const REASON_MAX_BYTES: usize = 256;

/// The `diverged` request: `{refs, cutId}`.
pub type DivergedRequest = crate::request::CutRequest;

/// The `diverged` reply: `{refs: [{state, reason?}]}`, one entry per request
/// ref, in request order.
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
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
#[serde(deny_unknown_fields)]
#[non_exhaustive]
pub struct DivergedEntry {
    pub state: DivergedState,
    /// Optional human-readable explanation shown in move status. It is
    /// allowed only for `unknown` or `advanced` and must be short plain text.
    #[serde(skip_serializing_if = "Option::is_none", default)]
    pub reason: Option<String>,
}

impl DivergedEntry {
    pub fn new(state: DivergedState) -> Self {
        Self {
            state,
            reason: None,
        }
    }

    /// Build an entry with a human-readable explanation.
    ///
    /// Returns an error naming the rule the reason violates: reasons are not
    /// allowed with `at_cut`, must be non-empty, must not exceed
    /// [`REASON_MAX_BYTES`] UTF-8 bytes, and must not contain control
    /// characters.
    pub fn with_reason(
        state: DivergedState,
        reason: impl Into<String>,
    ) -> Result<Self, &'static str> {
        let reason = reason.into();
        validate_reason(state, &reason)?;
        Ok(Self {
            state,
            reason: Some(reason),
        })
    }
}

impl<'de> Deserialize<'de> for DivergedEntry {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        #[derive(Deserialize)]
        #[serde(deny_unknown_fields)]
        struct RawDivergedEntry {
            state: DivergedState,
            #[serde(default)]
            reason: Option<String>,
        }

        let raw = RawDivergedEntry::deserialize(deserializer)?;
        match raw.reason {
            Some(reason) => Self::with_reason(raw.state, reason).map_err(D::Error::custom),
            None => Ok(Self::new(raw.state)),
        }
    }
}

fn validate_reason(state: DivergedState, reason: &str) -> Result<(), &'static str> {
    if state == DivergedState::AtCut {
        return Err("reason is not allowed with state `at_cut`");
    }
    if reason.is_empty() {
        return Err("reason must be non-empty");
    }
    if reason.len() > REASON_MAX_BYTES {
        return Err("reason must be at most 256 UTF-8 bytes");
    }
    if reason.chars().any(char::is_control) {
        return Err("reason must not contain control characters");
    }
    Ok(())
}

/// `at_cut`, `advanced` or `unknown`.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Deserialize, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum DivergedState {
    /// This Mac has nothing for these refs that the cut doesn't contain:
    /// an append-only stream's bytes are a prefix of the cut's, a row
    /// store's newest rows match the last cut it exported or imported, or
    /// this Mac holds nothing for the ref.
    AtCut,
    /// This Mac has data for these refs that the cut doesn't contain,
    /// including host data past an external `check`.
    Advanced,
    /// Blocks the move; never read as `at_cut`.
    Unknown,
}
