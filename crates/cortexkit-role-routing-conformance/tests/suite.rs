//! Tests of the kit itself, against the reference fake in `fake/`.
//! Conformance of a real provider is run by that provider, never here.
//!
//! Each deliberate break in the fake must turn its check red by name, and
//! only that check: each test pins exactly which checks a break turns red,
//! so a check that starts passing a break, or a break that starts leaking
//! into unrelated checks, shows up.
//! Panel checks share caller-side validation; their faulty-router matrix
//! requires the named check to fail even when another panel check also detects it.

mod fake;

const WARM_CREDENTIAL_CASES: &[&str] = &[
    "warm_credential_fresh_honoured",
    "warm_credential_hard_filter_ignored",
    "warm_credential_unknown_ignored",
    "warm_credential_existing_affinity",
    "warm_credential_admission_conflict",
    "warm_credential_non_admit_refused",
    "warm_credential_undeclared_refused",
];

#[tokio::test]
async fn warm_credential_cases_pass_and_gate_on_capability() {
    for declared in [true, false] {
        let subject = FakeSubject::new(Defects {
            extension: (!declared).then_some("warm_credential_without_group"),
            ..Defects::default()
        });
        let report = run_suite(&subject).await;
        assert_eq!(report.verdict, SuiteVerdict::Passed, "{}", report.render());
        for name in WARM_CREDENTIAL_CASES {
            let applicable = declared != (*name == "warm_credential_undeclared_refused");
            match report.outcome(name) {
                Some(CheckOutcome::Passed) if applicable => {}
                Some(CheckOutcome::NotApplicable { reason }) if !applicable => {
                    assert!(!reason.is_empty())
                }
                other => panic!(
                    "{name}, declared={declared}: {other:?}\n{}",
                    report.render()
                ),
            }
        }
    }
}

#[tokio::test]
async fn warm_credential_faulty_routers_fail_by_check_name() {
    for (name, reason) in [
        ("warm_credential_fresh_honoured", "expected credential"),
        ("warm_credential_hard_filter_ignored", "expected credential"),
        ("warm_credential_unknown_ignored", "unknown warm credential"),
        ("warm_credential_existing_affinity", "expected credential"),
        (
            "warm_credential_admission_conflict",
            "not refused admission_conflict",
        ),
        (
            "warm_credential_non_admit_refused",
            "not refused invalid_request",
        ),
        (
            "warm_credential_undeclared_refused",
            "not refused invalid_request",
        ),
    ] {
        breaks(
            Defects {
                extension: Some(name),
                ..Defects::default()
            },
            name,
            reason,
        )
        .await;
    }
}

const ROUTING_070_CASES: &[&str] = &[
    "panel_full",
    "panel_shortfall",
    "panel_family_and_route_unique",
    "panel_temperatures_by_slot",
    "panel_same_seed",
    "panel_replacement_preserves_slot",
    "panel_foreign_replacement_unknown",
    "panel_replacement_invalid_slot",
    "budget_quote_matches_helper",
    "budget_quote_foreign_unknown",
    "explain_matches_selection",
    "explain_foreign_unknown",
    "account_status_blocked_limited",
    "account_status_unknown",
    "head_choices_valid",
    "head_choices_runner_routes",
    "head_choices_non_starting_empty",
    "head_choices_billing_valid",
    "consumer_models_valid",
    "consumer_models_unknown_consumer",
    "report_usage_run_id_repeat_accepted",
    "report_usage_run_id_overlong_refused",
    "pin_slot_configured_pinned",
    "pin_slot_unknown_unavailable",
    "undeclared_pin_slot_refused",
    "available_routes_honoured",
    "available_routes_empty_refused",
    "undeclared_available_routes_refused",
    "prefer_prompt_cache_accepted",
    "undeclared_prefer_prompt_cache_refused",
    "settings_unresolved_selects",
    "settings_unknown_member_refused",
    "undeclared_settings_refused",
    "backoff_with_admission_accepted",
    "backoff_missing_retry_refused",
    "observe_credential_refused_requires_backoff",
    "credential_refused_blocks_credential",
    "credential_refused_retry_at_ms_refused",
    "undeclared_observe_decision_id_refused",
    "undeclared_observe_retry_at_ms_refused",
    "undeclared_observe_provider_code_refused",
];

#[tokio::test]
async fn routing_070_cases_and_not_applicable_split() {
    for declared in [true, false] {
        let subject = if declared {
            FakeSubject::new(Defects::default())
        } else {
            FakeSubject::without_extensions(Defects::default())
        };
        let report = run_suite(&subject).await;
        assert_eq!(report.verdict, SuiteVerdict::Passed, "{}", report.render());
        for name in ROUTING_070_CASES {
            let not_applicable = declared == name.starts_with("undeclared_");
            assert!(
                CHECKS.iter().any(|c| c.name == *name),
                "missing check {name}"
            );
            match report.outcome(name) {
                Some(CheckOutcome::Passed) if !not_applicable => {}
                Some(CheckOutcome::NotApplicable { reason }) if not_applicable => {
                    assert!(!reason.is_empty())
                }
                other => panic!(
                    "{name} with declared={declared}: {other:?}\n{}",
                    report.render()
                ),
            }
        }
    }
}

