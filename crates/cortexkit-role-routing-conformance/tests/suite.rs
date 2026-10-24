//! Tests of the kit itself, against the reference fake in `fake/`.
//! Conformance of a real provider is run by that provider, never here.
//!
//! Each deliberate break in the fake must turn its check red by name, and
//! only that check: each test pins exactly which checks a break turns red,
//! so a check that starts passing a break, or a break that starts leaking
//! into unrelated checks, shows up.

mod fake;

use cortexkit_role_routing_conformance::{run_suite, CheckOutcome, SuiteVerdict, CHECKS};
use fake::{Defects, FakeSubject};

/// Run the fake with `defects`, and check that exactly the check `red`
/// fails, for a reason containing `because`.
async fn breaks(defects: Defects, red: &str, because: &str) {
    breaks_all(defects, &[red], because).await;
}

/// Run the fake with `defects`, and check that exactly the checks `reds`
/// fail, the first for a reason containing `because`. For a break that
/// rightly fails more than one check.
async fn breaks_all(defects: Defects, reds: &[&str], because: &str) {
    let report = run_suite(&FakeSubject::new(defects)).await;
    assert_eq!(report.failing(), reds, "{}", report.render());
    let red = reds[0];
    match report.outcome(red) {
        Some(CheckOutcome::Failed { reason }) => {
            assert!(reason.contains(because), "{red}: {reason}")
        }
        other => panic!("{red} should fail, got {other:?}"),
    }
    assert!(matches!(report.verdict, SuiteVerdict::Failed { .. }));
}

/// The checks of the `select_hold_control` group, in run order.
const HOLD_CHECKS: &[&str] = &[
    "hold_answers_blocked_entry_within_limits",
    "hold_moves_when_reset_beyond_window",
    "hold_moves_when_reset_unknown",
    "hold_moves_beyond_caller_cap",
    "hold_ok_false_moves_to_usable_entry",
    "hold_ok_false_falls_back_to_soonest_entry",
    "hold_nothing_within_limit_accounts_blocked",
    "hold_demand_mode_skips_blocked_model",
    "hold_account_for_applies_recorded_cap",
];

#[tokio::test]
async fn without_the_group_the_hold_checks_are_not_applicable_and_the_verdict_passes() {
    let report = run_suite(&FakeSubject::without_hold_control(Defects::default())).await;
    assert_eq!(report.not_applicable(), HOLD_CHECKS, "{}", report.render());
    for check in &report.checks {
        if !HOLD_CHECKS.contains(&check.check) {
            assert_eq!(check.outcome, CheckOutcome::Passed, "{}", check.check);
        }
    }
    assert_eq!(report.verdict, SuiteVerdict::Passed);
    let rendered = report.render();
    assert!(rendered.contains(
        "N/A  hold_answers_blocked_entry_within_limits: role.describe does not declare select_hold_control"
    ));
    assert!(rendered.contains("VERDICT passed"));
}

#[tokio::test]
async fn a_hold_break_without_the_group_is_not_reached() {
    // The hold checks never run against a provider that does not declare
    // the group, so a hold break there leaves the verdict a pass.
    let report = run_suite(&FakeSubject::without_hold_control(Defects {
        never_holds_entry: true,
        ..Defects::default()
    }))
    .await;
    assert_eq!(report.verdict, SuiteVerdict::Passed, "{}", report.render());
}

#[tokio::test]
async fn naming_an_undeclared_harness_fails() {
    breaks(
        Defects {
            invents_harness: true,
            ..Defects::default()
        },
        "harness_only_when_declared",
        "was answered with harness Some(\"default-harness\"); runner-two declared none",
    )
    .await;
}

#[tokio::test]
async fn dropping_a_declared_harness_fails() {
    breaks(
        Defects {
            drops_harness: true,
            ..Defects::default()
        },
        "harness_only_when_declared",
        "was answered with harness None; runner-one declared \"example-harness\"",
    )
    .await;
}

#[tokio::test]
async fn never_holding_a_blocked_entry_fails() {
    // The recorded-cap check needs a held answer to start from, so it
    // fails too.
    breaks_all(
        Defects {
            never_holds_entry: true,
            ..Defects::default()
        },
        &[
            "hold_answers_blocked_entry_within_limits",
            "hold_account_for_applies_recorded_cap",
        ],
        "was answered example-provider/model-a, not example-subscription/model-c",
    )
    .await;
}

