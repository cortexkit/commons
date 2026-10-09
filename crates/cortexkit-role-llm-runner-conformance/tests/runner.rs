//! Tests of the suite itself, against the reference fake in `fake/`.
//! Conformance of a real runner is run by that runner, never here.

mod fake;

use cortexkit_role_llm_runner::send::{delivered_as, Delivered};
use cortexkit_role_llm_runner_conformance::{
    harness::KillMechanism, run_suite, Capability, CaseOutcome, SetupError, SuiteReport,
    SuiteVerdict, CASES,
};
use fake::{CompactionDefect, Defects, FakeSubject, RetentionDefect, StepRecovery};
use std::time::{Duration, Instant};

async fn run(subject: &FakeSubject) -> SuiteReport {
    let dir = tempfile::tempdir().unwrap();
    run_suite(subject, dir.path()).await.unwrap()
}

fn failed(report: &SuiteReport, case: &str) -> String {
    match report.outcome(case) {
        Some(CaseOutcome::Failed { reason }) => reason.clone(),
        other => panic!("{case} should fail, got {other:?}\n{}", report.render()),
    }
}

fn assert_passed(report: &SuiteReport, case: &str) {
    // Legacy fake configurations intentionally do not declare retention.
    // Assert the named skip, never count it as a passed retention case.
    if CASES
        .iter()
        .find(|s| s.name == case)
        .unwrap()
        .requires
        .contains(&Capability::Retention)
        && report.outcome(case)
            == Some(&CaseOutcome::Skipped {
                missing: vec![Capability::Retention],
            })
    {
        return;
    }
    assert_eq!(
        report.outcome(case),
        Some(&CaseOutcome::Passed),
        "{case}\n{}",
        report.render()
    );
}

/// Every case not in `failing` passed.
fn assert_others_passed(report: &SuiteReport, failing: &[&str]) {
    for spec in CASES {
        if !failing.contains(&spec.name) {
            assert_passed(report, spec.name);
        }
    }
}

/// The halves of the `send_id` cases that check only what `session.send`
/// answers.
const REPLY_HALVES: &[&str] = &[
    "send_id_retry_same_answer",
    "send_id_retry_settled_same_answer",
    "send_id_reuse_refused_naming_field",
    "delivery_change_refused",
];

/// The cases that retry a send and check the replies: the unsettled one and
/// the one that waits for the run to end first.
const RETRY_REPLY_CASES: &[&str] = &[
    "send_id_retry_same_answer",
    "send_id_retry_settled_same_answer",
];

/// The halves of the `send_id` cases that read what the send wrote.
const WRITTEN_ONCE_HALVES: &[&str] = &[
    "send_id_retry_written_once",
    "send_id_reuse_writes_nothing",
    "delivery_change_writes_nothing",
];

#[tokio::test]
async fn a_faithful_runner_passes_every_case_but_its_simulated_kills_fail_the_run() {
    let report = run(&FakeSubject::new(Defects::default())).await;
    assert_eq!(report.cases.len(), CASES.len());
    for spec in CASES {
        assert_passed(&report, spec.name);
    }
    assert_eq!(report.kills.len(), 8, "{}", report.render());
    assert!(report
        .kills
        .iter()
        .all(|kill| kill.mechanism == KillMechanism::FaultHook));
    match &report.verdict {
        SuiteVerdict::Failed { reasons } => {
            assert_eq!(reasons.len(), 1, "{reasons:?}");
            assert!(reasons[0].contains("no kill in this run ended a real process"));
        }
        other => panic!("expected the real-kill rule to fail the run, got {other:?}"),
    }
}

const TIMED_RETENTION_CASES: &[&str] = &[
    "retention_shorten_continues_lineage",
    "retention_equal_noop",
    "retention_honoured",
    "retention_run_status_expired",
    "retention_no_content_served",
    "retention_active_run_never_expires",
    "crash_at_RetentionTombstoned",
];

#[tokio::test(start_paused = true)]
async fn a_faithful_runner_whose_kills_end_a_process_passes() {
    let mut subject = FakeSubject::new(Defects::default());
    subject.claim_process_kill = true;
    subject.enable_retention();
    let started = Instant::now();
    let report = run(&subject).await;
    assert_eq!(report.verdict, SuiteVerdict::Passed, "{}", report.render());
    for name in TIMED_RETENTION_CASES {
        assert_eq!(
            report.outcome(name),
            Some(&CaseOutcome::Passed),
            "{name}\n{}",
            report.render()
        );
    }
    eprintln!(
        "{} time-based retention cases passed in {:?}",
        TIMED_RETENTION_CASES.len(),
        started.elapsed()
    );
}

