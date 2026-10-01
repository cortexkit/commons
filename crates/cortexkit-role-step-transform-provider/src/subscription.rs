//! Subscriptions: what a provider declares it can do, and what a plan
//! subscribes it to.
//!
//! The provider's declaration (`transform.declare`) lists, per hook (and per
//! phase on `pre_tool`), the tools it can act on, the operations it may
//! return, what the runner does when it is unavailable, and its time budget.
//! A plan item's subscriptions choose within those bounds; the runner refuses
//! at admission a plan that subscribes a provider to more than it declared.
//!
//! The session has at most one reduction owner, its compaction provider.
//! Only the reduction owner may `replace` on `pre_user` and
//! `post_assistant`. Every other step transform is preserving: it may
//! prepend or append. The reduction owner's hooks run first; the preserving
//! transforms then run in plan order.

use serde::{Deserialize, Serialize};
use serde_json::{Map, Value};

/// A hook. Strict: an unknown hook does not decode.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Deserialize, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum Hook {
    /// A user message, or a steered or queued prompt, before it is written.
    PreUser,
    /// A completed assistant message, before it is written. Text only.
    PostAssistant,
    /// A tool call, before it executes, in three phases.
    PreTool,
    /// A tool's result, before it is written.
    PostTool,
}

impl Hook {
    pub const ALL: [Hook; 4] = [
        Hook::PreUser,
        Hook::PostAssistant,
        Hook::PreTool,
        Hook::PostTool,
    ];

    /// Whether this hook's answers are operations (`prepend`, `append`,
    /// `replace`). `pre_tool` answers by phase instead.
    pub fn takes_ops(self) -> bool {
        self != Hook::PreTool
    }
}

/// A `pre_tool` phase, run in this order where the runner owns hook order.
/// Strict.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Deserialize, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum Phase {
    /// May rewrite the input that executes. Must be a pure function of the
    /// call and the frozen config.
    Mutate,
    /// May deny the call.
    Validate,
    /// May ask a human before the call executes.
    Approve,
}

/// An operation on a hook's subject. Strict.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Deserialize, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum Op {
    Prepend,
    Append,
    Replace,
}

/// What the runner does when a hook times out, fails, or answers something
/// it may not. Strict.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Deserialize, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum OnUnavailable {
    /// Go on without the hook's output, and record that.
    Pass,
    /// `pre_tool`: deny the call (`pre_tool_unavailable`). `post_tool`:
    /// withhold the result (`post_tool_unavailable`). `pre_user`: refuse the
    /// step (`pre_user_unavailable`).
    Refuse,
}

/// One subscription in a plan item, as the fetch plan carries it: `{hook,
/// tools?, ops, phase?}`. Decoded leniently on fields, strictly on values.
#[derive(Clone, Debug, PartialEq, Eq, Deserialize, Serialize)]
pub struct Subscription {
    pub hook: Hook,
    /// The tools the subscription covers; absent means every tool. Only on
    /// `pre_tool` and `post_tool`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub tools: Option<Vec<String>>,
    /// The operations the provider may return. Empty on `pre_tool`.
    pub ops: Vec<Op>,
    /// Present on `pre_tool` only.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub phase: Option<Phase>,
}

/// One subscription a provider declares: the bounds a plan may choose
/// within, plus what the runner does when the hook is unavailable and how
/// long it may take.
#[derive(Clone, Debug, PartialEq, Eq, Deserialize, Serialize)]
pub struct DeclaredSubscription {
    pub hook: Hook,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub phase: Option<Phase>,
    /// The tools the provider can act on; absent means every tool.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub tools: Option<Vec<String>>,
    pub ops: Vec<Op>,
    /// Absent means `refuse`, except on `post_assistant`, which always
    /// passes through ([`DeclaredSubscription::effective_on_unavailable`]).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub on_unavailable: Option<OnUnavailable>,
    /// The provider's time budget per call, in milliseconds. The runner caps
    /// it with its own engine-wide limit.
    pub budget_ms: u64,
}

