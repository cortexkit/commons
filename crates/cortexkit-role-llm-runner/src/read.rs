//! `session.read` and `session.head`: the transcript reads.
//!
//! Three read modes share one request ([`ReadRequest::mode`]):
//!
//! - **tail**: no cursor. The newest page. This is the contract, not a
//!   default a runner may change.
//! - **range**: `from_ordinal`. Messages from that ordinal forward.
//! - **after**: `after_mid` with its `history_version`. Messages strictly after
//!   that message. Refused by name as `history_changed` or `unknown_mid`,
//!   never answered from the tail instead.
//!
//! Reads serve the whole active path from ordinal 1, including entries below
//! the latest host marker, read from the host's own store. `from_ordinal: 0`
//! and `from_ordinal: 1` both start at the first entry. A marker hides older
//! entries from the model, never from reads.
//!
//! A page stops at `limit` messages or at the byte cap, whichever comes
//! first, and carries `next_from_ordinal` whenever it stops before the end
//! of the transcript. Every page carries `history_version` and `incarnation`.
//!
//! A page is an ordinal range and nothing more: it is not aligned to turns,
//! runs or tool-call boundaries, so a tool call and its result may arrive on
//! different pages.
//!
use serde::{Deserialize, Deserializer, Serialize};
use serde_json::{Map, Value};

pub mod descriptors;
pub use descriptors::MediaMode;

/// The `field` an `invalid_params` refusal names when a read combines
/// fields that exclude each other, or names `after_mid` without
/// `history_version`.
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
    pub const HISTORY_VERSION: &str = "history_version";
    pub const VIEW: &str = "view";
    /// The `session.read` opt-in to each assistant entry's rich UI stamp,
    /// served only by a runner declaring `rich_ui_stamp`.
    pub const INCLUDE_RICH_UI: &str = "include_rich_ui";
    pub const INCLUDE_DISPLAY_FACTS: &str = "include_display_facts";
}

/// Which list of messages a read pages through.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Deserialize, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum ReadView {
    /// The transcript as written, with each message's final values. The
    /// default.
    Raw,
}

fn deserialize_view<'de, D>(deserializer: D) -> Result<Option<ReadView>, D::Error>
where
    D: Deserializer<'de>,
{
    crate::strict::optional(deserializer, fields::VIEW, "\"raw\"")
}

/// The `session.read` request.
///
/// Strict on unknown fields: absence is meaningful here (no cursor means
/// "the newest page"), so a misspelled cursor must be refused rather than
/// read as a tail request. A runner refuses a field it does not serve with
/// `invalid_params` naming it.
///
/// For a staged migration from 0.7, decoding accepts a legacy `lineage_id`
/// (absent, null or a string) and discards it; serializing omits it. It does
/// not validate continuity or replace `history_version`: an `after_mid`
/// request without that version is still refused `invalid_params` naming
/// `history_version`, so the caller fetches a fresh head and page rather than
/// reusing old ordinals. Scheduled for removal in 0.9.
///
/// Non-exhaustive so a later optional member is additive: build one with
/// [`ReadRequest::tail`], [`ReadRequest::range`] or [`ReadRequest::after`]
/// and the `with_*` setters, or decode one.
#[derive(Clone, Debug, Default, PartialEq, Eq, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
#[non_exhaustive]
pub struct ReadRequest {
    #[serde(default, rename = "lineage_id", skip_serializing)]
    _lineage_id: crate::wire::DiscardedOptionalString,
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
    /// Read strictly after this message id. Requires `history_version`. A `mid`
    /// is opaque to consumers: only ever a value a page returned.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub after_mid: Option<String>,
    /// The history version the cursor came from. A mismatch is refused
    /// `history_changed`; a cut or append does not invalidate the cursor.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub history_version: Option<u64>,
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
    /// Add client ids, completion times, reasoning summaries and per-call times
    /// beside message bodies. Only served by a runner declaring
    /// `read_display_facts`; absent means false.
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub include_display_facts: bool,
}