#[tokio::test(start_paused = true)]
async fn without_a_retention_clock_long_cases_skip_by_name_instead_of_waiting() {
    let mut subject = FakeSubject::new(Defects::default());
    subject.enable_retention();
    subject.claim_process_kill = true;
    subject.capabilities.remove(&Capability::RetentionClock);
    subject.retention_delete_ms = 30_000;

    let report = tokio::time::timeout(Duration::from_secs(1), run(&subject))
        .await
        .expect("a missing clock must not make long retention cases wait");
    for name in TIMED_RETENTION_CASES {
        match report.outcome(name) {
            Some(CaseOutcome::SkippedWithReason { missing, reason }) => {
                assert_eq!(missing, &[Capability::RetentionClock], "{name}: {reason}");
                assert!(
                    reason.contains("needs advance_retention_clock"),
                    "{name}: {reason}"
                );
                assert!(reason.contains("real wait would be"), "{name}: {reason}");
                eprintln!("{name}: {reason}");
                assert!(report.render().contains(&format!("SKIP {name}")));
            }
            outcome => panic!(
                "{name} should be skipped with its wait bound, got {outcome:?}\n{}",
                report.render()
            ),
        }
    }
    assert_passed(&report, "retention_zero_refused");
}

#[tokio::test]
async fn a_tail_that_answers_the_oldest_page_fails_the_tail_case() {
    let report = run(&FakeSubject::new(Defects {
        tail_returns_oldest: true,
        ..Defects::default()
    }))
    .await;
    let reason = failed(&report, "tail_read_is_newest_page");
    assert!(reason.contains("last_ordinal"), "{reason}");
    assert_passed(&report, "range_read_stops_by_count");
}

#[tokio::test]
async fn a_call_key_reused_for_a_recurring_model_id_fails_the_recurring_case() {
    let report = run(&FakeSubject::new(Defects {
        call_key_from_model_id: true,
        ..Defects::default()
    }))
    .await;
    let reason = failed(&report, "recurring_model_id_distinct_call_keys");
    assert!(reason.contains("share call_key"), "{reason}");
    assert_passed(&report, "sibling_calls_distinct_call_keys");
}

#[tokio::test]
async fn a_crash_cut_sealed_cancelled_fails_the_dispatch_intent_and_interrupted_cases() {
    let report = run(&FakeSubject::new(Defects {
        cancel_cut_runs: true,
        ..Defects::default()
    }))
    .await;
    let reason = failed(&report, "crash_at_DispatchIntent");
    assert!(reason.contains("cancelled"), "{reason}");
    let reason = failed(&report, "run_result_interrupted_not_cancelled");
    assert!(reason.contains("cancelled"), "{reason}");
    assert_passed(&report, "crash_at_Terminal");
}

#[tokio::test]
async fn step_recorded_resume_passes() {
    let report = run(&FakeSubject::new(Defects::default())).await;
    assert_passed(&report, "crash_at_StepRecorded");
}

#[tokio::test]
async fn step_recorded_seal_interrupted_passes() {
    let mut subject = FakeSubject::new(Defects::default());
    subject.step_recovery = StepRecovery::SealInterrupted;
    let report = run(&subject).await;
    for spec in CASES {
        assert_passed(&report, spec.name);
    }
}

#[tokio::test]
async fn step_recorded_seal_drop_step_passes() {
    let mut subject = FakeSubject::new(Defects::default());
    subject.step_recovery = StepRecovery::SealDropStep;
    let report = run(&subject).await;
    assert_passed(&report, "crash_at_StepRecorded");
}

#[tokio::test]
async fn step_recorded_seal_drop_call_keep_text_passes() {
    let mut subject = FakeSubject::new(Defects::default());
    subject.step_recovery = StepRecovery::SealDropCall;
    let report = run(&subject).await;
    assert_passed(&report, "crash_at_StepRecorded");
}

#[tokio::test]
async fn step_recorded_sealed_unattributed_dangling_call_fails() {
    let mut subject = FakeSubject::new(Defects {
        sealed_call_without_result: true,
        sealed_call_without_attribution: true,
        ..Defects::default()
    });
    subject.step_recovery = StepRecovery::SealInterrupted;
    let report = run(&subject).await;
    let reason = failed(&report, "crash_at_StepRecorded");
    assert!(reason.contains("sealed branch:"), "{reason}");
    assert!(
        reason.contains("remains in attribution or message history"),
        "{reason}"
    );
    assert_passed(&report, "crash_at_DispatchIntent");
}

#[tokio::test]
async fn step_recorded_sealed_indeterminate_fails() {
    let mut subject = FakeSubject::new(Defects {
        sealed_call_indeterminate: true,
        ..Defects::default()
    });
    subject.step_recovery = StepRecovery::SealInterrupted;
    let report = run(&subject).await;
    let reason = failed(&report, "crash_at_StepRecorded");
    assert!(reason.contains("sealed branch"), "{reason}");
    assert!(reason.contains("indeterminate: false"), "{reason}");
    assert_passed(&report, "crash_at_DispatchIntent");
}

