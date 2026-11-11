use super::*;

const CASE_NAMES: &[&str] = &[
    "session_class_malformed",
    "session_class_frozen",
    "session_class_repeat_or_omit",
    "session_class_unlabelled",
];

fn subject(defect: Option<(&'static str, &'static str)>) -> FakeSubject {
    FakeSubject::new(Defects {
        session_class: defect,
        ..Defects::default()
    })
}

async fn exercise(subject: &FakeSubject) -> SuiteReport {
    let current = subject.current_case.clone();
    let options = SuiteOptions::new().with_progress(move |progress| match progress {
        SuiteProgress::CaseStarted { case, .. } => *current.lock().unwrap() = Some(*case),
        SuiteProgress::CaseFinished { .. } => *current.lock().unwrap() = None,
        _ => {}
    });
    let dir = tempfile::tempdir().unwrap();
    run_suite_with(subject, dir.path(), options).await.unwrap()
}

#[tokio::test]
async fn session_class_control_passes() {
    let report = exercise(&subject(None)).await;
    for name in CASE_NAMES {
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
async fn session_class_without_group_is_inapplicable() {
    let mut subject = FakeSubject::new(Defects::default());
    subject.capabilities.remove(&Capability::SessionClass);
    let report = exercise(&subject).await;
    for name in CASE_NAMES {
        assert!(
            matches!(report.outcome(name), Some(CaseOutcome::Inapplicable { .. })),
            "{}",
            report.render()
        );
    }
}

#[tokio::test]
async fn session_class_faults_fail_named_cases() {
    for (name, fault) in [
        ("session_class_malformed", "accept_malformed"),
        ("session_class_frozen", "accept_conflict"),
        ("session_class_repeat_or_omit", "reject_repeat"),
        ("session_class_repeat_or_omit", "reject_omitted"),
        ("session_class_unlabelled", "reject_unlabelled"),
    ] {
        let report = exercise(&subject(Some((name, fault)))).await;
        let reason = failed(&report, name);
        eprintln!("FAIL {name} ({fault}): {reason}");
        assert!(reason.contains("session_class"), "{name}: {reason}");
        assert_none_failed_but(&report, &[name]);
    }
}
