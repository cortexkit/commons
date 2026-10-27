//! `session.read` and `session.head`: the transcript reads.
//!
//! Three read modes share one request ([`ReadRequest::mode`]):
//!
//! - **tail**: no cursor. The newest page. This is the contract, not a
//!   default a runner may change.
//! - **range**: `from_ordinal`. Messages from that ordinal forward.
//! - **after**: `after_mid` with its `lineage_id`. Messages strictly after
//!   that message. Refused by name as `lineage_changed` or `unknown_mid`,
//!   never answered from the tail instead.
//!
//! Every read stays within the session's current lineage: the readable
//! history from its latest cut. Ordinals are session-wide and never
//! renumbered, so a lineage a cut started begins at the cut's first
//! retained ordinal, `first_ordinal` on [`HeadMeta`]. A range or
//! after-message read naming a position below it is refused
//! `before_lineage_start` with `detail.first_ordinal`
//! (`errors::BeforeLineageStartDetail`), and the tail is the newest page
//! within the lineage. Raw messages before the lineage are served by
//! `session.history` (the `host_history` group, [`crate::history`]), never
//! by this op; the model view (`view: "model"`) carries that earlier
//! history only as the compaction view's head.
//!
//! A page stops at `limit` messages or at the byte cap, whichever comes
//! first, and carries `next_from_ordinal` whenever it stops before the end
//! of the transcript. Every page carries the session's `lineage_id`; a read
//! that names another lineage is refused `lineage_changed`.
//!
//! A page is an ordinal range and nothing more: it is not aligned to turns,
//! runs or tool-call boundaries, so a tool call and its result may arrive on
//! different pages.
//!
//! With `view: "model"` the page is a [`ModelPage`]: the message list the
//! next model request would be built from, after hooks and compaction. Its
//! entries say where they come from ([`EntrySource`]), and tail and range
//! reads key on transcript ordinals as they do for the raw view. `after_mid`
//! is refused on the model view, because a message inside a replaced range
//! has no clean "after" there.

use serde::{Deserialize, Deserializer, Serialize};
use serde_json::{Map, Value};

pub mod descriptors;
pub use descriptors::MediaMode;

/// The `field` an `invalid_params` refusal names when a read combines
/// fields that exclude each other, or names `after_mid` without
/// `lineage_id`.
pub mod fields {
    pub const MEDIA: &str = "media";
    /// `session.history`'s opaque page cursor.
    pub const CURSOR: &str = "cursor";
    /// `session.history`'s forward start, inclusive.
    pub const FROM_MID: &str = "from_mid";
    /// `session.history`'s backward end, exclusive.
    pub const BEFORE_MID: &str = "before_mid";
    /// `session.history`'s backward seek by ordinal, exclusive.
    pub const BEFORE_ORDINAL: &str = "before_ordinal";
    pub const FROM_ORDINAL: &str = "from_ordinal";
    pub const LINEAGE_ID: &str = "lineage_id";
    pub const VIEW: &str = "view";
    /// The `session.read` opt-in to each assistant entry's rich UI stamp,
    /// served only by a runner declaring `rich_ui_stamp`.
    pub const INCLUDE_RICH_UI: &str = "include_rich_ui";
}

/// Which list of messages a read pages through.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Deserialize, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum ReadView {
    /// The transcript as written, with each message's final values. The
    /// default.
    Raw,
    /// The canonical message list the model is given after hooks and
    /// compaction, answered as a [`ModelPage`]. Served only by a runner that
    /// declares `model_view`. Tail and range reads only: `after_mid` is
    /// refused naming `view`.
    Model,
}

fn deserialize_view<'de, D>(deserializer: D) -> Result<Option<ReadView>, D::Error>
where
    D: Deserializer<'de>,
{
    crate::strict::optional(deserializer, fields::VIEW, "\"raw\" or \"model\"")
}

/// The `session.read` request.
///
/// Strict on unknown fields: absence is meaningful here (no cursor means
/// "the newest page"), so a misspelled cursor must be refused rather than
/// read as a tail request. A runner refuses a field it does not serve with
/// `invalid_params` naming it.
///
/// Non-exhaustive so a later optional member is additive: build one with
/// [`ReadRequest::tail`], [`ReadRequest::range`] or [`ReadRequest::after`]
/// and the `with_*` setters, or decode one.
#[derive(Clone, Debug, Default, PartialEq, Eq, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
#[non_exhaustive]
pub struct ReadRequest {
    /// Payload representation; absent means `inline`.
    #[serde(
        default,
        skip_serializing_if = "Option::is_none",
        deserialize_with = "descriptors::deserialize_media"
    )]
    pub media: Option<MediaMode>,
    /// Read forward from this ordinal. Excludes `after_mid`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub from_ordinal: Option<u64>,
    /// Read strictly after this message id. Requires `lineage_id`. A `mid`
    /// is opaque to consumers: only ever a value a page returned.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub after_mid: Option<String>,
    /// The lineage the cursor came from. A read naming a lineage other than
    /// the session's current one is refused `lineage_changed`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub lineage_id: Option<String>,
    /// At most this many messages. The runner may cap it lower.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub limit: Option<u64>,
    /// At most this many bytes of messages. The runner may cap it lower.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub max_bytes: Option<u64>,
    /// Add `originals` to every message a hook changed.
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub include_originals: bool,
    /// The view to page through. Absent means `raw`. The value is strict: an
    /// unknown view is refused naming `view`.
    #[serde(
        default,
        skip_serializing_if = "Option::is_none",
        deserialize_with = "deserialize_view"
    )]
    pub view: Option<ReadView>,
    /// Add `rich_ui` to every assistant entry a stamped run wrote. Served
    /// only by a runner declaring `rich_ui_stamp`; any other runner refuses
    /// it as it refuses any field it does not serve, `invalid_params` naming
    /// `include_rich_ui` ([`ReadRequest::check_rich_ui`]). Absent means
    /// `false`, and a read without it is answered exactly as before.
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub include_rich_ui: bool,
}

/// The mode a well-formed [`ReadRequest`] selects: the newest page, a range
/// from an ordinal, or the messages after a message id in a lineage.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ReadMode<'a> {
    Tail,
    Range { from_ordinal: u64 },
    After { mid: &'a str, lineage_id: &'a str },
}

impl ReadRequest {
    pub fn with_media(mut self, media: MediaMode) -> Self {
        self.media = Some(media);
        self
    }
    /// The newest page.
    pub fn tail() -> Self {
        Self::default()
    }

    /// Messages from `from_ordinal` forward.
    pub fn range(from_ordinal: u64) -> Self {
        Self {
            from_ordinal: Some(from_ordinal),
            ..Self::default()
        }
    }

    /// Messages strictly after `mid` in `lineage_id`.
    pub fn after(mid: impl Into<String>, lineage_id: impl Into<String>) -> Self {
        Self {
            after_mid: Some(mid.into()),
            lineage_id: Some(lineage_id.into()),
            ..Self::default()
        }
    }

    /// Check the cursor names `lineage_id`.
    pub fn with_lineage_id(mut self, lineage_id: impl Into<String>) -> Self {
        self.lineage_id = Some(lineage_id.into());
        self
    }

    pub fn with_limit(mut self, limit: u64) -> Self {
        self.limit = Some(limit);
        self
    }

    pub fn with_max_bytes(mut self, max_bytes: u64) -> Self {
        self.max_bytes = Some(max_bytes);
        self
    }

    pub fn with_include_originals(mut self, include_originals: bool) -> Self {
        self.include_originals = include_originals;
        self
    }

    pub fn with_view(mut self, view: ReadView) -> Self {
        self.view = Some(view);
        self
    }

    /// Ask for each stamped assistant entry's `rich_ui`. Send it only to a
    /// runner whose `role.describe` declares `rich_ui_stamp`.
    pub fn with_include_rich_ui(mut self, include_rich_ui: bool) -> Self {
        self.include_rich_ui = include_rich_ui;
        self
    }

    /// The field an `invalid_params` refusal names when the request asks
    /// for `include_rich_ui` from a runner whose declared `groups` lack
    /// `rich_ui_stamp`.
    pub fn check_rich_ui<S: AsRef<str>>(&self, groups: &[S]) -> Result<(), &'static str> {
        let declared = groups
            .iter()
            .any(|group| group.as_ref() == crate::capabilities::RICH_UI_STAMP);
        if self.include_rich_ui && !declared {
            return Err(fields::INCLUDE_RICH_UI);
        }
        Ok(())
    }

    /// The read mode, or the field an `invalid_params` refusal names:
    /// `view` when `after_mid` is combined with the model view,
    /// `from_ordinal` when it is combined with `after_mid`, `lineage_id`
    /// when `after_mid` comes without it.
    pub fn mode(&self) -> Result<ReadMode<'_>, &'static str> {
        if self.view == Some(ReadView::Model) && self.media == Some(MediaMode::Descriptors) {
            return Err(fields::MEDIA);
        }
        if self.after_mid.is_some() && self.view == Some(ReadView::Model) {
            return Err(fields::VIEW);
        }
        match (&self.after_mid, self.from_ordinal, &self.lineage_id) {
            (Some(_), Some(_), _) => Err(fields::FROM_ORDINAL),
            (Some(_), None, None) => Err(fields::LINEAGE_ID),
            (Some(mid), None, Some(lineage_id)) => Ok(ReadMode::After { mid, lineage_id }),
            (None, Some(from_ordinal), _) => Ok(ReadMode::Range { from_ordinal }),
            (None, None, _) => Ok(ReadMode::Tail),
        }
    }
}