impl DeclaredSubscription {
    /// What the runner does when this hook is unavailable.
    pub fn effective_on_unavailable(&self) -> OnUnavailable {
        if self.hook == Hook::PostAssistant {
            return OnUnavailable::Pass;
        }
        self.on_unavailable.unwrap_or(OnUnavailable::Refuse)
    }
}

/// The `transform.declare` request: the plan item's preset and params and
/// the session's composition, verbatim. Decoded leniently.
#[derive(Clone, Debug, PartialEq, Deserialize, Serialize)]
pub struct DeclareRequest {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub preset: Option<String>,
    #[serde(default)]
    pub params: Map<String, Value>,
    /// Absent on a preflight call, made before the composition exists.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub composition: Option<Map<String, Value>>,
}

/// The `transform.declare` answer. A pure function of the request and the
/// provider's configuration: the same inputs give the same bytes, and no
/// scope, agent or session identity enters it.
#[derive(Clone, Debug, PartialEq, Eq, Deserialize, Serialize)]
pub struct Declaration {
    pub subscriptions: Vec<DeclaredSubscription>,
}

/// Why a subscription, declared or planned, is malformed or out of bounds.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SubscriptionProblem {
    /// `phase` on a hook other than `pre_tool`.
    PhaseOutsidePreTool,
    /// `pre_tool` without a `phase`.
    MissingPhase,
    /// `ops` on `pre_tool`, whose answers are by phase.
    OpsOnPreTool,
    /// No `ops` on a hook that answers with operations.
    NoOps,
    /// `tools` on a hook that does not act on a tool.
    ToolsOutsideToolHooks,
    /// `on_unavailable: refuse` on `post_assistant`, which always passes.
    RefuseOnPostAssistant,
    /// A zero time budget.
    ZeroBudget,
    /// The plan subscribes to a hook and phase the provider did not declare.
    NotDeclared,
    /// The plan covers tools the declaration does not.
    ToolsNotCovered,
    /// The plan names an operation the declaration does not.
    OpNotDeclared,
    /// `replace` on `pre_user` or `post_assistant` by a provider that is not
    /// the session's reduction owner.
    ReplaceNotReductionOwner,
}

impl SubscriptionProblem {
    /// The problem's name as the vectors spell it.
    pub fn name(self) -> &'static str {
        match self {
            Self::PhaseOutsidePreTool => "phase_outside_pre_tool",
            Self::MissingPhase => "missing_phase",
            Self::OpsOnPreTool => "ops_on_pre_tool",
            Self::NoOps => "no_ops",
            Self::ToolsOutsideToolHooks => "tools_outside_tool_hooks",
            Self::RefuseOnPostAssistant => "refuse_on_post_assistant",
            Self::ZeroBudget => "zero_budget",
            Self::NotDeclared => "not_declared",
            Self::ToolsNotCovered => "tools_not_covered",
            Self::OpNotDeclared => "op_not_declared",
            Self::ReplaceNotReductionOwner => "replace_not_reduction_owner",
        }
    }
}

/// The shape rules a declared and a planned subscription share.
fn check_shape(
    hook: Hook,
    phase: Option<Phase>,
    tools: Option<&Vec<String>>,
    ops: &[Op],
) -> Result<(), SubscriptionProblem> {
    match (hook, phase) {
        (Hook::PreTool, None) => return Err(SubscriptionProblem::MissingPhase),
        (Hook::PreTool, Some(_)) => {}
        (_, Some(_)) => return Err(SubscriptionProblem::PhaseOutsidePreTool),
        (_, None) => {}
    }
    if hook.takes_ops() && ops.is_empty() {
        return Err(SubscriptionProblem::NoOps);
    }
    if !hook.takes_ops() && !ops.is_empty() {
        return Err(SubscriptionProblem::OpsOnPreTool);
    }
    if tools.is_some() && !matches!(hook, Hook::PreTool | Hook::PostTool) {
        return Err(SubscriptionProblem::ToolsOutsideToolHooks);
    }
    Ok(())
}

