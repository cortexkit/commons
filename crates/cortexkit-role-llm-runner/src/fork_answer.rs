//! Isolated question-answering branches of the head's last served request.
//!
//! These wire types and admission helpers do not implement branch storage,
//! model execution or recovery. Runners implement those rules in CONTRACT.md §9.20.

use serde::{Deserialize, Serialize};
use serde_json::{Map, Value};

use crate::{capabilities, session_change, usage::RunUsage};

pub const PLAN_FORK_CALLERS: &str = "fork_callers";
pub const PLAN_FORK_CALLERS_FIELD: &str = "plan.fork_callers";

/// Strict, empty `session.fork_prepare` request on the management route's session.
#[derive(Clone, Debug, Default, PartialEq, Eq, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
#[non_exhaustive]
pub struct ForkPrepareRequest {}

impl ForkPrepareRequest {
    pub fn new() -> Self {
        Self::default()
    }
}

/// The prepared fork's runner-wide unique, never-reused identity.
#[derive(Clone, Debug, PartialEq, Eq, Deserialize, Serialize)]
#[non_exhaustive]
pub struct ForkPrepareReply {
    pub fork_id: String,
}

impl ForkPrepareReply {
    pub fn new(fork_id: impl Into<String>) -> Self {
        Self {
            fork_id: fork_id.into(),
        }
    }
}

/// Strict `session.fork_ask` request. Validate bounds with [`Self::check`]
/// before looking up a fork or making a model call.
#[derive(Clone, Debug, PartialEq, Eq, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
#[non_exhaustive]
pub struct ForkAskRequest {
    pub fork_id: String,
    pub ask_id: String,
    pub question: String,
}

impl ForkAskRequest {
    pub fn new(
        fork_id: impl Into<String>,
        ask_id: impl Into<String>,
        question: impl Into<String>,
    ) -> Self {
        Self {
            fork_id: fork_id.into(),
            ask_id: ask_id.into(),
            question: question.into(),
        }
    }

    /// The field an `invalid_params` refusal must name. Lengths count Unicode
    /// scalar values; whitespace is a non-empty question, not a missing one.
    pub fn check(&self) -> Result<(), &'static str> {
        if !(1..=256).contains(&self.ask_id.chars().count()) {
            return Err("ask_id");
        }
        if self.question.is_empty() {
            return Err("question");
        }
        Ok(())
    }
}

/// Open fork finish value, preserving later spellings verbatim.
#[derive(Clone, Debug, PartialEq, Eq, Deserialize, Serialize)]
#[serde(from = "String", into = "String")]
#[non_exhaustive]
pub enum ForkFinish {
    EndTurn,
    MaxTokens,
    ToolCallRefused,
    ModelError,
    Unknown(String),
}

impl ForkFinish {
    pub fn parse(value: &str) -> Self {
        match value {
            "end_turn" => Self::EndTurn,
            "max_tokens" => Self::MaxTokens,
            "tool_call_refused" => Self::ToolCallRefused,
            "model_error" => Self::ModelError,
            value => Self::Unknown(value.to_owned()),
        }
    }

    pub fn as_str(&self) -> &str {
        match self {
            Self::EndTurn => "end_turn",
            Self::MaxTokens => "max_tokens",
            Self::ToolCallRefused => "tool_call_refused",
            Self::ModelError => "model_error",
            Self::Unknown(value) => value,
        }
    }
}

impl From<String> for ForkFinish {
    fn from(value: String) -> Self {
        Self::parse(&value)
    }
}

impl From<ForkFinish> for String {
    fn from(value: ForkFinish) -> Self {
        value.as_str().to_owned()
    }
}

/// A stored ask answer. Replays return all three members unchanged, including
/// usage incurred by a tool refusal or a model failure.
#[derive(Clone, Debug, PartialEq, Eq, Deserialize, Serialize)]
#[non_exhaustive]
pub struct ForkAskReply {
    pub answer: String,
    pub finish: ForkFinish,
    pub usage: RunUsage,
}

impl ForkAskReply {
    pub fn new(answer: impl Into<String>, finish: ForkFinish, usage: RunUsage) -> Self {
        Self {
            answer: answer.into(),
            finish,
            usage,
        }
    }
}

/// Strict `session.fork_end` request, scoped to the preparing principal.
#[derive(Clone, Debug, PartialEq, Eq, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
#[non_exhaustive]
pub struct ForkEndRequest {
    pub fork_id: String,
}

impl ForkEndRequest {
    pub fn new(fork_id: impl Into<String>) -> Self {
        Self {
            fork_id: fork_id.into(),
        }
    }
}

