//! `agent_data.release {refs, cutId, holder}`: on the source, once
//! entorhinal's move record reads `activated` (the receiver has made the
//! data live). The write freeze that `prepare` set becomes a durable
//! tombstone naming the new holder, and the old copy is hidden. The reply
//! says whether later local writes are blocked ([`ReleaseReply::enforced`]):
//! when they are, the participant refuses its own local callers' writes to
//! these refs, telling them which Mac now holds the agent (that refusal is
//! the participant's own, outside this contract); when they are not,
//! anything written afterwards becomes a fork that `diverged` reports on a
//! move back. The refs must be the set `prepare` named for this `cutId`, in any order;
//! another set is refused `invalid_state` ([`crate::step::check_ref_set`]).
//! Step: `exported` → `released`.
//!
//! The contract terms used here (source, receiver, cut, the door, driver
//! and the rest) are defined in the "Terms" section of `CONTRACT.md`.

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

/// The `release` reply: `{enforced}`. Strict, and `enforced` is required:
/// a reply without it does not decode.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
#[non_exhaustive]
pub struct ReleaseReply {
    /// `true`: later local writes for these refs are refused, naming the
    /// holder. `false`: the tombstone is recorded and the old copy hidden,
    /// but writes are not blocked, so anything written afterwards becomes a
    /// fork that `diverged` reports if the agent moves back here.
    pub enforced: bool,
}

impl ReleaseReply {
    pub fn new(enforced: bool) -> Self {
        Self { enforced }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn release_reply_requires_enforced() {
        for enforced in [true, false] {
            let wire = serde_json::json!({ "enforced": enforced });
            let reply: ReleaseReply = serde_json::from_value(wire.clone()).unwrap();
            assert_eq!(reply, ReleaseReply::new(enforced));
            assert_eq!(serde_json::to_value(reply).unwrap(), wire);
        }
        assert!(serde_json::from_value::<ReleaseReply>(serde_json::json!({})).is_err());
        assert!(
            serde_json::from_value::<ReleaseReply>(serde_json::json!({"enforced": "yes"})).is_err()
        );
    }
}
