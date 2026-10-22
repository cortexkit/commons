//! The CompactionMessage: the initial view Setup returns.
//!
//! A CompactionMessage replaces one contiguous range of the transcript,
//! named by ordinals, with the provider's finished messages. Setup's
//! `initial` is one; the runner renders it until it applies the session's
//! first view (CONTRACT.md §6), which replaces it entirely.

use cortexkit_role_llm_runner::read::EntrySource;
use serde::{Deserialize, Serialize};
use serde_json::Value;

/// A half-open range of transcript ordinals, `from` included and `to`
/// excluded, in the session's lineage.
///
/// Messages before `from` are not sent. The replacement stands in for the
/// messages in `[from, to)`. Messages from `to` onward are sent as written.
/// `from == to` is an empty range: nothing is replaced, and the replacement
/// is inserted before `to`. `{from: f, to: f}`, with `f` the lineage's
/// first ordinal (0 before the session's first cut), therefore puts the
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

/// One CompactionMessage.
#[derive(Clone, Debug, PartialEq, Deserialize, Serialize)]
pub struct CompactionMessage {
    /// The provider's own name for this content: an opaque string the
    /// runner never interprets. The runner shows it in the model view.
    pub compaction_id: String,
    /// The provider's version of this content.
    pub version: u64,
    /// The range the replacement stands in for.
    pub range: Range,
    /// The provider's finished messages, in the runner's message schema
    /// (the schema `session.read` pages carry). The runner does not run step
    /// transforms over them.
    pub replacement: Vec<Value>,
}

/// Why a CompactionMessage is malformed on its own.
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

    /// Check the message on its own. A runner treats a Setup answer whose
    /// `initial` fails as a failed Setup.
    pub fn check(&self) -> Result<(), MessageProblem> {
        if self.range.from > self.range.to {
            return Err(MessageProblem::RangeInverted);
        }
        Ok(())
    }

    /// The `source` the runner's model view (`llm-runner/v1`'s
    /// `EntrySource`) gives each replacement message of this
    /// CompactionMessage while it is rendered: its id, its version and its
    /// half-open range, `from_ordinal` = `range.from` and `to_ordinal` =
    /// `range.to`. An empty range is an insertion: its messages go before
    /// the transcript message at `from_ordinal`, so Setup's usual head is
    /// `[0, 0)`. `None` for a message that fails
    /// [`CompactionMessage::check`], which the runner never renders.
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::vectors;
    use cortexkit_role_llm_runner::read::{ModelEntry, ModelPage};

    #[test]
    fn malformed_messages_are_named() {
        let file = vectors::load("setup.json");
        for case in vectors::cases(&file, "malformed_initial") {
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
        let file = vectors::load("setup.json");
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
