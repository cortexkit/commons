//! Late results: calls a provider settles after its reply, pulled by their
//! custodian.
//!
//! A provider that settles calls later declares the `late_results` session
//! capability and serves [`crate::ops::LATE_RESULTS`] and
//! [`crate::ops::LATE_RESULTS_ACK`]. The custodian of a result (the scope's
//! owner for a direct call, or the carrier of an onward call) pulls it; the
//! provider never pushes, and serves each entry only to a caller whose
//! verified principal is its custodian. Each custodian has its own cursor and
//! acks only its own entries.

use serde::{Deserialize, Serialize};
use serde_json::{json, Map, Value};
use subc_protocol::ErrorBody;

/// The session capability a provider declares when it may settle calls later.
pub const CAPABILITY: &str = "late_results";

/// A cursor from another provider incarnation, or one past the entries the
/// provider has (a gap), is refused with this route error. The reader then
/// re-reads from `since: null` and deduplicates on [`IntakeKey`].
pub const CURSOR_INCARNATION_CHANGED: &str = "cursor_incarnation_changed";

/// Entry kinds. `kind` is an open string.
pub mod kinds {
    /// The call settled; the entry carries `result` or, when reduced,
    /// `outcome`.
    pub const RESULT: &str = "result";
    /// The call was held and will never run, for example because the provider
    /// restarted before dispatching it. The entry carries `reason`.
    pub const NOT_STARTED: &str = "not_started";
    /// A result expired unacked; only its identity and `settled_at` remain.
    pub const EXPIRED: &str = "expired";
}

/// `not_started` reasons. `reason` is an open string.
pub mod reasons {
    /// The provider restarted with the call Prepared or Authorized and never
    /// dispatched it.
    pub const RESTART_BEFORE_DISPATCH: &str = "restart_before_dispatch";
}

/// A position in one custodian's view of the log. The incarnation changes
/// whenever the provider restarts, so a cursor never silently survives a lost
/// sequence.
#[derive(Clone, Debug, PartialEq, Eq, Hash, Deserialize, Serialize)]
pub struct Cursor {
    /// Opaque; changes on every provider start.
    pub provider_incarnation: String,
    /// The last entry the cursor covers; `0` before any.
    pub seq: u64,
}

/// `late_results {since, limit?}`. `since: null` reads from the start.
///
/// Non-exhaustive so later optional members are additive: use
/// [`LateResultsRequest::new`] and the `with_*` setters, or decode one.
#[derive(Clone, Debug, Default, PartialEq, Eq, Deserialize, Serialize)]
#[non_exhaustive]
pub struct LateResultsRequest {
    pub since: Option<Cursor>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub limit: Option<u32>,
}

impl LateResultsRequest {
    /// Read from the start with no limit.
    pub fn new() -> Self {
        Self::default()
    }

    /// Continue reading after this cursor.
    pub fn with_since(mut self, since: Cursor) -> Self {
        self.since = Some(since);
        self
    }

    /// Limit the number of entries returned.
    pub fn with_limit(mut self, limit: u32) -> Self {
        self.limit = Some(limit);
        self
    }
}

/// The `late_results` reply.
#[derive(Clone, Debug, PartialEq, Deserialize, Serialize)]
pub struct LateResultsReply {
    /// The caller's entries after `since`, oldest first.
    pub entries: Vec<LateEntry>,
    /// Where the next read continues.
    pub cursor: Cursor,
    /// More entries are waiting beyond `limit`.
    pub more: bool,
}

/// `late_results.ack {through}`: the custodian has taken every one of its
/// entries up to and including `through`.
#[derive(Clone, Debug, PartialEq, Eq, Deserialize, Serialize)]
pub struct AckRequest {
    pub through: Cursor,
}

