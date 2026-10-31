//! The kit's checks and what a run reports.

/// One check: its stable name, the contract rule it holds a provider to,
/// and what it asserts.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct CheckSpec {
    pub name: &'static str,
    pub checks: &'static str,
}

const fn check(name: &'static str, checks: &'static str) -> CheckSpec {
    CheckSpec { name, checks }
}

/// Every check, in the order the kit runs them.
pub const CHECKS: &[CheckSpec] = &[
    check("service_tier_eligible_account", "service_tier: only an account whose plan includes the chosen tier is admitted for the decision"),
    check("service_tier_fallback_order", "service_tier: ultrafast falls back to fast then standard, recording service_tier_chosen"),
    check("service_tier_absent_is_standard", "service_tier: an absent tier means standard"),
    check("service_tier_report_members_accepted", "service_tier: a declaring router accepts both tier members in report usage"),
    check("parent_account_preference", "an eligible account other than the parent's is preferred"),
    check("parent_account_fallback", "the parent's account is used when it is the only eligible one"),
    check("eligibility_never_holds_or_exhausts", "a model eligibility report never holds or exhausts the account for another model"),
    check("unknown_pool_accepted", "unseen well-formed pools are accepted in account_for, account_observe and report"),
    check("malformed_pool_refused_by_field", "malformed pools are invalid_request with detail.field pool on all three ops"),
    check("pool_body_admission_conflict", "changing pool under a reused admission_id is admission_conflict"),
    check("parent_body_admission_conflict", "changing parent_conversation_key under a reused admission_id is admission_conflict"),
    check("new_report_members_conflict", "changing pool, eligibility, request_input_tokens or usage under a reused decision_id is report_conflict"),
    check(
        "describe_lists_consumers_and_fields",
        "role.describe names routing version 1 and every required op and requirement field, and lists exactly the configured consumers, each with its priced_only",
    ),
    check(
        "unknown_requirement_refused_by_name",
        "a select whose requirements carry a member the provider does not read (gpu_count) is refused invalid_requirements with detail.field gpu_count, never answered",
    ),
    check(
        "unknown_consumer_refused",
        "a select for a consumer id the provider is not configured with is refused unknown_consumer naming it",
    ),
    check(
        "exclude_honoured",
        "a select that excludes the route key the provider otherwise answers is answered with another route",
    ),
    check(
        "declared_consumer_owner_uses_default",
        "a declaring module is admitted for its catalog-declared consumer; an omitted output floor uses the declared default, and an explicit request floor overrides it",
    ),
    check(
        "declared_consumer_scope_owner_admitted",
        "a third-party caller carrying a daemon-stamped scope owned by the declaring module is admitted for that module's declared consumer",
    ),
    check(
        "declared_consumer_non_owner_refused",
        "a third-party caller without the declaring module's scope, unscoped or carrying its own scope, is refused terminal consumer_not_owned naming the consumer",
    ),
    check(
        "operator_config_overrides_declared_default",
        "operator demand overrides both request demand and the declared default, even when those would rule out the explicitly pinned model",
    ),
    check(
        "declared_consumer_unverified_entry_ignored",
        "an unverified catalog declaration grants no name or defaults; configuration-only names remain usable by any caller, and a verified owner wins regardless of catalog order",
    ),
    check(
        "declared_consumer_conflict_refused",
        "two verified declarers of the same name are both refused terminal consumer_not_owned naming the consumer, and role.describe omits the name even when it is configured",
    ),
    check(
        "runners_honoured",
        "a select whose runners list only the runner the provider does not otherwise answer is answered on that runner",
    ),
    check(
        "timed_refusals_carry_reset_at_ms",
        "a pin on a cooled-down route is pin_unavailable reason cooldown with a future reset_at_ms; a pin on an unknown model is pin_unavailable reason unknown_model with no reset_at_ms; when every route is cooling down a select is no_route reason quota_or_cooldown with a future reset_at_ms",
    ),
    check(
        "scope_comes_from_route_stamp",
        "a per-project pin applies to a select on a route bound to that project, and not to one on an unscoped route or a route bound to another project, even when the body names the pinned project",
    ),
    check(
        "report_from_other_principal_unknown_decision",
        "a report on a decision from a principal other than the one that selected it is unknown_decision, the same refusal as for a decision that never existed, and the selecting principal can still report it",
    ),
    check(
        "repeated_report_returns_first_answer",
        "the same report body sent again returns exactly the first answer",
    ),
    check(
        "different_report_is_report_conflict",
        "a report with a different outcome after the first is report_conflict",
    ),
    check(
        "affinity_holds_until_failure_report",
        "a select with an affinity key keeps the previous decision's route for that key, and after a failure report on that decision the next select with the key is answered with another route",
    ),
    check(
        "answer_carries_limits_and_future_expiry",
        "an answer carries a decision_id, the catalog's context_window and max_output_tokens for the answered model, a runner that runs it, an expires_at_ms in the future, and a non-empty explanation of at most 200 characters",
    ),
    check(
        "account_for_idempotent_on_admission_id",
        "route.account_for sent again with the same admission_id and body returns exactly the first answer",
    ),
    check(
        "account_for_different_body_admission_conflict",
        "route.account_for with a used admission_id and a different body is admission_conflict",
    ),
    check(
        "account_for_foreign_decision_unknown",
        "route.account_for with a decision_id from another runner, or for another model than the decision's, is unknown_decision, the same refusal as for a decision that never existed; from the decision's runner and for its model it is honoured",
    ),
    check(
        "account_observe_idempotent_per_event_id",
        "route.account_observe sent twice with one event_id answers duplicate false then duplicate true, with the same observation id",
    ),
    check(
        "tool_capabilities_matched_by_tag",
        "a select requiring a tool capability tag is answered on the runner that reaches a tool provider serving it, and a select listing a tool provider's module name as a tag is no_route",
    ),
    check(
        "harness_only_when_declared",
        "runner.harness is never invented for a runner that declared none, and is not dropped for one declaring a default",
    ),
    check("requested_harness_is_hosted_and_echoed", "select_harness: only a runner listing the requested program answers, echoing it; an unhosted program is no_route harness_unavailable"),
    check("unrequested_harness_uses_declaration", "without harness, answer the declared default, else singleton, else omit it without filtering candidates"),
    check("non_starting_runner_never_selected", "starts_sessions false runners never answer select, with or without harness"),
    check("undeclared_harness_group_refuses_member", "a provider without select_harness refuses harness as invalid_request naming the field"),
    check(
        "hold_answers_blocked_entry_within_limits",
        "select_hold_control: a ladder entry whose accounts are all blocked, with a reset within both hold_when_reset_within and max_hold, is answered, and the runner then holds at account_for",
    ),
    check(
        "hold_moves_when_reset_beyond_window",
        "select_hold_control: a blocked entry whose reset is further away than hold_when_reset_within, though within max_hold, is moved past to the next entry",
    ),
    check(
        "hold_moves_when_reset_unknown",
        "select_hold_control: a blocked entry whose reset is unknown is moved past to the next entry",
    ),
    check(
        "hold_moves_beyond_caller_cap",
        "select_hold_control: a blocked entry whose reset is within max_hold but beyond the request's smaller hold_wait_max_s is moved past to the next entry",
    ),
    check(
        "hold_ok_false_moves_to_usable_entry",
        "select_hold_control: with hold_ok false, a blocked entry that would be held is moved past to a later entry usable now",
    ),
    check(
        "hold_ok_false_falls_back_to_soonest_entry",
        "select_hold_control: with hold_ok false and no entry usable now, the blocked entry whose reset is soonest within the limit is answered, not the first in ladder order",
    ),
    check(
        "hold_nothing_within_limit_accounts_blocked",
        "select_hold_control: when every entry is blocked and none resets within the limit, select is no_route accounts_blocked with reset_at_ms the earliest known reset, and without reset_at_ms when no reset is known",
    ),
    check(
        "hold_demand_mode_skips_blocked_model",
        "select_hold_control: a demand-mode consumer's model whose accounts are all blocked is skipped for the next adequate one, even with its reset within both limits",
    ),
    check(
        "hold_account_for_applies_recorded_cap",
        "select_hold_control: account_for naming a decision holds only within the cap that decision's select recorded; the same wait without the decision is account_hold",
    ),
    check(
        "pin_fallback_routes_and_labels",
        "an unservable pin with pin_fallback route is answered with pin_outcome fell_back and detail pin_reason, and a servable pin with fallback is answered pinned; without fallback it is pin_unavailable",
    ),
    check(
        "report_reset_fences_only_decision_route_and_account",
        "a failed report with a future reset fences only that decision's route on that decision's account until the reset, never other routes, accounts or providers",
    ),
    check(
        "report_past_reset_fences_nothing_and_is_recorded",
        "a failed report with a past reset is recorded as a failure with no fence, never refused for being in the past",
    ),
    check(
        "report_reset_capped_at_24h",
        "a caller-supplied reset further than 24 hours away is capped at 24 hours from now",
    ),
    check(
        "prefer_prefers_listed_adequate_route",
        "select_prefer: among the routes the provider would already answer, a listed adequate one is answered in list order",
    ),
    check(
        "prefer_never_admits_inadequate_route",
        "select_prefer: a listed route the request would not otherwise get is never admitted",
    ),
    check("undeclared_prefer_group_refuses_member", "a provider without select_prefer refuses prefer as invalid_request naming the field"),
    check(
        "report_usage_strict",
        "a report usage with a negative or over-limit count or an unknown member is invalid_request naming usage",
    ),
    check(
        "exclude_providers_answers_another_provider",
        "select_exclude_providers: a select excluding the provider of the route it is otherwise answered is answered with a route from another provider",
    ),
    check(
        "exclude_providers_every_provider_no_route",
        "select_exclude_providers: a select excluding every provider is refused no_route, with the reason and reset_at_ms of a select excluding every route key",
    ),
    check("undeclared_exclude_providers_group_refuses_member", "a provider without select_exclude_providers refuses exclude_providers as invalid_request naming the field"),
    check(
        "task_ref_never_changes_selection",
        "select_task_ref: the same select with and without task_ref answers the same route",
    ),
    check(
        "invalid_task_ref_refused_by_name",
        "select_task_ref: an invalid task_ref is refused as invalid_request naming task_ref",
    ),
];

