//! `agent_data.import {refs, cutId}`: on the receiver, read the cut
//! through the door (`cut.read`, then `session.read` with the `cutId`,
//! which needs no lease) into staging, and check every digest.
//!
//! Compatibility is decided from the cut's authenticated bytes, reading only
//! as far as needed (typically the first frame); on `incompatible_version`
//! the private staging is discarded and nothing is installed. A cut not yet
//! readable through the door is
//! `cut_not_ready`, which the driver retries within the step. The
//! receiving Mac isn't frozen, so a run may start on the same session
//! between `diverged` and `import`: that is `busy`, retried later. The
//! participant re-checks divergence atomically with staging; if this Mac's
//! copy gained data after `diverged` answered `at_cut`, it answers
//! `local_advanced` instead of overwriting, leaving its local copy
//! untouched. That is terminal for the step; the driver then aborts the
//! move and offers the operator the fork choice. Refs this
//! participant doesn't hold yet are accepted: that is a first-time import.
//! A released (tombstoned) copy of the same refs here, as on a move back,
//! is replaced. Step: `none` → `imported`.
//!
//! The contract terms used here (source, receiver, cut, the door, driver
//! and the rest) are defined in the "Terms" section of `CONTRACT.md`.

use serde::{Deserialize, Serialize};

/// The `import` request: `{refs, cutId}`.
pub type ImportRequest = crate::request::CutRequest;

/// The `import` reply: `{items: [{sessionRef, endOffset, digest}]}`.
#[derive(Clone, Debug, PartialEq, Eq, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
#[non_exhaustive]
pub struct ImportReply {
    pub items: Vec<ImportedItem>,
}

impl ImportReply {
    pub fn new(items: Vec<ImportedItem>) -> Self {
        Self { items }
    }
}

/// One staged stream, with the digest the participant checked it against.
#[derive(Clone, Debug, PartialEq, Eq, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
#[non_exhaustive]
pub struct ImportedItem {
    #[serde(rename = "sessionRef")]
    pub session_ref: String,
    #[serde(rename = "endOffset")]
    pub end_offset: u64,
    pub digest: String,
}

impl ImportedItem {
    pub fn new(session_ref: impl Into<String>, end_offset: u64, digest: impl Into<String>) -> Self {
        Self {
            session_ref: session_ref.into(),
            end_offset,
            digest: digest.into(),
        }
    }
}
