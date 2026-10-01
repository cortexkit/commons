//! The `compaction` group: `compaction.ready`, and the admission check that
//! keeps a compaction provider off a runner that cannot call it.
//!
//! The compaction interface itself (Setup, the per-step status, the
//! answers `NOOP`, CompactionMessage, `WAIT` and `REFUSE`) is a set of calls
//! the runner makes to the session's compaction provider; the
//! `compaction-provider/v1` role defines those shapes. A runner owes them
//! only if it declares the `compaction` group, and the one op of the group
//! it serves is `compaction.ready`: when a provider that answered `WAIT`
//! finishes its work, it tells the runner, which asks again with a fresh
//! status instead of waiting out the bound. It is a hint: a runner that
//! never receives it asks again when the `WAIT`'s bound runs out.
//!
//! It arrives on the provider's module-level route, which is not bound to a
//! session, so it names the session, and the request it answers.

use serde::{Deserialize, Serialize};
use serde_json::{Map, Value};

use crate::{capabilities, errors};

/// The plan member that names the session's compaction provider, and the
/// `field` an `invalid_params` refusal names when a plan carries it to a
/// runner that does not declare `compaction`.
pub const PLAN_COMPACTION_ITEM: &str = "compaction_item";

/// The `field` of that admission refusal: the plan's `compaction_item`.
pub const PLAN_COMPACTION_ITEM_FIELD: &str = "plan.compaction_item";

/// The `compaction.ready` request. Decoded leniently: the answer is the
/// same whatever else it carries.
#[derive(Clone, Debug, PartialEq, Eq, Deserialize, Serialize)]
pub struct CompactionReady {
    /// The session the provider was asked about, echoed verbatim from the
    /// status that carried the `WAIT`. Opaque to the provider.
    pub session: String,
    /// The id of the request whose answer was `WAIT`, as that status named
    /// it. Only a ready for the newest request the runner issued wakes the
    /// session.
    pub request_id: String,
}

/// What a runner does with a `compaction.ready` it accepts from the
/// session's compaction provider.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ReadyOutcome {
    /// The ready names the newest issued request: the runner calls the
    /// provider again with a fresh status instead of waiting out the bound.
    CallAgain,
    /// The ready names an older request, or none was issued: the runner
    /// ignores it. This is not an error; the caller is answered as usual.
    Ignored,
}

impl CompactionReady {
    pub fn new(session: impl Into<String>, request_id: impl Into<String>) -> Self {
        Self {
            session: session.into(),
            request_id: request_id.into(),
        }
    }

    /// What the runner does with this ready, or the code it refuses it with.
    ///
    /// `caller` is the module the route stamps as the caller,
    /// `session_provider` the provider of the session's frozen plan's
    /// `compaction_item` (`None` for a session without one), and
    /// `newest_request_id` the newest compaction request the runner issued
    /// for the session (`None` before any). A ready from anyone but the
    /// session's provider is refused `not_session_compaction_provider`: it is
    /// only a hint, but a module that does not own the session's compaction
    /// must not be able to wake it.
    pub fn check(
        &self,
        caller: &str,
        session_provider: Option<&str>,
        newest_request_id: Option<&str>,
    ) -> Result<ReadyOutcome, &'static str> {
        if session_provider != Some(caller) {
            return Err(errors::NOT_SESSION_COMPACTION_PROVIDER);
        }
        if newest_request_id == Some(self.request_id.as_str()) {
            Ok(ReadyOutcome::CallAgain)
        } else {
            Ok(ReadyOutcome::Ignored)
        }
    }
}

/// The provider a plan's `compaction_item` names, or `None` when the plan
/// has no `compaction_item` or it names no provider.
pub fn plan_compaction_provider(plan: &Map<String, Value>) -> Option<&str> {
    plan.get(PLAN_COMPACTION_ITEM)?.get("provider")?.as_str()
}

