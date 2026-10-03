//! Tests of the suite itself, against the reference fake in `fake/`.
//! Conformance of a real runner is run by that runner, never here.

mod fake;

use cortexkit_role_llm_runner_conformance::{
    harness::KillMechanism, run_suite, Capability, CaseOutcome, SetupError, SuiteReport,
    SuiteVerdict, CASES,
};
use fake::{Defects, FakeSubject, StepRecovery};
use std::time::Duration;

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
    assert_eq!(report.kills.len(), 6, "{}", report.render());
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

#[tokio::test]
async fn a_faithful_runner_whose_kills_end_a_process_passes() {
    let mut subject = FakeSubject::new(Defects::default());
    subject.claim_process_kill = true;
    let report = run(&subject).await;
    assert_eq!(report.verdict, SuiteVerdict::Passed, "{}", report.render());
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
                &vec![Capability::Streaming, Capability::HoldToolCalls]
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
/// the defect changed exactly one answer, and require that it fails the
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
async fn a_guaranteed_runner_with_an_absent_first_steer_receipt_passes() {
    let mut subject = FakeSubject::new(Defects::default());
    subject.omit_first_steer_receipt = true;
    let report = run(&subject).await;
    assert_others_passed(&report, &[]);
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
            assert_eq!(skipped, &vec![Capability::RunOps], "{}", report.render());
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
