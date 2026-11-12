//! `route.report`: the request, its validator, and the answer.
//!
//! A report is idempotent on `decision_id`: the same body again returns the
//! first answer, and a different body after the first is `report_conflict`.
//! Bodies are compared as decoded ([`ReportRequest`] equality), so key
//! order, and a member sent as `null` instead of left out, never make a
//! body different.

use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::{errors::Refusal, members::Members};

open_set! {
    /// How running on a decision ended.
    pub enum Outcome {
        Completed => "completed",
        Refused => "refused",
        Failed => "failed",
        TimedOut => "timed_out",
        Cancelled => "cancelled",
        /// A caller request field was invalid, including a pre-send refusal.
        ContractError => "contract_error",
        SessionError => "session_error",
        Interrupted => "interrupted",
        /// The run never reached the provider for a reason other than an
        /// invalid caller request. A provider refusal at launch is `failed`
        /// with its `runner_code`, not `launch_failed`.
        LaunchFailed => "launch_failed",
    }
}

/// Whether a report keeps or ends the decision's affinity preference.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub enum AffinityEffect {
    Kept,
    Ended,
}

/// The availability and affinity consequences of a report outcome.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub struct OutcomeEffect {
    pub availability_penalty: bool,
    pub affinity: AffinityEffect,
}

impl OutcomeEffect {
    pub const fn new(availability_penalty: bool, affinity: AffinityEffect) -> Self {
        Self {
            availability_penalty,
            affinity,
        }
    }
}

impl Outcome {
    /// The effects applied by a router declaring `report_outcomes`.
    /// Caller-side failures that never reached the provider do not penalize
    /// candidate routes. Unknown outcomes retain the legacy failure rule.
    pub fn effect(&self) -> OutcomeEffect {
        match self {
            Self::Completed
            | Self::Cancelled
            | Self::ContractError
            | Self::SessionError
            | Self::Interrupted
            | Self::LaunchFailed => OutcomeEffect::new(false, AffinityEffect::Kept),
            _ => OutcomeEffect::new(true, AffinityEffect::Ended),
        }
    }

    /// Whether the outcome penalizes the route's availability.
    pub fn is_failure(&self) -> bool {
        self.effect().availability_penalty
    }
}

/// Largest usage count exactly representable by readers using JSON doubles.
/// Every `usage` count shares this bound.
pub const MAX_USAGE_COUNT: u64 = (1 << 53) - 1;

/// Token usage and timing of the work run on a decision.
/// Build with [`Self::new`] and the `with_*` setters.
///
/// The token field names and their meaning match
/// `cortexkit-role-llm-runner`'s usage: `input_tokens` counts only input
/// that was neither read from nor written to the prompt cache; cache reads
/// and writes are the separate `cached_input_tokens` and `cache_write_tokens`
/// counts. `reasoning_tokens` is a breakdown of `output_tokens`, never an
/// addition to it. An absent count means unrecorded, never zero. Every present
/// count is a non-negative integer up to [`MAX_USAGE_COUNT`]. `cost_usd` is
/// the caller's own cost claim, a non-negative finite JSON number. The provider may
/// ignore it, since it prices usage from its own catalog.
/// Unknown members are refused.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[non_exhaustive]
#[serde(deny_unknown_fields)]
pub struct Usage {
    /// Opaque run attribution, 1–128 characters without controls. Send only to
    /// routers declaring `report_usage_run_id`; absent preserves legacy totals.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub run_id: Option<String>,
    /// Applied request level; omitted for standard. Send only to a router
    /// declaring `service_tier`; older strict usage readers refuse this field.
    /// Producers use ServiceTier::as_str; readers keep unknown level strings.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub service_tier_applied: Option<String>,
    /// Provider-stated tier on a reported wire: a runner discovery entry with
    /// `served: "reported"`, allowing metadata only when that response states it
    /// (runner CONTRACT §9.3.2). Map known levels; keep others verbatim. Never copy applied.
    /// Send only to a router declaring `service_tier`. Prefer a provider-stated
    /// served tier; its price stays unconfirmed if its tier or rate is unrecognised.
    /// Without served, applied is only an unconfirmed estimate; absent applied
    /// means standard. Keep unfamiliar strings rather than inventing a rate.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub service_tier_served: Option<String>,
    /// Input tokens excluding cache reads and writes.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub input_tokens: Option<u64>,
    /// Input read from the prompt cache.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub cached_input_tokens: Option<u64>,
    /// Input written to the prompt cache.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub cache_write_tokens: Option<u64>,
    /// Output tokens, including any counted in `reasoning_tokens`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub output_tokens: Option<u64>,
    /// Output reported as reasoning, already inside `output_tokens`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub reasoning_tokens: Option<u64>,
    /// Wall-clock run time, in milliseconds.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub duration_ms: Option<u64>,
    /// Time to the first output token, in milliseconds.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub first_token_ms: Option<u64>,
    /// The caller's own cost claim in USD. The provider may ignore it.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub cost_usd: Option<f64>,
}

