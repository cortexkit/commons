//! Subscriptions: what a provider declares it can do, and what a plan
//! subscribes it to.
//!
//! The provider's declaration (`transform.declare`) lists, per hook (and per
//! phase on `pre_tool`), the tools it can act on, the operations it may
//! return, what the runner does when it is unavailable, and its time budget.
//! The declaration is the source: whoever composes the plan copies each
//! subscription's `on_unavailable` and `budget_ms` from it into the plan,
//! where they are frozen. A plan item's subscriptions choose within the
//! declaration's bounds. Admission is the plan check before the runner accepts
//! a session (CONTRACT.md §4). The runner checks the frozen plan
//! against the provider's current declaration: an equal or stricter
//! subscription admits, and a looser one, or one whose hook or preset the
//! provider no longer declares, refuses the plan as stale.
//!
//! The session has at most one reduction owner: its compaction provider,
//! whose `replace` rewrites history and so must run before other providers'
//! prepends and appends, and which is the only party allowed to remove or
//! rewrite what was written (CONTRACT.md §5). Only the reduction owner may
//! `replace` on `pre_user` and `post_assistant`. Every other step transform is preserving: it may
//! prepend or append. Hooks run in the frozen plan's exact order; the plan
//! composer places the reduction owner first ([`reduction_owner_first`]),
//! so a later preserving prepend is never wiped by its replace.
use serde::{Deserialize, Serialize};
use serde_json::{Map, Value};

use crate::errors::runner_codes;

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

impl OnUnavailable {
    /// Whether a plan may freeze `self` where the declaration says
    /// `declared`: `refuse` is stricter than `pass`, so only `pass` under a
    /// declared `refuse` is looser.
    pub fn within(self, declared: OnUnavailable) -> bool {
        self == OnUnavailable::Refuse || declared == OnUnavailable::Pass
    }
}

/// One subscription in a plan item, as the fetch plan carries it: `{hook,
/// phase?, tools?, ops, on_unavailable, budget_ms}`. Decoded leniently on
/// fields, strictly on values.
///
/// `on_unavailable` and `budget_ms` are copied from the provider's
/// declaration when the plan is composed and frozen with the plan. A plan
/// may carry a stricter value than the declaration (`refuse` for `pass`, a
/// smaller budget), never a looser one.
#[derive(Clone, Debug, PartialEq, Eq, Deserialize, Serialize)]
#[non_exhaustive]
pub struct Subscription {
    pub hook: Hook,
    /// Present on `pre_tool` only.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub phase: Option<Phase>,
    /// The tools the subscription covers; absent means every tool. Only on
    /// `pre_tool` and `post_tool`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub tools: Option<Vec<String>>,
    /// The operations the provider may return. Empty on `pre_tool`, whose
    /// answers are by phase.
    pub ops: Vec<Op>,
    /// What the runner does when the hook is unavailable for a call. Always
    /// `pass` on `post_assistant`.
    pub on_unavailable: OnUnavailable,
    /// The provider's time budget per call, in milliseconds, which the
    /// runner caps with its own engine-wide limit.
    pub budget_ms: u64,
}

