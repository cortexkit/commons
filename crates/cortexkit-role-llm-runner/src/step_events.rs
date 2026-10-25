//! Content-free live hints published only after a step's record is durable.
//!
//! `step.finished.v1` is never authority or billing and grants nothing. The
//! `ck.{acct}.live.<runner>.step.finished.v1` subject is a plain publish, not
//! captured by a stream. Durable history remains the source of truth.

use serde::{Deserialize, Serialize};

/// Publication policy advertised alongside the `step_events` capability.
#[derive(Clone, Debug, PartialEq, Eq, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
#[non_exhaustive]
pub struct StepEvents {
    pub granularity: String,
    pub history_key: String,
}

impl StepEvents {
    pub fn new(granularity: impl Into<String>, history_key: impl Into<String>) -> Self {
        Self {
            granularity: granularity.into(),
            history_key: history_key.into(),
        }
    }

    pub fn with_granularity(mut self, granularity: impl Into<String>) -> Self {
        self.granularity = granularity.into();
        self
    }

    pub fn with_history_key(mut self, history_key: impl Into<String>) -> Self {
        self.history_key = history_key.into();
        self
    }

    /// Discovery refuses policies outside these two closed choices by name.
    pub fn valid(&self) -> bool {
        matches!(
            self.granularity.as_str(),
            "per_attempt" | "per_completed_step"
        ) && matches!(self.history_key.as_str(), "step_ref" | "harness")
    }
}

/// The runner's native durable-log position for a step. Sequence numbers are
/// scoped by runner, session reference and log incarnation, not message ordinals.
#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
#[non_exhaustive]
pub struct StepRef {
    pub runner: String,
    /// The §14.1 reference from `session_ref(&SessionIdentity { runner, harness,
    /// session })`, with the attested principal's bare runner module id.
    /// Opaque to consumers; consumers never derive it.
    pub session_ref: String,
    pub log_incarnation: String,
    pub seq: u64,
}

impl StepRef {
    pub fn new(
        runner: impl Into<String>,
        session_ref: impl Into<String>,
        log_incarnation: impl Into<String>,
        seq: u64,
    ) -> Self {
        Self {
            runner: runner.into(),
            session_ref: session_ref.into(),
            log_incarnation: log_incarnation.into(),
            seq,
        }
    }

    pub fn with_runner(mut self, runner: impl Into<String>) -> Self {
        self.runner = runner.into();
        self
    }
    pub fn with_session_ref(mut self, session_ref: impl Into<String>) -> Self {
        self.session_ref = session_ref.into();
        self
    }
    pub fn with_log_incarnation(mut self, log_incarnation: impl Into<String>) -> Self {
        self.log_incarnation = log_incarnation.into();
        self
    }
    pub fn with_seq(mut self, seq: u64) -> Self {
        self.seq = seq;
        self
    }
}

/// The CLOSED body of `step.finished.v1`. All strings are identifiers or
/// classifications, never a place for prompt, tool, error or other user text.
/// Unknown members are refused even though status/classification values are open.
#[derive(Clone, Debug, PartialEq, Eq, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
#[non_exhaustive]
pub struct StepFinished {
    pub runner: String,
    pub session_ref: String,
    pub log_incarnation: String,
    pub seq: u64,
    pub run_id: String,
    pub step_id: String,
    /// One-based, including on a per-completed-step publisher. It is not an
    /// event counter: internally retried attempts may be invisible.
    pub attempt: std::num::NonZeroU64,
    pub terminal_status: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub harness_step_ref: Option<String>,
    /// Opaque identity copied from the durable provider-attempt record. Required
    /// without `harness_step_ref`; never reused, including after restore.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub attempt_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub provider_request_id: Option<String>,
    pub harness: String,
    pub session: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub work_class: Option<String>,
    pub provider: String,
    pub model: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub input_tokens: Option<u64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub cached_input_tokens: Option<u64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub cache_write_tokens: Option<u64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub output_tokens: Option<u64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub reasoning_tokens: Option<u64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub cache_write_5m_tokens: Option<u64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub cache_write_1h_tokens: Option<u64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub reasoning_included_in_output: Option<bool>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub request_body_bytes: Option<u64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub context_limit: Option<u64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub finish_reason: Option<String>,
    pub started_at_ms: u64,
    pub finished_at_ms: u64,
}

