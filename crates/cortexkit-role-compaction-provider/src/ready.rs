//! `compaction.ready`: a provider tells the runner that the work behind its
//! `wait` answer is ready for the runner to request again.
//!
//! The runner serves this op; the provider sends it. On receiving it for
//! the newest request it issued, the runner calls `compaction.step` again
//! with a fresh status instead of waiting out the bound. It is a hint: the
//! runner's bound is the backstop, so a provider that never sends it only
//! makes the step wait longer.
//!
//! The request type and the runner's check belong to `llm-runner/v1`, and
//! are re-exported here so a provider builds exactly what the runner
//! decodes. Before considering a ready hint, the runner compares the route's
//! caller stamp (the daemon-verified module that opened the route) with the
//! session's configured provider at `plan.compaction_item.provider`. It refuses
//! a different module, or a session with no compaction provider, with
//! `not_session_compaction_provider`: an unrelated module must not wake the
//! session's compaction wait. A matching provider wakes the wait only when the
//! hint names the newest issued request; older hints are ignored. CONTRACT.md
//! §10 defines these checks.

pub use cortexkit_role_llm_runner::compaction::{
    plan_compaction_provider, CompactionReady, ReadyOutcome,
};

/// The runner's reply to `compaction.ready`: the empty object `{}`, the
/// same whether the hint was acted on or ignored, so it reveals no outcome.
/// Unknown fields are ignored.
#[derive(Clone, Debug, Default, PartialEq, Eq, serde::Deserialize, serde::Serialize)]
#[non_exhaustive]
pub struct ReadyReply {}

impl ReadyReply {
    pub fn new() -> Self {
        Self::default()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{errors, vectors};

    #[test]
    fn ready_vectors_check_as_recorded() {
        let file = vectors::load("ready.json");
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
            vectors::refused::<CompactionReady>(case["name"].as_str().unwrap(), &case["request"]);
        }
        for case in vectors::cases(&file, "checks") {
            let name = case["name"].as_str().unwrap();
            let ready: CompactionReady = serde_json::from_value(case["request"].clone()).unwrap();
            let plan = case["plan"].as_object().unwrap();
            let outcome = ready.check(
                case["caller"].as_str().unwrap(),
                plan_compaction_provider(plan),
                case["newest_request_id"].as_str(),
            );
            let got = match outcome {
                Ok(ReadyOutcome::CallAgain) => "call_again",
                Ok(ReadyOutcome::Ignored) => "ignored",
                Err(code) => {
                    assert!(errors::runner_codes::READY_CODES.contains(&code), "{name}");
                    code
                }
            };
            assert_eq!(got, case["outcome"].as_str().unwrap(), "{name}");
        }
    }
}
