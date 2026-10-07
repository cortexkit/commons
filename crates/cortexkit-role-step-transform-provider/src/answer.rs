//! A hook's answer, and which answers each hook and phase allows.
//!
//! `pre_user`, `post_assistant` and `post_tool` answer with operations, not
//! a new value: `prepend(text)`, `append(text)` or `replace(value)`, each
//! addressed to one of the subject's text blocks by index. The runner
//! applies them in order ([`apply_ops`]) and records each one with the
//! provider that returned it, so attribution needs no diffing. `pre_tool`
//! answers by phase: `mutate` may rewrite the input, `validate` may deny,
//! and `approve` may ask a human.
//!
//! An answer the hook does not allow is refused before anything is applied
//! or recorded ([`check_answer`]), and the runner then treats the hook as
//! unavailable for that call: the subscription's `on_unavailable` decides.

use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::{
    grant::UserGrant,
    hook::Subject,
    subscription::{Hook, Op, Phase},
};

/// One operation on one of the subject's text blocks. Strict on `op`.
///
/// `block` indexes the subject's `blocks`, which list only its text blocks
/// in message order. An operation changes that block's text and nothing
/// else: it never adds, removes or reorders blocks.
#[derive(Clone, Debug, PartialEq, Eq, Deserialize, Serialize)]
#[serde(tag = "op", rename_all = "snake_case")]
pub enum Operation {
    /// Put `text` before the block's text, in the same block. Several
    /// providers' prepends stack outward.
    Prepend {
        block: u32,
        text: String,
        /// A short note for whoever reads the attribution.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        note: Option<String>,
    },
    /// Put `text` after the block's text, in the same block. Appends stack
    /// in order.
    Append {
        block: u32,
        text: String,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        note: Option<String>,
    },
    /// Replace the block's text, as it stands at this point, with `value`.
    Replace {
        block: u32,
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

    /// The index of the text block the operation acts on.
    pub fn block(&self) -> u32 {
        match self {
            Self::Prepend { block, .. }
            | Self::Append { block, .. }
            | Self::Replace { block, .. } => *block,
        }
    }

    /// Apply the operation to one block's text.
    fn apply_to(&self, text: &mut String) {
        match self {
            Self::Prepend { text: before, .. } => text.insert_str(0, before),
            Self::Append { text: after, .. } => text.push_str(after),
            Self::Replace { value, .. } => value.clone_into(text),
        }
    }
}

/// What happens when an approval question expires unanswered. Strict: an
/// unknown value does not decode, so the answer is refused and the hook is
/// unavailable; nothing runs on a policy the runner does not understand.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Deserialize, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum OnExpiry {
    /// The call ends with the tool-result reason `pre_tool_expired`.
    Deny,
}

/// What an approval arriving after the run moved on does. Strict, like
/// [`OnExpiry`].
#[derive(Clone, Copy, Debug, PartialEq, Eq, Deserialize, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum LateExecution {
    /// The call executes.
    Execute,
    /// Nothing executes; the human is told the answer came too late.
    NotifyOnly,
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
    /// The answers offered, an enumerated list of strings, when the gate
    /// wants more than allow and decline. Omitted when empty.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub options: Vec<String>,
    /// When the question expires: an absolute time in milliseconds since the
    /// Unix epoch, not a duration.
    pub expires_at_ms: u64,
    /// What happens when it expires.
    pub on_expiry: OnExpiry,
    /// Whether the call can cause damage that cannot be undone.
    pub material_damage: bool,
    /// What an answer arriving after the run moved on does.
    pub late_execution: LateExecution,
}