#[tokio::test]
async fn step_recorded_sealed_dangling_call_fails() {
    let mut subject = FakeSubject::new(Defects {
        sealed_call_without_result: true,
        ..Defects::default()
    });
    subject.step_recovery = StepRecovery::SealInterrupted;
    let report = run(&subject).await;
    let reason = failed(&report, "crash_at_StepRecorded");
    assert!(reason.contains("sealed branch:"), "{reason}");
    assert!(reason.contains("dangling call call_b"), "{reason}");
    assert!(reason.contains("expected one result before"), "{reason}");
    assert_passed(&report, "crash_at_DispatchIntent");
}

#[tokio::test]
async fn step_recorded_dispatch_twice_fails() {
    let report = run(&FakeSubject::new(Defects {
        dispatch_twice_on_resume: true,
        ..Defects::default()
    }))
    .await;
    let reason = failed(&report, "crash_at_StepRecorded");
    assert!(reason.contains("resume branch"), "{reason}");
    assert!(reason.contains("at-most-once dispatch"), "{reason}");
    assert!(reason.contains("ran 2 times"), "{reason}");
    assert_passed(&report, "crash_at_DispatchIntent");
}

#[tokio::test]
async fn a_group_claimed_on_the_wire_but_skipped_by_the_suite_fails_the_groups_case() {
    let report = run(&FakeSubject::new(Defects {
        claim_unserved_group: true,
        ..Defects::default()
    }))
    .await;
    let reason = failed(&report, "role_describe_groups_complete");
    assert!(
        reason.contains("declares interrupt, which the subject does not"),
        "{reason}"
    );
}

#[tokio::test]
async fn a_subject_declaring_nothing_is_skipped_everywhere_and_never_passes() {
    let mut subject = FakeSubject::new(Defects::default());
    subject.capabilities.clear();
    subject.kill_points.clear();
    let report = run(&subject).await;
    for spec in CASES {
        if spec.requires.is_empty() {
            continue;
        }
        assert!(
            matches!(report.outcome(spec.name), Some(CaseOutcome::Skipped { missing }) if !missing.is_empty()),
            "{}: {:?}",
            spec.name,
            report.outcome(spec.name)
        );
    }
    assert!(report.kills.is_empty());
    match &report.verdict {
        SuiteVerdict::Failed { reasons } => {
            assert_eq!(reasons.len(), 1, "{reasons:?}");
            assert!(reasons[0].contains("no kill was made"), "{reasons:?}");
        }
        other => panic!("a run that declares nothing must not pass, got {other:?}"),
    }
}

#[tokio::test]
async fn undeclared_capabilities_make_the_run_conforming_only_for_the_declared_ones() {
    let mut subject = FakeSubject::new(Defects::default());
    subject.claim_process_kill = true;
    subject.capabilities.remove(&Capability::Streaming);
    subject.capabilities.remove(&Capability::HoldToolCalls);
    let report = run(&subject).await;
    for case in [
        "subscribe_from_head_no_gap_no_duplicate",
        "indeterminate_until_closed",
    ] {
        assert!(
            matches!(report.outcome(case), Some(CaseOutcome::Skipped { .. })),
            "{case}: {:?}",
            report.outcome(case)
        );
    }
    match &report.verdict {
        SuiteVerdict::ConformingForDeclaredCapabilities { declared, skipped } => {
            assert_eq!(
                skipped,
                &vec![
                    Capability::Streaming,
                    Capability::Retention,
                    Capability::HoldToolCalls
                ]
            );
            assert!(declared.contains(&Capability::TranscriptReads));
            assert!(!declared.contains(&Capability::Streaming));
        }
        other => panic!("expected conforming for declared capabilities, got {other:?}"),
    }
}

#[tokio::test]
async fn a_runner_declaring_every_delivery_mode_cannot_be_asked_for_an_undeclared_one() {
    let mut subject = FakeSubject::new(Defects::default());
    subject.capabilities.insert(Capability::Interrupt);
    let report = run(&subject).await;
    assert!(
        matches!(
            report.outcome("undeclared_delivery_refused"),
            Some(CaseOutcome::Inapplicable { .. })
        ),
        "{}",
        report.render()
    );
}

#[tokio::test]
async fn a_guaranteed_runner_answering_pending_fails() {
    let report = run(&FakeSubject::new(Defects {
        guaranteed_steer_pending: true,
        ..Defects::default()
    }))
    .await;
    let reason = failed(&report, "guaranteed_steer_never_pending_or_unknown");
    assert!(reason.contains("pending"), "{reason}");
    assert_passed(&report, "resend_steer_delivered_stable");
}

/// Run the suite against a fake with one held-turn steer defect, prove that
/// the defect produced exactly one invalid answer, and require that it fails the
/// guaranteed-steer check and nothing else.
async fn held_steer_break(defects: Defects, because: &str) {
    let subject = FakeSubject::new(defects);
    let report = run(&subject).await;
    assert_eq!(
        subject.held_steer_break_answers(),
        1,
        "the held-turn break did not apply exactly once\n{}",
        report.render()
    );
    let case = "guaranteed_steer_never_pending_or_unknown";
    let reason = failed(&report, case);
    assert!(reason.contains(because), "{reason}");
    assert_others_passed(&report, &[case]);
    eprintln!("held-turn break applied once; only {case} failed: {reason}");
}

