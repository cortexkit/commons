//! `session.history`: whole active-path history with backward navigation
//! The `host_history` group adds backward navigation with opaque cursors.
//!
//! Both read operations reach entries below native markers from the host's
//! store. Every active-path message counts from 1, including an empty dead
//! step; marker and control records are not entries. History's opaque cursors
//! survive a cut or archive prune. Removing or rewriting a cursor's anchor
//! message, or anything older, invalidates that cursor.
//!
//! - No position means the newest page.
//! - `from_mid` reads forward, inclusive; `before_mid` reads backward, exclusive.
//! - `cursor` continues a prior page's `older` or `newer` position.
//! - **Ordinal seeks**: `from_ordinal` reads from that ordinal, inclusive;
//!   `before_ordinal` reads the page ending directly before it. Only an
//!   ordinal the runner assigned and still holds is valid; any other is
//!   refused `unknown_ordinal`. Zero aliases the first entry for from_ordinal;
//!   before_ordinal 1 returns an empty page with no older cursor.
//!   Paging onward uses the returned cursors.
//!
//! Entries run oldest first within a page, and following `older` or `newer`
//! never overlaps or skips an entry. `older` is absent exactly when nothing
//! older exists; `newer` is absent exactly when nothing newer existed when
//! the page was read.

use serde::{Deserialize, Deserializer, Serialize};
use serde_json::Value;

use crate::read::{
    descriptors, fields, MediaMode, Original, ReadMessage, ReadView, RunAttribution,
    ToolCallAttribution,
};

fn deserialize_view<'de, D>(deserializer: D) -> Result<Option<ReadView>, D::Error>
where
    D: Deserializer<'de>,
{
    crate::strict::optional(deserializer, fields::VIEW, "\"raw\"")
}

/// The `session.history` request.
///
/// Strict on unknown fields, as `session.read`'s request is: a misspelled
/// cursor must be refused rather than read as a request for the newest
/// page. At most one of `cursor`, `from_mid` and `before_mid` is given. Byte
/// caps and media modes are `session.read`'s. `view: "model"` is refused
/// naming `view`: history is raw entries only.
///
/// Non-exhaustive so a later optional member is additive: build one with
/// [`HistoryRequest::newest`], [`HistoryRequest::from_mid`],
/// [`HistoryRequest::before_mid`] or [`HistoryRequest::cursor`] and the
/// `with_*` setters, or decode one.
#[derive(Clone, Debug, Default, PartialEq, Eq, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
#[non_exhaustive]
pub struct HistoryRequest {
    /// A page's `older` or `newer`, passed back exactly as received.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub cursor: Option<String>,
    /// Read forward from this message, inclusive.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub from_mid: Option<String>,
    /// Read the page that ends directly before this message, exclusive.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub before_mid: Option<String>,
    /// Read forward from the entry with this ordinal, inclusive.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub from_ordinal: Option<u64>,
    /// Read the page that ends directly before the entry with this ordinal,
    /// exclusive.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub before_ordinal: Option<u64>,
    /// At most this many entries. The runner may cap it lower.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub limit: Option<u64>,
    /// At most this many bytes of entries, as `session.read` caps a page.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub max_bytes: Option<u64>,
    /// Payload representation, as on `session.read`; absent means `inline`.
    #[serde(
        default,
        skip_serializing_if = "Option::is_none",
        deserialize_with = "descriptors::deserialize_media"
    )]
    pub media: Option<MediaMode>,
    /// Add `originals` to every entry a hook changed, where the runner
    /// holds them.
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub include_originals: bool,
    /// Only `raw` is served; `model` is refused naming `view`.
    #[serde(
        default,
        skip_serializing_if = "Option::is_none",
        deserialize_with = "deserialize_view"
    )]
    pub view: Option<ReadView>,
    /// `display` serves each entry as a canonical row
    /// ([`crate::display::DisplayPage`]) on a runner declaring
    /// `history_display`; absent or `raw` serves stored bodies.
    #[serde(
        default,
        skip_serializing_if = "Option::is_none",
        deserialize_with = "deserialize_form"
    )]
    pub form: Option<HistoryForm>,
    /// Byte bounds on a display row's tool payloads. Only with
    /// `form: "display"`; absent means whole content.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub limits: Option<crate::display::Limits>,
}

/// What a `session.history` entry carries: the stored body, or a display
/// row in its place.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Deserialize, Serialize)]
#[serde(rename_all = "snake_case")]
#[non_exhaustive]
pub enum HistoryForm {
    /// The stored body, as `session.read` serves it. The default.
    #[default]
    Raw,
    /// A canonical row ([`crate::display::DisplayRow`]) in place of the
    /// body.
    Display,
}

