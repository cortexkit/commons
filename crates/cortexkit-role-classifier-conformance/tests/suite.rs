//! Tests of the suite itself, against the reference fake in `fake/`.
//! Conformance of a real module is run by that module, never here.
//!
//! Each deliberate break in the fake must turn its check red by name. Each
//! test also pins exactly which checks a break turns red, so a check that
//! starts passing a break, or a break that starts leaking into unrelated
//! checks, shows up.

mod fake;

use cortexkit_role_classifier_conformance::{
    run_suite, CheckOutcome, SuiteReport, SuiteVerdict, CHECKS,
};
use fake::{Defects, FakeSubject};

async fn run(subject: &FakeSubject) -> SuiteReport {
    run_suite(subject).await
}

fn failing(report: &SuiteReport) -> Vec<&'static str> {
    report
        .checks
        .iter()
        .filter(|check| matches!(check.outcome, CheckOutcome::Failed { .. }))
        .map(|check| check.check)
        .collect()
}

/// Run the fake with `defects`, and check that exactly `red` fail, the
/// first of them for a reason containing `because`.
async fn breaks(defects: Defects, red: &[&str], because: &str) {
    let report = run(&FakeSubject::new(defects)).await;
    assert_eq!(failing(&report), red, "{}", report.render());
    match report.outcome(red[0]) {
        Some(CheckOutcome::Failed { reason }) => {
            assert!(reason.contains(because), "{}: {reason}", red[0])
        }
        other => panic!("{} should fail, got {other:?}", red[0]),
    }
    assert!(matches!(report.verdict, SuiteVerdict::Failed { .. }));
}

#[tokio::test]
async fn a_faithful_module_passes_every_check() {
    let report = run(&FakeSubject::new(Defects::default())).await;
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
}

#[tokio::test]
async fn an_unpriced_module_gets_not_applicable_never_passed() {
    let report = run(&FakeSubject::unpriced()).await;
    let not_applicable = [
        "cost_usd_stable_on_replay",
        "looser_max_cost_does_not_raise_ceiling",
        "tighter_max_cost_stops_crossing_items",
    ];
    for name in not_applicable {
        assert!(
            matches!(
                report.outcome(name),
                Some(CheckOutcome::NotApplicable { .. })
            ),
            "{name}\n{}",
            report.render()
        );
    }
    assert_eq!(
        report.verdict,
        SuiteVerdict::PassedExceptNotApplicable {
            not_applicable: not_applicable.to_vec()
        },
        "{}",
        report.render()
    );
    assert!(report.render().contains("N/A  cost_usd_stable_on_replay"));
}

#[tokio::test]
async fn a_stand_in_that_cannot_hold_makes_the_concurrent_check_not_applicable() {
    let report = run(&FakeSubject::without_hold()).await;
    assert!(matches!(
        report.outcome("concurrent_call_refused_batch_in_progress"),
        Some(CheckOutcome::NotApplicable { .. })
    ));
    assert_eq!(
        report.verdict,
        SuiteVerdict::PassedExceptNotApplicable {
            not_applicable: vec!["concurrent_call_refused_batch_in_progress"]
        }
    );
}

#[tokio::test]
async fn wrong_context_tokens_fail_describe_complete() {
    breaks(
        Defects {
            describe_wrong_context_tokens: true,
            ..Defects::default()
        },
        &["describe_complete"],
        "context_tokens 1, the catalog says 8192",
    )
    .await;
}

#[tokio::test]
async fn an_omitted_model_fails_describe_complete() {
    breaks(
        Defects {
            describe_omits_model: true,
            ..Defects::default()
        },
        &["describe_complete"],
        "the catalog serves",
    )
    .await;
}

#[tokio::test]
async fn no_single_flight_fails_the_concurrent_check() {
    breaks(
        Defects {
            no_single_flight: true,
            ..Defects::default()
        },
        &["concurrent_call_refused_batch_in_progress"],
        "expected a refusal",
    )
    .await;
}

#[tokio::test]
async fn waiting_on_the_first_call_fails_the_concurrent_check_without_hanging() {
    breaks(
        Defects {
            wait_for_in_flight: true,
            ..Defects::default()
        },
        &["concurrent_call_refused_batch_in_progress"],
        "it waited instead of being refused batch_in_progress",
    )
    .await;
}

#[tokio::test]
async fn reasking_answered_items_fails_the_retry_check() {
    breaks(
        Defects {
            reask_answered_on_retry: true,
            ..Defects::default()
        },
        &[
            "concurrent_call_refused_batch_in_progress",
            "retry_calls_only_unanswered_items",
            "cost_usd_stable_on_replay",
            "reuse_refused_naming_field",
            "reordered_object_state_is_replay",
        ],
        "",
    )
    .await;
    let report = run(&FakeSubject::new(Defects {
        reask_answered_on_retry: true,
        ..Defects::default()
    }))
    .await;
    match report.outcome("retry_calls_only_unanswered_items") {
        Some(CheckOutcome::Failed { reason }) => assert!(
            reason.contains("item 0, answered before the retry"),
            "{reason}"
        ),
        other => panic!("{other:?}"),
    }
}

