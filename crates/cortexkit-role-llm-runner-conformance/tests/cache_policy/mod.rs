use super::*;

const CASE_NAMES: &[&str] = &[
    "cache_policy_plan_decode",
    "cache_policy_send_decode",
    "cache_policy_plan_refusals",
    "cache_policy_send_refusals",
    "cache_policy_continuation_refused",
    "cache_policy_plan_and_send_refused",
    "cache_policy_step_report",
];

fn subject(defect: Option<&'static str>) -> FakeSubject {
    let mut subject = FakeSubject::new(Defects {
        cache_policy: defect,
        ..Defects::default()
    });
    subject.capabilities.extend([
        Capability::CachePolicy,
        Capability::Plans,
        Capability::StepEvents,
    ]);
    subject
}

#[tokio::test]
async fn cache_policy_faithful_runner_passes() {
    let report = run(&subject(None)).await;
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
async fn cache_policy_adapter_known_zone_is_used() {
    let mut subject = subject(None);
    subject.cache_policy_zone = "Europe/Istanbul".into();
    let report = run(&subject).await;
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
async fn cache_policy_faulty_runners_fail_each_case_by_name() {
    for (mode, name) in [
        ("reject_plan", "cache_policy_plan_decode"),
        ("reject_send", "cache_policy_send_decode"),
        ("accept_plan_invalid", "cache_policy_plan_refusals"),
        ("accept_send_invalid", "cache_policy_send_refusals"),
        ("accept_continuation", "cache_policy_continuation_refused"),
        ("accept_both", "cache_policy_plan_and_send_refused"),
        ("missing_report", "cache_policy_step_report"),
    ] {
        let report = run(&subject(Some(mode))).await;
        let reason = failed(&report, name);
        eprintln!("FAIL {name} ({mode}): {reason}");
        assert_none_failed_but(&report, &[name]);
    }
}

#[tokio::test]
async fn cache_policy_undeclared_is_inapplicable_and_untouched() {
    let subject = FakeSubject::new(Defects::default());
    let current = subject.current_case.clone();
    let options = SuiteOptions::new().with_progress(move |progress| match progress {
        SuiteProgress::CaseStarted { case, .. } => *current.lock().unwrap() = Some(*case),
        SuiteProgress::CaseFinished { .. } => *current.lock().unwrap() = None,
        _ => {}
    });
    let dir = tempfile::tempdir().unwrap();
    let report = run_suite_with(&subject, dir.path(), options).await.unwrap();
    for name in CASE_NAMES {
        assert!(
            matches!(report.outcome(name), Some(CaseOutcome::Inapplicable { .. })),
            "{}",
            report.render()
        );
    }
    assert!(!subject
        .route_calls
        .lock()
        .unwrap()
        .iter()
        .any(|(case, _)| case.is_some_and(|case| case.starts_with("cache_policy_"))));
}