/// The request members of the display form.
pub mod history_fields {
    pub const FORM: &str = "form";
    pub const LIMITS: &str = "limits";
}

fn deserialize_form<'de, D>(deserializer: D) -> Result<Option<HistoryForm>, D::Error>
where
    D: Deserializer<'de>,
{
    crate::strict::optional(deserializer, history_fields::FORM, "\"raw\" or \"display\"")
}

/// The position a well-formed [`HistoryRequest`] names.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum HistoryMode<'a> {
    /// The newest page.
    Newest,
    /// A page's `older` or `newer` cursor.
    Cursor(&'a str),
    /// Forward from this message, inclusive.
    FromMid(&'a str),
    /// The page ending directly before this message, exclusive.
    BeforeMid(&'a str),
    /// Forward from the entry with this ordinal, inclusive.
    FromOrdinal(u64),
    /// The page ending directly before the entry with this ordinal,
    /// exclusive.
    BeforeOrdinal(u64),
}

impl HistoryRequest {
    /// The newest page.
    pub fn newest() -> Self {
        Self::default()
    }

    /// The page starting at `mid`, inclusive.
    pub fn from_mid(mid: impl Into<String>) -> Self {
        Self {
            from_mid: Some(mid.into()),
            ..Self::default()
        }
    }

    /// The page ending directly before `mid`, exclusive.
    pub fn before_mid(mid: impl Into<String>) -> Self {
        Self {
            before_mid: Some(mid.into()),
            ..Self::default()
        }
    }

    /// The page a page's `older` or `newer` names.
    pub fn cursor(cursor: impl Into<String>) -> Self {
        Self {
            cursor: Some(cursor.into()),
            ..Self::default()
        }
    }

    /// The page starting at the entry with `ordinal`, inclusive.
    pub fn from_ordinal(ordinal: u64) -> Self {
        Self {
            from_ordinal: Some(ordinal),
            ..Self::default()
        }
    }

    /// The page ending directly before the entry with `ordinal`.
    pub fn before_ordinal(ordinal: u64) -> Self {
        Self {
            before_ordinal: Some(ordinal),
            ..Self::default()
        }
    }

    pub fn with_limit(mut self, limit: u64) -> Self {
        self.limit = Some(limit);
        self
    }

    pub fn with_max_bytes(mut self, max_bytes: u64) -> Self {
        self.max_bytes = Some(max_bytes);
        self
    }

    pub fn with_media(mut self, media: MediaMode) -> Self {
        self.media = Some(media);
        self
    }

    /// Check opt-in media modes against the runner's declared groups.
    /// Display-form requests still reject every explicit media mode in `mode`.
    pub fn check_media<S: AsRef<str>>(&self, groups: &[S]) -> Result<(), &'static str> {
        self.media.unwrap_or_default().check_declared(groups)
    }

    pub fn with_include_originals(mut self, include_originals: bool) -> Self {
        self.include_originals = include_originals;
        self
    }

    pub fn with_view(mut self, view: ReadView) -> Self {
        self.view = Some(view);
        self
    }

    pub fn with_form(mut self, form: HistoryForm) -> Self {
        self.form = Some(form);
        self
    }

    pub fn with_limits(mut self, limits: crate::display::Limits) -> Self {
        self.limits = Some(limits);
        self
    }

    /// The form the request asks for, with absence read as raw.
    pub fn form(&self) -> HistoryForm {
        self.form.unwrap_or_default()
    }

    /// The position the request names, or the conflicting display/position
    /// field an `invalid_params` refusal names: in display form, `media`
    /// or `include_originals` when given (a row always describes media by
    /// refs and carries no originals); `limits` outside display form; and
    /// otherwise the second of `cursor`, `from_mid`, `before_mid`,
    /// `from_ordinal` and `before_ordinal` present, in that order.
    /// View validity is checked while decoding.
    pub fn mode(&self) -> Result<HistoryMode<'_>, &'static str> {
        if self.form() == HistoryForm::Display {
            if self.media.is_some() {
                return Err(fields::MEDIA);
            }
            if self.include_originals {
                return Err("include_originals");
            }
        } else if self.limits.is_some() {
            return Err(history_fields::LIMITS);
        }
        let positions = [
            (
                fields::CURSOR,
                self.cursor.as_deref().map(HistoryMode::Cursor),
            ),
            (
                fields::FROM_MID,
                self.from_mid.as_deref().map(HistoryMode::FromMid),
            ),
            (
                fields::BEFORE_MID,
                self.before_mid.as_deref().map(HistoryMode::BeforeMid),
            ),
            (
                fields::FROM_ORDINAL,
                self.from_ordinal
                    .map(|ordinal| HistoryMode::FromOrdinal(ordinal.max(1))),
            ),
            (
                fields::BEFORE_ORDINAL,
                self.before_ordinal.map(HistoryMode::BeforeOrdinal),
            ),
        ];
        let mut given = positions
            .into_iter()
            .filter_map(|(field, mode)| mode.map(|mode| (field, mode)));
        match (given.next(), given.next()) {
            (_, Some((second, _))) => Err(second),
            (Some((_, mode)), None) => Ok(mode),
            (None, None) => Ok(HistoryMode::Newest),
        }
    }
}

/// A `session.history` page. Decoded leniently.
///
/// Non-exhaustive so later optional members are additive: use
/// [`HistoryPage::new`] and the `with_*`
/// setters, or decode one.
#[derive(Clone, Debug, PartialEq, Deserialize, Serialize)]
#[non_exhaustive]
pub struct HistoryPage {
    /// Original identities of the pruned parts served on this page, on every read.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub pruned_reasoning: Vec<crate::archive_prunes_reasoning::PrunedReasoning>,
    /// Oldest first.
    pub messages: Vec<HistoryEntry>,
    pub incarnation: String,
    /// The cursor of the page directly before this one. Absent exactly when
    /// nothing older exists.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub older: Option<String>,
    /// The cursor of the page directly after this one. Absent exactly when
    /// nothing newer existed when the page was read.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub newer: Option<String>,
    /// The rewrite version of this snapshot, including empty pages.
    pub history_version: u64,
}

impl HistoryPage {
    pub fn with_pruned_reasoning(
        mut self,
        parts: Vec<crate::archive_prunes_reasoning::PrunedReasoning>,
    ) -> Self {
        self.pruned_reasoning = parts;
        self
    }
    /// A page bound to the session's rewrite version and record identity.
    pub fn new(
        history_version: u64,
        incarnation: impl Into<String>,
        messages: Vec<HistoryEntry>,
    ) -> Self {
        Self {
            pruned_reasoning: Vec::new(),
            messages,
            incarnation: incarnation.into(),
            older: None,
            newer: None,
            history_version,
        }
    }

    pub fn with_history_version(mut self, history_version: u64) -> Self {
        self.history_version = history_version;
        self
    }

    pub fn with_older(mut self, cursor: impl Into<String>) -> Self {
        self.older = Some(cursor.into());
        self
    }

    pub fn with_newer(mut self, cursor: impl Into<String>) -> Self {
        self.newer = Some(cursor.into());
        self
    }

    /// Check the envelope's record identity, unique mids and rising one-based
    /// ordinals. Markers and controls are not entries. The model's marker
    /// boundary never limits this check or the read.
    pub fn check(&self) -> Result<(), HistoryPageProblem> {
        if !(1..=128).contains(&self.incarnation.len()) {
            return Err(HistoryPageProblem::InvalidIncarnation);
        }
        let mut seen = std::collections::BTreeSet::new();
        let mut previous: Option<u64> = None;
        for entry in &self.messages {
            if entry.ordinal == 0 {
                return Err(HistoryPageProblem::InvalidOrdinal(entry.mid.clone()));
            }
            if !seen.insert(entry.mid.as_str()) {
                return Err(HistoryPageProblem::DuplicateMid(entry.mid.clone()));
            }
            if previous.is_some_and(|before| entry.ordinal <= before) {
                return Err(HistoryPageProblem::OrdinalsNotRising(entry.mid.clone()));
            }
            previous = Some(entry.ordinal);
        }
        Ok(())
    }
}

/// Why a [`HistoryPage`] breaks a rule it shows on its own.
#[derive(Clone, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub enum HistoryPageProblem {
    InvalidIncarnation,
    InvalidOrdinal(String),
    /// This `mid` appears twice on the page.
    DuplicateMid(String),
    /// This entry's ordinal is not above the previous entry's.
    OrdinalsNotRising(String),
}