#[tokio::test]
async fn a_guaranteed_runner_answering_pending_into_a_held_turn_fails_only_the_guaranteed_case() {
    held_steer_break(
        Defects {
            held_steer_pending: true,
            ..Defects::default()
        },
        "pending on session.send steer",
    )
    .await;
}

#[tokio::test]
async fn a_guaranteed_runner_answering_pending_on_a_held_turn_resend_fails_only_the_guaranteed_case(
) {
    held_steer_break(
        Defects {
            held_steer_retry_pending: true,
            ..Defects::default()
        },
        "pending on session.send steer re-send",
    )
    .await;
}

#[tokio::test]
async fn a_changed_held_turn_resend_receipt_fails_only_the_guaranteed_case() {
    held_steer_break(
        Defects {
            held_steer_retry_unstable: true,
            ..Defects::default()
        },
        "re-send delivered changed",
    )
    .await;
}

#[tokio::test]
async fn a_steer_answered_after_the_held_turn_ends_fails_only_the_guaranteed_case() {
    held_steer_break(
        Defects {
            held_steer_ends_run: true,
            ..Defects::default()
        },
        "without the held run",
    )
    .await;
}

#[tokio::test]
async fn an_absent_final_held_turn_receipt_fails_only_the_guaranteed_case() {
    held_steer_break(
        Defects {
            held_steer_final_absent: true,
            ..Defects::default()
        },
        "omitted delivered on final re-send after the run ended",
    )
    .await;
}

#[tokio::test]
async fn a_held_turn_receipt_changed_after_release_fails_only_the_guaranteed_case() {
    held_steer_break(
        Defects {
            held_steer_after_release_unstable: true,
            ..Defects::default()
        },
        "final re-send delivered changed",
    )
    .await;
}

#[tokio::test]
async fn a_guaranteed_runner_with_an_absent_first_steer_receipt_passes() {
    let mut subject = FakeSubject::new(Defects::default());
    subject.omit_first_steer_receipt = true;
    let report = run(&subject).await;
    assert_others_passed(&report, &[]);
}

#[tokio::test]
async fn a_guaranteed_runner_defers_held_turn_receipts_until_release() {
    let subject = FakeSubject::new(Defects::default());
    let report = run(&subject).await;
    assert_others_passed(&report, &[]);
    let receipts = subject.held_steer_receipts();
    assert_eq!(receipts.len(), 3);
    assert_eq!(&receipts[..2], &[None, None]);
    assert!(receipts[2].as_ref().unwrap().is_delivered());
}

#[tokio::test]
async fn a_guaranteed_runner_preserves_optional_early_receipts_and_opaque_step_refs() {
    for receipt in [
        Delivered::new(delivered_as::STEP),
        Delivered::new(delivered_as::STEP).with_ref("opaque-stored-row-not-a-mid"),
        Delivered::new(delivered_as::TURN),
        Delivered::new(delivered_as::TURN).with_ref("run-1"),
    ] {
        let mut subject = FakeSubject::new(Defects::default());
        subject.early_held_steer_receipt = Some(receipt.clone());
        let report = run(&subject).await;
        assert_others_passed(&report, &[]);
        assert_eq!(subject.held_steer_receipts(), vec![Some(receipt); 3]);
    }
}

#[tokio::test]
async fn a_confirm_runner_reports_the_guaranteed_case_as_inapplicable() {
    let mut subject = FakeSubject::new(Defects::default());
    subject.steer_receipt_confirm = true;
    // With no way to hold a turn either, the case must still be inapplicable
    // because the runner declares confirmation receipts, never passed by a
    // steer into an idle session.
    subject.capabilities.remove(&Capability::HoldToolCalls);
    let report = run(&subject).await;
    assert!(matches!(
        report.outcome("guaranteed_steer_never_pending_or_unknown"),
        Some(CaseOutcome::Inapplicable { reason }) if reason.contains("steer_receipt: confirm")
    ));
}

#[tokio::test]
async fn without_held_calls_the_guaranteed_case_is_inapplicable_never_an_idle_pass() {
    let mut subject = FakeSubject::new(Defects::default());
    subject.capabilities.remove(&Capability::HoldToolCalls);
    let report = run(&subject).await;
    assert!(matches!(
        report.outcome("guaranteed_steer_never_pending_or_unknown"),
        Some(CaseOutcome::Inapplicable { reason }) if reason.contains("hold_tool_calls")
    ));
}

#[tokio::test]
async fn an_unstable_resend_steer_receipt_fails() {
    let report = run(&FakeSubject::new(Defects {
        resend_steer_unstable: true,
        ..Defects::default()
    }))
    .await;
    let reason = failed(&report, "resend_steer_delivered_stable");
    assert!(reason.contains("re-send delivered changed"), "{reason}");
    assert_passed(&report, "guaranteed_steer_never_pending_or_unknown");
}

