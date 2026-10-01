//! The provider's answers: a CompactionMessage, and the four answers to a
//! per-step status (`noop`, `compaction_message`, `wait`, `refuse`).
//!
//! A CompactionMessage replaces one contiguous range of the transcript,
//! named by ordinals, with the provider's finished messages. It always
//! describes the whole working range as it stands, never a change against
//! an earlier CompactionMessage, so the runner applies the newest one alone.

use cortexkit_role_llm_runner::read::EntrySource;
use serde::{Deserialize, Serialize};
use serde_json::Value;

/// A half-open range of transcript ordinals, `from` included and `to`
/// excluded, in the lineage of the request the answer answers.
///
/// Messages before `from` are not sent. The replacement stands in for the
/// messages in `[from, to)`. Messages from `to` onward are sent as written.
/// `from == to` is an empty range: nothing is replaced, and the replacement
/// is inserted before `to`. `{from: 0, to: 0}` therefore puts the
/// replacement before every message of the lineage.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Deserialize, Serialize)]
pub struct Range {
    pub from: u64,
    pub to: u64,
}

impl Range {
    pub fn new(from: u64, to: u64) -> Self {
        Self { from, to }
    }

    pub fn is_empty(&self) -> bool {
        self.from == self.to
    }
}

/// One CompactionMessage: the provider's view of the working range.
#[derive(Clone, Debug, PartialEq, Deserialize, Serialize)]
pub struct CompactionMessage {
    /// The provider's own name for this content: an opaque string the
    /// runner never interprets. The runner echoes it back in the status and
    /// shows it in the model view.
    pub compaction_id: String,
    /// Rises with every CompactionMessage the provider sends for the
    /// session. The runner applies one only if its version is higher than
    /// the last applied.
    pub version: u64,
    /// The range the replacement stands in for.
    pub range: Range,
    /// The provider's finished messages, in the runner's message schema
    /// (the schema `session.read` pages carry). The runner does not run step
    /// transforms over them.
    pub replacement: Vec<Value>,
}

/// Why a CompactionMessage is malformed on its own, before any comparison
/// with the runner's state.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum MessageProblem {
    /// `range.from` is greater than `range.to`.
    RangeInverted,
}

impl MessageProblem {
    /// The problem's name as the vectors spell it.
    pub fn name(self) -> &'static str {
        match self {
            Self::RangeInverted => "range_inverted",
        }
    }
}

impl CompactionMessage {
    pub fn new(
        compaction_id: impl Into<String>,
        version: u64,
        range: Range,
        replacement: Vec<Value>,
    ) -> Self {
        Self {
            compaction_id: compaction_id.into(),
            version,
            range,
            replacement,
        }
    }

    /// Check the message on its own. The runner records a message that
    /// fails as not applied and keeps the previous one.
    pub fn check(&self) -> Result<(), MessageProblem> {
        if self.range.from > self.range.to {
            return Err(MessageProblem::RangeInverted);
        }
        Ok(())
    }

    /// The `source` the runner's model view (`llm-runner/v1`'s
    /// `EntrySource`) gives each replacement message of this
    /// CompactionMessage once it is applied: its id, its version and its
    /// half-open range, `from_ordinal` = `range.from` and `to_ordinal` =
    /// `range.to`. An empty range is an insertion: its messages go before
    /// the transcript message at `from_ordinal`, so Setup's usual head is
    /// `[0, 0)`. `None` for a message that fails
    /// [`CompactionMessage::check`], which the runner never applies.
    pub fn model_view_source(&self) -> Option<EntrySource> {
        self.check().ok()?;
        Some(EntrySource::Replacement {
            compaction_id: self.compaction_id.clone(),
            version: self.version,
            from_ordinal: self.range.from,
            to_ordinal: self.range.to,
        })
    }
}