/// A `session.read` page. Decoded leniently: unknown fields are ignored.
///
/// Non-exhaustive so later optional members are additive: use
/// [`ReadPage::new`], [`ReadPage::no_lineage`] and the `with_*` setters, or
/// decode one.
#[derive(Clone, Debug, PartialEq, Deserialize, Serialize)]
#[non_exhaustive]
pub struct ReadPage {
    /// Original identities of the pruned parts served on this page, on every read.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub pruned_reasoning: Vec<crate::archive_prunes_reasoning::PrunedReasoning>,
    /// The session's prune state as of this page, the same value
    /// `session.head` reports. Absent when the session was never pruned or
    /// the runner does not declare `archive_prunes_reasoning`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub reasoning_pruned: Option<crate::archive_prunes_reasoning::ReasoningPruned>,
    pub messages: Vec<ReadMessage>,
    /// The session's lineage, on every page of a session that has one.
    /// Absent means the session has no lineage yet: it was never written,
    /// and the page is empty.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub lineage_id: Option<String>,
    /// Present whenever the page stopped before the end of the transcript,
    /// by count or by bytes: the ordinal to read from next. A single message
    /// larger than the byte cap comes alone on its page, over the cap, with
    /// this set if more follow; a message is never truncated. The complete
    /// encoded reply is served only if it fits the transport frame limit
    /// ([`crate::TRANSPORT_FRAME_LIMIT`], 4,194,304 bytes) including the
    /// 21-byte header; otherwise it is refused `reply_too_large`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub next_from_ordinal: Option<u64>,
    /// The position of the last durable event covered by the same snapshot
    /// as this page: a JSON object, opaque to consumers. A subscription from
    /// it continues strictly after it. Absent on a session with no events
    /// yet.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub head: Option<Map<String, Value>>,
}

impl ReadPage {
    /// A page of a session whose lineage is `lineage_id`.
    pub fn new(lineage_id: impl Into<String>, messages: Vec<ReadMessage>) -> Self {
        Self {
            pruned_reasoning: Vec::new(),
            reasoning_pruned: None,
            messages,
            lineage_id: Some(lineage_id.into()),
            next_from_ordinal: None,
            head: None,
        }
    }

    /// The page of a session that was never written: empty, with no lineage
    /// and no `next_from_ordinal`.
    pub fn no_lineage() -> Self {
        Self {
            pruned_reasoning: Vec::new(),
            reasoning_pruned: None,
            messages: Vec::new(),
            lineage_id: None,
            next_from_ordinal: None,
            head: None,
        }
    }

    pub fn with_next_from_ordinal(mut self, next_from_ordinal: u64) -> Self {
        self.next_from_ordinal = Some(next_from_ordinal);
        self
    }

    pub fn with_pruned_reasoning(
        mut self,
        parts: Vec<crate::archive_prunes_reasoning::PrunedReasoning>,
    ) -> Self {
        self.pruned_reasoning = parts;
        self
    }

    pub fn with_head(mut self, head: Map<String, Value>) -> Self {
        self.head = Some(head);
        self
    }

    pub fn with_reasoning_pruned(
        mut self,
        state: crate::archive_prunes_reasoning::ReasoningPruned,
    ) -> Self {
        self.reasoning_pruned = Some(state);
        self
    }

    /// Whether the page is consistent with its lineage: a page without a
    /// lineage holds no messages and no `next_from_ordinal`, because a
    /// session that has messages has a lineage.
    pub fn lineage_consistent(&self) -> bool {
        self.lineage_id.is_some() || (self.messages.is_empty() && self.next_from_ordinal.is_none())
    }
}

/// A `session.read` page of the model view (`view: "model"`). Decoded
/// leniently: unknown fields are ignored.
///
/// It names the compaction state it reflects (`compaction_id` and
/// `version`, both absent before any compaction applied), so a reader knows
/// which applied CompactionMessage it saw: the compaction provider's answer
/// that replaces ranges of raw history with replacement messages. It is what the next request would
/// be built from, not the bytes a model provider was sent.
///
/// Non-exhaustive so later optional members are additive: use
/// [`ModelPage::new`], [`ModelPage::no_lineage`] and the `with_*` setters,
/// or decode one.
#[derive(Clone, Debug, PartialEq, Deserialize, Serialize)]
#[non_exhaustive]
pub struct ModelPage {
    /// Pruned stored parts actually served, excluding parts hidden by compaction.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub pruned_reasoning: Vec<crate::archive_prunes_reasoning::PrunedReasoning>,
    /// The session's prune state, as on a raw page
    /// ([`ReadPage::reasoning_pruned`]).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub reasoning_pruned: Option<crate::archive_prunes_reasoning::ReasoningPruned>,
    /// Entries in non-decreasing anchor order, insertions first at a tie.
    pub messages: Vec<ModelEntry>,
    /// The session's lineage, as on a raw page ([`ReadPage::lineage_id`]):
    /// absent only for a session never written, whose page is empty.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub lineage_id: Option<String>,
    /// The transcript ordinal from which the next page should continue,
    /// present when this page stopped early. For an entry anchored at ordinal
    /// A, the cursor is at least A + 1 for a message, or the replacement's
    /// exclusive end. All insertions at A travel with that anchored entry,
    /// even if doing so exceeds the requested count or byte cap; tail
    /// insertions have no next cursor. The complete encoded reply is served
    /// only if it fits the transport frame limit
    /// ([`crate::TRANSPORT_FRAME_LIMIT`], 4,194,304 bytes) including the
    /// 21-byte header. Otherwise the reply is refused `reply_too_large`; on a
    /// model-view page, its detail identifies the group of insertions and
    /// their anchored entry by that entry's `source`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub next_from_ordinal: Option<u64>,
    /// The position of the last durable event covered by the same snapshot
    /// as this page, as on a raw page ([`ReadPage::head`]): opaque to
    /// consumers, absent on a session with no events yet.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub head: Option<Map<String, Value>>,
    /// The compaction whose latest applied CompactionMessage this page
    /// reflects. Absent, with `version`, before any compaction applied.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub compaction_id: Option<String>,
    /// The version of that CompactionMessage. Present exactly when
    /// `compaction_id` is.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub version: Option<u64>,
}

/// One entry of the model view: a message as the next request would carry
/// it, and where it comes from.
///
/// Non-exhaustive so later optional members are additive: use
/// [`ModelEntry::new`], or decode one.
#[derive(Clone, Debug, PartialEq, Deserialize, Serialize)]
#[non_exhaustive]
pub struct ModelEntry {
    /// Which transcript message, or which replaced range, this entry is.
    pub source: EntrySource,
    /// The message itself, in the runner's own schema.
    #[serde(deserialize_with = "descriptors::deserialize_content")]
    pub message: Value,
}

/// Where a model-view entry comes from. Tagged by `kind`.
#[derive(Clone, Debug, PartialEq, Eq, Deserialize, Serialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum EntrySource {
    /// A transcript message passed through, with its hooks applied. It keeps
    /// its transcript ordinal and `mid`.
    Message { ordinal: u64, mid: String },
    /// A compaction replacement standing for the half-open raw transcript
    /// range `[from_ordinal, to_ordinal)`, from the CompactionMessage named by
    /// `compaction_id` and `version`. An empty range inserts before
    /// `from_ordinal`; Setup's head message is `[0, 0)`.
    Replacement {
        compaction_id: String,
        version: u64,
        from_ordinal: u64,
        to_ordinal: u64,
    },
}

impl EntrySource {
    /// The transcript ordinal that anchors the entry, including an insertion.
    pub fn from_ordinal(&self) -> u64 {
        match self {
            Self::Message { ordinal, .. } => *ordinal,
            Self::Replacement { from_ordinal, .. } => *from_ordinal,
        }
    }

    /// The last covered transcript ordinal, or `None` for an empty or
    /// inverted replacement range.
    pub fn last_covered_ordinal(&self) -> Option<u64> {
        match self {
            Self::Message { ordinal, .. } => Some(*ordinal),
            Self::Replacement {
                from_ordinal,
                to_ordinal,
                ..
            } => (to_ordinal > from_ordinal).then(|| to_ordinal - 1),
        }
    }

    /// Whether this entry inserts without covering a transcript message.
    pub fn is_insertion(&self) -> bool {
        matches!(self, Self::Replacement { from_ordinal, to_ordinal, .. } if from_ordinal == to_ordinal)
    }
}

impl ModelEntry {
    pub fn new(source: EntrySource, message: Value) -> Self {
        Self { source, message }
    }
}

/// Why a [`ModelPage`] breaks the model view's paging rules. A consumer
/// treats such a page as malformed; it is not a refusal.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ModelPageProblem {
    /// The page has no lineage but carries entries, a next ordinal or a
    /// compaction state.
    NoLineage,
    /// Only one of `compaction_id` and `version` is present.
    PartialCompactionState,
    /// The page carries a replacement but names no compaction state.
    ReplacementWithoutCompactionState,
    /// A replacement's `to_ordinal` is below its `from_ordinal`.
    InvertedRange { from_ordinal: u64, to_ordinal: u64 },
    /// An entry starts below the one before it.
    OutOfOrder { from_ordinal: u64 },
    /// An insertion follows a message or non-empty replacement at the same
    /// anchor. Insertions must precede both at equal ordinals.
    InsertionAfterEntry { from_ordinal: u64 },
    /// The page ends with an insertion but has a continuation cursor. The
    /// anchored entry must travel on this page too; only tail insertions
    /// may end a page, and their page has no cursor.
    NonTailInsertion { from_ordinal: u64 },
    /// An entry starts below the ordinal the page was read from. A
    /// replacement belongs only on the page holding its `from_ordinal`; a
    /// page read from inside the replaced range must not repeat it.
    StartsBeforePage {
        entry_from_ordinal: u64,
        from_ordinal: u64,
    },
    /// `next_from_ordinal` is not past a covered ordinal or an insertion
    /// anchor whose anchored entry is absent, so the next page would overlap.
    NextInsideEntry {
        next_from_ordinal: u64,
        last_ordinal: u64,
    },
}