/// One late-result entry.
///
/// Non-exhaustive so later optional members are additive: use
/// [`LateEntry::new`] and the `with_*` setters, or decode one.
///
/// `kind` is decoded open: an entry of a kind this crate does not know still
/// decodes, keeps every member in `extra`, and is recorded and acked like
/// any other, never retried.
#[derive(Clone, Debug, PartialEq, Deserialize, Serialize)]
#[non_exhaustive]
pub struct LateEntry {
    pub kind: String,
    pub owner: String,
    #[serde(rename = "ref")]
    pub scope_ref: String,
    pub scope_epoch: u64,
    pub custodian: String,
    /// The call's key, qualified by the carrier that raised it.
    pub call_key: String,
    /// Present only for an onward request.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub invocation_id: Option<String>,
    /// Deterministic: the same late result always carries the same id,
    /// across provider restarts and lost acks.
    pub event_id: String,
    /// When the call settled (or was found not started, or expired), in
    /// milliseconds since the Unix epoch.
    pub settled_at: u64,
    /// The provider kept only the outcome, not the full result.
    #[serde(default)]
    pub reduced: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub result: Option<Value>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub outcome: Option<String>,
    /// Why a `not_started` call never ran.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub reason: Option<String>,
    /// Members this crate does not know, kept so an entry is recorded
    /// verbatim.
    #[serde(flatten)]
    pub extra: Map<String, Value>,
}

/// A decoded entry kind.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum EntryKind {
    Result,
    NotStarted,
    Expired,
    Unknown(String),
}

impl LateEntry {
    /// An entry with its required identity and settlement time, and no optional content.
    // Accept each required wire member directly rather than bundling unrelated fields to satisfy the argument-count lint.
    #[allow(clippy::too_many_arguments)]
    pub fn new(
        kind: impl Into<String>,
        owner: impl Into<String>,
        scope_ref: impl Into<String>,
        scope_epoch: u64,
        custodian: impl Into<String>,
        call_key: impl Into<String>,
        event_id: impl Into<String>,
        settled_at: u64,
    ) -> Self {
        Self {
            kind: kind.into(),
            owner: owner.into(),
            scope_ref: scope_ref.into(),
            scope_epoch,
            custodian: custodian.into(),
            call_key: call_key.into(),
            event_id: event_id.into(),
            settled_at,
            invocation_id: None,
            reduced: false,
            result: None,
            outcome: None,
            reason: None,
            extra: Map::new(),
        }
    }

    /// Name the onward invocation.
    pub fn with_invocation_id(mut self, invocation_id: impl Into<String>) -> Self {
        self.invocation_id = Some(invocation_id.into());
        self
    }

    /// Set whether only the outcome was retained.
    pub fn with_reduced(mut self, reduced: bool) -> Self {
        self.reduced = reduced;
        self
    }

    /// Include the retained result.
    pub fn with_result(mut self, result: Value) -> Self {
        self.result = Some(result);
        self
    }

    /// Include the retained outcome code.
    pub fn with_outcome(mut self, outcome: impl Into<String>) -> Self {
        self.outcome = Some(outcome.into());
        self
    }

    /// Explain why the call never started.
    pub fn with_reason(mut self, reason: impl Into<String>) -> Self {
        self.reason = Some(reason.into());
        self
    }

    /// Carry extension members verbatim.
    pub fn with_extra(mut self, extra: Map<String, Value>) -> Self {
        self.extra = extra;
        self
    }

    pub fn kind(&self) -> EntryKind {
        match self.kind.as_str() {
            kinds::RESULT => EntryKind::Result,
            kinds::NOT_STARTED => EntryKind::NotStarted,
            kinds::EXPIRED => EntryKind::Expired,
            other => EntryKind::Unknown(other.to_owned()),
        }
    }

    /// The key the custodian deduplicates on, across re-reads after a
    /// refused cursor.
    pub fn intake_key(&self, provider: &str) -> IntakeKey {
        IntakeKey {
            provider: provider.to_owned(),
            custodian: self.custodian.clone(),
            call_key: self.call_key.clone(),
            event_id: self.event_id.clone(),
        }
    }
}

