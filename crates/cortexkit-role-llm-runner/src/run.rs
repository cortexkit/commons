//! Run results, status polling and cancellation.
//!
//! Every run reaches exactly one terminal state, and a crash never gives it a
//! second one. `interrupted` (the runner stopped or crashed under the run)
//! is distinct from `cancelled` (a caller asked it to stop).

use serde::{Deserialize, Serialize};

pub use crate::structured_output::{SkippedSibling, StructuredOutputResult};
pub use crate::usage::{ModelUsage, RunUsage, StepUsage};

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
    /// The session's input-token cap was reached, or usage was unreported.
    pub const BUDGET_EXHAUSTED: &str = "budget_exhausted";
    /// The session's context reached its ceiling at a step boundary, so the
    /// runner sent no further request. Carried with `context_ceiling`
    /// detail; never `error`, because no model or provider failed.
    pub const CONTEXT_CEILING: &str = "context_ceiling";
    /// A caller cancelled the run.
    pub const CANCELLED: &str = "cancelled";
    /// The run ended with an error, carried in `error`.
    pub const ERROR: &str = "error";
    /// The runner stopped or crashed under the run.
    pub const INTERRUPTED: &str = "interrupted";
}

/// A run state, classified. Non-exhaustive: a state the role adds later
/// must not break a consumer's `match`, which keeps a catch-all arm. A state
/// a runner names that this crate does not know decodes as
/// [`RunState::Other`], keeping its name.
#[derive(Clone, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub enum RunState {
    Active,
    Paused,
    Completed,
    MaxSteps,
    /// The session's input-token cap was reached, or usage was unreported.
    BudgetExhausted,
    /// The session's context reached its ceiling at a step boundary.
    ContextCeiling,
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
            states::BUDGET_EXHAUSTED => Self::BudgetExhausted,
            states::CONTEXT_CEILING => Self::ContextCeiling,
            states::CANCELLED => Self::Cancelled,
            states::ERROR => Self::Error,
            states::INTERRUPTED => Self::Interrupted,
            other => Self::Other(other.to_owned()),
        }
    }

    /// The state's name as the wire spells it: `parse(state.as_str())`
    /// gives back the same state.
    pub fn as_str(&self) -> &str {
        match self {
            Self::Active => states::ACTIVE,
            Self::Paused => states::PAUSED,
            Self::Completed => states::COMPLETED,
            Self::MaxSteps => states::MAX_STEPS,
            Self::BudgetExhausted => states::BUDGET_EXHAUSTED,
            Self::ContextCeiling => states::CONTEXT_CEILING,
            Self::Cancelled => states::CANCELLED,
            Self::Error => states::ERROR,
            Self::Interrupted => states::INTERRUPTED,
            Self::Other(state) => state,
        }
    }

    pub fn is_budget_exhausted(&self) -> bool {
        matches!(self, Self::BudgetExhausted)
    }

    pub fn is_context_ceiling(&self) -> bool {
        matches!(self, Self::ContextCeiling)
    }

    /// Whether the state is terminal. A state this crate does not know
    /// ([`RunState::Other`]) is not terminal.
    pub fn is_terminal(&self) -> bool {
        matches!(
            self,
            Self::Completed
                | Self::MaxSteps
                | Self::BudgetExhausted
                | Self::ContextCeiling
                | Self::Cancelled
                | Self::Error
                | Self::Interrupted
        )
    }
}

/// Causes of a `budget_exhausted` terminal state, decoded as open strings.
pub mod budget_exhausted_causes {
    pub const LIMIT_REACHED: &str = "limit_reached";
    pub const USAGE_UNREPORTED: &str = "usage_unreported";
}

/// Detail carried by both `run.result` and `run.status` on budget exhaustion.
#[derive(Clone, Debug, PartialEq, Eq, Deserialize, Serialize)]
#[non_exhaustive]
pub struct BudgetExhaustedDetail {
    pub limit: u64,
    /// Measured input + cache-read + cache-write tokens across the session.
    /// With `usage_unreported`, excludes the unreported step entirely.
    pub spent: u64,
    pub cause: String,
}