impl ModelPage {
    pub fn with_reasoning_pruned(
        mut self,
        state: crate::archive_prunes_reasoning::ReasoningPruned,
    ) -> Self {
        self.reasoning_pruned = Some(state);
        self
    }

    pub fn with_pruned_reasoning(
        mut self,
        parts: Vec<crate::archive_prunes_reasoning::PrunedReasoning>,
    ) -> Self {
        self.pruned_reasoning = parts;
        self
    }
    /// A page of a session whose lineage is `lineage_id`, before any
    /// compaction applied.
    pub fn new(lineage_id: impl Into<String>, messages: Vec<ModelEntry>) -> Self {
        Self {
            pruned_reasoning: Vec::new(),
            reasoning_pruned: None,
            messages,
            lineage_id: Some(lineage_id.into()),
            next_from_ordinal: None,
            head: None,
            compaction_id: None,
            version: None,
        }
    }

    /// The page of a session that was never written.
    pub fn no_lineage() -> Self {
        Self {
            pruned_reasoning: Vec::new(),
            reasoning_pruned: None,
            messages: Vec::new(),
            lineage_id: None,
            next_from_ordinal: None,
            head: None,
            compaction_id: None,
            version: None,
        }
    }

    pub fn with_next_from_ordinal(mut self, next_from_ordinal: u64) -> Self {
        self.next_from_ordinal = Some(next_from_ordinal);
        self
    }

    pub fn with_head(mut self, head: Map<String, Value>) -> Self {
        self.head = Some(head);
        self
    }

    /// Name the compaction state the page reflects.
    pub fn with_compaction(mut self, compaction_id: impl Into<String>, version: u64) -> Self {
        self.compaction_id = Some(compaction_id.into());
        self.version = Some(version);
        self
    }

    /// Check the model view's paging rules for a range read from
    /// `from_ordinal` (`None` for a tail read, whose start the runner chose).
    /// Entries have non-decreasing anchors, with insertions before messages
    /// or non-empty replacements at a tie. Replacements come whole, once,
    /// on the page holding their anchor. Insertions travel together with
    /// their anchored entry even over the page caps. Only tail insertions
    /// may end a page, without a cursor. The next cursor is past the anchored
    /// entry, not a reason to skip it. A replacement requires compaction state.
    pub fn check(&self, from_ordinal: Option<u64>) -> Result<(), ModelPageProblem> {
        let has_compaction_state = self.compaction_id.is_some() || self.version.is_some();
        if self.lineage_id.is_none()
            && (!self.messages.is_empty()
                || self.next_from_ordinal.is_some()
                || has_compaction_state)
        {
            return Err(ModelPageProblem::NoLineage);
        }
        if self.compaction_id.is_some() != self.version.is_some() {
            return Err(ModelPageProblem::PartialCompactionState);
        }
        let mut previous: Option<(u64, bool)> = None;
        for entry in &self.messages {
            let anchor = entry.source.from_ordinal();
            let insertion = entry.source.is_insertion();
            if let EntrySource::Replacement {
                from_ordinal,
                to_ordinal,
                ..
            } = entry.source
            {
                if to_ordinal < from_ordinal {
                    return Err(ModelPageProblem::InvertedRange {
                        from_ordinal,
                        to_ordinal,
                    });
                }
                if !has_compaction_state {
                    return Err(ModelPageProblem::ReplacementWithoutCompactionState);
                }
            }
            if let Some(from_ordinal) = from_ordinal {
                if anchor < from_ordinal {
                    return Err(ModelPageProblem::StartsBeforePage {
                        entry_from_ordinal: anchor,
                        from_ordinal,
                    });
                }
            }
            if let Some((previous_anchor, previous_insertion)) = previous {
                if anchor < previous_anchor {
                    return Err(ModelPageProblem::OutOfOrder {
                        from_ordinal: anchor,
                    });
                }
                if anchor == previous_anchor && insertion && !previous_insertion {
                    return Err(ModelPageProblem::InsertionAfterEntry {
                        from_ordinal: anchor,
                    });
                }
            }
            previous = Some((anchor, insertion));
        }
        if let Some(next_from_ordinal) = self.next_from_ordinal {
            if let Some(entry) = self.messages.last() {
                if entry.source.is_insertion() {
                    return Err(ModelPageProblem::NonTailInsertion {
                        from_ordinal: entry.source.from_ordinal(),
                    });
                }
            }
            for entry in &self.messages {
                // Covered entries bound the cursor. An unpaired insertion
                // also prevents a cursor from landing on or before its anchor.
                let last_ordinal = entry
                    .source
                    .last_covered_ordinal()
                    .unwrap_or_else(|| entry.source.from_ordinal());
                if next_from_ordinal <= last_ordinal {
                    return Err(ModelPageProblem::NextInsideEntry {
                        next_from_ordinal,
                        last_ordinal,
                    });
                }
            }
        }
        Ok(())
    }
}

/// One message on a page.
///
/// Non-exhaustive so later optional members are additive: use
/// [`ReadMessage::new`] and the `with_*` setters, or decode one.
/// External callers cannot construct one with a struct literal:
///
/// ```compile_fail,E0639
/// use cortexkit_role_llm_runner::read::ReadMessage;
/// let message = ReadMessage {
///     ordinal: 0,
///     mid: "m0".into(),
///     message: serde_json::json!({}),
///     run: None,
///     originals: None,
///     tool_calls: Vec::new(),
/// };
/// ```
#[derive(Clone, Debug, PartialEq, Deserialize, Serialize)]
#[non_exhaustive]
pub struct ReadMessage {
    /// Write-time content identity over the original full inline message.
    /// Required in descriptors mode, optional in inline mode; never computed
    /// over refs or a payload changed by archive pruning.
    #[serde(
        default,
        skip_serializing_if = "Option::is_none",
        deserialize_with = "descriptors::deserialize_digest"
    )]
    pub digest: Option<String>,
    /// Original write-time identities of every top-level part in message
    /// order, nested media excluded. Required
    /// in descriptors mode, optional in inline mode. Descriptors list exactly
    /// the same keys as this entry's complete `tags` rows, in the same order.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub part_digests: Option<Vec<descriptors::PartDigest>>,
    /// The message's position in the transcript. Never reused or
    /// renumbered within a lineage.
    pub ordinal: u64,
    /// The message's id, opaque to consumers. Never reused within a
    /// lineage.
    pub mid: String,
    /// The message itself, carrying its final values: what later steps
    /// render and what executed. Its schema is the runner's.
    #[serde(deserialize_with = "descriptors::deserialize_content")]
    pub message: Value,
    /// Which run produced the message. Present on a runner that declares
    /// `run_ops`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub run: Option<RunAttribution>,
    /// What hooks changed, oldest first. Present only when the request set
    /// `include_originals` and a hook changed this message.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub originals: Option<Vec<Original>>,
    /// One entry per tool call in the message. Present on a runner that
    /// declares `dispatch_attribution`, for messages that carry tool calls.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub tool_calls: Vec<ToolCallAttribution>,
    /// One row per part the runner exposes, in part order: the part's
    /// stable key, and its `§N§` tag in the current lineage when tagged.
    /// Present on a runner that declares `tags`. Digests ride separately in
    /// `part_digests`, including on runners without tags.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub tags: Vec<crate::tags::PartTag>,
    /// The message's role (`role::USER`, `role::ASSISTANT`, `role::TOOL`,
    /// `role::SYSTEM`), decoded as an open string, so a consumer learns who
    /// spoke without decoding `message`. Absent when the runner does not
    /// know it.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub role: Option<String>,
    /// When the message was written (or first ingested, for a message the
    /// host stored itself), in milliseconds since the Unix epoch. Absent
    /// when the runner does not know it.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub written_at: Option<u64>,
    /// On the entry a `delivery_order` send wrote when it was delivered: the
    /// mode it went out in and, for `tool_result`, the tool result it went
    /// out with, so readers and a resumed run render exactly what was sent.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub delivered_as: Option<crate::delivery_order::EntryDelivery>,
    /// Only when the request set `include_rich_ui`, on a runner declaring
    /// `rich_ui_stamp`: the rich UI stamp of the run that wrote this
    /// assistant message, copied verbatim from that run's frozen plan
    /// (CONTRACT §9.17). It rides beside `mid`, never inside `message`, so
    /// no body, profile or digest changes. With the opt-in, absent on an
    /// assistant entry means unstamped. Never on a user, steered, tool,
    /// imported or adopted entry, and never without the opt-in.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub rich_ui: Option<crate::rich_ui::RichUiStamp>,
}

/// The roles a read entry's `role` names. Decoded as open strings: a
/// runner's own role decodes as itself.
pub mod role {
    /// A message into the session: a prompt, a send, a wake or a reminder.
    pub const USER: &str = "user";
    /// The model's own output.
    pub const ASSISTANT: &str = "assistant";
    /// A tool's result, on a runner that stores it as a message of its own.
    pub const TOOL: &str = "tool";
    /// System text the runner stores in the transcript.
    pub const SYSTEM: &str = "system";
}