/// Idempotent end acknowledgement. Producers always set `ended` to true.
#[derive(Clone, Debug, PartialEq, Eq, Deserialize, Serialize)]
#[non_exhaustive]
pub struct ForkEndReply {
    pub fork_id: String,
    pub ended: bool,
}

impl ForkEndReply {
    pub fn new(fork_id: impl Into<String>) -> Self {
        Self {
            fork_id: fork_id.into(),
            ended: true,
        }
    }
}

/// Decode the optional admission list. Absence and an empty list both admit
/// nobody; null, non-strings, empty principals and exact duplicates are invalid.
pub fn plan_callers(plan: &Map<String, Value>) -> Result<Vec<String>, &'static str> {
    let Some(raw) = plan.get(PLAN_FORK_CALLERS) else {
        return Ok(Vec::new());
    };
    let callers: Vec<String> =
        serde_json::from_value(raw.clone()).map_err(|_| PLAN_FORK_CALLERS_FIELD)?;
    let mut seen = std::collections::BTreeSet::new();
    if callers
        .iter()
        .any(|caller| caller.is_empty() || !seen.insert(caller))
    {
        return Err(PLAN_FORK_CALLERS_FIELD);
    }
    Ok(callers)
}

/// Admission and refresh validation. An error means `invalid_params` naming
/// `plan.fork_callers`, before any writes. Without the group, leave the member
/// uninterpreted and verbatim, even when its shape would otherwise be invalid.
pub fn check_plan<S: AsRef<str>>(
    plan: &Map<String, Value>,
    declared: &[S],
) -> Result<(), &'static str> {
    if declared
        .iter()
        .any(|group| group.as_ref() == capabilities::FORK_ANSWER)
    {
        plan_callers(plan).map(|_| ())
    } else {
        Ok(())
    }
}