impl BudgetExhaustedDetail {
    pub fn new(limit: u64, spent: u64, cause: impl Into<String>) -> Self {
        Self {
            limit,
            spent,
            cause: cause.into(),
        }
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
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub detail: Option<BudgetExhaustedDetail>,
    /// Why the run ended, where the runner names a reason.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub reason: Option<String>,
    /// The error of a run that ended `error`, with optional measured details.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub error: Option<RunError>,
    /// The final assistant message of a run that ended `completed`: always
    /// present on a completed run, with `text: ""` when the message has no
    /// text parts. Absent on a run that has not ended.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub final_message: Option<FinalMessage>,
    /// On an ended run admitted with `structured_output`: what it produced
    /// and whether that validated. Absent otherwise.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub structured_output: Option<StructuredOutputResult>,
    /// On a run that ended `context_ceiling`: the window, the request's size
    /// and the cause. Absent otherwise.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub context_ceiling: Option<crate::errors::ContextCeilingDetail>,
    /// The run's token usage (§6.4). A runner that declares `run_usage`
    /// always includes it once the run has ended. Absent means the runner
    /// does not report usage, never zero.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub usage: Option<RunUsage>,
    /// Trailing directives parsed from the final text. Present only on a
    /// completed run whose send asked for them in `directives` (§9.9).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub directives: Option<Vec<crate::directives::Directive>>,
}

impl RunResult {
    pub fn new(run_id: impl Into<String>, state: impl Into<String>) -> Self {
        Self {
            run_id: run_id.into(),
            state: state.into(),
            detail: None,
            reason: None,
            error: None,
            final_message: None,
            structured_output: None,
            context_ceiling: None,
            usage: None,
            directives: None,
        }
    }

    pub fn with_context_ceiling(mut self, detail: crate::errors::ContextCeilingDetail) -> Self {
        self.context_ceiling = Some(detail);
        self
    }

    pub fn with_directives(mut self, directives: Vec<crate::directives::Directive>) -> Self {
        self.directives = Some(directives);
        self
    }

    pub fn with_usage(mut self, usage: RunUsage) -> Self {
        self.usage = Some(usage);
        self
    }

    pub fn with_structured_output(mut self, structured_output: StructuredOutputResult) -> Self {
        self.structured_output = Some(structured_output);
        self
    }

    pub fn with_reason(mut self, reason: impl Into<String>) -> Self {
        self.reason = Some(reason.into());
        self
    }

    pub fn with_detail(mut self, detail: BudgetExhaustedDetail) -> Self {
        self.detail = Some(detail);
        self
    }