impl ReadMessage {
    pub fn new(ordinal: u64, mid: impl Into<String>, message: Value) -> Self {
        Self {
            ordinal,
            digest: None,
            part_digests: None,
            mid: mid.into(),
            message,
            run: None,
            originals: None,
            tool_calls: Vec::new(),
            tags: Vec::new(),
            role: None,
            written_at: None,
            delivered_as: None,
            rich_ui: None,
        }
    }

    /// Stamp this assistant entry with its run's rich UI stamp, for a read
    /// that set `include_rich_ui`.
    pub fn with_rich_ui(mut self, rich_ui: crate::rich_ui::RichUiStamp) -> Self {
        self.rich_ui = Some(rich_ui);
        self
    }

    pub fn with_delivered_as(mut self, delivered_as: crate::delivery_order::EntryDelivery) -> Self {
        self.delivered_as = Some(delivered_as);
        self
    }

    pub fn with_role(mut self, role: impl Into<String>) -> Self {
        self.role = Some(role.into());
        self
    }

    pub fn with_written_at(mut self, written_at: u64) -> Self {
        self.written_at = Some(written_at);
        self
    }

    pub fn with_tags(mut self, tags: Vec<crate::tags::PartTag>) -> Self {
        self.tags = tags;
        self
    }

    pub fn with_run(mut self, run: RunAttribution) -> Self {
        self.run = Some(run);
        self
    }

    pub fn with_originals(mut self, originals: Vec<Original>) -> Self {
        self.originals = Some(originals);
        self
    }

    pub fn with_tool_calls(mut self, tool_calls: Vec<ToolCallAttribution>) -> Self {
        self.tool_calls = tool_calls;
        self
    }
}

/// The run and episode that produced a message, and whether the message is
/// that run's final one.
#[derive(Clone, Debug, PartialEq, Eq, Deserialize, Serialize)]
pub struct RunAttribution {
    pub run_id: String,
    /// The episode within the session, opaque to consumers.
    pub episode: String,
    /// Whether this is the run's final message.
    #[serde(rename = "final")]
    pub is_final: bool,
}

/// One hooked field's value before a hook changed it.
#[derive(Clone, Debug, PartialEq, Deserialize, Serialize)]
pub struct Original {
    /// The message field the hook changed, as the runner's message schema
    /// names it.
    pub field: String,
    /// The value before the change.
    pub value: Value,
    /// The module whose hook changed it.
    pub provider: String,
}

/// Where one tool call went.
///
/// Non-exhaustive so later optional members are additive: use
/// [`ToolCallAttribution::new`] and the `with_*` setters, or decode one.
#[derive(Clone, Debug, PartialEq, Eq, Deserialize, Serialize)]
#[non_exhaustive]
pub struct ToolCallAttribution {
    /// The model's id for the call. Display only: it is not unique.
    pub tool_call_id: String,
    /// The call's key, unique per runner: present once the call was
    /// prepared or dispatched.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub call_key: Option<String>,
    /// The module the call was dispatched to. Absent for a call never
    /// dispatched (denied, say).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub dispatched_to: Option<String>,
    /// The call has a durable dispatch intent and no result: it may or may
    /// not have run.
    #[serde(default)]
    pub indeterminate: bool,
}

impl ToolCallAttribution {
    pub fn new(tool_call_id: impl Into<String>) -> Self {
        Self {
            tool_call_id: tool_call_id.into(),
            call_key: None,
            dispatched_to: None,
            indeterminate: false,
        }
    }

    pub fn with_call_key(mut self, call_key: impl Into<String>) -> Self {
        self.call_key = Some(call_key.into());
        self
    }

    pub fn with_dispatched_to(mut self, module: impl Into<String>) -> Self {
        self.dispatched_to = Some(module.into());
        self
    }

    pub fn with_indeterminate(mut self, indeterminate: bool) -> Self {
        self.indeterminate = indeterminate;
        self
    }
}

/// The `session.head` request. It takes no fields, and refuses any.
#[derive(Clone, Debug, Default, PartialEq, Eq, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct HeadRequest {}

/// The `session.head` answer: where the transcript stands, without message
/// bodies. Decoded leniently.
///
/// It follows the read rule for a session that was never written: no
/// `lineage_id`, and then none of the lineage-dependent members either
/// ([`HeadMeta::no_lineage`], checked by [`HeadMeta::lineage_consistent`]).
///
/// Non-exhaustive so later optional members are additive: use
/// [`HeadMeta::new`], [`HeadMeta::no_lineage`] and the `with_*` setters, or
/// decode one.
#[derive(Clone, Debug, PartialEq, Eq, Deserialize, Serialize)]
#[non_exhaustive]
pub struct HeadMeta {
    /// The session's lineage. Absent means the session has no lineage yet:
    /// it was never written.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub lineage_id: Option<String>,
    /// The newest message's ordinal. Absent on an empty transcript.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub last_ordinal: Option<u64>,
    /// The current lineage's first ordinal: the lowest position a range
    /// read, an after-message read or `session.resolve` may name. Ordinals
    /// are session-wide, so after a cut it is the cut's first retained
    /// ordinal, not 0. Present whenever `lineage_id` is, and at most one above
    /// `last_ordinal` (a cut that retained nothing).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub first_ordinal: Option<u64>,
    /// The cut that started the current lineage's readable history,
    /// described without its content. Absent on a lineage with no cut, whose
    /// `first_ordinal` is then 0.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub cut: Option<HeadCut>,
    /// The version of the session's visible history. It rises when the
    /// lineage changes because of a revert, a recovery that rewrites
    /// written messages, or a restart after the runner lost its state. It
    /// does not change at a cut or on an append, and it never decreases. A reader holding a
    /// position in the visible history keys on it, so a compaction does not
    /// invalidate the position while a rewrite does. Present whenever
    /// `lineage_id` is.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub history_version: Option<u64>,
    /// The same opaque position a page's `head` carries.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub head: Option<Map<String, Value>>,
    /// The session's most recent run. Absent before the first run.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub last_run_state: Option<LastRunState>,
    /// Retry classification and timing for that same run, only when
    /// `last_run_state.state` is `"error"`. Paused and interrupted runs carry
    /// none: their `reason` already explains why they stopped. Absence on an
    /// error means not reported, never permanent; no describe group is needed.
    /// See [`LastRunError`] for the source and intentionally omitted details.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub last_run_error: Option<LastRunError>,
    /// On a runner declaring `rich_ui_stamp`: the rich UI stamp of the run
    /// `last_run_state` names, present exactly while that run's frozen plan
    /// carries one (CONTRACT §9.17). It describes that same run, so after a
    /// later unstamped run it is gone. It sits beside `last_run_state` rather
    /// than inside it, so code that builds [`LastRunState`] directly keeps
    /// compiling. Absent on a runner without the group says nothing.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub last_run_rich_ui: Option<crate::rich_ui::RichUiStamp>,
    /// When the transcript last changed, in milliseconds since the Unix
    /// epoch. Present whenever `lineage_id` is.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub updated_at: Option<u64>,
    /// What a send that names no model can carry. Absent means unknown.
    /// Outside the lineage rule: a never-written session may state it.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub input: Option<HeadInput>,
    /// The runner's recorded selection for the next send that names no model,
    /// not the frozen plan's original selection. Absent means unknown.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub model: Option<HeadModel>,
    /// Durable session spend, present only with the `session_usage` group.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub usage: Option<crate::usage::SessionUsage>,
    /// The attested caller's grant under the plan checked on its next send.
    /// With the `head_sender` group, absent means no grant for this caller;
    /// without the group, absent means unknown. Use [`Self::sender_grant`].
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub sender: Option<HeadSender>,
    /// Opaque identity of this key's log, stable across its entire lifetime.
    /// Absent means unknown; compare only by equality, not lexical order.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub incarnation: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub retention_policy: Option<crate::session_retention::RetentionPolicy>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub retention_state: Option<crate::session_retention::RetentionState>,
    /// Recorded release and the end of its minimum storage hold, until deletion.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub released: Option<crate::session_retention::Released>,
    /// The session's prune state (CONTRACT §9.14.1), the authority every
    /// copy on pages, Setup requests and notices repeats. Only on a runner
    /// that declares `archive_prunes_reasoning`, and absent until the
    /// session's first prune, so a session never written has none. A prune
    /// is not a lineage change: the value survives cuts and reverts.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub reasoning_pruned: Option<crate::archive_prunes_reasoning::ReasoningPruned>,
}

/// The caller's own delivery grant on `session.head`, independent of lineage.
#[derive(Clone, Debug, PartialEq, Eq, Deserialize, Serialize)]
#[non_exhaustive]
pub struct HeadSender {
    /// Delivery-mode names exactly as the effective plan uses them.
    pub modes: Vec<String>,
    pub interrupt: bool,
    /// Encoded only for the owner, as `owner: true`.
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub owner: bool,
}

/// The caller's delivery grant, interpreted using the runner's describe groups.
#[derive(Clone, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub enum SenderGrant {
    /// The runner reported the caller's effective grant.
    Granted(HeadSender),
    /// The runner declares `head_sender` and reported no grant for this caller.
    NoGrant,
    /// No sender was reported and the runner does not declare `head_sender`.
    Unknown,
}

impl HeadSender {
    pub fn new(modes: Vec<String>, interrupt: bool) -> Self {
        Self {
            modes,
            interrupt,
            owner: false,
        }
    }

    /// Every mode accepted from the owner on its next send, with interruption.
    pub fn for_owner(modes: Vec<String>) -> Self {
        Self {
            modes,
            interrupt: true,
            owner: true,
        }
    }
}