/// The answer to a per-step status. Strict on `answer`: an unknown answer
/// does not decode, and the runner treats it as a failed call (it keeps
/// the last applied CompactionMessage), never as `noop`.
///
/// Every answer names the request it answers, so the runner can apply the
/// request fence ([`crate::fence`]).
#[derive(Clone, Debug, PartialEq, Deserialize, Serialize)]
#[serde(tag = "answer", rename_all = "snake_case")]
pub enum StepAnswer {
    /// Nothing changes. The runner records it, so the cursor advances past
    /// the messages the status carried.
    Noop { request_id: String },
    /// Replace the working range with this CompactionMessage.
    CompactionMessage {
        request_id: String,
        compaction: CompactionMessage,
    },
    /// Hold this step while the provider works, up to `bound_ms`. The
    /// provider sends `compaction.ready` when it is done.
    Wait {
        request_id: String,
        /// User-facing text: the runner shows "waiting on compaction:
        /// <reason>".
        reason: String,
        /// How long the provider expects to need, in milliseconds. The
        /// runner caps it with its own bounds.
        bound_ms: u64,
    },
    /// Fail this step. The run ends `error` with `code` as its
    /// `provider_code`, nothing is added to history, and the session stays
    /// usable.
    Refuse {
        request_id: String,
        /// The provider's code ([`crate::errors::refuse_codes`] lists the
        /// ones this role names; a provider may use its own).
        code: String,
        /// User-facing text.
        reason: String,
        /// Whether retrying without the user acting makes sense.
        retryable: bool,
    },
}

impl StepAnswer {
    /// The request this answer answers.
    pub fn request_id(&self) -> &str {
        match self {
            Self::Noop { request_id }
            | Self::CompactionMessage { request_id, .. }
            | Self::Wait { request_id, .. }
            | Self::Refuse { request_id, .. } => request_id,
        }
    }

    /// The answer's name as it appears in `answer`.
    pub fn name(&self) -> &'static str {
        match self {
            Self::Noop { .. } => "noop",
            Self::CompactionMessage { .. } => "compaction_message",
            Self::Wait { .. } => "wait",
            Self::Refuse { .. } => "refuse",
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::vectors;
    use cortexkit_role_llm_runner::read::{ModelEntry, ModelPage};

    #[test]
    fn answer_vectors_round_trip() {
        let file = vectors::load("answers.json");
        for case in vectors::cases(&file, "answers") {
            let name = case["name"].as_str().unwrap();
            let answer = vectors::round_trip::<StepAnswer>(name, &case["answer"]);
            assert_eq!(answer.name(), case["answer"]["answer"].as_str().unwrap());
            assert_eq!(answer.request_id(), case["answer"]["request_id"]);
            if let StepAnswer::CompactionMessage { compaction, .. } = &answer {
                assert_eq!(compaction.check(), Ok(()), "{name}");
            }
        }
        for case in vectors::cases(&file, "tolerated") {
            let name = case["name"].as_str().unwrap();
            serde_json::from_value::<StepAnswer>(case["answer"].clone())
                .unwrap_or_else(|e| panic!("{name}: {e}"));
        }
        for case in vectors::cases(&file, "undecodable") {
            vectors::refused::<StepAnswer>(case["name"].as_str().unwrap(), &case["answer"]);
        }
    }

    #[test]
    fn malformed_messages_are_named() {
        let file = vectors::load("answers.json");
        for case in vectors::cases(&file, "malformed_messages") {
            let name = case["name"].as_str().unwrap();
            let message: CompactionMessage = serde_json::from_value(case["compaction"].clone())
                .unwrap_or_else(|e| panic!("{name}: {e}"));
            assert_eq!(
                message.check().map_err(MessageProblem::name),
                Err(case["problem"].as_str().unwrap()),
                "{name}"
            );
        }
    }

    #[test]
    fn model_view_sources_match_the_vectors() {
        let file = vectors::load("answers.json");
        for case in vectors::cases(&file, "model_view") {
            let name = case["name"].as_str().unwrap();
            let message: CompactionMessage =
                serde_json::from_value(case["compaction"].clone()).unwrap();
            let source = message.model_view_source();
            match case.get("source") {
                Some(expected) if !expected.is_null() => {
                    let source = source.unwrap();
                    assert_eq!(
                        vectors::round_trip::<EntrySource>(name, expected),
                        source,
                        "{name}"
                    );
                    // The model view treats a source as an insertion exactly
                    // when the CompactionMessage's range is empty.
                    assert_eq!(source.is_insertion(), message.range.is_empty(), "{name}");
                    // The runner's own page check accepts the entry.
                    let page = ModelPage::new("lin", vec![ModelEntry::new(source, Value::Null)])
                        .with_compaction(message.compaction_id.clone(), message.version);
                    assert_eq!(page.check(None), Ok(()), "{name}");
                }
                _ => assert!(source.is_none(), "{name}"),
            }
        }
    }
}