    pub fn with_error(mut self, error: RunError) -> Self {
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

    /// Whether `final_message` agrees with the state: present on a
    /// `completed` run (a final message with no text parts still answers,
    /// with `text: ""`), and absent on a run that has not ended (`active`,
    /// `paused`). So an absent `final_message` means only that the run has
    /// not ended, and a consumer can tell "completed with no text" from "no
    /// final message yet". Other terminal states and unknown states are not
    /// constrained. A consumer treats an inconsistent answer as malformed;
    /// it is not a refusal.
    pub fn final_message_consistent(&self) -> bool {
        match self.run_state() {
            RunState::Completed => self.final_message.is_some(),
            RunState::Active | RunState::Paused => self.final_message.is_none(),
            _ => true,
        }
    }
}

/// Durable reasons recorded when sealing an interrupted run (§6.5).
pub mod interrupt_reasons {
    /// The runner sealed the run during a planned stop.
    pub const RUNNER_SHUTDOWN: &str = "runner_shutdown";
    /// The next runner start sealed a run left open by process death.
    pub const RUNNER_CRASH: &str = "runner_crash";
    /// The harness host the runner drives exited while the runner itself
    /// stayed up, so the run could not continue.
    pub const HOST_EXITED: &str = "host_exited";
}

/// What a final message's text parts are joined with: nothing. Text parts
/// are concatenated exactly as emitted, so when a provider splits a JSON
/// value across parts, the joined text is that JSON value, byte for byte;
/// any separator would land inside one of its strings.
pub const TEXT_SEPARATOR: &str = "";

/// A final message's `text`: its text parts, in order, joined with
/// [`TEXT_SEPARATOR`]. No parts give `""`.
pub fn join_text_parts<I, S>(parts: I) -> String
where
    I: IntoIterator<Item = S>,
    S: AsRef<str>,
{
    parts
        .into_iter()
        .map(|part| part.as_ref().to_owned())
        .collect::<Vec<_>>()
        .join(TEXT_SEPARATOR)
}

/// A completed run's final assistant message, with its final values.
#[derive(Clone, Debug, PartialEq, Eq, Deserialize, Serialize)]
pub struct FinalMessage {
    pub ordinal: u64,
    pub mid: String,
    /// The message's text parts concatenated in order, with nothing
    /// inserted between them ([`join_text_parts`]). Reasoning parts are
    /// excluded. `""` when the message has no text parts (reasoning only,
    /// or a provider's safety halt).
    pub text: String,
}

/// A run error. Codes and classes are open strings; unknown values are kept.
/// Missing optional members mean not measured, never zero or an empty string.
#[derive(Clone, Debug, PartialEq, Deserialize, Serialize)]
#[non_exhaustive]
pub struct RunError {
    pub class: String,
    pub message: String,
    /// The role code consumers branch on, not the provider's finer diagnostic.
    pub provider_code: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub status: Option<u16>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub retry_after_secs: Option<f64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub resets_at_ms: Option<u64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub account_ref: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub provider: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub reason: Option<String>,
    /// Diagnostics only: never replaces `provider_code` or decides retry.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub provider_detail_code: Option<String>,
}

impl RunError {
    pub fn new(
        class: impl Into<String>,
        message: impl Into<String>,
        provider_code: impl Into<String>,
    ) -> Self {
        Self {
            class: class.into(),
            message: message.into(),
            provider_code: provider_code.into(),
            status: None,
            retry_after_secs: None,
            resets_at_ms: None,
            account_ref: None,
            provider: None,
            reason: None,
            provider_detail_code: None,
        }
    }
    pub fn with_status(mut self, status: u16) -> Self {
        self.status = Some(status);
        self
    }
    pub fn with_retry_after_secs(mut self, seconds: f64) -> Self {
        self.retry_after_secs = Some(seconds);
        self
    }
    pub fn with_resets_at_ms(mut self, ms: u64) -> Self {
        self.resets_at_ms = Some(ms);
        self
    }
    pub fn with_account_ref(mut self, value: impl Into<String>) -> Self {
        self.account_ref = Some(value.into());
        self
    }
    pub fn with_provider(mut self, value: impl Into<String>) -> Self {
        self.provider = Some(value.into());
        self
    }
    pub fn with_reason(mut self, value: impl Into<String>) -> Self {
        self.reason = Some(value.into());
        self
    }
    pub fn with_provider_detail_code(mut self, value: impl Into<String>) -> Self {
        self.provider_detail_code = Some(value.into());
        self
    }
}

/// The strict `run.status` request. The session-level form `{}` is optional
/// for runners: declaring `run_status` only promises the run-id form.
#[derive(Clone, Debug, Default, PartialEq, Eq, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
#[non_exhaustive]
pub struct RunStatusRequest {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub run_id: Option<String>,
}
impl RunStatusRequest {
    pub fn new() -> Self {
        Self::default()
    }
    pub fn with_run_id(mut self, run_id: impl Into<String>) -> Self {
        self.run_id = Some(run_id.into());
        self
    }
}

/// The `reason` of an `active` run waiting at its session's context ceiling
/// for a compaction view, with `deadline_ms` beside it on `run.status`. The
/// run is waiting, not stalled; the wait ends in progress (a view arrived)
/// or in the terminal `context_ceiling` state.
pub const RUN_REASON_CONTEXT_CEILING_WAIT: &str = "context_ceiling_wait";

/// The `reason` of an `active` run whose step waits on a rewrite ride
/// (compaction-provider's `ttl_expired`, `model_change` or `surface_change`
/// cause) whose answer has been pending for at least
/// [`COMPACTION_WAIT_REPORT_AFTER_MS`], with `deadline_ms` beside it on
/// `run.status`: the absolute time the ride's deadline ends. The wait ends
/// when the ride resolves (an answer, a refusal or the missed deadline), and
/// the reason and deadline go with it.
pub const RUN_REASON_COMPACTION_WAIT: &str = "compaction_wait";

/// How long a rewrite ride's answer is pending before the runner reports the
/// wait as `compaction_wait`, in milliseconds. A ride answered sooner is
/// never reported, so a quick `nothing` never shows.
pub const COMPACTION_WAIT_REPORT_AFTER_MS: u64 = 1_000;

/// Known reasons a run names on `run.status`, `session.head` and stream
/// events: why a paused run is paused, or why an active run is waiting.
/// Other reasons survive decoding unchanged.
#[derive(Clone, Debug, PartialEq, Eq, Deserialize, Serialize)]
#[serde(from = "String", into = "String")]
#[non_exhaustive]
pub enum RunReason {
    AuthRequired,
    Restart,
    /// An `active` run held at the context ceiling until `deadline_ms`.
    ContextCeilingWait,
    /// An `active` run whose step waits on a rewrite ride until
    /// `deadline_ms`.
    CompactionWait,
    Unknown(String),
}
impl From<String> for RunReason {
    fn from(value: String) -> Self {
        match value.as_str() {
            "auth_required" => Self::AuthRequired,
            "restart" => Self::Restart,
            RUN_REASON_CONTEXT_CEILING_WAIT => Self::ContextCeilingWait,
            RUN_REASON_COMPACTION_WAIT => Self::CompactionWait,
            _ => Self::Unknown(value),
        }
    }
}
impl From<RunReason> for String {
    fn from(value: RunReason) -> Self {
        match value {
            RunReason::AuthRequired => "auth_required".into(),
            RunReason::Restart => "restart".into(),
            RunReason::ContextCeilingWait => RUN_REASON_CONTEXT_CEILING_WAIT.into(),
            RunReason::CompactionWait => RUN_REASON_COMPACTION_WAIT.into(),
            RunReason::Unknown(s) => s,
        }
    }
}

/// The lenient run-id `run.status` answer. Session-level answers may use a
/// runner's own schema. An unknown run answers only `{state: "unknown"}`.
#[derive(Clone, Debug, PartialEq, Deserialize, Serialize)]
#[non_exhaustive]
pub struct RunStatus {
    pub state: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub detail: Option<BudgetExhaustedDetail>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub reason: Option<RunReason>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub error: Option<RunError>,
    /// On an ended run, `None` means delivery could not be determined; an
    /// empty list means checked and clean. Absent on a non-terminal run.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub undelivered_steers: Option<Vec<String>>,
    /// Exactly what `run.result` carries for the same run.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub structured_output: Option<StructuredOutputResult>,
    /// Exactly what `run.result` carries for the same run.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub context_ceiling: Option<crate::errors::ContextCeilingDetail>,
    /// When the run is in a wait the runner owns and ends by itself, in
    /// milliseconds since the Unix epoch: on an `active` or `paused` run,
    /// whatever its `reason` (`context_ceiling_wait` is one such wait). A
    /// watchdog treats the run as healthy until then
    /// ([`RunStatus::runner_wait_until`]).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub deadline_ms: Option<u64>,
    /// The run's token usage so far (§6.4). A runner that declares
    /// `run_usage` always includes it once a step of the run has completed,
    /// and may include it earlier with `steps: 0`. Absent means not
    /// reported, never zero.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub usage: Option<RunUsage>,
    /// Trailing directives parsed from the final text. Present only on a
    /// completed run whose send asked for them in `directives` (§9.9).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub directives: Option<Vec<crate::directives::Directive>>,
}
impl RunStatus {
    pub fn new(state: impl Into<String>) -> Self {
        Self {
            state: state.into(),
            detail: None,
            reason: None,
            error: None,
            undelivered_steers: None,
            structured_output: None,
            context_ceiling: None,
            deadline_ms: None,
            usage: None,
            directives: None,
        }
    }
    /// An `active` run waiting at the context ceiling until `deadline_ms`.
    pub fn context_ceiling_wait(deadline_ms: u64) -> Self {
        let mut status = Self::new(states::ACTIVE).with_reason(RunReason::ContextCeilingWait);
        status.deadline_ms = Some(deadline_ms);
        status
    }
    /// An `active` run whose step waits on a rewrite ride until
    /// `deadline_ms`, the absolute time the ride's deadline ends.
    pub fn compaction_wait(deadline_ms: u64) -> Self {
        Self::new(states::ACTIVE)
            .with_reason(RunReason::CompactionWait)
            .with_deadline_ms(deadline_ms)
    }
    pub fn with_deadline_ms(mut self, deadline_ms: u64) -> Self {
        self.deadline_ms = Some(deadline_ms);
        self
    }
    pub fn with_context_ceiling(mut self, detail: crate::errors::ContextCeilingDetail) -> Self {
        self.context_ceiling = Some(detail);
        self
    }
    pub fn with_structured_output(mut self, structured_output: StructuredOutputResult) -> Self {
        self.structured_output = Some(structured_output);
        self
    }
    pub fn with_reason(mut self, reason: RunReason) -> Self {
        self.reason = Some(reason);
        self
    }
    pub fn with_detail(mut self, detail: BudgetExhaustedDetail) -> Self {
        self.detail = Some(detail);
        self
    }
    pub fn with_error(mut self, error: RunError) -> Self {
        self.error = Some(error);
        self
    }
    pub fn with_undelivered_steers(mut self, steers: Vec<String>) -> Self {
        self.undelivered_steers = Some(steers);
        self
    }
    pub fn with_directives(mut self, directives: Vec<crate::directives::Directive>) -> Self {
        self.directives = Some(directives);
        self
    }