impl Usage {
    pub fn with_run_id(mut self, run_id: impl Into<String>) -> Self {
        self.run_id = Some(run_id.into());
        self
    }

    /// Producer check only; report readers retain unfamiliar applied levels.
    pub fn check_service_tier(&self) -> Result<(), &'static str> {
        crate::service_tier::check_applied(self.service_tier_applied.as_deref())
    }
    pub fn with_service_tier_applied(mut self, tier: crate::service_tier::ServiceTier) -> Self {
        self.service_tier_applied = (!tier.is_standard()).then(|| tier.as_str().into());
        self
    }

    /// Set only from a provider response on a reported wire, never from applied.
    pub fn with_service_tier_served(mut self, tier: impl Into<String>) -> Self {
        self.service_tier_served = Some(tier.into());
        self
    }
    /// Empty usage: no count known yet.
    pub fn new() -> Self {
        Self::default()
    }

    /// Set input tokens excluding cache reads and writes.
    pub fn with_input_tokens(mut self, input_tokens: u64) -> Self {
        self.input_tokens = Some(input_tokens);
        self
    }

    /// Set input read from the prompt cache.
    pub fn with_cached_input_tokens(mut self, cached_input_tokens: u64) -> Self {
        self.cached_input_tokens = Some(cached_input_tokens);
        self
    }

    /// Set input written to the prompt cache.
    pub fn with_cache_write_tokens(mut self, cache_write_tokens: u64) -> Self {
        self.cache_write_tokens = Some(cache_write_tokens);
        self
    }

    /// Set output tokens.
    pub fn with_output_tokens(mut self, output_tokens: u64) -> Self {
        self.output_tokens = Some(output_tokens);
        self
    }

    /// Set output reported as reasoning.
    pub fn with_reasoning_tokens(mut self, reasoning_tokens: u64) -> Self {
        self.reasoning_tokens = Some(reasoning_tokens);
        self
    }

    /// Set wall-clock run time in milliseconds.
    pub fn with_duration_ms(mut self, duration_ms: u64) -> Self {
        self.duration_ms = Some(duration_ms);
        self
    }

    /// Set time to the first output token in milliseconds.
    pub fn with_first_token_ms(mut self, first_token_ms: u64) -> Self {
        self.first_token_ms = Some(first_token_ms);
        self
    }

    /// Set the caller's own cost claim in USD.
    pub fn with_cost_usd(mut self, cost_usd: f64) -> Self {
        self.cost_usd = Some(cost_usd);
        self
    }
}

/// Whether the account cannot serve this model or cannot serve at all. A
/// closed set: a router refuses a scope it doesn't name, as
/// `invalid_request {field: "eligibility"}`, rather than guessing one. A new
/// scope needs a describe group.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
#[non_exhaustive]
pub enum EligibilityScope {
    /// The account can't serve this model.
    Model,
    /// The account can't serve at all.
    Account,
}

/// What kind of refusal a report carries, mapped by the runner from its
/// provider's own wording, so the router never has to read vendor text. A
/// closed set, sent only to a router declaring `report_outcomes`.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
#[non_exhaustive]
pub enum RefusalClass {
    /// The provider refused this credential for any use, such as an entitlement
    /// or attestation refusal. The router blocks the credential. A credit wall
    /// with a still-valid credential is an account exhaustion observation instead.
    CredentialRefused,
    /// The request was over a size limit. Requires `request_input_tokens`;
    /// the router records a refusal for that request size, not for the
    /// account.
    RequestTooLarge,
}

/// Ineligibility reported by the model provider, never exhaustion or a hold.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Eligibility {
    pub scope: EligibilityScope,
    pub code: String,
}