#[tokio::test]
async fn routing_070_faulty_routers_fail_each_case_by_name() {
    for name in ROUTING_070_CASES {
        let defects = Defects {
            extension: Some(name),
            ..Defects::default()
        };
        let subject = if name.starts_with("undeclared_") {
            FakeSubject::without_extensions(defects)
        } else {
            FakeSubject::new(defects)
        };
        let report = run_suite(&subject).await;
        assert!(
            matches!(report.outcome(name), Some(CheckOutcome::Failed { .. })),
            "faulty router must fail {name}: {}",
            report.render()
        );
        assert!(
            report
                .failing()
                .iter()
                .all(|failed| ROUTING_070_CASES.contains(failed)
                    || (*failed == "panel_replacement_detail_requires_cause"
                        && name.starts_with("panel_"))
                    || (*name == "head_choices_billing_valid"
                        && matches!(*failed,
                             "harness_limits_pairs_narrow" | "harness_limits_providers_narrow" | "harness_limits_invalid_falls_back" | "runner_requires_wire_api"))
                    // An invalid role.describe also fails the checks that
                    // validate role.describe before testing anything else.
                    || (*name == "observe_credential_refused_requires_backoff"
                        && matches!(
                            *failed,
                            "describe_lists_consumers_and_fields" | "select_engine_opt_in"
                        ))),
            "new faulty router must not break unrelated legacy checks: {}",
            report.render()
        );
        println!(
            "faulty router rejected by {name}: {:?}",
            report.outcome(name)
        );
    }
}

#[tokio::test]
async fn routing_078_panel_replacement_detail_requires_cause() {
    let check = "panel_replacement_detail_requires_cause";
    for declared in [true, false] {
        let subject = if declared {
            FakeSubject::new(Defects::default())
        } else {
            FakeSubject::without_extensions(Defects::default())
        };
        let report = run_suite(&subject).await;
        assert_eq!(report.verdict, SuiteVerdict::Passed, "{}", report.render());
        match report.outcome(check) {
            Some(CheckOutcome::Passed) if declared => {}
            Some(CheckOutcome::NotApplicable { reason }) if !declared => {
                assert!(!reason.is_empty())
            }
            other => panic!("{check} with declared={declared}: {other:?}"),
        }
    }
    let subject = FakeSubject::new(Defects {
        extension: Some(check),
        ..Defects::default()
    });
    let report = run_suite(&subject).await;
    assert!(
        matches!(report.outcome(check), Some(CheckOutcome::Failed { .. })),
        "faulty seated reply must fail {check}: {}",
        report.render()
    );
    assert_eq!(
        report.failing(),
        vec!["panel_replacement_preserves_slot", check],
        "{}",
        report.render()
    );
}

#[tokio::test]
async fn continue_signed_out_faulty_offer_fails_by_name() {
    breaks(
        Defects {
            continue_signed_out_offer: true,
            ..Defects::default()
        },
        "continue_signed_out",
        "offer names the signed-out account",
    )
    .await;
}

#[tokio::test]
async fn continue_signed_out_faulty_fresh_admission_fails_by_name() {
    breaks(
        Defects {
            continue_signed_out_fresh: true,
            ..Defects::default()
        },
        "continue_signed_out",
        "fresh admission names the signed-out credential",
    )
    .await;
}

#[tokio::test]
async fn continue_signed_out_faulty_no_spare_offer_fails_by_name() {
    breaks(
        Defects {
            continue_signed_out_no_spare: true,
            ..Defects::default()
        },
        "continue_signed_out",
        "without a spare must hold or refuse",
    )
    .await;
}

#[tokio::test]
async fn continue_signed_out_without_capability_refuses_by_field() {
    let report = run_suite(&FakeSubject::without_signed_out_control(Defects::default())).await;
    assert_eq!(report.verdict, SuiteVerdict::Passed, "{}", report.render());
    assert_eq!(
        report.outcome("continue_signed_out_undeclared_refuses_window"),
        Some(&CheckOutcome::Passed)
    );
    assert!(matches!(
        report.outcome("continue_signed_out"),
        Some(CheckOutcome::NotApplicable { .. })
    ));
}

#[tokio::test]
async fn continue_signed_out_faulty_undeclared_capability_fails_by_name() {
    let report = run_suite(&FakeSubject::without_signed_out_control(Defects {
        continue_signed_out_undeclared: true,
        ..Defects::default()
    }))
    .await;
    assert_eq!(
        report.failing(),
        ["continue_signed_out_undeclared_refuses_window"],
        "{}",
        report.render()
    );
    assert!(
        matches!(report.outcome("continue_signed_out_undeclared_refuses_window"),
        Some(CheckOutcome::Failed { reason }) if reason.contains("not refused invalid_request"))
    );
}

#[tokio::test]
async fn continue_faulty_exhausted_account_fails_by_name() {
    breaks(
        Defects {
            continue_exhausted: true,
            ..Defects::default()
        },
        "continue_offer_excludes_exhausted_account",
        "exhausted account",
    )
    .await;
}

#[tokio::test]
async fn continue_faulty_provider_fails_by_name() {
    breaks(
        Defects {
            continue_provider: true,
            ..Defects::default()
        },
        "continue_offer_preserves_provider",
        "changes provider",
    )
    .await;
}