impl ApprovalAsk {
    pub fn new(
        prompt: impl Into<String>,
        expires_at_ms: u64,
        on_expiry: OnExpiry,
        material_damage: bool,
        late_execution: LateExecution,
    ) -> Self {
        Self {
            prompt: prompt.into(),
            options: Vec::new(),
            expires_at_ms,
            on_expiry,
            material_damage,
            late_execution,
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
    /// Operations on the subject's text blocks, applied in order.
    /// `pre_user`, `post_assistant` and `post_tool` only.
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

/// Why the runner refuses an answer before applying it. Any of these makes
/// the hook unavailable for the call.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum AnswerProblem {
    /// The hook or phase does not allow this kind of answer.
    AnswerNotAllowed,
    /// An operation the session's subscription does not name.
    OpNotSubscribed,
    /// An operation the tool does not accept on its result (the tool's
    /// `result_ops` in `tool-provider/v1`).
    OpNotAcceptedByTool,
    /// An operation's `block` is not an index into the subject's `blocks`.
    BlockOutOfRange,
    /// A `replace` on `post_tool` that no user-tier grant in the frozen
    /// plan's `user_grants` covers ([`check_post_tool_grant`]).
    ReplaceNotGranted,
}

impl AnswerProblem {
    pub fn name(self) -> &'static str {
        match self {
            Self::AnswerNotAllowed => "answer_not_allowed",
            Self::OpNotSubscribed => "op_not_subscribed",
            Self::OpNotAcceptedByTool => "op_not_accepted_by_tool",
            Self::BlockOutOfRange => "block_out_of_range",
            Self::ReplaceNotGranted => "replace_not_granted",
        }
    }
}

/// Check `answer` against the hook it answers, the operations the
/// session's subscription names, on `post_tool` the operations the tool
/// accepts on its result, and each operation's block index against the
/// subject's `blocks`. `tool_result_ops` of `None` means the tool accepts
/// every operation, the default `tool-provider/v1` gives.
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
        let blocks = subject.blocks().map_or(0, <[String]>::len);
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
            if operation.block() as usize >= blocks {
                return Err(AnswerProblem::BlockOutOfRange);
            }
        }
    }
    Ok(())
}

/// Check a `post_tool` answer from `provider` against the user-tier grants
/// frozen in the plan's `user_grants` ([`crate::grant::plan_user_grants`]):
/// a `replace` is allowed only when a grant names the provider, the
/// `post_tool` hook and the subject's tool. Answers on other hooks, and
/// answers without a `replace`, pass this check.
pub fn check_post_tool_grant(
    subject: &Subject,
    provider: &str,
    grants: &[UserGrant],
    answer: &HookAnswer,
) -> Result<(), AnswerProblem> {
    let (Subject::PostTool { tool, .. }, HookAnswer::Ops { ops }) = (subject, answer) else {
        return Ok(());
    };
    let replaces = ops.iter().any(|operation| operation.op() == Op::Replace);
    if replaces
        && !grants
            .iter()
            .any(|grant| grant.allows_post_tool_replace(provider, tool))
    {
        return Err(AnswerProblem::ReplaceNotGranted);
    }
    Ok(())
}

/// Apply `ops` in order to a subject's text blocks and return the new
/// blocks. Each operation changes only the block it names; the number of
/// blocks never changes. An index outside `blocks` is refused, and nothing
/// is applied.
pub fn apply_ops(blocks: &[String], ops: &[Operation]) -> Result<Vec<String>, AnswerProblem> {
    if ops.iter().any(|op| op.block() as usize >= blocks.len()) {
        return Err(AnswerProblem::BlockOutOfRange);
    }
    let mut blocks = blocks.to_vec();
    for operation in ops {
        operation.apply_to(&mut blocks[operation.block() as usize]);
    }
    Ok(blocks)
}