/// Admission's check of a plan against the runner's module-level
/// capabilities: a plan carrying a `compaction_item` (any value but `null`)
/// is refused `invalid_params` naming [`PLAN_COMPACTION_ITEM_FIELD`] unless
/// `declared` includes `compaction`. A runner without the group never calls
/// a compaction provider, so admitting the plan would leave the session's
/// compaction owner uncalled. The refusal writes nothing.
pub fn check_plan_compaction<S: AsRef<str>>(
    plan: &Map<String, Value>,
    declared: &[S],
) -> Result<(), &'static str> {
    let carries_item = plan
        .get(PLAN_COMPACTION_ITEM)
        .is_some_and(|item| !item.is_null());
    let declares_group = declared
        .iter()
        .any(|name| name.as_ref() == capabilities::COMPACTION);
    if carries_item && !declares_group {
        Err(PLAN_COMPACTION_ITEM_FIELD)
    } else {
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::vectors;

    // This crate defines neither provider-answer shapes nor their application
    // classifier: the provider's role owns the shapes, and a runner implements
    // the fence. Keep answers as JSON objects and round-trip the case metadata
    // without claiming to test whether a running engine applies an answer.
    #[derive(Debug, PartialEq, Deserialize, Serialize)]
    struct FenceCase {
        name: String,
        state: FenceState,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        arrived_at_ms: Option<u64>,
        answer: Map<String, Value>,
        disposition: FenceDisposition,
    }

    #[derive(Debug, PartialEq, Deserialize, Serialize)]
    struct FenceState {
        newest_request_id: String,
        last_applied_version: u64,
        newest_ordinal: u64,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        newest_deadline_ms: Option<u64>,
    }

    #[derive(Debug, PartialEq, Deserialize, Serialize)]
    #[serde(rename_all = "snake_case")]
    enum FenceDisposition {
        SupersededRequest,
        Late,
        Act,
    }

    #[test]
    fn compaction_answer_fence_vectors_round_trip() {
        let file = vectors::load("fence.json");
        for case in vectors::cases(&file, "cases") {
            vectors::round_trip::<FenceCase>(case["name"].as_str().unwrap(), case);
        }
    }

    #[test]
    fn compaction_ready_vectors_decode() {
        let file = vectors::load("compaction-ready.json");
        for case in vectors::cases(&file, "requests") {
            vectors::round_trip::<CompactionReady>(
                case["name"].as_str().unwrap(),
                &case["request"],
            );
        }
        for case in vectors::cases(&file, "tolerated") {
            let name = case["name"].as_str().unwrap();
            serde_json::from_value::<CompactionReady>(case["request"].clone())
                .unwrap_or_else(|e| panic!("{name}: {e}"));
        }
        for case in vectors::cases(&file, "undecodable") {
            let name = case["name"].as_str().unwrap();
            let field = case["field"].as_str().unwrap();
            let error =
                serde_json::from_value::<CompactionReady>(case["request"].clone()).expect_err(name);
            assert!(error.to_string().contains(field), "{name}: {error}");
        }
        let ready = CompactionReady::new("s", "r1");
        assert_eq!(
            (ready.session.as_str(), ready.request_id.as_str()),
            ("s", "r1")
        );
    }

    #[test]
    fn compaction_ready_wakes_only_for_the_newest_request_from_the_session_provider() {
        let file = vectors::load("compaction-ready.json");
        for case in vectors::cases(&file, "checks") {
            let name = case["name"].as_str().unwrap();
            let ready: CompactionReady = serde_json::from_value(case["request"].clone()).unwrap();
            let outcome = ready.check(
                case["caller"].as_str().unwrap(),
                case["session_provider"].as_str(),
                case["newest_request_id"].as_str(),
            );
            match (outcome, case.get("refusal")) {
                (Ok(outcome), None) => {
                    let expected = match case["outcome"].as_str().unwrap() {
                        "call_again" => ReadyOutcome::CallAgain,
                        "ignored" => ReadyOutcome::Ignored,
                        other => panic!("{name}: unknown outcome {other}"),
                    };
                    assert_eq!(outcome, expected, "{name}");
                }
                (Err(code), Some(refusal)) => {
                    assert_eq!(refusal["code"], code, "{name}");
                    assert!(!errors::is_retryable(code), "{name}");
                }
                (outcome, refusal) => panic!("{name}: {outcome:?} against {refusal:?}"),
            }
        }
    }
}