#[tokio::test]
async fn retrying_stored_permanent_errors_fails_the_stored_error_check() {
    breaks(
        Defects {
            retry_stored_permanent: true,
            ..Defects::default()
        },
        &["stored_permanent_error_replayed_transient_retried"],
        "the stored permanent error was not returned as stored",
    )
    .await;
}

#[tokio::test]
async fn replaying_stored_transient_errors_fails_the_stored_error_check() {
    breaks(
        Defects {
            replay_stored_transient: true,
            ..Defects::default()
        },
        &[
            "retry_calls_only_unanswered_items",
            "stored_permanent_error_replayed_transient_retried",
            "looser_max_cost_does_not_raise_ceiling",
            "tighter_max_cost_stops_crossing_items",
        ],
        "",
    )
    .await;
    let report = run(&FakeSubject::new(Defects {
        replay_stored_transient: true,
        ..Defects::default()
    }))
    .await;
    match report.outcome("stored_permanent_error_replayed_transient_retried") {
        Some(CheckOutcome::Failed { reason }) => assert!(
            reason.contains("the stored transient error was not retried"),
            "{reason}"
        ),
        other => panic!("{other:?}"),
    }
}

#[tokio::test]
async fn a_per_call_cost_fails_the_cost_check() {
    breaks(
        Defects {
            cost_per_call: true,
            ..Defects::default()
        },
        &["cost_usd_stable_on_replay"],
        "on the first reply and",
    )
    .await;
}

#[tokio::test]
async fn a_ceiling_from_the_latest_call_fails_the_looser_check() {
    breaks(
        Defects {
            ceiling_from_latest_call: true,
            ..Defects::default()
        },
        &["looser_max_cost_does_not_raise_ceiling"],
        "raised by the looser value",
    )
    .await;
}

#[tokio::test]
async fn a_ceiling_from_the_first_call_fails_the_tighter_check() {
    breaks(
        Defects {
            ceiling_from_first_call_only: true,
            ..Defects::default()
        },
        &["tighter_max_cost_stops_crossing_items"],
        "not tightened by the lower value",
    )
    .await;
}

#[tokio::test]
async fn accepting_reuse_fails_the_reuse_check() {
    breaks(
        Defects {
            reuse_accepted: true,
            ..Defects::default()
        },
        &["reuse_refused_naming_field"],
        "expected a refusal",
    )
    .await;
}

#[tokio::test]
async fn hashing_raw_item_text_fails_the_reordered_state_check() {
    breaks(
        Defects {
            identity_from_raw_text: true,
            ..Defects::default()
        },
        &[
            "reuse_refused_naming_field",
            "reordered_object_state_is_replay",
        ],
        "",
    )
    .await;
    let report = run(&FakeSubject::new(Defects {
        identity_from_raw_text: true,
        ..Defects::default()
    }))
    .await;
    match report.outcome("reordered_object_state_is_replay") {
        Some(CheckOutcome::Failed { reason }) => {
            assert!(
                reason.contains("key order changed the body identity"),
                "{reason}"
            )
        }
        other => panic!("{other:?}"),
    }
}

#[tokio::test]
async fn validating_after_dispatch_fails_the_validation_check() {
    breaks(
        Defects {
            score_levels_checked_after_dispatch: true,
            ..Defects::default()
        },
        &["validation_refusals_reach_no_provider"],
        "a score with eleven levels: the stand-in was called 1 times",
    )
    .await;
}

#[tokio::test]
async fn forwarding_a_null_state_fails_the_null_state_check() {
    breaks(
        Defects {
            null_state_forwarded: true,
            ..Defects::default()
        },
        &["null_state_refused_naming_field"],
        "expected a refusal",
    )
    .await;
}

#[tokio::test]
async fn items_out_of_request_order_fail_the_order_check() {
    breaks(
        Defects {
            items_sorted_by_state: true,
            ..Defects::default()
        },
        &["answers_in_request_order"],
        "position 0 holds the answer",
    )
    .await;
}

#[tokio::test]
async fn numbers_through_f64_fail_the_numbers_check() {
    breaks(
        Defects {
            numbers_through_f64: true,
            ..Defects::default()
        },
        &["provider_numbers_byte_for_byte"],
        "0.1234567890123456789 became 0.12345678901234568",
    )
    .await;
}

#[tokio::test]
async fn failing_the_whole_batch_fails_the_one_failing_item_check() {
    breaks(
        Defects {
            fail_whole_batch_on_4xx: true,
            ..Defects::default()
        },
        &[
            "stored_permanent_error_replayed_transient_retried",
            "one_failing_item_does_not_fail_batch",
        ],
        "",
    )
    .await;
    let report = run(&FakeSubject::new(Defects {
        fail_whole_batch_on_4xx: true,
        ..Defects::default()
    }))
    .await;
    match report.outcome("one_failing_item_does_not_fail_batch") {
        Some(CheckOutcome::Failed { reason }) => {
            assert!(reason.contains("refused provider_error"), "{reason}")
        }
        other => panic!("{other:?}"),
    }
}

#[tokio::test]
async fn zero_filled_usage_fails_the_usage_check() {
    breaks(
        Defects {
            usage_zero_filled: true,
            ..Defects::default()
        },
        &["unreported_usage_stays_absent"],
        "reported no usage, but its usage reads",
    )
    .await;
}