/// Where the kit checks less than the contract says, and why. Every report
/// prints these.
pub const NARROWINGS: &[&str] = &[
    "priced_only is checked only as role.describe states it; the kit does not check that a priced_only consumer is never answered with a subscription route.",
    "Tool provider reachability is configured for every project at once, so the kit does not check that reachability is read for the scoped project.",
    "Affinity is checked with one failure outcome (failed); the other failure outcomes, and the rule that a cancelled or completed report keeps the preference, are not checked.",
    "account_observe idempotency is checked for one runner; the kit does not check that another runner's equal event_id is a separate event.",
    "The kit does not check that the runner ops refuse a caller that is not a runner module, nor the account_for refusals other than admission_conflict, unknown_decision and invalid_request for malformed pools.",
    "Eligibility is checked with model scope and another model on the same account. Account scope cannot distinguish a legitimate account-wide eligibility fence from an incorrect exhaustion/hold with the current hooks, so that distinction is not checked.",
    "Parent preference and fallback are checked on fresh admit requests; priority against an existing child pin, requested_account, renew or reassign is not checked.",
    "Pool isolation, disabled-window omission by runners, and request-size fencing policy are not exercised by the current catalog and provider hooks; vectors check the wire members only.",
    "Declaration precedence is checked with max_output_tokens only; the other demand fields are not independently checked for precedence.",
    "The kit does not check invalid catalog declarations or provider health reports for invalid, unverified or conflicting entries.",
    "requirements other than tool_capabilities and the declared max_output_tokens floor are not checked against the catalog beyond what every check's answer must meet.",
    "Holds are checked with whole-account blocks set through a hook, on ladders of two entries; the operator max_hold limit alone, variants as separate entries, the metered-fallback rule and pooled windows are not checked, and the hold_ok true path past every entry is checked only through hold_ok false.",
    "When account_for holds only within a decision's recorded cap, the kit checks that a wait beyond it is refused with some code other than account_hold; it does not check which.",
    "exclude_providers is checked on demand-mode selects with no pin and nothing cooling down: a pin on an excluded provider, its pin fallback, its interaction with prefer and ladders, and the no_route comparison with a timed cause are not checked.",
    "task_ref selection neutrality is checked on a fresh demand-mode select; stored attribution, per-task cost profiles, retention and non-disclosure to other callers are not inspected by the kit.",
];

