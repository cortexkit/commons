//! `session.history`: the session's whole surviving history, on demand
//! (the `host_history` group, CONTRACT.md §4.6).
//!
//! `session.read` serves only the current lineage, the readable history from
//! the latest cut. `session.history` pages through everything the session
//! still holds from its first message: the part before the lineage, which a
//! runner on a host serves from the host's own storage, and the lineage
//! itself, which it serves exactly as `session.read` does. It is for an app's
//! reader and for raw recovery (a model asking to expand a folded range); a
//! compaction provider never syncs or builds a view from it.
//!
//! History is paged by opaque cursors, never by ordinal: a runner that
//! adopted a session with history already in its host has no ordinals for
//! that history, and numbering it would mean walking the whole session.
//! Entries before the first numbered one carry no `ordinal`.
//!
//! - **Newest page**: no `cursor`, `from_mid` or `before_mid`.
//! - **Forward**: `from_mid` reads from that message, inclusive.
//! - **Backward**: `before_mid` reads the page that ends directly before
//!   that message, exclusive. A reader holding the `session.read` window
//!   continues into older history with the `mid` of the window's first
//!   message; it never infers the session's start from `first_ordinal`,
//!   because a window can start at ordinal 0 with host history before it.
//! - **Cursor**: a page's `older` or `newer`.
//! - **Ordinal seeks**: `from_ordinal` reads from that ordinal, inclusive;
//!   `before_ordinal` reads the page ending directly before it. Only an
//!   ordinal the runner assigned and still holds is valid; any other is
//!   refused `unknown_ordinal`. `before_ordinal` at the lowest assigned
//!   ordinal reaches the newest entries without one, by the boundary rule.
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
    crate::strict::optional(deserializer, fields::VIEW, "\"raw\" or \"model\"")
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

    /// The position the request names, or the field an `invalid_params`
    /// refusal names: `view` for the model view; in display form, `media`
    /// or `include_originals` when given (a row always describes media by
    /// refs and carries no originals); `limits` outside display form; and
    /// otherwise the second of `cursor`, `from_mid`, `before_mid`,
    /// `from_ordinal` and `before_ordinal` present, in that order.
    pub fn mode(&self) -> Result<HistoryMode<'_>, &'static str> {
        if self.view == Some(ReadView::Model) {
            return Err(fields::VIEW);
        }
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
                self.from_ordinal.map(HistoryMode::FromOrdinal),
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
/// [`HistoryPage::new`], [`HistoryPage::no_lineage`] and the `with_*`
/// setters, or decode one.
#[derive(Clone, Debug, PartialEq, Deserialize, Serialize)]
#[non_exhaustive]
pub struct HistoryPage {
    /// Original identities of the pruned parts served on this page, on every read.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub pruned_reasoning: Vec<crate::archive_prunes_reasoning::PrunedReasoning>,
    /// Oldest first.
    pub messages: Vec<HistoryEntry>,
    /// The session's current lineage, as on a `session.read` page. Absent
    /// only for a session that was never written, whose page is empty.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub lineage_id: Option<String>,
    /// The cursor of the page directly before this one. Absent exactly when
    /// nothing older exists.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub older: Option<String>,
    /// The cursor of the page directly after this one. Absent exactly when
    /// nothing newer existed when the page was read.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub newer: Option<String>,
    /// The session's history version, as `session.head` states it. Present
    /// whenever `lineage_id` is.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub history_version: Option<u64>,
}

impl HistoryPage {
    pub fn with_pruned_reasoning(
        mut self,
        parts: Vec<crate::archive_prunes_reasoning::PrunedReasoning>,
    ) -> Self {
        self.pruned_reasoning = parts;
        self
    }
    /// A page of a session whose current lineage is `lineage_id`.
    pub fn new(lineage_id: impl Into<String>, messages: Vec<HistoryEntry>) -> Self {
        Self {
            pruned_reasoning: Vec::new(),
            messages,
            lineage_id: Some(lineage_id.into()),
            older: None,
            newer: None,
            history_version: None,
        }
    }

    pub fn with_history_version(mut self, history_version: u64) -> Self {
        self.history_version = Some(history_version);
        self
    }

    /// The page of a session that was never written: empty, with no
    /// lineage and no cursors.
    pub fn no_lineage() -> Self {
        Self {
            pruned_reasoning: Vec::new(),
            messages: Vec::new(),
            lineage_id: None,
            older: None,
            newer: None,
            history_version: None,
        }
    }

    pub fn with_older(mut self, cursor: impl Into<String>) -> Self {
        self.older = Some(cursor.into());
        self
    }

    pub fn with_newer(mut self, cursor: impl Into<String>) -> Self {
        self.newer = Some(cursor.into());
        self
    }

    /// Check the rules a single page shows on its own, against the current
    /// lineage's `first_ordinal` (`session.head`'s; `None` for a session
    /// without a lineage): a page without a lineage is empty and has no
    /// cursors and no history version, and a page with one carries its
    /// history version; every `mid` appears once; an entry without an ordinal never
    /// follows one with an ordinal; ordinals rise; and only an entry at or
    /// after `first_ordinal` carries `tags`.
    pub fn check(&self, first_ordinal: Option<u64>) -> Result<(), HistoryPageProblem> {
        let unanchored = !self.messages.is_empty()
            || self.older.is_some()
            || self.newer.is_some()
            || self.history_version.is_some();
        if self.lineage_id.is_none() && unanchored {
            return Err(HistoryPageProblem::WithoutLineage);
        }
        if self.lineage_id.is_some() && self.history_version.is_none() {
            return Err(HistoryPageProblem::WithoutHistoryVersion);
        }
        let mut seen = std::collections::BTreeSet::new();
        let mut previous: Option<Option<u64>> = None;
        for entry in &self.messages {
            if !seen.insert(entry.mid.as_str()) {
                return Err(HistoryPageProblem::DuplicateMid(entry.mid.clone()));
            }
            match (previous, entry.ordinal) {
                (Some(Some(_)), None) => {
                    return Err(HistoryPageProblem::UnnumberedAfterNumbered(
                        entry.mid.clone(),
                    ))
                }
                (Some(Some(before)), Some(ordinal)) if ordinal <= before => {
                    return Err(HistoryPageProblem::OrdinalsNotRising(entry.mid.clone()))
                }
                _ => {}
            }
            previous = Some(entry.ordinal);
            let in_window = matches!(
                (entry.ordinal, first_ordinal),
                (Some(ordinal), Some(first)) if ordinal >= first
            );
            if !entry.tags.is_empty() && !in_window {
                return Err(HistoryPageProblem::TagsBelowFirstOrdinal(entry.mid.clone()));
            }
        }
        Ok(())
    }
}

/// Why a [`HistoryPage`] breaks a rule it shows on its own.
#[derive(Clone, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub enum HistoryPageProblem {
    /// A page without a lineage carries entries, cursors or a history
    /// version.
    WithoutLineage,
    /// A page with a lineage carries no history version.
    WithoutHistoryVersion,
    /// This `mid` appears twice on the page.
    DuplicateMid(String),
    /// This entry has no ordinal but follows one that has.
    UnnumberedAfterNumbered(String),
    /// This entry's ordinal is not above the previous entry's.
    OrdinalsNotRising(String),
    /// This entry carries `tags` but lies below `first_ordinal`, or has no
    /// ordinal.
    TagsBelowFirstOrdinal(String),
}

