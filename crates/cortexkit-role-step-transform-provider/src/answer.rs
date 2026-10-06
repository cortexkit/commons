//! A hook's answer, and which answers each hook and phase allows.
//!
//! `pre_user`, `post_assistant` and `post_tool` answer with operations, not
//! a new value: `prepend(text)`, `append(text)` or `replace(value)`. The
//! runner applies them in order and records each one with the provider that
//! returned it, so attribution needs no diffing. `pre_tool` answers by
//! phase: `mutate` may rewrite the input, `validate` may deny, and `approve`
//! may ask a human.
//!
//! An answer the hook does not allow is refused before anything is applied
//! or recorded ([`check_answer`]), and the runner then treats the hook as
//! unavailable.

use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::{
    hook::Subject,
    subscription::{Hook, Op, Phase},
};

/// One operation on the subject's text. Strict on `op`.
#[derive(Clone, Debug, PartialEq, Eq, Deserialize, Serialize)]
#[serde(tag = "op", rename_all = "snake_case")]
pub enum Operation {
    /// Put `text` before the subject. Several providers' prepends stack
    /// outward.
    Prepend {
        text: String,
        /// A short note for whoever reads the attribution.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        note: Option<String>,
    },
    /// Put `text` after the subject. Appends stack in order.
    Append {
        text: String,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        note: Option<String>,
    },
    /// Replace the subject's text with `value`, as it stands at this point.
    Replace {
        value: String,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        note: Option<String>,
    },
}

impl Operation {
    pub fn op(&self) -> Op {
        match self {
            Self::Prepend { .. } => Op::Prepend,
            Self::Append { .. } => Op::Append,
            Self::Replace { .. } => Op::Replace,
        }
    }
}

/// What an approve-phase hook asks the human. The elicitation role defines
/// how the ask is filed and answered; this role carries only what the hook
/// decides.
#[derive(Clone, Debug, PartialEq, Eq, Deserialize, Serialize)]
#[non_exhaustive]
pub struct ApprovalAsk {
    /// The question, shown on the card beside the tool, its target and a
    /// short form of the final input.
    pub prompt: String,
    /// The answers offered, when the gate wants more than allow and decline.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub options: Vec<String>,
    /// How long the question stays open, in milliseconds.
    pub expiry_ms: u64,
    /// What happens when it expires: `deny` (the call ends
    /// `pre_tool_expired`), decoded open.
    pub on_expiry: String,
    /// Whether the call can cause damage that cannot be undone.
    pub material_damage: bool,
    /// What an answer arriving after the run moved on does: `execute`, or
    /// `notify_only` (nothing executes). Decoded open.
    pub late_execution: String,
}

impl ApprovalAsk {
    pub fn new(
        prompt: impl Into<String>,
        expiry_ms: u64,
        on_expiry: impl Into<String>,
        material_damage: bool,
        late_execution: impl Into<String>,
    ) -> Self {
        Self {
            prompt: prompt.into(),
            options: Vec::new(),
            expiry_ms,
            on_expiry: on_expiry.into(),
            material_damage,
            late_execution: late_execution.into(),
        }
    }

    pub fn with_options(mut self, options: Vec<String>) -> Self {
        self.options = options;
        self
    }
}

/// A hook's answer. Strict on `answer`: an unknown answer does not decode,
/// and the runner treats the hook as unavailable.
#[derive(Clone, Debug, PartialEq, Deserialize, Serialize)]
#[serde(tag = "answer", rename_all = "snake_case")]
pub enum HookAnswer {
    /// No change. Every hook and phase allows it.
    Pass,
    /// Operations on the subject, applied in order. `pre_user`,
    /// `post_assistant` and `post_tool` only.
    Ops { ops: Vec<Operation> },
    /// The input that executes instead. `pre_tool` `mutate` only. Must be a
    /// pure function of the call and the frozen config.
    Mutate {
        input: Value,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        note: Option<String>,
    },
    /// Deny the call: `text` becomes its error result, with reason
    /// `pre_tool_denied`. `pre_tool` `validate` only.
    Deny { text: String },
    /// Ask a human before the call executes. `pre_tool` `approve` only.
    Ask { ask: ApprovalAsk },
}

