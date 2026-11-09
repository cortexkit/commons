use super::*;

fn subject(defect: Option<&'static str>, enabled: bool) -> FakeSubject {
    let mut subject = declaring(
        Capability::DeliveryOrder,
        Defects {
            message_marks: defect,
            ..Defects::default()
        },
    );
    subject.capabilities.extend([
        Capability::HostHistory,
        Capability::HistoryDisplay,
        Capability::ReadDisplayFacts,
    ]);
    if enabled {
        subject.capabilities.insert(Capability::MessageMarks);
    }
    subject
}

#[tokio::test]
async fn notice_with_speaks_for_faithful_runner_passes() {
    let mut subject = subject(None, true);
    subject.capabilities.insert(Capability::NoticeWithSpeaksFor);
    let report = run(&subject).await;
    for name in [
        "notice_with_speaks_for_served",
        "notice_with_speaks_for_retry",
    ] {
        assert_eq!(
            report.outcome(name),
            Some(&CaseOutcome::Passed),
            "{}",
            report.render()
        );
    }
    assert!(
        matches!(
            report.outcome("message_marks_both_refused"),
            Some(CaseOutcome::Inapplicable { .. })
        ),
        "{}",
        report.render()
    );
    assert_none_failed_but(&report, &[]);
}

#[tokio::test]
async fn notice_with_speaks_for_undeclared_refuses_combination_and_name() {
    let report = run(&subject(None, true)).await;
    for name in [
        "message_marks_both_refused",
        "notice_with_speaks_for_name_undeclared",
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
async fn notice_with_speaks_for_rejects_incorrect_implementations_by_check_name() {
    for (defect, name, reason, declared) in [
        (
            "named_both",
            "notice_with_speaks_for_served",
            "was refused",
            true,
        ),
        (
            "name_drop",
            "notice_with_speaks_for_served",
            "incorrect speaks_for",
            true,
        ),
        (
            "retry_name",
            "notice_with_speaks_for_retry",
            "not refused",
            true,
        ),
        (
            "name_undeclared",
            "notice_with_speaks_for_name_undeclared",
            "not refused",
            false,
        ),
    ] {
        let mut subject = subject(Some(defect), true);
        if declared {
            subject.capabilities.insert(Capability::NoticeWithSpeaksFor);
        }
        let report = run(&subject).await;
        let failure = failed(&report, name);
        assert!(failure.contains(reason), "{name}: {failure}");
        assert_none_failed_but(&report, &[name]);
    }
}

#[tokio::test]
async fn message_marks_faithful_runner_passes() {
    let report = run(&subject(None, true)).await;
    for name in cases_of(&["message_marks_"]) {
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
async fn message_marks_undeclared_is_inapplicable() {
    let report = run(&subject(None, false)).await;
    for name in cases_of(&["message_marks_"]) {
        assert!(
            matches!(report.outcome(name), Some(CaseOutcome::Inapplicable { .. })),
            "{}",
            report.render()
        );
    }
}

#[tokio::test]
async fn message_marks_without_carrier_hook_skips_only_owner_case() {
    let mut subject = subject(None, true);
    subject.defects.scoped_start_hook_unavailable = true;
    let report = run(&subject).await;
    assert_eq!(
        report.outcome("message_marks_served"),
        Some(&CaseOutcome::Passed),
        "{}",
        report.render()
    );
    assert_eq!(
        report.outcome("message_marks_both_refused"),
        Some(&CaseOutcome::Passed),
        "{}",
        report.render()
    );
    assert!(
        matches!(
            report.outcome("message_marks_speaks_for_owner_only"),
            Some(CaseOutcome::Inapplicable { .. })
        ),
        "{}",
        report.render()
    );
}

async fn rejects(defect: &'static str, name: &str, reason: &str) {
    let report = run(&subject(Some(defect), true)).await;
    let failure = failed(&report, name);
    assert!(failure.contains(reason), "{name}: {failure}");
    println!("FAIL {name}: {failure}");
    for other in cases_of(&["message_marks_"]) {
        if other != name {
            assert_eq!(
                report.outcome(other),
                Some(&CaseOutcome::Passed),
                "{}",
                report.render()
            );
        }
    }
    assert_none_failed_but(&report, &[name]);
}

#[tokio::test]
async fn message_marks_faulty_runner_drops_marks() {
    rejects("drop", "message_marks_served", "incorrect notice").await;
}

#[tokio::test]
async fn message_marks_faulty_runner_accepts_both() {
    rejects("both", "message_marks_both_refused", "not refused").await;
}

#[tokio::test]
async fn message_marks_faulty_runner_accepts_non_owner() {
    rejects(
        "owner",
        "message_marks_speaks_for_owner_only",
        "not refused",
    )
    .await;
}

#[tokio::test]
async fn message_marks_faulty_runner_accepts_changed_retry_marks() {
    rejects("retry", "message_marks_retry_changed", "not refused").await;
}

#[tokio::test]
async fn message_marks_retry_checks_keep_when_declared() {
    let mut faithful = subject(None, true);
    faithful.capabilities.insert(Capability::NoticeKeep);
    let report = run(&faithful).await;
    assert_passed(&report, "message_marks_retry_changed");
    assert_none_failed_but(&report, &[]);

    let mut faulty = subject(Some("retry_keep"), true);
    faulty.capabilities.insert(Capability::NoticeKeep);
    let report = run(&faulty).await;
    let name = "message_marks_retry_changed";
    let reason = failed(&report, name);
    assert!(reason.contains("changed keep marks"), "{reason}");
    println!("FAIL {name}: {reason}");
    assert_none_failed_but(&report, &[name]);
}