/// One subscription a provider declares: the bounds a plan may choose
/// within, plus what the runner does when the hook is unavailable and how
/// long it may take.
#[derive(Clone, Debug, PartialEq, Eq, Deserialize, Serialize)]
#[non_exhaustive]
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
    pub fn new(hook: Hook, ops: Vec<Op>, budget_ms: u64) -> Self {
        Self {
            hook,
            phase: None,
            tools: None,
            ops,
            on_unavailable: None,
            budget_ms,
        }
    }

    pub fn with_phase(mut self, phase: Phase) -> Self {
        self.phase = Some(phase);
        self
    }

    pub fn with_tools(mut self, tools: Vec<String>) -> Self {
        self.tools = Some(tools);
        self
    }

    pub fn with_on_unavailable(mut self, policy: OnUnavailable) -> Self {
        self.on_unavailable = Some(policy);
        self
    }

    /// The `on_unavailable` this subscription means in effect, and the value
    /// a plan composed from it freezes: `pass` on `post_assistant`,
    /// otherwise the declared value, `refuse` when none is declared.
    pub fn effective_on_unavailable(&self) -> OnUnavailable {
        if self.hook == Hook::PostAssistant {
            return OnUnavailable::Pass;
        }
        self.on_unavailable.unwrap_or(OnUnavailable::Refuse)
    }

    /// The fields on which `planned` is looser than this declared
    /// subscription, in the order `on_unavailable`, `budget_ms`. Empty when
    /// the planned values are equal or stricter.
    pub fn loosened_fields(&self, planned: &Subscription) -> Vec<LoosenedField> {
        let mut fields = Vec::new();
        if !planned
            .on_unavailable
            .within(self.effective_on_unavailable())
        {
            fields.push(LoosenedField::OnUnavailable);
        }
        if planned.budget_ms > self.budget_ms {
            fields.push(LoosenedField::BudgetMs);
        }
        fields
    }
}

/// A planned subscription field that may equal or tighten the declared
/// value but never loosen it. A `subscription_loosened` difference names
/// the field that broke this rule in its `field` member.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Deserialize, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum LoosenedField {
    /// Tools outside the declared bounds.
    Tools,
    /// Operations outside the declared bounds.
    Ops,
    /// `pass` planned where the declaration says `refuse`.
    OnUnavailable,
    /// A budget above the declared one.
    BudgetMs,
}

/// The `transform.declare` request: the plan item's preset and params and
/// the session's composition, verbatim. Decoded leniently.
#[derive(Clone, Debug, PartialEq, Deserialize, Serialize)]
#[non_exhaustive]
pub struct DeclareRequest {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub preset: Option<String>,
    #[serde(default)]
    pub params: Map<String, Value>,
    /// Absent on a preflight call, made before the composition exists.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub composition: Option<Map<String, Value>>,
}

impl DeclareRequest {
    pub fn new(params: Map<String, Value>) -> Self {
        Self {
            preset: None,
            params,
            composition: None,
        }
    }

    pub fn with_preset(mut self, preset: impl Into<String>) -> Self {
        self.preset = Some(preset.into());
        self
    }

    pub fn with_composition(mut self, composition: Map<String, Value>) -> Self {
        self.composition = Some(composition);
        self
    }
}

/// The `transform.declare` answer. A pure function of the request and the
/// provider's configuration: the same inputs give the same bytes, and no
/// scope, agent or session identity enters it.
#[derive(Clone, Debug, PartialEq, Eq, Deserialize, Serialize)]
pub struct Declaration {
    pub subscriptions: Vec<DeclaredSubscription>,
}