    pub fn with_usage(mut self, usage: RunUsage) -> Self {
        self.usage = Some(usage);
        self
    }
    /// When an `active` or `paused` run carries `deadline_ms`: the run is in
    /// a wait the runner owns and ends by itself, and this is the time (in
    /// milliseconds since the Unix epoch) before which a watchdog or owner
    /// must not nudge, stop or end it. Whatever the run's `reason` is: a
    /// caller recognizes the wait by `deadline_ms` alone. `None` for any
    /// other run, and for a run without `deadline_ms`. After the returned
    /// time has passed, the run counts as ordinary silence again.
    pub fn runner_wait_until(&self) -> Option<u64> {
        matches!(self.state.as_str(), states::ACTIVE | states::PAUSED)
            .then_some(self.deadline_ms)
            .flatten()
    }
    pub fn run_state(&self) -> RunState {
        RunState::parse(&self.state)
    }
}

/// Known cancellation points. Requests decode these as open strings.
pub mod cancel_at {
    /// Cancel immediately; the default when `at` is absent.
    pub const NOW: &str = "now";
    /// Finish the current model step and its tool batch, then cancel.
    pub const STEP_BOUNDARY: &str = "step_boundary";
}

/// Stop the named active or paused run, under the session owner's scope.
///
/// Non-exhaustive so a later optional field is additive: build one with
/// [`RunCancelRequest::new`] and [`RunCancelRequest::with_at`].
#[derive(Clone, Debug, PartialEq, Eq, Deserialize, Serialize)]
#[non_exhaustive]
pub struct RunCancelRequest {
    pub run_id: String,
    /// Cancellation point: `now` or `step_boundary` ([`cancel_at`]). Send this
    /// field only if the runner's discovery answer lists the value in
    /// `role.describe.run_cancel.at`; older runners would ignore it.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub at: Option<String>,
}
impl RunCancelRequest {
    pub fn new(run_id: impl Into<String>) -> Self {
        Self {
            run_id: run_id.into(),
            at: None,
        }
    }