/// Apply `ops` to a whole message's parts, in the runner's own message
/// schema. `text_of` returns a part's text when the part is a text block and
/// `None` for every other part (images, tool calls, signed thinking). Block
/// `n` is the `n`-th text part in order; every other part is left exactly as
/// it was, and the number and order of parts never change.
pub fn apply_to_parts<P>(
    parts: &mut [P],
    mut text_of: impl FnMut(&mut P) -> Option<&mut String>,
    ops: &[Operation],
) -> Result<(), AnswerProblem> {
    let mut texts: Vec<&mut String> = parts.iter_mut().filter_map(&mut text_of).collect();
    if ops.iter().any(|op| op.block() as usize >= texts.len()) {
        return Err(AnswerProblem::BlockOutOfRange);
    }
    for operation in ops {
        operation.apply_to(texts[operation.block() as usize]);
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
            let name = case["name"].as_str().unwrap();
            vectors::refused::<HookAnswer>(name, &case["answer"]);
            // An unknown policy value is refused by name: the error says
            // which value it did not know.
            if let Some(value) = case.get("unknown_value") {
                let error = serde_json::from_value::<HookAnswer>(case["answer"].clone())
                    .unwrap_err()
                    .to_string();
                let value = value.as_str().unwrap();
                assert!(
                    error.contains(&format!("unknown variant `{value}`")),
                    "{name}: {error}"
                );
            }
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

    fn text(value: &str) -> String {
        value.to_owned()
    }

    fn append(block: u32, text: &str) -> Operation {
        Operation::Append {
            block,
            text: text.into(),
            note: None,
        }
    }

    #[test]
    fn multi_block_subjects_and_operations_round_trip() {
        let call = HookCall::new(
            "s",
            "broca",
            Subject::PostAssistant {
                step_id: "st".into(),
                blocks: vec![text("one"), text("two")],
            },
        );
        let wire = serde_json::json!({"session": "s", "harness": "broca", "params": {}, "hook": "post_assistant",
            "step_id": "st", "blocks": ["one", "two"]});
        assert_eq!(vectors::round_trip::<HookCall>("call", &wire), call);
        assert_eq!(call.subject.blocks().unwrap().len(), 2);
        let answer = HookAnswer::Ops {
            ops: vec![
                Operation::Prepend {
                    block: 0,
                    text: "> ".into(),
                    note: Some("tag".into()),
                },
                append(1, "!"),
                Operation::Replace {
                    block: 1,
                    value: "2".into(),
                    note: None,
                },
            ],
        };
        let wire = serde_json::json!({"answer": "ops", "ops": [
            {"op": "prepend", "block": 0, "text": "> ", "note": "tag"},
            {"op": "append", "block": 1, "text": "!"},
            {"op": "replace", "block": 1, "value": "2"}
        ]});
        assert_eq!(vectors::round_trip::<HookAnswer>("ops", &wire), answer);
    }

    #[test]
    fn an_operation_changes_only_its_block_and_never_the_count() {
        let blocks = vec![text("alpha"), text("beta"), text("gamma")];
        let out = apply_ops(&blocks, &[append(1, "+"), append(1, "!")]).unwrap();
        assert_eq!(out, [text("alpha"), text("beta+!"), text("gamma")]);
        let out = apply_ops(
            &blocks,
            &[
                Operation::Prepend {
                    block: 2,
                    text: "a ".into(),
                    note: None,
                },
                Operation::Prepend {
                    block: 2,
                    text: "b ".into(),
                    note: None,
                },
            ],
        )
        .unwrap();
        // Prepends stack outward, within the same block.
        assert_eq!(out, [text("alpha"), text("beta"), text("b a gamma")]);
        let replace = Operation::Replace {
            block: 0,
            value: String::new(),
            note: None,
        };
        let out = apply_ops(&blocks, std::slice::from_ref(&replace)).unwrap();
        assert_eq!(out.len(), blocks.len(), "replace keeps the block count");
        assert_eq!(out, [text(""), text("beta"), text("gamma")]);
    }

    #[test]
    fn an_out_of_range_block_is_refused_by_name_and_nothing_applies() {
        let blocks = vec![text("only")];
        let outcome = apply_ops(&blocks, &[append(0, "x"), append(1, "y")]);
        assert_eq!(
            outcome.map_err(AnswerProblem::name),
            Err("block_out_of_range")
        );
        let subject = Subject::PreUser {
            blocks,
            mark: None,
            delivery: None,
        };
        let answer = HookAnswer::Ops {
            ops: vec![append(1, "y")],
        };
        assert_eq!(
            check_answer(&subject, &[Op::Append], None, &answer).map_err(AnswerProblem::name),
            Err("block_out_of_range")
        );
    }

    /// The runner's text accessor for a message part in the shape the
    /// vectors' messages use: `{type: "text", text}` is a text block; every
    /// other part is not.
    fn text_part(part: &mut Value) -> Option<&mut String> {
        if part.get("type")? != "text" {
            return None;
        }
        match part.get_mut("text")? {
            Value::String(text) => Some(text),
            _ => None,
        }
    }

    #[test]
    fn a_block_index_counts_only_text_blocks_and_leaves_the_rest_byte_identical() {
        let original = serde_json::json!([
            {"type": "thinking", "thinking": "plan", "signature": "c2lnbmVk"},
            {"type": "text", "text": "first"},
            {"type": "image", "source": {"type": "base64", "data": "iVBORw0K"}},
            {"type": "tool_use", "id": "call_0", "name": "read", "input": {"path": "a"}},
            {"type": "text", "text": "second"},
            {"type": "text", "text": "third"}
        ]);
        let mut parts = original.as_array().unwrap().clone();
        apply_to_parts(
            &mut parts,
            text_part,
            &[
                append(1, " (edited)"),
                Operation::Prepend {
                    block: 1,
                    text: "> ".into(),
                    note: None,
                },
            ],
        )
        .unwrap();
        assert_eq!(parts.len(), 6, "no part added or removed");
        assert_eq!(
            parts[4]["text"], "> second (edited)",
            "block 1 is the second text part"
        );
        for index in [0, 1, 2, 3, 5] {
            assert_eq!(
                serde_json::to_vec(&parts[index]).unwrap(),
                serde_json::to_vec(&original[index]).unwrap(),
                "part {index} changed"
            );
        }
        // The message has three text parts, so text-block indexes run 0 to 2.
        // An operation on block 3 is out of range: the whole call is refused,
        // and no part changes, not even block 0, which an earlier operation
        // in the same list targeted.
        let mut untouched = original.as_array().unwrap().clone();
        let outcome = apply_to_parts(&mut untouched, text_part, &[append(0, "x"), append(3, "y")]);
        assert_eq!(outcome, Err(AnswerProblem::BlockOutOfRange));
        assert_eq!(Value::Array(untouched), original);
    }

    #[test]
    fn the_approval_question_round_trips_with_absolute_expiry_and_known_policies() {
        for (on_expiry, late, late_wire) in [
            (OnExpiry::Deny, LateExecution::Execute, "execute"),
            (OnExpiry::Deny, LateExecution::NotifyOnly, "notify_only"),
        ] {
            let ask = ApprovalAsk::new("Run?", 1_790_000_723_000, on_expiry, true, late)
                .with_options(vec!["run".into(), "skip".into()]);
            let wire = serde_json::json!({"prompt": "Run?", "options": ["run", "skip"],
                "expires_at_ms": 1_790_000_723_000u64, "on_expiry": "deny",
                "material_damage": true, "late_execution": late_wire});
            assert_eq!(vectors::round_trip::<ApprovalAsk>(late_wire, &wire), ask);
        }
        let bare = ApprovalAsk::new("Run?", 5, OnExpiry::Deny, false, LateExecution::Execute);
        assert!(serde_json::to_value(&bare)
            .unwrap()
            .get("options")
            .is_none());
    }

    #[test]
    fn a_post_tool_replace_needs_a_user_grant() {
        let subject = Subject::PostTool {
            step_id: "st".into(),
            tool: "read".into(),
            tool_call_id: "call_1".into(),
            call_key: None,
            blocks: vec![text("long output")],
            is_error: false,
        };
        let replace = HookAnswer::Ops {
            ops: vec![Operation::Replace {
                block: 0,
                value: "short".into(),
                note: None,
            }],
        };
        let grants = [UserGrant::new(
            "acme-trim",
            Hook::PostTool,
            vec!["read".into()],
        )];
        assert_eq!(
            check_post_tool_grant(&subject, "acme-trim", &grants, &replace),
            Ok(())
        );
        assert_eq!(
            check_post_tool_grant(&subject, "acme-tags", &grants, &replace)
                .map_err(AnswerProblem::name),
            Err("replace_not_granted")
        );
        assert_eq!(
            check_post_tool_grant(&subject, "acme-trim", &[], &replace),
            Err(AnswerProblem::ReplaceNotGranted)
        );
        let append_only = HookAnswer::Ops {
            ops: vec![append(0, "x")],
        };
        assert_eq!(
            check_post_tool_grant(&subject, "acme-tags", &[], &append_only),
            Ok(())
        );
    }
}