/// Why a subscription, declared or planned, is malformed or out of bounds.
///
/// Two kinds, refused with different codes at admission (the plan check
/// before accepting a session, CONTRACT.md §4)
/// ([`SubscriptionProblem::admission_code`]): a plan that is malformed in
/// itself, or breaks a rule no declaration can change, is `invalid_params`;
/// a plan that the provider's current declaration no longer covers is
/// `plan_stale`, since recomposing against the current declaration can fix it.
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
    /// The provider's declaration for the item's preset and params no
    /// longer has a subscription with the planned hook and phase.
    SubscriptionMissing,
    /// The provider no longer declares anything for the item's preset: it
    /// refused `transform.declare` with `invalid_params {field: "preset"}`.
    PresetMissing,
    /// The planned subscription is looser than the declared one on this
    /// field.
    SubscriptionLoosened(LoosenedField),
    /// `replace` on `pre_user` or `post_assistant` by a provider that is not
    /// the session's reduction owner, its compaction provider (CONTRACT.md §5).
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
            Self::SubscriptionMissing => "subscription_missing",
            Self::PresetMissing => "preset_missing",
            Self::SubscriptionLoosened(_) => "subscription_loosened",
            Self::ReplaceNotReductionOwner => "replace_not_reduction_owner",
        }
    }

    /// The loosened field, for [`SubscriptionProblem::SubscriptionLoosened`].
    pub fn field(self) -> Option<LoosenedField> {
        match self {
            Self::SubscriptionLoosened(field) => Some(field),
            _ => None,
        }
    }

    /// The code the runner refuses a plan with at admission, the plan check
    /// before accepting a session (CONTRACT.md §4), for this
    /// problem: `plan_stale` for a subscription the current declaration no
    /// longer covers (missing, its preset missing, or loosened),
    /// `invalid_params` for everything else.
    pub fn admission_code(self) -> &'static str {
        match self {
            Self::SubscriptionMissing | Self::PresetMissing | Self::SubscriptionLoosened(_) => {
                runner_codes::PLAN_STALE
            }
            _ => runner_codes::INVALID_PARAMS,
        }
    }

    /// Construct the `plan_stale` difference from this problem and the
    /// supplied provider, preset, hook and phase. Returns `None` for an
    /// `invalid_params` problem.
    pub fn stale_difference(
        self,
        provider: &str,
        preset: &str,
        hook: Hook,
        phase: Option<Phase>,
    ) -> Option<StaleDifference> {
        let provider = provider.to_owned();
        match self {
            Self::SubscriptionMissing => Some(StaleDifference::SubscriptionMissing {
                provider,
                hook,
                phase,
            }),
            Self::PresetMissing => Some(StaleDifference::PresetMissing {
                provider,
                preset: preset.to_owned(),
            }),
            Self::SubscriptionLoosened(field) => Some(StaleDifference::SubscriptionLoosened {
                provider,
                hook,
                phase,
                field,
            }),
            _ => None,
        }
    }
}

/// One entry of a `plan_stale` refusal's `detail.differences` that this
/// role defines, internally tagged as the fetch plan spells it:
/// `{kind: "subscription_loosened", provider, hook, phase, field}`.
/// The same array may carry the fetch plan's own kinds (a digest or tool difference), which
/// this type does not decode.
#[derive(Clone, Debug, PartialEq, Eq, Deserialize, Serialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum StaleDifference {
    /// The provider's current declaration for the plan item's preset and
    /// params has no subscription with the planned hook and phase.
    SubscriptionMissing {
        provider: String,
        hook: Hook,
        #[serde(deserialize_with = "Option::deserialize")]
        phase: Option<Phase>,
    },
    /// The provider no longer knows the plan item's preset, so it declares
    /// nothing for it.
    PresetMissing { provider: String, preset: String },
    /// A planned subscription's frozen `field` is looser than the current
    /// declaration's: wider tools or ops, `pass` for a declared `refuse`,
    /// or a larger budget.
    SubscriptionLoosened {
        provider: String,
        hook: Hook,
        #[serde(deserialize_with = "Option::deserialize")]
        phase: Option<Phase>,
        field: LoosenedField,
    },
}

/// The `detail` of an `invalid_params` refusal during the plan check before
/// accepting a session (admission, CONTRACT.md §4), for one planned
/// subscription: `{field: "plan.step_transform_items", item, subscription,
/// problem}`, where `item` and `subscription` are indices into the plan's
/// `step_transform_items` and that item's `subscriptions`.
#[derive(Clone, Debug, PartialEq, Eq, Deserialize, Serialize)]
pub struct InvalidSubscriptionDetail {
    pub field: String,
    pub item: usize,
    pub subscription: usize,
    pub problem: String,
}