/// Quota-exhaustion labels in the contract's refusal, reason and observation
/// vocabulary (sections 5.3, 9.1 and 9.2), when relayed as a runner code.
pub const EXHAUSTION_CODES: &[&str] = &[
    "account_limit_exhausted",
    "account_pool_exhausted",
    "quota_exhausted",
    "credential_billing_exhausted",
];

/// The `route.report` request.
/// Build with [`Self::new`] and the `with_*` setters.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[non_exhaustive]
pub struct ReportRequest {
    /// Authoritative attempt result when `report_attempt_outcomes` is declared.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub attempt_outcome: Option<crate::attempt::AttemptOutcome>,
    pub decision_id: String,
    pub outcome: Outcome,
    /// The runner's typed refusal or error code, verbatim.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub runner_code: Option<String>,
    /// When the runner reported it.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub credential_id: Option<String>,
    /// When the runner reported it.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub reset_at_ms: Option<u64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub usage: Option<Usage>,
    /// The pool the refused or served model drew from.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub pool: Option<String>,
    /// Sent instead of exhaustion when the account cannot serve the model
    /// or cannot serve at all. Never causes exhaustion or a hold.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub eligibility: Option<Eligibility>,
    /// Input size of the request, so a refusal can fence only its size class.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub request_input_tokens: Option<u64>,
    /// The refusal's kind, with `refused` or `failed` only. Requires
    /// `report_outcomes`. When present, the router acts on it and on nothing
    /// it might infer from `runner_code`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub refusal_class: Option<RefusalClass>,
}

impl ReportRequest {
    pub fn with_attempt_outcome(mut self, attempt: crate::attempt::AttemptOutcome) -> Self {
        self.outcome = attempt.legacy_outcome();
        self.attempt_outcome = Some(attempt);
        self
    }

    /// Only use this effect when the router declares `report_attempt_outcomes`.
    pub fn counts_against_route(&self) -> bool {
        self.attempt_outcome.as_ref().map_or_else(
            || self.outcome.is_failure(),
            |attempt| attempt.counts_against_route(),
        )
    }
    pub fn new(decision_id: impl Into<String>, outcome: Outcome) -> Self {
        Self {
            attempt_outcome: None,
            decision_id: decision_id.into(),
            outcome,
            runner_code: None,
            credential_id: None,
            reset_at_ms: None,
            usage: None,
            pool: None,
            eligibility: None,
            request_input_tokens: None,
            refusal_class: None,
        }
    }

    /// Set the runner's refusal or error code verbatim.
    pub fn with_runner_code(mut self, runner_code: impl Into<String>) -> Self {
        self.runner_code = Some(runner_code.into());
        self
    }

    /// Set the public credential id the runner reported.
    pub fn with_credential_id(mut self, credential_id: impl Into<String>) -> Self {
        self.credential_id = Some(credential_id.into());
        self
    }

    /// Set the reset time the runner reported, in Unix milliseconds.
    pub fn with_reset_at_ms(mut self, reset_at_ms: u64) -> Self {
        self.reset_at_ms = Some(reset_at_ms);
        self
    }

    /// Set the work's token usage and timing.
    pub fn with_usage(mut self, usage: Usage) -> Self {
        self.usage = Some(usage);
        self
    }

    /// Set the quota pool the refused or served model drew from.
    pub fn with_pool(mut self, pool: impl Into<String>) -> Self {
        self.pool = Some(pool.into());
        self
    }

    /// Set model or account ineligibility, instead of exhaustion or a hold.
    pub fn with_eligibility(mut self, eligibility: Eligibility) -> Self {
        self.eligibility = Some(eligibility);
        self
    }

    /// Set the request's input size in tokens.
    pub fn with_request_input_tokens(mut self, request_input_tokens: u64) -> Self {
        self.request_input_tokens = Some(request_input_tokens);
        self
    }

    /// Set the refusal's kind. Send it only to a router declaring
    /// `report_outcomes`.
    pub fn with_refusal_class(mut self, refusal_class: RefusalClass) -> Self {
        self.refusal_class = Some(refusal_class);
        self
    }
}

/// The `route.report` answer. A repeated report returns this answer exactly
/// as it was first given.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct ReportAnswer {
    pub decision_id: String,
    /// The provider's id for the recorded report.
    pub report_id: String,
    /// The outcome recorded.
    pub outcome: Outcome,
}