#[tokio::test]
async fn a_runner_without_steer_reports_steer_checks_as_inapplicable() {
    let mut subject = FakeSubject::new(Defects::default());
    subject.capabilities.remove(&Capability::Steer);
    let report = run(&subject).await;
    for case in [
        "guaranteed_steer_never_pending_or_unknown",
        "resend_steer_delivered_stable",
    ] {
        assert!(
            matches!(
                report.outcome(case),
                Some(CaseOutcome::Inapplicable { reason }) if reason.contains("does not declare steer")
            ),
            "{case}: {:?}",
            report.outcome(case)
        );
    }
}

#[tokio::test]
async fn without_transcript_reads_the_reply_halves_run_and_the_written_once_halves_are_inapplicable(
) {
    let mut subject = FakeSubject::new(Defects::default());
    subject.capabilities.remove(&Capability::TranscriptReads);
    let report = run(&subject).await;
    for case in REPLY_HALVES {
        assert_passed(&report, case);
    }
    for case in WRITTEN_ONCE_HALVES {
        assert!(
            matches!(
                report.outcome(case),
                Some(CaseOutcome::Inapplicable { reason }) if reason.contains("transcript_reads")
            ),
            "{case}: {:?}\n{}",
            report.outcome(case),
            report.render()
        );
    }
    // The fake refuses session.read and session.head here, as a runner that
    // does not declare transcript_reads would; none should have been asked.
    assert_eq!(subject.transcript_calls(), 0, "{}", report.render());
}

#[tokio::test]
async fn a_retry_naming_another_submission_fails_the_retry_reply_half() {
    let report = run(&FakeSubject::new(Defects {
        retry_new_submission_id: true,
        ..Defects::default()
    }))
    .await;
    let reason = failed(&report, "send_id_retry_same_answer");
    assert!(reason.contains("submission_id"), "{reason}");
    let reason = failed(&report, "send_id_retry_settled_same_answer");
    assert!(reason.contains("submission_id"), "{reason}");
    assert_others_passed(&report, RETRY_REPLY_CASES);
}

#[tokio::test]
async fn a_retry_moving_delivered_back_from_step_to_pending_fails_the_retry_reply_half() {
    let report = run(&FakeSubject::new(Defects {
        retry_delivered_step_then_pending: true,
        ..Defects::default()
    }))
    .await;
    for case in RETRY_REPLY_CASES {
        let reason = failed(&report, case);
        assert!(
            reason.contains("delivered changed from step to pending"),
            "{case}: {reason}"
        );
    }
    assert_others_passed(&report, RETRY_REPLY_CASES);
}

#[tokio::test]
async fn a_retry_changing_unknown_to_step_fails_the_retry_reply_half() {
    let report = run(&FakeSubject::new(Defects {
        retry_delivered_unknown_then_step: true,
        ..Defects::default()
    }))
    .await;
    for case in RETRY_REPLY_CASES {
        let reason = failed(&report, case);
        assert!(
            reason.contains("delivered changed from unknown to step"),
            "{case}: {reason}"
        );
        assert!(reason.contains("unknown is final"), "{case}: {reason}");
    }
    assert_others_passed(&report, RETRY_REPLY_CASES);
}

#[tokio::test]
async fn retries_differing_in_state_after_the_run_ended_fail_only_the_settled_retry_case() {
    let report = run(&FakeSubject::new(Defects {
        retry_state_moves_after_end: true,
        ..Defects::default()
    }))
    .await;
    let reason = failed(&report, "send_id_retry_settled_same_answer");
    assert!(reason.contains("after its run ended"), "{reason}");
    // The unsettled case cannot tell this from a state still moving, so it
    // passes, as does every other case.
    assert_others_passed(&report, &["send_id_retry_settled_same_answer"]);
}

#[tokio::test]
async fn without_run_ops_the_settled_retry_case_is_skipped_and_nothing_waits() {
    let mut subject = FakeSubject::new(Defects::default());
    subject.claim_process_kill = true;
    subject.capabilities.remove(&Capability::RunOps);
    // Each poll pauses half a second, so a case that waited out the suite's
    // poll bound would take 400 polls, over three minutes. The timeout sits
    // under that, so a case that waits on a run it cannot observe fails this
    // test by name instead of passing slowly or hanging it. It sits well over
    // an honest suite run, which other cases' half-second polls make take
    // about half a minute on a loaded host.
    subject.pause_for = Some(Duration::from_millis(500));
    let dir = tempfile::tempdir().unwrap();
    let report = tokio::time::timeout(Duration::from_secs(150), run_suite(&subject, dir.path()))
        .await
        .expect("the suite did not finish within 150 seconds on a runner without run_ops")
        .unwrap();
    assert_passed(&report, "send_id_retry_same_answer");
    assert_eq!(
        report.outcome("send_id_retry_settled_same_answer"),
        Some(&CaseOutcome::Skipped {
            missing: vec![Capability::RunOps]
        }),
        "{}",
        report.render()
    );
    // The fake refuses run.result here, as a runner that does not declare
    // run_ops would; none should have been asked.
    assert_eq!(subject.run_result_calls(), 0, "{}", report.render());
    match &report.verdict {
        SuiteVerdict::ConformingForDeclaredCapabilities { skipped, .. } => {
            assert_eq!(
                skipped,
                &vec![Capability::RunOps, Capability::Retention],
                "{}",
                report.render()
            );
        }
        other => panic!(
            "expected conforming for declared capabilities, got {other:?}\n{}",
            report.render()
        ),
    }
}

