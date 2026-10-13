//! `agent_data.release {refs, cutId, holder}`: on the source, once
//! entorhinal's move record reads `activated` (the receiver has made the
//! data live). The write freeze that `prepare` set becomes a durable
//! tombstone naming the new holder. From then on the participant refuses
//! its own local callers' writes to these refs, telling them which Mac now
//! holds the agent; that refusal is the participant's own, outside this
//! contract. The
//! refs must be the set `prepare` named for this `cutId`, in any order;
//! another set is refused `invalid_state` ([`crate::step::check_ref_set`]).
//! Step: `exported` → `released`.

use serde::{Deserialize, Serialize};

use crate::refs::{CutId, Refs};

/// The `release` request: `{refs, cutId, holder}`. Strict: a request with
/// any other top-level member does not decode and is refused
/// `invalid_request`.
#[derive(Clone, Debug, PartialEq, Eq, Hash, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
#[non_exhaustive]
pub struct ReleaseRequest {
    pub refs: Refs,
    #[serde(rename = "cutId")]
    pub cut_id: CutId,
    /// The new holder's author pseudonym.
    pub holder: String,
}

impl ReleaseRequest {
    pub fn new(refs: Refs, cut_id: CutId, holder: impl Into<String>) -> Self {
        Self {
            refs,
            cut_id,
            holder: holder.into(),
        }
    }
}

/// The `release` reply: `{}`.
pub type ReleaseReply = crate::EmptyReply;