/// One `session.history` entry: a `session.read` entry whose `ordinal` is
/// present only where the runner has one, and whose `written_at` is always
/// present.
///
/// An entry at or after the current lineage's `first_ordinal` is exactly
/// the entry `session.read` serves for that message (bytes, ordinal and
/// tags). An entry below it carries no `tags`. Non-exhaustive: use
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
    /// The message's session-wide ordinal, where the runner has one. An
    /// entry served with an ordinal carries the same one in every later
    /// read; an entry served without one never gains one. A runner never
    /// numbers history on demand.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub ordinal: Option<u64>,
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
    /// Present only on an entry at or after the current lineage's
    /// `first_ordinal`, exactly as `session.read` serves them.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub tags: Vec<crate::tags::PartTag>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub role: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub delivered_as: Option<crate::delivery_order::EntryDelivery>,
}

impl HistoryEntry {
    /// An entry with no ordinal, as the history before a runner's first
    /// numbered message is served.
    pub fn new(mid: impl Into<String>, written_at: u64, message: Value) -> Self {
        Self {
            digest: None,
            part_digests: None,
            ordinal: None,
            mid: mid.into(),
            written_at,
            message,
            run: None,
            originals: None,
            tool_calls: Vec::new(),
            tags: Vec::new(),
            role: None,
            delivered_as: None,
        }
    }

