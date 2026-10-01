//! `compaction.setup`: called once per session, before its first model
//! call.
//!
//! Setup returns the initial CompactionMessage (how a session starts with
//! its head already in place; a provider with no head returns an empty
//! replacement), the stability rank of each of the provider's own messages,
//! and `call_when`, the conditions for calling the provider. The runner
//! records the answer before the first model call; once it is recorded,
//! Setup never runs again for the session.

use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};
use serde_json::{Map, Value};

use crate::{answer::CompactionMessage, status::MessageRef};

/// The `compaction.setup` request. Decoded leniently.
///
/// Non-exhaustive so a later optional member is additive: decode one, or
/// build one with [`SetupRequest::new`] and the `with_*` setters.
#[derive(Clone, Debug, PartialEq, Deserialize, Serialize)]
#[non_exhaustive]
pub struct SetupRequest {
    /// The runner's opaque name for the session, as every later status
    /// spells it.
    pub session: String,
    /// This request's id; the answer names it.
    pub request_id: String,
    /// The session's lineage, absent when nothing has been written yet.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub lineage_id: Option<String>,
    /// The compaction item's preset, verbatim from the plan. Absent means
    /// the provider's default.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub preset: Option<String>,
    /// The compaction item's params, verbatim from the plan.
    #[serde(default)]
    pub params: Map<String, Value>,
    /// The session's composition, verbatim from the plan; opaque.
    pub composition: Map<String, Value>,
    /// The model of the session's first step.
    pub model: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub variant: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub context_window: Option<u64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub output_limit: Option<u64>,
    /// The newest message already written, if any (an imported session, or
    /// the first user message when the runner writes it before Setup).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub newest: Option<MessageRef>,
    /// The runner's clock, in milliseconds since the Unix epoch.
    pub now: u64,
}

impl SetupRequest {
    pub fn new(
        session: impl Into<String>,
        request_id: impl Into<String>,
        composition: Map<String, Value>,
        model: impl Into<String>,
        now: u64,
    ) -> Self {
        Self {
            session: session.into(),
            request_id: request_id.into(),
            lineage_id: None,
            preset: None,
            params: Map::new(),
            composition,
            model: model.into(),
            variant: None,
            context_window: None,
            output_limit: None,
            newest: None,
            now,
        }
    }

    /// Set the plan item's preset and params.
    pub fn with_item(mut self, preset: Option<String>, params: Map<String, Value>) -> Self {
        self.preset = preset;
        self.params = params;
        self
    }
}

/// The stability rank of one of the provider's own messages: `index` is the
/// position in every CompactionMessage's `replacement`, and a higher `rank`
/// means the message changes less often. The runner places cache
/// breakpoints and judges change policies on these segments.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Deserialize, Serialize)]
pub struct StabilityRank {
    pub index: u32,
    pub rank: u32,
}

/// When the runner calls the provider, frozen with the session. The runner
/// calls when the step's reported usage reaches the model's share of the
/// window: the share in `models` whose key matches the model, else
/// `default`. It also always calls on a prefix rebuild and after an
/// execution error. With no `call_when` it calls on every step.
#[derive(Clone, Debug, PartialEq, Deserialize, Serialize)]
pub struct CallWhen {
    /// The share of the context window, above 0 and at most 1.
    pub default: f64,
    /// Per-model overrides, keyed by a model id or pattern.
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub models: BTreeMap<String, f64>,
    /// Condition kinds this crate does not know. New kinds are additive,
    /// and a runner that meets one it does not know calls on every step
    /// rather than guess ([`CallWhen::has_unknown_conditions`]).
    #[serde(flatten)]
    pub unknown: Map<String, Value>,
}

impl CallWhen {
    pub fn has_unknown_conditions(&self) -> bool {
        !self.unknown.is_empty()
    }

    /// Whether every share is above 0 and at most 1.
    pub fn shares_in_range(&self) -> bool {
        std::iter::once(&self.default)
            .chain(self.models.values())
            .all(|share| *share > 0.0 && *share <= 1.0)
    }
}

/// The `compaction.setup` answer. Strict on `answer`, like a step answer.
#[derive(Clone, Debug, PartialEq, Deserialize, Serialize)]
#[serde(tag = "answer", rename_all = "snake_case")]
pub enum SetupAnswer {
    /// The session's initial view and declarations.
    Ready {
        request_id: String,
        /// The initial CompactionMessage. Its range is usually empty at the
        /// start of the lineage (`{from: 0, to: 0}`): the head goes before
        /// every message.
        initial: CompactionMessage,
        /// Ranks for the provider's own messages; empty for a provider that
        /// declares none (a subagent session with no head, say).
        #[serde(default, skip_serializing_if = "Vec::is_empty")]
        stability: Vec<StabilityRank>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        call_when: Option<CallWhen>,
    },
    /// Setup failed. No model call is made, and the run ends `error` with
    /// `code` as its `provider_code`. The next send runs Setup again.
    Refuse {
        request_id: String,
        code: String,
        reason: String,
        retryable: bool,
    },
}

impl SetupAnswer {
    pub fn request_id(&self) -> &str {
        match self {
            Self::Ready { request_id, .. } | Self::Refuse { request_id, .. } => request_id,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::vectors;

    #[test]
    fn setup_vectors_round_trip() {
        let file = vectors::load("setup.json");
        for case in vectors::cases(&file, "requests") {
            vectors::round_trip::<SetupRequest>(case["name"].as_str().unwrap(), &case["request"]);
        }
        for case in vectors::cases(&file, "answers") {
            let name = case["name"].as_str().unwrap();
            let answer = vectors::round_trip::<SetupAnswer>(name, &case["answer"]);
            assert_eq!(answer.request_id(), case["answer"]["request_id"]);
            if let SetupAnswer::Ready {
                initial, call_when, ..
            } = &answer
            {
                assert_eq!(initial.check(), Ok(()), "{name}");
                if let Some(call_when) = call_when {
                    assert!(call_when.shares_in_range(), "{name}");
                    assert_eq!(
                        call_when.has_unknown_conditions(),
                        case["unknown_conditions"].as_bool().unwrap_or(false),
                        "{name}"
                    );
                }
            }
        }
        for case in vectors::cases(&file, "undecodable") {
            vectors::refused::<SetupAnswer>(case["name"].as_str().unwrap(), &case["answer"]);
        }
        for case in vectors::cases(&file, "shares_out_of_range") {
            let name = case["name"].as_str().unwrap();
            let call_when: CallWhen = serde_json::from_value(case["call_when"].clone()).unwrap();
            assert!(!call_when.shares_in_range(), "{name}");
        }
    }

    #[test]
    fn builders_match_decoding() {
        let mut params = Map::new();
        params.insert("budget".into(), Value::from(3));
        let request =
            SetupRequest::new("s", "r0", Map::new(), "m", 1).with_item(Some("head".into()), params);
        let encoded = serde_json::to_value(&request).unwrap();
        assert_eq!(
            serde_json::from_value::<SetupRequest>(encoded).unwrap(),
            request
        );
    }
}