impl StepFinished {
    /// Supply the display and position facts; set `attempt_id` or
    /// `harness_step_ref` before publishing, then call [`Self::check`].
    #[allow(clippy::too_many_arguments)]
    pub fn new(
        position: StepRef,
        run_id: impl Into<String>,
        step_id: impl Into<String>,
        attempt: std::num::NonZeroU64,
        terminal_status: impl Into<String>,
        harness: impl Into<String>,
        session: impl Into<String>,
        provider: impl Into<String>,
        model: impl Into<String>,
        started_at_ms: u64,
        finished_at_ms: u64,
    ) -> Self {
        Self {
            runner: position.runner,
            session_ref: position.session_ref,
            log_incarnation: position.log_incarnation,
            seq: position.seq,
            run_id: run_id.into(),
            step_id: step_id.into(),
            attempt,
            terminal_status: terminal_status.into(),
            harness: harness.into(),
            session: session.into(),
            provider: provider.into(),
            model: model.into(),
            started_at_ms,
            finished_at_ms,
            harness_step_ref: None,
            attempt_id: None,
            provider_request_id: None,
            work_class: None,
            input_tokens: None,
            cached_input_tokens: None,
            cache_write_tokens: None,
            output_tokens: None,
            reasoning_tokens: None,
            cache_write_5m_tokens: None,
            cache_write_1h_tokens: None,
            reasoning_included_in_output: None,
            request_body_bytes: None,
            context_limit: None,
            finish_reason: None,
        }
    }

    pub fn step_ref(&self) -> StepRef {
        StepRef::new(
            &self.runner,
            &self.session_ref,
            &self.log_incarnation,
            self.seq,
        )
    }

    /// Copy the five per-step token counters without changing the aggregate
    /// run-usage wire type or interpreting absent counters as zero.
    pub fn with_usage(mut self, usage: crate::usage::StepUsage) -> Self {
        self.input_tokens = usage.input_tokens;
        self.cached_input_tokens = usage.cached_input_tokens;
        self.cache_write_tokens = usage.cache_write_tokens;
        self.output_tokens = usage.output_tokens;
        self.reasoning_tokens = usage.reasoning_tokens;
        self
    }

    pub fn with_harness_step_ref(mut self, value: impl Into<String>) -> Self {
        self.harness_step_ref = Some(value.into());
        self
    }
    pub fn with_attempt_id(mut self, value: impl Into<String>) -> Self {
        self.attempt_id = Some(value.into());
        self
    }

    /// Check producer-side identity requirements before publication. Uniqueness
    /// and equality with the durable record must be enforced by the runner.
    pub fn check(&self) -> Result<(), &'static str> {
        if self
            .attempt_id
            .as_deref()
            .is_some_and(|id| !valid_attempt_id(id))
            || (self.attempt_id.is_none() && self.harness_step_ref.is_none())
        {
            return Err("attempt_id");
        }
        Ok(())
    }
    pub fn with_provider_request_id(mut self, value: impl Into<String>) -> Self {
        self.provider_request_id = Some(value.into());
        self
    }
    pub fn with_work_class(mut self, value: impl Into<String>) -> Self {
        self.work_class = Some(value.into());
        self
    }
    pub fn with_cache_write_5m_tokens(mut self, value: u64) -> Self {
        self.cache_write_5m_tokens = Some(value);
        self
    }
    pub fn with_cache_write_1h_tokens(mut self, value: u64) -> Self {
        self.cache_write_1h_tokens = Some(value);
        self
    }
    pub fn with_reasoning_included_in_output(mut self, value: bool) -> Self {
        self.reasoning_included_in_output = Some(value);
        self
    }
    pub fn with_request_body_bytes(mut self, value: u64) -> Self {
        self.request_body_bytes = Some(value);
        self
    }
    pub fn with_context_limit(mut self, value: u64) -> Self {
        self.context_limit = Some(value);
        self
    }
    pub fn with_finish_reason(mut self, value: impl Into<String>) -> Self {
        self.finish_reason = Some(value.into());
        self
    }
}

/// Whether an opaque attempt identity has 1–128 ASCII letters, digits, `_` or
/// `-`. The grammar does not establish uniqueness or durable-record equality.
pub fn valid_attempt_id(value: &str) -> bool {
    (1..=128).contains(&value.len())
        && value
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || byte == b'_' || byte == b'-')
}

/// Why a live step hint must be dropped rather than used to update a view.
#[derive(Debug)]
#[non_exhaustive]
pub enum StepEventDecodeError {
    InvalidSubject,
    Body(serde_json::Error),
    InvalidField(&'static str),
    PublisherMismatch { publisher: String, runner: String },
}

impl std::fmt::Display for StepEventDecodeError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::InvalidSubject => f.write_str(
                "expected a concrete ck.<account>.live.<module>.step.finished.v1 subject",
            ),
            Self::Body(error) => error.fmt(f),
            Self::InvalidField(field) => write!(f, "invalid or missing {field}"),
            Self::PublisherMismatch { publisher, runner } => write!(
                f,
                "subject publisher {publisher:?} differs from body runner {runner:?}"
            ),
        }
    }
}

