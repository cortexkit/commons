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
pub struct Estimate {
    /// Estimated input tokens of the request about to be sent. An estimate:
    /// most runners have no tokenizer for most model families.
    pub request_tokens: u64,
    /// The previous step's measured input tokens, when there was one.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub previous_input: Option<u64>,
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
/// why ([`crate::fence::NotApplied::name`], or `structural` for the
/// runner's own checks).
#[derive(Clone, Debug, PartialEq, Eq, Deserialize, Serialize)]
pub struct NotAppliedRef {
    pub compaction_id: String,
    pub version: u64,
    pub reason: String,
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
}
