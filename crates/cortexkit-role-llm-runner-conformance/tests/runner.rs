//! Tests of the suite itself, against the reference fake in `fake/`.
//! Conformance of a real runner is run by that runner, never here.

mod fake;

fn held_call_subject(defects: fake::Defects) -> fake::FakeSubject {
    let mut subject = fake::FakeSubject::new(defects);
    subject.held_calls_group = true;
    subject.advertise_status = true;
    subject.capabilities.insert(Capability::RunStatus);
    subject
}

#[tokio::test]
async fn held_calls_faithful_runner_passes_all_cases() {
    let report = run(&held_call_subject(Defects::default())).await;
    for name in cases_of(&["held_calls_"]) {
        assert_eq!(
            report.outcome(name),
            Some(&CaseOutcome::Passed),
            "{name}\n{}",
            report.render()
        );
    }
    assert_none_failed_but(&report, &[]);
}

#[tokio::test]
async fn held_calls_never_listed_fails_only_listing_case() {
    let report = run(&held_call_subject(Defects {
        held_calls_never_listed: true,
        ..Defects::default()
    }))
    .await;
    let name = "held_calls_lists_waiting_call";
    let reason = failed(&report, name);
    println!("{name}: {reason}");
    assert!(reason.contains("never listed"), "{reason}");
    assert_none_failed_but(&report, &[name]);
}

#[tokio::test]
async fn held_calls_stale_entry_fails_only_late_result_case() {
    let report = run(&held_call_subject(Defects {
        held_calls_stale: true,
        ..Defects::default()
    }))
    .await;
    let name = "held_calls_applies_late_result";
    let reason = failed(&report, name);
    println!("{name}: {reason}");
    assert!(reason.contains("kept the held call"), "{reason}");
    assert_none_failed_but(&report, &[name]);
}

#[tokio::test]
async fn held_calls_undeclared_group_is_inapplicable_by_name() {
    let mut subject = held_call_subject(Defects::default());
    subject.held_calls_group = false;
    let report = run(&subject).await;
    for name in cases_of(&["held_calls_"]) {
        assert!(matches!(report.outcome(name),
            Some(CaseOutcome::Inapplicable { reason }) if reason == "the runner does not declare held_calls"));
    }
}

use cortexkit_role_llm_runner::send::{delivered_as, Delivered};
use cortexkit_role_llm_runner_conformance::{
    harness::KillMechanism, progress, run_suite, run_suite_with, Capability, CaseOutcome,
    SetupError, SuiteOptions, SuiteProgress, SuiteReport, SuiteVerdict, CASES,
};
use fake::{
    CacheTtlDefect, CompactionDefect, CompactionRouteMode, Defects, DirectiveDefect, FakeSubject,
    LineageDefect, RetentionDefect, StepRecovery,
};
use std::{
    sync::{Arc, Mutex},
    time::{Duration, Instant},
};

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

/// The case that resolves tags only after a `StepRecorded` seal that moves
/// the session onto a new lineage. The fake resumes by default, so the case
/// is inapplicable by name; the tests that seal check it passes and fails.
const LEFT_LINEAGE_CASE: &str = "tags_resolve_answers_the_current_lineage";

/// Optional groups the fake leaves undeclared unless a test declares them.
const OPTIONAL_GROUPS: &[Capability] = &[
    Capability::Attachments,
    Capability::RetentionClock,
    Capability::UserCommands,
    Capability::InputTokenCap,
    Capability::MaxSteps,
    Capability::MaxOutputTokens,
    Capability::NoTools,
    Capability::TurnDirectives,
    Capability::InterruptReason,
    Capability::StructuredOutput,
    Capability::Adopt,
    Capability::Tags,
    Capability::RunToolCalls,
    Capability::RunUsage,
    Capability::SessionUsage,
    Capability::DeliveryOrder,
    Capability::Retract,
    Capability::SendStatus,
    Capability::SessionChange,
    Capability::HostHistory,
    Capability::HistoryDisplay,
    Capability::LiveEvents,
    Capability::ClientMessageId,
    Capability::RunCause,
];

/// The opt-in cost case: a subject without the seeding hook cannot run it.
const COST_CASE: &str = "per_step_cost_scales_with_new_messages";

/// The cases that interrupt a running turn: inapplicable on a runner
/// without `interrupt`.
const INTERRUPT_CASES: &[&str] = &[
    "run_cause_interrupt_names_the_new_send",
    "delivery_order_ungranted_interrupt_refused",
    "delivery_order_granted_interrupt_interrupts",
];

/// The case reading the `sender` of a plan sender's display row.
const SENDER_CASE: &str = "display_row_names_the_sender_never_in_read";

fn assert_passed(report: &SuiteReport, case: &str) {
    if case.starts_with("system_text_default_")
        && matches!(report.outcome(case), Some(CaseOutcome::Inapplicable { reason }) if reason == "runner does not declare system_text_default")
    {
        return;
    }
    if case == "head_incarnation_survives_cut_and_changes_on_recreation"
        && matches!(report.outcome(case), Some(CaseOutcome::Inapplicable { reason }) if reason == "adapter cannot cut the incarnation log")
    {
        return;
    }
    if case == "head_sender_is_callers_effective_grant"
        && matches!(report.outcome(case), Some(CaseOutcome::Inapplicable { reason }) if reason == "adapter supplies no two-identity head.sender fixture")
    {
        return;
    }
    if case == "head_model_reflects_recorded_switch"
        && matches!(report.outcome(case), Some(CaseOutcome::Inapplicable { reason }) if reason == "runner does not report head.model")
    {
        return;
    }
    if case.starts_with("output_refs_")
        && matches!(report.outcome(case),Some(CaseOutcome::Inapplicable { reason }) if reason == "the runner does not declare output_refs" || reason == "the runner declares output_refs")
    {
        return;
    }
    if case.starts_with("held_calls_")
        && matches!(report.outcome(case),
        Some(CaseOutcome::Inapplicable { reason }) if reason == "the runner does not declare held_calls")
    {
        return;
    }
    if case == "cut_setup_leaves_reads_alone"
        && matches!(report.outcome(case), Some(CaseOutcome::Inapplicable { reason }) if reason.contains("never refuses reads"))
    {
        return;
    }
    if case == HOST_CUT_CASE
        && matches!(report.outcome(case), Some(CaseOutcome::Inapplicable { reason }) if reason.contains("host cut"))
    {
        return;
    }
    if case == COST_CASE
        && matches!(report.outcome(case), Some(CaseOutcome::Inapplicable { reason }) if reason.contains("cannot seed"))
    {
        return;
    }
    if case.starts_with("media_")
        && matches!(report.outcome(case), Some(CaseOutcome::Inapplicable { reason }) if reason.contains("byte-backed image"))
    {
        return;
    }
    // A case skipped only for optional groups the subject leaves undeclared.
    if let Some(CaseOutcome::Skipped { missing }) = report.outcome(case) {
        if !missing.is_empty() && missing.iter().all(|group| OPTIONAL_GROUPS.contains(group)) {
            return;
        }
    }
    if case == NO_RUN_OPS_STEER_CASE {
        if matches!(
            report.outcome(case),
            Some(CaseOutcome::Inapplicable { reason }) if reason.contains("run_ops")
        ) {
            return;
        }
        if let Some(CaseOutcome::Skipped { missing }) = report.outcome(case) {
            let needs = [
                Capability::Steer,
                Capability::Queue,
                Capability::TranscriptReads,
                Capability::HoldToolCalls,
            ];
            if !missing.is_empty() && missing.iter().all(|capability| needs.contains(capability)) {
                return;
            }
        }
    }
    if case == UNDECLARED_MODE_CASE {
        if matches!(
            report.outcome(case),
            Some(CaseOutcome::Inapplicable { reason }) if reason.contains("steer")
        ) {
            return;
        }
        if let Some(CaseOutcome::Skipped { missing }) = report.outcome(case) {
            let needs = [
                Capability::DeliveryOrder,
                Capability::Queue,
                Capability::TranscriptReads,
                Capability::HoldToolCalls,
            ];
            if !missing.is_empty() && missing.iter().all(|capability| needs.contains(capability)) {
                return;
            }
        }
    }
    if case == "delivery_order_template_renders_as_system"
        && matches!(
            report.outcome(case),
            Some(CaseOutcome::Inapplicable { reason })
                if reason.contains("neither steer nor a supported turn_system mode")
        )
    {
        return;
    }
    if case == LEFT_LINEAGE_CASE
        && matches!(
            report.outcome(case),
            Some(CaseOutcome::Inapplicable { reason }) if reason.contains("no lineage")
        )
    {
        return;
    }
    // The fake leaves interrupt undeclared unless a test declares it, so the
    // cases that interrupt a running turn cannot be asked.
    if INTERRUPT_CASES.contains(&case)
        && matches!(
            report.outcome(case),
            Some(CaseOutcome::Inapplicable { reason }) if reason.contains("interrupt")
        )
    {
        return;
    }
    // A fake narrowing its list modes may leave idle_start out, which the
    // sender case lists.
    if case == SENDER_CASE
        && matches!(
            report.outcome(case),
            Some(CaseOutcome::Inapplicable { reason }) if reason.contains("idle_start")
        )
    {
        return;
    }
    // The fake leaves structured_output undeclared unless a test declares
    // it, so the cleaning case names the structured-output rule unchecked.
    if [
        "tags_model_written_tags_cleaned_on_exposure",
        "tags_host_store_cleaned_at_the_save_seam",
    ]
    .contains(&case)
        && matches!(
            report.outcome(case),
            Some(CaseOutcome::PassedWithInapplicableRule { reason }) if reason.contains("structured_output")
        )
    {
        return;
    }
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
    assert_eq!(report.kills.len(), 7, "{}", report.render());
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
    subject.held_calls_group = true;
    subject.capabilities.insert(Capability::RunStatus);
    subject.claim_process_kill = true;
    subject.descriptor_fixtures = true;
    subject.capabilities.insert(Capability::Attachments);
    subject.enable_retention();
    subject.capabilities.insert(Capability::InputTokenCap);
    subject.capabilities.insert(Capability::StructuredOutput);
    subject.capabilities.insert(Capability::Adopt);
    subject.capabilities.insert(Capability::Tags);
    subject.capabilities.insert(Capability::RunToolCalls);
    subject.capabilities.insert(Capability::RunUsage);
    subject.capabilities.insert(Capability::SessionUsage);
    subject.capabilities.insert(Capability::DeliveryOrder);
    subject.capabilities.insert(Capability::SendStatus);
    subject.capabilities.insert(Capability::UserCommands);
    subject.capabilities.insert(Capability::HostHistory);
    subject.capabilities.extend([
        Capability::HistoryDisplay,
        Capability::LiveEvents,
        Capability::ClientMessageId,
        Capability::RunCause,
    ]);
    subject.host_prefix_hook = true;
    subject.capabilities.extend([
        Capability::MaxSteps,
        Capability::MaxOutputTokens,
        Capability::NoTools,
        Capability::TurnDirectives,
        Capability::InterruptReason,
        Capability::SessionChange,
    ]);
    let started = Instant::now();
    let report = run(&subject).await;
    assert_eq!(report.verdict, SuiteVerdict::Passed, "{}", report.render());
    for spec in CASES
        .iter()
        .filter(|case| case.name.starts_with("media_") || case.name.starts_with("attachments_"))
    {
        assert_eq!(
            report.outcome(spec.name),
            Some(&CaseOutcome::Passed),
            "{}\n{}",
            spec.name,
            report.render()
        );
    }
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
    assert!(reason.contains("closest to shape (b)"), "{reason}");
    assert!(reason.contains("dangling call call_b"), "{reason}");
    assert!(reason.contains("never-executed result"), "{reason}");
    assert_passed(&report, "crash_at_DispatchIntent");
}

/// A seal that keeps the call but writes its never-executed result only
/// when the owner's next send arrives: the transcript read between the
/// commit that ended the run and that send holds a dangling call.
#[tokio::test]
async fn step_recorded_kept_call_result_in_a_later_commit_fails() {
    let mut subject = FakeSubject::new(Defects {
        sealed_result_in_later_commit: true,
        ..Defects::default()
    });
    subject.step_recovery = StepRecovery::SealInterrupted;
    let report = run(&subject).await;
    let reason = failed(&report, "crash_at_StepRecorded");
    assert!(reason.contains("sealed branch:"), "{reason}");
    assert!(reason.contains("closest to shape (b)"), "{reason}");
    assert!(reason.contains("dangling call call_b"), "{reason}");
    // Both the read once the run ended and the follow-up's read catch this
    // defect; each reason names the commit that ended the run.
    assert!(reason.contains("the commit that ended the run"), "{reason}");
    assert_others_passed(&report, &["crash_at_StepRecorded"]);
}

