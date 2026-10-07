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

use crate::{answer::CompactionMessage, errors::RefuseCode, status::MessageRef};

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
    /// The harness named in the session's key, which identifies the caller
    /// (for example `broca`). This differs from the route's bind harness,
    /// which a runner binds as `runner`. Required: a request without it does
    /// not decode. A provider keys a runner conversation on
    /// `(project_root, session, harness)`.
    pub harness: String,
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
        harness: impl Into<String>,
        request_id: impl Into<String>,
        composition: Map<String, Value>,
        model: impl Into<String>,
        now: u64,
    ) -> Self {
        Self {
            session: session.into(),
            harness: harness.into(),
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

    pub fn with_lineage(mut self, lineage_id: impl Into<String>) -> Self {
        self.lineage_id = Some(lineage_id.into());
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

    pub fn with_newest(mut self, newest: MessageRef) -> Self {
        self.newest = Some(newest);
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
/// window ([`CallWhen::share_for`]). It also always calls on a prefix
/// rebuild and after an execution error. With no `call_when`, or with a
/// condition kind it does not know, it calls on every step
/// ([`calls_every_step`]).
#[derive(Clone, Debug, PartialEq, Deserialize, Serialize)]
#[non_exhaustive]
pub struct CallWhen {
    /// The share of the context window, above 0 and at most 1.
    pub default: f64,
    /// Per-model overrides, keyed by an exact model id or by a pattern that
    /// ends in one `*` and matches every model id starting with the text
    /// before it.
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub models: BTreeMap<String, f64>,
    /// Condition kinds this crate does not know. New kinds are additive,
    /// and a runner that meets one it does not know calls on every step
    /// rather than guess ([`CallWhen::has_unknown_conditions`]).
    #[serde(flatten)]
    pub unknown: Map<String, Value>,
}

impl CallWhen {
    pub fn new(default: f64) -> Self {
        Self {
            default,
            models: BTreeMap::new(),
            unknown: Map::new(),
        }
    }

    pub fn with_models(mut self, models: BTreeMap<String, f64>) -> Self {
        self.models = models;
        self
    }

    pub fn with_unknown_conditions(mut self, unknown: Map<String, Value>) -> Self {
        self.unknown = unknown;
        self
    }

    pub fn has_unknown_conditions(&self) -> bool {
        !self.unknown.is_empty()
    }

    /// The share of the window that applies to `model`. Matching is in a
    /// fixed order: a key equal to `model`; else the trailing-`*` key with
    /// the longest prefix that `model` starts with; else `default`. A `*`
    /// anywhere but at the end of a key is literal, so such a key matches
    /// only exactly.
    pub fn share_for(&self, model: &str) -> f64 {
        if let Some(share) = self.models.get(model) {
            return *share;
        }
        self.models
            .iter()
            .filter_map(|(key, share)| {
                let prefix = key.strip_suffix('*')?;
                model.starts_with(prefix).then_some((prefix.len(), *share))
            })
            .max_by_key(|(length, _)| *length)
            .map_or(self.default, |(_, share)| share)
    }

    /// Whether every share is above 0 and at most 1.
    pub fn shares_in_range(&self) -> bool {
        std::iter::once(&self.default)
            .chain(self.models.values())
            .all(|share| *share > 0.0 && *share <= 1.0)
    }
}

/// Whether the runner calls the provider on every step regardless of
/// usage: when Setup declared no `call_when`, or declared a condition kind
/// the runner does not know. A runner never skips a call because it could
/// not read a condition.
pub fn calls_every_step(call_when: Option<&CallWhen>) -> bool {
    call_when.is_none_or(CallWhen::has_unknown_conditions)
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
    /// the role's `code` and, when present, the provider's own
    /// `provider_code` beside it. The next send runs Setup again. A
    /// `retryable` member an older provider still sends is ignored: `code`
    /// fixes retryability.
    Refuse {
        request_id: String,
        code: RefuseCode,
        reason: String,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        provider_code: Option<String>,
    },
}

impl SetupAnswer {
    pub fn request_id(&self) -> &str {
        match self {
            Self::Ready { request_id, .. } | Self::Refuse { request_id, .. } => request_id,
        }
    }

    /// Whether a `refuse` may be retried; `None` for `ready`. Its
    /// retryability is fixed by the code: `true` means the runner may
    /// retry without the user acting.
    pub fn retryable(&self) -> Option<bool> {
        match self {
            Self::Refuse { code, .. } => Some(code.retryable()),
            Self::Ready { .. } => None,
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
            if let Some(retryable) = case.get("retryable") {
                assert_eq!(answer.retryable(), retryable.as_bool(), "{name}");
            }
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
        for case in vectors::cases(&file, "tolerated") {
            let name = case["name"].as_str().unwrap();
            let answer: SetupAnswer = serde_json::from_value(case["answer"].clone())
                .unwrap_or_else(|e| panic!("{name}: {e}"));
            assert_eq!(answer.retryable(), case["retryable"].as_bool(), "{name}");
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
        let request = SetupRequest::new("s", "broca", "r0", Map::new(), "m", 1)
            .with_item(Some("head".into()), params);
        let encoded = serde_json::to_value(&request).unwrap();
        assert_eq!(
            serde_json::from_value::<SetupRequest>(encoded).unwrap(),
            request
        );
    }

    #[test]
    fn every_provider_refuse_answer_round_trips() {
        for code in crate::errors::refuse_codes::ALL {
            for provider_code in [None, Some("mc:historian_offline")] {
                let mut answer = serde_json::json!({
                    "answer": "refuse", "request_id": "r0", "code": code,
                    "reason": "cannot compact"
                });
                if let Some(own) = provider_code {
                    answer["provider_code"] = Value::from(own);
                }
                let setup = vectors::round_trip::<SetupAnswer>(code, &answer);
                let step = vectors::round_trip::<crate::answer::StepAnswer>(code, &answer);
                let fixed = crate::errors::refuse_codes::retryable(code);
                assert_eq!(setup.retryable(), Some(fixed), "{code}");
                assert_eq!(step.retryable(), Some(fixed), "{code}");
            }
        }
    }

    /// An older provider still sends `retryable`. It decodes, the member is
    /// dropped, and it never decides retry: `code` alone does.
    #[test]
    fn a_stale_retryable_member_is_ignored() {
        for (code, sent, fixed) in [
            ("misconfigured", true, false),
            ("provider_busy", false, true),
            ("acme_own_code", true, false),
        ] {
            let answer = serde_json::json!({
                "answer": "refuse", "request_id": "r0", "code": code,
                "reason": "x", "retryable": sent
            });
            let setup: SetupAnswer = serde_json::from_value(answer.clone()).unwrap();
            let step: crate::answer::StepAnswer = serde_json::from_value(answer).unwrap();
            assert_eq!(setup.retryable(), Some(fixed), "{code}");
            assert_eq!(step.retryable(), Some(fixed), "{code}");
            assert!(serde_json::to_value(&setup)
                .unwrap()
                .get("retryable")
                .is_none());
        }
    }

    /// Model matching: an exact key wins over any pattern, the longest
    /// matching trailing-`*` pattern wins over shorter ones, and `default`
    /// applies only when nothing matches.
    #[test]
    fn model_matching_is_exact_then_longest_star_then_default() {
        let file = vectors::load("setup.json");
        for case in vectors::cases(&file, "model_matching") {
            let name = case["name"].as_str().unwrap();
            let call_when: CallWhen = serde_json::from_value(case["call_when"].clone())
                .unwrap_or_else(|e| panic!("{name}: {e}"));
            for expected in case["expected"].as_array().unwrap() {
                let model = expected["model"].as_str().unwrap();
                assert_eq!(
                    call_when.share_for(model),
                    expected["share"].as_f64().unwrap(),
                    "{name}: {model}"
                );
            }
        }
        let call_when = CallWhen::new(0.8).with_models(BTreeMap::from([
            ("anthropic/*".to_owned(), 0.6),
            ("anthropic/claude-*".to_owned(), 0.65),
            ("anthropic/claude-opus-4-5".to_owned(), 0.7),
        ]));
        assert_eq!(call_when.share_for("anthropic/claude-opus-4-5"), 0.7);
        assert_eq!(call_when.share_for("anthropic/claude-sonnet-4"), 0.65);
        assert_eq!(call_when.share_for("anthropic/other"), 0.6);
        assert_eq!(call_when.share_for("openai/gpt-5"), 0.8);
    }

    #[test]
    fn no_call_when_or_an_unknown_condition_calls_every_step() {
        assert!(calls_every_step(None));
        assert!(!calls_every_step(Some(&CallWhen::new(0.8))));
        let mut unknown = Map::new();
        unknown.insert("every_n_steps".into(), Value::from(10));
        let call_when = CallWhen::new(0.8).with_unknown_conditions(unknown);
        assert!(calls_every_step(Some(&call_when)));
    }
}
