use super::*;

const CASES: &[&str] = &[
    "fork_of_parent_account",
    "fork_of_blocked_parent",
    "fork_of_unknown_retry",
    "fork_of_private_parent",
    "fork_of_excludes_requested_account",
    "fork_of_excludes_warm_credential",
    "fork_of_excludes_parent_conversation_key",
    "fork_of_excludes_candidates",
    "fork_of_non_admit_refused",
    "fork_of_admission_conflict",
    "fork_of_undeclared_refused",
];

#[tokio::test]
async fn fork_of_cases_pass_and_gate_on_capability() {
    for declared in [true, false] {
        let subject = FakeSubject::new(Defects {
            extension: (!declared).then_some("fork_of_without_group"),
            ..Defects::default()
        });
        let report = run_suite(&subject).await;
        assert_eq!(report.verdict, SuiteVerdict::Passed, "{}", report.render());
        for name in CASES {
            let applicable = declared != (*name == "fork_of_undeclared_refused");
            match report.outcome(name) {
                Some(CheckOutcome::Passed) if applicable => {}
                Some(CheckOutcome::NotApplicable { reason }) if !applicable => {
                    assert!(!reason.is_empty());
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
async fn fork_of_scoring_control_cannot_pass_vacuously() {
    let subject = FakeSubject::new(Defects {
        extension: Some("fork_of_no_better_sibling"),
        ..Defects::default()
    });
    let report = run_suite(&subject).await;
    assert_eq!(report.verdict, SuiteVerdict::Passed, "{}", report.render());
    assert!(matches!(
        report.outcome("fork_of_parent_account"),
        Some(CheckOutcome::NotApplicable { reason }) if reason.contains("does not establish a better sibling")
    ));
}

#[tokio::test]
async fn fork_of_faulty_routers_fail_by_check_name() {
    for (name, reason) in [
        ("fork_of_parent_account", "parent's exact account"),
        ("fork_of_blocked_parent", "not refused account_hold"),
        ("fork_of_unknown_retry", "hold recheck"),
        ("fork_of_private_parent", "refusals differ"),
        (
            "fork_of_excludes_requested_account",
            "not refused invalid_request",
        ),
        (
            "fork_of_excludes_warm_credential",
            "not refused invalid_request",
        ),
        (
            "fork_of_excludes_parent_conversation_key",
            "not refused invalid_request",
        ),
        ("fork_of_excludes_candidates", "not refused invalid_request"),
        ("fork_of_non_admit_refused", "not refused invalid_request"),
        (
            "fork_of_admission_conflict",
            "not refused admission_conflict",
        ),
        ("fork_of_undeclared_refused", "not refused invalid_request"),
    ] {
        let subject = FakeSubject::new(Defects {
            extension: Some(name),
            ..Defects::default()
        });
        let report = run_suite(&subject).await;
        assert_eq!(report.failing(), [name], "{}", report.render());
        let Some(CheckOutcome::Failed { reason: actual }) = report.outcome(name) else {
            panic!("faulty router did not fail {name}: {}", report.render());
        };
        assert!(actual.contains(reason), "{name}: {actual}");
        println!("rejected wrong router: {name}: {actual}");
    }
}