    /// The history entry for a message `session.read` serves as `read`:
    /// the same bytes, ordinal, tags and envelope. `None` when `read` has
    /// no `written_at`, which every history entry carries.
    pub fn from_read(read: ReadMessage) -> Option<Self> {
        Some(Self {
            digest: read.digest,
            part_digests: read.part_digests,
            ordinal: Some(read.ordinal),
            mid: read.mid,
            written_at: read.written_at?,
            message: read.message,
            run: read.run,
            originals: read.originals,
            tool_calls: read.tool_calls,
            tags: read.tags,
            role: read.role,
            delivered_as: read.delivered_as,
        })
    }

    pub fn with_ordinal(mut self, ordinal: u64) -> Self {
        self.ordinal = Some(ordinal);
        self
    }

    pub fn with_role(mut self, role: impl Into<String>) -> Self {
        self.role = Some(role.into());
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

    /// The entry without its `tags`, as history serves a message below the
    /// current lineage's `first_ordinal`.
    pub fn without_tags(mut self) -> Self {
        self.tags = Vec::new();
        self
    }

    /// Whether this entry and `read` serve the same message the same way:
    /// the same `mid`, ordinal, bytes and tags. A message both ops serve
    /// must compare equal.
    pub fn matches_read(&self, read: &ReadMessage) -> bool {
        self.mid == read.mid
            && self.ordinal == Some(read.ordinal)
            && self.message == read.message
            && self.tags == read.tags
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
            assert_eq!(page.check(case["first_ordinal"].as_u64()), Ok(()), "{name}");
        }
        for case in vectors::cases(&file, "pages_refused") {
            let name = case["name"].as_str().unwrap();
            let page: HistoryPage = serde_json::from_value(case["page"].clone())
                .unwrap_or_else(|e| panic!("{name}: {e}"));
            let problem = page.check(case["first_ordinal"].as_u64()).expect_err(name);
            let spelled = match problem {
                HistoryPageProblem::WithoutLineage => "without_lineage",
                HistoryPageProblem::WithoutHistoryVersion => "without_history_version",
                HistoryPageProblem::DuplicateMid(_) => "duplicate_mid",
                HistoryPageProblem::UnnumberedAfterNumbered(_) => "unnumbered_after_numbered",
                HistoryPageProblem::OrdinalsNotRising(_) => "ordinals_not_rising",
                HistoryPageProblem::TagsBelowFirstOrdinal(_) => "tags_below_first_ordinal",
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
            .with_tags(vec![crate::tags::PartTag::tagged("0", "§12§")]);
        let entry = HistoryEntry::from_read(read.clone()).unwrap();
        assert!(entry.matches_read(&read));
        assert!(!entry.clone().without_tags().matches_read(&read));
        assert_eq!(
            HistoryEntry::from_read(ReadMessage::new(7, "m7", serde_json::json!({}))),
            None
        );
        let page = HistoryPage::new("lin-2", vec![entry.clone()]).with_history_version(1);
        assert_eq!(page.check(Some(7)), Ok(()));
        assert_eq!(
            page.check(Some(8)),
            Err(HistoryPageProblem::TagsBelowFirstOrdinal("m7".into()))
        );
        assert_eq!(
            HistoryRequest::from_mid("m1")
                .with_view(ReadView::Model)
                .mode(),
            Err(fields::VIEW)
        );
    }
}