/// The mode a well-formed [`ReadRequest`] selects: the newest page, a range
/// from an ordinal, or the messages after a message id at a history version.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ReadMode<'a> {
    Tail,
    Range { from_ordinal: u64 },
    After { mid: &'a str, history_version: u64 },
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

    /// Messages strictly after `mid` at `history_version`.
    pub fn after(mid: impl Into<String>, history_version: u64) -> Self {
        Self {
            after_mid: Some(mid.into()),
            history_version: Some(history_version),
            ..Self::default()
        }
    }

    /// Set the expected history version. The runner refuses a mismatch in
    /// any read mode.
    pub fn with_history_version(mut self, history_version: u64) -> Self {
        self.history_version = Some(history_version);
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
    /// for display facts without the declared group.
    pub fn check_display_facts<S: AsRef<str>>(&self, groups: &[S]) -> Result<(), &'static str> {
        if self.include_display_facts
            && !groups
                .iter()
                .any(|group| group.as_ref() == crate::capabilities::READ_DISPLAY_FACTS)
        {
            return Err(fields::INCLUDE_DISPLAY_FACTS);
        }
        Ok(())
    }

    pub fn with_include_display_facts(mut self, include_display_facts: bool) -> Self {
        self.include_display_facts = include_display_facts;
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

    /// Select tail, range or after-message mode. Return `from_ordinal` when
    /// combined with `after_mid`, or `history_version` when `after_mid` omits
    /// it. Normalize range ordinal 0 to 1. View validity is checked while decoding.
    pub fn mode(&self) -> Result<ReadMode<'_>, &'static str> {
        match (&self.after_mid, self.from_ordinal, self.history_version) {
            (Some(_), Some(_), _) => Err(fields::FROM_ORDINAL),
            (Some(_), None, None) => Err(fields::HISTORY_VERSION),
            (Some(mid), None, Some(history_version)) => Ok(ReadMode::After {
                mid,
                history_version,
            }),
            (None, Some(from_ordinal), _) => Ok(ReadMode::Range {
                from_ordinal: from_ordinal.max(1),
            }),
            (None, None, _) => Ok(ReadMode::Tail),
        }
    }
}

/// A `session.read` page. Decoded leniently: unknown fields are ignored.
///
/// Non-exhaustive so later optional members are additive: use
/// [`ReadPage::new`] and the `with_*` setters, or
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
    /// The rewrite version of this snapshot, even on an empty page.
    pub history_version: u64,
    /// Opaque identity of the runner's session record. Readers cache pages
    /// under the pair `(history_version, incarnation)`.
    pub incarnation: String,
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
    pub fn new(
        history_version: u64,
        incarnation: impl Into<String>,
        messages: Vec<ReadMessage>,
    ) -> Self {
        Self {
            pruned_reasoning: Vec::new(),
            reasoning_pruned: None,
            messages,
            history_version,
            incarnation: incarnation.into(),
            next_from_ordinal: None,
            head: None,
        }
    }

    pub fn empty(history_version: u64, incarnation: impl Into<String>) -> Self {
        Self::new(history_version, incarnation, Vec::new())
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

    /// Check that incarnation is 1–128 UTF-8 bytes and every served entry
    /// ordinal is at least 1.
    pub fn snapshot_consistent(&self) -> bool {
        (1..=128).contains(&self.incarnation.len())
            && self.messages.iter().all(|entry| entry.ordinal >= 1)
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
    /// the runner's keys in stored order; readers never derive those keys.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub part_digests: Option<Vec<descriptors::PartDigest>>,
    /// Stored-order part keys when `archive_prunes_reasoning` is declared
    /// and `part_digests` is absent. Readers use these keys, never derive them.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub part_keys: Option<Vec<String>>,
    /// The message's position in the transcript. Never reused or
    /// renumbered within a snapshot.
    pub ordinal: u64,
    /// The message's id, opaque to consumers. Never reused within a
    /// history_version.
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
    /// The sent user message's client id, exactly as on its display row.
    /// Present only with `include_display_facts`, never inside `message`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub client_message_id: Option<String>,
    /// When the host recorded this assistant message's finish, in Unix epoch
    /// milliseconds. Only with `include_display_facts`, where an assistant
    /// entry must carry it once the host recorded a finish time. Absent while
    /// the message is unfinished, when no finish was recorded, and on other
    /// roles. A later read may gain it without changing stored message bytes or
    /// content digests.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub completed_at: Option<u64>,
    /// Readable Responses summaries, in stored part order, only with
    /// `include_display_facts`. This envelope member adds no stored message
    /// parts and is excluded from message and part content-digest inputs.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub reasoning_summaries: Vec<ReasoningSummary>,
    /// One item per tool call, in call order, only with `include_display_facts`.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub tool_times: Vec<ToolTime>,
}

/// A readable summary of one stored Responses reasoning part, not an extra
/// message part. `part` is the zero-based index of the stored top-level
/// content part; with `part_keys` present it names the row at that index, not
/// that row's opaque key.
#[derive(Clone, Debug, PartialEq, Eq, Deserialize, Serialize)]
#[non_exhaustive]
pub struct ReasoningSummary {
    pub part: u32,
    pub text: String,
}

impl ReasoningSummary {
    pub fn new(part: u32, text: impl Into<String>) -> Self {
        Self {
            part,
            text: text.into(),
        }
    }
}

/// A read entry's per-call display facts, copied from stored call metadata.
/// Missing times are unknown, never inferred from another clock or call.
/// `call_key` and `schema_pin` appear only on a runner that also declares
/// `call_identity`, copied from the call's stored facts under §9.16's
/// presence rules.
#[derive(Clone, Debug, PartialEq, Eq, Deserialize, Serialize)]
#[non_exhaustive]
pub struct ToolTime {
    pub tool_call_id: String,
    /// Dispatch time in Unix epoch milliseconds, when stored.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub started_at: Option<u64>,
    /// Answer-recording time in Unix epoch milliseconds, when stored.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub finished_at: Option<u64>,
    /// Why the provider never answered, decoded as an open string (§4.7.1).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub unanswered: Option<String>,
    /// The call's stored `call_key` (§9.16), only on a runner declaring
    /// `call_identity`, and absent when the call has none stored.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub call_key: Option<String>,
    /// The `schema_pin` stored when the call was dispatched (§9.16), only on
    /// a runner declaring `call_identity`; a call denied before dispatch has
    /// none.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub schema_pin: Option<String>,
}

impl ToolTime {
    pub fn new(tool_call_id: impl Into<String>) -> Self {
        Self {
            tool_call_id: tool_call_id.into(),
            started_at: None,
            finished_at: None,
            unanswered: None,
            call_key: None,
            schema_pin: None,
        }
    }

    pub fn with_started_at(mut self, started_at: u64) -> Self {
        self.started_at = Some(started_at);
        self
    }

    pub fn with_finished_at(mut self, finished_at: u64) -> Self {
        self.finished_at = Some(finished_at);
        self
    }

    pub fn with_unanswered(mut self, unanswered: impl Into<String>) -> Self {
        self.unanswered = Some(unanswered.into());
        self
    }

    pub fn with_call_key(mut self, call_key: impl Into<String>) -> Self {
        self.call_key = Some(call_key.into());
        self
    }

    pub fn with_schema_pin(mut self, schema_pin: impl Into<String>) -> Self {
        self.schema_pin = Some(schema_pin.into());
        self
    }
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
    pub fn with_part_keys(mut self, keys: Vec<String>) -> Self {
        self.part_keys = Some(keys);
        self
    }

    /// The runner's served identities, preferring write-time digest rows.
    pub fn served_part_keys(&self) -> Option<Vec<&str>> {
        self.part_digests
            .as_ref()
            .map(|rows| rows.iter().map(|row| row.part.as_str()).collect())
            .or_else(|| {
                self.part_keys
                    .as_ref()
                    .map(|keys| keys.iter().map(String::as_str).collect())
            })
    }
    pub fn with_client_message_id(mut self, client_message_id: impl Into<String>) -> Self {
        self.client_message_id = Some(client_message_id.into());
        self
    }

    pub fn with_tool_times(mut self, tool_times: Vec<ToolTime>) -> Self {
        self.tool_times = tool_times;
        self
    }

    pub fn with_completed_at(mut self, completed_at: u64) -> Self {
        self.completed_at = Some(completed_at);
        self
    }

    pub fn with_reasoning_summaries(mut self, summaries: Vec<ReasoningSummary>) -> Self {
        self.reasoning_summaries = summaries;
        self
    }

    pub fn new(ordinal: u64, mid: impl Into<String>, message: Value) -> Self {
        Self {
            ordinal,
            digest: None,
            part_digests: None,
            part_keys: None,
            mid: mid.into(),
            message,
            run: None,
            originals: None,
            tool_calls: Vec::new(),
            role: None,
            written_at: None,
            delivered_as: None,
            rich_ui: None,
            client_message_id: None,
            completed_at: None,
            reasoning_summaries: Vec::new(),
            tool_times: Vec::new(),
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
/// Non-exhaustive so later optional members are additive: use
/// [`HeadMeta::new`], [`HeadMeta::empty`] and the `with_*` setters, or
/// decode one.
#[derive(Clone, Debug, PartialEq, Eq, Deserialize, Serialize)]
#[non_exhaustive]
pub struct HeadMeta {
    /// The newest message's ordinal. Absent on an empty transcript.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub last_ordinal: Option<u64>,
    /// The first entry the latest host marker keeps for the model, or 1
    /// without a marker. Reads still reach all entries from ordinal 1.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub first_ordinal: Option<u64>,
    /// The last entry hidden from the model by the newest host marker.
    /// Absent without a cut; markers themselves are never entries.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub cut: Option<HeadCut>,
    /// The sole rewrite marker. It rises on revert, branch switch, recovery
    /// rewriting written messages, renumbering and re-hosting a session id;
    /// never on a cut or append, and never decreases. The runner keeps a
    /// per-id high-water outside discarded/restored data. A version served for
    /// an earlier history state is never reallocated to a replacement state,
    /// independently of the never-reused (version, incarnation) pair.
    pub history_version: u64,
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
    /// epoch. Present after the first transcript change.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub updated_at: Option<u64>,
    /// What a send that names no model can carry. Absent means unknown.
    /// A never-written session may state it.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub input: Option<HeadInput>,
    /// The runner's recorded selection for the next send that names no model,
    /// not the frozen plan's original selection. Absent means unknown.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub model: Option<HeadModel>,
    /// Durable session spend, present only with the `session_usage` group.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub usage: Option<crate::usage::SessionUsage>,
    /// The attested caller's grant under the latest durably accepted plan,
    /// including a pending refresh whose model input waits for a later run.
    /// With the `head_sender` group, absent means no grant for this caller;
    /// without the group, absent means unknown. Use [`Self::sender_grant`].
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub sender: Option<HeadSender>,
    /// Opaque 1–128-byte identity minted when the runner's session record is
    /// created, and new whenever discarded data is re-hosted. Compare only
    /// by equality, together with `history_version`.
    pub incarnation: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub retention_policy: Option<crate::session_retention::RetentionPolicy>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub retention_state: Option<crate::session_retention::RetentionState>,
    /// Recorded release and the end of its minimum storage hold, until deletion.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub released: Option<crate::session_retention::Released>,
    /// The session's prune state (CONTRACT §9.14.1), the authority every
    /// copy on read pages repeats. Only on a runner
    /// that declares `archive_prunes_reasoning`, and absent until the
    /// session's first prune, so a session never written has none. A prune
    /// is not a snapshot change: the value survives cuts and reverts.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub reasoning_pruned: Option<crate::archive_prunes_reasoning::ReasoningPruned>,
}

/// The caller's own delivery grant on `session.head`, independent of history_version.
#[derive(Clone, Debug, PartialEq, Eq, Deserialize, Serialize)]
#[non_exhaustive]
pub struct HeadSender {
    /// Delivery-mode names exactly as the admission plan uses them
    /// ([`crate::delivery_order::effective_senders`]), never deferred by a rung.
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

/// The next-send selection on `session.head`, independent of its snapshot.
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
    /// A written session's head. Without a marker the model starts at 1.
    pub fn new(
        history_version: u64,
        incarnation: impl Into<String>,
        first_ordinal: u64,
        updated_at: u64,
    ) -> Self {
        let mut head = Self::empty(history_version, incarnation);
        head.first_ordinal = Some(first_ordinal);
        head.updated_at = Some(updated_at);
        head
    }

    /// A never-written session still has a cache identity.
    pub fn empty(history_version: u64, incarnation: impl Into<String>) -> Self {
        Self {
            last_ordinal: None,
            first_ordinal: None,
            cut: None,
            history_version,
            head: None,
            last_run_state: None,
            last_run_error: None,
            last_run_rich_ui: None,
            updated_at: None,
            input: None,
            model: None,
            usage: None,
            sender: None,
            incarnation: incarnation.into(),
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
        self.history_version = history_version;
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
        self.incarnation = incarnation.into();
        self
    }

    /// Whether a reported log identity fits the opaque 1–128-byte wire bound.
    pub fn incarnation_valid(&self) -> bool {
        (1..=128).contains(&self.incarnation.len())
    }

    /// Check ordinal bounds, marker metadata and the required record identity.
    /// A marker limits model input, not the path available to readers.
    pub fn snapshot_consistent(&self) -> bool {
        if !self.incarnation_valid() {
            return false;
        }
        let stamp_has_run = self.last_run_rich_ui.is_none() || self.last_run_state.is_some();
        if self.updated_at.is_none() {
            return self.last_ordinal.is_none()
                && self.first_ordinal.is_none()
                && self.cut.is_none()
                && self.head.is_none()
                && self.last_run_state.is_none()
                && self.reasoning_pruned.is_none()
                && stamp_has_run;
        }
        let bounded = match (self.first_ordinal, self.last_ordinal) {
            (Some(first), Some(last)) => first >= 1 && last >= 1 && first <= last.saturating_add(1),
            (Some(first), None) => first == 1,
            _ => false,
        };
        bounded && (self.cut.is_some() || self.first_ordinal == Some(1)) && stamp_has_run
    }
}

/// The last entry hidden from the model by the newest host marker, described
/// by identity rather than content. The marker and its summary are not entries.
/// `digest` is that entry's stored-content checksum. Reads still reach the
/// whole active path below this boundary.
#[derive(Clone, Debug, PartialEq, Eq, Deserialize, Serialize)]
#[non_exhaustive]
pub struct HeadCut {
    pub mid: String,
    pub digest: String,
    /// When the entry was created, in milliseconds since the Unix epoch.
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
    fn head_incarnation_is_required_and_bounded_in_bytes() {
        assert!(
            serde_json::from_value::<HeadMeta>(serde_json::json!({"history_version":1})).is_err()
        );
        let wire = vectors::load("head-incarnation.json");
        let head: HeadMeta = vectors::round_trip("incarnation", &wire);
        assert!(head.incarnation_valid());
        assert!(head.snapshot_consistent());
        assert!(HeadMeta::empty(1, "log-1").incarnation_valid());
        for (value, valid) in [
            ("".into(), false),
            ("x".repeat(128), true),
            ("x".repeat(129), false),
            ("é".repeat(64), true),
            ("é".repeat(65), false),
        ] {
            assert_eq!(
                HeadMeta::empty(1, "log-1")
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
            HeadMeta::empty(1, "log-1").sender_grant(&describe),
            SenderGrant::Unknown
        );
    }

    #[test]
    fn absent_sender_with_head_sender_group_is_no_grant() {
        let describe = crate::describe::RoleDescribe::default()
            .with_capabilities(vec![crate::capabilities::HEAD_SENDER.into()]);
        assert_eq!(
            HeadMeta::empty(1, "log-1").sender_grant(&describe),
            SenderGrant::NoGrant
        );
    }

    #[test]
    fn present_sender_is_granted_with_or_without_head_sender_group() {
        let sender = HeadSender::for_owner(vec!["idle_start".into()]);
        let head = HeadMeta::empty(1, "log-1").with_sender(sender.clone());
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
            assert!(head.snapshot_consistent());
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
    fn legacy_0_7_read_discards_lineage_id_but_stays_strict() {
        let mut legacy = serde_json::json!({"from_ordinal": 0, "lineage_id": "lineage-sample"});
        let request: ReadRequest = serde_json::from_value(legacy.clone()).unwrap();
        assert_eq!(request, ReadRequest::range(0));
        assert_eq!(request.mode(), Ok(ReadMode::Range { from_ordinal: 1 }));
        assert_eq!(
            serde_json::to_value(request).unwrap(),
            serde_json::json!({"from_ordinal": 0})
        );

        legacy["lineage_idd"] = serde_json::json!("lineage-sample");
        let error = serde_json::from_value::<ReadRequest>(legacy).unwrap_err();
        assert!(
            error.to_string().contains("unknown field `lineage_idd`"),
            "{error}"
        );
    }

    #[test]
    fn legacy_0_7_after_mid_requires_history_version() {
        let request: ReadRequest = serde_json::from_value(serde_json::json!({
            "after_mid": "message-sample", "lineage_id": "lineage-sample"
        }))
        .unwrap();
        assert_eq!(request.mode(), Err(fields::HISTORY_VERSION));
        let refusal = vectors::load("read-requests.json")["refused"][0].clone();
        assert_eq!(refusal["refusal"]["code"], errors::INVALID_PARAMS);
        assert_eq!(
            refusal["refusal"]["detail"]["field"],
            fields::HISTORY_VERSION
        );
        assert_eq!(
            request.with_history_version(7).mode(),
            Ok(ReadMode::After {
                mid: "message-sample",
                history_version: 7
            })
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
        let switch = vectors::load("read-renumbering-switch.json");
        let before = &switch["before"];
        let after: ReadPage = vectors::round_trip("renumbered page", &switch["after"]);
        assert!(after.snapshot_consistent());
        assert_eq!(switch["history_version_before_switch"], 4);
        assert_eq!(after.history_version, 5);
        assert_eq!(after.incarnation, "record-cobalt");
        assert_eq!(before["lineage_id"], "lineage-cobalt");
        assert!(before.get("history_version").is_none());
        assert!(before.get("incarnation").is_none());
        assert!(switch["after"].get("lineage_id").is_none());
        let old_messages = before["messages"].as_array().unwrap();
        assert_eq!(old_messages.len(), 2);
        assert_eq!(after.messages.len(), 2);
        assert_eq!(old_messages[0]["ordinal"], 0);
        assert_eq!(old_messages[1]["ordinal"], 1);
        assert_eq!(after.messages[0].ordinal, 1);
        assert_eq!(after.messages[1].ordinal, 2);
        for (old, new) in old_messages.iter().zip(&after.messages) {
            assert_eq!(old["mid"], new.mid);
            assert_eq!(old["message"], new.message);
        }

        let file = vectors::load("read-pages.json");
        for case in vectors::cases(&file, "pages") {
            let name = case["name"].as_str().unwrap();
            let page: ReadPage = vectors::round_trip(name, &case["page"]);
            if case["stopped_early"].as_bool().unwrap() {
                assert!(page.next_from_ordinal.is_some(), "{name}");
            }
            assert!(page.snapshot_consistent(), "{name}");
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
            assert!(!page.snapshot_consistent(), "{name}");
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
                [errors::HISTORY_CHANGED, errors::UNKNOWN_MID].contains(&code),
                "{name}"
            );
            assert!(!errors::is_retryable(code), "{name}");
            serde_json::from_value::<ReadRequest>(case["request"].clone())
                .unwrap_or_else(|e| panic!("{name}: {e}"));
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
            assert!(head.snapshot_consistent(), "{name}");
        }
        assert!(HeadMeta::empty(1, "log-1").snapshot_consistent());
    }

    #[test]
    fn last_run_error_round_trips_every_field() {
        let head = HeadMeta::new(1, "log-1", 0, 1790000001500)
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
    fn head_without_optional_last_run_error_round_trips_exactly() {
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
            serde_json::to_string(&HeadMeta::empty(1, "log-1")).unwrap(),
            r#"{"history_version":1,"incarnation":"log-1"}"#
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
            assert!(head.snapshot_consistent());
            assert_eq!(head.last_run_state.as_ref().unwrap().state, "error");
            assert_eq!(head.last_run_error().unwrap().retry_after_secs(), Some(1.5));
        }
    }

    #[test]
    fn head_answers_inconsistent_with_their_snapshot_are_rejected() {
        let file = vectors::load("head.json");
        for case in vectors::cases(&file, "inconsistent") {
            let name = case["name"].as_str().unwrap();
            let head: HeadMeta = serde_json::from_value(case["answer"].clone())
                .unwrap_or_else(|e| panic!("{name}: {e}"));
            assert!(!head.snapshot_consistent(), "{name}");
        }
    }
}
