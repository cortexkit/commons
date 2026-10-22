//! Tests of the suite itself, against the reference fake in `fake/`.
//! Conformance of a real module is run by that module, never here.
//!
//! Each deliberate break in the fake must turn its check red by name, and
//! only that check: each test pins exactly which checks a break turns red.

mod fake;

use cortexkit_role_extractor_conformance::{
    run_suite, CheckOutcome, SuiteReport, SuiteVerdict, CHECKS,
};
use fake::{Defects, FakeSubject};

fn failing(report: &SuiteReport) -> Vec<&'static str> {
    report
        .checks
        .iter()
        .filter(|check| matches!(check.outcome, CheckOutcome::Failed { .. }))
        .map(|check| check.check)
        .collect()
}

/// Run the fake with `defects`, and check that exactly `red` fails, for a
/// reason containing `because`.
async fn breaks(defects: Defects, red: &str, because: &str) {
    let report = run_suite(&FakeSubject::new(defects)).await;
    assert_eq!(failing(&report), [red], "{}", report.render());
    match report.outcome(red) {
        Some(CheckOutcome::Failed { reason }) => {
            assert!(reason.contains(because), "{red}: {reason}")
        }
        other => panic!("{red} should fail, got {other:?}"),
    }
    assert!(matches!(report.verdict, SuiteVerdict::Failed { .. }));
}

#[tokio::test]
async fn a_faithful_module_passes_every_check() {
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
}

macro_rules! break_test {
    ($test:ident, $defect:ident, $check:literal, $because:literal) => {
        #[tokio::test]
        async fn $test() {
            breaks(
                Defects {
                    $defect: true,
                    ..Defects::default()
                },
                $check,
                $because,
            )
            .await;
        }
    };
}

break_test!(
    wrong_pass_tokens_fail_describe,
    describe_wrong_pass_tokens,
    "describe_complete",
    "pass_tokens 1"
);
break_test!(
    an_unchecked_batch_id_fails,
    batch_id_unchecked,
    "bad_batch_id_refused",
    "expected a refusal"
);
break_test!(
    serving_an_unknown_model_fails,
    unknown_model_served,
    "unknown_model_refused",
    "expected a refusal"
);
break_test!(
    dropping_the_kind_fails,
    non_extraction_kind_dropped,
    "non_extraction_model_refused",
    "leaves detail.kind out"
);
break_test!(
    a_kindless_row_called_unknown_fails,
    kindless_row_called_unknown,
    "kindless_model_refused",
    "expected model_not_extractor"
);
break_test!(
    unchecked_field_ids_fail,
    field_ids_unchecked,
    "field_id_refused_naming_field",
    "expected a refusal"
);
break_test!(
    options_before_type_fail,
    options_checked_before_type,
    "unknown_field_type_refused_before_options",
    "expected fields.total.type"
);
break_test!(
    dropped_options_fail,
    unknown_options_dropped,
    "misspelt_or_misplaced_option_refused",
    "expected a refusal"
);
break_test!(
    optional_required_options_fail,
    required_options_optional,
    "missing_required_option_refused",
    "expected a refusal"
);
break_test!(
    unchecked_option_values_fail,
    option_values_unchecked,
    "option_value_out_of_range_refused",
    "expected a refusal"
);
break_test!(
    unchecked_instructions_fail,
    instructions_unchecked,
    "instructions_refused_empty_or_over_bound",
    "expected a refusal"
);
break_test!(
    an_unchecked_reference_date_fails,
    reference_date_unchecked,
    "item_state_and_reference_date_refused",
    "expected a refusal"
);
break_test!(
    dropped_members_fail,
    unknown_members_dropped,
    "unknown_member_refused",
    "expected a refusal"
);
break_test!(
    unchecked_counts_fail,
    counts_unchecked,
    "counts_refused_with_limit",
    "expected a refusal"
);
break_test!(
    unchecked_images_fail,
    images_unchecked,
    "images_refused_before_any_call",
    "expected a refusal"
);
break_test!(
    an_unchecked_request_size_fails,
    request_size_unchecked,
    "oversize_request_refused",
    "expected a refusal"
);
break_test!(
    an_unchecked_reply_size_fails,
    reply_size_unchecked,
    "reply_over_frame_refused",
    "expected a refusal"
);
break_test!(
    unforwarded_options_fail,
    options_not_forwarded,
    "clairic_request_mapped",
    "the stand-in received"
);
break_test!(
    accepted_reuse_fails,
    reuse_accepted,
    "reuse_refused_naming_part",
    "expected a refusal"
);
break_test!(
    images_hashed_as_written_fail,
    images_hashed_as_written,
    "image_spellings_are_one_batch",
    "batch_id_reuse"
);
break_test!(
    an_ignored_ceiling_fails,
    ceiling_ignored,
    "zero_ceiling_sends_nothing",
    "expected cost_exceeded"
);
break_test!(
    dropped_late_types_fail,
    late_types_dropped,
    "provider_answers_forwarded",
    "is missing"
);
break_test!(
    spans_converted_to_bytes_fail,
    spans_converted_to_bytes,
    "spans_forwarded_unconverted",
    "expected [53,61]"
);
break_test!(
    a_refused_empty_result_fails,
    empty_result_refused,
    "provider_errors_mapped",
    "expected invalid_item"
);
break_test!(
    cost_from_the_estimate_fails,
    cost_from_estimate,
    "cost_usd_from_reported_tokens",
    "the reported input_tokens 304"
);

#[test]
fn every_check_has_a_break() {
    let source = include_str!("suite.rs");
    for check in CHECKS {
        assert!(
            source.contains(&format!("\"{}\"", check.name)),
            "{} has no break test",
            check.name
        );
    }
}
