//! `compaction.step`: the per-step status the runner sends.
//!
//! The status carries everything the provider needs for one step: the
//! messages added since its last call, with the ordinal cursor they start
//! after, so it never depends on reading the runner's storage; the id and
//! version of the last applied CompactionMessage, so a provider that lost
//! its counter resumes above it; and the step's model, window and usage
//! numbers. For anything older the provider reads the transcript itself.
//!
//! Decoded leniently: a provider ignores fields it does not know, so a
//! newer runner can add some without breaking an older provider.

use serde::{Deserialize, Serialize};
use serde_json::Value;

/// A message the runner wrote, by its place in the lineage.
#[derive(Clone, Debug, PartialEq, Eq, Deserialize, Serialize)]
pub struct MessageRef {
    pub ordinal: u64,
    /// The runner's opaque message id.
    pub mid: String,
}

/// One message the status carries: its ordinal, its id and its body in the
/// runner's message schema, with its final values (what later steps render
/// and what executed). The same three members a `session.read` page gives
/// each message; a runner may add others, which a provider ignores.
#[derive(Clone, Debug, PartialEq, Deserialize, Serialize)]
pub struct StatusMessage {
    pub ordinal: u64,
    pub mid: String,
    pub message: Value,
}

/// Token usage the previous step reported.
#[derive(Clone, Debug, PartialEq, Eq, Deserialize, Serialize)]
pub struct Usage {
    pub input: u64,
    pub cache_read: u64,
    pub cache_write: u64,
    pub output: u64,
    /// When the step completed, in milliseconds since the Unix epoch.
    pub completed_at: u64,
    /// The model provider's finish reason, as an open string.
    pub finish_reason: String,
}

/// The runner's size estimate of the request it is about to send.
#[derive(Clone, Debug, PartialEq, Eq, Deserialize, Serialize)]
#[non_exhaustive]
pub struct Estimate {
    /// Estimated input tokens of the request about to be sent. An estimate:
    /// most runners have no tokenizer for most model families.
    pub request_tokens: u64,
    /// The previous step's measured input tokens, when there was one.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub previous_input: Option<u64>,
}

impl Estimate {
    pub fn new(request_tokens: u64) -> Self {
        Self {
            request_tokens,
            previous_input: None,
        }
    }

    pub fn with_previous_input(mut self, input: u64) -> Self {
        self.previous_input = Some(input);
        self
    }
}

/// Set when this step's prefix is being rebuilt for another reason, so a
/// compaction on this step costs no extra prompt-cache miss.
#[derive(Clone, Debug, PartialEq, Eq, Deserialize, Serialize)]
pub struct PrefixRebuild {
    /// Why, as an open string: `flush`, `manifest_change`, `model_switch`
    /// or `expired_cache` ([`rebuild_reasons`]).
    pub reason: String,
}

/// The values [`PrefixRebuild::reason`] takes today.
pub mod rebuild_reasons {
    pub const FLUSH: &str = "flush";
    pub const MANIFEST_CHANGE: &str = "manifest_change";
    pub const MODEL_SWITCH: &str = "model_switch";
    pub const EXPIRED_CACHE: &str = "expired_cache";
}

/// The kinds of step, as [`StepStatus::step_kind`] spells them.
pub mod step_kinds {
    /// The step that answers a new user turn.
    pub const USER_TURN: &str = "user_turn";
    /// A step that follows tool results within a turn.
    pub const TOOL_STEP: &str = "tool_step";
}

/// A CompactionMessage by its id and version.
#[derive(Clone, Debug, PartialEq, Eq, Deserialize, Serialize)]
pub struct AppliedRef {
    pub compaction_id: String,
    pub version: u64,
}

/// The newest CompactionMessage the runner recorded and did not apply, and
/// why: a [`crate::fence::NotApplied::name`], or [`STRUCTURAL`] for the
/// runner's own checks. `reason` is decoded open.
#[derive(Clone, Debug, PartialEq, Eq, Deserialize, Serialize)]
pub struct NotAppliedRef {
    pub compaction_id: String,
    pub version: u64,
    pub reason: String,
}

impl NotAppliedRef {
    pub fn new(compaction_id: impl Into<String>, version: u64, reason: impl Into<String>) -> Self {
        Self {
            compaction_id: compaction_id.into(),
            version,
            reason: reason.into(),
        }
    }
}

/// The `last_not_applied.reason` a runner writes when its own structural
/// checks (roles, tool-call pairing, its validity rules) fail a
/// CompactionMessage that passed the fence.
pub const STRUCTURAL: &str = "structural";

/// The byte cap on a status's `messages` a runner uses unless it is
/// configured otherwise: 4 MiB, measured as the sum of each entry's compact
/// JSON encoding ([`message_bytes`]).
pub const DEFAULT_MESSAGE_CAP_BYTES: usize = 4 * 1024 * 1024;