/// One `session.history` entry: a `session.read` entry with required one-based ordinal and write time. Markers and
/// control records are not entries.
///
/// Every entry has the same identity and stored bytes as its corresponding `session.read` entry,
/// including below a model cut. Non-exhaustive: use
/// [`HistoryEntry::new`] and the `with_*` setters, [`HistoryEntry::from_read`],
/// or decode one.
#[derive(Clone, Debug, PartialEq, Deserialize, Serialize)]
#[non_exhaustive]
pub struct HistoryEntry {
    /// As on a `session.read` entry.
    #[serde(
        default,
        skip_serializing_if = "Option::is_none",
        deserialize_with = "descriptors::deserialize_digest"
    )]
    pub digest: Option<String>,
    /// As on a `session.read` entry.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub part_digests: Option<Vec<descriptors::PartDigest>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub part_keys: Option<Vec<String>>,
    /// Required one-based position on the active path. A rewrite raises
    /// history_version before a position changes; a cut never changes it.
    pub ordinal: u64,
    /// The message's id, unchanged for the life of the session, and the
    /// same id `session.read` serves for the message. On a runner over a
    /// host, a host-served entry's `mid` is the host's own message id.
    pub mid: String,
    /// When the message was created, in milliseconds since the Unix epoch;
    /// for a host-served entry, the host's own time.
    pub written_at: u64,
    /// The message itself, in the runner's schema.
    #[serde(deserialize_with = "descriptors::deserialize_content")]
    pub message: Value,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub run: Option<RunAttribution>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub originals: Option<Vec<Original>>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub tool_calls: Vec<ToolCallAttribution>,
    /// The message role, as on `ReadMessage::role`: present whenever known,
    /// including below the model's `first_ordinal`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub role: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub delivered_as: Option<crate::delivery_order::EntryDelivery>,
    /// Sending module id, with the same positive-only guarantee as
    /// [`ReadMessage::sender`]. Raw history has no display-facts request flag;
    /// outside all checksums and does not move `history_version`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub sender: Option<String>,
    /// The same positive-only module send identity as [`ReadMessage::send_id`].
    /// Served beside sender under read_display_facts, without a request flag.
    /// The send_ids group guarantees it when known; otherwise absence is unknown.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub send_id: Option<String>,
    /// Agent whose words this module carries; outside bodies and checksums.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub speaks_for: Option<crate::message_marks::SpeaksFor>,
    /// Positive automatic-message mark; false is absent on the wire.
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub notice: bool,
    /// Verified stored relay frame when `sender_attribution` is declared.
    /// Raw history has no display-facts request flag; message bytes stay whole.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub attribution: Option<crate::attribution::Attribution>,
}

