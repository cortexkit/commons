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

use std::collections::BTreeMap;

use serde::{de::Error as _, Deserialize, Deserializer, Serialize};

/// Maximum number of drop reasons in one imported item.
pub const DROPPED_MAX_REASONS: usize = 16;
/// Maximum ASCII byte length of a drop reason.
pub const DROPPED_REASON_MAX_BYTES: usize = 64;
/// Largest drop count exactly representable as a JSON safe integer (2^53 - 1).
pub const DROPPED_MAX_COUNT: u64 = (1 << 53) - 1;

/// The `import` request: `{refs, cutId}`.
pub type ImportRequest = crate::request::CutRequest;

/// The `import` reply: `{items: [{sessionRef, endOffset, digest, dropped?}]}`.
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
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
#[serde(deny_unknown_fields)]
#[non_exhaustive]
pub struct ImportedItem {
    #[serde(rename = "sessionRef")]
    pub session_ref: String,
    #[serde(rename = "endOffset")]
    pub end_offset: u64,
    pub digest: String,
    /// Participant-defined reasons and counts for entries dropped during import.
    /// Any reported drops mean the move must be shown as completed with losses,
    /// never as a clean success. Consumers display reasons without matching them.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub dropped: Option<BTreeMap<String, u64>>,
}

impl ImportedItem {
    pub fn new(session_ref: impl Into<String>, end_offset: u64, digest: impl Into<String>) -> Self {
        Self {
            session_ref: session_ref.into(),
            end_offset,
            digest: digest.into(),
            dropped: None,
        }
    }

    /// Report losses on this item, returning an error naming any violated rule.
    ///
    /// The map must contain 1–16 reasons. Each reason is 1–64 ASCII bytes,
    /// starts with a lowercase letter, and contains only lowercase letters,
    /// digits and `_`. Counts are positive and at most [`DROPPED_MAX_COUNT`].
    pub fn with_dropped(mut self, dropped: BTreeMap<String, u64>) -> Result<Self, &'static str> {
        validate_dropped(&dropped)?;
        self.dropped = Some(dropped);
        Ok(self)
    }
}

impl<'de> Deserialize<'de> for ImportedItem {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        #[derive(Deserialize)]
        #[serde(deny_unknown_fields)]
        struct RawImportedItem {
            #[serde(rename = "sessionRef")]
            session_ref: String,
            #[serde(rename = "endOffset")]
            end_offset: u64,
            digest: String,
            #[serde(default)]
            dropped: Option<BTreeMap<String, u64>>,
        }

        let raw = RawImportedItem::deserialize(deserializer)?;
        let item = Self::new(raw.session_ref, raw.end_offset, raw.digest);
        match raw.dropped {
            Some(dropped) => item.with_dropped(dropped).map_err(D::Error::custom),
            None => Ok(item),
        }
    }
}

fn validate_dropped(dropped: &BTreeMap<String, u64>) -> Result<(), &'static str> {
    if dropped.is_empty() {
        return Err("dropped must be non-empty");
    }
    if dropped.len() > DROPPED_MAX_REASONS {
        return Err("dropped must have at most 16 reasons");
    }
    for (reason, &count) in dropped {
        if reason.is_empty() {
            return Err("dropped reason must be non-empty");
        }
        if reason.len() > DROPPED_REASON_MAX_BYTES {
            return Err("dropped reason must be at most 64 ASCII bytes");
        }
        if !reason.as_bytes()[0].is_ascii_lowercase() {
            return Err("dropped reason must start with a lowercase ASCII letter");
        }
        if !reason
            .bytes()
            .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'_')
        {
            return Err("dropped reason must contain only lowercase ASCII letters, digits and `_`");
        }
        if count == 0 {
            return Err("dropped count must be positive");
        }
        if count > DROPPED_MAX_COUNT {
            return Err("dropped count must be at most 9007199254740991 (2^53 - 1)");
        }
    }
    Ok(())
}