#[tokio::test]
async fn a_reuse_answered_instead_of_refused_fails_the_reuse_reply_halves() {
    let report = run(&FakeSubject::new(Defects {
        reuse_accepted: true,
        ..Defects::default()
    }))
    .await;
    for case in [
        "send_id_reuse_refused_naming_field",
        "delivery_change_refused",
    ] {
        let reason = failed(&report, case);
        assert!(reason.contains("not refused"), "{case}: {reason}");
    }
    // The fake answers the reuse as a retry and writes nothing, so the
    // written-once halves still pass.
    assert_others_passed(
        &report,
        &[
            "send_id_reuse_refused_naming_field",
            "delivery_change_refused",
        ],
    );
}

#[tokio::test]
async fn a_retry_moving_delivered_from_pending_to_unknown_passes() {
    let mut subject = FakeSubject::new(Defects::default());
    subject.queue_receipt_pending_then_unknown = true;
    let report = run(&subject).await;
    assert_others_passed(&report, &[]);
}

#[tokio::test]
async fn a_used_work_dir_is_refused() {
    let dir = tempfile::tempdir().unwrap();
    std::fs::write(dir.path().join("left-over"), b"x").unwrap();
    let error = run_suite(&FakeSubject::new(Defects::default()), dir.path())
        .await
        .unwrap_err();
    assert!(matches!(error, SetupError::WorkDirNotEmpty(_)));
}

// Retention tests use the fake's explicit clock hook and can therefore run with
// Tokio time paused; real subjects may use the same hook or short real waits.
async fn retention_break(defect: RetentionDefect, expected: &str, paused: bool) {
    let mut subject = FakeSubject::new(Defects::default());
    subject.enable_retention();
    subject.claim_process_kill = true;
    subject.retention_defect = defect;
    subject.held_paused = paused;
    if defect == RetentionDefect::WithoutGroupIgnored {
        subject.capabilities.remove(&Capability::Retention);
    }
    let report = run(&subject).await;
    let reason = failed(&report, expected);
    let failing: Vec<_> = report
        .cases
        .iter()
        .filter_map(|case| match &case.outcome {
            CaseOutcome::Failed { reason } => Some(format!("{}: {reason}", case.case)),
            _ => None,
        })
        .collect();
    // A retention defect must not trip an unrelated contract check.
    for spec in CASES.iter().filter(|spec| {
        !spec.name.starts_with("retention_") && spec.name != "crash_at_RetentionTombstoned"
    }) {
        assert_passed(&report, spec.name);
    }
    eprintln!(
        "NON-VACUITY BREAK {defect:?}: expected {expected}: {reason}; all failures: {}",
        failing.join("; ")
    );
}

macro_rules! retention_violation {
    ($name:ident, $defect:ident, $case:literal) => {
        #[tokio::test(start_paused = true)]
        async fn $name() {
            retention_break(RetentionDefect::$defect, $case, false).await;
        }
    };
}

retention_violation!(
    retention_violation_ignored_fails_by_name,
    Ignore,
    "retention_honoured"
);
retention_violation!(
    retention_violation_empty_fails_by_name,
    Empty,
    "retention_honoured"
);
retention_violation!(
    retention_violation_timestamp_fails_by_name,
    Timestamp,
    "retention_honoured"
);
retention_violation!(
    retention_violation_lineage_reused_fails_by_name,
    ReuseLineage,
    "retention_honoured"
);
retention_violation!(
    retention_violation_inherited_fails_by_name,
    Inherit,
    "retention_honoured"
);
retention_violation!(
    retention_violation_shorten_ignored_fails_by_name,
    ShortenIgnored,
    "retention_shorten_continues_lineage"
);
retention_violation!(
    retention_violation_shorten_replaces_fails_by_name,
    ShortenReplaces,
    "retention_shorten_continues_lineage"
);
retention_violation!(
    retention_violation_equal_refused_fails_by_name,
    EqualRefused,
    "retention_equal_noop"
);
retention_violation!(
    retention_violation_lengthen_accepted_fails_by_name,
    LengthenAccepted,
    "retention_lengthen_refused"
);
retention_violation!(
    retention_violation_late_opt_in_fails_by_name,
    LateOptInAccepted,
    "retention_late_opt_in_refused"
);
retention_violation!(
    retention_violation_zero_fails_by_name,
    ZeroAccepted,
    "retention_zero_refused"
);
retention_violation!(
    retention_violation_above_max_fails_by_name,
    AboveMaxAccepted,
    "retention_above_max_refused"
);
retention_violation!(
    retention_violation_max_detail_fails_by_name,
    MaxDetailMissing,
    "retention_above_max_refused"
);
retention_violation!(
    retention_violation_without_group_fails_by_name,
    WithoutGroupIgnored,
    "retention_without_group_refused"
);
retention_violation!(
    retention_violation_held_fails_by_name,
    ExpireHeld,
    "retention_active_run_never_expires"
);
retention_violation!(
    retention_violation_clock_fails_by_name,
    ClockNotRestarted,
    "retention_active_run_never_expires"
);