impl HistoryEntry {
    pub fn with_speaks_for(mut self, speaks_for: crate::message_marks::SpeaksFor) -> Self {
        self.speaks_for = Some(speaks_for);
        self
    }

    pub fn with_notice(mut self, notice: bool) -> Self {
        self.notice = notice;
        self
    }

    pub fn with_sender(mut self, sender: impl Into<String>) -> Self {
        self.sender = Some(sender.into());
        self
    }

    pub fn with_send_id(mut self, send_id: impl Into<String>) -> Self {
        self.send_id = Some(send_id.into());
        self
    }

    pub fn with_part_keys(mut self, keys: Vec<String>) -> Self {
        self.part_keys = Some(keys);
        self
    }
    /// A counted active-path entry, including messages the host already held
    /// before the runner began serving the session.
    pub fn new(ordinal: u64, mid: impl Into<String>, written_at: u64, message: Value) -> Self {
        Self {
            digest: None,
            part_digests: None,
            part_keys: None,
            ordinal,
            mid: mid.into(),
            written_at,
            message,
            run: None,
            originals: None,
            tool_calls: Vec::new(),
            role: None,
            delivered_as: None,
            sender: None,
            send_id: None,
            speaks_for: None,
            notice: false,
            attribution: None,
        }
    }

    /// The history entry for a message `session.read` serves as `read`:
    /// the same bytes, ordinal, part-digest rows and served part keys. `None` when `read` has
    /// no `written_at`, which every history entry carries.
    pub fn from_read(read: ReadMessage) -> Option<Self> {
        Some(Self {
            digest: read.digest,
            part_digests: read.part_digests,
            part_keys: read.part_keys,
            ordinal: read.ordinal,
            mid: read.mid,
            written_at: read.written_at?,
            message: read.message,
            run: read.run,
            originals: read.originals,
            tool_calls: read.tool_calls,
            role: read.role,
            delivered_as: read.delivered_as,
            sender: read.sender,
            send_id: read.send_id,
            speaks_for: read.speaks_for,
            notice: read.notice,
            attribution: read.attribution,
        })
    }

