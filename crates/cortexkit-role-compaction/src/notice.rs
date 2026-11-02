//! `compaction.notice`, served by the provider. After every durable write to
//! a session's transcript, the runner sends where the transcript stands, plus
//! the new entries in compact form. The provider acknowledges at once and does
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

/// Above roughly this many bytes of entries, a notice carries none and the
/// provider reads them instead, keeping every call far below the transport's
/// frame limit. Compact form keeps text parts and full tool inputs, so large
/// text, a large tool input or many entries can reach it.
pub const ENTRIES_BUDGET_BYTES: usize = 1 << 20;

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[non_exhaustive]
pub struct NoticeRequest {
    /// 1 for the first notice after each Setup, then one more for each
    /// notice, so the provider can tell a repeated or reordered notice apart.
    pub seq: u64,
    pub newest: Cursor,
    /// The view the runner applies now, absent when it applies none.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub applied_view: Option<AppliedView>,
    pub conditions: Conditions,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub events: Vec<Event>,
    /// Entries written since the previous notice, oldest first, in compact
    /// form. A message that changes after it was sent (a running tool gets
    /// its result, or a send is delivered with it) is sent again at the same
    /// ordinal, even when the host stored the result in a later message.
    /// Empty when they would exceed [`ENTRIES_BUDGET_BYTES`].
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub entries: Vec<Entry>,
}

/// What the session runs under now. Notices and rides both carry it, so a
/// change reaches the provider with the next one.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[non_exhaustive]
pub struct Conditions {
    pub model: ModelRef,
    pub context_window: u64,
    pub output_reserve: u64,
    pub rules: ProviderRules,
    /// The prompt size of the newest request, as the provider reported it.
    /// Absent before the first request, or when the provider reported none.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub last_prompt_tokens: Option<u64>,
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
    TranscriptChanged => "transcript_changed" { from_ordinal: u64 },
    /// The runner wrote the host's own compaction marker for this view, so the
    /// host stops loading the messages the view leaves out at the start.
    MarkerWritten => "marker_written" { version: u64 },
    /// A view the runner refused when it came to apply it: a ride's answer,
    /// or an accepted view that no longer fits the transcript at the next step
    /// boundary. Otherwise a view handed over with `session.apply_view` is
    /// refused in that call's answer.
    ViewRefused => "view_refused" { version: u64, refusal: ViewRefusal },
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
    pub fn new(seq: u64, newest: Cursor, conditions: Conditions) -> Self {
        Self {
            seq,
            newest,
            conditions,
            applied_view: None,
            events: Vec::new(),
            entries: Vec::new(),
        }
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
            context_window,
            output_reserve,
            rules,
            last_prompt_tokens: None,
        }
    }

    pub fn with_last_prompt_tokens(mut self, last_prompt_tokens: u64) -> Self {
        self.last_prompt_tokens = Some(last_prompt_tokens);
        self
    }
}