impl HookAnswer {
    /// The answer's name as it appears in `answer`.
    pub fn name(&self) -> &'static str {
        match self {
            Self::Pass => "pass",
            Self::Ops { .. } => "ops",
            Self::Mutate { .. } => "mutate",
            Self::Deny { .. } => "deny",
            Self::Ask { .. } => "ask",
        }
    }
}

/// Why the runner refuses an answer before applying it.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum AnswerProblem {
    /// The hook or phase does not allow this kind of answer.
    AnswerNotAllowed,
    /// An operation the session's subscription does not name.
    OpNotSubscribed,
    /// An operation the tool does not accept on its result (the tool's
    /// `result_ops` in `tool-provider/v1`).
    OpNotAcceptedByTool,
}

impl AnswerProblem {
    pub fn name(self) -> &'static str {
        match self {
            Self::AnswerNotAllowed => "answer_not_allowed",
            Self::OpNotSubscribed => "op_not_subscribed",
            Self::OpNotAcceptedByTool => "op_not_accepted_by_tool",
        }
    }
}

/// Check `answer` against the hook it answers, the operations the
/// session's subscription names, and, on `post_tool`, the operations the
/// tool accepts on its result. `tool_result_ops` of `None` means the tool
/// accepts every operation, the default `tool-provider/v1` gives.
pub fn check_answer(
    subject: &Subject,
    subscribed_ops: &[Op],
    tool_result_ops: Option<&[Op]>,
    answer: &HookAnswer,
) -> Result<(), AnswerProblem> {
    let allowed = match (subject.hook(), subject.phase(), answer) {
        (_, _, HookAnswer::Pass) => true,
        (hook, _, HookAnswer::Ops { .. }) => hook.takes_ops(),
        (Hook::PreTool, Some(Phase::Mutate), HookAnswer::Mutate { .. }) => true,
        (Hook::PreTool, Some(Phase::Validate), HookAnswer::Deny { .. }) => true,
        (Hook::PreTool, Some(Phase::Approve), HookAnswer::Ask { .. }) => true,
        _ => false,
    };
    if !allowed {
        return Err(AnswerProblem::AnswerNotAllowed);
    }
    if let HookAnswer::Ops { ops } = answer {
        for operation in ops {
            let op = operation.op();
            if !subscribed_ops.contains(&op) {
                return Err(AnswerProblem::OpNotSubscribed);
            }
            if subject.hook() == Hook::PostTool
                && tool_result_ops.is_some_and(|accepted| !accepted.contains(&op))
            {
                return Err(AnswerProblem::OpNotAcceptedByTool);
            }
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{hook::HookCall, vectors};

    #[test]
    fn hook_answer_vectors_check_as_recorded() {
        let file = vectors::load("hook-answers.json");
        for case in vectors::cases(&file, "answers") {
            let name = case["name"].as_str().unwrap();
            let answer = vectors::round_trip::<HookAnswer>(name, &case["answer"]);
            assert_eq!(answer.name(), case["answer"]["answer"].as_str().unwrap());
        }
        for case in vectors::cases(&file, "tolerated") {
            let name = case["name"].as_str().unwrap();
            serde_json::from_value::<HookAnswer>(case["answer"].clone())
                .unwrap_or_else(|e| panic!("{name}: {e}"));
        }
        for case in vectors::cases(&file, "undecodable") {
            vectors::refused::<HookAnswer>(case["name"].as_str().unwrap(), &case["answer"]);
        }
        for case in vectors::cases(&file, "checks") {
            let name = case["name"].as_str().unwrap();
            let call: HookCall = serde_json::from_value(case["request"].clone())
                .unwrap_or_else(|e| panic!("{name}: request: {e}"));
            let answer: HookAnswer = serde_json::from_value(case["answer"].clone())
                .unwrap_or_else(|e| panic!("{name}: answer: {e}"));
            let subscribed: Vec<Op> =
                serde_json::from_value(case["subscribed_ops"].clone()).unwrap();
            let accepted: Option<Vec<Op>> = case
                .get("tool_result_ops")
                .map(|ops| serde_json::from_value(ops.clone()).unwrap());
            let outcome = check_answer(&call.subject, &subscribed, accepted.as_deref(), &answer);
            match case["problem"].as_str() {
                None => assert_eq!(outcome, Ok(()), "{name}"),
                Some(problem) => {
                    assert_eq!(outcome.map_err(AnswerProblem::name), Err(problem), "{name}")
                }
            }
        }
    }
}
