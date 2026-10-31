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
    }
}

impl Outcome {
    /// Whether the outcome is a failure of the route, which ends the
    /// decision's affinity preference. `completed` and `cancelled` are not;
    /// an outcome this crate does not name is, so a provider never keeps
    /// preferring a route on a report it cannot read.
    pub fn is_failure(&self) -> bool {
        !matches!(self, Self::Completed | Self::Cancelled)
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

open_set! {
    /// Whether the account cannot serve this model or cannot serve at all.
    pub enum EligibilityScope {
        Model => "model",
        Account => "account",
    }
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
}

impl ReportRequest {
    pub fn new(decision_id: impl Into<String>, outcome: Outcome) -> Self {
        Self {
            decision_id: decision_id.into(),
            outcome,
            runner_code: None,
            credential_id: None,
            reset_at_ms: None,
            usage: None,
            pool: None,
            eligibility: None,
            request_input_tokens: None,
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
    let members = Members::of(params)?;
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
        decision_id: members.text("decision_id", None)?,
        outcome: members.required("outcome")?,
        runner_code: members.optional_text("runner_code", None)?,
        credential_id: members.optional_text("credential_id", None)?,
        reset_at_ms: members.optional("reset_at_ms")?,
        usage,
        pool: members.pool()?,
        eligibility: members.optional("eligibility")?,
        request_input_tokens: members.optional("request_input_tokens")?,
    };
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
}