#[tokio::test(start_paused = true)]
async fn retention_violation_paused_fails_by_name() {
    retention_break(
        RetentionDefect::ExpireHeld,
        "retention_active_run_never_expires",
        true,
    )
    .await;
}
retention_violation!(
    retention_violation_never_purges_fails_by_name,
    NeverPurge,
    "retention_honoured"
);
retention_violation!(
    retention_violation_restart_fails_by_name,
    TombstoneForgotten,
    "crash_at_RetentionTombstoned"
);
retention_violation!(
    retention_violation_title_fails_by_name,
    TitleLeak,
    "retention_no_content_served"
);
retention_violation!(
    retention_violation_prompt_fails_by_name,
    PromptLeak,
    "retention_no_content_served"
);
retention_violation!(
    retention_violation_tool_fails_by_name,
    ToolLeak,
    "retention_no_content_served"
);
retention_violation!(
    retention_violation_status_fails_by_name,
    Status,
    "retention_run_status_expired"
);

#[tokio::test(start_paused = true)]
async fn retention_paused_non_terminal_run_passes() {
    let mut subject = FakeSubject::new(Defects::default());
    subject.enable_retention();
    subject.claim_process_kill = true;
    subject.held_paused = true;
    let report = run(&subject).await;
    assert_passed(&report, "retention_active_run_never_expires");
    assert_eq!(report.verdict, SuiteVerdict::Passed, "{}", report.render());
}

#[tokio::test(start_paused = true)]
async fn retention_status_is_inapplicable_when_not_served() {
    let mut subject = FakeSubject::new(Defects::default());
    subject.enable_retention();
    subject.claim_process_kill = true;
    subject.advertise_status = false;
    let report = run(&subject).await;
    assert!(
        matches!(report.outcome("retention_run_status_expired"), Some(CaseOutcome::Inapplicable { reason }) if reason.contains("run.status"))
    );
    assert_eq!(report.verdict, SuiteVerdict::Passed, "{}", report.render());
}

#[tokio::test(start_paused = true)]
async fn retention_requires_a_real_tombstone_process_kill() {
    let mut subject = FakeSubject::new(Defects::default());
    subject.enable_retention();
    let report = run(&subject).await;
    let reason = failed(&report, "crash_at_RetentionTombstoned");
    assert!(reason.contains("requires a real process kill"), "{reason}");
    for spec in CASES
        .iter()
        .filter(|s| s.name.starts_with("retention_") && s.name != "retention_without_group_refused")
    {
        assert_passed(&report, spec.name);
    }
}

const COMPACTION_CASES: &[&str] = &[
    "compaction_setup_durable_once",
    "compaction_fence_crash_replays_model_view",
    "model_view_half_open_ranges_and_travel",
    "compaction_unavailable_distinct_from_refuse",
    "compaction_wait_cap",
    "compaction_late_or_stale_answers_discarded",
    "compaction_step_timeout_uses_last_view",
    "compaction_one_view_per_step",
];

async fn compaction_break(defect: CompactionDefect, case: &str) {
    let subject = FakeSubject::new(Defects {
        compaction: defect,
        ..Defects::default()
    });
    let report = run(&subject).await;
    assert!(!failed(&report, case).is_empty());
    let failures: Vec<_> = report
        .cases
        .iter()
        .filter(|c| matches!(c.outcome, CaseOutcome::Failed { .. }))
        .map(|c| c.case)
        .collect();
    assert_eq!(failures, vec![case], "{}", report.render());
    assert_others_passed(&report, &[case]);
}

#[tokio::test]
async fn compaction_good_fake_passes_all_named_cases() {
    let report = run(&FakeSubject::new(Defects::default())).await;
    for name in COMPACTION_CASES {
        assert_passed(&report, name);
    }
}

#[tokio::test]
async fn compaction_setup_rerun_fails_only_setup_durability() {
    compaction_break(
        CompactionDefect::RerunRecordedSetup,
        "compaction_setup_durable_once",
    )
    .await;
}

#[tokio::test]
async fn compaction_setup_sender_driven_recovery_passes() {
    let mut subject = FakeSubject::new(Defects::default());
    subject.seal_compaction_setup_crash = true;
    let report = run(&subject).await;
    for name in COMPACTION_CASES {
        assert_passed(&report, name);
    }
    assert_others_passed(&report, &[]);
}

