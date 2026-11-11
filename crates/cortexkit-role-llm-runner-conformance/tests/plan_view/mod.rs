use super::*;

const CASE_NAMES: &[&str] = &["plan_view_reply", "plan_view_items", "plan_view_no_plan"];

fn subject(defect: Option<(&'static str, &'static str)>) -> FakeSubject {
    FakeSubject::new(Defects {
        plan_view: defect,
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
async fn plan_view_control_passes() {
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
async fn plan_view_without_group_is_inapplicable() {
    let mut subject = FakeSubject::new(Defects::default());
    subject.capabilities.remove(&Capability::PlanView);
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
async fn plan_view_faults_fail_named_cases() {
    for (name, fault, expected) in [
        ("plan_view_reply", "not_planned", "as_planned"),
        ("plan_view_items", "empty_range", "system.items"),
        ("plan_view_items", "split_utf8", "system.items"),
        ("plan_view_items", "outside_text", "system.items"),
        ("plan_view_items", "overlap", "system.items"),
        ("plan_view_items", "unordered", "system.items"),
        ("plan_view_no_plan", "placeholder", "not refused"),
    ] {
        let report = exercise(&subject(Some((name, fault)))).await;
        let reason = failed(&report, name);
        eprintln!("FAIL {name} ({fault}): {reason}");
        assert!(reason.contains(expected), "{name}: {reason}");
        assert_none_failed_but(&report, &[name]);
    }
}

#[tokio::test]
async fn plan_view_no_plan_skips_only_plan_required_admission() {
    let report = exercise(&subject(Some(("plan_view_no_plan", "require_plan")))).await;
    assert!(
        matches!(report.outcome("plan_view_no_plan"), Some(CaseOutcome::Inapplicable { reason }) if reason == "runner requires an admission plan")
    );
    assert_none_failed_but(&report, &[]);
    let report = exercise(&subject(Some(("plan_view_no_plan", "refuse_other")))).await;
    assert!(failed(&report, "plan_view_no_plan").contains("other"));
    assert_none_failed_but(&report, &["plan_view_no_plan"]);
}
