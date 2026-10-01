//! `run.result`: a run's terminal state and its final assistant message.
//!
//! Every run reaches exactly one terminal state, and a crash never gives it a
//! second one. `interrupted` (the runner stopped or crashed under the run)
//! is distinct from `cancelled` (a caller asked it to stop).

use serde::{Deserialize, Serialize};
use serde_json::Value;

/// Run state names. A state not listed here decodes as [`RunState::Other`],
/// keeping its name, and a consumer treats it as unclassified.
pub mod states {
    /// The run is running.
    pub const ACTIVE: &str = "active";
    /// The run is paused; retrying the send that started it resumes it.
    pub const PAUSED: &str = "paused";
    /// The run ended normally.
    pub const COMPLETED: &str = "completed";
    /// The run ended at its step limit.
    pub const MAX_STEPS: &str = "max_steps";
    /// A caller cancelled the run.
    pub const CANCELLED: &str = "cancelled";
    /// The run ended with an error, carried in `error`.
    pub const ERROR: &str = "error";
    /// The runner stopped or crashed under the run.
    pub const INTERRUPTED: &str = "interrupted";
}

/// A run state, classified.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum RunState {
    Active,
    Paused,
    Completed,
    MaxSteps,
    Cancelled,
    Error,
    Interrupted,
    Other(String),
}

impl RunState {
    pub fn parse(state: &str) -> Self {
        match state {
            states::ACTIVE => Self::Active,
            states::PAUSED => Self::Paused,
            states::COMPLETED => Self::Completed,
            states::MAX_STEPS => Self::MaxSteps,
            states::CANCELLED => Self::Cancelled,
            states::ERROR => Self::Error,
            states::INTERRUPTED => Self::Interrupted,
            other => Self::Other(other.to_owned()),
        }
    }

    /// Whether the state is terminal. An unknown state is not known to be
    /// terminal, so this answers `false` for it.
    pub fn is_terminal(&self) -> bool {
        matches!(
            self,
            Self::Completed | Self::MaxSteps | Self::Cancelled | Self::Error | Self::Interrupted
        )
    }
}

/// The `run.result` request. Strict on unknown fields: a misspelled
/// `run_id` is refused rather than silently ignored.
#[derive(Clone, Debug, PartialEq, Eq, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct RunResultRequest {
    pub run_id: String,
}

impl RunResultRequest {
    pub fn new(run_id: impl Into<String>) -> Self {
        Self {
            run_id: run_id.into(),
        }
    }
}

/// The `run.result` answer. Decoded leniently.
///
/// Non-exhaustive so later optional members are additive: use
/// [`RunResult::new`] and the `with_*` setters, or decode one.
#[derive(Clone, Debug, PartialEq, Deserialize, Serialize)]
#[non_exhaustive]
pub struct RunResult {
    pub run_id: String,
    /// The run's state, one of [`states`]; any other value decodes as a
    /// plain string.
    pub state: String,
    /// Why the run ended, where the runner names a reason.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub reason: Option<String>,
    /// The typed error of a run that ended `error`. Its schema is the
    /// runner's; it carries the `provider_code` when a provider ended the
    /// run.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub error: Option<Value>,
    /// The final assistant message of a run that ended `completed`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub final_message: Option<FinalMessage>,
}

impl RunResult {
    pub fn new(run_id: impl Into<String>, state: impl Into<String>) -> Self {
        Self {
            run_id: run_id.into(),
            state: state.into(),
            reason: None,
            error: None,
            final_message: None,
        }
    }

    pub fn with_reason(mut self, reason: impl Into<String>) -> Self {
        self.reason = Some(reason.into());
        self
    }

    pub fn with_error(mut self, error: Value) -> Self {
        self.error = Some(error);
        self
    }

    pub fn with_final_message(mut self, final_message: FinalMessage) -> Self {
        self.final_message = Some(final_message);
        self
    }

    pub fn run_state(&self) -> RunState {
        RunState::parse(&self.state)
    }
}

/// A completed run's final assistant message, with its final values.
#[derive(Clone, Debug, PartialEq, Eq, Deserialize, Serialize)]
pub struct FinalMessage {
    pub ordinal: u64,
    pub mid: String,
    /// The message's text parts joined in order. Reasoning parts are
    /// excluded.
    pub text: String,
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{errors, vectors};

    #[test]
    fn run_result_vectors_decode_and_round_trip() {
        let file = vectors::load("run-result.json");
        for case in vectors::cases(&file, "requests") {
            vectors::round_trip::<RunResultRequest>(
                case["name"].as_str().unwrap(),
                &case["request"],
            );
        }
        for case in vectors::cases(&file, "refused_requests") {
            assert!(
                serde_json::from_value::<RunResultRequest>(case["request"].clone()).is_err(),
                "{}",
                case["name"]
            );
        }
        for case in vectors::cases(&file, "answers") {
            let name = case["name"].as_str().unwrap();
            let result: RunResult = vectors::round_trip(name, &case["answer"]);
            assert_eq!(
                result.run_state().is_terminal(),
                case["terminal"].as_bool().unwrap(),
                "{name}"
            );
            if result.run_state() == RunState::Completed {
                assert!(result.final_message.is_some(), "{name}");
            }
        }
        for case in vectors::cases(&file, "refusals") {
            assert_eq!(case["refusal"]["code"], errors::UNKNOWN_RUN);
        }
    }

    #[test]
    fn unknown_states_are_not_terminal_and_interrupted_is_not_cancelled() {
        assert_eq!(
            RunState::parse("vendor_state"),
            RunState::Other("vendor_state".into())
        );
        assert!(!RunState::parse("vendor_state").is_terminal());
        assert_ne!(
            RunState::parse(states::INTERRUPTED),
            RunState::parse(states::CANCELLED)
        );
        let result = RunResult::new("r1", states::COMPLETED).with_final_message(FinalMessage {
            ordinal: 4,
            mid: "m4".into(),
            text: "done".into(),
        });
        let encoded = serde_json::to_value(&result).unwrap();
        assert_eq!(
            serde_json::from_value::<RunResult>(encoded).unwrap(),
            result
        );
    }
}
