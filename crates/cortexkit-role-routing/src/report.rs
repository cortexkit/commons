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

/// Token usage of the work run on a decision.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Usage {
    pub input_tokens: u64,
    /// Reasoning tokens are counted here.
    pub output_tokens: u64,
    /// The caller's own claim. The provider prices usage from its own
    /// catalog and may ignore it.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub cost_usd: Option<f64>,
}

/// The `route.report` request.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
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
        }
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

/// Validate and decode `route.report` params. A bad member is
/// `invalid_request` naming it. An outcome this crate does not name is
/// accepted and kept.
pub fn parse_report_request(params: &Value) -> Result<ReportRequest, Refusal> {
    let members = Members::of(params)?;
    Ok(ReportRequest {
        decision_id: members.text("decision_id", None)?,
        outcome: members.required("outcome")?,
        runner_code: members.optional_text("runner_code", None)?,
        credential_id: members.optional_text("credential_id", None)?,
        reset_at_ms: members.optional("reset_at_ms")?,
        usage: members.optional("usage")?,
    })
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