/// The next-send selection on `session.head`, independent of its lineage.
#[derive(Clone, Debug, PartialEq, Eq, Deserialize, Serialize)]
#[non_exhaustive]
pub struct HeadModel {
    pub provider: String,
    pub model: String,
    /// Absent when the recorded selection names no variant.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub variant: Option<String>,
}

impl HeadModel {
    pub fn new(provider: impl Into<String>, model: impl Into<String>) -> Self {
        Self {
            provider: provider.into(),
            model: model.into(),
            variant: None,
        }
    }

    pub fn with_variant(mut self, variant: impl Into<String>) -> Self {
        self.variant = Some(variant.into());
        self
    }
}

/// `session.head.input`: what a send that names no model can carry.
/// Decoded leniently.
///
/// Non-exhaustive: build one with [`HeadInput::new`] and the `with_*`
/// setters, or decode one.
#[derive(Clone, Debug, PartialEq, Eq, Deserialize, Serialize)]
#[non_exhaustive]
pub struct HeadInput {
    /// Whether such a send can carry attachments: `yes`, `no` or `unknown`
    /// ([`attachments`]), decoded open. A fact the runner lacks is
    /// `unknown`, never `no`; `no` is a known fact, such as a runner that
    /// cannot carry attachments at all.
    pub attachments: String,
    /// The input modalities the model accepts, as the model catalog states
    /// them (`text`, `image`, `pdf`, `audio`, ...).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub modalities: Option<Vec<String>>,
    /// The media types the runner's own provider adapter accepts. Absent
    /// means the modalities are accepted but the exact types are unknown.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub mime_types: Option<Vec<String>>,
}

/// The values of [`HeadInput::attachments`].
pub mod attachments {
    pub const YES: &str = "yes";
    pub const NO: &str = "no";
    pub const UNKNOWN: &str = "unknown";
}

impl HeadInput {
    pub fn new(attachments: impl Into<String>) -> Self {
        Self {
            attachments: attachments.into(),
            modalities: None,
            mime_types: None,
        }
    }

    pub fn with_modalities(mut self, modalities: Vec<String>) -> Self {
        self.modalities = Some(modalities);
        self
    }

    pub fn with_mime_types(mut self, mime_types: Vec<String>) -> Self {
        self.mime_types = Some(mime_types);
        self
    }

    /// Whether attachments are known to be carried (`Some(true)`), known
    /// not to be (`Some(false)`), or not known (`None`, an unknown value
    /// included).
    pub fn carries_attachments(&self) -> Option<bool> {
        match self.attachments.as_str() {
            attachments::YES => Some(true),
            attachments::NO => Some(false),
            _ => None,
        }
    }
}

impl HeadMeta {
    pub fn with_reasoning_pruned(
        mut self,
        state: crate::archive_prunes_reasoning::ReasoningPruned,
    ) -> Self {
        self.reasoning_pruned = Some(state);
        self
    }
    pub fn with_released(mut self, released: crate::session_retention::Released) -> Self {
        self.released = Some(released);
        self
    }
    pub fn with_retention_policy(
        mut self,
        policy: crate::session_retention::RetentionPolicy,
        state: crate::session_retention::RetentionState,
    ) -> Self {
        self.retention_policy = Some(policy);
        self.retention_state = Some(state);
        self
    }
    /// The head of a session whose lineage is `lineage_id` and starts at
    /// `first_ordinal`. Add the history version with
    /// [`HeadMeta::with_history_version`] and, when `first_ordinal` is above
    /// 0, the lineage's cut with [`HeadMeta::with_cut`]: a head without them
    /// is not [`HeadMeta::lineage_consistent`].
    pub fn new(lineage_id: impl Into<String>, first_ordinal: u64, updated_at: u64) -> Self {
        Self {
            lineage_id: Some(lineage_id.into()),
            last_ordinal: None,
            first_ordinal: Some(first_ordinal),
            cut: None,
            history_version: None,
            head: None,
            last_run_state: None,
            last_run_error: None,
            last_run_rich_ui: None,
            updated_at: Some(updated_at),
            input: None,
            model: None,
            usage: None,
            sender: None,
            incarnation: None,
            retention_policy: None,
            retention_state: None,
            released: None,
            reasoning_pruned: None,
        }
    }

    /// The head of a session that was never written: every member absent.
    pub fn no_lineage() -> Self {
        Self {
            lineage_id: None,
            last_ordinal: None,
            first_ordinal: None,
            cut: None,
            history_version: None,
            head: None,
            last_run_state: None,
            last_run_error: None,
            last_run_rich_ui: None,
            updated_at: None,
            input: None,
            model: None,
            usage: None,
            sender: None,
            incarnation: None,
            retention_policy: None,
            retention_state: None,
            released: None,
            reasoning_pruned: None,
        }
    }

    pub fn with_last_ordinal(mut self, last_ordinal: u64) -> Self {
        self.last_ordinal = Some(last_ordinal);
        self
    }

    pub fn with_head(mut self, head: Map<String, Value>) -> Self {
        self.head = Some(head);
        self
    }

    pub fn with_cut(mut self, cut: HeadCut) -> Self {
        self.cut = Some(cut);
        self
    }

    pub fn with_history_version(mut self, history_version: u64) -> Self {
        self.history_version = Some(history_version);
        self
    }

    pub fn with_last_run_state(mut self, last_run_state: LastRunState) -> Self {
        self.last_run_state = Some(last_run_state);
        self
    }

    pub fn with_last_run_error(mut self, last_run_error: LastRunError) -> Self {
        self.last_run_error = Some(last_run_error);
        self
    }

    pub fn last_run_error(&self) -> Option<&LastRunError> {
        self.last_run_error.as_ref()
    }

    /// Name the rich UI stamp of the run `last_run_state` names.
    pub fn with_last_run_rich_ui(mut self, rich_ui: crate::rich_ui::RichUiStamp) -> Self {
        self.last_run_rich_ui = Some(rich_ui);
        self
    }

    pub fn with_input(mut self, input: HeadInput) -> Self {
        self.input = Some(input);
        self
    }

    pub fn with_model(mut self, model: HeadModel) -> Self {
        self.model = Some(model);
        self
    }

    pub fn with_usage(mut self, usage: crate::usage::SessionUsage) -> Self {
        self.usage = Some(usage);
        self
    }

    pub fn with_sender(mut self, sender: HeadSender) -> Self {
        self.sender = Some(sender);
        self
    }

    /// Interpret the caller's grant without mistaking an older runner's
    /// omitted field for a known lack of permission.
    pub fn sender_grant(&self, describe: &crate::describe::RoleDescribe) -> SenderGrant {
        match &self.sender {
            Some(sender) => SenderGrant::Granted(sender.clone()),
            None if describe.declares(crate::capabilities::HEAD_SENDER) => SenderGrant::NoGrant,
            None => SenderGrant::Unknown,
        }
    }

    pub fn with_incarnation(mut self, incarnation: impl Into<String>) -> Self {
        self.incarnation = Some(incarnation.into());
        self
    }

    /// Whether a reported log identity fits the opaque 1–128-byte wire bound.
    pub fn incarnation_valid(&self) -> bool {
        self.incarnation
            .as_ref()
            .is_none_or(|value| (1..=128).contains(&value.len()))
    }

    /// Whether the answer is consistent with its lineage. Without a
    /// `lineage_id`, none of `last_ordinal`, `first_ordinal`, `cut`,
    /// `history_version`, `head`, `last_run_state` or `updated_at` may be
    /// present, because a session with any of them has a lineage. With one,
    /// `updated_at`, `first_ordinal` and `history_version` must be present, `first_ordinal` is at most one above
    /// `last_ordinal` when that is present, and a lineage without `cut` has
    /// `first_ordinal` 0; the other members may be absent (an empty
    /// transcript, no events, no run yet). `last_run_rich_ui` is present only
    /// beside `last_run_state`, whose run it describes. A consumer
    /// treats an inconsistent answer as malformed; it is not a refusal.
    pub fn lineage_consistent(&self) -> bool {
        match self.lineage_id {
            None => {
                self.last_ordinal.is_none()
                    && self.first_ordinal.is_none()
                    && self.cut.is_none()
                    && self.history_version.is_none()
                    && self.head.is_none()
                    && self.last_run_state.is_none()
                    && self.last_run_rich_ui.is_none()
                    && self.updated_at.is_none()
                    && self.reasoning_pruned.is_none()
            }
            Some(_) => {
                let bounded = match (self.first_ordinal, self.last_ordinal) {
                    (Some(first), Some(last)) => first <= last.saturating_add(1),
                    (first, None) => first.is_some(),
                    (None, Some(_)) => false,
                };
                let cut_named = self.cut.is_some() || self.first_ordinal == Some(0);
                // The last run's stamp describes the run `last_run_state`
                // names, so it never stands without one.
                let stamp_has_run =
                    self.last_run_rich_ui.is_none() || self.last_run_state.is_some();
                self.updated_at.is_some()
                    && self.history_version.is_some()
                    && bounded
                    && cut_named
                    && stamp_has_run
            }
        }
    }
}

/// The cut that started a lineage, on `session.head`: the message the cut
/// folds last, the one at its `through_ordinal`, described by identity and
/// never by content. `digest` is that message's content digest
/// ([`crate::content_digest::content_digest`]), the digest a cut lineage's
/// `descends_from` names. A cut made before the runner numbered anything
/// (in a host's history before adoption) names the host's message: the last
/// one the host's compaction folded away, never one the host keeps visible
/// after compacting, such as its compaction marker or summary.
#[derive(Clone, Debug, PartialEq, Eq, Deserialize, Serialize)]
#[non_exhaustive]
pub struct HeadCut {
    pub mid: String,
    pub digest: String,
    /// When the message was created, in milliseconds since the Unix epoch.
    pub written_at: u64,
}