/// The size a message counts against the cap: the length of its compact
/// JSON encoding as a status entry `{ordinal, mid, message}`.
pub fn message_bytes(message: &StatusMessage) -> usize {
    serde_json::to_vec(message).map_or(0, |bytes| bytes.len())
}

/// How many of `pending` (the messages after the cursor, oldest first) one
/// status carries under `cap_bytes`, and whether more remain (`more`).
/// Messages are taken in order while their total stays within the cap. The
/// first message is always taken, so a single message over the cap is sent
/// alone and the cursor still moves past it; a later message that would
/// overflow the cap waits for the next status.
pub fn take_capped(pending: &[StatusMessage], cap_bytes: usize) -> (usize, bool) {
    let mut used = 0usize;
    let mut count = 0usize;
    for message in pending {
        let size = message_bytes(message);
        if count > 0 && used.saturating_add(size) > cap_bytes {
            break;
        }
        used = used.saturating_add(size);
        count += 1;
    }
    (count, count < pending.len())
}

/// The `compaction.step` request.
///
/// Non-exhaustive so a later optional member is additive: decode one, or
/// build one with [`StepStatus::new`] and the `with_*` setters.
#[derive(Clone, Debug, PartialEq, Deserialize, Serialize)]
#[non_exhaustive]
pub struct StepStatus {
    /// The runner's opaque name for the session. The provider keys its own
    /// per-session state on it and echoes it verbatim in `compaction.ready`;
    /// it never uses it to choose what it returns.
    pub session: String,
    /// The harness named in the session's key, which identifies the caller
    /// (for example `broca`). This differs from the route's bind harness,
    /// which a runner binds as `runner`. Required: a request without it does
    /// not decode. A provider keys a runner conversation on
    /// `(project_root, session, harness)`.
    pub harness: String,
    /// This request's id, an opaque string the provider compares only for
    /// equality. Every answer names it, and `compaction.ready` echoes it.
    pub request_id: String,
    /// The session's current lineage. Ordinals, the cursor and ranges are
    /// all in this lineage.
    pub lineage_id: String,
    /// The step this status is for, opaque.
    pub step_id: String,
    /// `user_turn` or `tool_step` ([`step_kinds`]), decoded open.
    pub step_kind: String,
    /// The model the step is for.
    pub model: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub variant: Option<String>,
    /// The model's context window in tokens, when the runner's model catalog
    /// resolves it.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub context_window: Option<u64>,
    /// The model's output limit in tokens, when resolved.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub output_limit: Option<u64>,
    /// The previous step's usage, when it recorded any.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub previous_usage: Option<Usage>,
    /// When the previous run ended `error`, its `provider_code`. An overflow
    /// ends a run that way with no usage, so this can come without
    /// `previous_usage`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub previous_provider_code: Option<String>,
    pub estimate: Estimate,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub prefix_rebuilding: Option<PrefixRebuild>,
    /// The newest message written in the lineage; absent when it has none.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub newest: Option<MessageRef>,
    /// The last applied CompactionMessage. Absent only before Setup's is
    /// recorded, which never happens on a step call.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub last_applied: Option<AppliedRef>,
    /// The newest CompactionMessage recorded and not applied, if it is newer
    /// than the last applied.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub last_not_applied: Option<NotAppliedRef>,
    /// The cursor: the last ordinal of this lineage the provider was sent.
    /// Absent on the first call in the lineage.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub after_ordinal: Option<u64>,
    /// Every message after the cursor, oldest first, contiguous from it.
    pub messages: Vec<StatusMessage>,
    /// `true` when the runner stopped `messages` at its byte cap before the
    /// newest message. The cursor then advances only to the last message
    /// sent, and the next status carries the rest.
    #[serde(default, skip_serializing_if = "is_false")]
    pub more: bool,
    /// The runner's clock, in milliseconds since the Unix epoch.
    pub now: u64,
}

fn is_false(value: &bool) -> bool {
    !*value
}

impl StepStatus {
    /// A status with the required members and nothing else.
    #[allow(clippy::too_many_arguments)]
    pub fn new(
        session: impl Into<String>,
        harness: impl Into<String>,
        request_id: impl Into<String>,
        lineage_id: impl Into<String>,
        step_id: impl Into<String>,
        step_kind: impl Into<String>,
        model: impl Into<String>,
        estimate: Estimate,
        now: u64,
    ) -> Self {
        Self {
            session: session.into(),
            harness: harness.into(),
            request_id: request_id.into(),
            lineage_id: lineage_id.into(),
            step_id: step_id.into(),
            step_kind: step_kind.into(),
            model: model.into(),
            variant: None,
            context_window: None,
            output_limit: None,
            previous_usage: None,
            previous_provider_code: None,
            estimate,
            prefix_rebuilding: None,
            newest: None,
            last_applied: None,
            last_not_applied: None,
            after_ordinal: None,
            messages: Vec::new(),
            more: false,
            now,
        }
    }