/// How one check ended.
#[derive(Clone, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub enum CheckOutcome {
    Passed,
    Failed {
        reason: String,
    },
    /// The check covers an optional group the provider does not declare.
    /// It leaves the verdict a pass: the provider is not asked to serve
    /// what it does not declare.
    NotApplicable {
        reason: String,
    },
}

/// One check's outcome.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct CheckReport {
    pub check: &'static str,
    pub outcome: CheckOutcome,
}

/// The run's verdict.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum SuiteVerdict {
    /// No check failed. Checks of optional groups the provider does not
    /// declare are not applicable and do not change this.
    Passed,
    Failed {
        reasons: Vec<String>,
    },
}

/// What a run found.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SuiteReport {
    pub checks: Vec<CheckReport>,
    pub verdict: SuiteVerdict,
}

impl SuiteReport {
    /// The outcome of the check named `name`.
    pub fn outcome(&self, name: &str) -> Option<&CheckOutcome> {
        self.checks
            .iter()
            .find(|report| report.check == name)
            .map(|report| &report.outcome)
    }

    /// The names of the checks that failed, in run order.
    pub fn failing(&self) -> Vec<&'static str> {
        self.checks
            .iter()
            .filter(|report| matches!(report.outcome, CheckOutcome::Failed { .. }))
            .map(|report| report.check)
            .collect()
    }

    /// The names of the checks that were not applicable, in run order.
    pub fn not_applicable(&self) -> Vec<&'static str> {
        self.checks
            .iter()
            .filter(|report| matches!(report.outcome, CheckOutcome::NotApplicable { .. }))
            .map(|report| report.check)
            .collect()
    }

    /// The report as text, one line per check, then the narrowings and the
    /// verdict.
    pub fn render(&self) -> String {
        let mut out = String::new();
        for report in &self.checks {
            let line = match &report.outcome {
                CheckOutcome::Passed => format!("PASS {}", report.check),
                CheckOutcome::Failed { reason } => format!("FAIL {}: {reason}", report.check),
                CheckOutcome::NotApplicable { reason } => {
                    format!("N/A  {}: {reason}", report.check)
                }
            };
            out.push_str(&line);
            out.push('\n');
        }
        for narrowing in NARROWINGS {
            out.push_str(&format!("NARROWED {narrowing}\n"));
        }
        let verdict = match &self.verdict {
            SuiteVerdict::Passed => "passed".to_owned(),
            SuiteVerdict::Failed { reasons } => format!("failed: {}", reasons.join("; ")),
        };
        out.push_str(&format!("VERDICT {verdict}\n"));
        out
    }
}