async fn compaction_sender_driven_break(defect: CompactionDefect, reason: &str) {
    let mut subject = FakeSubject::new(Defects {
        compaction: defect,
        ..Defects::default()
    });
    subject.seal_compaction_setup_crash = true;
    let report = run(&subject).await;
    let failure = failed(&report, "compaction_setup_durable_once");
    assert!(failure.contains(reason), "{failure}");
    assert_others_passed(&report, &["compaction_setup_durable_once"]);
}

#[tokio::test]
async fn compaction_setup_sender_driven_rerun_fails_only_setup_durability() {
    compaction_sender_driven_break(CompactionDefect::RerunSetupOnResumeSend, "Setup was re-run")
        .await;
}

#[tokio::test]
async fn compaction_setup_sender_driven_changed_view_fails_only_setup_durability() {
    compaction_sender_driven_break(
        CompactionDefect::ChangeSetupResumeView,
        "content at message 1 differs",
    )
    .await;
}

#[tokio::test]
async fn compaction_setup_unexpected_recovery_state_fails_only_setup_durability() {
    compaction_sender_driven_break(
        CompactionDefect::CancelSetupCrash,
        "unexpected run state cancelled",
    )
    .await;
}

#[tokio::test]
async fn compaction_early_fold_fails_only_fence_crash() {
    compaction_break(
        CompactionDefect::FoldBeforeAnswer,
        "compaction_fence_crash_replays_model_view",
    )
    .await;
}
#[tokio::test]
async fn compaction_forgotten_answer_fails_only_fence_crash() {
    compaction_break(
        CompactionDefect::ForgetRecordedFold,
        "compaction_fence_crash_replays_model_view",
    )
    .await;
}
#[tokio::test]
async fn compaction_insertion_after_message_fails_only_model_ranges() {
    compaction_break(
        CompactionDefect::InsertionAfterMessage,
        "model_view_half_open_ranges_and_travel",
    )
    .await;
}
#[tokio::test]
async fn compaction_unavailable_as_refusal_fails_only_refusals() {
    compaction_break(
        CompactionDefect::UnavailableAsRefusal,
        "compaction_unavailable_distinct_from_refuse",
    )
    .await;
}
#[tokio::test]
async fn compaction_finer_code_replaces_role_code_fails_only_refusals() {
    compaction_break(
        CompactionDefect::FinerCodeReplacesRoleCode,
        "compaction_unavailable_distinct_from_refuse",
    )
    .await;
}
#[tokio::test]
async fn compaction_over_wait_cap_fails_only_wait_cap() {
    compaction_break(CompactionDefect::ExceedWaitCap, "compaction_wait_cap").await;
}
#[tokio::test]
async fn compaction_applies_late_fails_only_late_stale_answers() {
    compaction_break(
        CompactionDefect::ApplyLate,
        "compaction_late_or_stale_answers_discarded",
    )
    .await;
}
#[tokio::test]
async fn compaction_applies_stale_fails_only_late_stale_answers() {
    compaction_break(
        CompactionDefect::ApplyStale,
        "compaction_late_or_stale_answers_discarded",
    )
    .await;
}
#[tokio::test]
async fn compaction_timeout_ends_unavailable_fails_only_step_timeout() {
    compaction_break(
        CompactionDefect::TimeoutEndsUnavailable,
        "compaction_step_timeout_uses_last_view",
    )
    .await;
}
#[tokio::test]
async fn compaction_two_views_fails_only_one_view_per_step() {
    compaction_break(
        CompactionDefect::ApplyTwoViews,
        "compaction_one_view_per_step",
    )
    .await;
}
#[tokio::test]
async fn compaction_missing_groups_are_not_applicable_by_name() {
    let mut subject = FakeSubject::new(Defects::default());
    subject.capabilities.remove(&Capability::Compaction);
    subject.capabilities.remove(&Capability::ModelView);
    let report = run(&subject).await;
    for name in COMPACTION_CASES {
        assert!(
            matches!(report.outcome(name), Some(CaseOutcome::Skipped { missing }) if missing.contains(&Capability::Compaction)),
            "{name}: {:?}",
            report.outcome(name)
        );
        assert!(report
            .render()
            .lines()
            .any(|line| line.starts_with(&format!("N/A  {name} "))));
    }
}

#[tokio::test]
async fn compaction_model_view_missing_is_not_applicable_independently() {
    let mut subject = FakeSubject::new(Defects::default());
    subject.capabilities.remove(&Capability::ModelView);
    let report = run(&subject).await;
    for name in COMPACTION_CASES {
        if *name == "compaction_unavailable_distinct_from_refuse" {
            assert_passed(&report, name);
        } else {
            assert_eq!(
                report.outcome(name),
                Some(&CaseOutcome::Skipped {
                    missing: vec![Capability::ModelView]
                })
            );
            assert!(report
                .render()
                .lines()
                .any(|line| line.starts_with(&format!("N/A  {name} "))));
        }
    }
}