    pub fn with_at(mut self, at: impl Into<String>) -> Self {
        self.at = Some(at.into());
        self
    }

    /// Omission preserves immediate cancellation.
    pub fn at(&self) -> &str {
        self.at.as_deref().unwrap_or(cancel_at::NOW)
    }
}

/// A JSON string. `Ack` accepts cancellation, but does not determine which
/// terminal wins a race. Unknown replies do not confirm cancellation.
#[derive(Clone, Debug, PartialEq, Eq, Deserialize, Serialize)]
#[serde(from = "String", into = "String")]
#[non_exhaustive]
pub enum RunCancelReply {
    Ack,
    NotActive,
    Unknown(String),
}
impl From<String> for RunCancelReply {
    fn from(value: String) -> Self {
        match value.as_str() {
            "ack" => Self::Ack,
            "not_active" => Self::NotActive,
            _ => Self::Unknown(value),
        }
    }
}
impl From<RunCancelReply> for String {
    fn from(value: RunCancelReply) -> Self {
        match value {
            RunCancelReply::Ack => "ack".into(),
            RunCancelReply::NotActive => "not_active".into(),
            RunCancelReply::Unknown(s) => s,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A runner-owned wait is read from `deadline_ms` on an `active` or
    /// `paused` run, whatever its `reason`; never on an ended run, and never
    /// from `reason` alone.
    #[test]
    fn a_runner_owned_wait_is_recognized_by_its_deadline() {
        assert_eq!(
            RunStatus::context_ceiling_wait(7).runner_wait_until(),
            Some(7)
        );
        assert_eq!(RunStatus::compaction_wait(8).runner_wait_until(), Some(8));
        // A wait kind this crate does not name is still a wait.
        let unnamed = RunStatus::new(states::ACTIVE)
            .with_reason(RunReason::from("some_later_wait".to_owned()))
            .with_deadline_ms(9);
        assert_eq!(unnamed.runner_wait_until(), Some(9));
        let paused = RunStatus::new(states::PAUSED).with_deadline_ms(11);
        assert_eq!(paused.runner_wait_until(), Some(11));
        // A paused run waiting for a caller's retry carries no deadline.
        let retry = RunStatus::new(states::PAUSED).with_reason(RunReason::AuthRequired);
        assert_eq!(retry.runner_wait_until(), None);
        // The reason alone is display text, never a wait.
        let reason_only = RunStatus::new(states::ACTIVE).with_reason(RunReason::ContextCeilingWait);
        assert_eq!(reason_only.runner_wait_until(), None);
        for ended in [
            states::COMPLETED,
            states::CONTEXT_CEILING,
            "some_later_state",
        ] {
            let status = RunStatus::new(ended).with_deadline_ms(5);
            assert_eq!(status.runner_wait_until(), None, "{ended}");
        }
    }

    #[test]
    fn every_named_state_round_trips_through_its_own_variant() {
        for name in [
            states::ACTIVE,
            states::PAUSED,
            states::COMPLETED,
            states::MAX_STEPS,
            states::BUDGET_EXHAUSTED,
            states::CONTEXT_CEILING,
            states::CANCELLED,
            states::ERROR,
            states::INTERRUPTED,
        ] {
            let state = RunState::parse(name);
            assert!(
                !matches!(state, RunState::Other(_)),
                "{name} decoded as Other"
            );
            assert_eq!(state.as_str(), name);
            assert_eq!(RunState::parse(state.as_str()), state, "{name}");
        }
        assert_eq!(
            RunState::parse(states::BUDGET_EXHAUSTED),
            RunState::BudgetExhausted
        );
        assert!(
            RunState::BudgetExhausted.is_budget_exhausted()
                && RunState::BudgetExhausted.is_terminal()
        );
        assert!(
            RunState::ContextCeiling.is_context_ceiling() && RunState::ContextCeiling.is_terminal()
        );
        let vendor = RunState::parse("vendor_state");
        assert_eq!(vendor, RunState::Other("vendor_state".into()));
        assert_eq!(vendor.as_str(), "vendor_state");
        assert!(!vendor.is_terminal());
    }
    use crate::{errors, vectors};

    #[test]
    fn run_error_absent_members_are_not_measured_and_bytes_are_pinned() {
        let bytes = r#"{"class":"future-class","message":"failed","provider_code":"future-code"}"#;
        let error: RunError = serde_json::from_str(bytes).unwrap();
        assert_eq!(
            error,
            RunError::new("future-class", "failed", "future-code")
        );
        assert_eq!(error.status, None);
        assert_eq!(error.retry_after_secs, None);
        assert_eq!(error.resets_at_ms, None);
        assert_eq!(error.account_ref, None);
        assert_eq!(error.provider, None);
        assert_eq!(error.reason, None);
        assert_eq!(error.provider_detail_code, None);
        assert_eq!(serde_json::to_string(&error).unwrap(), bytes);
        let full = RunError::new("transient", "limited", "billing_exhausted")
            .with_status(429)
            .with_retry_after_secs(0.5)
            .with_resets_at_ms(1234)
            .with_account_ref("account-1")
            .with_provider("provider-1")
            .with_reason("limit")
            .with_provider_detail_code("diagnostic");
        let full_bytes = r#"{"class":"transient","message":"limited","provider_code":"billing_exhausted","status":429,"retry_after_secs":0.5,"resets_at_ms":1234,"account_ref":"account-1","provider":"provider-1","reason":"limit","provider_detail_code":"diagnostic"}"#;
        assert_eq!(serde_json::to_string(&full).unwrap(), full_bytes);
        assert_eq!(serde_json::from_str::<RunError>(full_bytes).unwrap(), full);
        for bytes in [
            r#"{"message":"m","provider_code":"c"}"#,
            r#"{"class":"c","provider_code":"p"}"#,
            r#"{"class":"c","message":"m"}"#,
        ] {
            assert!(serde_json::from_str::<RunError>(bytes).is_err());
        }
        assert!(serde_json::from_str::<RunError>(
            r#"{"class":"c","message":"m","provider_code":"p","future":true}"#
        )
        .is_ok());
    }

    #[test]
    fn run_status_wire_bytes_open_reason_and_unknown_run() {
        for (request, bytes) in [
            (RunStatusRequest::new(), "{}"),
            (
                RunStatusRequest::new().with_run_id("r1"),
                r#"{"run_id":"r1"}"#,
            ),
        ] {
            assert_eq!(serde_json::to_string(&request).unwrap(), bytes);
            assert_eq!(
                serde_json::from_str::<RunStatusRequest>(bytes).unwrap(),
                request
            );
        }
        let error = serde_json::from_str::<RunStatusRequest>(r#"{"runId":"r1"}"#).unwrap_err();
        assert!(error.to_string().contains("runId"));
        let unknown = RunStatus::new("unknown");
        assert_eq!(
            serde_json::to_string(&unknown).unwrap(),
            r#"{"state":"unknown"}"#
        );
        assert!(!unknown.run_state().is_terminal());
        assert_eq!(
            serde_json::from_str::<RunStatus>(r#"{"state":"unknown","future":true}"#).unwrap(),
            unknown
        );
        let full = RunStatus::new("error")
            .with_reason(RunReason::Unknown("future-reason".into()))
            .with_error(RunError::new("permanent", "failed", "future-code"))
            .with_undelivered_steers(vec!["send-1".into(), "send-2".into()]);
        let bytes = r#"{"state":"error","reason":"future-reason","error":{"class":"permanent","message":"failed","provider_code":"future-code"},"undelivered_steers":["send-1","send-2"]}"#;
        assert_eq!(serde_json::to_string(&full).unwrap(), bytes);
        assert_eq!(serde_json::from_str::<RunStatus>(bytes).unwrap(), full);
        let clean = RunStatus::new("completed").with_undelivered_steers(vec![]);
        assert_eq!(
            serde_json::to_string(&clean).unwrap(),
            r#"{"state":"completed","undelivered_steers":[]}"#
        );
        assert_ne!(
            clean.undelivered_steers,
            RunStatus::new("completed").undelivered_steers
        );
        for (bytes, reason) in [
            (r#""auth_required""#, RunReason::AuthRequired),
            (r#""restart""#, RunReason::Restart),
        ] {
            assert_eq!(serde_json::from_str::<RunReason>(bytes).unwrap(), reason);
            assert_eq!(serde_json::to_string(&reason).unwrap(), bytes);
        }
    }

    #[test]
    fn run_cancel_wire_bytes_and_unknown_reply() {
        let request = RunCancelRequest::new("r1");
        assert_eq!(
            serde_json::to_string(&request).unwrap(),
            r#"{"run_id":"r1"}"#
        );
        assert_eq!(
            serde_json::from_str::<RunCancelRequest>(r#"{"run_id":"r1","future":true}"#).unwrap(),
            request
        );
        assert!(serde_json::from_str::<RunCancelRequest>("{}").is_err());
        for (bytes, reply) in [
            (r#""ack""#, RunCancelReply::Ack),
            (r#""not_active""#, RunCancelReply::NotActive),
            (r#""future""#, RunCancelReply::Unknown("future".into())),
        ] {
            assert_eq!(
                serde_json::from_str::<RunCancelReply>(bytes).unwrap(),
                reply
            );
            assert_eq!(serde_json::to_string(&reply).unwrap(), bytes);
        }
    }

    #[test]
    fn run_cancel_vectors_preserve_bytes_defaults_and_open_points() {
        let file = vectors::load("run-cancel.json");
        for case in vectors::cases(&file, "canonical") {
            let name = case["name"].as_str().unwrap();
            let request: RunCancelRequest = vectors::round_trip(name, &case["params"]);
            assert_eq!(request.at(), case["at"].as_str().unwrap(), "{name}");
            assert_eq!(
                serde_json::to_string(&request).unwrap(),
                case["bytes"].as_str().unwrap(),
                "{name}"
            );
        }
        for case in vectors::cases(&file, "edges") {
            let name = case["name"].as_str().unwrap();
            let initial: RunResult = vectors::round_trip(name, &case["initial"]);
            let terminal: RunResult = vectors::round_trip(name, &case["terminal"]);
            assert!(terminal.run_state().is_terminal(), "{name}");
            assert!(terminal.final_message_consistent(), "{name}");
            let requests = case["requests"].as_array().unwrap();
            let answers = case["answers"].as_array().unwrap();
            assert_eq!(requests.len(), answers.len(), "{name}");
            for (params, answer) in requests.iter().zip(answers) {
                let request: RunCancelRequest = vectors::round_trip(name, params);
                let reply: RunCancelReply = vectors::round_trip(name, answer);
                assert_eq!(request.run_id, initial.run_id, "{name}");
                assert_eq!(request.run_id, terminal.run_id, "{name}");
                assert!(
                    matches!(reply, RunCancelReply::Ack | RunCancelReply::NotActive),
                    "{name}"
                );
            }
        }
    }

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
            assert!(result.final_message_consistent(), "{name}");
        }
        for case in vectors::cases(&file, "refusals") {
            assert_eq!(case["refusal"]["code"], errors::UNKNOWN_RUN);
        }
    }

    #[test]
    fn a_completed_run_always_answers_its_final_message() {
        let file = vectors::load("run-result.json");
        for case in vectors::cases(&file, "inconsistent") {
            let name = case["name"].as_str().unwrap();
            let result: RunResult = serde_json::from_value(case["answer"].clone())
                .unwrap_or_else(|e| panic!("{name}: {e}"));
            assert!(!result.final_message_consistent(), "{name}");
        }
    }

    #[test]
    fn text_parts_are_joined_with_nothing_inserted() {
        let file = vectors::load("run-result.json");
        for case in vectors::cases(&file, "text_joins") {
            let name = case["name"].as_str().unwrap();
            let parts: Vec<&str> = case["parts"]
                .as_array()
                .unwrap()
                .iter()
                .map(|part| part.as_str().unwrap())
                .collect();
            assert_eq!(join_text_parts(&parts), case["text"], "{name}");
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

#[cfg(test)]
mod input_cap_tests {
    use super::*;
    use crate::{errors, vectors};

    #[test]
    fn input_cap_result_vectors_preserve_detail_and_open_states() {
        let file = vectors::load("input-token-cap-results.json");
        let mut causes = std::collections::BTreeSet::new();
        for case in vectors::cases(&file, "canonical") {
            let result: RunResult = vectors::round_trip("result", &case["result"]);
            let status: RunStatus = vectors::round_trip("status", &case["status"]);
            assert_eq!(result.detail, status.detail);
            assert!(result.run_state().is_budget_exhausted());
            assert!(status.run_state().is_terminal());
            assert_eq!(result.run_state(), RunState::BudgetExhausted);
            causes.insert(result.detail.unwrap().cause);
        }
        assert_eq!(
            causes,
            [
                budget_exhausted_causes::LIMIT_REACHED.into(),
                budget_exhausted_causes::USAGE_UNREPORTED.into()
            ]
            .into()
        );
        assert!(!RunState::parse("future_budget_state").is_terminal());
        assert_eq!(errors::BUDGET_EXHAUSTED, states::BUDGET_EXHAUSTED);
        assert_eq!(
            errors::tool_result_reasons::BUDGET_EXHAUSTED,
            states::BUDGET_EXHAUSTED
        );
        assert!(!errors::is_retryable(errors::BUDGET_EXHAUSTED));
        let detail = BudgetExhaustedDetail::new(10, 10, budget_exhausted_causes::LIMIT_REACHED);
        assert_eq!(
            RunResult::new("r1", states::BUDGET_EXHAUSTED)
                .with_detail(detail.clone())
                .detail,
            Some(detail.clone())
        );
        assert_eq!(
            RunStatus::new(states::BUDGET_EXHAUSTED)
                .with_detail(detail.clone())
                .detail,
            Some(detail)
        );
    }
}

#[cfg(test)]
mod interrupt_reason_tests {
    use super::*;
    use crate::{read::LastRunState, vectors};

    #[test]
    fn interrupted_reasons_decode_open_without_guessing_absence() {
        let file = vectors::load("interrupt-reasons.json");
        for raw in vectors::cases(&file, "results") {
            let result: RunResult = vectors::round_trip("interrupted reason", raw);
            let status: RunStatus = serde_json::from_value(raw.clone()).unwrap();
            let last: LastRunState = serde_json::from_value(raw.clone()).unwrap();
            assert_eq!(status.reason.map(String::from), result.reason);
            assert_eq!(last.reason, result.reason);
        }
        assert_eq!(
            file["results"][0]["reason"],
            interrupt_reasons::RUNNER_SHUTDOWN
        );
        assert_eq!(
            file["results"][1]["reason"],
            interrupt_reasons::RUNNER_CRASH
        );
        let old: RunResult = serde_json::from_value(file["results"][2].clone()).unwrap();
        assert_eq!(old.reason, None);
    }
}
