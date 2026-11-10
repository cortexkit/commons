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
    check("select_engine_eval_measured", "engine_evals: measured engines outrank unmeasured engines and cost"),
    check("select_engine_eval_higher_score", "engine_evals: newest score on the same set ranks higher first"),
    check("select_engine_eval_different_set", "engine_evals: scores from another set are not comparable"),
    check("select_engine_eval_new_model", "engine_evals: a new model id starts unmeasured"),
    check("select_engine_eval_default_metric", "engine_evals: transcription defaults to wer regardless of arrival order"),
    check("select_engine_eval_requested_metric", "engine_evals: the requested metric overrides the kind default"),
    check("select_engine_eval_newest_set", "engine_evals: the newest reference set decides ranking"),
    check("select_engine_eval_authorization", "engine_evals: refuse non-declaring callers"),
    check("select_engine_eval_score_range", "engine_evals: refuse out-of-range scores naming score"),
    check("select_engine_speed_authorization", "engine_reports: refuse non-declaring callers"),
    check("select_engine_opt_in", "select_engine: declare the operation with its group"),
    check("select_engine_slot_malformed", "select_engine: reject the whole malformed slot"),
    check("select_engine_slot_duplicate", "select_engine: reject a duplicate kind/model atomically"),
    check("select_engine_slot_empty_formats", "select_engine: reject empty formats atomically"),
    check("select_engine_slot_unverified", "select_engine: ignore nonce-unverified slots"),
    check("select_engine_request", "select_engine: validate ownership and params without priced_only restricting local engines"),
    check("select_engine_requirements_valid", "select_engine: refuse unknown or malformed requirements by path"),
    check("select_engine_caller_local", "select_engine: refuse non-local caller engines by indexed path"),
    check("select_engine_filters", "select_engine: name the first empty filter without a reset time"),
    check("select_engine_match_requirements", "select_engine: match every requirement against otherwise equivalent engines and missing facts"),
    check("select_engine_privacy", "select_engine: keep local engines and admit remote engines only when allowed"),
    check("select_engine_reachability", "select_engine: fence remote but not local or caller-local engines"),
    check("select_engine_answer", "select_engine: check module/index matrix and ten-minute expiry"),
    check("select_engine_ranking", "select_engine: rank unmeasured engines by known cost and deterministic ties"),
    check("select_engine_report", "select_engine: join by decision_id and outcome and refuse foreign reports"),
    check("select_engine_explain", "select_engine: refuse engine decisions as unknown_decision"),
    check("select_engine_media_admission", "select_engine: admit attested remote slot pairs only, without requiring a decision"),
    check("select_engine_admission_before_expiry", "select_engine: honour an engine decision from its selected module"),
    check("select_engine_admission_expiry", "select_engine: refuse a new admission at expiry"),
    check("select_engine_admission_foreign", "select_engine: refuse another attested module even for its authorized pair"),
    check("select_engine_admission_pair", "select_engine: refuse another authorized provider/model pair"),
    check("select_engine_admission_replay", "select_engine: replay the pre-expiry admission unchanged after expiry"),
    check("select_engine_admission_caller_local", "select_engine: refuse caller-local decisions for admission"),
    check("panel_full", "select_panel: enough eligible families fill the configured policy"),
    check("panel_shortfall", "select_panel: one candidate seats one member with shortfall"),
    check("panel_family_and_route_unique", "select_panel: no repeated family or route; families match the catalog"),
    check("panel_temperatures_by_slot", "select_panel: temperatures follow slot indices, absent past the list"),
    check("panel_same_seed", "select_panel: reset and reconfigure, then compare deterministic panel members for the same seed"),
    check("panel_replacement_preserves_slot", "select_panel: replacements preserve phase and temperature in top and extra slots"),
    check("panel_replacement_detail_requires_cause", "select_panel: a seated replacement must not carry diagnostic detail"),
    check("panel_foreign_replacement_unknown", "select_panel: a foreign panel replacement is unknown_decision"),
    check("panel_replacement_invalid_slot", "select_panel: replace.slot at policy.size is invalid_request naming replace.slot"),
    check("budget_quote_matches_helper", "budget_quote: the answer equals budget::quote over retained selection capacities"),
    check("budget_quote_foreign_unknown", "budget_quote: a foreign decision is unknown_decision"),
    check("explain_matches_selection", "explain: check_explain validates decision, route, runner and serving provider"),
    check("explain_foreign_unknown", "explain: a foreign decision is unknown_decision"),
    check("account_status_blocked_limited", "account_status: a blocked account is limited with its configured reset"),
    check("account_status_unknown", "account_status: an unconfigured account is unknown with no reset"),
    check("head_choices_valid", "head_choices: check_head_choices validates sorting, uniqueness and default_variant without prescribing provider listing policy"),
    check("head_choices_runner_routes", "head_choices: every listed pair belongs to the named runner's route_keys"),
    check("head_choices_non_starting_empty", "head_choices: starts_sessions false answers empty models"),
    check("head_choices_billing_valid", "head_choices: decoding rejects billing outside subscription/per_token as malformed"),
    check("consumer_models_valid", "consumer_models: check_consumer_models validates echo, ordering, uniqueness and finite scores; decoding checks billing"),
    check("consumer_models_unknown_consumer", "consumer_models: an unknown consumer gets unknown_consumer naming consumer"),
    check("report_usage_run_id_repeat_accepted", "report_usage_run_id: different runs and a repeated decision/run pair with changed usage are accepted"),
    check("report_usage_run_id_overlong_refused", "report_usage_run_id: a 129-character id is invalid_request naming usage.run_id"),
    check("pin_slot_configured_pinned", "select_pin_slot: a configured slot answers pinned on its configured route"),
    check("pin_slot_unknown_unavailable", "select_pin_slot: an unknown slot is pin_unavailable naming the slot with unknown_slot"),
    check("undeclared_pin_slot_refused", "without select_pin_slot: pin_slot is refused by name"),
    check("available_routes_honoured", "select_available_routes: the answered route is among available_routes"),
    check("available_routes_empty_refused", "select_available_routes: an empty list is refused by name"),
    check("undeclared_available_routes_refused", "without select_available_routes: available_routes is refused by name"),
    check("prefer_prompt_cache_accepted", "select_prefer_prompt_cache: a request with the preference answers and passes check_answer"),
    check("undeclared_prefer_prompt_cache_refused", "without select_prefer_prompt_cache: prefer_prompt_cache is refused by name"),
    check("settings_unresolved_selects", "select_settings: an unconfigured project selects with project_settings unresolved"),
    check("settings_unknown_member_refused", "select_settings: an unknown settings member is refused by field path"),
    check("undeclared_settings_refused", "without select_settings: settings is refused by name"),
    check("backoff_with_admission_accepted", "observe_backoff: transient_backoff with retry_at_ms and an admission answers"),
    check("backoff_missing_retry_refused", "observe_backoff: transient_backoff without retry_at_ms is refused by name"),
    check("observe_credential_refused_requires_backoff", "observe_credential_refused: describe also declares observe_backoff"),
    check("credential_refused_blocks_credential", "observe_credential_refused: a refused credential is not admitted again, across runners and models"),
    check("credential_refused_retry_at_ms_refused", "observe_credential_refused: retry_at_ms is refused by name"),
    check("undeclared_observe_decision_id_refused", "without observe_backoff: decision_id is refused by name"),
    check("undeclared_observe_retry_at_ms_refused", "without observe_backoff: retry_at_ms is refused by name"),
    check("undeclared_observe_provider_code_refused", "without observe_backoff: provider_code is refused by name"),
    check("account_for_tier_available", "account_for records fast and pins an account whose plan includes it"),
    check("account_for_tier_fallback", "account_for steps ultrafast down to fast then standard, recording each choice"),
    check("account_for_tier_undeclared", "a caller sends no tier to a non-declaring router and receives standard admission"),
    check("continue_credit_window_refused", "account_continue: credit is refused by exhausted.limit_window because continuation is quota-only"),
    check("continue_offer_excludes_exhausted_account", "account_continue: an offer never names the exhausted account"),
    check("continue_offer_preserves_provider", "account_continue: the offered credential has the previous provider"),
    check("continue_offer_preserves_auth_method", "account_continue: the offered credential has the previous auth method"),
    check("continue_offer_preserves_model", "account_continue: the offered credential is eligible for the previous model"),
    check("continue_offer_excludes_known_hold", "account_continue: an account under a known hold for the relevant window or pool is not offered; the check blocks a whole account"),
    check("continue_same_id_returns_identical_offer", "account_continue: retrying the same selection id returns the identical offer"),
    check("continue_changed_question_refused_by_selection_id", "account_continue: a reused selection id with a changed parsed question is invalid_request naming selection_id; the first answer stands"),
    check("continue_hold_unknown_until_stays_unknown", "account_continue: a hold with unknown reset omits until_ms, never substitutes now"),
    check("continue_undeclared_group_refuses_mode", "without account_continue: continue is invalid_request naming mode"),
    check("service_tier_eligible_account", "service_tier: decision admission uses an eligible account and records its non-standard tier with or without a repeated request tier"),
    check("service_tier_fallback_order", "service_tier: ultrafast falls back to fast then standard, recording service_tier_chosen"),
    check("service_tier_absent_is_standard", "service_tier: an absent tier, with no tier settings for the consumer, means standard"),
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
        "account_observe_credit_without_reset_accepted",
        "route.account_observe accepts account_limit_exhausted with limit_window credit and no reset evidence or retry hint",
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
    check("questions_requirement_honoured", "question limits admit a model at the served limit and refuse when no model meets it; not applicable without the requirement or model-input fixture hook"),
    check("max_cost_usd_requirement_honoured", "an input-cost cap admits equality, refuses over-cap prices, and excludes unstated input prices; not applicable without the requirements or model-input fixture hook"),
];

