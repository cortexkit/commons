//! Decision explanations (`route.explain`, group `explain`). Only the selecting
//! `(principal, consumer)` may ask. Unknown, forgotten, foreign and
//! non-selection decisions all answer `unknown_decision`; expiry alone does
//! not prevent explaining a retained decision. Account-admission reason ids
//! and continuation-offer ids are not selection decisions.

use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::errors::Refusal;
use crate::members::Members;
use crate::select::{RouteRef, RunnerRef, SelectAnswer};

open_set! {
    /// Informative routing reason for logs, never a closed decision language.
    pub enum ReasonCode {
        Pinned => "pinned",
        FellBack => "fell_back",
        Affinity => "affinity",
        Ranked => "ranked",
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[non_exhaustive]
pub struct ExplainRequest {
    pub decision_id: String,
}

impl ExplainRequest {
    pub fn new(decision_id: impl Into<String>) -> Self {
        Self {
            decision_id: decision_id.into(),
        }
    }
}

/// Public serving identities from the latest `account_for` admission carrying
/// the decision, or from the select answer's auth when no admission carried it.
/// Unbound members are absent. A continuation does not move these bindings:
/// it carries `previous_decision_id`, not the explained selection decision.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[non_exhaustive]
pub struct Serving {
    pub provider: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub account_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub credential_id: Option<String>,
}

impl Serving {
    pub fn new(provider: impl Into<String>) -> Self {
        Self {
            provider: provider.into(),
            account_id: None,
            credential_id: None,
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[non_exhaustive]
pub struct ExplainAnswer {
    pub decision_id: String,
    pub route: RouteRef,
    pub runner: RunnerRef,
    pub serving: Serving,
    pub policy_version: String,
    pub reason_code: ReasonCode,
    /// Informative prose, never parsed by a caller.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub note: Option<String>,
}

impl ExplainAnswer {
    pub fn new(
        decision_id: impl Into<String>,
        route: RouteRef,
        runner: RunnerRef,
        serving: Serving,
        policy_version: impl Into<String>,
        reason_code: ReasonCode,
    ) -> Self {
        Self {
            decision_id: decision_id.into(),
            route,
            runner,
            serving,
            policy_version: policy_version.into(),
            reason_code,
            note: None,
        }
    }
}

/// Decode a request once discovery has declared `explain`. Decision ownership,
/// provenance and retention must be resolved by the router from its state.
pub fn parse_explain_request(params: &Value) -> Result<ExplainRequest, Refusal> {
    let members = Members::of(params)?;
    Ok(ExplainRequest::new(members.text("decision_id", None)?))
}

#[derive(Clone, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub enum ExplainProblem {
    DecisionId,
    Route,
    Runner,
    ServingProvider,
}

impl std::fmt::Display for ExplainProblem {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(match self {
            Self::DecisionId => "decision_id does not echo the request",
            Self::Route => "route differs from the selection answer",
            Self::Runner => "runner differs from the selection answer",
            Self::ServingProvider => "serving.provider differs from the route provider",
        })
    }
}

/// Check the request echo, full route and runner equality with the retained
/// selection answer, and the serving provider. Account/credential bindings
/// require admission history and cannot be checked from a select answer alone.
pub fn check_explain(
    select_answer: &SelectAnswer,
    request: &ExplainRequest,
    answer: &ExplainAnswer,
) -> Result<(), ExplainProblem> {
    if answer.decision_id != request.decision_id {
        return Err(ExplainProblem::DecisionId);
    }
    if answer.route != select_answer.route {
        return Err(ExplainProblem::Route);
    }
    if answer.runner != select_answer.runner {
        return Err(ExplainProblem::Runner);
    }
    if answer.serving.provider != answer.route.provider {
        return Err(ExplainProblem::ServingProvider);
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::select::RunnerAuth;
    use serde_json::json;

    fn fixture() -> (SelectAnswer, ExplainRequest, ExplainAnswer) {
        let mut route = RouteRef::new("p", "model");
        route.variant = Some("variant".into());
        let runner = RunnerRef::new("runner", route.clone())
            .with_harness("session")
            .with_auth(RunnerAuth {
                pool: Some("pool".into()),
                credential_id: Some("selected-credential".into()),
            });
        let selected = SelectAnswer::new(
            "d",
            route.clone(),
            runner.clone(),
            100_000,
            10_000,
            0,
            "Selected.",
        );
        let answer = ExplainAnswer::new(
            "d",
            route,
            runner,
            Serving::new("p"),
            "policy-1",
            ReasonCode::Ranked,
        );
        (selected, ExplainRequest::new("d"), answer)
    }

    #[test]
    fn explain_checks_echo_route_runner_and_provider() {
        let (selected, request, answer) = fixture();
        assert_eq!(check_explain(&selected, &request, &answer), Ok(()));
        let mut wrong = answer.clone();
        wrong.decision_id = "other".into();
        assert_eq!(
            check_explain(&selected, &request, &wrong),
            Err(ExplainProblem::DecisionId)
        );
        let mut wrong = answer.clone();
        wrong.route.provider = "other".into();
        assert_eq!(
            check_explain(&selected, &request, &wrong),
            Err(ExplainProblem::Route)
        );
        let mut wrong = answer.clone();
        wrong.route.model = "other".into();
        assert_eq!(
            check_explain(&selected, &request, &wrong),
            Err(ExplainProblem::Route)
        );
        let mut wrong = answer.clone();
        wrong.route.variant = None;
        assert_eq!(
            check_explain(&selected, &request, &wrong),
            Err(ExplainProblem::Route)
        );
        let mut wrong = answer.clone();
        wrong.runner.module = "other".into();
        assert_eq!(
            check_explain(&selected, &request, &wrong),
            Err(ExplainProblem::Runner)
        );
        let mut wrong = answer.clone();
        wrong.runner.model.model = "other".into();
        assert_eq!(
            check_explain(&selected, &request, &wrong),
            Err(ExplainProblem::Runner)
        );
        let mut wrong = answer.clone();
        wrong.runner.harness = None;
        assert_eq!(
            check_explain(&selected, &request, &wrong),
            Err(ExplainProblem::Runner)
        );
        let mut wrong = answer.clone();
        wrong.runner.auth = None;
        assert_eq!(
            check_explain(&selected, &request, &wrong),
            Err(ExplainProblem::Runner)
        );
        let mut wrong = answer;
        wrong.serving.provider = "other".into();
        assert_eq!(
            check_explain(&selected, &request, &wrong),
            Err(ExplainProblem::ServingProvider)
        );
    }

    #[test]
    fn explain_accepts_informative_members_and_admission_binding_changes() {
        let (selected, request, mut answer) = fixture();
        answer.serving.account_id = Some("admitted-account".into());
        answer.serving.credential_id = Some("admitted-credential".into());
        answer.reason_code = ReasonCode::Other("future".into());
        answer.policy_version = "policy-2".into();
        answer.note = Some("Informative prose.".into());
        assert_eq!(check_explain(&selected, &request, &answer), Ok(()));
        let wire = serde_json::to_string(&answer).unwrap();
        assert_eq!(
            serde_json::from_str::<ExplainAnswer>(&wire).unwrap(),
            answer
        );
    }

    #[test]
    fn explain_wire_omits_unbound_members_and_keeps_unknown_reason() {
        let wire = r#"{"decision_id":"d","route":{"provider":"p","model":"m"},"runner":{"module":"runner","model":{"provider":"p","model":"m"}},"serving":{"provider":"p"},"policy_version":"v","reason_code":"future"}"#;
        let answer: ExplainAnswer = serde_json::from_str(wire).unwrap();
        assert_eq!(answer.reason_code, ReasonCode::Other("future".into()));
        assert_eq!(serde_json::to_string(&answer).unwrap(), wire);
        assert_eq!(
            serde_json::to_string(&ExplainRequest::new("d")).unwrap(),
            r#"{"decision_id":"d"}"#
        );
    }

    #[test]
    fn explain_parser_names_malformed_decision_id() {
        for body in [
            json!({}),
            json!({"decision_id": null}),
            json!({"decision_id": ""}),
            json!({"decision_id": 1}),
        ] {
            assert_eq!(
                parse_explain_request(&body).unwrap_err().detail["field"],
                "decision_id"
            );
        }
        assert_eq!(
            parse_explain_request(&json!({"decision_id": "d"})).unwrap(),
            ExplainRequest::new("d")
        );
    }
}