#[tokio::test]
async fn continue_faulty_auth_method_fails_by_name() {
    breaks(
        Defects {
            continue_auth: true,
            ..Defects::default()
        },
        "continue_offer_preserves_auth_method",
        "changes auth method",
    )
    .await;
}

#[tokio::test]
async fn continue_faulty_model_fails_by_name() {
    breaks(
        Defects {
            continue_model: true,
            ..Defects::default()
        },
        "continue_offer_preserves_model",
        "changes model",
    )
    .await;
}

#[tokio::test]
async fn continue_faulty_known_hold_fails_by_name() {
    breaks(
        Defects {
            continue_held: true,
            ..Defects::default()
        },
        "continue_offer_excludes_known_hold",
        "known hold",
    )
    .await;
}

#[tokio::test]
async fn continue_faulty_retry_fails_by_name() {
    breaks_all(
        Defects {
            continue_retry: true,
            ..Defects::default()
        },
        &[
            "continue_same_id_returns_identical_offer",
            "continue_changed_question_refused_by_selection_id",
        ],
        "retry changed offer",
    )
    .await;
}

#[tokio::test]
async fn continue_changed_question_preserves_first_answer() {
    let report = run_suite(&FakeSubject::new(Defects::default())).await;
    assert_eq!(
        report.outcome("continue_changed_question_refused_by_selection_id"),
        Some(&CheckOutcome::Passed),
        "{}",
        report.render()
    );
}

#[tokio::test]
async fn continue_faulty_changed_question_replay_fails_by_name() {
    breaks(
        Defects {
            continue_changed_question: true,
            ..Defects::default()
        },
        "continue_changed_question_refused_by_selection_id",
        "not refused invalid_request",
    )
    .await;
}

#[tokio::test]
async fn continue_faulty_unknown_reset_fails_by_name() {
    breaks(
        Defects {
            continue_unknown_now: true,
            ..Defects::default()
        },
        "continue_hold_unknown_until_stays_unknown",
        "untimed hold",
    )
    .await;
}

#[tokio::test]
async fn continue_faulty_undeclared_group_fails_by_name() {
    let subject = FakeSubject::without_continue_control(Defects {
        continue_undeclared: true,
        ..Defects::default()
    });
    let report = run_suite(&subject).await;
    assert_eq!(
        report.failing(),
        ["continue_undeclared_group_refuses_mode"],
        "{}",
        report.render()
    );
    assert!(
        matches!(report.outcome("continue_undeclared_group_refuses_mode"),
        Some(CheckOutcome::Failed { reason }) if reason.contains("not refused invalid_request"))
    );
}

#[tokio::test]
async fn continue_without_group_refuses_and_skips_optional_cases() {
    let report = run_suite(&FakeSubject::without_continue_control(Defects::default())).await;
    assert_eq!(report.verdict, SuiteVerdict::Passed, "{}", report.render());
    for check in CHECKS.iter().filter(|c| c.name.starts_with("continue_")) {
        if check.name == "continue_undeclared_group_refuses_mode" {
            assert_eq!(report.outcome(check.name), Some(&CheckOutcome::Passed));
        } else {
            assert!(
                matches!(
                    report.outcome(check.name),
                    Some(CheckOutcome::NotApplicable { .. })
                ),
                "{}",
                report.render()
            );
        }
    }
}

const SERVICE_TIER_CHECKS: &[&str] = &[
    "service_tier_eligible_account",
    "service_tier_fallback_order",
    "service_tier_absent_is_standard",
    "service_tier_report_members_accepted",
];

async fn decision_tier_fault_fails_by_name(defects: Defects) {
    let report = run_suite(&FakeSubject::new(defects)).await;
    let expected = "service_tier_eligible_account";
    assert_eq!(report.failing(), [expected], "{}", report.render());
    eprintln!("FAIL {expected}: {:?}", report.outcome(expected));
}

#[tokio::test]
async fn service_tier_decision_answer_requires_chosen() {
    decision_tier_fault_fails_by_name(Defects {
        decision_tier_chosen_missing: true,
        ..Defects::default()
    })
    .await;
}

#[tokio::test]
async fn service_tier_repeated_decision_tier_requires_chosen() {
    decision_tier_fault_fails_by_name(Defects {
        repeated_decision_tier_wrong: true,
        ..Defects::default()
    })
    .await;
}

#[tokio::test]
async fn account_for_tier_faithful_and_undeclared_routers_pass() {
    let report = run_suite(&FakeSubject::new(Defects::default())).await;
    for name in ["account_for_tier_available", "account_for_tier_fallback"] {
        assert_eq!(
            report.outcome(name),
            Some(&CheckOutcome::Passed),
            "{}",
            report.render()
        );
    }
    let report = run_suite(&FakeSubject::without_service_tier(Defects::default())).await;
    assert_eq!(
        report.outcome("account_for_tier_undeclared"),
        Some(&CheckOutcome::Passed),
        "{}",
        report.render()
    );
    for name in ["account_for_tier_available", "account_for_tier_fallback"] {
        assert!(
            matches!(
                report.outcome(name),
                Some(CheckOutcome::NotApplicable { .. })
            ),
            "{}",
            report.render()
        );
    }
}