    /// Set the cursor and the messages after it.
    pub fn with_messages(
        mut self,
        after_ordinal: Option<u64>,
        messages: Vec<StatusMessage>,
    ) -> Self {
        self.after_ordinal = after_ordinal;
        self.messages = messages;
        self
    }

    /// Set the last applied CompactionMessage.
    pub fn with_last_applied(mut self, compaction_id: impl Into<String>, version: u64) -> Self {
        self.last_applied = Some(AppliedRef {
            compaction_id: compaction_id.into(),
            version,
        });
        self
    }

    pub fn with_model_details(
        mut self,
        variant: Option<String>,
        context_window: Option<u64>,
        output_limit: Option<u64>,
    ) -> Self {
        self.variant = variant;
        self.context_window = context_window;
        self.output_limit = output_limit;
        self
    }

    pub fn with_previous(mut self, usage: Option<Usage>, code: Option<String>) -> Self {
        self.previous_usage = usage;
        self.previous_provider_code = code;
        self
    }

    pub fn with_prefix_rebuilding(mut self, reason: impl Into<String>) -> Self {
        self.prefix_rebuilding = Some(PrefixRebuild {
            reason: reason.into(),
        });
        self
    }

    pub fn with_newest(mut self, newest: MessageRef) -> Self {
        self.newest = Some(newest);
        self
    }

    pub fn with_not_applied(mut self, reference: NotAppliedRef) -> Self {
        self.last_not_applied = Some(reference);
        self
    }

    pub fn with_more(mut self, more: bool) -> Self {
        self.more = more;
        self
    }

    /// Set the cursor and as many of `pending` as fit under `cap_bytes`
    /// ([`take_capped`]), with `more` when some are left for the next
    /// status.
    pub fn with_capped_messages(
        self,
        after_ordinal: Option<u64>,
        pending: &[StatusMessage],
        cap_bytes: usize,
    ) -> Self {
        let (count, more) = take_capped(pending, cap_bytes);
        self.with_messages(after_ordinal, pending[..count].to_vec())
            .with_more(more)
    }

    /// Where the cursor stands once this status's answer is durable: the
    /// last message sent, or the old cursor when none was sent. A capped
    /// status advances it only to its last message, never to `newest`.
    pub fn next_cursor(&self) -> Option<u64> {
        self.messages
            .last()
            .map(|message| message.ordinal)
            .or(self.after_ordinal)
    }

    /// Check what a provider relies on: the messages are oldest first,
    /// strictly after the cursor, and none is newer than `newest`.
    pub fn check_messages(&self) -> Result<(), StatusProblem> {
        let mut previous = self.after_ordinal;
        for message in &self.messages {
            if previous.is_some_and(|p| message.ordinal <= p) {
                return Err(StatusProblem::MessagesOutOfOrder);
            }
            previous = Some(message.ordinal);
        }
        let newest = self.newest.as_ref().map(|n| n.ordinal);
        if previous.is_some() && previous > newest && !self.messages.is_empty() {
            return Err(StatusProblem::MessageAfterNewest);
        }
        Ok(())
    }

    /// The lowest version the provider's next CompactionMessage may carry:
    /// one above the higher of the last applied version and `own`, the
    /// provider's own counter (`None` when it lost it).
    pub fn next_version(&self, own: Option<u64>) -> u64 {
        let applied = self.last_applied.as_ref().map_or(0, |a| a.version);
        applied.max(own.unwrap_or(0)).saturating_add(1)
    }
}

/// Why a status breaks what a provider relies on.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum StatusProblem {
    /// A message is not strictly after the cursor or the one before it.
    MessagesOutOfOrder,
    /// A message is newer than `newest`.
    MessageAfterNewest,
}

