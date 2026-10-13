//! `agent_data.import {refs, cutId}`: on the receiver, read the cut
//! through the door (`cut.read`, then `session.read` with the `cutId`,
//! which needs no lease) into staging, and check every digest.
//!
//! Each item's `format` is checked before anything is read
//! (`incompatible_version`). A cut not yet readable through the door is
//! `cut_not_ready`, which the driver retries within the step. Refs this
//! participant doesn't hold yet are accepted: that is a first-time import.
//! A released (tombstoned) copy of the same refs here, as on a move back,
//! is replaced. Step: `none` → `imported`.

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
