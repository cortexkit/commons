//! `compaction.notice`, served by the provider. On each W5.2 notice trigger,
//! the runner sends where the transcript stands, plus new and changed entries
//! in compact form. Count metadata updates alone do not trigger a notice;
//! an update that also includes a trigger sends the notice normally.
//! The provider acknowledges at once and does
//! its work afterwards, so the runner never waits on it.
//!
//! A notice is a convenience, not the source of truth: the provider keeps its
//! own position and reads any gap with `session.entries`. So a notice that never
//! arrives (the provider was down, a call failed) only delays its work until
//! the next notice or Setup.

use serde::{Deserialize, Serialize};

use crate::read::{Cursor, Entry};
use crate::rules::ProviderRules;
use crate::setup::ModelRef;
use crate::view::{AppliedView, ViewRefusal};

/// Budget for the serialized compact `entries` array. Drop entry counts
/// first, then omit entries if still over budget; the provider may read them
/// with `session.entries`. Condition counts stay, and family metadata stays
/// whenever any count remains. Compact detail keeps text and full tool inputs,
/// so large text, a large tool input or many entries can reach this limit.
pub const ENTRIES_BUDGET_BYTES: usize = 1 << 20;

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[non_exhaustive]
pub struct NoticeRequest {
    /// 1 for the first notice after each Setup, then one more for each
    /// notice, so the provider can tell a repeated or reordered notice apart.
    #[serde(deserialize_with = "crate::integer::required")]
    pub seq: u64,
    /// The newest entry, absent only when the transcript is empty.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub newest: Option<Cursor>,
    /// The view the runner applies now, absent when it applies none.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub applied_view: Option<AppliedView>,
    pub conditions: Conditions,
    /// Opaque count family, required when this notice carries W4.11 counts.
    /// Adopt each response's family in receipt order and discard held counts
    /// from other families. Compare the whole id by equality only.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub token_family: Option<String>,
    /// Whether the family uses the model's own tokenizer (`true`) rather
    /// than a stand-in (`false`). Absent is unknown, treated as `false`.
    /// Sent alongside `token_family`; excluded from all W7 checksums.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub token_family_exact: Option<bool>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub events: Vec<Event>,
    /// Entries written since the previous notice, oldest first, in compact
    /// form. A message that changes after it was sent (a running tool gets
    /// its result, or a send is delivered with it) is sent again at the same
    /// ordinal, even when the host stored the result in a later message.
    /// Drop entry counts first to fit [`ENTRIES_BUDGET_BYTES`], then omit
    /// entries if still over budget. Condition counts stay; `session.entries`
    /// returns omitted entries and their counts, never condition counts.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub entries: Vec<Entry>,
}

/// What the session runs under now. Notices and rides both carry it, so a
/// change reaches the provider with the next one.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[non_exhaustive]
pub struct Conditions {
    pub model: ModelRef,
    /// Runner-authoritative signed reasoning boundary. Send only to providers
    /// declaring `compaction-signed-floor/v1`; see [`crate::SignedFloor`].
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub signed_floor: Option<crate::SignedFloor>,
    /// Whether the session's model reads images. Set only when known; absent
    /// means unknown. A provider inserts images only when this is `true`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub accepts_images: Option<bool>,
    #[serde(deserialize_with = "crate::integer::required")]
    pub context_window: u64,
    #[serde(deserialize_with = "crate::integer::required")]
    pub output_reserve: u64,
    pub rules: ProviderRules,
    /// The prompt size of the newest request, as the provider reported it.
    /// Absent before the first request, or when the provider reported none.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[serde(deserialize_with = "crate::integer::optional")]
    pub last_prompt_tokens: Option<u64>,
    /// The applied view version used to render the request measured by
    /// `last_prompt_tokens` when the runner built it. `0` means no view was
    /// applied; absent means unknown, for example after a runner restart.
    /// Send only with `last_prompt_tokens`; providers ignore it without a reading.
    /// W6.5's `stale_version` refuses any version not greater than the latest
    /// accepted, so a runner-reported version names exactly one view in that
    /// session. When present, a reading counts toward compaction only if this
    /// equals the newest version the provider has seen the runner apply; a lower
    /// version, or `0` once any view is applied, is stale.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[serde(deserialize_with = "crate::integer::optional")]
    pub last_prompt_view: Option<u64>,
    /// Count of the runner's own system text for the next request under these
    /// conditions, excluding system entries. Notices only, never rides (W4.11).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub system_tokens: Option<crate::tokens::SurfaceTokens>,
    /// Count of the next request's tool definitions under these conditions.
    /// Notices only, never rides (W4.11).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub tools_tokens: Option<crate::tokens::SurfaceTokens>,
}