/// `(provider, custodian, call_key, event_id)`: the custodian's dedupe key.
#[derive(Clone, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct IntakeKey {
    pub provider: String,
    pub custodian: String,
    pub call_key: String,
    pub event_id: String,
}

/// The provider's check of a `since` cursor: `null` is always accepted; a
/// cursor from another incarnation, or one past `head_seq` (the provider's
/// newest sequence in this incarnation), is refused as
/// `cursor_incarnation_changed`.
pub fn check_since(
    incarnation: &str,
    head_seq: u64,
    since: Option<&Cursor>,
) -> Result<(), ErrorBody> {
    let Some(since) = since else {
        return Ok(());
    };
    if since.provider_incarnation != incarnation || since.seq > head_seq {
        return Err(ErrorBody::new(
            CURSOR_INCARNATION_CHANGED,
            "the cursor is from another provider incarnation or past its log; re-read from null",
        )
        .with_detail(json!({ "provider_incarnation": incarnation })));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::vectors;

    #[test]
    fn late_result_vectors_decode_as_recorded() {
        let vectors = vectors::load("late-results.json");
        let reply: LateResultsReply = serde_json::from_value(vectors["reply"].clone()).unwrap();
        let kinds: Vec<EntryKind> = reply.entries.iter().map(LateEntry::kind).collect();
        assert_eq!(
            kinds,
            [
                EntryKind::Result,
                EntryKind::Result,
                EntryKind::NotStarted,
                EntryKind::Expired
            ]
        );
        assert!(reply.entries[1].reduced);
        assert_eq!(
            reply.entries[2].reason.as_deref(),
            Some(reasons::RESTART_BEFORE_DISPATCH)
        );
        assert_eq!(serde_json::to_value(&reply).unwrap(), vectors["reply"]);

        let request: LateResultsRequest =
            serde_json::from_value(vectors["first_request"].clone()).unwrap();
        assert_eq!(request.since, None);
        assert_eq!(
            serde_json::to_value(&request).unwrap(),
            vectors["first_request"]
        );
        let ack: AckRequest = serde_json::from_value(vectors["ack"].clone()).unwrap();
        assert_eq!(ack.through, reply.cursor);
        for case in vectors["malformed_entries"].as_array().unwrap() {
            assert!(
                serde_json::from_value::<LateEntry>(case["entry"].clone()).is_err(),
                "{}",
                case["name"]
            );
        }
    }

    #[test]
    fn an_unknown_kind_is_recorded_verbatim() {
        let vectors = vectors::load("late-results.json");
        for case in vectors["unknown_kinds"].as_array().unwrap() {
            let entry: LateEntry = serde_json::from_value(case["entry"].clone())
                .expect("an unknown kind still decodes");
            assert_eq!(
                entry.kind(),
                EntryKind::Unknown(case["entry"]["kind"].as_str().unwrap().to_owned())
            );
            assert_eq!(
                serde_json::to_value(&entry).unwrap(),
                case["entry"],
                "verbatim"
            );
        }
    }

    #[test]
    fn a_foreign_or_future_cursor_is_refused() {
        let cursor = |incarnation: &str, seq| Cursor {
            provider_incarnation: incarnation.into(),
            seq,
        };
        assert_eq!(check_since("i2", 5, None), Ok(()));
        assert_eq!(check_since("i2", 5, Some(&cursor("i2", 5))), Ok(()));
        assert_eq!(check_since("i2", 5, Some(&cursor("i2", 0))), Ok(()));
        let past = check_since("i2", 5, Some(&cursor("i2", 6))).unwrap_err();
        assert_eq!(past.code, CURSOR_INCARNATION_CHANGED);
        let foreign = check_since("i2", 5, Some(&cursor("i1", 3))).unwrap_err();
        assert_eq!(foreign.code, CURSOR_INCARNATION_CHANGED);
    }
}