#[tokio::test]
async fn holding_beyond_the_reset_window_fails() {
    breaks(
        Defects {
            ignores_hold_window: true,
            ..Defects::default()
        },
        "hold_moves_when_reset_beyond_window",
        "was answered example-subscription/model-c, not example-provider/model-a",
    )
    .await;
}

#[tokio::test]
async fn holding_on_an_unknown_reset_fails() {
    breaks(
        Defects {
            holds_on_unknown_reset: true,
            ..Defects::default()
        },
        "hold_moves_when_reset_unknown",
        "was answered example-subscription/model-c, not example-provider/model-a",
    )
    .await;
}

#[tokio::test]
async fn ignoring_the_callers_cap_fails() {
    breaks(
        Defects {
            ignores_caller_cap: true,
            ..Defects::default()
        },
        "hold_moves_beyond_caller_cap",
        "was answered example-subscription/model-c, not example-provider/model-a",
    )
    .await;
}

#[tokio::test]
async fn holding_despite_hold_ok_false_fails() {
    // Reading hold_ok false as true also holds the first entry where the
    // fallback must take the soonest, so both hold_ok false checks fail.
    breaks_all(
        Defects {
            holds_despite_hold_ok_false: true,
            ..Defects::default()
        },
        &[
            "hold_ok_false_moves_to_usable_entry",
            "hold_ok_false_falls_back_to_soonest_entry",
        ],
        "was answered example-subscription/model-c, not example-provider/model-a",
    )
    .await;
}

#[tokio::test]
async fn falling_back_in_ladder_order_fails() {
    breaks(
        Defects {
            fallback_in_ladder_order: true,
            ..Defects::default()
        },
        "hold_ok_false_falls_back_to_soonest_entry",
        "was answered example-subscription/model-c, not example-subscription-two/model-e",
    )
    .await;
}

#[tokio::test]
async fn an_accounts_blocked_reset_from_the_first_entry_fails() {
    breaks(
        Defects {
            accounts_blocked_reset_from_first_entry: true,
            ..Defects::default()
        },
        "hold_nothing_within_limit_accounts_blocked",
        "not the earliest known reset",
    )
    .await;
}

#[tokio::test]
async fn an_invented_accounts_blocked_reset_fails() {
    breaks(
        Defects {
            accounts_blocked_invents_reset: true,
            ..Defects::default()
        },
        "hold_nothing_within_limit_accounts_blocked",
        "though no reset is known",
    )
    .await;
}

#[tokio::test]
async fn holding_in_demand_mode_fails() {
    breaks(
        Defects {
            holds_in_demand_mode: true,
            ..Defects::default()
        },
        "hold_demand_mode_skips_blocked_model",
        "was answered example-subscription/model-c, not example-subscription-two/model-e",
    )
    .await;
}

#[tokio::test]
async fn account_for_ignoring_the_recorded_cap_fails() {
    breaks(
        Defects {
            account_for_ignores_recorded_cap: true,
            ..Defects::default()
        },
        "hold_account_for_applies_recorded_cap",
        "recorded hold_wait_max_s 600 was refused account_hold for a 700 s wait",
    )
    .await;
}

#[tokio::test]
async fn a_faithful_provider_passes_every_check() {
    let report = run_suite(&FakeSubject::new(Defects::default())).await;
    assert_eq!(report.checks.len(), CHECKS.len());
    for check in &report.checks {
        assert_eq!(
            check.outcome,
            CheckOutcome::Passed,
            "{}\n{}",
            check.check,
            report.render()
        );
    }
    assert_eq!(report.verdict, SuiteVerdict::Passed);
    assert!(report.render().contains("VERDICT passed"));
}

#[test]
fn check_names_are_unique() {
    let mut names: Vec<&str> = CHECKS.iter().map(|check| check.name).collect();
    names.sort_unstable();
    names.dedup();
    assert_eq!(names.len(), CHECKS.len());
}

#[tokio::test]
async fn dropping_priced_only_fails_describe() {
    breaks(
        Defects {
            describe_drops_priced_only: true,
            ..Defects::default()
        },
        "describe_lists_consumers_and_fields",
        "role.describe states priced_only false for consumer codemode, configured true",
    )
    .await;
}

#[tokio::test]
async fn ignoring_an_unknown_requirement_fails() {
    breaks(
        Defects {
            ignores_unknown_requirements: true,
            ..Defects::default()
        },
        "unknown_requirement_refused_by_name",
        "a select with the unknown requirement gpu_count was answered",
    )
    .await;
}