impl InvalidSubscriptionDetail {
    pub fn new(item: usize, subscription: usize, problem: SubscriptionProblem) -> Self {
        Self {
            field: runner_codes::PLAN_STEP_TRANSFORM_ITEMS.to_owned(),
            item,
            subscription,
            problem: problem.name().to_owned(),
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

/// The policy rules a declared and a planned subscription share.
fn check_policy(
    hook: Hook,
    on_unavailable: Option<OnUnavailable>,
    budget_ms: u64,
) -> Vec<SubscriptionProblem> {
    let mut problems = Vec::new();
    if hook == Hook::PostAssistant && on_unavailable == Some(OnUnavailable::Refuse) {
        problems.push(SubscriptionProblem::RefuseOnPostAssistant);
    }
    if budget_ms == 0 {
        problems.push(SubscriptionProblem::ZeroBudget);
    }
    problems
}

impl Subscription {
    pub fn new(hook: Hook, ops: Vec<Op>, on_unavailable: OnUnavailable, budget_ms: u64) -> Self {
        Self {
            hook,
            phase: None,
            tools: None,
            ops,
            on_unavailable,
            budget_ms,
        }
    }

    pub fn with_phase(mut self, phase: Phase) -> Self {
        self.phase = Some(phase);
        self
    }

    pub fn with_tools(mut self, tools: Vec<String>) -> Self {
        self.tools = Some(tools);
        self
    }

    /// Check the planned subscription on its own: its shape and its frozen
    /// policy. Every problem found here is `invalid_params`.
    pub fn check(&self) -> Result<(), SubscriptionProblem> {
        check_shape(self.hook, self.phase, self.tools.as_ref(), &self.ops)?;
        match check_policy(self.hook, Some(self.on_unavailable), self.budget_ms).first() {
            Some(problem) => Err(*problem),
            None => Ok(()),
        }
    }
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
            for problem in check_policy(declared.hook, declared.on_unavailable, declared.budget_ms)
            {
                problems.push((index, problem));
            }
        }
        problems
    }

    /// Find the declared subscription that bounds `planned`, and return its
    /// index. Checks run in a fixed order and report the first failure.
    /// First check the planned subscription itself ([`Subscription::check`]).
    /// If no declaration has the same hook and phase, report
    /// `subscription_missing`. Then filter candidates by tool coverage
    /// (absent covers every tool; only absent covers an absent planned list)
    /// and by ops coverage (every planned op must be declared). Finally
    /// require the planned policy and budget to equal or tighten a candidate's
    /// bounds. Failed bounds report `subscription_loosened`, naming `tools`
    /// or `ops` when those filters fail, otherwise the first loosened policy
    /// or budget field of the first remaining candidate.
    pub fn bound(&self, planned: &Subscription) -> Result<usize, SubscriptionProblem> {
        planned.check()?;
        let candidates: Vec<(usize, &DeclaredSubscription)> = self
            .subscriptions
            .iter()
            .enumerate()
            .filter(|(_, d)| d.hook == planned.hook && d.phase == planned.phase)
            .collect();
        if candidates.is_empty() {
            return Err(SubscriptionProblem::SubscriptionMissing);
        }
        let covering: Vec<(usize, &DeclaredSubscription)> = candidates
            .into_iter()
            .filter(|(_, d)| tools_cover(d.tools.as_ref(), planned.tools.as_ref()))
            .collect();
        if covering.is_empty() {
            return Err(SubscriptionProblem::SubscriptionLoosened(
                LoosenedField::Tools,
            ));
        }
        let with_ops: Vec<(usize, &DeclaredSubscription)> = covering
            .into_iter()
            .filter(|(_, d)| planned.ops.iter().all(|op| d.ops.contains(op)))
            .collect();
        let Some((_, first)) = with_ops.first() else {
            return Err(SubscriptionProblem::SubscriptionLoosened(
                LoosenedField::Ops,
            ));
        };
        if let Some((index, _)) = with_ops
            .iter()
            .find(|(_, d)| d.loosened_fields(planned).is_empty())
        {
            return Ok(*index);
        }
        let field = first.loosened_fields(planned)[0];
        Err(SubscriptionProblem::SubscriptionLoosened(field))
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
/// reduction owner (its compaction provider, CONTRACT.md §5) alone, and
/// without one it is refused. `replace` on `post_tool` is a separate
/// user-tier grant carried in the plan's `user_grants`, checked per answer
/// (`answer::check_post_tool_grant`), not here.
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
/// hook: the frozen plan's exact order. The runner never moves an item,
/// including the reduction owner. Returns indices into `providers`.
pub fn hook_order(providers: &[&str]) -> Vec<usize> {
    (0..providers.len()).collect()
}

/// The plan composer's obligation on order: when the session's reduction
/// owner (its compaction provider, CONTRACT.md §5) is among the
/// step-transform items, it is the first of them, so its `replace` runs
/// before every preserving prepend or append. The plan composer checks this
/// when it writes the plan; providers do not enforce it, and the runner
/// still calls items in plan order ([`hook_order`]). `true` when there is
/// no reduction owner or it has no step-transform item.
pub fn reduction_owner_first(providers: &[&str], reduction_owner: Option<&str>) -> bool {
    match reduction_owner.and_then(|owner| providers.iter().position(|p| *p == owner)) {
        None | Some(0) => true,
        Some(_) => false,
    }
}

/// Why the runner refuses a step-transform item during admission, the plan
/// check before accepting a session (CONTRACT.md §4).
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ItemRefusal {
    /// `invalid_params`, with the detail of the first malformed or
    /// forbidden subscription.
    InvalidParams(InvalidSubscriptionDetail),
    /// `plan_stale`, with one difference per subscription the current
    /// declaration no longer covers.
    PlanStale(Vec<StaleDifference>),
}

/// Admit plan item `item`, naming `provider` and `preset`, with frozen
/// subscriptions `planned`, against the provider's current
/// `declaration` for the item's preset and params. `declaration` is `None`
/// when the provider refused `transform.declare` with `invalid_params
/// {field: "preset"}`: it no longer knows the preset.
///
/// Returns the index of the declared subscription that bounds each planned
/// one. An `invalid_params` problem (a malformed subscription or `replace`
/// on `pre_user` or `post_assistant` by a provider other than the reduction
/// owner) wins over any staleness. The first such problem in subscription
/// order is reported.
/// Otherwise every stale subscription is listed, in order. A missing preset
/// produces one difference per item, even if the item has no subscriptions.
pub fn check_item(
    item: usize,
    provider: &str,
    preset: &str,
    declaration: Option<&Declaration>,
    planned: &[Subscription],
    reduction_owner: Option<&str>,
) -> Result<Vec<usize>, ItemRefusal> {
    let mut bounds = Vec::new();
    let mut differences = Vec::new();
    for (index, subscription) in planned.iter().enumerate() {
        // The rules no declaration can change come first, so a subscription
        // that is both malformed and stale is reported as malformed.
        let outcome = subscription
            .check()
            .and_then(|()| check_reduction(subscription, provider, reduction_owner))
            .and_then(|()| match declaration {
                Some(declaration) => declaration.bound(subscription),
                None => Err(SubscriptionProblem::PresetMissing),
            });
        match outcome {
            Ok(bound) => bounds.push(bound),
            Err(problem) => match problem.stale_difference(
                provider,
                preset,
                subscription.hook,
                subscription.phase,
            ) {
                Some(difference) => {
                    if declaration.is_some() {
                        differences.push(difference);
                    }
                }
                None => {
                    return Err(ItemRefusal::InvalidParams(InvalidSubscriptionDetail::new(
                        item, index, problem,
                    )))
                }
            },
        }
    }
    if declaration.is_none() {
        differences.push(StaleDifference::PresetMissing {
            provider: provider.to_owned(),
            preset: preset.to_owned(),
        });
    }
    if differences.is_empty() {
        Ok(bounds)
    } else {
        Err(ItemRefusal::PlanStale(differences))
    }
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
                None => {
                    let problem = outcome.unwrap_err();
                    assert_eq!(problem.name(), case["problem"].as_str().unwrap(), "{name}");
                    let field = problem.field().map(|f| serde_json::to_value(f).unwrap());
                    assert_eq!(field.as_ref(), case.get("field"), "{name}");
                }
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
            assert_eq!(hook_order(&providers), expected, "{name}");
            assert_eq!(
                reduction_owner_first(&providers, case["reduction_owner"].as_str()),
                case["reduction_owner_first"].as_bool().unwrap(),
                "{name}"
            );
        }
    }

    /// Admit one item the way the vectors spell the outcome: `{bounds}` when
    /// admitted, or the refusal's `{code, detail}`.
    fn admission(
        item: usize,
        provider: &str,
        preset: &str,
        declaration: Option<&Declaration>,
        planned: &[Subscription],
        reduction_owner: Option<&str>,
    ) -> Value {
        match check_item(
            item,
            provider,
            preset,
            declaration,
            planned,
            reduction_owner,
        ) {
            Ok(bounds) => serde_json::json!({"bounds": bounds}),
            Err(ItemRefusal::InvalidParams(detail)) => serde_json::json!({
                "code": runner_codes::INVALID_PARAMS,
                "detail": detail,
            }),
            Err(ItemRefusal::PlanStale(differences)) => serde_json::json!({
                "code": runner_codes::PLAN_STALE,
                "detail": {"differences": differences},
            }),
        }
    }

    #[test]
    fn item_vectors_admit_as_recorded() {
        let file = vectors::load("subscriptions.json");
        let declared: Declaration = serde_json::from_value(file["declaration"].clone()).unwrap();
        for case in vectors::cases(&file, "items") {
            let name = case["name"].as_str().unwrap();
            let planned: Vec<Subscription> =
                serde_json::from_value(case["subscriptions"].clone()).unwrap();
            let declaration = match &case["declaration"] {
                Value::Null => None,
                other => {
                    assert_eq!(other, "file", "{name}");
                    Some(&declared)
                }
            };
            let outcome = admission(
                case["item"].as_u64().unwrap_or(0) as usize,
                case["provider"].as_str().unwrap(),
                case["preset"].as_str().unwrap(),
                declaration,
                &planned,
                case["reduction_owner"].as_str(),
            );
            assert_eq!(outcome, case["expected"], "{name}");
            if let Some(differences) = outcome["detail"]["differences"].as_array() {
                for difference in differences {
                    vectors::round_trip::<StaleDifference>(name, difference);
                    if difference["kind"] != "preset_missing" {
                        let mut missing_phase = difference.clone();
                        missing_phase.as_object_mut().unwrap().remove("phase");
                        vectors::refused::<StaleDifference>(name, &missing_phase);
                    }
                }
            }
        }
    }

    /// These admission cases come from the plan composer's own test cases
    /// (`fetch-plan-v1`; each case's `source` names the upstream file and
    /// commit), so both sides must reach the same admission result: the same
    /// plan and declaration give the same refusal bytes, or admit and freeze
    /// the plan's subscriptions unchanged.
    #[test]
    fn fetch_plan_admission_vectors_agree() {
        let file = vectors::load("subscriptions.json");
        let cases = vectors::cases(&file, "fetch_plan_v1");
        assert_eq!(cases.len(), 11);
        for case in cases {
            assert!(case["source"].as_str().unwrap().starts_with(
                "prefrontal 3e69ed9d4a048a458db54203bd93a1069598222e test-vectors/fetch-plan-v1/"
            ));
            let name = case["name"].as_str().unwrap();
            let planned: Vec<Subscription> = case["subscriptions"]
                .as_array()
                .unwrap()
                .iter()
                .map(|s| vectors::round_trip::<Subscription>(name, s))
                .collect();
            let declaration: Option<Declaration> = match &case["declaration"] {
                Value::Null => None,
                other => Some(vectors::round_trip(name, other)),
            };
            let outcome = admission(
                0,
                case["provider"].as_str().unwrap(),
                case["preset"].as_str().unwrap(),
                declaration.as_ref(),
                &planned,
                None,
            );
            let expected = &case["expected"];
            if expected["admitted"] == true {
                assert!(outcome.get("bounds").is_some(), "{name}: {outcome}");
                let frozen = &expected["frozen_step_transform_items"][0]["subscriptions"];
                assert_eq!(frozen, &case["subscriptions"], "{name}");
            } else {
                assert_eq!(expected["admitted"], false, "{name}");
                assert_eq!(outcome["code"], expected["code"], "{name}");
                assert_eq!(outcome["detail"], expected["detail"], "{name}");
            }
        }
    }
}