impl std::error::Error for StepEventDecodeError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Self::Body(error) => Some(error),
            _ => None,
        }
    }
}

/// Decode a captured publication, taking its publisher from the subject's module
/// token, never from the body. Bus grants restrict a module to its own live
/// subjects. A consumer drops every error, including a mismatched body runner.
pub fn decode_step_finished(
    subject: &str,
    body: &[u8],
) -> Result<StepFinished, StepEventDecodeError> {
    let tokens: Vec<_> = subject.split('.').collect();
    let concrete = |token: &str| {
        !token.is_empty()
            && !token
                .bytes()
                .any(|b| b.is_ascii_whitespace() || b == b'*' || b == b'>')
    };
    let publisher = match tokens.as_slice() {
        ["ck", account, "live", publisher, "step", "finished", "v1"]
            if concrete(account) && concrete(publisher) =>
        {
            *publisher
        }
        _ => return Err(StepEventDecodeError::InvalidSubject),
    };
    let event: StepFinished = serde_json::from_slice(body).map_err(StepEventDecodeError::Body)?;
    event.check().map_err(StepEventDecodeError::InvalidField)?;
    if event.runner != publisher {
        return Err(StepEventDecodeError::PublisherMismatch {
            publisher: publisher.into(),
            runner: event.runner,
        });
    }
    Ok(event)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{
        capabilities,
        describe::{check_describe, problem_names, DescribeProblem},
        PROVIDES, REQUIRED_OPS,
    };
    use serde_json::{json, Value};

    fn vectors() -> Value {
        serde_json::from_str(crate::vectors::file("step-events.json").unwrap()).unwrap()
    }

    fn describe(policy: Value) -> Value {
        json!({"majors": [{"version": PROVIDES, "ops": REQUIRED_OPS}],
            "implementation_version": "fixture", "capabilities": [capabilities::STEP_EVENTS],
            "session_capabilities_from": "admission", "step_events": policy})
    }

    #[test]
    fn full_and_minimal_step_events_round_trip() {
        let vectors = vectors();
        for name in ["full", "minimal"] {
            let event: StepFinished = serde_json::from_value(vectors[name].clone()).unwrap();
            assert_eq!(serde_json::to_value(&event).unwrap(), vectors[name]);
            assert_eq!(event.step_ref().seq, vectors[name]["seq"].as_u64().unwrap());
        }
        let minimal: StepFinished = serde_json::from_value(vectors["minimal"].clone()).unwrap();
        assert_eq!(minimal.input_tokens, None);
        assert_eq!(minimal.reasoning_included_in_output, None);
        assert_eq!(minimal.provider_request_id, None);
        let mut zero = vectors["minimal"].clone();
        zero["input_tokens"] = json!(0);
        let zero: StepFinished = serde_json::from_value(zero).unwrap();
        assert_eq!(zero.input_tokens, Some(0));
    }

    #[test]
    fn closed_step_event_refuses_unknown_and_content_members() {
        let vectors = vectors();
        for extra in vectors["refused_members"].as_array().unwrap() {
            let mut body = vectors["minimal"].clone();
            body.as_object_mut()
                .unwrap()
                .extend(extra.as_object().unwrap().clone());
            assert!(
                serde_json::from_value::<StepFinished>(body).is_err(),
                "{extra}"
            );
        }
    }

    #[test]
    fn step_event_publisher_comes_from_subject() {
        let body = serde_json::to_vec(&vectors()["minimal"]).unwrap();
        let accepted =
            decode_step_finished("ck.account_a.live.runner_a.step.finished.v1", &body).unwrap();
        assert_eq!(accepted.runner, "runner_a");
        assert!(
            matches!(decode_step_finished("ck.account_a.live.runner_b.step.finished.v1", &body), Err(StepEventDecodeError::PublisherMismatch { publisher, runner }) if publisher == "runner_b" && runner == "runner_a")
        );
        for subject in [
            "ck.account_a.event.runner_a.step.finished.v1",
            "ck.account_a.live.*.step.finished.v1",
            "ck..live.runner_a.step.finished.v1",
            "ck.account_a.live.runner_a.step.finished.v2",
            "ck.account_a.live.runner_a.step.finished.v1.extra",
        ] {
            assert!(
                matches!(
                    decode_step_finished(subject, &body),
                    Err(StepEventDecodeError::InvalidSubject)
                ),
                "{subject}"
            );
        }
    }

    #[test]
    fn step_event_requires_facts_and_one_based_attempt() {
        let vectors = vectors();
        for key in vectors["minimal"].as_object().unwrap().keys() {
            let mut body = vectors["minimal"].clone();
            body.as_object_mut().unwrap().remove(key);
            assert!(
                decode_step_finished(
                    "ck.account_a.live.runner_a.step.finished.v1",
                    &serde_json::to_vec(&body).unwrap(),
                )
                .is_err(),
                "missing {key}"
            );
        }
        let mut body = vectors["minimal"].clone();
        body["attempt"] = json!(0);
        assert!(serde_json::from_value::<StepFinished>(body).is_err());
    }

    #[test]
    fn step_event_attempt_id_refusals() {
        let vectors = vectors();
        for case in vectors["refused_attempt_ids"].as_array().unwrap() {
            let mut body = vectors["minimal"].clone();
            body.as_object_mut().unwrap().remove("attempt_id");
            body.as_object_mut()
                .unwrap()
                .extend(case["members"].as_object().unwrap().clone());
            let event: StepFinished = serde_json::from_value(body.clone()).unwrap();
            assert_eq!(event.check(), Err("attempt_id"), "{}", case["name"]);
            assert!(
                matches!(
                    decode_step_finished(
                        "ck.account_a.live.runner_a.step.finished.v1",
                        &serde_json::to_vec(&body).unwrap(),
                    ),
                    Err(StepEventDecodeError::InvalidField("attempt_id"))
                ),
                "{}",
                case["name"]
            );
        }
    }

    #[test]
    fn step_event_attempt_id_boundaries_and_host_reference() {
        let mut event: StepFinished = serde_json::from_value(vectors()["full"].clone()).unwrap();
        assert!(event.attempt_id.is_none());
        assert_eq!(event.check(), Ok(()));
        for id in ["a".to_owned(), "A0_-".repeat(32)] {
            event = event.with_attempt_id(&id);
            assert_eq!(event.check(), Ok(()));
            assert_eq!(event.attempt_id.as_deref(), Some(id.as_str()));
        }
        for id in ["", "bad.id", "é"] {
            event = event.with_attempt_id(id);
            assert_eq!(event.check(), Err("attempt_id"));
        }
    }

    #[test]
    fn step_events_declarations_refused_by_name() {
        let vectors = vectors();
        for policy in vectors["valid_policies"].as_array().unwrap() {
            assert!(check_describe(&describe(policy.clone())).is_ok());
        }
        for policy in vectors["malformed_groups"].as_array().unwrap() {
            assert_eq!(
                check_describe(&describe(policy.clone())),
                Err(vec![DescribeProblem::Undecodable(
                    problem_names::INVALID_STEP_EVENTS.into()
                )])
            );
        }
        let mut raw = describe(vectors["valid_policies"][0].clone());
        raw.as_object_mut().unwrap().remove("step_events");
        assert_eq!(
            check_describe(&raw),
            Err(vec![DescribeProblem::Undecodable(
                problem_names::STEP_EVENTS_MISSING.into()
            )])
        );
        raw["step_events"] = vectors["valid_policies"][0].clone();
        raw["capabilities"] = json!([]);
        assert_eq!(
            check_describe(&raw),
            Err(vec![DescribeProblem::Undecodable(
                problem_names::STEP_EVENTS_UNDECLARED.into()
            )])
        );
    }

    #[test]
    fn constructor_and_setters_preserve_reported_usage() {
        let event = StepFinished::new(
            StepRef::new("runner_a", "ref", "log", 9),
            "run",
            "step",
            std::num::NonZeroU64::new(1).unwrap(),
            "completed",
            "host",
            "session",
            "provider",
            "model",
            10,
            20,
        )
        .with_usage(
            crate::usage::StepUsage::unreported()
                .with_input_tokens(0)
                .with_output_tokens(3),
        )
        .with_cache_write_5m_tokens(1)
        .with_cache_write_1h_tokens(2)
        .with_reasoning_included_in_output(false)
        .with_request_body_bytes(4)
        .with_context_limit(5)
        .with_finish_reason("stop")
        .with_harness_step_ref("record")
        .with_provider_request_id("request")
        .with_work_class("interactive");
        assert_eq!(event.input_tokens, Some(0));
        assert_eq!(event.reasoning_tokens, None);
        assert_eq!(event.reasoning_included_in_output, Some(false));
        assert_eq!(
            serde_json::from_value::<StepFinished>(serde_json::to_value(&event).unwrap()).unwrap(),
            event
        );
    }
}