impl HeadCut {
    pub fn new(mid: impl Into<String>, digest: impl Into<String>, written_at: u64) -> Self {
        Self {
            mid: mid.into(),
            digest: digest.into(),
            written_at,
        }
    }
}

/// The state of a session's most recent run. Describes that run, never the
/// transcript as a whole.
#[derive(Clone, Debug, PartialEq, Eq, Deserialize, Serialize)]
pub struct LastRunState {
    pub run_id: String,
    /// A run state ([`crate::run::states`]); any other value decodes as a
    /// plain string.
    pub state: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub reason: Option<String>,
}

/// A finite, nonnegative retry delay in seconds, encoded as a JSON number.
/// Fractions are retained exactly; absence is represented by `None`, not zero.
#[derive(Clone, Copy, Debug, PartialEq, Deserialize, Serialize)]
#[serde(try_from = "f64", into = "f64")]
pub struct RetryAfterSecs(f64);

// Every stored value is finite, and f64 equality on finite values is an
// equivalence relation. The private field keeps NaN and infinities out.
impl Eq for RetryAfterSecs {}

impl RetryAfterSecs {
    pub fn new(value: f64) -> Option<Self> {
        (value.is_finite() && value >= 0.0).then_some(Self(value))
    }

    pub fn get(&self) -> f64 {
        self.0
    }
}

impl TryFrom<f64> for RetryAfterSecs {
    type Error = &'static str;

    fn try_from(value: f64) -> Result<Self, Self::Error> {
        Self::new(value).ok_or("retry_after_secs must be finite and at least 0")
    }
}

impl From<RetryAfterSecs> for f64 {
    fn from(value: RetryAfterSecs) -> Self {
        value.get()
    }
}

/// Retry metadata for the same run named by [`HeadMeta::last_run_state`],
/// present only when its state is `"error"`. Paused (for example
/// `account_changed`) and interrupted runs carry none because `reason`
/// already explains why they stopped. Absence on an error means not reported
/// (including by older runners), never permanent; no describe group is needed.
///
/// Values equal that run's [`crate::run::RunError`] as served by `run.status`,
/// never a separate derivation. A runner should write both from the same
/// terminal record. A negative or non-finite retry delay is omitted here.
///
/// The head is served to client apps and intentionally omits `message`,
/// `provider_code`, `status`, `account_ref` and provider diagnostics: retry
/// decisions need only class and timing. A reader wanting text calls
/// `run.status` once.
#[derive(Clone, Debug, PartialEq, Eq, Deserialize, Serialize)]
#[non_exhaustive]
pub struct LastRunError {
    /// Open string, retaining unknown values. Examples today are `transient`,
    /// `permanent`, `auth_required`, `context_overflow` and
    /// `provider_unavailable`; older records may say `auth`.
    pub class: String,
    /// Fractional seconds, finite and at least 0. Absent when the provider
    /// gave none, never zero as a stand-in for missing information.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub retry_after_secs: Option<RetryAfterSecs>,
    /// Provider reset time in milliseconds since the Unix epoch, if reported.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub resets_at_ms: Option<u64>,
}

impl LastRunError {
    pub fn new(class: impl Into<String>) -> Self {
        Self {
            class: class.into(),
            retry_after_secs: None,
            resets_at_ms: None,
        }
    }

    pub fn with_retry_after_secs(mut self, retry_after_secs: RetryAfterSecs) -> Self {
        self.retry_after_secs = Some(retry_after_secs);
        self
    }

    pub fn retry_after_secs(&self) -> Option<f64> {
        self.retry_after_secs.map(|value| value.get())
    }

    pub fn with_resets_at_ms(mut self, resets_at_ms: u64) -> Self {
        self.resets_at_ms = Some(resets_at_ms);
        self
    }

    pub fn resets_at_ms(&self) -> Option<u64> {
        self.resets_at_ms
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{errors, vectors};

    #[test]
    fn head_incarnation_is_optional_and_bounded_in_bytes() {
        let wire = vectors::load("head-incarnation.json");
        let head: HeadMeta = vectors::round_trip("incarnation", &wire);
        assert!(head.incarnation_valid());
        assert!(head.lineage_consistent());
        assert!(HeadMeta::no_lineage().incarnation_valid());
        for (value, valid) in [
            ("".into(), false),
            ("x".repeat(128), true),
            ("x".repeat(129), false),
            ("é".repeat(64), true),
            ("é".repeat(65), false),
        ] {
            assert_eq!(
                HeadMeta::no_lineage()
                    .with_incarnation(value)
                    .incarnation_valid(),
                valid
            );
        }
    }

    #[test]
    fn absent_sender_without_head_sender_group_is_unknown() {
        let describe = crate::describe::RoleDescribe::default();
        assert_eq!(
            HeadMeta::no_lineage().sender_grant(&describe),
            SenderGrant::Unknown
        );
    }

    #[test]
    fn absent_sender_with_head_sender_group_is_no_grant() {
        let describe = crate::describe::RoleDescribe::default()
            .with_capabilities(vec![crate::capabilities::HEAD_SENDER.into()]);
        assert_eq!(
            HeadMeta::no_lineage().sender_grant(&describe),
            SenderGrant::NoGrant
        );
    }

    #[test]
    fn present_sender_is_granted_with_or_without_head_sender_group() {
        let sender = HeadSender::for_owner(vec!["idle_start".into()]);
        let head = HeadMeta::no_lineage().with_sender(sender.clone());
        for capabilities in [vec![], vec![crate::capabilities::HEAD_SENDER.into()]] {
            let describe = crate::describe::RoleDescribe::default().with_capabilities(capabilities);
            assert_eq!(
                head.sender_grant(&describe),
                SenderGrant::Granted(sender.clone())
            );
        }
    }

    #[test]
    fn head_sender_vectors_round_trip() {
        let vector: Value =
            serde_json::from_str(crate::vectors::file("head-sender.json").unwrap()).unwrap();
        for case in vector["answers"].as_array().unwrap() {
            let head: HeadMeta = serde_json::from_value(case["answer"].clone()).unwrap();
            assert!(head.lineage_consistent());
            assert_eq!(
                serde_json::to_value(head).unwrap(),
                case["answer"],
                "{}",
                case["name"]
            );
        }
        assert_eq!(
            serde_json::to_value(HeadSender::new(
                vec!["tool_result".into(), "turn_system".into(), "steer".into()],
                false
            ))
            .unwrap(),
            vector["answers"][0]["answer"]["sender"]
        );
        assert_eq!(
            serde_json::to_value(HeadSender::for_owner(vec![
                "tool_result".into(),
                "turn_system".into(),
                "steer".into(),
                "idle_start".into()
            ]))
            .unwrap(),
            vector["answers"][3]["answer"]["sender"]
        );
    }
    #[test]
    fn request_vectors_decode_round_trip_and_select_their_mode() {
        let file = vectors::load("read-requests.json");
        for case in vectors::cases(&file, "accepted") {
            let name = case["name"].as_str().unwrap();
            let request: ReadRequest = vectors::round_trip(name, &case["request"]);
            let mode = match request.mode() {
                Ok(ReadMode::Tail) => "tail",
                Ok(ReadMode::Range { .. }) => "range",
                Ok(ReadMode::After { .. }) => "after",
                Err(field) => panic!("{name}: refused naming {field}"),
            };
            assert_eq!(mode, case["mode"].as_str().unwrap(), "{name}");
        }
        for case in vectors::cases(&file, "refused") {
            let name = case["name"].as_str().unwrap();
            let field = case["field"].as_str().unwrap();
            match serde_json::from_value::<ReadRequest>(case["request"].clone()) {
                Err(error) => assert!(error.to_string().contains(field), "{name}: {error}"),
                Ok(request) => assert_eq!(request.mode(), Err(field), "{name}"),
            }
            assert_eq!(case["refusal"]["code"], errors::INVALID_PARAMS, "{name}");
            assert_eq!(
                errors::refused_field(errors::INVALID_PARAMS, Some(&case["refusal"]["detail"])),
                Some(field),
                "{name}"
            );
        }
    }

    #[test]
    fn page_vectors_decode_and_round_trip() {
        let file = vectors::load("read-pages.json");
        for case in vectors::cases(&file, "pages") {
            let name = case["name"].as_str().unwrap();
            let page: ReadPage = vectors::round_trip(name, &case["page"]);
            if case["stopped_early"].as_bool().unwrap() {
                assert!(page.next_from_ordinal.is_some(), "{name}");
            }
            assert!(page.lineage_consistent(), "{name}");
            if let Some(cap) = case.get("max_bytes").and_then(Value::as_u64) {
                // An oversize message comes alone, whole, over the cap.
                assert_eq!(page.messages.len(), 1, "{name}");
                let bytes = serde_json::to_vec(&page.messages[0]).unwrap().len() as u64;
                assert!(bytes > cap, "{name}: {bytes} bytes is not over {cap}");
            }
        }
        for case in vectors::cases(&file, "inconsistent") {
            let name = case["name"].as_str().unwrap();
            let page: ReadPage = serde_json::from_value(case["page"].clone())
                .unwrap_or_else(|e| panic!("{name}: {e}"));
            assert!(!page.lineage_consistent(), "{name}");
        }
        for case in vectors::cases(&file, "tolerated") {
            let name = case["name"].as_str().unwrap();
            serde_json::from_value::<ReadPage>(case["page"].clone())
                .unwrap_or_else(|e| panic!("{name}: {e}"));
        }
        for case in vectors::cases(&file, "undecodable") {
            let name = case["name"].as_str().unwrap();
            assert!(
                serde_json::from_value::<ReadPage>(case["page"].clone()).is_err(),
                "{name}"
            );
        }
        for case in vectors::cases(&file, "refusals") {
            let name = case["name"].as_str().unwrap();
            let code = case["refusal"]["code"].as_str().unwrap();
            assert!(
                [errors::LINEAGE_CHANGED, errors::UNKNOWN_MID].contains(&code),
                "{name}"
            );
            assert!(!errors::is_retryable(code), "{name}");
            serde_json::from_value::<ReadRequest>(case["request"].clone())
                .unwrap_or_else(|e| panic!("{name}: {e}"));
        }
    }

    fn model_problem_name(problem: &ModelPageProblem) -> &'static str {
        match problem {
            ModelPageProblem::NoLineage => "no_lineage",
            ModelPageProblem::PartialCompactionState => "partial_compaction_state",
            ModelPageProblem::ReplacementWithoutCompactionState => {
                "replacement_without_compaction_state"
            }
            ModelPageProblem::InvertedRange { .. } => "inverted_range",
            ModelPageProblem::OutOfOrder { .. } => "out_of_order",
            ModelPageProblem::InsertionAfterEntry { .. } => "insertion_after_entry",
            ModelPageProblem::NonTailInsertion { .. } => "non_tail_insertion",
            ModelPageProblem::StartsBeforePage { .. } => "starts_before_page",
            ModelPageProblem::NextInsideEntry { .. } => "next_inside_entry",
        }
    }