#[tokio::test]
async fn answering_an_unknown_consumer_fails() {
    breaks(
        Defects {
            answers_unknown_consumer: true,
            ..Defects::default()
        },
        "unknown_consumer_refused",
        "a select for an unconfigured consumer was answered",
    )
    .await;
}

#[tokio::test]
async fn serving_another_modules_consumer_fails() {
    breaks(
        Defects {
            serves_foreign_consumer: true,
            ..Defects::default()
        },
        "declared_consumer_non_owner_refused",
        "not refused consumer_not_owned",
    )
    .await;
}

#[tokio::test]
async fn ignoring_a_declared_default_fails() {
    breaks(
        Defects {
            ignores_declared_default: true,
            ..Defects::default()
        },
        "declared_consumer_owner_uses_default",
        "ignored the declared max_output_tokens default",
    )
    .await;
}

#[tokio::test]
async fn letting_a_declaration_override_config_fails() {
    breaks(
        Defects {
            declaration_overrides_config: true,
            ..Defects::default()
        },
        "operator_config_overrides_declared_default",
        "operator config overrides request and declaration was refused",
    )
    .await;
}

#[tokio::test]
async fn admitting_only_by_caller_principal_fails() {
    breaks(
        Defects {
            admits_only_declaring_principal: true,
            ..Defects::default()
        },
        "declared_consumer_scope_owner_admitted",
        "carrying the declaring module's scope was refused consumer_not_owned",
    )
    .await;
}

#[tokio::test]
async fn trusting_unverified_declarations_fails() {
    breaks(
        Defects {
            trusts_unverified_declarations: true,
            ..Defects::default()
        },
        "declared_consumer_unverified_entry_ignored",
        "not refused unknown_consumer",
    )
    .await;
}

#[tokio::test]
async fn letting_first_declarer_win_conflict_fails() {
    breaks(
        Defects {
            first_declarer_wins_conflict: true,
            ..Defects::default()
        },
        "declared_consumer_conflict_refused",
        "not refused consumer_not_owned",
    )
    .await;
}

#[tokio::test]
async fn ignoring_exclude_fails() {
    breaks(
        Defects {
            ignores_exclude: true,
            ..Defects::default()
        },
        "exclude_honoured",
        "route \"example-provider/model-a\" is in the request's exclude",
    )
    .await;
}

#[tokio::test]
async fn answering_a_runner_outside_runners_fails() {
    breaks(
        Defects {
            ignores_runners: true,
            ..Defects::default()
        },
        "runners_honoured",
        "runner \"runner-one\" is not one of the request's runners",
    )
    .await;
}

#[tokio::test]
async fn dropping_reset_at_ms_fails() {
    // accounts_blocked is a timed cause too, so its check fails with it.
    breaks_all(
        Defects {
            drops_reset_at_ms: true,
            ..Defects::default()
        },
        &[
            "timed_refusals_carry_reset_at_ms",
            "hold_nothing_within_limit_accounts_blocked",
        ],
        "a timed cause, but carries no reset_at_ms",
    )
    .await;
}

#[tokio::test]
async fn reading_scope_from_the_body_fails() {
    breaks(
        Defects {
            reads_scope_from_body: true,
            ..Defects::default()
        },
        "scope_comes_from_route_stamp",
        "scope was read from the body",
    )
    .await;
}

#[tokio::test]
async fn accepting_another_principals_report_fails() {
    breaks(
        Defects {
            accepts_report_from_any_principal: true,
            ..Defects::default()
        },
        "report_from_other_principal_unknown_decision",
        "a report from another principal on its decision was answered",
    )
    .await;
}

#[tokio::test]
async fn re_recording_a_repeated_report_fails() {
    breaks(
        Defects {
            rerecords_repeated_report: true,
            ..Defects::default()
        },
        "repeated_report_returns_first_answer",
        "the same report sent again answered",
    )
    .await;
}

#[tokio::test]
async fn overwriting_a_conflicting_report_fails() {
    breaks(
        Defects {
            overwrites_conflicting_report: true,
            ..Defects::default()
        },
        "different_report_is_report_conflict",
        "a report with outcome failed after one with completed was answered",
    )
    .await;
}

