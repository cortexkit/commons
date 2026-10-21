//! `compaction.setup`: called once per session, before its first model
//! call. The provider takes `project_root` from the incoming route; together
//! with Setup's `harness` and `session`, it uses these values as the identity
//! for later reverse calls.
//!
//! Setup returns the initial CompactionMessage, which the runner renders
//! until it applies the session's first view (a provider with no head
//! returns an empty replacement), and optionally the ceiling policy. The
//! runner records the answer before the first model call; once it is
//! recorded, Setup never runs again for the session.

use serde::{Deserialize, Serialize};
use serde_json::{Map, Value};

use crate::{
    ceiling::CeilingPolicy,
    errors::RefuseCode,
    lineage::{DescendsFrom, MessageRef},
    message::CompactionMessage,
};

/// The `compaction.setup` request. Decoded leniently.
///
/// Non-exhaustive so a later optional member is additive: decode one, or
/// build one with [`SetupRequest::new`] and the `with_*` setters.
#[derive(Clone, Debug, PartialEq, Deserialize, Serialize)]
#[non_exhaustive]
pub struct SetupRequest {
    /// The runner's opaque name for the session. If the incoming route binds
    /// a non-empty session that differs, the provider may refuse Setup with
    /// `setup_identity_mismatch`; a shared route has an empty bind session.
    /// `project_root` is not carried here and comes only from the route's bind
    /// identity.
    pub session: String,
    /// The caller-chosen harness in the session identity. The daemon does not
    /// attest this string, and a shared project route may carry a different
    /// bind harness. Required; a request without it does not decode.
    pub harness: String,
    /// This request's id; the answer names it.
    pub request_id: String,
    /// The session's lineage, absent when nothing has been written yet.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub lineage_id: Option<String>,
    /// Optional inherited prefix: the session's first lineage continues
    /// another lineage of the same conversation through an inclusive cut.
    /// Sent only to a provider whose `role.describe` declares
    /// `setup_descends_from`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub descends_from: Option<DescendsFrom>,
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
            descends_from: None,
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

    pub fn with_descends_from(mut self, descends_from: DescendsFrom) -> Self {
        self.descends_from = Some(descends_from);
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

/// The `compaction.setup` answer. Strict on `answer`: a runner treats an
/// answer it cannot decode as a failed Setup.
#[derive(Clone, Debug, PartialEq, Deserialize, Serialize)]
#[serde(tag = "answer", rename_all = "snake_case")]
pub enum SetupAnswer {
    /// The session's initial view and ceiling policy.
    Ready {
        request_id: String,
        /// The initial CompactionMessage. Its range is usually empty at the
        /// start of the lineage (`{from: 0, to: 0}`): the head goes before
        /// every message. The runner renders it until it applies the
        /// session's first view.
        initial: CompactionMessage,
        /// When the runner pauses for a view and, failing one, refuses the
        /// run (CONTRACT.md §8). Absent, the runner never pauses for the
        /// provider but still never sends over the window.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        ceiling: Option<CeilingPolicy>,
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
    fn setup_descent_round_trips_without_dropping_the_field() {
        let file = vectors::load("setup-descent.json");
        let case = &vectors::cases(&file, "requests")[1];
        let request =
            vectors::round_trip::<SetupRequest>(case["name"].as_str().unwrap(), &case["request"]);
        assert_eq!(request.descends_from, Some(DescendsFrom::new("parent", 2)));
        let built = SetupRequest::new("s", "opencode", "setup", Map::new(), "m", 1)
            .with_lineage("child")
            .with_descends_from(DescendsFrom::new("parent", 2));
        assert_eq!(built, request);
    }

    #[test]
    fn setup_descent_absence_and_lenient_decode_are_unchanged() {
        let file = vectors::load("setup-descent.json");
        let case = &vectors::cases(&file, "requests")[0];
        let request =
            vectors::round_trip::<SetupRequest>(case["name"].as_str().unwrap(), &case["request"]);
        assert_eq!(request.descends_from, None);
        let mut extended = case["request"].clone();
        extended["future_extension"] = serde_json::json!({"unknown": true});
        assert_eq!(
            serde_json::from_value::<SetupRequest>(extended).unwrap(),
            request
        );
        assert_eq!(
            SetupRequest::new("s", "opencode", "setup", Map::new(), "m", 1).descends_from,
            None
        );
    }

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
            if let SetupAnswer::Ready { initial, .. } = &answer {
                assert_eq!(initial.check(), Ok(()), "{name}");
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
                let fixed = crate::errors::refuse_codes::retryable(code);
                assert_eq!(setup.retryable(), Some(fixed), "{code}");
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
            assert_eq!(setup.retryable(), Some(fixed), "{code}");
            assert!(serde_json::to_value(&setup)
                .unwrap()
                .get("retryable")
                .is_none());
        }
    }
}
