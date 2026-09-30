//! Tests of the runner itself, against the reference fake in `fake/`.
//! Conformance of a real provider is run by that provider, never here.

mod fake;

use cortexkit_role_tool_provider_conformance::{
    harness::KillMechanism, run_suite, Capability, CaseOutcome, SetupError, SuiteReport,
    SuiteVerdict, CASES,
};
use fake::{Defects, FakeSubject};

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
        "{}",
        report.render()
    );
}

#[tokio::test]
async fn a_faithful_provider_passes_every_case_but_its_simulated_kills_fail_the_run() {
    let report = run(&FakeSubject::new(Defects::default())).await;
    assert_eq!(report.cases.len(), CASES.len());
    for spec in CASES {
        assert_passed(&report, spec.name);
    }
    assert_eq!(report.kills.len(), 2);
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
async fn a_second_terminal_frame_on_cancel_fails_the_terminal_frame_case() {
    let report = run(&FakeSubject::new(Defects {
        double_terminal_on_cancel: true,
        ..Defects::default()
    }))
    .await;
    let reason = failed(&report, "terminal_frame_on_cancel");
    assert!(reason.contains("2 terminal frames"), "{reason}");
    assert_passed(&report, "terminal_frame_on_success");
}

#[tokio::test]
async fn refusing_a_key_never_held_fails_the_unknown_call_case() {
    let report = run(&FakeSubject::new(Defects {
        refuse_unknown_call: true,
        ..Defects::default()
    }))
    .await;
    let reason = failed(&report, "withdraw_unknown_call");
    assert!(reason.contains("not unknown_call"), "{reason}");
}

#[tokio::test]
async fn running_a_held_call_after_restart_fails_both_crash_cases() {
    let report = run(&FakeSubject::new(Defects {
        run_held_calls_on_restart: true,
        ..Defects::default()
    }))
    .await;
    for case in [
        "crash_after_prepared_not_started",
        "crash_after_authorized_not_started",
    ] {
        let reason = failed(&report, case);
        assert!(reason.contains("ran the call"), "{case}: {reason}");
    }
}

#[tokio::test]
async fn a_root_level_union_fails_the_flatness_case() {
    let report = run(&FakeSubject::new(Defects {
        root_union_schema: true,
        ..Defects::default()
    }))
    .await;
    let reason = failed(&report, "catalog_schemas_flat");
    assert!(reason.contains("anyOf"), "{reason}");
}

#[tokio::test]
async fn a_case_whose_capability_is_undeclared_is_skipped_never_passed() {
    let mut subject = FakeSubject::new(Defects::default());
    subject.capabilities.remove(&Capability::ApprovalExecution);
    subject.capabilities.remove(&Capability::DisableTool);
    let report = run(&subject).await;
    for case in [
        "catalog_disabled_tool_absent",
        "call_disabled_tool_refused_by_name",
        "late_results_cursor_round_trip",
        "late_results_ack",
        "crash_after_prepared_not_started",
        "crash_after_authorized_not_started",
    ] {
        assert!(
            matches!(report.outcome(case), Some(CaseOutcome::Skipped { missing }) if !missing.is_empty()),
            "{case}: {:?}",
            report.outcome(case)
        );
    }
    assert!(report.kills.is_empty());
    assert_eq!(
        report.verdict,
        SuiteVerdict::ConformingForDeclaredCapabilities {
            skipped: vec![Capability::DisableTool, Capability::ApprovalExecution],
        },
        "{}",
        report.render()
    );
}

#[tokio::test]
async fn forgetting_acks_fails_the_ack_case() {
    let report = run(&FakeSubject::new(Defects {
        forget_acks: true,
        ..Defects::default()
    }))
    .await;
    let reason = failed(&report, "late_results_ack");
    assert!(reason.contains("still served"), "{reason}");
    assert_passed(&report, "late_results_cursor_round_trip");
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