impl Declaration {
    /// Check every declared subscription. Returns each problem with the
    /// index of the subscription it was found in.
    pub fn check(&self) -> Vec<(usize, SubscriptionProblem)> {
        let mut problems = Vec::new();
        for (index, declared) in self.subscriptions.iter().enumerate() {
            let shape = check_shape(
                declared.hook,
                declared.phase,
                declared.tools.as_ref(),
                &declared.ops,
            );
            if let Err(problem) = shape {
                problems.push((index, problem));
            }
            if declared.hook == Hook::PostAssistant
                && declared.on_unavailable == Some(OnUnavailable::Refuse)
            {
                problems.push((index, SubscriptionProblem::RefuseOnPostAssistant));
            }
            if declared.budget_ms == 0 {
                problems.push((index, SubscriptionProblem::ZeroBudget));
            }
        }
        problems
    }

    /// Find the declared subscription that bounds `planned`, and return its
    /// index. A planned subscription fits a declared one with the same hook
    /// and phase whose tools cover the planned tools (absent covers every
    /// tool, and only absent covers an absent list) and whose ops include
    /// every planned op.
    pub fn bound(&self, planned: &Subscription) -> Result<usize, SubscriptionProblem> {
        check_shape(
            planned.hook,
            planned.phase,
            planned.tools.as_ref(),
            &planned.ops,
        )?;
        let candidates: Vec<(usize, &DeclaredSubscription)> = self
            .subscriptions
            .iter()
            .enumerate()
            .filter(|(_, d)| d.hook == planned.hook && d.phase == planned.phase)
            .collect();
        if candidates.is_empty() {
            return Err(SubscriptionProblem::NotDeclared);
        }
        let covering: Vec<(usize, &DeclaredSubscription)> = candidates
            .into_iter()
            .filter(|(_, d)| tools_cover(d.tools.as_ref(), planned.tools.as_ref()))
            .collect();
        if covering.is_empty() {
            return Err(SubscriptionProblem::ToolsNotCovered);
        }
        covering
            .into_iter()
            .find(|(_, d)| planned.ops.iter().all(|op| d.ops.contains(op)))
            .map(|(index, _)| index)
            .ok_or(SubscriptionProblem::OpNotDeclared)
    }
}

fn tools_cover(declared: Option<&Vec<String>>, planned: Option<&Vec<String>>) -> bool {
    match (declared, planned) {
        (None, _) => true,
        (Some(_), None) => false,
        (Some(declared), Some(planned)) => planned.iter().all(|tool| declared.contains(tool)),
    }
}

/// Check the reduction rule for one planned subscription of `provider`:
/// `replace` on `pre_user` or `post_assistant` belongs to the session's
/// reduction owner alone, and without one it is refused. `replace` on
/// `post_tool` is a separate user-tier grant, which this role does not
/// carry and the runner checks on its own.
pub fn check_reduction(
    planned: &Subscription,
    provider: &str,
    reduction_owner: Option<&str>,
) -> Result<(), SubscriptionProblem> {
    let reducing_hook = matches!(planned.hook, Hook::PreUser | Hook::PostAssistant);
    if reducing_hook && planned.ops.contains(&Op::Replace) && reduction_owner != Some(provider) {
        return Err(SubscriptionProblem::ReplaceNotReductionOwner);
    }
    Ok(())
}