/// A seal that keeps the call and its lineage but changes the bytes of a
/// message read before the kill.
#[tokio::test]
async fn step_recorded_kept_call_rewriting_a_read_message_fails() {
    let mut subject = FakeSubject::new(Defects {
        sealed_rewrites_read_message: true,
        ..Defects::default()
    });
    subject.step_recovery = StepRecovery::SealInterrupted;
    let report = run(&subject).await;
    let reason = failed(&report, "crash_at_StepRecorded");
    assert!(reason.contains("sealed branch:"), "{reason}");
    assert!(reason.contains("closest to shape (b)"), "{reason}");
    assert!(reason.contains("(rewritten)"), "{reason}");
    assert!(reason.contains("a message already read"), "{reason}");
    assert_others_passed(&report, &["crash_at_StepRecorded"]);
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
                    Capability::Attachments,
                    Capability::HostHistory,
                    Capability::Streaming,
                    Capability::SessionChange,
                    Capability::Retention,
                    Capability::InputTokenCap,
                    Capability::MaxSteps,
                    Capability::MaxOutputTokens,
                    Capability::NoTools,
                    Capability::TurnDirectives,
                    Capability::InterruptReason,
                    Capability::StructuredOutput,
                    Capability::Adopt,
                    Capability::RunToolCalls,
                    Capability::RunUsage,
                    Capability::SessionUsage,
                    Capability::DeliveryOrder,
                    Capability::SendStatus,
                    Capability::UserCommands,
                    Capability::HistoryDisplay,
                    Capability::LiveEvents,
                    Capability::ClientMessageId,
                    Capability::RunCause,
                    Capability::RetentionClock,
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
                &vec![
                    Capability::Attachments,
                    Capability::HostHistory,
                    Capability::RunOps,
                    Capability::SessionChange,
                    Capability::Retention,
                    Capability::InputTokenCap,
                    Capability::MaxSteps,
                    Capability::MaxOutputTokens,
                    Capability::NoTools,
                    Capability::TurnDirectives,
                    Capability::InterruptReason,
                    Capability::StructuredOutput,
                    Capability::Adopt,
                    Capability::RunToolCalls,
                    Capability::RunUsage,
                    Capability::SessionUsage,
                    Capability::DeliveryOrder,
                    Capability::SendStatus,
                    Capability::UserCommands,
                    Capability::HistoryDisplay,
                    Capability::LiveEvents,
                    Capability::ClientMessageId,
                    Capability::RunCause,
                    Capability::RetentionClock
                ],
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

/// A faulty fake with the default (partial) declarations: its run has a
/// failing case and cases gated off for a missing capability.
fn tail_returns_oldest() -> FakeSubject {
    FakeSubject::new(Defects {
        tail_returns_oldest: true,
        ..Defects::default()
    })
}

#[tokio::test]
async fn progress_reports_every_case_started_then_finished_in_suite_order() {
    let subject = tail_returns_oldest();
    let events = Arc::new(Mutex::new(Vec::new()));
    let recorder = Arc::clone(&events);
    let options = SuiteOptions::new()
        .with_progress(move |event: &SuiteProgress| recorder.lock().unwrap().push(event.clone()));
    let dir = tempfile::tempdir().unwrap();
    let report = run_suite_with(&subject, dir.path(), options).await.unwrap();
    let events = events.lock().unwrap().clone();

    assert_eq!(events.len(), 2 * CASES.len(), "{events:#?}");
    let mut gated = 0;
    for (index, (spec, pair)) in CASES.iter().zip(events.chunks(2)).enumerate() {
        assert_eq!(
            pair[0],
            SuiteProgress::CaseStarted {
                case: spec.name,
                index,
                total: CASES.len(),
            }
        );
        match &pair[1] {
            SuiteProgress::CaseFinished {
                case,
                outcome,
                elapsed_ms,
            } => {
                assert_eq!(*case, spec.name);
                assert_eq!(Some(outcome), report.outcome(spec.name), "{case}");
                // A case skipped for a missing capability is decided before
                // anything runs, so its START and END come back to back.
                if matches!(outcome, CaseOutcome::Skipped { .. }) {
                    gated += 1;
                    assert!(*elapsed_ms < 1_000, "{case} took {elapsed_ms} ms");
                }
            }
            other => panic!("expected {} to finish, got {other:?}", spec.name),
        }
    }
    assert!(gated > 0, "the run should gate off at least one case");
    assert!(events.iter().any(|event| matches!(
        event,
        SuiteProgress::CaseFinished {
            case: "tail_read_is_newest_page",
            outcome: CaseOutcome::Failed { .. },
            ..
        }
    )));
}

/// The property progress exists for: START goes out before a case does any
/// work, so a case stuck in the runner leaves START with no END. The
/// callback marks the case the suite is in from its START to its END, and
/// the fake labels every route request it receives with that mark.
#[tokio::test]
async fn progress_starts_each_case_before_its_route_calls() {
    let subject = tail_returns_oldest();
    let current = Arc::clone(&subject.current_case);
    let options = SuiteOptions::new().with_progress(move |event: &SuiteProgress| {
        *current.lock().unwrap() = match event {
            SuiteProgress::CaseStarted { case, .. } => Some(*case),
            _ => None,
        };
    });
    let dir = tempfile::tempdir().unwrap();
    let report = run_suite_with(&subject, dir.path(), options).await.unwrap();
    let calls = subject.route_calls.lock().unwrap().clone();

    let outside: Vec<&str> = calls
        .iter()
        .filter(|(case, _)| case.is_none())
        .map(|(_, method)| method.as_str())
        .collect();
    assert!(
        outside.is_empty(),
        "{} of {} route requests arrived outside every case's START..END: {outside:?}",
        outside.len(),
        calls.len()
    );
    let with_calls: std::collections::BTreeSet<&str> =
        calls.iter().filter_map(|(case, _)| *case).collect();
    for case in &with_calls {
        // A case skipped for a missing capability never reaches the runner,
        // so requests labelled with it belong to some other case.
        assert!(
            !matches!(report.outcome(case), Some(CaseOutcome::Skipped { .. })),
            "route requests were labelled with {case}, which was skipped"
        );
    }
    assert!(
        with_calls.contains("tail_read_is_newest_page"),
        "{with_calls:?}"
    );
    assert!(
        with_calls.len() >= 50,
        "only {} cases made route requests",
        with_calls.len()
    );
}

#[tokio::test]
async fn run_suite_reports_the_same_with_or_without_progress() {
    // A fake keeps state between runs (held calls, sessions), so each run
    // gets its own.
    let plain = run(&tail_returns_oldest()).await;
    let dir = tempfile::tempdir().unwrap();
    let no_options = run_suite_with(&tail_returns_oldest(), dir.path(), SuiteOptions::new())
        .await
        .unwrap();
    let dir = tempfile::tempdir().unwrap();
    let events = Arc::new(Mutex::new(0usize));
    let counter = Arc::clone(&events);
    let observed = run_suite_with(
        &tail_returns_oldest(),
        dir.path(),
        SuiteOptions::new().with_progress(move |_: &SuiteProgress| *counter.lock().unwrap() += 1),
    )
    .await
    .unwrap();
    assert!(matches!(plain.verdict, SuiteVerdict::Failed { .. }));
    assert_eq!(plain, no_options);
    assert_eq!(plain, observed);
    assert_eq!(*events.lock().unwrap(), 2 * CASES.len());
}

/// The ready-made stderr printer fits `with_progress` as is. It is not run
/// here: its lines would bypass the test harness's output capture.
#[allow(dead_code)]
fn stderr_lines_fit_suite_options() -> SuiteOptions {
    SuiteOptions::new().with_progress(progress::stderr_lines())
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
    subject.held_calls_group = true;
    subject.capabilities.insert(Capability::RunStatus);
    subject.capabilities.insert(Capability::Attachments);
    subject.enable_retention();
    subject.claim_process_kill = true;
    subject.capabilities.insert(Capability::InputTokenCap);
    subject.capabilities.insert(Capability::StructuredOutput);
    subject.capabilities.insert(Capability::Adopt);
    subject.capabilities.insert(Capability::Tags);
    subject.capabilities.insert(Capability::RunToolCalls);
    subject.capabilities.insert(Capability::RunUsage);
    subject.capabilities.insert(Capability::SessionUsage);
    subject.capabilities.insert(Capability::DeliveryOrder);
    subject.capabilities.insert(Capability::SendStatus);
    subject.capabilities.insert(Capability::UserCommands);
    subject.capabilities.insert(Capability::HostHistory);
    subject.capabilities.extend([
        Capability::HistoryDisplay,
        Capability::LiveEvents,
        Capability::ClientMessageId,
        Capability::RunCause,
    ]);
    subject.host_prefix_hook = true;
    subject.capabilities.extend([
        Capability::MaxSteps,
        Capability::MaxOutputTokens,
        Capability::NoTools,
        Capability::TurnDirectives,
        Capability::InterruptReason,
        Capability::SessionChange,
    ]);
    subject.held_paused = true;
    let report = run(&subject).await;
    assert_passed(&report, "retention_active_run_never_expires");
    assert_eq!(report.verdict, SuiteVerdict::Passed, "{}", report.render());
}

#[tokio::test(start_paused = true)]
async fn retention_status_is_inapplicable_when_not_served() {
    let mut subject = FakeSubject::new(Defects::default());
    subject.capabilities.insert(Capability::Attachments);
    subject.enable_retention();
    subject.claim_process_kill = true;
    subject.capabilities.insert(Capability::InputTokenCap);
    subject.capabilities.insert(Capability::StructuredOutput);
    subject.capabilities.insert(Capability::Adopt);
    subject.capabilities.insert(Capability::Tags);
    subject.capabilities.insert(Capability::RunToolCalls);
    subject.capabilities.insert(Capability::RunUsage);
    subject.capabilities.insert(Capability::SessionUsage);
    subject.capabilities.insert(Capability::DeliveryOrder);
    subject.capabilities.insert(Capability::SendStatus);
    subject.capabilities.insert(Capability::UserCommands);
    subject.capabilities.insert(Capability::HostHistory);
    subject.capabilities.extend([
        Capability::HistoryDisplay,
        Capability::LiveEvents,
        Capability::ClientMessageId,
        Capability::RunCause,
    ]);
    subject.host_prefix_hook = true;
    subject.capabilities.extend([
        Capability::MaxSteps,
        Capability::MaxOutputTokens,
        Capability::NoTools,
        Capability::TurnDirectives,
        Capability::InterruptReason,
        Capability::SessionChange,
    ]);
    subject.advertise_status = false;
    let report = run(&subject).await;
    assert!(
        matches!(report.outcome("retention_run_status_expired"), Some(CaseOutcome::Inapplicable { reason }) if reason.contains("run.status"))
    );
    assert_eq!(report.verdict, SuiteVerdict::Passed, "{}", report.render());
    println!(
        "retention_status_is_inapplicable_when_not_served verdict: {:?}",
        report.verdict
    );
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
    "compaction_setup_route_session_matches_setup",
    "model_view_half_open_ranges_and_travel",
    "compaction_unavailable_distinct_from_refuse",
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
async fn compaction_setup_route_session_mismatch_fails_only_named_case() {
    compaction_break(
        CompactionDefect::BindWrongSession,
        "compaction_setup_route_session_matches_setup",
    )
    .await;
}

/// A runner that records a `setup_identity_mismatch` refusal under another
/// code fails the route-session case, by name and with that reason.
#[tokio::test]
async fn compaction_identity_mismatch_under_another_code_fails_only_the_route_session_case() {
    compaction_break(
        CompactionDefect::IdentityMismatchUnderAnotherCode,
        "compaction_setup_route_session_matches_setup",
    )
    .await;
    let report = run(&FakeSubject::new(Defects {
        compaction: CompactionDefect::IdentityMismatchUnderAnotherCode,
        ..Defects::default()
    }))
    .await;
    let reason = failed(&report, "compaction_setup_route_session_matches_setup");
    assert!(
        reason.contains("a Setup refused setup_identity_mismatch"),
        "{reason}"
    );
}

#[tokio::test]
async fn compaction_shared_project_route_with_runner_harness_passes_identity_case() {
    let mut subject = FakeSubject::new(Defects::default());
    subject.compaction_route_mode = CompactionRouteMode::SharedProject;
    let report = run(&subject).await;
    assert_passed(&report, "compaction_setup_route_session_matches_setup");
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
        if *name == "compaction_unavailable_distinct_from_refuse"
            || *name == "compaction_setup_route_session_matches_setup"
        {
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

#[tokio::test]
async fn run_attribution_installed_fixture_passes() {
    let report = run(&FakeSubject::new(Defects::default())).await;
    assert_passed(&report, "run_attribution_per_message");
}

#[tokio::test]
async fn run_attribution_missing_fixture_fails_by_name() {
    let report = run(&FakeSubject::new(Defects {
        attribution_fixture_missing: true,
        ..Defects::default()
    }))
    .await;
    assert!(failed(&report, "run_attribution_per_message").contains("hook is missing"));
}

#[tokio::test]
async fn run_attribution_impossible_fixture_names_only_unknown_origin_skip() {
    let report = run(&FakeSubject::new(Defects {
        attribution_cannot_hold_unattributable: true,
        ..Defects::default()
    }))
    .await;
    assert!(
        matches!(report.outcome("run_attribution_per_message"), Some(CaseOutcome::PassedWithInapplicableRule { reason }) if reason.contains("unknown-origin") && reason.contains("CannotHoldUnattributable"))
    );
    assert!(report
        .render()
        .contains("PASS run_attribution_per_message (one rule not applicable by declaration:"));
    let broken = run(&FakeSubject::new(Defects {
        attribution_cannot_hold_unattributable: true,
        attribution_wrong_final: true,
        ..Defects::default()
    }))
    .await;
    assert!(failed(&broken, "run_attribution_per_message").contains("final is not last-only"));
}

#[tokio::test]
async fn run_attribution_missing_run_fails_by_name() {
    let report = run(&FakeSubject::new(Defects {
        attribution_missing_run: true,
        ..Defects::default()
    }))
    .await;
    assert!(failed(&report, "run_attribution_per_message").contains("missing run"));
}

#[tokio::test]
async fn run_attribution_wrong_final_fails_by_name() {
    let report = run(&FakeSubject::new(Defects {
        attribution_wrong_final: true,
        ..Defects::default()
    }))
    .await;
    assert!(failed(&report, "run_attribution_per_message").contains("final is not last-only"));
}

#[tokio::test]
async fn run_attribution_invented_run_fails_by_name() {
    let report = run(&FakeSubject::new(Defects {
        attribution_invent_run: true,
        ..Defects::default()
    }))
    .await;
    assert!(failed(&report, "run_attribution_per_message").contains("invented run attribution"));
}

#[tokio::test]
async fn run_attribution_running_final_fails_by_name() {
    let report = run(&FakeSubject::new(Defects {
        attribution_running_final: true,
        ..Defects::default()
    }))
    .await;
    assert!(failed(&report, "run_attribution_per_message").contains("durable terminal=false"));
}

#[tokio::test]
async fn run_attribution_tool_result_final_fails_by_name() {
    let report = run(&FakeSubject::new(Defects {
        attribution_assistant_only_final: true,
        ..Defects::default()
    }))
    .await;
    assert!(failed(&report, "run_attribution_per_message").contains("durable terminal=true"));
}

/// A runner may accept a send and end the run a moment later, so a read immediately
/// after the send can still say `active`. The compaction cases that assert how
/// a run ended must wait for the end rather than read once.
#[tokio::test]
async fn compaction_cases_wait_for_a_run_reported_ended_late() {
    let mut subject = FakeSubject::new(Defects::default());
    subject.late_seal_reads = 3;
    let report = run(&subject).await;
    assert_passed(&report, "compaction_unavailable_distinct_from_refuse");
}

const BOUNDARY_CANCEL_CASES: &[&str] = &[
    "run_cancel_step_boundary_finishes_tool_batch",
    "run_cancel_unlisted_at_refused",
    "run_cancel_step_boundary_escalates_to_now",
];

#[tokio::test]
async fn boundary_cancel_cases_pass_for_a_faithful_runner() {
    let report = run(&FakeSubject::new(Defects::default())).await;
    for name in BOUNDARY_CANCEL_CASES {
        assert_passed(&report, name);
    }
}

#[tokio::test]
async fn boundary_cancel_cases_skip_by_name_when_discovery_omits_the_point() {
    let mut subject = FakeSubject::new(Defects::default());
    subject.cancel_step_boundary = false;
    let report = run(&subject).await;
    for name in BOUNDARY_CANCEL_CASES {
        assert_eq!(
            report.outcome(name),
            Some(&CaseOutcome::SkippedWithReason {
                missing: vec![Capability::RunCancel],
                reason: "role.describe.run_cancel.at does not list step_boundary".into(),
            }),
            "{name}\n{}",
            report.render()
        );
    }
}

#[tokio::test]
async fn boundary_cancel_case_rejects_a_runner_starting_another_step() {
    let report = run(&FakeSubject::new(Defects {
        cancel_starts_next_step: true,
        ..Defects::default()
    }))
    .await;
    let reason = failed(&report, "run_cancel_step_boundary_finishes_tool_batch");
    assert!(reason.contains("another model step"), "{reason}");
    assert_others_passed(&report, &["run_cancel_step_boundary_finishes_tool_batch"]);
}

#[tokio::test]
async fn unlisted_cancel_case_rejects_a_runner_ignoring_at() {
    let report = run(&FakeSubject::new(Defects {
        cancel_accepts_unlisted_at: true,
        ..Defects::default()
    }))
    .await;
    let reason = failed(&report, "run_cancel_unlisted_at_refused");
    assert!(reason.contains("not refused"), "{reason}");
    assert_others_passed(&report, &["run_cancel_unlisted_at_refused"]);
}

#[tokio::test]
async fn escalation_cancel_case_rejects_a_runner_ignoring_now() {
    let report = run(&FakeSubject::new(Defects {
        cancel_ignores_escalation: true,
        ..Defects::default()
    }))
    .await;
    let reason = failed(&report, "run_cancel_step_boundary_escalates_to_now");
    assert!(reason.contains("did not end"), "{reason}");
    assert_others_passed(&report, &["run_cancel_step_boundary_escalates_to_now"]);
}

const INPUT_CAP_CASES: &[&str] = &[
    "input_token_cap_at_limit",
    "input_token_cap_crossing_tools",
    "input_token_cap_restart_between_episodes",
    "input_token_cap_raise_refused",
    "input_token_cap_closed_members",
    "input_token_cap_usage_unreported",
];

async fn input_cap_case(name: &str) {
    let mut subject = FakeSubject::new(Defects::default());
    subject.capabilities.insert(Capability::InputTokenCap);
    subject.capabilities.insert(Capability::StructuredOutput);
    let report = run(&subject).await;
    assert_eq!(
        report.outcome(name),
        Some(&CaseOutcome::Passed),
        "{name}: {:?}",
        report.outcome(name)
    );
}

#[tokio::test]
async fn input_token_cap_at_limit() {
    input_cap_case("input_token_cap_at_limit").await;
}
#[tokio::test]
async fn input_token_cap_crossing_tools() {
    input_cap_case("input_token_cap_crossing_tools").await;
}
#[tokio::test]
async fn input_token_cap_restart_between_episodes() {
    input_cap_case("input_token_cap_restart_between_episodes").await;
}
#[tokio::test]
async fn input_token_cap_raise_refused() {
    input_cap_case("input_token_cap_raise_refused").await;
}
#[tokio::test]
async fn input_token_cap_closed_members() {
    input_cap_case("input_token_cap_closed_members").await;
}
#[tokio::test]
async fn input_token_cap_usage_unreported() {
    input_cap_case("input_token_cap_usage_unreported").await;
}

#[tokio::test]
async fn input_token_cap_undeclared_cases_skip_by_name() {
    let report = run(&FakeSubject::new(Defects::default())).await;
    for name in INPUT_CAP_CASES {
        assert_eq!(
            report.outcome(name),
            Some(&CaseOutcome::Skipped {
                missing: vec![Capability::InputTokenCap]
            }),
            "{name}"
        );
        assert!(report.render().contains(&format!("SKIP {name}")));
    }
}

const STRUCTURED_OUTPUT_CASES: &[&str] = &[
    "structured_output_final_text_enforced_passed",
    "structured_output_final_text_with_tools_refused",
    "structured_output_submit_tool_ends_run",
    "structured_output_reserved_name_refused",
    "structured_output_absent_without_submit",
    "structured_output_failed_validation_completes",
    "structured_output_frozen_across_restart",
    "structured_output_not_inherited",
    "structured_output_external_ref_refused",
    "structured_output_sibling_never_dispatched",
];

async fn structured_output_case(name: &str) {
    let mut subject = FakeSubject::new(Defects::default());
    subject.capabilities.insert(Capability::StructuredOutput);
    let report = run(&subject).await;
    assert_eq!(
        report.outcome(name),
        Some(&CaseOutcome::Passed),
        "{name}: {:?}",
        report.outcome(name)
    );
}

#[tokio::test]
async fn structured_output_final_text_enforced_passed() {
    structured_output_case("structured_output_final_text_enforced_passed").await;
}
#[tokio::test]
async fn structured_output_final_text_with_tools_refused() {
    structured_output_case("structured_output_final_text_with_tools_refused").await;
}
#[tokio::test]
async fn structured_output_submit_tool_ends_run() {
    structured_output_case("structured_output_submit_tool_ends_run").await;
}
#[tokio::test]
async fn structured_output_reserved_name_refused() {
    structured_output_case("structured_output_reserved_name_refused").await;
}
#[tokio::test]
async fn structured_output_absent_without_submit() {
    structured_output_case("structured_output_absent_without_submit").await;
}
#[tokio::test]
async fn structured_output_failed_validation_completes() {
    structured_output_case("structured_output_failed_validation_completes").await;
}
#[tokio::test]
async fn structured_output_frozen_across_restart() {
    structured_output_case("structured_output_frozen_across_restart").await;
}
#[tokio::test]
async fn structured_output_not_inherited() {
    structured_output_case("structured_output_not_inherited").await;
}
#[tokio::test]
async fn structured_output_external_ref_refused() {
    structured_output_case("structured_output_external_ref_refused").await;
}
#[tokio::test]
async fn structured_output_sibling_never_dispatched() {
    structured_output_case("structured_output_sibling_never_dispatched").await;
}

#[tokio::test]
async fn structured_output_undeclared_cases_skip_by_name() {
    let report = run(&FakeSubject::new(Defects::default())).await;
    for name in STRUCTURED_OUTPUT_CASES {
        assert_eq!(
            report.outcome(name),
            Some(&CaseOutcome::Skipped {
                missing: vec![Capability::StructuredOutput]
            }),
            "{name}"
        );
        assert!(report.render().contains(&format!("SKIP {name}")));
    }
}

#[tokio::test]
async fn a_runner_dispatching_a_submit_sibling_fails_only_the_sibling_case() {
    let mut subject = FakeSubject::new(Defects {
        structured_output_dispatches_siblings: true,
        ..Defects::default()
    });
    subject.capabilities.insert(Capability::StructuredOutput);
    let report = run(&subject).await;
    let reason = failed(&report, "structured_output_sibling_never_dispatched");
    assert!(reason.contains("was invoked 1 times"), "{reason}");
    for name in STRUCTURED_OUTPUT_CASES {
        if *name != "structured_output_sibling_never_dispatched" {
            assert_passed(&report, name);
        }
    }
}

const ADOPT_CASES: &[&str] = &[
    "adopt_binds_owner_without_a_run",
    "adopt_exact_repeat_replays_reply",
    "adopt_send_id_reuse_names_plan",
    "adopt_plan_drift_refused",
    "adopt_during_active_run",
    "adopt_other_owner_refused",
    "adopt_same_owner_readopts",
    "adopt_then_owner_sends",
];

fn adopting(defects: Defects) -> FakeSubject {
    let mut subject = FakeSubject::new(defects);
    subject.capabilities.insert(Capability::Adopt);
    subject
}

async fn adopt_case(name: &str) {
    let report = run(&adopting(Defects::default())).await;
    assert_eq!(
        report.outcome(name),
        Some(&CaseOutcome::Passed),
        "{name}: {:?}",
        report.outcome(name)
    );
}

#[tokio::test]
async fn adopt_binds_owner_without_a_run() {
    adopt_case("adopt_binds_owner_without_a_run").await;
}
#[tokio::test]
async fn adopt_exact_repeat_replays_reply() {
    adopt_case("adopt_exact_repeat_replays_reply").await;
}
#[tokio::test]
async fn adopt_send_id_reuse_names_plan() {
    adopt_case("adopt_send_id_reuse_names_plan").await;
}
#[tokio::test]
async fn adopt_plan_drift_refused() {
    adopt_case("adopt_plan_drift_refused").await;
}
#[tokio::test]
async fn adopt_during_active_run() {
    adopt_case("adopt_during_active_run").await;
}
#[tokio::test]
async fn adopt_other_owner_refused() {
    adopt_case("adopt_other_owner_refused").await;
}
#[tokio::test]
async fn adopt_same_owner_readopts() {
    adopt_case("adopt_same_owner_readopts").await;
}
#[tokio::test]
async fn adopt_then_owner_sends() {
    adopt_case("adopt_then_owner_sends").await;
}

#[tokio::test]
async fn without_the_adopt_group_no_adoption_is_sent_and_its_cases_skip_by_name() {
    let subject = FakeSubject::new(Defects::default());
    let report = run(&subject).await;
    for name in ADOPT_CASES {
        assert_eq!(
            report.outcome(name),
            Some(&CaseOutcome::Skipped {
                missing: vec![Capability::Adopt]
            }),
            "{name}"
        );
        assert!(report.render().contains(&format!("SKIP {name}")));
    }
    assert_eq!(subject.adopt_calls(), 0, "{}", report.render());
}

#[tokio::test]
async fn an_adoption_that_writes_a_message_fails_the_adopt_cases() {
    let subject = adopting(Defects {
        adopt_writes_message: true,
        ..Defects::default()
    });
    let report = run(&subject).await;
    for name in [
        "adopt_binds_owner_without_a_run",
        "adopt_exact_repeat_replays_reply",
        "adopt_send_id_reuse_names_plan",
        "adopt_plan_drift_refused",
        "adopt_during_active_run",
        "adopt_other_owner_refused",
        "adopt_same_owner_readopts",
    ] {
        let reason = failed(&report, name);
        assert!(
            reason.contains("changed the transcript"),
            "{name}: {reason}"
        );
    }
}

#[tokio::test]
async fn an_adoption_taking_over_another_owner_fails_only_the_owner_case() {
    let subject = adopting(Defects {
        adopt_takes_over_owner: true,
        ..Defects::default()
    });
    let report = run(&subject).await;
    let reason = failed(&report, "adopt_other_owner_refused");
    assert!(reason.contains("was answered"), "{reason}");
    for name in ADOPT_CASES {
        if *name != "adopt_other_owner_refused" {
            assert_passed(&report, name);
        }
    }
}

/// A subject declaring `capability`, with `defects`.
fn declaring(capability: Capability, defects: Defects) -> FakeSubject {
    let mut subject = FakeSubject::new(defects);
    subject.capabilities.insert(capability);
    subject
}

/// A fake declaring `delivery_order` and `queue`, but not `steer`.
fn delivery_order_without_steer(defects: Defects) -> FakeSubject {
    let mut subject = declaring(Capability::DeliveryOrder, defects);
    subject.capabilities.remove(&Capability::Steer);
    subject
}

/// The cases a capability gates, by name prefix.
fn cases_of(prefixes: &[&str]) -> Vec<&'static str> {
    CASES
        .iter()
        .map(|spec| spec.name)
        .filter(|name| prefixes.iter().any(|prefix| name.starts_with(prefix)))
        .collect()
}

/// Run `capability`'s cases on a fake with `defects`: every case in
/// `failing` fails with a reason containing its text, and every other case
/// of the capability passes.
async fn only_these_fail(
    capability: Capability,
    prefixes: &[&str],
    defects: Defects,
    failing: &[(&str, &str)],
) {
    let report = run(&declaring(capability, defects)).await;
    for (name, text) in failing {
        let reason = failed(&report, name);
        assert!(reason.contains(text), "{name}: {reason}");
    }
    for name in cases_of(prefixes) {
        // A case that needs the capability left undeclared cannot be asked.
        if matches!(report.outcome(name), Some(CaseOutcome::Inapplicable { .. })) {
            continue;
        }
        if !failing.iter().any(|(failing, _)| *failing == name) {
            assert_passed(&report, name);
        }
    }
}

const TAG_PREFIXES: &[&str] = &["tags_"];
const TOOL_CALL_PREFIXES: &[&str] = &["run_tool_calls_"];
const ORDER_PREFIXES: &[&str] = &["delivery_order_"];
const USAGE_PREFIXES: &[&str] = &["run_usage_"];
const SESSION_USAGE_CASES: &[&str] = &[
    "session_usage_sums_runs_since_first",
    "session_usage_rewind_never_decreases",
    "session_usage_unknown_fields_absent",
    "head_model_reflects_recorded_switch",
];

async fn head_sender_defect(defects: Defects, expected: Option<&str>) {
    let mut subject = declaring(Capability::DeliveryOrder, defects);
    subject.head_sender_fixtures = true;
    let report = run(&subject).await;
    let name = "head_sender_is_callers_effective_grant";
    if let Some(reason) = expected {
        assert!(
            failed(&report, name).contains(reason),
            "{}",
            report.render()
        );
        assert_none_failed_but(&report, &[name]);
    } else {
        assert_eq!(
            report.outcome(name),
            Some(&CaseOutcome::Passed),
            "{}",
            report.render()
        );
        assert_none_failed_but(&report, &[]);
    }
}

#[tokio::test]
async fn head_sender_correct_runner_passes() {
    head_sender_defect(Defects::default(), None).await;
}

#[tokio::test]
async fn head_sender_omitted_everywhere_fails_by_name() {
    head_sender_defect(
        Defects {
            head_sender_omitted: true,
            ..Defects::default()
        },
        Some("A head.sender None"),
    )
    .await;
}

#[tokio::test]
async fn head_sender_undeclared_group_is_inapplicable_by_name() {
    let mut subject = declaring(Capability::DeliveryOrder, Defects::default());
    subject.head_sender_fixtures = true;
    subject.head_sender_group = false;
    let report = run(&subject).await;
    assert!(
        matches!(report.outcome("head_sender_is_callers_effective_grant"), Some(CaseOutcome::Inapplicable { reason }) if reason == "runner does not declare head_sender"),
        "{}",
        report.render()
    );
    assert_none_failed_but(&report, &[]);
}

#[tokio::test]
async fn head_sender_ignoring_caller_fails_by_name() {
    head_sender_defect(
        Defects {
            head_sender_ignores_caller: true,
            ..Defects::default()
        },
        Some("B head.sender"),
    )
    .await;
}

#[tokio::test]
async fn head_sender_absent_owner_fails_by_name() {
    head_sender_defect(
        Defects {
            head_sender_owner_absent: true,
            ..Defects::default()
        },
        Some("owner head.sender"),
    )
    .await;
}

#[tokio::test]
async fn head_sender_grant_for_unnamed_caller_fails_by_name() {
    head_sender_defect(
        Defects {
            head_sender_grant_for_unnamed_caller: true,
            ..Defects::default()
        },
        Some("unnamed caller head.sender"),
    )
    .await;
}

#[tokio::test]
async fn head_sender_ungranted_idle_start_fails_by_name() {
    head_sender_defect(
        Defects {
            head_sender_a_idle_start: true,
            ..Defects::default()
        },
        Some("A head.sender"),
    )
    .await;
}

#[tokio::test]
async fn head_sender_default_hook_is_inapplicable() {
    let report = run(&FakeSubject::new(Defects::default())).await;
    assert!(
        matches!(report.outcome("head_sender_is_callers_effective_grant"), Some(CaseOutcome::Inapplicable { reason }) if reason.contains("two-identity"))
    );
}

#[tokio::test]
async fn head_sender_respects_supported_mode_narrowing() {
    for modes in [vec!["idle_start".into()], vec!["steer".into()]] {
        let mut subject = declaring(Capability::DeliveryOrder, Defects::default());
        subject.head_sender_fixtures = true;
        subject.delivery_order_modes = Some(modes);
        let report = run(&subject).await;
        assert_eq!(
            report.outcome("head_sender_is_callers_effective_grant"),
            Some(&CaseOutcome::Passed),
            "{}",
            report.render()
        );
    }
}

async fn incarnation_defect(defects: Defects, expected: Option<&str>) {
    let mut subject = FakeSubject::new(defects);
    subject.incarnation_fixtures = true;
    let report = run(&subject).await;
    let name = "head_incarnation_survives_cut_and_changes_on_recreation";
    if let Some(reason) = expected {
        assert!(
            failed(&report, name).contains(reason),
            "{}",
            report.render()
        );
    } else {
        assert_eq!(
            report.outcome(name),
            Some(&CaseOutcome::Passed),
            "{}",
            report.render()
        );
    }
}

#[tokio::test]
async fn incarnation_correct_runner_passes() {
    incarnation_defect(Defects::default(), None).await;
}

#[tokio::test]
async fn incarnation_changed_at_cut_fails_by_name() {
    incarnation_defect(
        Defects {
            incarnation_changes_on_cut: true,
            ..Defects::default()
        },
        Some("cut changed incarnation"),
    )
    .await;
}

#[tokio::test]
async fn incarnation_reused_after_recreation_fails_by_name() {
    incarnation_defect(
        Defects {
            incarnation_reused_on_recreation: true,
            ..Defects::default()
        },
        Some("re-created log kept or omitted incarnation"),
    )
    .await;
}

#[tokio::test]
async fn incarnation_default_hooks_are_inapplicable() {
    let report = run(&FakeSubject::new(Defects::default())).await;
    assert!(
        matches!(report.outcome("head_incarnation_survives_cut_and_changes_on_recreation"), Some(CaseOutcome::Inapplicable { reason }) if reason == "adapter cannot cut the incarnation log")
    );
}

async fn session_usage_defect(defects: Defects, expected: Option<&str>) {
    let mut subject = declaring(Capability::SessionUsage, defects);
    subject.capabilities.insert(Capability::RunUsage);
    let report = run(&subject).await;
    for name in SESSION_USAGE_CASES {
        if Some(*name) == expected {
            failed(&report, name);
        } else {
            assert_eq!(
                report.outcome(name),
                Some(&CaseOutcome::Passed),
                "{name}: {}",
                report.render()
            );
        }
    }
}

#[tokio::test]
async fn session_usage_correct_runner_passes() {
    session_usage_defect(Defects::default(), None).await;
}

#[tokio::test]
async fn session_usage_wrong_sum_fails_by_name() {
    session_usage_defect(
        Defects {
            session_usage_sum_wrong: true,
            ..Defects::default()
        },
        Some("session_usage_sums_runs_since_first"),
    )
    .await;
}

#[tokio::test]
async fn session_usage_wrong_since_fails_by_name() {
    session_usage_defect(
        Defects {
            session_usage_since_wrong: true,
            ..Defects::default()
        },
        Some("session_usage_sums_runs_since_first"),
    )
    .await;
}

#[tokio::test]
async fn session_usage_reset_after_rewind_fails_by_name() {
    session_usage_defect(
        Defects {
            session_usage_reset_on_rewind: true,
            ..Defects::default()
        },
        Some("session_usage_rewind_never_decreases"),
    )
    .await;
}

#[tokio::test]
async fn session_usage_unknown_zero_fails_by_name() {
    session_usage_defect(
        Defects {
            session_usage_unknown_zero: true,
            ..Defects::default()
        },
        Some("session_usage_unknown_fields_absent"),
    )
    .await;
}

#[tokio::test]
async fn session_usage_plan_model_fails_by_name() {
    session_usage_defect(
        Defects {
            head_model_from_plan: true,
            ..Defects::default()
        },
        Some("head_model_reflects_recorded_switch"),
    )
    .await;
}
const SEND_STATUS_PREFIXES: &[&str] = &["send_status_"];

/// The steer-receipt case that follows a delivered steer by re-sending it.
const RESEND_CASE: &str = "delivery_order_steer_receipt_durable_across_restart";
/// The same steer followed through `send.status`.
const STATUS_CASE: &str = "send_status_answers_as_a_resend_after_restart";
const UNKNOWN_SEND_CASE: &str = "send_status_unknown_send_refused";

/// The step-receipt case for a runner that does not declare `run_ops`.
const NO_RUN_OPS_STEER_CASE: &str = "steer_without_run_ops_reports_state_and_step";
/// The ordered-list declaration case, which is inapplicable when `steer` is declared.
const UNDECLARED_MODE_CASE: &str = "delivery_order_undeclared_mode_refused";
const DECLARED_MODES_CASE: &str = "delivery_order_declared_modes_respected";

fn steer_only_lists(defects: Defects) -> FakeSubject {
    let mut subject = declaring(Capability::DeliveryOrder, defects);
    subject.delivery_order_modes = Some(vec!["steer".into()]);
    subject
}

#[tokio::test]
async fn a_runner_advertising_only_steer_lists_passes() {
    let report = run(&steer_only_lists(Defects::default())).await;
    assert_passed(&report, DECLARED_MODES_CASE);
    assert_none_failed_but(&report, &[]);
}

#[tokio::test]
async fn ignoring_delivery_order_modes_fails_by_name() {
    let report = run(&steer_only_lists(Defects {
        order_modes_ignored: true,
        ..Defects::default()
    }))
    .await;
    let reason = failed(&report, DECLARED_MODES_CASE);
    assert!(reason.contains("delivery_unsupported"), "{reason}");
    assert_none_failed_but(&report, &[DECLARED_MODES_CASE]);
}

#[tokio::test]
async fn delivery_order_modes_case_is_inapplicable_without_the_member() {
    let report = run(&declaring(Capability::DeliveryOrder, Defects::default())).await;
    assert!(
        matches!(report.outcome(DECLARED_MODES_CASE), Some(CaseOutcome::Inapplicable { reason })
        if reason.contains("delivery_order_modes"))
    );
}

/// A fake that does not declare `run_ops`.
fn without_run_ops(defects: Defects) -> FakeSubject {
    let mut subject = FakeSubject::new(defects);
    subject.capabilities.remove(&Capability::RunOps);
    subject
}

/// A fake declaring `delivery_order`, `send_status` and `run_ops`.
fn following(defects: Defects) -> FakeSubject {
    let mut subject = declaring(Capability::DeliveryOrder, defects);
    subject.capabilities.insert(Capability::SendStatus);
    subject.capabilities.insert(Capability::RunOps);
    subject
}

/// No case but those in `failing` failed. Unlike [`assert_others_passed`],
/// a case the subject's groups make inapplicable is allowed.
fn assert_none_failed_but(report: &SuiteReport, failing: &[&str]) {
    for spec in CASES {
        if !failing.contains(&spec.name) {
            assert!(
                !matches!(report.outcome(spec.name), Some(CaseOutcome::Failed { .. })),
                "{}: {:?}",
                spec.name,
                report.outcome(spec.name)
            );
        }
    }
}
#[tokio::test]
async fn a_runner_answering_delivered_steers_durably_passes_the_follow_cases() {
    let report = run(&following(Defects::default())).await;
    for name in [RESEND_CASE, STATUS_CASE, UNKNOWN_SEND_CASE] {
        assert_eq!(
            report.outcome(name),
            Some(&CaseOutcome::Passed),
            "{name}\n{}",
            report.render()
        );
    }
    assert_none_failed_but(&report, &[]);
}

/// `send.status` that answers `pending` after a restart for a steer
/// delivered before it: only the status case fails, by name; a re-send,
/// answered from the durable record, still passes.
#[tokio::test]
async fn send_status_answering_pending_after_a_restart_fails_only_the_status_case() {
    let report = run(&following(Defects {
        send_status_pending_after_restart: true,
        ..Defects::default()
    }))
    .await;
    let reason = failed(&report, STATUS_CASE);
    assert!(reason.contains("after the restart"), "{reason}");
    assert!(reason.contains("never pending again"), "{reason}");
    assert_none_failed_but(&report, &[STATUS_CASE]);
    assert_passed(&report, RESEND_CASE);
    assert_passed(&report, UNKNOWN_SEND_CASE);
}

/// A faithful runner without `run_ops` answers a steer with the turn state
/// and a step receipt, without a run id.
#[tokio::test]
async fn a_runner_without_run_ops_answers_with_run_state_and_step_receipt() {
    let report = run(&without_run_ops(Defects::default())).await;
    assert_passed(&report, NO_RUN_OPS_STEER_CASE);
    assert_none_failed_but(&report, &[]);
}

/// The fake defect supplies `state: "step"` on a delivered steer; only the
/// no-`run_ops` receipt case must fail, by its report name.
#[tokio::test]
async fn a_steer_reply_with_state_step_fails_only_the_no_run_ops_case() {
    let report = run(&without_run_ops(Defects {
        steer_reply_state_step: true,
        ..Defects::default()
    }))
    .await;
    let reason = failed(&report, NO_RUN_OPS_STEER_CASE);
    assert!(reason.contains("not active or finished"), "{reason}");
    assert_none_failed_but(&report, &[NO_RUN_OPS_STEER_CASE]);
}

/// A delivered steer answered without the run it joined while the fake
/// declares `run_ops`: the re-send case and the `send.status` case both fail
/// by name, each naming the joined run, and nothing else fails.
#[tokio::test]
async fn a_delivered_steer_without_its_run_while_declaring_run_ops_fails_both_follow_cases() {
    let report = run(&following(Defects {
        delivered_steer_without_run_id: true,
        ..Defects::default()
    }))
    .await;
    for name in [RESEND_CASE, STATUS_CASE] {
        let reason = failed(&report, name);
        // The case's session runs its opening turn as run-1 and the held
        // turn the steer joins as run-2.
        assert!(
            reason.contains("answered run_id None, not run-2, the run the steer joined"),
            "{name}: {reason}"
        );
    }
    assert_none_failed_but(&report, &[RESEND_CASE, STATUS_CASE]);
    assert_passed(&report, UNKNOWN_SEND_CASE);
}

#[tokio::test]
async fn without_send_status_its_cases_skip_by_name_and_the_resend_case_still_runs() {
    let report = run(&declaring(Capability::DeliveryOrder, Defects::default())).await;
    for name in cases_of(SEND_STATUS_PREFIXES) {
        assert_eq!(
            report.outcome(name),
            Some(&CaseOutcome::Skipped {
                missing: vec![Capability::SendStatus]
            }),
            "{name}"
        );
    }
    assert_eq!(report.outcome(RESEND_CASE), Some(&CaseOutcome::Passed));
}

/// The tags case reads the `StepRecorded` scenario: a seal that drops the
/// step moves the session to a new lineage, whose numbers may repeat the
/// dropped part's; resolve answers the part the new lineage shows.
#[tokio::test]
async fn a_dropped_step_resolves_in_the_new_lineage_and_resolving_old_content_fails_by_name() {
    for recovery in [StepRecovery::SealDropStep, StepRecovery::SealDropCall] {
        let mut subject = FakeSubject::new(Defects::default());
        subject.step_recovery = recovery;
        let report = run(&subject).await;
        assert_eq!(
            report.outcome(LEFT_LINEAGE_CASE),
            Some(&CaseOutcome::Passed),
            "{recovery:?}: {}",
            report.render()
        );
        assert_passed(&report, "crash_at_StepRecorded");
    }

    let mut subject = FakeSubject::new(Defects {
        resolve_left_lineage_tags: true,
        ..Defects::default()
    });
    subject.step_recovery = StepRecovery::SealDropStep;
    let report = run(&subject).await;
    let reason = failed(&report, LEFT_LINEAGE_CASE);
    assert!(
        reason.contains("never the one the session left"),
        "{reason}"
    );
    assert_passed(&report, "crash_at_StepRecorded");
    assert_passed(&report, "tags_resolve_unknown_tag_refused");
}

/// Without a seal that leaves a lineage, the left-lineage case has nothing
/// to ask and says so by name; without `tags` it is skipped.
#[tokio::test]
async fn without_a_new_lineage_the_left_lineage_case_is_inapplicable() {
    for recovery in [StepRecovery::Resume, StepRecovery::SealInterrupted] {
        let mut subject = FakeSubject::new(Defects::default());
        subject.step_recovery = recovery;
        let report = run(&subject).await;
        match report.outcome(LEFT_LINEAGE_CASE) {
            Some(CaseOutcome::Inapplicable { reason }) => {
                assert!(reason.contains("no lineage"), "{recovery:?}: {reason}")
            }
            other => panic!("{recovery:?}: {other:?}"),
        }
    }
    let mut subject = FakeSubject::new(Defects::default());
    subject.step_recovery = StepRecovery::SealDropStep;
    subject.capabilities.remove(&Capability::Compaction);
    subject.capabilities.remove(&Capability::TagCheckpoint);
    subject.capabilities.remove(&Capability::Tags);
    let report = run(&subject).await;
    assert_eq!(
        report.outcome(LEFT_LINEAGE_CASE),
        Some(&CaseOutcome::Skipped {
            missing: vec![Capability::Tags]
        })
    );
}

#[tokio::test]
async fn a_faithful_usage_runner_passes_the_usage_cases() {
    only_these_fail(
        Capability::RunUsage,
        USAGE_PREFIXES,
        Defects::default(),
        &[],
    )
    .await;
}

/// A runner that writes `reasoning_tokens: 0` where no step reported any: the
/// usage case must say the total is absent, never 0.
#[tokio::test]
async fn run_usage_total_nobody_reported_as_zero_fails_by_name() {
    only_these_fail(
        Capability::RunUsage,
        USAGE_PREFIXES,
        Defects {
            usage_absent_reasoning_as_zero: true,
            ..Defects::default()
        },
        &[(
            "run_usage_counts_reported_steps",
            "reports reasoning_tokens as 0, but the scripted model reported no reasoning_tokens",
        )],
    )
    .await;
}

/// Reasoning is a breakdown of output, never a count added to it: a runner
/// that bills it as extra output reports more output than the provider did,
/// and the usage case must say so by name.
#[tokio::test]
async fn run_usage_reasoning_billed_on_top_of_output_fails_by_name() {
    only_these_fail(
        Capability::RunUsage,
        USAGE_PREFIXES,
        Defects {
            usage_reasoning_added_to_output: true,
            ..Defects::default()
        },
        &[(
            "run_usage_counts_reported_steps",
            "output_tokens Some(39), but the scripted model reported Some(32)",
        )],
    )
    .await;
}

#[tokio::test]
async fn run_status_carries_the_same_usage_as_run_result() {
    let mut subject = declaring(Capability::RunUsage, Defects::default());
    subject.capabilities.insert(Capability::RunStatus);
    subject.advertise_status = true;
    let report = run(&subject).await;
    assert_passed(&report, "run_usage_counts_reported_steps");
}

#[tokio::test]
async fn a_tagging_runner_passes_the_tags_cases() {
    only_these_fail(Capability::Tags, TAG_PREFIXES, Defects::default(), &[]).await;
    let report = run(&declaring(Capability::Tags, Defects::default())).await;
    for name in [
        "tags_numbered_in_rendered_order",
        "tags_renumbering_waits_for_the_step_boundary",
        "tags_count_only_parts_rendered_with_a_number",
        "tags_setup_checkpoint_anchors_at_its_cut",
        "tags_fork_of_a_managed_session_refused",
        "tags_model_written_tags_cleaned_on_exposure",
    ] {
        assert!(
            matches!(
                report.outcome(name),
                Some(CaseOutcome::Passed | CaseOutcome::PassedWithInapplicableRule { .. })
            ),
            "{name}: {:?}",
            report.outcome(name)
        );
    }
}

/// The two cases that read which parts carry a number, each of which must
/// accept both layouts.
const PART_COUNTING_CASES: [&str; 2] = [
    "tags_on_every_read_entry",
    "tags_count_only_parts_rendered_with_a_number",
];

/// A tagging fake in an embedded layout, as OpenCode's: a call's result is
/// stored in the requesting entry's tool part, so that entry carries two
/// numbers, its text's and its tool part's. `tool_calls` also declares
/// `run_tool_calls`, so the case locates the result through `result_ordinal`
/// rather than the result's marker.
fn embedded_tags_runner(defects: Defects, tool_calls: bool) -> FakeSubject {
    let mut subject = declaring(
        Capability::Tags,
        Defects {
            run_tool_calls_embedded_results: true,
            ..defects
        },
    );
    if tool_calls {
        subject.capabilities.insert(Capability::RunToolCalls);
    }
    subject
}

#[tokio::test]
async fn an_embedded_layout_runner_passes_the_part_counting_cases() {
    for tool_calls in [false, true] {
        let report = run(&embedded_tags_runner(Defects::default(), tool_calls)).await;
        for name in PART_COUNTING_CASES {
            assert_eq!(
                report.outcome(name),
                Some(&CaseOutcome::Passed),
                "{name} (run_tool_calls declared: {tool_calls})\n{}",
                report.render()
            );
        }
    }
}

#[tokio::test]
async fn a_message_array_runner_declaring_run_tool_calls_passes_the_part_counting_cases() {
    let mut subject = declaring(Capability::Tags, Defects::default());
    subject.capabilities.insert(Capability::RunToolCalls);
    let report = run(&subject).await;
    for name in PART_COUNTING_CASES {
        assert_eq!(
            report.outcome(name),
            Some(&CaseOutcome::Passed),
            "{name}\n{}",
            report.render()
        );
    }
}

#[tokio::test]
async fn an_embedded_runner_leaving_its_result_part_unnumbered_fails_by_name() {
    for tool_calls in [false, true] {
        let defects = Defects {
            tags_embedded_result_unnumbered: true,
            ..Defects::default()
        };
        let report = run(&embedded_tags_runner(defects, tool_calls)).await;
        let reason = failed(&report, "tags_count_only_parts_rendered_with_a_number");
        assert!(
            reason.contains("holds the tool call's result and carries no number"),
            "{reason}"
        );
        assert_passed(&report, "tags_on_every_read_entry");
    }
}

/// A message-array runner stores a call's text result in a tool-result
/// entry of its own; leaving that entry unnumbered fails the counted-parts
/// case by name, whether the case finds the entry through `result_ordinal`
/// (`run_tool_calls` declared) or through the result's marker. The same
/// runner without the defect passes, naming the image rule it could not
/// check.
#[tokio::test]
async fn a_message_array_runner_leaving_its_result_entry_unnumbered_fails_by_name() {
    for tool_calls in [false, true] {
        for defective in [false, true] {
            let mut subject = declaring(
                Capability::Tags,
                Defects {
                    tags_tool_result_entry_unnumbered: defective,
                    ..Defects::default()
                },
            );
            subject.no_image_results = true;
            if tool_calls {
                subject.capabilities.insert(Capability::RunToolCalls);
            }
            let report = run(&subject).await;
            let name = "tags_count_only_parts_rendered_with_a_number";
            if defective {
                let reason = failed(&report, name);
                assert!(
                    reason.contains("holds the tool call's result and carries no number"),
                    "run_tool_calls declared: {tool_calls}: {reason}"
                );
            } else {
                assert!(
                    matches!(
                        report.outcome(name),
                        Some(CaseOutcome::PassedWithInapplicableRule { .. })
                    ),
                    "run_tool_calls declared: {tool_calls}: {:?}",
                    report.outcome(name)
                );
            }
        }
    }
}

/// Each defect fails its named rule. Restarting numbering every run also
/// breaks the lost-key case, whose setup spans runs; other tag cases pass.
#[tokio::test]
async fn each_tags_defect_fails_its_case_by_name() {
    let cases: &[(Defects, &str, &str)] = &[
        (
            Defects {
                tags_restart_each_run: true,
                ..Defects::default()
            },
            "tags_numbers_stable_within_a_lineage",
            "used twice",
        ),
        (
            Defects {
                tags_exposed_raw: true,
                ..Defects::default()
            },
            "tags_model_written_tags_cleaned_on_exposure",
            "cleaned on every exposure",
        ),
        (
            Defects {
                tags_numbered_in_storage_order: true,
                ..Defects::default()
            },
            "tags_numbered_in_rendered_order",
            "never in storage order",
        ),
        (
            Defects {
                tags_renumber_mid_step: true,
                ..Defects::default()
            },
            "tags_renumbering_waits_for_the_step_boundary",
            "still held",
        ),
        (
            Defects {
                tags_renumber_without_new_lineage: true,
                ..Defects::default()
            },
            "tags_renumbering_waits_for_the_step_boundary",
            "starts a new lineage",
        ),
        (
            Defects {
                tags_count_reasoning: true,
                ..Defects::default()
            },
            "tags_count_only_parts_rendered_with_a_number",
            "only its text counts",
        ),
        (
            Defects {
                tags_numbered_without_render_tags: true,
                ..Defects::default()
            },
            "tags_only_on_tag_rendering_sessions",
            "numbering applies only to sessions whose plan renders tags",
        ),
        (
            Defects {
                tags_setup_checkpoint_ignored: true,
                ..Defects::default()
            },
            "tags_setup_checkpoint_anchors_at_its_cut",
            "numbering starts after the checkpoint",
        ),
        (
            Defects {
                fork_accepted: true,
                ..Defects::default()
            },
            "tags_fork_of_a_managed_session_refused",
            "not refused",
        ),
        (
            Defects {
                tags_count_only_rendered_parts: true,
                ..Defects::default()
            },
            "tags_lost_marker_key_keeps_numbers",
            "fallback counts every part",
        ),
        (
            Defects {
                host_seam_not_cleaning: true,
                ..Defects::default()
            },
            "tags_host_store_cleaned_at_the_save_seam",
            "save seam",
        ),
    ];
    for (defects, case, text) in cases {
        let mut expected = vec![(*case, *text)];
        if defects.tags_restart_each_run {
            expected.push(("tags_lost_marker_key_keeps_numbers", "used twice"));
        }
        only_these_fail(Capability::Tags, TAG_PREFIXES, *defects, &expected).await;
    }
}

/// With `structured_output` declared, the cleaning case also checks that a
/// structured-output run's final text stays raw.
#[tokio::test]
async fn a_structured_output_runner_keeps_its_final_text_raw() {
    let mut subject = declaring(Capability::StructuredOutput, Defects::default());
    subject.capabilities.insert(Capability::Tags);
    let report = run(&subject).await;
    assert_eq!(
        report.outcome("tags_model_written_tags_cleaned_on_exposure"),
        Some(&CaseOutcome::Passed),
        "{}",
        report.render()
    );
}

/// A host turn that creates no run fails the adopt host-turn case by name.
#[tokio::test]
async fn host_turns_are_runs_and_one_without_a_run_fails_by_name() {
    let report = run(&declaring(Capability::Adopt, Defects::default())).await;
    assert_passed(&report, "adopt_host_turn_is_a_run");
    let report = run(&declaring(
        Capability::Adopt,
        Defects {
            host_turns_without_run: true,
            ..Defects::default()
        },
    ))
    .await;
    let reason = failed(&report, "adopt_host_turn_is_a_run");
    assert!(
        reason.contains("a turn the host starts is a run"),
        "{reason}"
    );
}

/// A gone part in a template's `part_tag` slot is refused by name.
#[tokio::test]
async fn a_template_naming_a_gone_part_is_refused_and_accepting_it_fails_by_name() {
    only_these_fail(
        Capability::DeliveryOrder,
        ORDER_PREFIXES,
        Defects::default(),
        &[],
    )
    .await;
    only_these_fail(
        Capability::DeliveryOrder,
        ORDER_PREFIXES,
        Defects {
            order_template_part_gone_accepted: true,
            ..Defects::default()
        },
        &[("delivery_order_template_part_gone_refused", "not refused")],
    )
    .await;
}

#[tokio::test]
async fn a_runner_listing_tool_calls_passes_and_reversed_order_fails_by_name() {
    only_these_fail(
        Capability::RunToolCalls,
        TOOL_CALL_PREFIXES,
        Defects::default(),
        &[],
    )
    .await;
    only_these_fail(
        Capability::RunToolCalls,
        TOOL_CALL_PREFIXES,
        Defects {
            run_tool_calls_reversed: true,
            ..Defects::default()
        },
        &[("run_tool_calls_listed_in_dispatch_order", "dispatch order")],
    )
    .await;
}

#[tokio::test]
async fn a_runner_honouring_delivery_order_passes_its_cases() {
    only_these_fail(
        Capability::DeliveryOrder,
        ORDER_PREFIXES,
        Defects::default(),
        &[],
    )
    .await;
}

#[tokio::test]
async fn a_delivery_order_runner_checks_declared_list_modes() {
    let report = run(&delivery_order_without_steer(Defects::default())).await;
    assert_passed(&report, UNDECLARED_MODE_CASE);
    assert_none_failed_but(&report, &[]);
}

#[tokio::test]
async fn queuing_a_list_with_no_declared_mode_fails_by_name() {
    let report = run(&delivery_order_without_steer(Defects {
        order_undeclared_modes_queued: true,
        ..Defects::default()
    }))
    .await;
    let reason = failed(&report, UNDECLARED_MODE_CASE);
    assert!(reason.contains("delivery_unsupported"), "{reason}");
    assert_none_failed_but(&report, &[UNDECLARED_MODE_CASE]);
}

#[tokio::test]
async fn a_delivery_order_send_pending_before_a_tool_result_can_be_retracted() {
    let mut subject = FakeSubject::new(Defects::default());
    subject.capabilities.insert(Capability::Retract);
    subject.capabilities.insert(Capability::DeliveryOrder);
    let report = run(&subject).await;
    assert_passed(&report, "delivery_order_retract_pending_tool_result");
    assert_none_failed_but(&report, &[]);
}

#[tokio::test]
async fn without_turn_system_a_template_goes_out_as_the_same_text_at_user_authority() {
    let mut subject = declaring(Capability::DeliveryOrder, Defects::default());
    subject.turn_system = false;
    let report = run(&subject).await;
    for name in cases_of(ORDER_PREFIXES) {
        if !matches!(report.outcome(name), Some(CaseOutcome::Inapplicable { .. })) {
            assert_passed(&report, name);
        }
    }
}

#[tokio::test]
async fn a_runner_without_the_group_refuses_a_list_and_one_accepting_it_fails_by_name() {
    let report = run(&FakeSubject::new(Defects::default())).await;
    assert_passed(&report, "delivery_order_list_refused_without_group");
    let report = run(&FakeSubject::new(Defects {
        order_list_accepted_without_group: true,
        ..Defects::default()
    }))
    .await;
    let reason = failed(&report, "delivery_order_list_refused_without_group");
    assert!(reason.contains("not refused"), "{reason}");
    assert_others_passed(&report, &["delivery_order_list_refused_without_group"]);
}

#[tokio::test]
async fn a_template_send_with_a_string_delivery_accepted_fails_by_name() {
    only_these_fail(
        Capability::DeliveryOrder,
        ORDER_PREFIXES,
        Defects {
            order_members_without_list_accepted: true,
            ..Defects::default()
        },
        &[("delivery_order_members_without_list_refused", "not refused")],
    )
    .await;
}

#[tokio::test]
async fn a_list_without_idle_start_that_starts_a_run_fails_the_never_starts_case() {
    only_these_fail(
        Capability::DeliveryOrder,
        ORDER_PREFIXES,
        Defects {
            order_starts_runs_without_idle_start: true,
            ..Defects::default()
        },
        // Every other case that queues a steer on an idle session sees it
        // start a run too, and fails on what that run delivered.
        &[
            ("delivery_order_never_starts_a_run", "never start a run"),
            ("delivery_order_dedupe_supersedes_queued", "not superseded"),
            ("delivery_order_unnamed_sender_refused", "not steer"),
            (
                "delivery_order_template_renders_as_system",
                "not turn_system",
            ),
        ],
    )
    .await;
}

#[tokio::test]
async fn an_out_of_range_slot_rendered_as_system_fails_the_bad_slot_case() {
    only_these_fail(
        Capability::DeliveryOrder,
        ORDER_PREFIXES,
        Defects {
            order_template_slots_unchecked: true,
            ..Defects::default()
        },
        &[("delivery_order_bad_slot_refused", "not refused")],
    )
    .await;
}

#[tokio::test]
async fn an_unnamed_sender_accepted_fails_the_sender_case() {
    only_these_fail(
        Capability::DeliveryOrder,
        ORDER_PREFIXES,
        Defects {
            order_admits_any_sender: true,
            ..Defects::default()
        },
        &[("delivery_order_unnamed_sender_refused", "not refused")],
    )
    .await;
}

#[tokio::test]
async fn delivery_order_defects_each_fail_their_case() {
    for (defects, name, text) in [
        (
            Defects {
                order_dedupe_keeps_both: true,
                ..Defects::default()
            },
            "delivery_order_dedupe_supersedes_queued",
            "superseded",
        ),
        (
            Defects {
                order_expiry_ignored: true,
                ..Defects::default()
            },
            "delivery_order_expired_never_delivered",
            "expires_at_ms",
        ),
        (
            Defects {
                order_members_without_list_accepted: true,
                ..Defects::default()
            },
            "delivery_order_members_without_list_refused",
            "not refused",
        ),
    ] {
        only_these_fail(
            Capability::DeliveryOrder,
            ORDER_PREFIXES,
            defects,
            &[(name, text)],
        )
        .await;
    }
}

#[tokio::test]
async fn the_last_possible_mode_winning_fails_the_order_cases() {
    // Wherever two modes are possible at one boundary, the defect picks the
    // later one: the held tool result's boundary, and the template's.
    only_these_fail(
        Capability::DeliveryOrder,
        ORDER_PREFIXES,
        Defects {
            order_last_possible_mode: true,
            ..Defects::default()
        },
        &[
            ("delivery_order_first_possible_mode", "tool_result"),
            (
                "delivery_order_template_renders_as_system",
                "not turn_system",
            ),
        ],
    )
    .await;
}

#[tokio::test]
async fn without_the_new_groups_their_cases_skip_by_name() {
    let report = run(&FakeSubject::new(Defects::default())).await;
    // The default fake declares `tags`, which `compaction` requires, so
    // the optional groups below are left undeclared here.
    for (capability, prefixes) in [
        (Capability::RunToolCalls, TOOL_CALL_PREFIXES),
        (Capability::RunUsage, USAGE_PREFIXES),
        (Capability::SessionUsage, &["session_usage_"][..]),
        (Capability::DeliveryOrder, ORDER_PREFIXES),
        (Capability::SendStatus, SEND_STATUS_PREFIXES),
    ] {
        // The one case that asks a runner without the group runs here.
        for name in cases_of(prefixes)
            .into_iter()
            .filter(|name| *name != "delivery_order_list_refused_without_group")
        {
            assert!(
                matches!(report.outcome(name), Some(CaseOutcome::Skipped { missing }) if missing.contains(&capability)),
                "{name}: {:?}",
                report.outcome(name)
            );
        }
    }
}

#[tokio::test]
async fn a_ceiling_wait_reads_as_waiting_and_ends_either_way() {
    let mut subject = FakeSubject::new(Defects::default());
    subject.advertise_status = true;
    let report = run(&subject).await;
    assert_passed(&report, "run_context_ceiling_wait_is_not_a_stall");
}

#[tokio::test]
async fn a_read_serving_live_host_metadata_fails_the_stability_case() {
    let report = run(&FakeSubject::new(Defects {
        read_serves_live_host_metadata: true,
        ..Defects::default()
    }))
    .await;
    let reason = failed(&report, "read_stable_across_host_metadata_change");
    assert!(reason.contains("other entries"), "{reason}");
}

#[tokio::test]
async fn a_drop_recovery_keeping_its_lineage_fails_the_step_recorded_crash_case() {
    for recovery in [StepRecovery::SealDropStep, StepRecovery::SealDropCall] {
        let mut subject = FakeSubject::new(Defects {
            drop_keeps_lineage: true,
            ..Defects::default()
        });
        subject.step_recovery = recovery;
        let report = run(&subject).await;
        let reason = failed(&report, "crash_at_StepRecorded");
        assert!(reason.contains("new lineage"), "{recovery:?}: {reason}");
    }
}

#[tokio::test]
async fn a_template_send_carrying_an_empty_prompt_accepted_fails_by_name() {
    only_these_fail(
        Capability::DeliveryOrder,
        ORDER_PREFIXES,
        Defects {
            order_template_with_empty_prompt_accepted: true,
            ..Defects::default()
        },
        &[("delivery_order_template_renders_as_system", "not refused")],
    )
    .await;
}

const RUN_PARAMETER_CASES: &[&str] = &[
    "max_steps_seals_pending_calls",
    "max_output_tokens_caps_response",
    "no_tools_offered_to_model",
    "no_tools_later_plan_send_refused",
];

fn run_parameters(defects: Defects) -> FakeSubject {
    let mut subject = FakeSubject::new(defects);
    subject.capabilities.extend([
        Capability::MaxSteps,
        Capability::MaxOutputTokens,
        Capability::NoTools,
        Capability::RunUsage,
    ]);
    subject
}

#[tokio::test]
async fn declared_run_parameters_pass_all_four_cases() {
    let report = run(&run_parameters(Defects::default())).await;
    for name in RUN_PARAMETER_CASES {
        assert_eq!(
            report.outcome(name),
            Some(&CaseOutcome::Passed),
            "{name}: {}",
            report.render()
        );
    }
    assert_none_failed_but(&report, &[]);
}

#[tokio::test]
async fn undeclared_run_parameters_skip_each_case_by_name() {
    let report = run(&FakeSubject::new(Defects::default())).await;
    for (name, missing) in [
        (RUN_PARAMETER_CASES[0], Capability::MaxSteps),
        (RUN_PARAMETER_CASES[1], Capability::MaxOutputTokens),
        (RUN_PARAMETER_CASES[2], Capability::NoTools),
        (RUN_PARAMETER_CASES[3], Capability::NoTools),
    ] {
        assert!(
            matches!(report.outcome(name), Some(CaseOutcome::Skipped { missing: names }) if names.contains(&missing)),
            "{name}: {:?}",
            report.outcome(name)
        );
        assert!(report.render().contains(&format!("SKIP {name}")));
    }
}

async fn run_parameter_break(defects: Defects, expected: &str, message: &str) {
    let report = run(&run_parameters(defects)).await;
    let reason = failed(&report, expected);
    assert!(reason.contains(message), "{expected}: {reason}");
    assert_none_failed_but(&report, &[expected]);
    for name in RUN_PARAMETER_CASES {
        if *name != expected {
            assert_passed(&report, name);
        }
    }
    println!("NON-VACUITY BREAK {expected}: {reason}; other run-parameter cases passed");
}

#[tokio::test]
async fn ignoring_max_steps_fails_only_its_named_case() {
    run_parameter_break(
        Defects {
            max_steps_ignored: true,
            ..Defects::default()
        },
        RUN_PARAMETER_CASES[0],
        "expected max_steps",
    )
    .await;
}

#[tokio::test]
async fn unsealed_max_steps_fails_only_its_named_case() {
    run_parameter_break(
        Defects {
            max_steps_unsealed: true,
            ..Defects::default()
        },
        RUN_PARAMETER_CASES[0],
        "never-executed result",
    )
    .await;
}

#[tokio::test]
async fn ignoring_max_output_tokens_fails_only_its_named_case() {
    run_parameter_break(
        Defects {
            max_output_tokens_ignored: true,
            ..Defects::default()
        },
        RUN_PARAMETER_CASES[1],
        "response exceeds cap",
    )
    .await;
}

#[tokio::test]
async fn ignoring_no_tools_fails_only_its_named_case() {
    run_parameter_break(
        Defects {
            no_tools_ignored: true,
            ..Defects::default()
        },
        RUN_PARAMETER_CASES[2],
        "suppression=true",
    )
    .await;
}

#[tokio::test]
async fn ignoring_later_plan_tools_fails_only_its_named_case() {
    run_parameter_break(
        Defects {
            late_plan_tools_ignored: true,
            ..Defects::default()
        },
        RUN_PARAMETER_CASES[3],
        "late plan tools",
    )
    .await;
}

async fn run_parameter_case(name: &str) {
    let report = run(&run_parameters(Defects::default())).await;
    assert_eq!(
        report.outcome(name),
        Some(&CaseOutcome::Passed),
        "{name}: {:?}",
        report.outcome(name)
    );
}

#[tokio::test]
async fn run_parameter_case_max_steps_seals_pending_calls() {
    run_parameter_case(RUN_PARAMETER_CASES[0]).await;
}

#[tokio::test]
async fn run_parameter_case_max_output_tokens_caps_response() {
    run_parameter_case(RUN_PARAMETER_CASES[1]).await;
}

#[tokio::test]
async fn run_parameter_case_no_tools_offered_to_model() {
    run_parameter_case(RUN_PARAMETER_CASES[2]).await;
}

#[tokio::test]
async fn run_parameter_case_no_tools_later_plan_send_refused() {
    run_parameter_case(RUN_PARAMETER_CASES[3]).await;
}

#[tokio::test]
async fn no_tools_without_plan_session_support_skips_later_send_case_by_name() {
    let mut subject = declaring(Capability::NoTools, Defects::default());
    subject.without_plans = true;
    let report = run(&subject).await;
    let name = RUN_PARAMETER_CASES[3];
    assert!(
        matches!(report.outcome(name), Some(CaseOutcome::Inapplicable { reason }) if reason.contains("cannot create a fetch-plan session")),
        "{name}: {:?}",
        report.outcome(name)
    );
    assert!(report.render().contains(name));
}

const DIRECTIVE_CASES: &[&str] = &[
    "turn_directives_in",
    "turn_directives_note",
    "turn_directives_off",
    "turn_directives_max_steps_reports_none",
    "turn_directives_cancelled_reports_none",
    "turn_directives_not_requested_reports_none",
    "turn_directives_unknown_refused",
];

fn directing(defect: DirectiveDefect) -> FakeSubject {
    let mut subject = FakeSubject::new(Defects {
        directives: defect,
        ..Defects::default()
    });
    subject
        .capabilities
        .extend([Capability::TurnDirectives, Capability::MaxSteps]);
    subject
}

#[tokio::test]
async fn turn_directives_declared_cases_pass() {
    let report = run(&directing(DirectiveDefect::None)).await;
    for name in DIRECTIVE_CASES {
        assert_eq!(
            report.outcome(name),
            Some(&CaseOutcome::Passed),
            "{name}: {}",
            report.render()
        );
    }
    assert_none_failed_but(&report, &[]);
}

#[tokio::test]
async fn turn_directives_undeclared_skips_by_name() {
    let report = run(&FakeSubject::new(Defects::default())).await;
    for name in DIRECTIVE_CASES {
        assert!(
            matches!(report.outcome(name), Some(CaseOutcome::Skipped { missing }) if missing.contains(&Capability::TurnDirectives)),
            "{name}: {:?}",
            report.outcome(name)
        );
        assert!(report.render().contains(&format!("SKIP {name}")));
    }
}

#[tokio::test]
async fn turn_directives_each_defect_fails_only_its_named_case() {
    for (defect, name) in [
        (DirectiveDefect::In, DIRECTIVE_CASES[0]),
        (DirectiveDefect::Note, DIRECTIVE_CASES[1]),
        (DirectiveDefect::Off, DIRECTIVE_CASES[2]),
        (DirectiveDefect::MaxSteps, DIRECTIVE_CASES[3]),
        (DirectiveDefect::Cancelled, DIRECTIVE_CASES[4]),
        (DirectiveDefect::Unrequested, DIRECTIVE_CASES[5]),
        (DirectiveDefect::Unknown, DIRECTIVE_CASES[6]),
    ] {
        let report = run(&directing(defect)).await;
        let reason = failed(&report, name);
        assert_none_failed_but(&report, &[name]);
        for other in DIRECTIVE_CASES {
            if *other != name {
                assert_eq!(
                    report.outcome(other),
                    Some(&CaseOutcome::Passed),
                    "{other}: {}",
                    report.render()
                );
            }
        }
        println!("NON-VACUITY BREAK {name}: {reason}; other directive cases passed");
    }
}

macro_rules! directive_case {
    ($test:ident, $case:literal) => {
        #[tokio::test]
        async fn $test() {
            let report = run(&directing(DirectiveDefect::None)).await;
            assert_eq!(
                report.outcome($case),
                Some(&CaseOutcome::Passed),
                "{}",
                report.render()
            );
        }
    };
}
directive_case!(turn_directives_case_in, "turn_directives_in");
directive_case!(turn_directives_case_note, "turn_directives_note");
directive_case!(turn_directives_case_off, "turn_directives_off");
directive_case!(
    turn_directives_case_max_steps_reports_none,
    "turn_directives_max_steps_reports_none"
);
directive_case!(
    turn_directives_case_cancelled_reports_none,
    "turn_directives_cancelled_reports_none"
);
directive_case!(
    turn_directives_case_not_requested_reports_none,
    "turn_directives_not_requested_reports_none"
);
directive_case!(
    turn_directives_case_unknown_refused,
    "turn_directives_unknown_refused"
);

#[tokio::test]
async fn run_tool_calls_new_locations_pass() {
    let report = run(&declaring(Capability::RunToolCalls, Defects::default())).await;
    for name in [
        "run_tool_calls_positions_identify_siblings",
        "run_tool_calls_result_ordinal_tracks_storage",
    ] {
        assert_eq!(
            report.outcome(name),
            Some(&CaseOutcome::Passed),
            "{}",
            report.render()
        );
    }
    assert_none_failed_but(&report, &[]);
}

#[tokio::test]
async fn run_tool_calls_embedded_layout_passes() {
    let report = run(&declaring(
        Capability::RunToolCalls,
        Defects {
            run_tool_calls_embedded_results: true,
            ..Defects::default()
        },
    ))
    .await;
    for name in cases_of(TOOL_CALL_PREFIXES) {
        assert_passed(&report, name);
    }
}

#[tokio::test]
async fn run_tool_calls_missing_request_arguments_after_release_fails_by_name() {
    let report = run(&declaring(
        Capability::RunToolCalls,
        Defects {
            run_tool_calls_omit_finished_arguments: true,
            ..Defects::default()
        },
    ))
    .await;
    let reason = failed(&report, "run_tool_calls_result_ordinal_tracks_storage");
    assert!(
        reason.contains("run.tool_calls names ordinal")
            && reason.contains("but that message does not hold them"),
        "{reason}"
    );
}

#[tokio::test]
async fn run_tool_calls_open_message_without_arguments_fails_by_name() {
    only_these_fail(
        Capability::RunToolCalls,
        TOOL_CALL_PREFIXES,
        Defects {
            run_tool_calls_open_message_without_arguments: true,
            ..Defects::default()
        },
        &[(
            "run_tool_calls_result_ordinal_tracks_storage",
            "but that message does not hold them",
        )],
    )
    .await;
}

#[tokio::test]
async fn swapped_tool_call_positions_fail_by_name() {
    only_these_fail(
        Capability::RunToolCalls,
        TOOL_CALL_PREFIXES,
        Defects {
            run_tool_calls_positions_swapped: true,
            ..Defects::default()
        },
        &[("run_tool_calls_positions_identify_siblings", "positions")],
    )
    .await;
}

#[tokio::test]
async fn open_tool_call_result_ordinal_fails_by_name() {
    only_these_fail(
        Capability::RunToolCalls,
        TOOL_CALL_PREFIXES,
        Defects {
            run_tool_calls_open_result_ordinal: true,
            ..Defects::default()
        },
        &[(
            "run_tool_calls_result_ordinal_tracks_storage",
            "no result_ordinal",
        )],
    )
    .await;
}

#[tokio::test]
async fn interrupt_reason_shutdown_survives_restart() {
    let report = run(&declaring(Capability::InterruptReason, Defects::default())).await;
    assert_eq!(
        report.outcome("interrupt_reason_shutdown_durable"),
        Some(&CaseOutcome::Passed),
        "{}",
        report.render()
    );
    assert_none_failed_but(&report, &[]);
}

#[tokio::test]
async fn interrupted_shutdown_without_reason_fails_by_name() {
    only_these_fail(
        Capability::InterruptReason,
        &["interrupt_reason_"],
        Defects {
            interrupt_shutdown_reason_missing: true,
            ..Defects::default()
        },
        &[("interrupt_reason_shutdown_durable", "runner_shutdown")],
    )
    .await;
}

#[tokio::test]
async fn interrupt_reason_undeclared_skips_by_name() {
    let report = run(&FakeSubject::new(Defects::default())).await;
    assert!(
        matches!(report.outcome("interrupt_reason_shutdown_durable"), Some(CaseOutcome::Skipped { missing }) if missing == &[Capability::InterruptReason])
    );
}

fn command_subject(defect: Option<&'static str>) -> FakeSubject {
    let mut subject = FakeSubject::new(Defects {
        user_command: defect,
        ..Defects::default()
    });
    subject.capabilities.insert(Capability::UserCommands);
    subject
}

#[tokio::test]
async fn user_commands_keep_replies_out_of_history() {
    let report = run(&command_subject(None)).await;
    for case in CASES
        .iter()
        .filter(|case| case.name.starts_with("user_commands_"))
    {
        assert_passed(&report, case.name);
    }
}

#[tokio::test]
async fn user_command_faults_fail_by_name() {
    for (defect, case) in [
        ("history", "user_commands_history_isolation"),
        ("duplicate", "user_commands_forward_once"),
        ("no_person", "user_commands_no_person"),
        ("collision", "user_commands_collision"),
        ("progress", "user_commands_progress_order"),
        ("unsafe_host", "user_commands_unsafe_host_menu"),
        ("owner", "user_commands_owner_only"),
    ] {
        failed(&run(&command_subject(Some(defect))).await, case);
    }
}

#[tokio::test]
async fn user_command_crash_faults_fail_by_name() {
    for defect in ["retry_after_crash", "report_failed"] {
        failed(
            &run(&command_subject(Some(defect))).await,
            "user_commands_outcome_unknown",
        );
    }
}

#[tokio::test]
async fn standalone_refusal_hands_off_only_before_service() {
    let mut subject = FakeSubject::new(Defects::default());
    subject.standalone_in_process = true;
    assert_passed(
        &run(&subject).await,
        "compaction_standalone_refusal_lifecycle",
    );
    for defect in [
        CompactionDefect::StandaloneNoHandoff,
        CompactionDefect::StandaloneLateHandoff,
    ] {
        let mut subject = FakeSubject::new(Defects {
            compaction: defect,
            ..Defects::default()
        });
        subject.standalone_in_process = true;
        let report = run(&subject).await;
        let reason = failed(&report, "compaction_standalone_refusal_lifecycle");
        assert!(reason.contains("standalone"), "{reason}");
    }
}

#[tokio::test]
async fn user_command_cutoff_defect_fails_after_forward_by_name() {
    let report = run(&command_subject(Some("cutoff_failed"))).await;
    let reason = failed(&report, "user_commands_cutoff_after_forward");
    assert!(reason.contains("outcome_unknown"), "{reason}");
    assert_passed(&report, "user_commands_cutoff_before_forward");
}

#[tokio::test]
async fn descriptor_defects_fail_their_cases_by_name() {
    for (defect, case) in [
        ("base64", "media_descriptors_image_bytes"),
        ("nested_base64", "media_descriptors_nested_media"),
        ("head_boundary", "media_descriptors_utf8_head"),
        ("pairing", "media_descriptors_pairing"),
        ("hash_descriptor", "media_descriptors_full_content_digests"),
        ("part_key", "media_descriptors_part_keys"),
        (
            "model_view_descriptors",
            "media_descriptors_model_view_refused",
        ),
        ("page_position", "media_descriptors_stable_across_pages"),
        ("smallest_first", "media_descriptors_frame_budget"),
    ] {
        let mut subject = FakeSubject::new(Defects {
            media: Some(defect),
            ..Defects::default()
        });
        subject.descriptor_fixtures = true;
        subject.descriptor_case = Some(case);
        let report = run(&subject).await;
        let reason = failed(&report, case);
        if defect == "smallest_first" {
            assert!(
                reason.contains("largest-first") && reason.contains("[0, 2, 4]"),
                "{reason}"
            );
        }
        eprintln!("{defect}: {case} failed: {reason}");
        assert_passed(&report, "tags_numbers_stable_within_a_lineage");
    }
}

/// The pairing case's call carries arguments over the inline limit with
/// top-level `id` and `name` members, which a descriptors read replaces
/// with an `args_ref`; a faithful runner keeps every pairing field and
/// passes.
#[tokio::test]
async fn described_arguments_with_id_and_name_keep_their_pairing() {
    let mut subject = FakeSubject::new(Defects::default());
    subject.descriptor_fixtures = true;
    subject.descriptor_case = Some("media_descriptors_pairing");
    let report = run(&subject).await;
    assert_passed(&report, "media_descriptors_pairing");
}

#[tokio::test]
async fn mixed_size_fallback_frame_budget_case() {
    let mut subject = FakeSubject::new(Defects::default());
    subject.descriptor_fixtures = true;
    subject.descriptor_case = Some("media_descriptors_frame_budget");
    let report = run(&subject).await;
    assert_passed(&report, "media_descriptors_frame_budget");
}

/// These two controls use the production tag primitives through the fake's
/// live route, not a second implementation of either rule.
#[tokio::test]
async fn shared_tag_rules_control_keeps_structured_output_raw() {
    let mut subject = declaring(Capability::StructuredOutput, Defects::default());
    subject.capabilities.insert(Capability::Tags);
    let report = run(&subject).await;
    assert_passed(&report, "tags_model_written_tags_cleaned_on_exposure");
}

#[tokio::test]
async fn shared_tag_rules_control_numbers_in_rendered_order() {
    let report = run(&declaring(Capability::Tags, Defects::default())).await;
    assert_passed(&report, "tags_numbered_in_rendered_order");
}

/// The cases where a step reaches `max_steps` without the limit cutting
/// anything off.
const FINAL_STEP_CASES: &[&str] = &[
    "max_steps_final_step_completes",
    "max_steps_submit_result_completes",
];

fn final_steps(defects: Defects) -> FakeSubject {
    let mut subject = FakeSubject::new(defects);
    subject.capabilities.extend([
        Capability::MaxSteps,
        Capability::StructuredOutput,
        Capability::TurnDirectives,
    ]);
    subject
}

#[tokio::test]
async fn a_limiting_step_without_a_further_step_completes() {
    let report = run(&final_steps(Defects::default())).await;
    for name in FINAL_STEP_CASES.iter().chain(&[
        "max_steps_seals_pending_calls",
        "turn_directives_max_steps_reports_none",
    ]) {
        assert_eq!(
            report.outcome(name),
            Some(&CaseOutcome::Passed),
            "{name}: {}",
            report.render()
        );
    }
    assert_none_failed_but(&report, &[]);
}

#[tokio::test]
async fn ending_a_final_step_max_steps_fails_both_cases_by_name() {
    let report = run(&final_steps(Defects {
        max_steps_on_final_step: true,
        ..Defects::default()
    }))
    .await;
    for name in FINAL_STEP_CASES {
        let reason = failed(&report, name);
        assert!(
            reason.contains("must end the run completed"),
            "{name}: {reason}"
        );
        println!("NON-VACUITY BREAK {name}: {reason}");
    }
    assert_none_failed_but(&report, FINAL_STEP_CASES);
    assert_passed(&report, "max_steps_seals_pending_calls");
    assert_passed(&report, "turn_directives_max_steps_reports_none");
}

#[tokio::test]
async fn the_final_step_cases_skip_by_name_without_their_groups() {
    let report = run(&FakeSubject::new(Defects::default())).await;
    for name in FINAL_STEP_CASES {
        assert!(
            matches!(report.outcome(name), Some(CaseOutcome::Skipped { missing }) if missing.contains(&Capability::MaxSteps)),
            "{name}: {:?}",
            report.outcome(name)
        );
    }
}

/// The `session_change` cases.
const SESSION_CHANGE_CASES: &[&str] = &[
    "session_refresh_applies_from_next_run",
    "session_refresh_waits_for_its_rung",
    "session_refresh_never_changes_a_running_run",
    "session_refresh_repeat_same_answer",
    "session_refresh_generation_stale_refused",
    "session_refresh_tool_outside_host_set_refused",
    "flush_prefix_applies_pending",
];

fn changing(defects: Defects) -> FakeSubject {
    declaring(Capability::SessionChange, defects)
}

#[tokio::test]
async fn a_session_change_runner_passes_every_session_change_case() {
    let report = run(&changing(Defects::default())).await;
    for name in SESSION_CHANGE_CASES {
        assert_eq!(
            report.outcome(name),
            Some(&CaseOutcome::Passed),
            "{name}: {}",
            report.render()
        );
    }
    assert_none_failed_but(&report, &[]);
}

/// A session stating only the `immediately` rung applies every change at the
/// next run with or without a flush, so the flush case can't see the flush's
/// own effect: it still checks the next-run rule, passes, and names the rest
/// unchecked, and the case that needs a waiting rung is inapplicable by name.
#[tokio::test]
async fn a_session_stating_only_immediately_names_the_flush_effect_unchecked() {
    let report = run(&changing(Defects {
        states_only_immediately: true,
        ..Defects::default()
    }))
    .await;
    assert!(
        matches!(
            report.outcome("flush_prefix_applies_pending"),
            Some(CaseOutcome::PassedWithInapplicableRule { reason })
                if reason.contains("only the immediately rung")
        ),
        "{}",
        report.render()
    );
    assert!(
        matches!(
            report.outcome("session_refresh_waits_for_its_rung"),
            Some(CaseOutcome::Inapplicable { .. })
        ),
        "{}",
        report.render()
    );
    assert_none_failed_but(&report, &[]);
}

#[tokio::test]
async fn session_change_cases_skip_by_name_without_the_group() {
    let report = run(&FakeSubject::new(Defects::default())).await;
    for name in SESSION_CHANGE_CASES {
        assert!(
            matches!(report.outcome(name), Some(CaseOutcome::Skipped { missing }) if missing.contains(&Capability::SessionChange)),
            "{name}: {:?}",
            report.outcome(name)
        );
        assert!(report.render().contains(&format!("SKIP {name}")));
    }
}

/// Each faulty runner fails exactly the named cases, each with a reason
/// holding `message`, and passes every other session_change case.
async fn session_change_break(defects: Defects, expected: &[&str], message: &str) {
    let report = run(&changing(defects)).await;
    for name in expected {
        let reason = failed(&report, name);
        assert!(reason.contains(message), "{name}: {reason}");
        println!("NON-VACUITY BREAK {name}: {reason}");
    }
    assert_none_failed_but(&report, expected);
    for name in SESSION_CHANGE_CASES {
        if !expected.contains(name) {
            assert_passed(&report, name);
        }
    }
}

#[tokio::test]
async fn a_refresh_applied_inside_a_running_run_fails_the_next_run_rule() {
    session_change_break(
        Defects {
            refresh_applies_mid_run: true,
            ..Defects::default()
        },
        &["session_refresh_never_changes_a_running_run"],
        "the change applied inside a running run",
    )
    .await;
}

#[tokio::test]
async fn a_flush_applied_inside_a_running_run_fails_the_flush_case() {
    session_change_break(
        Defects {
            flush_applies_mid_run: true,
            ..Defects::default()
        },
        &["flush_prefix_applies_pending"],
        "the change applied inside a running run",
    )
    .await;
}

#[tokio::test]
async fn a_flush_that_never_applies_fails_the_flush_case() {
    session_change_break(
        Defects {
            flush_ignored: true,
            ..Defects::default()
        },
        &["flush_prefix_applies_pending"],
        "not the new plan's tools",
    )
    .await;
}

#[tokio::test]
async fn hiding_the_running_run_fails_the_held_cases() {
    session_change_break(
        Defects {
            refresh_hides_running_run: true,
            ..Defects::default()
        },
        &[
            "session_refresh_never_changes_a_running_run",
            "flush_prefix_applies_pending",
        ],
        "run_in_progress: false",
    )
    .await;
}

#[tokio::test]
async fn claiming_the_next_run_for_a_waiting_rung_fails_the_rung_case() {
    session_change_break(
        Defects {
            refresh_claims_next_run: true,
            ..Defects::default()
        },
        &["session_refresh_waits_for_its_rung"],
        "applies_from_next_run: true",
    )
    .await;
}

#[tokio::test]
async fn an_immediately_rung_that_waits_fails_the_next_run_cases() {
    session_change_break(
        Defects {
            immediately_waits_for_rebuild: true,
            ..Defects::default()
        },
        &[
            "session_refresh_applies_from_next_run",
            "session_refresh_never_changes_a_running_run",
        ],
        "offered",
    )
    .await;
}

#[tokio::test]
async fn refusing_an_identical_repeat_fails_the_repeat_case() {
    session_change_break(
        Defects {
            refresh_repeat_refused: true,
            ..Defects::default()
        },
        &["session_refresh_repeat_same_answer"],
        "generation_stale",
    )
    .await;
}

#[tokio::test]
async fn accepting_a_stale_generation_fails_the_stale_case() {
    session_change_break(
        Defects {
            refresh_stale_accepted: true,
            ..Defects::default()
        },
        &["session_refresh_generation_stale_refused"],
        "not refused",
    )
    .await;
}

#[tokio::test]
async fn a_host_set_checked_by_name_only_fails_the_host_set_case() {
    session_change_break(
        Defects {
            host_set_by_name_only: true,
            ..Defects::default()
        },
        &["session_refresh_tool_outside_host_set_refused"],
        "a kept name with a changed definition",
    )
    .await;
}

/// The cut cases.
const CUT_CASES: &[&str] = &[
    "cut_starts_a_new_lineage",
    "old_lineage_reads_refused_after_a_cut",
    "read_below_first_ordinal_refused",
    "ordinals_never_renumber_across_a_cut",
    "setup_once_per_lineage",
    "view_ranges_start_at_first_ordinal",
    "cut_lineage_notice_descends_from",
    "head_names_the_cut",
    "notice_carries_first_ordinal",
    "revert_below_the_cut_restarts_at_the_previous_cut",
    "revert_below_a_cut_before_adoption_has_no_descent",
    "checkpoint_on_descending_setup_refused",
    "cut_setup_leaves_reads_alone",
    "cut_numbering_continues_from_the_old_lineage",
];

/// The cut case that needs a host cut before adoption, which the fake
/// gives only with its host-history hook.
const HOST_CUT_CASE: &str = "revert_below_a_cut_before_adoption_has_no_descent";

/// The `host_history` cases.
const HISTORY_CASES: &[&str] = &[
    "history_pages_back_to_the_first_message",
    "history_mid_matches_read",
    "history_no_tags_below_first_ordinal",
    "history_ordinal_kept_below_the_cut",
    "history_cursor_survives_a_cut",
    "history_never_serves_a_reverted_branch",
    "history_cursor_expired_after_a_revert_past_it",
    "history_boundary_order",
    "history_before_mid_continues_below_the_window",
    "history_version_unchanged_by_a_cut",
    "history_version_rises_at_a_revert",
    "history_from_ordinal_and_before_ordinal",
    "history_unknown_ordinal_refused",
    "history_before_lowest_ordinal_reaches_unnumbered_entries",
];

/// A fake that declares `host_history` and can seed host history; with
/// `prefix`, every session also holds that many host entries before the
/// runner's first numbered message.
fn history_subject(defect: LineageDefect, prefix: u64) -> FakeSubject {
    let mut subject = FakeSubject::new(Defects {
        lineage: defect,
        ..Defects::default()
    });
    subject.capabilities.insert(Capability::HostHistory);
    subject.host_prefix_hook = true;
    subject.host_prefix_everywhere = prefix;
    subject.holds_reads_during_setup = true;
    subject
}

/// The reference fake, with and without a host-served prefix that has no
/// ordinals, passes every cut and history case, and every other case too.
#[tokio::test]
async fn lineage_and_history_reference_fakes_pass_every_case() {
    for prefix in [0, 3] {
        let report = run(&history_subject(LineageDefect::None, prefix)).await;
        for name in CUT_CASES.iter().chain(HISTORY_CASES) {
            assert_eq!(
                report.outcome(name),
                Some(&CaseOutcome::Passed),
                "{name} with a host prefix of {prefix}\n{}",
                report.render()
            );
        }
        assert_others_passed(&report, &[]);
    }
}

/// Without the hooks a history case needs, it is inapplicable by name,
/// never passed.
#[tokio::test]
async fn history_cases_without_their_hooks_are_inapplicable() {
    let mut subject = history_subject(LineageDefect::None, 0);
    subject.host_prefix_hook = false;
    let report = run(&subject).await;
    for name in [
        "history_boundary_order",
        "history_before_mid_continues_below_the_window",
    ] {
        assert!(
            matches!(report.outcome(name), Some(CaseOutcome::Inapplicable { reason }) if reason.contains("host history")),
            "{name}: {:?}",
            report.outcome(name)
        );
    }
    assert!(matches!(
        report.outcome(COST_CASE),
        Some(CaseOutcome::Inapplicable { reason }) if reason.contains("cannot seed")
    ));
}

/// `defect` fails `case` by name. Other failures are allowed only among
/// `family`, the cases sharing the broken fixture; every case outside it
/// passes.
async fn lineage_break(subject: FakeSubject, case: &str, family: &[&str]) -> String {
    let report = run(&subject).await;
    let reason = failed(&report, case);
    for outcome in &report.cases {
        if matches!(outcome.outcome, CaseOutcome::Failed { .. }) {
            assert!(
                family.contains(&outcome.case),
                "{} also failed\n{}",
                outcome.case,
                report.render()
            );
        }
    }
    let outside: Vec<&str> = CASES
        .iter()
        .map(|spec| spec.name)
        .filter(|name| !family.contains(name))
        .collect();
    for name in outside {
        assert_passed(&report, name);
    }
    reason
}

fn family() -> Vec<&'static str> {
    CUT_CASES.iter().chain(HISTORY_CASES).copied().collect()
}

#[tokio::test]
async fn a_cut_that_keeps_the_lineage_fails_the_new_lineage_case() {
    let reason = lineage_break(
        history_subject(LineageDefect::CutKeepsLineage, 0),
        "cut_starts_a_new_lineage",
        &family(),
    )
    .await;
    assert!(reason.contains("kept lineage"), "{reason}");
}

#[tokio::test]
async fn old_lineage_reads_served_after_a_cut_fail_the_old_lineage_case() {
    let reason = lineage_break(
        history_subject(LineageDefect::OldLineageServedAfterCut, 0),
        "old_lineage_reads_refused_after_a_cut",
        &family(),
    )
    .await;
    assert!(
        reason.contains("not refused") || reason.contains("answered"),
        "{reason}"
    );
}

#[tokio::test]
async fn reads_below_first_ordinal_served_fail_the_read_bound_case() {
    let reason = lineage_break(
        history_subject(LineageDefect::BelowFirstOrdinalServed, 0),
        "read_below_first_ordinal_refused",
        &family(),
    )
    .await;
    assert!(reason.contains("below first_ordinal"), "{reason}");
}

#[tokio::test]
async fn ordinals_renumbered_at_a_cut_fail_the_renumbering_case() {
    let reason = lineage_break(
        history_subject(LineageDefect::CutRenumbersOrdinals, 0),
        "ordinals_never_renumber_across_a_cut",
        &family(),
    )
    .await;
    assert!(reason.contains("before the cut"), "{reason}");
}

#[tokio::test]
async fn setup_once_per_session_fails_the_setup_per_lineage_case() {
    let reason = lineage_break(
        history_subject(LineageDefect::SetupOncePerSession, 0),
        "setup_once_per_lineage",
        &family(),
    )
    .await;
    assert!(reason.contains("Setup calls"), "{reason}");
}

#[tokio::test]
async fn a_view_tiling_from_zero_accepted_fails_the_ranges_start_case() {
    let reason = lineage_break(
        history_subject(LineageDefect::ViewFromZeroAccepted, 0),
        "view_ranges_start_at_first_ordinal",
        &family(),
    )
    .await;
    assert!(reason.contains("ranges_start"), "{reason}");
}

#[tokio::test]
async fn a_cut_notice_without_descent_fails_the_descent_case() {
    let reason = lineage_break(
        history_subject(LineageDefect::CutNoticeWithoutDescent, 0),
        "cut_lineage_notice_descends_from",
        &family(),
    )
    .await;
    assert!(reason.contains("descend"), "{reason}");
}

#[tokio::test]
async fn an_older_cursor_skipping_a_page_fails_the_paging_case() {
    lineage_break(
        history_subject(LineageDefect::HistoryOlderSkipsPage, 0),
        "history_pages_back_to_the_first_message",
        HISTORY_CASES,
    )
    .await;
}

#[tokio::test]
async fn history_minting_its_own_ids_fails_the_mid_case() {
    let reason = lineage_break(
        history_subject(LineageDefect::HistoryMintsOwnIds, 0),
        "history_mid_matches_read",
        HISTORY_CASES,
    )
    .await;
    assert!(reason.contains("never serves under that mid"), "{reason}");
}

#[tokio::test]
async fn window_entries_served_from_the_host_fail_the_mid_case() {
    let reason = lineage_break(
        history_subject(LineageDefect::HistoryWindowFromHost, 0),
        "history_mid_matches_read",
        HISTORY_CASES,
    )
    .await;
    assert!(reason.contains("served differently"), "{reason}");
}

#[tokio::test]
async fn tags_below_first_ordinal_fail_the_no_tags_case() {
    let reason = lineage_break(
        history_subject(LineageDefect::HistoryTagsBelowFirst, 0),
        "history_no_tags_below_first_ordinal",
        HISTORY_CASES,
    )
    .await;
    assert!(reason.to_lowercase().contains("tags"), "{reason}");
}

#[tokio::test]
async fn ordinals_dropped_below_the_cut_fail_the_ordinal_case() {
    let reason = lineage_break(
        history_subject(LineageDefect::HistoryDropsOrdinalsBelowCut, 0),
        "history_ordinal_kept_below_the_cut",
        HISTORY_CASES,
    )
    .await;
    assert!(reason.contains("with ordinal None"), "{reason}");
}

#[tokio::test]
async fn a_cursor_refused_after_a_cut_fails_the_cursor_survival_case() {
    let reason = lineage_break(
        history_subject(LineageDefect::HistoryCursorLineageChanged, 0),
        "history_cursor_survives_a_cut",
        HISTORY_CASES,
    )
    .await;
    assert!(reason.contains("lineage_changed"), "{reason}");
}

#[tokio::test]
async fn history_serving_a_reverted_branch_fails_the_revert_case() {
    lineage_break(
        history_subject(LineageDefect::HistoryServesRevertedBranch, 0),
        "history_never_serves_a_reverted_branch",
        HISTORY_CASES,
    )
    .await;
}

#[tokio::test]
async fn an_expired_cursor_answered_empty_fails_the_expiry_case() {
    let reason = lineage_break(
        history_subject(LineageDefect::HistoryExpiredAsEmptyPage, 0),
        "history_cursor_expired_after_a_revert_past_it",
        HISTORY_CASES,
    )
    .await;
    assert!(reason.contains("not refused"), "{reason}");
}

#[tokio::test]
async fn a_host_prefix_after_the_numbered_entries_fails_the_boundary_case() {
    lineage_break(
        history_subject(LineageDefect::HistoryPrefixAfterNumbered, 0),
        "history_boundary_order",
        HISTORY_CASES,
    )
    .await;
}

#[tokio::test]
async fn before_mid_answered_empty_fails_the_below_window_case() {
    let reason = lineage_break(
        history_subject(LineageDefect::HistoryBeforeMidEmpty, 0),
        "history_before_mid_continues_below_the_window",
        HISTORY_CASES,
    )
    .await;
    assert!(reason.contains("answered 0 entries"), "{reason}");
}

/// The cost case passes a runner whose step work does not grow with the
/// seeded history, and fails one whose every step walks it.
#[tokio::test]
async fn per_step_cost_passes_a_flat_runner_and_fails_one_that_walks_history() {
    let mut flat = FakeSubject::new(Defects::default());
    flat.seed_hook = true;
    let report = run(&flat).await;
    assert_eq!(
        report.outcome(COST_CASE),
        Some(&CaseOutcome::Passed),
        "{}",
        report.render()
    );
    let mut walking = FakeSubject::new(Defects {
        lineage: LineageDefect::CostWalksHistory,
        ..Defects::default()
    });
    walking.seed_hook = true;
    let reason = lineage_break(walking, COST_CASE, &[COST_CASE]).await;
    assert!(reason.contains("ratio"), "{reason}");
}

#[tokio::test]
async fn a_head_naming_the_first_retained_message_fails_the_head_cut_case() {
    let reason = lineage_break(
        history_subject(LineageDefect::HeadCutNamesFirstRetained, 0),
        "head_names_the_cut",
        &family(),
    )
    .await;
    assert!(reason.contains("folds"), "{reason}");
}

#[tokio::test]
async fn notices_without_first_ordinal_fail_the_notice_case() {
    let reason = lineage_break(
        history_subject(LineageDefect::NoticeWithoutFirstOrdinal, 0),
        "notice_carries_first_ordinal",
        &family(),
    )
    .await;
    assert!(reason.contains("first_ordinal None"), "{reason}");
}

#[tokio::test]
async fn a_revert_that_keeps_the_undone_cut_fails_the_revert_case() {
    let reason = lineage_break(
        history_subject(LineageDefect::RevertKeepsTheCut, 0),
        "revert_below_the_cut_restarts_at_the_previous_cut",
        &family(),
    )
    .await;
    assert!(reason.contains("first_ordinal is"), "{reason}");
}

#[tokio::test]
async fn a_descent_invented_for_a_host_cut_fails_the_adoption_case() {
    let reason = lineage_break(
        history_subject(LineageDefect::RevertInventsDescent, 0),
        HOST_CUT_CASE,
        &family(),
    )
    .await;
    assert!(reason.contains("descends from Some"), "{reason}");
}

#[tokio::test]
async fn a_history_version_raised_at_a_cut_fails_the_cut_version_case() {
    let reason = lineage_break(
        history_subject(LineageDefect::HistoryVersionBumpsAtCut, 0),
        "history_version_unchanged_by_a_cut",
        &family(),
    )
    .await;
    assert!(reason.contains("moved history_version"), "{reason}");
}

#[tokio::test]
async fn a_history_version_kept_at_a_revert_fails_the_revert_version_case() {
    let reason = lineage_break(
        history_subject(LineageDefect::HistoryVersionStaticAtRevert, 0),
        "history_version_rises_at_a_revert",
        &family(),
    )
    .await;
    assert!(reason.contains("left history_version"), "{reason}");
}

#[tokio::test]
async fn an_ordinal_seek_off_by_one_fails_the_seek_case() {
    let reason = lineage_break(
        history_subject(LineageDefect::HistoryOrdinalSeekOffByOne, 0),
        "history_from_ordinal_and_before_ordinal",
        &family(),
    )
    .await;
    assert!(reason.contains("from_ordinal 2 served"), "{reason}");
}

#[tokio::test]
async fn an_unknown_ordinal_answered_empty_fails_the_unknown_ordinal_case() {
    let reason = lineage_break(
        history_subject(LineageDefect::HistoryUnknownOrdinalEmpty, 0),
        "history_unknown_ordinal_refused",
        &family(),
    )
    .await;
    assert!(reason.contains("not refused"), "{reason}");
}

#[tokio::test]
async fn before_the_lowest_ordinal_answered_empty_fails_the_unnumbered_case() {
    let reason = lineage_break(
        history_subject(LineageDefect::HistoryBeforeLowestOrdinalEmpty, 0),
        "history_before_lowest_ordinal_reaches_unnumbered_entries",
        &family(),
    )
    .await;
    assert!(reason.contains("before_ordinal 0 served"), "{reason}");
}

#[tokio::test]
async fn a_checkpoint_applied_on_a_descending_setup_fails_the_checkpoint_case() {
    let reason = lineage_break(
        history_subject(LineageDefect::CheckpointOnDescentApplied, 0),
        "checkpoint_on_descending_setup_refused",
        &family(),
    )
    .await;
    assert!(reason.contains("checkpoint_on_descent"), "{reason}");
}

#[tokio::test]
async fn reads_held_during_a_cut_setup_fail_the_reads_case() {
    let reason = lineage_break(
        history_subject(LineageDefect::ReadsHeldDuringCutSetup, 0),
        "cut_setup_leaves_reads_alone",
        &family(),
    )
    .await;
    assert!(reason.contains("transient"), "{reason}");
}

#[tokio::test]
async fn numbering_restarted_at_a_cut_fails_the_numbering_case() {
    let reason = lineage_break(
        history_subject(LineageDefect::CutRestartsNumbering, 0),
        "cut_numbering_continues_from_the_old_lineage",
        &family(),
    )
    .await;
    assert!(reason.contains("renumbers nothing"), "{reason}");
}

/// The display, client-message-id and live-event cases.
const DISPLAY_AND_LIVE_CASES: &[&str] = &[
    "display_row_is_the_profile_projection",
    "display_limits_cut_on_a_utf8_boundary",
    "display_without_limits_is_whole",
    "display_pages_carry_finished_messages_only",
    "client_message_id_on_display_rows_never_in_read",
    "client_message_id_resend_with_another_id_refused",
    "live_message_and_tool_events_durable",
    "live_run_events",
    "live_attach_replays_the_current_step",
    "live_seq_rises_across_lanes",
    "live_untimed_history_call_leaves_times_out",
];

/// The live `cache_ttl` cases.
const CACHE_TTL_CASES: &[&str] = &[
    "cache_ttl_entry_for_another_model_ignored",
    "cache_ttl_configured_null_never_expires",
    "cache_ttl_missed_model_change_falls_through_to_observed",
];

/// A fake declaring the display, client-message-id and live-event groups.
fn displaying(defects: Defects) -> FakeSubject {
    let mut subject = FakeSubject::new(defects);
    subject.capabilities.extend([
        Capability::HostHistory,
        Capability::HistoryDisplay,
        Capability::LiveEvents,
        Capability::ClientMessageId,
        Capability::RunCause,
    ]);
    subject
}

/// Each of `failing` failed, with a reason holding `says`, and every other
/// display, live-event and `cache_ttl` case passed.
fn fails_only(report: &SuiteReport, failing: &[&str], says: &str) {
    for case in DISPLAY_AND_LIVE_CASES.iter().chain(CACHE_TTL_CASES) {
        if failing.contains(case) {
            let reason = failed(report, case);
            assert!(reason.contains(says), "{case}: {reason}");
        } else {
            assert_eq!(
                report.outcome(case),
                Some(&CaseOutcome::Passed),
                "{case}\n{}",
                report.render()
            );
        }
    }
}

#[tokio::test]
async fn a_faithful_runner_passes_the_display_live_and_cache_ttl_cases() {
    fails_only(&run(&displaying(Defects::default())).await, &[], "");
}

#[tokio::test]
async fn a_display_row_without_model_fails_the_projection_case() {
    let report = run(&displaying(Defects {
        display_drops_model: true,
        ..Defects::default()
    }))
    .await;
    fails_only(
        &report,
        &["display_row_is_the_profile_projection"],
        "carries no model",
    );
}

#[tokio::test]
async fn a_cut_inside_a_character_fails_the_cut_case() {
    let report = run(&displaying(Defects {
        display_cut_lossy: true,
        ..Defects::default()
    }))
    .await;
    fails_only(
        &report,
        &["display_limits_cut_on_a_utf8_boundary"],
        "UTF-8 character boundary",
    );
}

#[tokio::test]
async fn a_cut_without_its_flag_fails_the_cut_case() {
    let report = run(&displaying(Defects {
        display_cut_unflagged: true,
        ..Defects::default()
    }))
    .await;
    fails_only(
        &report,
        &["display_limits_cut_on_a_utf8_boundary"],
        "arguments_truncated or truncated",
    );
}

#[tokio::test]
async fn a_default_cut_fails_the_whole_content_case() {
    let report = run(&displaying(Defects {
        display_default_limits: true,
        ..Defects::default()
    }))
    .await;
    fails_only(
        &report,
        &["display_without_limits_is_whole"],
        "absent limits mean whole content",
    );
}

#[tokio::test]
async fn a_row_for_the_step_in_flight_fails_the_finished_only_case() {
    let report = run(&displaying(Defects {
        display_in_progress_row: true,
        ..Defects::default()
    }))
    .await;
    fails_only(
        &report,
        &["display_pages_carry_finished_messages_only"],
        "finished messages only",
    );
}

#[tokio::test]
async fn a_client_message_id_in_the_default_read_fails_its_case() {
    let report = run(&displaying(Defects {
        client_id_in_read: true,
        ..Defects::default()
    }))
    .await;
    fails_only(
        &report,
        &["client_message_id_on_display_rows_never_in_read"],
        "never in a session.read entry",
    );
}

#[tokio::test]
async fn a_resend_with_another_client_message_id_accepted_fails_its_case() {
    let report = run(&displaying(Defects {
        client_id_reuse_accepted: true,
        ..Defects::default()
    }))
    .await;
    fails_only(
        &report,
        &["client_message_id_resend_with_another_id_refused"],
        "not refused",
    );
}

#[tokio::test]
async fn a_missing_tool_finished_fails_the_lifecycle_case() {
    let report = run(&displaying(Defects {
        live_tool_finished_missing: true,
        ..Defects::default()
    }))
    .await;
    // The call replayed from untimed history has no tool_finished either.
    fails_only(
        &report,
        &[
            "live_message_and_tool_events_durable",
            "live_untimed_history_call_leaves_times_out",
        ],
        "tool_finished",
    );
}

#[tokio::test]
async fn an_invented_time_for_an_untimed_call_fails_the_untimed_case() {
    let report = run(&displaying(Defects {
        live_tool_time_invented: true,
        ..Defects::default()
    }))
    .await;
    fails_only(
        &report,
        &["live_untimed_history_call_leaves_times_out"],
        "never invents one",
    );
}

#[tokio::test]
async fn a_missing_run_finished_fails_the_run_events_case() {
    let report = run(&displaying(Defects {
        live_run_finished_missing: true,
        ..Defects::default()
    }))
    .await;
    fails_only(&report, &["live_run_events"], "run_finished");
}

#[tokio::test]
async fn an_attach_without_the_step_replay_fails_the_attach_case() {
    let report = run(&displaying(Defects {
        live_attach_skips_replay: true,
        ..Defects::default()
    }))
    .await;
    // The stream-order case reads its display frames from the same replay,
    // so it has none to number and fails too.
    fails_only(
        &report,
        &[
            "live_attach_replays_the_current_step",
            "live_seq_rises_across_lanes",
        ],
        "display",
    );
}

#[tokio::test]
async fn a_seq_per_lane_fails_the_stream_order_case() {
    let report = run(&displaying(Defects {
        live_seq_per_lane: true,
        ..Defects::default()
    }))
    .await;
    fails_only(
        &report,
        &["live_seq_rises_across_lanes"],
        "rising by one per frame",
    );
}

/// A fake declaring the display groups and judging the cache lifetime by
/// the faulty `rule`.
fn caching(rule: CacheTtlDefect) -> FakeSubject {
    let mut subject = displaying(Defects::default());
    subject.cache_ttl_defect = rule;
    subject
}

#[tokio::test]
async fn an_entry_used_for_another_model_fails_its_cache_ttl_case() {
    let report = run(&caching(CacheTtlDefect::IgnoreModel)).await;
    let reason = failed(&report, "cache_ttl_entry_for_another_model_ignored");
    assert!(reason.contains("the model it names"), "{reason}");
}

#[tokio::test]
async fn a_configured_null_read_as_absent_fails_its_cache_ttl_case() {
    let report = run(&caching(CacheTtlDefect::NullAsAbsent)).await;
    fails_only(
        &report,
        &["cache_ttl_configured_null_never_expires"],
        "configured null",
    );
}

#[tokio::test]
async fn a_previous_model_lifetime_kept_fails_the_model_change_case() {
    let report = run(&caching(CacheTtlDefect::KeepPrevious)).await;
    fails_only(
        &report,
        &["cache_ttl_missed_model_change_falls_through_to_observed"],
        "never keeps a previous model's value",
    );
}

/// The `run_cause`, interrupt-grant, named-sender string and display
/// `sender` cases.
const SENDER_AND_CAUSE_CASES: &[&str] = &[
    "run_cause_queued_start_names_its_send",
    "run_cause_send_started_runs_never_omit_send_id",
    "run_cause_interrupt_names_the_new_send",
    "delivery_order_ungranted_interrupt_refused",
    "delivery_order_granted_interrupt_interrupts",
    "delivery_order_named_sender_queue_string_admitted",
    "display_row_names_the_sender_never_in_read",
];

/// A fake declaring the display and live groups, `run_cause`,
/// `delivery_order` and `interrupt`.
fn sending(defects: Defects) -> FakeSubject {
    let mut subject = displaying(defects);
    subject
        .capabilities
        .extend([Capability::DeliveryOrder, Capability::Interrupt]);
    subject
}

/// Each of `failing` failed with a reason holding `says`, and every other
/// sender and run-cause case passed.
fn only_sender_cases_fail(report: &SuiteReport, failing: &[&str], says: &str) {
    for case in SENDER_AND_CAUSE_CASES {
        if failing.contains(case) {
            let reason = failed(report, case);
            assert!(reason.contains(says), "{case}: {reason}");
        } else {
            assert_eq!(
                report.outcome(case),
                Some(&CaseOutcome::Passed),
                "{case}\n{}",
                report.render()
            );
        }
    }
}

#[tokio::test]
async fn a_faithful_runner_passes_the_sender_and_run_cause_cases() {
    only_sender_cases_fail(&run(&sending(Defects::default())).await, &[], "");
}

#[tokio::test]
async fn a_queue_id_as_the_run_cause_fails_the_queued_start_case() {
    let report = run(&sending(Defects {
        run_cause_internal_id: true,
        ..Defects::default()
    }))
    .await;
    only_sender_cases_fail(
        &report,
        &["run_cause_queued_start_names_its_send"],
        "never an id the runner made for its queue",
    );
}

#[tokio::test]
async fn an_omitted_run_cause_fails_the_never_omit_case() {
    let report = run(&sending(Defects {
        run_cause_omitted_after_admission: true,
        ..Defects::default()
    }))
    .await;
    only_sender_cases_fail(
        &report,
        &["run_cause_send_started_runs_never_omit_send_id"],
        "never omits send_id",
    );
}

#[tokio::test]
async fn naming_the_interrupted_send_fails_the_interrupt_cause_case() {
    let report = run(&sending(Defects {
        run_cause_names_interrupted: true,
        ..Defects::default()
    }))
    .await;
    only_sender_cases_fail(
        &report,
        &["run_cause_interrupt_names_the_new_send"],
        "never the send of the run it interrupted",
    );
}

#[tokio::test]
async fn an_ungranted_interrupt_admitted_fails_its_case() {
    let report = run(&sending(Defects {
        order_interrupt_grant_ignored: true,
        ..Defects::default()
    }))
    .await;
    only_sender_cases_fail(
        &report,
        &["delivery_order_ungranted_interrupt_refused"],
        "not refused",
    );
}

#[tokio::test]
async fn a_granted_interrupt_refused_fails_its_case() {
    let report = run(&sending(Defects {
        order_interrupt_owner_only: true,
        ..Defects::default()
    }))
    .await;
    only_sender_cases_fail(
        &report,
        &["delivery_order_granted_interrupt_interrupts"],
        "a granted interrupt is admitted",
    );
}

#[tokio::test]
async fn a_named_sender_queue_string_refused_fails_its_case() {
    let report = run(&sending(Defects {
        order_named_sender_strings_refused: true,
        ..Defects::default()
    }))
    .await;
    only_sender_cases_fail(
        &report,
        &["delivery_order_named_sender_queue_string_admitted"],
        "stands for [idle_start]",
    );
}

#[tokio::test]
async fn a_sender_in_the_default_read_fails_the_sender_case() {
    let report = run(&sending(Defects {
        sender_in_read: true,
        ..Defects::default()
    }))
    .await;
    only_sender_cases_fail(
        &report,
        &["display_row_names_the_sender_never_in_read"],
        "never in a session.read entry",
    );
}

#[tokio::test]
async fn without_interrupt_the_interrupt_cases_cannot_be_asked() {
    let mut subject = sending(Defects::default());
    subject.capabilities.remove(&Capability::Interrupt);
    let report = run(&subject).await;
    for case in INTERRUPT_CASES {
        assert!(
            matches!(
                report.outcome(case),
                Some(CaseOutcome::Inapplicable { reason }) if reason.contains("interrupt")
            ),
            "{case}\n{}",
            report.render()
        );
    }
}

/// The provider-set rewrite-ride wait cases.
const RIDE_WAIT_CASES: &[&str] = &[
    "ride_wait_provider_wait_honoured",
    "ride_wait_slow_answer_reported",
    "ride_wait_fast_answer_not_reported",
    "ride_wait_above_cap_fails_setup",
];

/// Each of `failing` failed with a reason holding `says`, and every other
/// ride-wait case passed.
fn only_ride_cases_fail(report: &SuiteReport, failing: &[&str], says: &str) {
    for case in RIDE_WAIT_CASES {
        if failing.contains(case) {
            let reason = failed(report, case);
            assert!(reason.contains(says), "{case}: {reason}");
        } else {
            assert_eq!(
                report.outcome(case),
                Some(&CaseOutcome::Passed),
                "{case}\n{}",
                report.render()
            );
        }
    }
}

#[tokio::test]
async fn a_faithful_runner_passes_the_ride_wait_cases() {
    only_ride_cases_fail(&run(&FakeSubject::new(Defects::default())).await, &[], "");
}

#[tokio::test]
async fn a_fixed_ride_deadline_fails_the_provider_wait_case() {
    let report = run(&FakeSubject::new(Defects {
        ride_wait_fixed_deadline: true,
        ..Defects::default()
    }))
    .await;
    only_ride_cases_fail(
        &report,
        &["ride_wait_provider_wait_honoured"],
        "as long as Setup's ride_wait_ms says",
    );
}

#[tokio::test]
async fn an_unreported_slow_ride_fails_the_report_case() {
    let report = run(&FakeSubject::new(Defects {
        ride_wait_never_reported: true,
        ..Defects::default()
    }))
    .await;
    only_ride_cases_fail(
        &report,
        &["ride_wait_slow_answer_reported"],
        "never read compaction_wait",
    );
}

#[tokio::test]
async fn a_ride_reported_at_once_fails_the_fast_answer_case() {
    let report = run(&FakeSubject::new(Defects {
        ride_wait_reported_at_once: true,
        ..Defects::default()
    }))
    .await;
    only_ride_cases_fail(
        &report,
        &["ride_wait_fast_answer_not_reported"],
        "is never reported",
    );
}

#[tokio::test]
async fn a_ride_wait_above_the_cap_accepted_fails_the_cap_case() {
    let report = run(&FakeSubject::new(Defects {
        ride_wait_cap_ignored: true,
        ..Defects::default()
    }))
    .await;
    only_ride_cases_fail(
        &report,
        &["ride_wait_above_cap_fails_setup"],
        "failed Setup naming ride_wait_ms",
    );
}

fn attachment_subject(defect: &'static str) -> FakeSubject {
    let mut subject = FakeSubject::new(Defects {
        attachments: defect,
        ..Defects::default()
    });
    subject.capabilities.insert(Capability::Attachments);
    subject.capabilities.insert(Capability::DeliveryOrder);
    subject.enable_retention();
    subject
}

#[tokio::test]
async fn attachments_reference_fake() {
    let report = run(&attachment_subject("")).await;
    for case in CASES
        .iter()
        .filter(|case| case.name.starts_with("attachments_"))
    {
        assert_passed(&report, case.name);
    }
}

#[tokio::test]
async fn attachments_inline_identities_optional() {
    for variant in [
        "attachments_inline_identities_omitted",
        "attachments_inline_digest_omitted",
        "attachments_inline_part_digests_omitted",
    ] {
        let report = run(&attachment_subject_with(variant)).await;
        for case in CASES
            .iter()
            .filter(|case| case.name.starts_with("attachments_"))
        {
            assert_eq!(
                report.outcome(case.name),
                Some(&CaseOutcome::Passed),
                "{variant}: {}\n{}",
                case.name,
                report.render()
            );
        }
        assert_none_failed_but(&report, &[]);
    }
}

#[tokio::test]
async fn attachments_inline_wrong_digest_fails_by_name() {
    attachment_read_fault(
        "attachments_inline_digest",
        "media rendering changed stored content digest",
    )
    .await;
}

#[tokio::test]
async fn attachments_inline_wrong_part_digests_fail_by_name() {
    attachment_read_fault(
        "attachments_inline_part_digests",
        "media rendering changed stored part digests",
    )
    .await;
}

macro_rules! attachment_control {
    ($name:ident) => {
        #[tokio::test]
        async fn $name() {
            let report = run(&attachment_subject("")).await;
            assert_passed(&report, stringify!($name));
        }
    };
}

/// One fault and the exact reason it must give, in each of the two read cases
/// (the tool-result one nests the media in a tool result), and nothing else
/// failed. This mirrors the external check the read cases and their faults
/// exist for.
async fn attachment_read_fault(defect: &'static str, reason: &str) {
    let report = run(&attachment_subject_with(defect)).await;
    eprintln!("REPORT {defect}: {}", report.render());
    for name in ["attachments_default_read", "attachments_tool_result"] {
        let got = failed(&report, name);
        println!("{name}: {got}");
        assert!(got.contains(reason), "{name} ({defect}): {got}");
    }
    assert_none_failed_but(
        &report,
        &["attachments_default_read", "attachments_tool_result"],
    );
}

/// The attachments fake carrying one read fault, with the process kill every
/// run of the suite wants.
fn attachment_subject_with(defect: &'static str) -> FakeSubject {
    let mut subject = FakeSubject::new(Defects {
        attachments: defect,
        ..Defects::default()
    });
    subject.capabilities.insert(Capability::Attachments);
    subject.capabilities.insert(Capability::DeliveryOrder);
    subject.claim_process_kill = true;
    subject.enable_retention();
    subject
}
attachment_control!(attachments_default_read);
attachment_control!(attachments_tool_result);
attachment_control!(attachments_media_unreferenced);
attachment_control!(attachments_commit_sha256);
attachment_control!(attachments_chunk_cap);

#[tokio::test]
async fn attachment_faults_fail_cases_by_name() {
    // The two read cases share one check over the same replies, so a runner
    // faulty in either of them fails both, and their own deliberate faults
    // (served markers, blocks digests, a stray prompt) redden the pair.
    const READ_CASES: [&str; 2] = ["attachments_default_read", "attachments_tool_result"];
    for spec in CASES
        .iter()
        .filter(|case| case.name.starts_with("attachments_"))
    {
        let report = run(&attachment_subject(spec.name)).await;
        failed(&report, spec.name);
        for other in CASES
            .iter()
            .filter(|case| case.name.starts_with("attachments_") && case.name != spec.name)
        {
            if READ_CASES.contains(&spec.name) && READ_CASES.contains(&other.name) {
                failed(&report, other.name);
                continue;
            }
            assert_passed(&report, other.name);
        }
    }
}

/// The two read cases must pass against a runner whose stored message is a
/// `data:` URL `file` part: it carries the media's sha256 nowhere, which the
/// contract does not require of a runner's own message schema.
#[tokio::test]
async fn attachments_read_cases_pass_when_stored_media_is_a_data_url() {
    let mut subject = FakeSubject::new(Defects {
        attachments_store_data_url: true,
        ..Defects::default()
    });
    subject.data_url_media = true;
    subject.capabilities.insert(Capability::Attachments);
    subject.capabilities.insert(Capability::DeliveryOrder);
    subject.claim_process_kill = true;
    subject.enable_retention();
    let report = run(&subject).await;
    for name in ["attachments_default_read", "attachments_tool_result"] {
        assert_passed(&report, name);
    }
    assert_none_failed_but(&report, &[]);
}

/// Every rule the read cases hold a blocks or descriptors page to has its own
/// faulty runner, and each fails both cases with that rule's reason alone.
#[tokio::test]
async fn attachment_read_rule_faults_fail_by_name() {
    let defaults: [(&'static str, &str); 2] = [
        (
            "attachments_blocks_digest",
            "blocks digest does not identify the stored message",
        ),
        (
            "attachments_blocks_part_digest",
            "part digests do not identify stored parts",
        ),
    ];
    for (defect, reason) in defaults {
        attachment_read_fault(defect, reason).await;
    }
}

#[tokio::test]
async fn attachments_are_inapplicable_without_the_group() {
    let report = run(&FakeSubject::new(Defects::default())).await;
    for spec in CASES
        .iter()
        .filter(|case| case.name.starts_with("attachments_"))
    {
        assert!(
            matches!(report.outcome(spec.name), Some(CaseOutcome::Skipped { missing }) if missing.contains(&Capability::Attachments)),
            "{}: {:?}",
            spec.name,
            report.outcome(spec.name)
        );
    }
}

fn output_refs_subject(defect: Option<&'static str>) -> FakeSubject {
    let mut subject = FakeSubject::new(Defects {
        output_refs: defect,
        ..Defects::default()
    });
    subject.output_refs_group = true;
    subject.capabilities.extend([
        Capability::Tags,
        Capability::HostHistory,
        Capability::HistoryDisplay,
    ]);
    subject
}

#[tokio::test]
async fn output_refs_faithful_runner_passes_all_cases() {
    let report = run(&output_refs_subject(None)).await;
    for name in cases_of(&["output_refs_"])
        .into_iter()
        .filter(|name| *name != "output_refs_unsupported_plan")
    {
        assert_eq!(
            report.outcome(name),
            Some(&CaseOutcome::Passed),
            "{name}\n{}",
            report.render()
        );
    }
    assert_none_failed_but(&report, &[]);
}

#[tokio::test]
async fn output_refs_undeclared_group_is_inapplicable() {
    let mut subject = output_refs_subject(None);
    subject.output_refs_group = false;
    subject
        .capabilities
        .extend([Capability::Adopt, Capability::SessionChange]);
    let report = run(&subject).await;
    for name in cases_of(&["output_refs_"])
        .into_iter()
        .filter(|name| *name != "output_refs_unsupported_plan")
    {
        assert!(
            matches!(report.outcome(name),Some(CaseOutcome::Inapplicable { reason }) if reason == "the runner does not declare output_refs"),
            "{name}"
        );
    }
    assert_eq!(
        report.outcome("output_refs_unsupported_plan"),
        Some(&CaseOutcome::Passed),
        "{}",
        report.render()
    );
}

macro_rules! output_refs_fault {
    ($test:ident, $case:literal) => {
        #[tokio::test]
        async fn $test() {
            let report = run(&output_refs_subject(Some($case))).await;
            println!("{}: {}", $case, failed(&report, $case));
            assert_none_failed_but(&report, &[$case]);
        }
    };
}
output_refs_fault!(output_refs_fault_scanning, "output_refs_scanning");
output_refs_fault!(
    output_refs_fault_shared_samples,
    "output_refs_shared_samples"
);
output_refs_fault!(
    output_refs_fault_writing_request,
    "output_refs_writing_request"
);
output_refs_fault!(
    output_refs_fault_target_reasons,
    "output_refs_target_reasons"
);
output_refs_fault!(
    output_refs_fault_held_placeholder,
    "output_refs_held_placeholder"
);
output_refs_fault!(output_refs_fault_snapshot_size, "output_refs_snapshot_size");
output_refs_fault!(
    output_refs_fault_snapshot_budget,
    "output_refs_snapshot_budget"
);
output_refs_fault!(
    output_refs_fault_descriptor_digest,
    "output_refs_descriptor_digest"
);
output_refs_fault!(output_refs_fault_rewind, "output_refs_rewind");
output_refs_fault!(output_refs_fault_compaction, "output_refs_compaction");
output_refs_fault!(
    output_refs_fault_session_isolation,
    "output_refs_session_isolation"
);
output_refs_fault!(output_refs_fault_slots, "output_refs_slots");
output_refs_fault!(
    output_refs_fault_reference_limit,
    "output_refs_reference_limit"
);
output_refs_fault!(output_refs_fault_read_limits, "output_refs_read_limits");

#[tokio::test]
async fn delivery_order_result_before_staging_fault_is_detected() {
    only_these_fail(
        Capability::DeliveryOrder,
        ORDER_PREFIXES,
        Defects {
            order_before_staging: true,
            ..Defects::default()
        },
        &[(
            "delivery_order_result_before_staging",
            "already-sent result",
        )],
    )
    .await;
}

#[tokio::test]
async fn output_refs_unsupported_plan_acceptance_is_detected() {
    let mut subject = output_refs_subject(Some("output_refs_unsupported_plan"));
    subject.output_refs_group = false;
    subject
        .capabilities
        .extend([Capability::Adopt, Capability::SessionChange]);
    let report = run(&subject).await;
    println!(
        "output_refs_unsupported_plan: {}",
        failed(&report, "output_refs_unsupported_plan")
    );
    assert_none_failed_but(&report, &["output_refs_unsupported_plan"]);
}

#[tokio::test]
async fn output_refs_unsupported_plan_write_before_refusal_is_detected() {
    let mut subject = output_refs_subject(Some("output_refs_unsupported_plan_written"));
    subject.output_refs_group = false;
    let report = run(&subject).await;
    println!(
        "output_refs_unsupported_plan: {}",
        failed(&report, "output_refs_unsupported_plan")
    );
    assert_none_failed_but(&report, &["output_refs_unsupported_plan"]);
}

fn system_text_default_subject(defects: Defects) -> FakeSubject {
    let mut subject = FakeSubject::new(defects);
    subject.capabilities.insert(Capability::SystemTextDefault);
    subject
}

#[tokio::test]
async fn system_text_default_forwarded_unchanged() {
    let report = run(&system_text_default_subject(Defects::default())).await;
    assert_eq!(
        report.outcome("system_text_default_forwarded_unchanged"),
        Some(&CaseOutcome::Passed),
        "{}",
        report.render()
    );
    assert_none_failed_but(&report, &[]);
}

#[tokio::test]
async fn system_text_default_preset_required() {
    let report = run(&system_text_default_subject(Defects::default())).await;
    assert_eq!(
        report.outcome("system_text_default_preset_required"),
        Some(&CaseOutcome::Passed),
        "{}",
        report.render()
    );
    assert_none_failed_but(&report, &[]);
}

#[tokio::test]
async fn system_text_default_invented_preset_fails_by_name() {
    let report = run(&system_text_default_subject(Defects {
        system_text_default_invents_preset: true,
        ..Defects::default()
    }))
    .await;
    let name = "system_text_default_forwarded_unchanged";
    assert!(failed(&report, name).contains("byte-equal"));
    assert_none_failed_but(&report, &[name]);
}

#[tokio::test]
async fn system_text_default_forward_without_capability_fails_by_name() {
    let report = run(&system_text_default_subject(Defects {
        system_text_default_ignores_catalog: true,
        ..Defects::default()
    }))
    .await;
    let name = "system_text_default_preset_required";
    assert!(failed(&report, name).contains("not refused"));
    assert_none_failed_but(&report, &[name]);
}

#[tokio::test]
async fn system_text_default_undeclared_is_not_applicable() {
    let report = run(&FakeSubject::new(Defects::default())).await;
    for name in [
        "system_text_default_forwarded_unchanged",
        "system_text_default_preset_required",
    ] {
        assert!(
            matches!(report.outcome(name), Some(CaseOutcome::Inapplicable { reason }) if reason == "runner does not declare system_text_default")
        );
    }
}