/// Where the kit checks less than the contract says, and why. Every report
/// prints these.
pub const NARROWINGS: &[&str] = &[
    "engine_reports: attested owner admission is checked, but rolling speed-window retention, latency inspection and realtime filtering are provider policy and are not observable through these hooks",
    "select_engine: malformed slot health diagnostics, retained consumer attribution and scope isolation are not inspected. Remote reachability is exercised with a cooldown, not separate backoff, quota or recorded-failure fixtures. Accuracy and cost are supplied fixture units, not real billing or engine measurements.",
    "report_usage_run_id: the hooks expose no usage counts, so the kit checks acceptance of distinct runs and repeated pairs with changed usage, not deduplicated totals",
    "consumer_models: the kit checks answer shape and unknown consumers, not stored score/exclusion policy or ownership admission; head_choices billing is checked by decoding, not billing policy or pin_only selection",
    "report_outcomes: the availability and affinity effects of the four added outcomes are unobservable; covered by contract vectors and Outcome::effect unit tests",
    "observe_backoff: the fence a transient_backoff observation sets and the decision-owner authorization arm are unobservable with these hooks",
    "select_prefer_prompt_cache: acceptance is checked, not internal tie-breaking",
    "head_choices: fixture models have no variants; variant rules are covered by check_head_choices unit tests",
    "select_panel: modalities are not visible in an answer",
    "Continuation checks use seeded credential/provider/auth/model facts and whole-account holds to test exclusion of accounts held for the relevant window or pool. They compare complete same-id offers and reject changed parsed questions without replacing the first answer, but cannot count internal reservations, inspect decision ancestry/pricing or exhaustion persistence, advance expiry, verify owner-scope isolation, or observe runner durability, frozen session plans, tools and renderers. Terminal refusal policy, transport retries and pool/window-specific holds are not exercised.",
    "priced_only is checked only as role.describe states it; the kit does not check that a priced_only consumer is never answered with a subscription route.",
    "Tool provider reachability is configured for every project at once, so the kit does not check that reachability is read for the scoped project.",
    "Affinity is checked with one failure outcome (failed); the other failure outcomes, and the rule that a cancelled or completed report keeps the preference, are not checked.",
    "account_observe idempotency is checked for one runner; the kit does not check that another runner's equal event_id is a separate event.",
    "Credit checks cover observation acceptance without reset evidence and refusal as a continuation window, not the one-hour fence cap, single probe, quota-reading isolation, or continued credential validity.",
    "The kit does not check that the runner ops refuse a caller that is not a runner module, nor the account_for refusals other than admission_conflict, unknown_decision and invalid_request for malformed pools, undeclared continuation mode (field: mode), credit continuation windows (field: exhausted.limit_window), and a changed continuation question under a reused selection id (field: selection_id).",
    "Eligibility is checked with model scope and another model on the same account. Account scope cannot distinguish a legitimate account-wide eligibility fence from an incorrect exhaustion/hold with the current hooks, so that distinction is not checked.",
    "Parent preference and fallback are checked on fresh admit requests; priority against an existing child pin, requested_account, renew or reassign is not checked.",
    "Pool isolation, disabled-window omission by runners, and request-size fencing policy are not exercised by the current catalog and provider hooks; vectors check the wire members only.",
    "Declaration precedence is checked with max_output_tokens only; the other demand fields are not independently checked for precedence.",
    "The consumer-declaration checks do not check invalid catalog declarations or provider health reports for invalid, unverified or conflicting entries.",
    "requirements other than tool_capabilities, the declared max_output_tokens floor, questions and max_cost_usd are not checked against the catalog beyond what every check's answer must meet. The new input checks use supplied model facts, not real token estimation or billing.",
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
    /// The check is inapplicable to the provider's declared groups: serving
    /// rules need the group, while undeclared-group refusal checks need its
    /// absence. Either kind of skipped check leaves the verdict a pass.
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
    /// No check failed. Checks skipped because a group is absent or because
    /// it is present (for undeclared-group refusal checks) do not change this.
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
