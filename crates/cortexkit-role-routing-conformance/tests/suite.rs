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
    let report = run_suite(&FakeSubject::new(defects)).await;
    assert_eq!(report.failing(), [red], "{}", report.render());
    match report.outcome(red) {
        Some(CheckOutcome::Failed { reason }) => {
            assert!(reason.contains(because), "{red}: {reason}")
        }
        other => panic!("{red} should fail, got {other:?}"),
    }
    assert!(matches!(report.verdict, SuiteVerdict::Failed { .. }));
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
    breaks(
        Defects {
            drops_reset_at_ms: true,
            ..Defects::default()
        },
        "timed_refusals_carry_reset_at_ms",
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