impl StatusProblem {
    pub fn name(self) -> &'static str {
        match self {
            Self::MessagesOutOfOrder => "messages_out_of_order",
            Self::MessageAfterNewest => "message_after_newest",
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::vectors;

    #[test]
    fn status_vectors_round_trip() {
        let file = vectors::load("status.json");
        for case in vectors::cases(&file, "requests") {
            let name = case["name"].as_str().unwrap();
            let status = vectors::round_trip::<StepStatus>(name, &case["request"]);
            assert_eq!(status.check_messages(), Ok(()), "{name}");
            assert_eq!(
                status.next_version(case["provider_counter"].as_u64()),
                case["next_version"].as_u64().unwrap(),
                "{name}: next version"
            );
        }
        for case in vectors::cases(&file, "tolerated") {
            let name = case["name"].as_str().unwrap();
            serde_json::from_value::<StepStatus>(case["request"].clone())
                .unwrap_or_else(|e| panic!("{name}: {e}"));
        }
        for case in vectors::cases(&file, "undecodable") {
            vectors::refused::<StepStatus>(case["name"].as_str().unwrap(), &case["request"]);
        }
        for case in vectors::cases(&file, "inconsistent") {
            let name = case["name"].as_str().unwrap();
            let status: StepStatus = serde_json::from_value(case["request"].clone()).unwrap();
            assert_eq!(
                status.check_messages().map_err(StatusProblem::name),
                Err(case["problem"].as_str().unwrap()),
                "{name}"
            );
        }
    }

    #[test]
    fn builders_match_decoding() {
        let status = StepStatus::new(
            "s",
            "broca",
            "r1",
            "l1",
            "step-1",
            step_kinds::USER_TURN,
            "m",
            Estimate {
                request_tokens: 10,
                previous_input: None,
            },
            1,
        )
        .with_last_applied("c1", 3)
        .with_messages(
            Some(4),
            vec![StatusMessage {
                ordinal: 5,
                mid: "m5".into(),
                message: serde_json::json!({}),
            }],
        );
        let encoded = serde_json::to_value(&status).unwrap();
        assert_eq!(
            serde_json::from_value::<StepStatus>(encoded).unwrap(),
            status
        );
        assert_eq!(status.next_version(None), 4);
        assert_eq!(status.next_version(Some(7)), 8);
    }

    fn message(ordinal: u64, text_len: usize) -> StatusMessage {
        StatusMessage {
            ordinal,
            mid: format!("m{ordinal}"),
            message: serde_json::json!({"role": "user", "text": "x".repeat(text_len)}),
        }
    }

    fn status() -> StepStatus {
        StepStatus::new(
            "s",
            "broca",
            "r",
            "l",
            "st",
            step_kinds::USER_TURN,
            "m",
            Estimate::new(1),
            1,
        )
        .with_newest(MessageRef {
            ordinal: 14,
            mid: "m14".into(),
        })
    }

    /// The cap: messages go in order while they fit; a single message over
    /// the cap is sent alone; the cursor advances only to the last message
    /// sent, and `more` says the rest follows.
    #[test]
    fn capped_messages_send_an_oversized_one_alone_and_advance_to_it() {
        let pending = vec![
            message(11, 10),
            message(12, 10),
            message(13, 500),
            message(14, 10),
        ];
        let small = message_bytes(&pending[0]);
        let cap = 2 * small + 1;
        assert!(message_bytes(&pending[2]) > cap);

        // The two small messages fit; the oversized one does not join them.
        let first = status().with_capped_messages(Some(10), &pending, cap);
        assert_eq!(first.messages, pending[..2]);
        assert!(first.more);
        assert_eq!(first.next_cursor(), Some(12));

        // Next status: the oversized message is first, so it goes alone.
        let second = status().with_capped_messages(first.next_cursor(), &pending[2..], cap);
        assert_eq!(second.messages, pending[2..3]);
        assert!(second.more);
        assert_eq!(second.next_cursor(), Some(13));
        assert_eq!(second.check_messages(), Ok(()));

        // The rest fits, and nothing more follows.
        let third = status().with_capped_messages(second.next_cursor(), &pending[3..], cap);
        assert_eq!(third.messages, pending[3..]);
        assert!(!third.more);
        assert_eq!(third.next_cursor(), Some(14));

        // An oversized message at the head of the queue is sent alone even
        // when nothing else is pending, and `more` stays false.
        let alone = status().with_capped_messages(Some(12), &pending[2..3], cap);
        assert_eq!((alone.messages.len(), alone.more), (1, false));
        assert_eq!(alone.next_cursor(), Some(13));

        // No messages: the cursor stays where it was.
        let empty = status().with_capped_messages(Some(14), &[], cap);
        assert_eq!((empty.next_cursor(), empty.more), (Some(14), false));
        assert_eq!(DEFAULT_MESSAGE_CAP_BYTES, 4_194_304);
        assert_eq!(take_capped(&pending, DEFAULT_MESSAGE_CAP_BYTES), (4, false));
    }

    #[test]
    fn more_and_last_not_applied_round_trip() {
        let status = status()
            .with_messages(Some(10), vec![message(11, 1)])
            .with_more(true)
            .with_not_applied(NotAppliedRef::new("mc:snap-9", 9, STRUCTURAL));
        let wire = serde_json::to_value(&status).unwrap();
        assert_eq!(wire["more"], true);
        assert_eq!(
            wire["last_not_applied"],
            serde_json::json!({"compaction_id": "mc:snap-9", "version": 9, "reason": "structural"})
        );
        assert_eq!(vectors::round_trip::<StepStatus>("more", &wire), status);
        // `more` is omitted when false.
        let plain = serde_json::to_value(status.with_more(false)).unwrap();
        assert!(plain.get("more").is_none());
    }
}
