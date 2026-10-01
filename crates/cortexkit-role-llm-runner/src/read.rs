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

/// The `field` an `invalid_params` refusal names when a read combines
/// `after_mid` with `from_ordinal`, or names `after_mid` without
/// `lineage_id`.
pub mod fields {
    pub const FROM_ORDINAL: &str = "from_ordinal";
    pub const LINEAGE_ID: &str = "lineage_id";
    pub const VIEW: &str = "view";
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

    /// The read mode, or the field an `invalid_params` refusal names:
    /// `view` when `after_mid` is combined with the model view,
    /// `from_ordinal` when it is combined with `after_mid`, `lineage_id`
    /// when `after_mid` comes without it.
    pub fn mode(&self) -> Result<ReadMode<'_>, &'static str> {
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
    pub messages: Vec<ReadMessage>,
    /// The session's lineage, on every page of a session that has one.
    /// Absent means the session has no lineage yet: it was never written,
    /// and the page is empty.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub lineage_id: Option<String>,
    /// Present whenever the page stopped before the end of the transcript,
    /// by count or by bytes: the ordinal to read from next. A single message
    /// larger than the byte cap comes alone on its page, over the cap, with
    /// this set if more follow; a message is never truncated.
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

    pub fn with_head(mut self, head: Map<String, Value>) -> Self {
        self.head = Some(head);
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
    /// Entries in non-decreasing anchor order, insertions first at a tie.
    pub messages: Vec<ModelEntry>,
    /// The session's lineage, as on a raw page ([`ReadPage::lineage_id`]):
    /// absent only for a session never written, whose page is empty.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub lineage_id: Option<String>,
    /// The transcript ordinal to read from next, when the page stopped
    /// early. Past the anchored entry: at least A + 1 for a message at A,
    /// or the replacement's exclusive end. Insertions share that entry's
    /// page, even over the caps; tail insertions have no next cursor.
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
    /// A page of a session whose lineage is `lineage_id`, before any
    /// compaction applied.
    pub fn new(lineage_id: impl Into<String>, messages: Vec<ModelEntry>) -> Self {
        Self {
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
    /// The message's position in the transcript. Never reused or
    /// renumbered within a lineage.
    pub ordinal: u64,
    /// The message's id, opaque to consumers. Never reused within a
    /// lineage.
    pub mid: String,
    /// The message itself, carrying its final values: what later steps
    /// render and what executed. Its schema is the runner's.
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
}

impl ReadMessage {
    pub fn new(ordinal: u64, mid: impl Into<String>, message: Value) -> Self {
        Self {
            ordinal,
            mid: mid.into(),
            message,
            run: None,
            originals: None,
            tool_calls: Vec::new(),
        }
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
/// `lineage_id`, and then none of the other members either
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
    /// The same opaque position a page's `head` carries.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub head: Option<Map<String, Value>>,
    /// The session's most recent run. Absent before the first run.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub last_run_state: Option<LastRunState>,
    /// When the transcript last changed, in milliseconds since the Unix
    /// epoch. Present whenever `lineage_id` is.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub updated_at: Option<u64>,
}

impl HeadMeta {
    /// The head of a session whose lineage is `lineage_id`.
    pub fn new(lineage_id: impl Into<String>, updated_at: u64) -> Self {
        Self {
            lineage_id: Some(lineage_id.into()),
            last_ordinal: None,
            head: None,
            last_run_state: None,
            updated_at: Some(updated_at),
        }
    }

    /// The head of a session that was never written: every member absent.
    pub fn no_lineage() -> Self {
        Self {
            lineage_id: None,
            last_ordinal: None,
            head: None,
            last_run_state: None,
            updated_at: None,
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

    pub fn with_last_run_state(mut self, last_run_state: LastRunState) -> Self {
        self.last_run_state = Some(last_run_state);
        self
    }

    /// Whether the answer is consistent with its lineage. Without a
    /// `lineage_id`, none of `last_ordinal`, `head`, `last_run_state` or
    /// `updated_at` may be present, because a session with any of them has a
    /// lineage. With one, `updated_at` must be present; the other three may
    /// be absent (an empty transcript, no events, no run yet). A consumer
    /// treats an inconsistent answer as malformed; it is not a refusal.
    pub fn lineage_consistent(&self) -> bool {
        match self.lineage_id {
            None => {
                self.last_ordinal.is_none()
                    && self.head.is_none()
                    && self.last_run_state.is_none()
                    && self.updated_at.is_none()
            }
            Some(_) => self.updated_at.is_some(),
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{errors, vectors};

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
