//! `agent_data.export {refs, cutId}`: on the source, make sure every
//! item's bytes up to `endOffset` are admitted through the door
//! (`session.admit` on engram's `session-door` service). Entorhinal then
//! calls `cut.seal`. The refs must be the set `prepare` named for this
//! `cutId`, in any order; another set is refused `invalid_state`
//! ([`crate::step::check_ref_set`]). Step: `prepared` → `exported`.

use serde::{Deserialize, Serialize};

/// The `export` request: `{refs, cutId}`.
pub type ExportRequest = crate::request::CutRequest;

/// The `export` reply: `{items: [{sessionRef, endOffset}]}`, sent once all
/// are admitted.
#[derive(Clone, Debug, PartialEq, Eq, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
#[non_exhaustive]
pub struct ExportReply {
    pub items: Vec<ExportedItem>,
}

impl ExportReply {
    pub fn new(items: Vec<ExportedItem>) -> Self {
        Self { items }
    }
}

/// One admitted stream.
#[derive(Clone, Debug, PartialEq, Eq, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
#[non_exhaustive]
pub struct ExportedItem {
    #[serde(rename = "sessionRef")]
    pub session_ref: String,
    #[serde(rename = "endOffset")]
    pub end_offset: u64,
}

impl ExportedItem {
    pub fn new(session_ref: impl Into<String>, end_offset: u64) -> Self {
        Self {
            session_ref: session_ref.into(),
            end_offset,
        }
    }
}