open_tagged! {
#[derive(Clone, Debug, PartialEq)]
pub enum Event, "event" {
    /// Entries from this ordinal on may differ from what the provider saw
    /// before: a revert, a branch switch, or a host that changed stored
    /// messages. The provider re-reads from there; edits naming messages that
    /// are gone leave its next view. Until that view arrives, the runner
    /// renders with the edits that still apply
    /// using the view library's transcript filtering.
    TranscriptChanged => "transcript_changed" {
        #[serde(deserialize_with = "crate::integer::required")]
        from_ordinal: u64
    },
    /// The runner wrote the host's own compaction marker for this view, so the
    /// host stops loading the messages the view leaves out at the start.
    MarkerWritten => "marker_written" {
        #[serde(deserialize_with = "crate::integer::required")]
        version: u64
    },
    /// A view the runner refused when it came to apply it: a ride's answer,
    /// or an accepted view that no longer fits the transcript at the next step
    /// boundary. Otherwise a view handed over with `session.apply_view` is
    /// refused in that call's answer.
    ViewRefused => "view_refused" {
        #[serde(deserialize_with = "crate::integer::required")]
        version: u64,
        refusal: ViewRefusal
    },
}
}

open_tagged! {
/// The provider's answer: it has the notice, which says nothing about the
/// work the notice causes; or it does not know the session (it restarted),
/// and the runner calls Setup again before its next notice.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum NoticeAnswer, "answer" {
    Received => "received",
    SetupRequired => "setup_required",
}
}

impl NoticeRequest {
    /// Build the value with its required fields.
    pub fn new(seq: u64, conditions: Conditions) -> Self {
        Self {
            seq,
            newest: None,
            conditions,
            applied_view: None,
            token_family: None,
            token_family_exact: None,
            events: Vec::new(),
            entries: Vec::new(),
        }
    }

    pub fn with_newest(mut self, newest: Cursor) -> Self {
        self.newest = Some(newest);
        self
    }

    pub fn with_applied_view(mut self, applied_view: AppliedView) -> Self {
        self.applied_view = Some(applied_view);
        self
    }

    pub fn with_events(mut self, events: Vec<Event>) -> Self {
        self.events = events;
        self
    }

    pub fn with_entries(mut self, entries: Vec<Entry>) -> Self {
        self.entries = entries;
        self
    }

    pub fn with_token_family(mut self, family: impl Into<String>) -> Self {
        self.token_family = Some(family.into());
        self
    }

    pub fn with_token_family_exact(mut self, exact: bool) -> Self {
        self.token_family_exact = Some(exact);
        self
    }
}

impl Conditions {
    /// Build the value with its required fields.
    pub fn new(
        model: ModelRef,
        context_window: u64,
        output_reserve: u64,
        rules: ProviderRules,
    ) -> Self {
        Self {
            model,
            signed_floor: None,
            accepts_images: None,
            context_window,
            output_reserve,
            rules,
            last_prompt_tokens: None,
            last_prompt_view: None,
            system_tokens: None,
            tools_tokens: None,
        }
    }

    pub fn with_accepts_images(mut self, accepts_images: bool) -> Self {
        self.accepts_images = Some(accepts_images);
        self
    }

    /// Project the boundary only when the provider declares its capability.
    pub fn with_signed_floor(mut self, floor: crate::SignedFloor, declared: bool) -> Self {
        self.signed_floor = declared.then_some(floor);
        self
    }

    pub fn with_last_prompt_tokens(mut self, last_prompt_tokens: u64) -> Self {
        self.last_prompt_tokens = Some(last_prompt_tokens);
        self
    }

    pub fn with_last_prompt_view(mut self, last_prompt_view: u64) -> Self {
        self.last_prompt_view = Some(last_prompt_view);
        self
    }

    pub fn with_system_tokens(mut self, tokens: crate::tokens::SurfaceTokens) -> Self {
        self.system_tokens = Some(tokens);
        self
    }

    pub fn with_tools_tokens(mut self, tokens: crate::tokens::SurfaceTokens) -> Self {
        self.tools_tokens = Some(tokens);
        self
    }
}