#[tokio::test]
async fn account_for_tier_faulty_routers_fail_by_name() {
    for (defects, expected, undeclared) in [
        (
            Defects {
                account_tier_ignores: true,
                ..Defects::default()
            },
            "account_for_tier_available",
            false,
        ),
        (
            Defects {
                account_tier_skip_fast: true,
                ..Defects::default()
            },
            "account_for_tier_fallback",
            false,
        ),
        (
            Defects {
                account_tier_undeclared_fast: true,
                ..Defects::default()
            },
            "account_for_tier_undeclared",
            true,
        ),
    ] {
        let subject = if undeclared {
            FakeSubject::without_service_tier(defects)
        } else {
            FakeSubject::new(defects)
        };
        let report = run_suite(&subject).await;
        assert_eq!(report.failing(), [expected], "{}", report.render());
        eprintln!("FAIL {expected}: {:?}", report.outcome(expected));
    }
}

#[tokio::test]
async fn service_tier_faulty_routers_fail_each_case_by_name() {
    for (defects, expected, because) in [
        (
            Defects {
                tier_ineligible_account: true,
                ..Defects::default()
            },
            SERVICE_TIER_CHECKS[0],
            "ineligible account",
        ),
        (
            Defects {
                tier_skips_fast_fallback: true,
                ..Defects::default()
            },
            SERVICE_TIER_CHECKS[1],
            "fallback expected",
        ),
        (
            Defects {
                tier_absent_is_fast: true,
                ..Defects::default()
            },
            SERVICE_TIER_CHECKS[2],
            "did not mean standard",
        ),
        (
            Defects {
                tier_report_refused: true,
                ..Defects::default()
            },
            SERVICE_TIER_CHECKS[3],
            "was refused",
        ),
    ] {
        breaks(defects, expected, because).await;
    }
}