    #[test]
    fn model_page_vectors_decode_round_trip_and_follow_the_paging_rules() {
        let file = vectors::load("read-pages.json");
        let mut kinds = Vec::new();
        for case in vectors::cases(&file, "model_pages") {
            let name = case["name"].as_str().unwrap();
            let page: ModelPage = vectors::round_trip(name, &case["page"]);
            page.check(case["from_ordinal"].as_u64())
                .unwrap_or_else(|problem| panic!("{name}: {problem:?}"));
            // The case's `compacted` flag says whether a compaction had
            // applied: if so the page carries both `compaction_id` and
            // `version`, and if not it carries neither.
            let compacted = case["compacted"].as_bool().unwrap();
            assert_eq!(page.compaction_id.is_some(), compacted, "{name}");
            assert_eq!(page.version.is_some(), compacted, "{name}");
            for entry in &page.messages {
                let kind = serde_json::to_value(&entry.source).unwrap()["kind"].clone();
                kinds.push(kind);
            }
        }
        for kind in ["message", "replacement"] {
            assert!(kinds.contains(&Value::from(kind)), "no {kind} entry");
        }
        for case in vectors::cases(&file, "model_pages_rejected") {
            if [
                "inverted_range",
                "insertion_after_entry",
                "non_tail_insertion",
            ]
            .contains(&case["problem"].as_str().unwrap())
            {
                continue;
            }
            let name = case["name"].as_str().unwrap();
            let page: ModelPage = serde_json::from_value(case["page"].clone())
                .unwrap_or_else(|e| panic!("{name}: {e}"));
            let problem = page.check(case["from_ordinal"].as_u64()).expect_err(name);
            assert_eq!(
                model_problem_name(&problem),
                case["problem"].as_str().unwrap(),
                "{name}"
            );
        }
        for case in vectors::cases(&file, "model_pages_undecodable") {
            let name = case["name"].as_str().unwrap();
            assert!(
                serde_json::from_value::<ModelPage>(case["page"].clone()).is_err(),
                "{name}"
            );
        }
    }

    fn rejected_model_vector(name: &str, expected: ModelPageProblem) {
        let file = vectors::load("read-pages.json");
        let case = vectors::cases(&file, "model_pages_rejected")
            .iter()
            .find(|case| case["name"] == name)
            .unwrap();
        let page: ModelPage = serde_json::from_value(case["page"].clone()).unwrap();
        assert_eq!(
            page.check(case["from_ordinal"].as_u64()),
            Err(expected),
            "{name}"
        );
    }

    #[test]
    fn model_page_refuses_ending_on_a_non_tail_insertion() {
        rejected_model_vector(
            "non-tail page ends between insertion and message five",
            ModelPageProblem::NonTailInsertion { from_ordinal: 5 },
        );
        rejected_model_vector(
            "next cursor repeats an insertion anchor",
            ModelPageProblem::NonTailInsertion { from_ordinal: 2 },
        );
    }

    #[test]
    fn model_page_keeps_insertions_and_anchored_entries_together_over_caps() {
        let file = vectors::load("read-pages.json");
        for (name, anchor, next) in [
            (
                "count cap after insertion keeps message five on the page",
                5,
                6,
            ),
            (
                "count cap after insertion keeps replacement on the page",
                3,
                7,
            ),
            (
                "several insertions travel together in order over the byte cap",
                5,
                6,
            ),
        ] {
            let case = vectors::cases(&file, "model_pages")
                .iter()
                .find(|case| case["name"] == name)
                .unwrap();
            let page: ModelPage = vectors::round_trip(name, &case["page"]);
            assert_eq!(page.check(Some(anchor)), Ok(()), "{name}");
            assert_eq!(page.next_from_ordinal, Some(next), "{name}");
            let (anchored_entry, insertions) = page.messages.split_last().unwrap();
            assert!(!anchored_entry.source.is_insertion(), "{name}");
            assert_eq!(anchored_entry.source.from_ordinal(), anchor, "{name}");
            assert!(!insertions.is_empty(), "{name}");
            assert!(
                insertions
                    .iter()
                    .all(|entry| entry.source.is_insertion()
                        && entry.source.from_ordinal() == anchor),
                "{name}"
            );
            if let Some(limit) = case["limit"].as_u64() {
                assert!(page.messages.len() as u64 > limit, "{name}");
            }
            if let Some(max_bytes) = case["max_bytes"].as_u64() {
                assert!(
                    serde_json::to_vec(&page.messages).unwrap().len() as u64 > max_bytes,
                    "{name}"
                );
                assert_eq!(insertions[0].message["text"], "first");
                assert_eq!(insertions[1].message["text"], "second");
            }
        }
    }

    #[test]
    fn model_page_refuses_inverted_half_open_range() {
        rejected_model_vector(
            "a replacement whose range is inverted",
            ModelPageProblem::InvertedRange {
                from_ordinal: 9,
                to_ordinal: 4,
            },
        );
    }

    #[test]
    fn model_page_refuses_insertion_after_its_message() {
        rejected_model_vector(
            "insertion after message at the same anchor",
            ModelPageProblem::InsertionAfterEntry { from_ordinal: 2 },
        );
    }

    #[test]
    fn model_page_accepts_insertion_before_its_replacement() {
        let file = vectors::load("read-pages.json");
        let case = vectors::cases(&file, "model_pages")
            .iter()
            .find(|case| case["name"] == "insertion before replacement at the same anchor")
            .unwrap();
        let page: ModelPage = vectors::round_trip("insertion before replacement", &case["page"]);
        assert_eq!(page.check(Some(2)), Ok(()));
        assert!(page.messages[0].source.is_insertion());
        assert!(!page.messages[1].source.is_insertion());
        assert_eq!(page.messages[1].source.last_covered_ordinal(), Some(4));
        assert_eq!(page.next_from_ordinal, Some(5));
    }

    #[test]
    fn model_page_refuses_insertion_after_its_replacement() {
        rejected_model_vector(
            "insertion after replacement at the same anchor",
            ModelPageProblem::InsertionAfterEntry { from_ordinal: 2 },
        );
    }

    #[test]
    fn model_view_head_and_tail_insertions_are_on_boundary_pages() {
        let file = vectors::load("read-pages.json");
        let pages = vectors::cases(&file, "model_pages");
        let head = pages
            .iter()
            .find(|case| case["name"] == "Setup head insertion before message zero")
            .unwrap();
        let tail = pages
            .iter()
            .find(|case| case["name"] == "tail insertion on the last page")
            .unwrap();
        let head: ModelPage = vectors::round_trip("head", &head["page"]);
        let tail: ModelPage = vectors::round_trip("tail", &tail["page"]);
        assert_eq!(head.check(Some(0)), Ok(()));
        assert_eq!(tail.check(head.next_from_ordinal), Ok(()));
        assert!(head.messages[0].source.is_insertion());
        assert_eq!(head.messages[0].source.from_ordinal(), 0);
        assert_eq!(head.messages[0].source.last_covered_ordinal(), None);
        assert!(tail.messages.last().unwrap().source.is_insertion());
        assert_eq!(tail.next_from_ordinal, None);
        assert_eq!(tail.messages.last().unwrap().source.from_ordinal(), 2);
    }