/// The order in which the runner calls the plan's step-transform items on a
/// hook: the reduction owner first, if it is among them, then every other
/// item in plan order. Returns indices into `providers`.
pub fn hook_order(providers: &[&str], reduction_owner: Option<&str>) -> Vec<usize> {
    let owner = reduction_owner.and_then(|owner| providers.iter().position(|p| *p == owner));
    owner
        .into_iter()
        .chain((0..providers.len()).filter(|index| Some(*index) != owner))
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::vectors;

    fn names(problems: &[(usize, SubscriptionProblem)]) -> Vec<&'static str> {
        problems.iter().map(|(_, p)| p.name()).collect()
    }

    #[test]
    fn declare_vectors_check_as_recorded() {
        let file = vectors::load("declare.json");
        for case in vectors::cases(&file, "requests") {
            vectors::round_trip::<DeclareRequest>(case["name"].as_str().unwrap(), &case["request"]);
        }
        for case in vectors::cases(&file, "declarations") {
            let name = case["name"].as_str().unwrap();
            let declaration = vectors::round_trip::<Declaration>(name, &case["declaration"]);
            assert!(
                declaration.check().is_empty(),
                "{name}: {:?}",
                declaration.check()
            );
            let effective: Vec<OnUnavailable> = declaration
                .subscriptions
                .iter()
                .map(DeclaredSubscription::effective_on_unavailable)
                .collect();
            let expected: Vec<OnUnavailable> =
                serde_json::from_value(case["effective_on_unavailable"].clone()).unwrap();
            assert_eq!(effective, expected, "{name}");
        }
        for case in vectors::cases(&file, "malformed") {
            let name = case["name"].as_str().unwrap();
            let declaration: Declaration = serde_json::from_value(case["declaration"].clone())
                .unwrap_or_else(|e| panic!("{name}: {e}"));
            assert!(
                names(&declaration.check()).contains(&case["problem"].as_str().unwrap()),
                "{name}: {:?}",
                declaration.check()
            );
        }
        for case in vectors::cases(&file, "undecodable") {
            vectors::refused::<Declaration>(case["name"].as_str().unwrap(), &case["declaration"]);
        }
    }

    #[test]
    fn subscription_vectors_bound_as_recorded() {
        let file = vectors::load("subscriptions.json");
        let declaration: Declaration = serde_json::from_value(file["declaration"].clone()).unwrap();
        assert!(declaration.check().is_empty());
        for case in vectors::cases(&file, "planned") {
            let name = case["name"].as_str().unwrap();
            let planned = vectors::round_trip::<Subscription>(name, &case["subscription"]);
            let outcome = declaration.bound(&planned);
            match case.get("bound_by") {
                Some(index) => assert_eq!(outcome, Ok(index.as_u64().unwrap() as usize), "{name}"),
                None => assert_eq!(
                    outcome.map_err(SubscriptionProblem::name),
                    Err(case["problem"].as_str().unwrap()),
                    "{name}"
                ),
            }
        }
        for case in vectors::cases(&file, "undecodable") {
            vectors::refused::<Subscription>(case["name"].as_str().unwrap(), &case["subscription"]);
        }
        for case in vectors::cases(&file, "reduction") {
            let name = case["name"].as_str().unwrap();
            let planned: Subscription =
                serde_json::from_value(case["subscription"].clone()).unwrap();
            let outcome = check_reduction(
                &planned,
                case["provider"].as_str().unwrap(),
                case["reduction_owner"].as_str(),
            );
            match case["problem"].as_str() {
                None => assert_eq!(outcome, Ok(()), "{name}"),
                Some(problem) => assert_eq!(
                    outcome.map_err(SubscriptionProblem::name),
                    Err(problem),
                    "{name}"
                ),
            }
        }
        for case in vectors::cases(&file, "order") {
            let name = case["name"].as_str().unwrap();
            let providers: Vec<&str> = case["providers"]
                .as_array()
                .unwrap()
                .iter()
                .map(|p| p.as_str().unwrap())
                .collect();
            let expected: Vec<usize> = serde_json::from_value(case["order"].clone()).unwrap();
            assert_eq!(
                hook_order(&providers, case["reduction_owner"].as_str()),
                expected,
                "{name}"
            );
        }
    }
}