#[tokio::test]
async fn service_tier_without_group_is_not_applicable_by_name() {
    let report = run_suite(&FakeSubject::without_service_tier(Defects::default())).await;
    for name in SERVICE_TIER_CHECKS {
        assert!(
            matches!(
                report.outcome(name),
                Some(CheckOutcome::NotApplicable { .. })
            ),
            "{name}: {}",
            report.render()
        );
    }
}

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
    let mut not_applicable = vec![
        "placement_consumer_declarations",
        "placement_harness_facts",
        "placement_decision_harness",
        "placement_select_kind",
        "placement_eligible_routes",
        "placement_select_price",
        "placement_attempt_outcomes",
        "undeclared_pin_slot_refused",
        "undeclared_available_routes_refused",
        "undeclared_prefer_prompt_cache_refused",
        "undeclared_settings_refused",
        "undeclared_observe_decision_id_refused",
        "undeclared_observe_retry_at_ms_refused",
        "undeclared_observe_provider_code_refused",
        "account_for_tier_undeclared",
        "warm_credential_undeclared_refused",
        "continue_signed_out_undeclared_refuses_window",
        "continue_undeclared_group_refuses_mode",
        "undeclared_harness_group_refuses_member",
    ];
    not_applicable.extend_from_slice(HOLD_CHECKS);
    not_applicable.push("undeclared_prefer_group_refuses_member");
    not_applicable.push("undeclared_exclude_providers_group_refuses_member");
    assert_eq!(
        report.not_applicable(),
        not_applicable,
        "{}",
        report.render()
    );
    for check in &report.checks {
        if !not_applicable.contains(&check.check) {
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
    breaks_all(
        Defects {
            invents_harness: true,
            ..Defects::default()
        },
        &[
            "harness_only_when_declared",
            "unrequested_harness_uses_declaration",
        ],
        "was answered with harness Some(\"default-harness\"); runner-two declared none",
    )
    .await;
}

#[tokio::test]
async fn dropping_a_declared_harness_fails() {
    breaks_all(
        Defects {
            drops_harness: true,
            ..Defects::default()
        },
        &[
            "harness_only_when_declared",
            "requested_harness_is_hosted_and_echoed",
            "unrequested_harness_uses_declaration",
        ],
        "was answered with harness None; runner-one declared \"alpha\"",
    )
    .await;
}

#[tokio::test]
async fn ignoring_requested_harness_membership_fails() {
    breaks_all(
        Defects {
            ignores_harness_filter: true,
            ..Defects::default()
        },
        &[
            "requested_harness_is_hosted_and_echoed",
            "harness_limits_pairs_narrow",
            "harness_limits_providers_narrow",
            "harness_limits_invalid_falls_back",
        ],
        "requested solo must be hosted and echoed",
    )
    .await;
}

#[tokio::test]
async fn not_echoing_requested_harness_fails() {
    breaks_all(
        Defects {
            echoes_default_harness: true,
            ..Defects::default()
        },
        &[
            "requested_harness_is_hosted_and_echoed",
            "harness_limits_pairs_narrow",
            "harness_limits_providers_narrow",
        ],
        "requested beta must be hosted and echoed",
    )
    .await;
}

#[tokio::test]
async fn wrong_unhosted_harness_reason_fails() {
    breaks_all(
        Defects {
            wrong_harness_refusal: true,
            ..Defects::default()
        },
        &[
            "requested_harness_is_hosted_and_echoed",
            "harness_limits_pairs_narrow",
            "harness_limits_providers_narrow",
            "harness_limits_invalid_falls_back",
        ],
        "unhosted harness needs reason harness_unavailable",
    )
    .await;
}

#[tokio::test]
async fn dropping_singleton_harness_fails() {
    breaks(
        Defects {
            ignores_single_harness: true,
            ..Defects::default()
        },
        "unrequested_harness_uses_declaration",
        "expected Some(\"solo\")",
    )
    .await;
}

#[tokio::test]
async fn ignoring_nonfirst_default_harness_fails() {
    breaks(
        Defects {
            chooses_first_harness: true,
            ..Defects::default()
        },
        "unrequested_harness_uses_declaration",
        "expected Some(\"beta\")",
    )
    .await;
}

#[tokio::test]
async fn choosing_first_of_multiple_harnesses_without_default_fails() {
    let report = run_suite(&FakeSubject::new(Defects {
        chooses_first_without_default: true,
        ..Defects::default()
    }))
    .await;
    assert_eq!(
        report.failing(),
        ["unrequested_harness_uses_declaration"],
        "{}",
        report.render()
    );
    match report.outcome("unrequested_harness_uses_declaration") {
        Some(CheckOutcome::Failed { reason }) => {
            assert!(reason.contains("expected None on runner-one"), "{reason}");
            assert!(reason.contains("harness: Some(\"alpha\")"), "{reason}");
        }
        other => panic!("unrequested_harness_uses_declaration should fail, got {other:?}"),
    }
    for check in [
        "harness_only_when_declared",
        "requested_harness_is_hosted_and_echoed",
        "non_starting_runner_never_selected",
    ] {
        assert_eq!(
            report.outcome(check),
            Some(&CheckOutcome::Passed),
            "{check}"
        );
    }
}

#[tokio::test]
async fn selecting_gateway_fails() {
    breaks(
        Defects {
            selects_gateway: true,
            ..Defects::default()
        },
        "non_starting_runner_never_selected",
        "starts_sessions false runner was selected",
    )
    .await;
}

#[tokio::test]
async fn without_harness_group_requests_are_refused() {
    let report = run_suite(&FakeSubject::without_harness_control(Defects::default())).await;
    assert_eq!(
        report.not_applicable(),
        [
            "placement_consumer_declarations",
            "placement_harness_facts",
            "placement_decision_harness",
            "placement_select_kind",
            "placement_eligible_routes",
            "placement_select_price",
            "placement_attempt_outcomes",
            "undeclared_pin_slot_refused",
            "undeclared_available_routes_refused",
            "undeclared_prefer_prompt_cache_refused",
            "undeclared_settings_refused",
            "undeclared_observe_decision_id_refused",
            "undeclared_observe_retry_at_ms_refused",
            "undeclared_observe_provider_code_refused",
            "account_for_tier_undeclared",
            "warm_credential_undeclared_refused",
            "continue_signed_out_undeclared_refuses_window",
            "continue_undeclared_group_refuses_mode",
            "requested_harness_is_hosted_and_echoed",
            "undeclared_prefer_group_refuses_member",
            "undeclared_exclude_providers_group_refuses_member"
        ]
    );
    assert_eq!(report.verdict, SuiteVerdict::Passed, "{}", report.render());
}

#[tokio::test]
async fn accepting_undeclared_harness_member_fails() {
    let report = run_suite(&FakeSubject::without_harness_control(Defects {
        accepts_undeclared_harness: true,
        ..Defects::default()
    }))
    .await;
    assert_eq!(
        report.failing(),
        ["undeclared_harness_group_refuses_member"],
        "{}",
        report.render()
    );
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
    passes_every_check(&FakeSubject::new(Defects::default())).await;
}

/// The contract leaves the mapping of a request's iq onto model facts to the
/// provider, so a provider reading iq 100 as "the strongest model I can
/// route" (always met) is as correct as one reading it as an absolute ability
/// no fixture model reaches. No check may depend on either reading.
#[tokio::test]
async fn a_provider_mapping_iq_relative_to_its_strongest_model_passes_every_check() {
    passes_every_check(&FakeSubject::with_relative_iq(Defects::default())).await;
}

/// Run the suite against `subject` and require every check to pass, except
/// the undeclared-group checks, which a provider declaring every group
/// cannot be asked.
async fn passes_every_check(subject: &FakeSubject) {
    let report = run_suite(subject).await;
    assert_eq!(report.checks.len(), CHECKS.len());
    for check in &report.checks {
        if [
            "placement_consumer_declarations",
            "placement_harness_facts",
            "placement_decision_harness",
            "placement_select_kind",
            "placement_eligible_routes",
            "placement_select_price",
            "placement_attempt_outcomes",
            "undeclared_pin_slot_refused",
            "undeclared_available_routes_refused",
            "undeclared_prefer_prompt_cache_refused",
            "undeclared_settings_refused",
            "undeclared_observe_decision_id_refused",
            "undeclared_observe_retry_at_ms_refused",
            "undeclared_observe_provider_code_refused",
            "account_for_tier_undeclared",
            "warm_credential_undeclared_refused",
            "undeclared_harness_group_refuses_member",
            "undeclared_prefer_group_refuses_member",
            "undeclared_exclude_providers_group_refuses_member",
            "continue_signed_out_undeclared_refuses_window",
            "continue_undeclared_group_refuses_mode",
        ]
        .contains(&check.check)
        {
            assert!(matches!(check.outcome, CheckOutcome::NotApplicable { .. }));
            continue;
        }
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
async fn ignoring_questions_is_detected() {
    breaks(
        Defects {
            ignore_questions: true,
            ..Defects::default()
        },
        "questions_requirement_honoured",
        "questions_requirement_honoured",
    )
    .await;
}

#[tokio::test]
async fn ignoring_max_cost_usd_is_detected() {
    breaks(
        Defects {
            ignore_max_cost_usd: true,
            ..Defects::default()
        },
        "max_cost_usd_requirement_honoured",
        "max_cost_usd_requirement_honoured",
    )
    .await;
}

#[tokio::test]
async fn decision_input_checks_without_fixture_hook_are_not_applicable() {
    let report = run_suite(&FakeSubject::without_model_input_hook()).await;
    assert!(report.failing().is_empty(), "{}", report.render());
    let expected = [
        "questions_requirement_honoured",
        "max_cost_usd_requirement_honoured",
    ];
    for name in expected {
        assert!(
            matches!(report.outcome(name), Some(CheckOutcome::NotApplicable { reason }) if reason.contains("no model-input facts hook")),
            "{}",
            report.render()
        );
    }
    let allowed_skips = [
        "placement_consumer_declarations",
        "placement_harness_facts",
        "placement_decision_harness",
        "placement_select_kind",
        "placement_eligible_routes",
        "placement_select_price",
        "placement_attempt_outcomes",
        "undeclared_pin_slot_refused",
        "undeclared_available_routes_refused",
        "undeclared_prefer_prompt_cache_refused",
        "undeclared_settings_refused",
        "undeclared_observe_decision_id_refused",
        "undeclared_observe_retry_at_ms_refused",
        "undeclared_observe_provider_code_refused",
        "account_for_tier_undeclared",
        "warm_credential_undeclared_refused",
        "undeclared_harness_group_refuses_member",
        "undeclared_prefer_group_refuses_member",
        "undeclared_exclude_providers_group_refuses_member",
        "continue_signed_out_undeclared_refuses_window",
        "continue_undeclared_group_refuses_mode",
        "questions_requirement_honoured",
        "max_cost_usd_requirement_honoured",
    ];
    let skipped: Vec<_> = CHECKS
        .iter()
        .filter(|check| allowed_skips.contains(&check.name))
        .map(|check| check.name)
        .collect();
    assert_eq!(report.not_applicable(), skipped);
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
    // The every-provider exclusion check compares its no_route with the one
    // an exclude of every route key gets, so ignoring exclude fails it too.
    breaks_all(
        Defects {
            ignores_exclude: true,
            ..Defects::default()
        },
        &[
            "exclude_honoured",
            "exclude_providers_every_provider_no_route",
        ],
        "route \"example-provider/model-a\" is in the request's exclude",
    )
    .await;
}

#[tokio::test]
async fn answering_a_runner_outside_runners_fails() {
    breaks_all(
        Defects {
            ignores_runners: true,
            ..Defects::default()
        },
        &["runners_honoured", "non_starting_runner_never_selected"],
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
    breaks_all(
        Defects {
            account_for_reallocates_on_replay: true,
            ..Defects::default()
        },
        &[
            "warm_credential_admission_conflict",
            "account_for_idempotent_on_admission_id",
        ],
        "hint conflict replaced the original admission answer",
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

#[tokio::test]
async fn pin_fallback_mislabelled_as_pinned_fails() {
    breaks(
        Defects {
            pin_fallback_labels_pinned: true,
            ..Defects::default()
        },
        "pin_fallback_routes_and_labels",
        "labelled",
    )
    .await;
}

#[tokio::test]
async fn pin_fallback_without_code_fails() {
    breaks(
        Defects {
            pin_fallback_omits_pin_reason: true,
            ..Defects::default()
        },
        "pin_fallback_routes_and_labels",
        "pin_reason",
    )
    .await;
}

#[tokio::test]
async fn fence_wider_than_route_and_account_fails() {
    breaks(
        Defects {
            fences_wide: true,
            ..Defects::default()
        },
        "report_reset_fences_only_decision_route_and_account",
        "another",
    )
    .await;
}

#[tokio::test]
async fn past_reset_that_fences_fails() {
    breaks(
        Defects {
            past_reset_fences: true,
            ..Defects::default()
        },
        "report_past_reset_fences_nothing_and_is_recorded",
        "fences",
    )
    .await;
}

#[tokio::test]
async fn past_reset_that_is_refused_fails() {
    breaks(
        Defects {
            past_reset_refused: true,
            ..Defects::default()
        },
        "report_past_reset_fences_nothing_and_is_recorded",
        "refused",
    )
    .await;
}

#[tokio::test]
async fn reset_beyond_24h_without_capping_fails() {
    breaks(
        Defects {
            reset_not_capped: true,
            ..Defects::default()
        },
        "report_reset_capped_at_24h",
        "without capping",
    )
    .await;
}

#[tokio::test]
async fn prefer_ignored_when_listed_adequate_fails() {
    breaks(
        Defects {
            ignores_prefer: true,
            ..Defects::default()
        },
        "prefer_prefers_listed_adequate_route",
        "ignored",
    )
    .await;
}

#[tokio::test]
async fn prefer_admitting_inadequate_fails() {
    breaks(
        Defects {
            admits_prefer: true,
            ..Defects::default()
        },
        "prefer_never_admits_inadequate_route",
        "admitted",
    )
    .await;
}

#[tokio::test]
async fn prefer_admitted_when_nothing_meets_fails() {
    breaks(
        Defects {
            admits_prefer_when_nothing_meets: true,
            ..Defects::default()
        },
        "prefer_never_admits_inadequate_route",
        "context_window above every fixture model's was answered",
    )
    .await;
}

#[tokio::test]
async fn without_prefer_group_requests_are_not_applicable() {
    let report = run_suite(&FakeSubject::without_prefer_control(Defects::default())).await;
    assert!(report
        .not_applicable()
        .contains(&"prefer_prefers_listed_adequate_route"));
    assert!(report
        .not_applicable()
        .contains(&"prefer_never_admits_inadequate_route"));
    assert_eq!(report.verdict, SuiteVerdict::Passed, "{}", report.render());
}

#[tokio::test]
async fn accepting_undeclared_prefer_member_fails() {
    let report = run_suite(&FakeSubject::without_prefer_control(Defects {
        accepts_undeclared_prefer: true,
        ..Defects::default()
    }))
    .await;
    assert_eq!(
        report.failing(),
        ["undeclared_prefer_group_refuses_member"],
        "{}",
        report.render()
    );
}

#[tokio::test]
async fn accepting_bad_usage_fails() {
    breaks(
        Defects {
            accepts_bad_usage: true,
            ..Defects::default()
        },
        "report_usage_strict",
        "was answered",
    )
    .await;
}

#[tokio::test]
async fn ignoring_usage_body_fails() {
    breaks(
        Defects {
            ignores_usage_body: true,
            ..Defects::default()
        },
        "new_report_members_conflict",
        "changed usage under reused decision_id was answered",
    )
    .await;
}

#[tokio::test]
async fn ignoring_exclude_providers_fails() {
    breaks_all(
        Defects {
            ignores_exclude_providers: true,
            ..Defects::default()
        },
        &[
            "exclude_providers_answers_another_provider",
            "exclude_providers_every_provider_no_route",
        ],
        "route \"example-provider/model-a\" is from a provider in the request's exclude_providers",
    )
    .await;
}

#[tokio::test]
async fn answering_an_excluded_provider_when_nothing_else_is_left_fails() {
    breaks(
        Defects {
            answers_excluded_provider_when_nothing_else: true,
            ..Defects::default()
        },
        "exclude_providers_every_provider_no_route",
        "a select excluding every provider [\"example-provider\", \"example-subscription\"] was answered",
    )
    .await;
}

#[tokio::test]
async fn without_exclude_providers_group_its_checks_are_not_applicable() {
    let report = run_suite(&FakeSubject::without_exclude_providers_control(
        Defects::default(),
    ))
    .await;
    assert_eq!(
        report.not_applicable(),
        [
            "placement_consumer_declarations",
            "placement_harness_facts",
            "placement_decision_harness",
            "placement_select_kind",
            "placement_eligible_routes",
            "placement_select_price",
            "placement_attempt_outcomes",
            "undeclared_pin_slot_refused",
            "undeclared_available_routes_refused",
            "undeclared_prefer_prompt_cache_refused",
            "undeclared_settings_refused",
            "undeclared_observe_decision_id_refused",
            "undeclared_observe_retry_at_ms_refused",
            "undeclared_observe_provider_code_refused",
            "account_for_tier_undeclared",
            "warm_credential_undeclared_refused",
            "continue_signed_out_undeclared_refuses_window",
            "continue_undeclared_group_refuses_mode",
            "undeclared_harness_group_refuses_member",
            "undeclared_prefer_group_refuses_member",
            "exclude_providers_answers_another_provider",
            "exclude_providers_every_provider_no_route",
        ],
        "{}",
        report.render()
    );
    assert_eq!(
        report.outcome("undeclared_exclude_providers_group_refuses_member"),
        Some(&CheckOutcome::Passed)
    );
    assert_eq!(report.verdict, SuiteVerdict::Passed, "{}", report.render());
    assert!(report.render().contains(
        "N/A  exclude_providers_answers_another_provider: role.describe does not declare select_exclude_providers"
    ));
}

#[tokio::test]
async fn an_exclude_providers_break_without_the_group_is_not_reached() {
    // The group's checks never run against a provider that does not declare
    // it, so ignoring the member there leaves the verdict a pass.
    let report = run_suite(&FakeSubject::without_exclude_providers_control(Defects {
        ignores_exclude_providers: true,
        ..Defects::default()
    }))
    .await;
    assert_eq!(report.verdict, SuiteVerdict::Passed, "{}", report.render());
}

#[tokio::test]
async fn accepting_undeclared_exclude_providers_member_fails() {
    let report = run_suite(&FakeSubject::without_exclude_providers_control(Defects {
        accepts_undeclared_exclude_providers: true,
        ..Defects::default()
    }))
    .await;
    assert_eq!(
        report.failing(),
        ["undeclared_exclude_providers_group_refuses_member"],
        "{}",
        report.render()
    );
}

#[tokio::test]
async fn task_ref_faithful_provider_passes_both_checks() {
    let report = run_suite(&FakeSubject::new(Defects::default())).await;
    for name in [
        "task_ref_never_changes_selection",
        "invalid_task_ref_refused_by_name",
    ] {
        assert!(CHECKS.iter().any(|check| check.name == name), "{name}");
        assert_eq!(
            report.outcome(name),
            Some(&CheckOutcome::Passed),
            "{}",
            report.render()
        );
    }
}

#[tokio::test]
async fn task_ref_changing_selection_fails_by_name() {
    breaks(
        Defects {
            task_ref_changes_selection: true,
            ..Defects::default()
        },
        "task_ref_never_changes_selection",
        "task_ref changed selection",
    )
    .await;
}

#[tokio::test]
async fn task_ref_accepting_invalid_values_fails_by_name() {
    breaks(
        Defects {
            accepts_invalid_task_ref: true,
            ..Defects::default()
        },
        "invalid_task_ref_refused_by_name",
        "invalid task_ref was answered",
    )
    .await;
}

#[tokio::test]
async fn task_ref_without_group_checks_are_not_applicable_by_name() {
    let report = run_suite(&FakeSubject::without_task_ref_control(Defects::default())).await;
    for name in [
        "task_ref_never_changes_selection",
        "invalid_task_ref_refused_by_name",
    ] {
        match report.outcome(name) {
            Some(CheckOutcome::NotApplicable { reason }) => {
                assert_eq!(reason, "role.describe does not declare select_task_ref");
            }
            other => panic!("{name} should be not applicable, got {other:?}"),
        }
        assert!(report.render().contains(&format!("N/A  {name}:")));
    }
    assert_eq!(report.verdict, SuiteVerdict::Passed, "{}", report.render());
}

const PLACEMENT_CASES: &[&str] = &[
    "placement_consumer_declarations",
    "placement_harness_facts",
    "placement_decision_harness",
    "placement_select_kind",
    "placement_eligible_routes",
    "placement_select_price",
    "placement_attempt_outcomes",
];

#[tokio::test]
async fn decision_harness_accepts_runner_without_programs() {
    let report = run_suite(&FakeSubject::with_placement(Defects::default())).await;
    assert!(
        matches!(
            report.outcome("placement_decision_harness"),
            Some(CheckOutcome::Passed)
        ),
        "{}",
        report.render()
    );
}

#[tokio::test]
async fn decision_harness_rejects_invented_program_by_check_name() {
    let subject = FakeSubject::with_placement(Defects {
        extension: Some("placement_invents_harness"),
        ..Defects::default()
    });
    let report = run_suite(&subject).await;
    assert_eq!(
        report.failing(),
        ["placement_decision_harness", "harness_only_when_declared"],
        "{}",
        report.render()
    );
    match report.outcome("placement_decision_harness") {
        Some(CheckOutcome::Failed { reason }) => {
            assert!(reason.contains("declares no harnesses"), "{reason}");
        }
        other => panic!("placement_decision_harness accepted an invented program: {other:?}"),
    }
}

#[tokio::test]
async fn placement_groups_pass_and_are_not_applicable_without_declaration() {
    for declared in [true, false] {
        let subject = if declared {
            FakeSubject::with_placement(Defects::default())
        } else {
            FakeSubject::new(Defects::default())
        };
        let report = run_suite(&subject).await;
        assert_eq!(report.verdict, SuiteVerdict::Passed, "{}", report.render());
        for name in PLACEMENT_CASES {
            match report.outcome(name) {
                Some(CheckOutcome::Passed) if declared => {}
                Some(CheckOutcome::NotApplicable { reason }) if !declared => {
                    assert!(!reason.is_empty())
                }
                other => panic!(
                    "{name}, declared={declared}: {other:?}\n{}",
                    report.render()
                ),
            }
        }
    }
}

#[tokio::test]
async fn placement_faulty_routers_fail_by_check_name() {
    for name in PLACEMENT_CASES {
        let subject = FakeSubject::with_placement(Defects {
            extension: Some(name),
            ..Defects::default()
        });
        let report = run_suite(&subject).await;
        assert!(
            matches!(report.outcome(name), Some(CheckOutcome::Failed { .. })),
            "{name} accepted an intentionally incorrect router\n{}",
            report.render()
        );
    }
}

#[tokio::test]
async fn placement_consumer_declarations_without_select_kind_passes() {
    let report = run_suite(&FakeSubject::with_placement(Defects {
        extension: Some("placement_without_select_kind"),
        ..Defects::default()
    }))
    .await;
    assert_eq!(report.verdict, SuiteVerdict::Passed, "{}", report.render());
    assert!(matches!(
        report.outcome("placement_consumer_declarations"),
        Some(CheckOutcome::Passed)
    ));
    assert!(matches!(
        report.outcome("placement_select_kind"),
        Some(CheckOutcome::NotApplicable { .. })
    ));
}

#[tokio::test]
async fn placement_caller_runner_and_kind_faults_fail_by_check_name() {
    for fault in [
        "placement_caller_runner_from_principal",
        "placement_caller_runner_guesses",
        "placement_ignores_select_kind_gate",
    ] {
        let report = run_suite(&FakeSubject::with_placement(Defects {
            extension: Some(fault),
            ..Defects::default()
        }))
        .await;
        assert_eq!(
            report.failing(),
            ["placement_consumer_declarations"],
            "{fault}: {}",
            report.render()
        );
    }
}
