use super::*;

const IDLE: &str = "keep_warm_auto_warms_idle_session";
const CAP: &str = "keep_warm_auto_stops_at_one_rewrite";
const REARM: &str = "keep_warm_auto_rearms_after_real_request";
const OFF: &str = "keep_warm_off_sends_no_warms";
const NAMES: &[&str] = &[IDLE, CAP, REARM, OFF];

fn subject(defect: Option<(&'static str, &'static str)>) -> FakeSubject {
    let mut subject = FakeSubject::new(Defects {
        keep_warm: defect,
        ..Defects::default()
    });
    subject.keep_warm_fixture = true;
    subject.capabilities.extend([
        Capability::CachePolicy,
        Capability::Plans,
        Capability::RunUsage,
    ]);
    subject
}

async fn run(subject: &FakeSubject) -> SuiteReport {
    let current = subject.current_case.clone();
    let options = SuiteOptions::new().with_progress(move |progress| match progress {
        SuiteProgress::CaseStarted { case, .. } => *current.lock().unwrap() = Some(*case),
        SuiteProgress::CaseFinished { .. } => *current.lock().unwrap() = None,
        _ => {}
    });
    let dir = tempfile::tempdir().unwrap();
    run_suite_with(subject, dir.path(), options).await.unwrap()
}

async fn rejects(name: &'static str, fault: &'static str, diagnostic: &str) {
    let report = run(&subject(Some((name, fault)))).await;
    let reason = failed(&report, name);
    assert!(reason.contains(diagnostic), "{name} ({fault}): {reason}");
    eprintln!("FAIL {name} ({fault}): {reason}");
    assert_none_failed_but(&report, &[name]);
}

#[tokio::test]
async fn keep_warm_faithful_runner_passes() {
    let report = run(&subject(None)).await;
    for name in NAMES {
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
async fn keep_warm_never_warming_is_rejected() {
    rejects(IDLE, "never", "no idle warm").await;
}

#[tokio::test]
async fn keep_warm_budget_faults_are_rejected() {
    for (fault, diagnostic) in [
        ("forever", "extra warm"),
        ("early", "stopped at cost 2000"),
        ("late", "extra warm"),
    ] {
        rejects(CAP, fault, diagnostic).await;
    }
}

#[tokio::test]
async fn keep_warm_not_rearming_is_rejected() {
    rejects(REARM, "no_rearm", "no idle warm").await;
}

#[tokio::test]
async fn keep_warm_ignoring_off_is_rejected() {
    rejects(OFF, "off", "off sent an idle").await;
}

#[tokio::test]
async fn keep_warm_changed_provider_bytes_are_rejected() {
    for fault in ["bytes", "prefix"] {
        rejects(IDLE, fault, "last real request's bytes and prefix").await;
    }
}

#[tokio::test]
async fn keep_warm_transcript_and_real_step_changes_are_rejected() {
    rejects(IDLE, "transcript", "changed the transcript").await;
    rejects(IDLE, "step", "exactly one model step").await;
}

#[tokio::test]
async fn keep_warm_uninstalled_prices_are_rejected() {
    rejects(CAP, "prices", "not fixture prices").await;
}

#[tokio::test]
async fn keep_warm_absent_fixture_is_inapplicable() {
    let mut subject = subject(None);
    subject.keep_warm_fixture = false;
    let report = run(&subject).await;
    for name in NAMES {
        assert!(
            matches!(report.outcome(name), Some(CaseOutcome::Inapplicable { .. })),
            "{}",
            report.render()
        );
    }
    assert_none_failed_but(&report, &[]);
}

#[tokio::test]
async fn keep_warm_undeclared_is_inapplicable_and_untouched() {
    let mut subject = subject(None);
    subject.capabilities.remove(&Capability::CachePolicy);
    let report = run(&subject).await;
    for name in NAMES {
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
        .any(|(case, _)| case.is_some_and(|case| case.starts_with("keep_warm_"))));
    assert_none_failed_but(&report, &[]);
}