    #[test]
    fn model_view_replacement_comes_once_on_the_page_holding_its_from_ordinal() {
        let file = vectors::load("read-pages.json");
        let once = &file["replacement_once"];
        let replacements = |page: &ModelPage| -> Vec<EntrySource> {
            page.messages
                .iter()
                .filter(|entry| matches!(entry.source, EntrySource::Replacement { .. }))
                .map(|entry| entry.source.clone())
                .collect()
        };

        // The series reads the model view from ordinal 0, each read starting
        // at the previous page's `next_from_ordinal`; together its pages
        // hold the view's one replacement exactly once.
        let mut seen = Vec::new();
        let mut expected_from = None;
        for case in vectors::cases(once, "series") {
            let name = case["name"].as_str().unwrap();
            let from_ordinal = case["from_ordinal"].as_u64().unwrap();
            if let Some(expected) = expected_from {
                assert_eq!(
                    from_ordinal, expected,
                    "{name}: not where the last page stopped"
                );
            }
            let page: ModelPage = vectors::round_trip(name, &case["page"]);
            page.check(Some(from_ordinal))
                .unwrap_or_else(|problem| panic!("{name}: {problem:?}"));
            for source in replacements(&page) {
                assert!(source.from_ordinal() >= from_ordinal, "{name}");
                assert!(!seen.contains(&source), "{name}: {source:?} repeated");
                seen.push(source);
            }
            expected_from = page.next_from_ordinal;
        }
        assert_eq!(seen.len(), 1, "the series holds one replacement");

        // A read starting inside that replacement's ordinal range does not
        // return the replacement again.
        for case in vectors::cases(once, "intersecting") {
            let name = case["name"].as_str().unwrap();
            let from_ordinal = case["from_ordinal"].as_u64().unwrap();
            let page: ModelPage = vectors::round_trip(name, &case["page"]);
            page.check(Some(from_ordinal))
                .unwrap_or_else(|problem| panic!("{name}: {problem:?}"));
            let replacement = &seen[0];
            assert!(
                replacement.from_ordinal() < from_ordinal
                    && Some(from_ordinal) <= replacement.last_covered_ordinal(),
                "{name}: the read does not intersect the replacement"
            );
            assert!(replacements(&page).is_empty(), "{name}");
        }

        // A page read from inside the range that does return the
        // replacement again is malformed.
        for case in vectors::cases(once, "repeated") {
            let name = case["name"].as_str().unwrap();
            let page: ModelPage = serde_json::from_value(case["page"].clone()).unwrap();
            let problem = page.check(case["from_ordinal"].as_u64()).expect_err(name);
            assert_eq!(model_problem_name(&problem), "starts_before_page", "{name}");
        }
    }

    #[test]
    fn head_vectors_decode_and_round_trip() {
        let file = vectors::load("head.json");
        for case in vectors::cases(&file, "requests") {
            vectors::round_trip::<HeadRequest>(case["name"].as_str().unwrap(), &case["request"]);
        }
        for case in vectors::cases(&file, "refused_requests") {
            assert!(
                serde_json::from_value::<HeadRequest>(case["request"].clone()).is_err(),
                "{}",
                case["name"]
            );
        }
        for case in vectors::cases(&file, "answers") {
            let name = case["name"].as_str().unwrap();
            let head: HeadMeta = vectors::round_trip(name, &case["answer"]);
            assert!(case["answer"].get("messages").is_none(), "{name}");
            assert!(head.lineage_consistent(), "{name}");
        }
        assert!(HeadMeta::no_lineage().lineage_consistent());
    }

    #[test]
    fn last_run_error_round_trips_every_field() {
        let head = HeadMeta::new("error-lineage", 0, 1790000001500)
            .with_history_version(1)
            .with_last_run_state(LastRunState {
                run_id: "error-run".into(),
                state: "error".into(),
                reason: Some("provider_error".into()),
            })
            .with_last_run_error(
                LastRunError::new("transient")
                    .with_retry_after_secs(RetryAfterSecs::new(1.5).unwrap())
                    .with_resets_at_ms(1790000003000),
            );
        let encoded = serde_json::to_string(&head).unwrap();
        let decoded: HeadMeta = serde_json::from_str(&encoded).unwrap();
        assert_eq!(decoded, head);
        let error = decoded.last_run_error().unwrap();
        assert_eq!(error.class, "transient");
        assert_eq!(error.retry_after_secs(), Some(1.5));
        assert_eq!(error.resets_at_ms(), Some(1790000003000));
        let value = serde_json::to_value(error).unwrap();
        assert_eq!(value["retry_after_secs"], serde_json::json!(1.5));
        assert_eq!(value.as_object().unwrap().len(), 3);
    }

    #[test]
    fn old_head_without_last_run_error_is_byte_identical() {
        // Pin the pre-existing error head, which predates retry metadata.
        let file = vectors::load("head.json");
        let answer = &file["answers"][1]["answer"];
        assert_eq!(file["answers"][1]["name"], "last run ended in error");
        let head: HeadMeta = serde_json::from_value(answer.clone()).unwrap();
        assert!(head.last_run_error().is_none());
        let encoded = serde_json::to_value(head).unwrap();
        assert_eq!(
            serde_json::to_vec(&encoded).unwrap(),
            serde_json::to_vec(answer).unwrap()
        );
        assert_eq!(
            serde_json::to_string(&HeadMeta::no_lineage()).unwrap(),
            "{}"
        );
    }

    #[test]
    fn last_run_error_unknown_class_and_absent_timing_are_preserved() {
        let encoded = r#"{"class":"future_retry_class"}"#;
        let error: LastRunError = serde_json::from_str(encoded).unwrap();
        assert_eq!(error.class, "future_retry_class");
        assert_eq!(error.retry_after_secs(), None);
        assert_eq!(error.resets_at_ms(), None);
        assert_eq!(serde_json::to_string(&error).unwrap(), encoded);
    }

    #[test]
    fn retry_after_secs_rejects_invalid_numbers_and_head_remains_eq() {
        fn assert_eq_trait<T: Eq>() {}
        assert_eq_trait::<HeadMeta>();
        assert_eq_trait::<LastRunError>();
        for value in [f64::NAN, f64::INFINITY, f64::NEG_INFINITY, -1.0] {
            assert_eq!(RetryAfterSecs::new(value), None);
        }
        assert_eq!(RetryAfterSecs::new(0.0).unwrap().get(), 0.0);
        assert_eq!(RetryAfterSecs::new(1.5).unwrap().get(), 1.5);
        let negative =
            serde_json::from_str::<LastRunError>(r#"{"class":"transient","retry_after_secs":-1}"#)
                .unwrap_err();
        assert!(negative.to_string().contains("finite and at least 0"));
        assert!(serde_json::from_str::<LastRunError>(
            r#"{"class":"transient","retry_after_secs":1e400}"#,
        )
        .is_err());
    }

    #[test]
    fn last_run_error_vector_decodes_and_round_trips() {
        let file = vectors::load("head-last-run-error.json");
        for case in vectors::cases(&file, "answers") {
            let head: HeadMeta =
                vectors::round_trip(case["name"].as_str().unwrap(), &case["answer"]);
            assert!(head.lineage_consistent());
            assert_eq!(head.last_run_state.as_ref().unwrap().state, "error");
            assert_eq!(head.last_run_error().unwrap().retry_after_secs(), Some(1.5));
        }
    }

    #[test]
    fn head_answers_inconsistent_with_their_lineage_are_rejected() {
        let file = vectors::load("head.json");
        for case in vectors::cases(&file, "inconsistent") {
            let name = case["name"].as_str().unwrap();
            let head: HeadMeta = serde_json::from_value(case["answer"].clone())
                .unwrap_or_else(|e| panic!("{name}: {e}"));
            assert!(!head.lineage_consistent(), "{name}");
        }
    }

    #[test]
    fn builders_match_the_decoded_shapes() {
        let request = ReadRequest::after("m7", "lin-1")
            .with_limit(50)
            .with_max_bytes(65_536)
            .with_include_originals(true)
            .with_view(ReadView::Raw);
        assert_eq!(
            request.mode(),
            Ok(ReadMode::After {
                mid: "m7",
                lineage_id: "lin-1"
            })
        );
        let encoded = serde_json::to_value(&request).unwrap();
        assert_eq!(
            serde_json::from_value::<ReadRequest>(encoded).unwrap(),
            request
        );

        let message = ReadMessage::new(3, "m3", serde_json::json!({"role": "assistant"}))
            .with_run(RunAttribution {
                run_id: "r1".into(),
                episode: "e1".into(),
                is_final: true,
            })
            .with_tool_calls(vec![ToolCallAttribution::new("call_0")
                .with_call_key("lin-1:42")
                .with_dispatched_to("aft")
                .with_indeterminate(true)]);
        assert!(ReadPage::no_lineage().lineage_consistent());
        let page = ReadPage::new("lin-1", vec![message])
            .with_next_from_ordinal(4)
            .with_head(serde_json::json!({"seq": 9}).as_object().unwrap().clone());
        let encoded = serde_json::to_value(&page).unwrap();
        assert_eq!(serde_json::from_value::<ReadPage>(encoded).unwrap(), page);
        assert_eq!(
            ReadRequest::range(5).with_lineage_id("lin-1").mode(),
            Ok(ReadMode::Range { from_ordinal: 5 })
        );
        assert_eq!(
            ReadRequest::after("m7", "lin-1")
                .with_view(ReadView::Model)
                .mode(),
            Err(fields::VIEW)
        );

        let model = ModelPage::new(
            "lin-1",
            vec![
                ModelEntry::new(
                    EntrySource::Replacement {
                        compaction_id: "cmp-1".into(),
                        version: 2,
                        from_ordinal: 0,
                        to_ordinal: 10,
                    },
                    serde_json::json!({"role": "user"}),
                ),
                ModelEntry::new(
                    EntrySource::Message {
                        ordinal: 10,
                        mid: "m10".into(),
                    },
                    serde_json::json!({"role": "assistant"}),
                ),
            ],
        )
        .with_compaction("cmp-1", 2)
        .with_next_from_ordinal(11);
        assert_eq!(model.check(Some(0)), Ok(()));
        let encoded = serde_json::to_value(&model).unwrap();
        assert_eq!(serde_json::from_value::<ModelPage>(encoded).unwrap(), model);
        assert_eq!(ModelPage::no_lineage().check(None), Ok(()));
    }
}