    pub fn with_ordinal(mut self, ordinal: u64) -> Self {
        self.ordinal = ordinal;
        self
    }

    pub fn with_role(mut self, role: impl Into<String>) -> Self {
        self.role = Some(role.into());
        self
    }

    pub fn with_run(mut self, run: RunAttribution) -> Self {
        self.run = Some(run);
        self
    }

    /// Whether this entry and `read` serve the same message the same way:
    /// the same `mid`, ordinal, bytes, part-digest rows and served part keys. A message both ops serve
    /// must compare equal.
    pub fn matches_read(&self, read: &ReadMessage) -> bool {
        self.mid == read.mid
            && self.ordinal == read.ordinal
            && self.message == read.message
            && self.part_digests == read.part_digests
            && self.part_keys == read.part_keys
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{errors, vectors};

    #[test]
    fn request_vectors_decode_round_trip_and_select_their_mode() {
        let file = vectors::load("history.json");
        for case in vectors::cases(&file, "requests") {
            let name = case["name"].as_str().unwrap();
            let request: HistoryRequest = vectors::round_trip(name, &case["request"]);
            let mode = match request.mode() {
                Ok(HistoryMode::Newest) => "newest",
                Ok(HistoryMode::Cursor(_)) => "cursor",
                Ok(HistoryMode::FromMid(_)) => "from_mid",
                Ok(HistoryMode::BeforeMid(_)) => "before_mid",
                Ok(HistoryMode::FromOrdinal(_)) => "from_ordinal",
                Ok(HistoryMode::BeforeOrdinal(_)) => "before_ordinal",
                Err(field) => panic!("{name}: refused naming {field}"),
            };
            assert_eq!(mode, case["mode"].as_str().unwrap(), "{name}");
        }
        for case in vectors::cases(&file, "refused_requests") {
            let name = case["name"].as_str().unwrap();
            let field = case["field"].as_str().unwrap();
            match serde_json::from_value::<HistoryRequest>(case["request"].clone()) {
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
    fn page_vectors_decode_round_trip_and_check() {
        let file = vectors::load("history.json");
        for case in vectors::cases(&file, "pages") {
            let name = case["name"].as_str().unwrap();
            let page: HistoryPage = vectors::round_trip(name, &case["page"]);
            assert_eq!(page.check(), Ok(()), "{name}");
        }
        for case in vectors::cases(&file, "pages_refused") {
            let name = case["name"].as_str().unwrap();
            let page: HistoryPage = serde_json::from_value(case["page"].clone())
                .unwrap_or_else(|e| panic!("{name}: {e}"));
            let problem = page.check().expect_err(name);
            let spelled = match problem {
                HistoryPageProblem::InvalidIncarnation => "invalid_incarnation",
                HistoryPageProblem::InvalidOrdinal(_) => "invalid_ordinal",
                HistoryPageProblem::DuplicateMid(_) => "duplicate_mid",
                HistoryPageProblem::OrdinalsNotRising(_) => "ordinals_not_rising",
            };
            assert_eq!(spelled, case["problem"].as_str().unwrap(), "{name}");
        }
        for case in vectors::cases(&file, "pages_undecodable") {
            assert!(
                serde_json::from_value::<HistoryPage>(case["page"].clone()).is_err(),
                "{}",
                case["name"]
            );
        }
    }

    #[test]
    fn refusals_are_named_and_terminal() {
        let file = vectors::load("history.json");
        for case in vectors::cases(&file, "refusals") {
            let name = case["name"].as_str().unwrap();
            let code = case["refusal"]["code"].as_str().unwrap();
            assert!(errors::CODES.contains(&code), "{name}: {code}");
            assert!(!errors::is_retryable(code), "{name}");
        }
    }

    #[test]
    fn a_window_entry_is_the_read_entry() {
        let read = ReadMessage::new(7, "m7", serde_json::json!({"role": "user"}))
            .with_written_at(1_790_000_000_000)
            .with_part_keys(vec!["opaque-part-key".into()]);
        let entry = HistoryEntry::from_read(read.clone()).unwrap();
        assert!(entry.matches_read(&read));
        assert_eq!(
            HistoryEntry::from_read(ReadMessage::new(7, "m7", serde_json::json!({}))),
            None
        );
        let page = HistoryPage::new(1, "log-1", vec![entry.clone()]).with_history_version(1);
        assert_eq!(page.check(), Ok(()));
        assert_eq!(entry.part_keys, Some(vec!["opaque-part-key".into()]));
    }
}