#[tokio::test]
async fn keeping_affinity_after_a_failure_fails() {
    breaks(
        Defects {
            affinity_survives_failure: true,
            ..Defects::default()
        },
        "affinity_holds_until_failure_report",
        "after a failure report on example-provider/model-b, a select with affinity affinity-check was still answered",
    )
    .await;
}

#[tokio::test]
async fn an_expiry_in_the_past_fails() {
    breaks(
        Defects {
            expiry_in_past: true,
            ..Defects::default()
        },
        "answer_carries_limits_and_future_expiry",
        "is not in the future",
    )
    .await;
}

#[tokio::test]
async fn reallocating_a_repeated_admission_fails() {
    breaks(
        Defects {
            account_for_reallocates_on_replay: true,
            ..Defects::default()
        },
        "account_for_idempotent_on_admission_id",
        "the same admission sent again answered",
    )
    .await;
}

#[tokio::test]
async fn admitting_a_changed_body_fails() {
    breaks(
        Defects {
            account_for_ignores_body_change: true,
            ..Defects::default()
        },
        "account_for_different_body_admission_conflict",
        "the same admission_id with another conversation_key was answered",
    )
    .await;
}

#[tokio::test]
async fn honouring_a_decision_from_another_runner_fails() {
    breaks(
        Defects {
            account_for_ignores_decision_runner: true,
            ..Defects::default()
        },
        "account_for_foreign_decision_unknown",
        "an admission from runner-two naming a decision for runner runner-one was answered",
    )
    .await;
}

#[tokio::test]
async fn honouring_a_decision_for_another_model_fails() {
    breaks(
        Defects {
            account_for_ignores_decision_model: true,
            ..Defects::default()
        },
        "account_for_foreign_decision_unknown",
        "an admission for model-d naming a decision for example-subscription/model-c was answered",
    )
    .await;
}

#[tokio::test]
async fn never_answering_duplicate_fails() {
    breaks(
        Defects {
            observe_never_duplicate: true,
            ..Defects::default()
        },
        "account_observe_idempotent_per_event_id",
        "the same event_id sent again answered duplicate false",
    )
    .await;
}

#[tokio::test]
async fn matching_tool_provider_module_names_fails() {
    breaks(
        Defects {
            tools_matched_by_module_name: true,
            ..Defects::default()
        },
        "tool_capabilities_matched_by_tag",
        "a select requiring the tool capability code.read/v1 was refused no_route; runner-one reaches example-tools",
    )
    .await;
}

// Each new rule has a positive probe and a single-rule faulty provider.
// The positive probes also make deliberate mutation runs fail by case name.
macro_rules! account_case {
    ($case:ident, $faulty:ident, $defect:ident, $because:literal) => {
        #[tokio::test]
        async fn $case() {
            let report = run_suite(&FakeSubject::new(Defects::default())).await;
            assert_eq!(
                report.outcome(stringify!($case)),
                Some(&CheckOutcome::Passed),
                "{}",
                report.render()
            );
        }

        #[tokio::test]
        async fn $faulty() {
            breaks(
                Defects {
                    $defect: true,
                    ..Defects::default()
                },
                stringify!($case),
                $because,
            )
            .await;
        }
    };
}

account_case!(
    parent_account_preference,
    choosing_parent_despite_alternative_fails,
    ignores_parent_preference,
    "picked the parent's account"
);
account_case!(
    parent_account_fallback,
    refusing_parent_fallback_fails,
    refuses_parent_fallback,
    "child admission was refused"
);
account_case!(
    eligibility_never_holds_or_exhausts,
    holding_on_eligibility_fails,
    holds_on_eligibility,
    "another model after eligibility report was refused account_hold"
);
account_case!(
    unknown_pool_accepted,
    refusing_unknown_pool_fails,
    refuses_unknown_pool,
    "an unseen well-formed pool was refused"
);
account_case!(
    malformed_pool_refused_by_field,
    wrong_malformed_pool_detail_fails,
    malformed_pool_wrong_detail,
    "malformed pool must name detail.field pool"
);
account_case!(
    pool_body_admission_conflict,
    ignoring_pool_body_fails,
    ignores_pool_body,
    "changed pool under reused admission_id was answered"
);
account_case!(
    parent_body_admission_conflict,
    ignoring_parent_body_fails,
    ignores_parent_body,
    "changed parent_conversation_key under reused admission_id was answered"
);
account_case!(
    new_report_members_conflict,
    ignoring_new_report_members_fails,
    ignores_report_members,
    "changed pool under reused decision_id was answered"
);
