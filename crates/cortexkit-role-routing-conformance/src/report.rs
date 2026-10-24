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
];

/// Where the kit checks less than the contract says, and why. Every report
/// prints these.
pub const NARROWINGS: &[&str] = &[
    "priced_only is checked only as role.describe states it; the kit does not check that a priced_only consumer is never answered with a subscription route.",
    "Tool provider reachability is configured for every project at once, so the kit does not check that reachability is read for the scoped project.",
    "Affinity is checked with one failure outcome (failed); the other failure outcomes, and the rule that a cancelled or completed report keeps the preference, are not checked.",
    "account_observe idempotency is checked for one runner; the kit does not check that another runner's equal event_id is a separate event.",
    "The kit does not check that the runner ops refuse a caller that is not a runner module, nor the account_for refusals other than admission_conflict and unknown_decision.",
    "Declaration precedence is checked with max_output_tokens only; the other demand fields are not independently checked for precedence.",
    "The kit does not check invalid catalog declarations or provider health reports for invalid, unverified or conflicting entries.",
    "requirements other than tool_capabilities and the declared max_output_tokens floor are not checked against the catalog beyond what every check's answer must meet.",
];

/// How one check ended.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum CheckOutcome {
    Passed,
    Failed { reason: String },
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
    /// Every check ran and passed.
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

    /// The report as text, one line per check, then the narrowings and the
    /// verdict.
    pub fn render(&self) -> String {
        let mut out = String::new();
        for report in &self.checks {
            let line = match &report.outcome {
                CheckOutcome::Passed => format!("PASS {}", report.check),
                CheckOutcome::Failed { reason } => format!("FAIL {}: {reason}", report.check),
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