/// Prepare admission from the route's attested principal and the most recently
/// accepted policy. Call only for a runner declaring `fork_answer`. No implicit
/// owner grant applies; false is `scope_owner_mismatch`. Ask/end additionally
/// require a principal-bound, unrevoked fork, which the runner must track.
pub fn admit_prepare(
    effective: &Map<String, Value>,
    pending: Option<&Map<String, Value>>,
    caller: &str,
) -> Result<bool, &'static str> {
    let plan = session_change::admission_plan(effective, pending);
    Ok(plan_callers(plan)?
        .iter()
        .any(|principal| principal == caller))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{
        send::SendRequest,
        session_change::{RefreshRequest, RelaunchRefreshRequest},
        vectors,
    };
    use serde_json::json;

    #[test]
    fn fork_answer_wire_vectors_and_constructors() {
        let file = vectors::load("fork-answer.json");
        assert_eq!(
            vectors::round_trip::<ForkPrepareRequest>("prepare", &file["prepare"]["request"]),
            ForkPrepareRequest::new()
        );
        assert_eq!(
            vectors::round_trip::<ForkPrepareReply>("prepared", &file["prepare"]["reply"]),
            ForkPrepareReply::new("fork-1")
        );
        let ask: ForkAskRequest = vectors::round_trip("ask", &file["ask"]["request"]);
        assert_eq!(ask, ForkAskRequest::new("fork-1", "ask-1", "What changed?"));
        assert_eq!(ask.check(), Ok(()));
        let reply: ForkAskReply = vectors::round_trip("answer", &file["ask"]["reply"]);
        assert_eq!(reply.usage.cached_input_tokens, Some(110));
        assert_eq!(
            reply,
            ForkAskReply::new(
                "The title changed.",
                ForkFinish::EndTurn,
                reply.usage.clone()
            )
        );
        assert_eq!(
            vectors::round_trip::<ForkEndRequest>("end", &file["end"]["request"]),
            ForkEndRequest::new("fork-1")
        );
        assert_eq!(
            vectors::round_trip::<ForkEndReply>("ended", &file["end"]["reply"]),
            ForkEndReply::new("fork-1")
        );
        for value in file["finishes"].as_array().unwrap() {
            let finish: ForkFinish = vectors::round_trip("finish", value);
            assert_eq!(finish.as_str(), value.as_str().unwrap());
        }
        assert_eq!(
            ForkFinish::parse("future-finish"),
            ForkFinish::Unknown("future-finish".into())
        );
        for finish in [
            ForkFinish::MaxTokens,
            ForkFinish::ToolCallRefused,
            ForkFinish::ModelError,
        ] {
            assert_eq!(ForkFinish::parse(finish.as_str()), finish);
        }
    }

    #[test]
    fn fork_answer_requests_are_strict_and_bounds_count_scalars() {
        assert!(
            serde_json::from_value::<ForkPrepareRequest>(json!({"session_id":"other"})).is_err()
        );
        assert!(serde_json::from_value::<ForkAskRequest>(
            json!({"fork_id":"f", "ask_id":"a", "question":"q", "future":true})
        )
        .is_err());
        assert!(
            serde_json::from_value::<ForkEndRequest>(json!({"fork_id":"f", "future":true}))
                .is_err()
        );
        let file = vectors::load("fork-answer.json");
        for case in file["refused_asks"].as_array().unwrap() {
            let ask: ForkAskRequest = serde_json::from_value(case["request"].clone()).unwrap();
            assert_eq!(ask.check(), Err(case["field"].as_str().unwrap()));
        }
        assert_eq!(
            ForkAskRequest::new("f", "🦀".repeat(256), " ").check(),
            Ok(())
        );
        assert_eq!(
            ForkAskRequest::new("f", "🦀".repeat(257), "q").check(),
            Err("ask_id")
        );
    }

    #[test]
    fn fork_answer_plan_validation_at_admission_and_refresh() {
        let file = vectors::load("fork-answer.json");
        for case in file["plans"].as_array().unwrap() {
            let plan = case["plan"].as_object().unwrap();
            assert_eq!(
                serde_json::to_value(plan_callers(plan).unwrap()).unwrap(),
                case["callers"]
            );
            assert_eq!(check_plan(plan, &[capabilities::FORK_ANSWER]), Ok(()));
        }
        for value in file["invalid_plans"].as_array().unwrap() {
            let plan = value.as_object().unwrap().clone();
            let original = plan.clone();
            assert_eq!(
                check_plan(&plan, &[capabilities::FORK_ANSWER]),
                Err(PLAN_FORK_CALLERS_FIELD)
            );
            assert_eq!(check_plan::<&str>(&plan, &[]), Ok(()));
            assert_eq!(plan, original);
            let send: SendRequest =
                serde_json::from_value(json!({"prompt":"p", "send_id":"s", "plan":plan})).unwrap();
            assert_eq!(
                send.check_plan(&[capabilities::PLANS, capabilities::FORK_ANSWER]),
                Err(PLAN_FORK_CALLERS_FIELD)
            );
            assert_eq!(send.check_plan(&[capabilities::PLANS]), Ok(()));
            let refresh = RefreshRequest::new(plan.clone(), "next_run", 1);
            let relaunch = RelaunchRefreshRequest::from(refresh.clone());
            let adopt = crate::adopt::AdoptRequest::new("adopt-1", plan.clone());
            assert_eq!(
                adopt.check_fork_callers(&[capabilities::FORK_ANSWER]),
                Err(PLAN_FORK_CALLERS_FIELD)
            );
            assert_eq!(adopt.check_fork_callers::<&str>(&[]), Ok(()));
            assert_eq!(serde_json::to_value(adopt).unwrap()["plan"], value.clone());
            assert_eq!(
                refresh.check_fork_callers(&[capabilities::FORK_ANSWER]),
                Err(PLAN_FORK_CALLERS_FIELD)
            );
            assert_eq!(
                relaunch.check_fork_callers(&[capabilities::FORK_ANSWER]),
                Err(PLAN_FORK_CALLERS_FIELD)
            );
            assert_eq!(refresh.check_fork_callers::<&str>(&[]), Ok(()));
            assert_eq!(relaunch.check_fork_callers::<&str>(&[]), Ok(()));
            assert_eq!(serde_json::to_value(send).unwrap()["plan"], value.clone());
            assert_eq!(
                serde_json::to_value(refresh).unwrap()["plan"],
                value.clone()
            );
            assert_eq!(
                serde_json::to_value(relaunch).unwrap()["plan"],
                value.clone()
            );
        }
    }

    #[test]
    fn fork_answer_prepare_uses_accepted_policy_without_owner_privilege() {
        let effective = json!({"fork_callers":["caller-a"]})
            .as_object()
            .unwrap()
            .clone();
        let pending = json!({"fork_callers":["caller-b"]})
            .as_object()
            .unwrap()
            .clone();
        assert_eq!(admit_prepare(&effective, None, "caller-a"), Ok(true));
        assert_eq!(admit_prepare(&effective, None, "owner"), Ok(false));
        assert_eq!(
            admit_prepare(&effective, Some(&pending), "caller-a"),
            Ok(false)
        );
        assert_eq!(
            admit_prepare(&effective, Some(&pending), "caller-b"),
            Ok(true)
        );
        assert_eq!(admit_prepare(&Map::new(), None, "owner"), Ok(false));
        assert_eq!(
            admit_prepare(&effective, Some(&Map::new()), "caller-a"),
            Ok(false)
        );
    }
}