/// A caller-supplied reset fences at most this long: 24 hours in
/// milliseconds. A reset further away is capped at this bound; a reset
/// already in the past fences nothing.
pub const RESET_FENCE_CAP_MS: u64 = 24 * 60 * 60 * 1000;

/// Validate and decode `route.report` params. A bad member is
/// `invalid_request` naming it. An outcome this crate does not name is
/// accepted and kept.
pub fn parse_report_request(params: &Value) -> Result<ReportRequest, Refusal> {
    parse_report_request_serving::<&str>(params, &[])
}

/// Validate and decode reports for the router's declared capability groups.
/// Without `report_outcomes`, the four added outcomes remain unknown strings
/// and retain the legacy failure effect. Callers send `cancelled` instead to
/// routers without the group when a run failed before reaching the provider.
pub fn parse_report_request_serving<C: AsRef<str>>(
    params: &Value,
    capabilities: &[C],
) -> Result<ReportRequest, Refusal> {
    let members = Members::of(params)?;
    if members
        .object
        .get("attempt_outcome")
        .and_then(|attempt| attempt.get("retry_after_ms"))
        .is_some_and(|retry| !retry.is_null() && retry.as_u64().is_none())
    {
        return Err(members.refuse(
            "attempt_outcome.retry_after_ms",
            "retry_after_ms must be an unsigned duration",
        ));
    }
    let attempt_outcome: Option<crate::attempt::AttemptOutcome> =
        members.optional("attempt_outcome")?;
    if attempt_outcome.is_some()
        && !capabilities
            .iter()
            .any(|cap| cap.as_ref() == crate::capabilities::REPORT_ATTEMPT_OUTCOMES)
    {
        return Err(members.refuse(
            "attempt_outcome",
            "attempt_outcome requires report_attempt_outcomes",
        ));
    }
    let mut outcome: Outcome = members.required("outcome")?;
    let report_outcomes = capabilities
        .iter()
        .any(|cap| cap.as_ref() == "report_outcomes");
    if !report_outcomes
        && matches!(
            outcome,
            Outcome::ContractError
                | Outcome::SessionError
                | Outcome::Interrupted
                | Outcome::LaunchFailed
        )
    {
        outcome = Outcome::Other(outcome.as_str().to_owned());
    }
    if let Some(run_id) = members
        .object
        .get("usage")
        .and_then(|usage| usage.get("run_id"))
    {
        if !capabilities
            .iter()
            .any(|cap| cap.as_ref() == crate::capabilities::REPORT_USAGE_RUN_ID)
        {
            return Err(members.refuse("usage.run_id", "usage.run_id requires report_usage_run_id"));
        }
        if !run_id.is_null()
            && !run_id.as_str().is_some_and(|id| {
                (1..=128).contains(&id.chars().count()) && !id.chars().any(char::is_control)
            })
        {
            return Err(members.refuse(
                "usage.run_id",
                "usage.run_id must be 1–128 characters without controls",
            ));
        }
    }
    let usage: Option<Usage> = members.optional("usage")?;
    if let Some(usage) = &usage {
        for (name, value) in [
            ("input_tokens", usage.input_tokens),
            ("cached_input_tokens", usage.cached_input_tokens),
            ("cache_write_tokens", usage.cache_write_tokens),
            ("output_tokens", usage.output_tokens),
            ("reasoning_tokens", usage.reasoning_tokens),
            ("duration_ms", usage.duration_ms),
            ("first_token_ms", usage.first_token_ms),
        ] {
            if value.is_some_and(|count| count > MAX_USAGE_COUNT) {
                return Err(
                    members.refuse("usage", format!("usage.{name} is above {MAX_USAGE_COUNT}"))
                );
            }
        }
        if usage
            .cost_usd
            .is_some_and(|cost| !cost.is_finite() || cost < 0.0)
        {
            return Err(members.refuse(
                "usage",
                "usage.cost_usd must be a non-negative finite number",
            ));
        }
    }
    let request = ReportRequest {
        attempt_outcome,
        decision_id: members.text("decision_id", None)?,
        outcome,
        runner_code: members.optional_text("runner_code", None)?,
        credential_id: members.optional_text("credential_id", None)?,
        reset_at_ms: members.optional("reset_at_ms")?,
        usage,
        pool: members.pool()?,
        eligibility: members.optional("eligibility")?,
        request_input_tokens: members.optional("request_input_tokens")?,
        refusal_class: members.optional("refusal_class")?,
    };
    if let Some(attempt) = &request.attempt_outcome {
        attempt.check(&request.outcome)?;
    }
    if let Some(class) = request.refusal_class {
        if !report_outcomes {
            return Err(members.refuse(
                "refusal_class",
                "refusal_class requires the report_outcomes group",
            ));
        }
        if !matches!(request.outcome, Outcome::Refused | Outcome::Failed) {
            return Err(members.refuse(
                "refusal_class",
                "refusal_class goes only with outcome refused or failed",
            ));
        }
        if request.eligibility.is_some() {
            return Err(members.refuse(
                "refusal_class",
                "refusal_class and eligibility are alternatives",
            ));
        }
        if class == RefusalClass::RequestTooLarge && request.request_input_tokens.is_none() {
            return Err(members.refuse(
                "request_input_tokens",
                "request_too_large requires request_input_tokens",
            ));
        }
    }
    if request.eligibility.is_some()
        && (request.reset_at_ms.is_some()
            || request
                .runner_code
                .as_deref()
                .is_some_and(|code| EXHAUSTION_CODES.contains(&code)))
    {
        return Err(members.refuse(
            "eligibility",
            "eligibility cannot accompany exhaustion or a reset time",
        ));
    }
    Ok(request)
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn outcome_effect_matrix() {
        for (outcome, availability_penalty, affinity) in [
            (Outcome::Completed, false, AffinityEffect::Kept),
            (Outcome::Cancelled, false, AffinityEffect::Kept),
            (Outcome::Refused, true, AffinityEffect::Ended),
            (Outcome::Failed, true, AffinityEffect::Ended),
            (Outcome::TimedOut, true, AffinityEffect::Ended),
            (Outcome::ContractError, false, AffinityEffect::Kept),
            (Outcome::SessionError, false, AffinityEffect::Kept),
            (Outcome::Interrupted, false, AffinityEffect::Kept),
            (Outcome::LaunchFailed, false, AffinityEffect::Kept),
            (
                Outcome::Other("future_outcome".into()),
                true,
                AffinityEffect::Ended,
            ),
            (
                Outcome::Other("contract_error".into()),
                true,
                AffinityEffect::Ended,
            ),
        ] {
            let effect = outcome.effect();
            assert_eq!(
                effect.availability_penalty, availability_penalty,
                "{outcome}"
            );
            assert_eq!(effect.affinity, affinity, "{outcome}");
            assert_eq!(
                outcome.is_failure(),
                effect.availability_penalty,
                "{outcome}"
            );
        }
    }

    #[test]
    fn new_outcomes_require_report_outcomes_for_named_decoding() {
        for (wire, named) in [
            ("contract_error", Outcome::ContractError),
            ("session_error", Outcome::SessionError),
            ("interrupted", Outcome::Interrupted),
            ("launch_failed", Outcome::LaunchFailed),
        ] {
            let params = json!({"decision_id": "d", "outcome": wire});
            for groups in [&[][..], &["unrelated"][..]] {
                let request = parse_report_request_serving(&params, groups).unwrap();
                assert_eq!(request.outcome, Outcome::Other(wire.into()));
                assert!(request.outcome.is_failure());
                assert_eq!(serde_json::to_value(&request).unwrap()["outcome"], wire);
            }
            let request =
                parse_report_request_serving(&params, &["report_outcomes".to_owned()]).unwrap();
            assert_eq!(request.outcome, named);
            assert!(!request.outcome.is_failure());
            assert_eq!(serde_json::to_value(&request).unwrap()["outcome"], wire);
        }
        let unknown = parse_report_request_serving(
            &json!({"decision_id": "d", "outcome": "future_outcome"}),
            &["report_outcomes"],
        )
        .unwrap();
        assert_eq!(unknown.outcome, Outcome::Other("future_outcome".into()));
        assert!(unknown.outcome.is_failure());
    }

    #[test]
    fn legacy_report_parser_is_serving_without_groups() {
        for params in [
            json!({"decision_id": "d", "outcome": "completed"}),
            json!({"decision_id": "d", "outcome": "contract_error"}),
            json!({"decision_id": "d", "outcome": "future_outcome"}),
            json!({"decision_id": "d", "outcome": 4}),
            json!({"outcome": "failed"}),
            Value::Null,
        ] {
            assert_eq!(
                parse_report_request(&params),
                parse_report_request_serving::<&str>(&params, &[])
            );
        }
    }

    #[test]
    fn report_0_6_0_bytes_round_trip() {
        for body in [
            r#"{"decision_id":"d","outcome":"completed"}"#,
            r#"{"decision_id":"d","outcome":"failed","runner_code":"quota_exhausted","credential_id":"cred","reset_at_ms":42,"pool":"default","request_input_tokens":12}"#,
        ] {
            let params: Value = serde_json::from_str(body).unwrap();
            let request = parse_report_request(&params).unwrap();
            assert_eq!(serde_json::to_string(&request).unwrap(), body);
            assert_eq!(
                parse_report_request_serving(&params, &["report_outcomes"]).unwrap(),
                request
            );
        }
        assert_eq!(
            serde_json::to_string(&ReportRequest::new("d", Outcome::Completed)).unwrap(),
            r#"{"decision_id":"d","outcome":"completed"}"#
        );
    }

    #[test]
    fn an_unknown_outcome_is_kept_and_counts_as_failure() {
        let request =
            parse_report_request(&json!({"decision_id": "d", "outcome": "paused"})).unwrap();
        assert_eq!(request.outcome, Outcome::Other("paused".into()));
        assert!(request.outcome.is_failure());
        assert!(!Outcome::Completed.is_failure());
        assert!(!Outcome::Cancelled.is_failure());
        assert!(Outcome::TimedOut.is_failure());
    }

    #[test]
    fn null_and_absent_are_the_same_body() {
        let a = parse_report_request(
            &json!({"decision_id": "d", "outcome": "failed", "usage": null, "runner_code": null}),
        )
        .unwrap();
        let b = parse_report_request(&json!({"outcome": "failed", "decision_id": "d"})).unwrap();
        assert_eq!(a, b);
    }

    #[test]
    fn a_missing_decision_is_invalid_request() {
        let refusal = parse_report_request(&json!({"outcome": "failed"})).unwrap_err();
        assert_eq!(refusal.detail["field"], "decision_id");
    }

    #[test]
    fn usage_run_id_is_group_gated_and_validated_by_path() {
        use crate::capabilities::REPORT_USAGE_RUN_ID;
        use serde_json::json;
        for id in [
            json!(""),
            json!("a".repeat(129)),
            json!("a\nb"),
            json!("a\u{007f}"),
            json!(1),
            json!(true),
        ] {
            let body = json!({"decision_id":"d", "outcome":"ok", "usage":{"run_id":id}});
            let refusal = parse_report_request_serving(&body, &[REPORT_USAGE_RUN_ID]).unwrap_err();
            assert_eq!(refusal.code.as_str(), "invalid_request");
            assert_eq!(refusal.detail["field"], "usage.run_id");
        }
        for id in ["a".to_owned(), "é".repeat(128)] {
            let body = json!({"decision_id":"d", "outcome":"ok", "usage":{"run_id":id}});
            assert_eq!(
                parse_report_request(&body).unwrap_err().detail["field"],
                "usage.run_id"
            );
            let report = parse_report_request_serving(&body, &[REPORT_USAGE_RUN_ID]).unwrap();
            assert_eq!(report.usage.unwrap().run_id.as_deref(), Some(id.as_str()));
        }
        let body = json!({"decision_id":"d", "outcome":"ok", "usage":{"run_id":null}});
        assert_eq!(
            parse_report_request(&body).unwrap_err().detail["field"],
            "usage.run_id"
        );
        assert_eq!(
            parse_report_request_serving(&body, &[REPORT_USAGE_RUN_ID])
                .unwrap()
                .usage
                .unwrap()
                .run_id,
            None
        );
        assert!(parse_report_request_serving(
            &json!({"decision_id":"d", "outcome":"ok", "usage":{"unknown":1}}),
            &[REPORT_USAGE_RUN_ID]
        )
        .is_err());
        assert_eq!(serde_json::to_value(Usage::new()).unwrap(), json!({}));
        assert_eq!(
            serde_json::to_value(Usage::new().with_run_id("run-1")).unwrap(),
            json!({"run_id":"run-1"})
        );
    }
}
