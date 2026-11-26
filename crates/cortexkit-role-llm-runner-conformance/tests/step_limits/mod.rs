use super::*;

const CASES: [&str; 4] = [
    "step_liveness_shape",
    "cut_call_not_run",
    "three_cut_steps_end_run",
    "runner_enforced_cap",
];

fn subject(defects: Defects) -> FakeSubject {
    let mut subject = declaring(Capability::StepLiveness, defects);
    subject.capabilities.extend([
        Capability::RunStatus,
        Capability::RunToolCalls,
        Capability::MaxOutputTokens,
        Capability::FinishReason,
    ]);
    subject.advertise_status = true;
    subject.step_limit_fixtures = true;
    subject
}

#[tokio::test]
async fn step_limits_reference_passes_all_four() {
    let report = run(&subject(Defects::default())).await;
    for name in CASES {
        assert_eq!(report.outcome(name), Some(&CaseOutcome::Passed), "{name}");
    }
    assert_none_failed_but(&report, &[]);
}

async fn rejected(defects: Defects, name: &str, reason: &str) {
    let report = run(&subject(defects)).await;
    let diagnostic = failed(&report, name);
    assert!(diagnostic.contains(reason), "{name}: {diagnostic}");
    for other in CASES.into_iter().filter(|other| *other != name) {
        assert_eq!(report.outcome(other), Some(&CaseOutcome::Passed), "{other}");
    }
    assert_none_failed_but(&report, &[name]);
}

#[tokio::test]
async fn step_liveness_shape_rejects_missing_deadline_by_name() {
    rejected(
        Defects {
            step_liveness_missing_deadline: true,
            ..Defects::default()
        },
        "step_liveness_shape",
        "awaiting_response with both deadlines",
    )
    .await;
}

#[tokio::test]
async fn cut_call_not_run_rejects_incomplete_dispatch_by_name() {
    rejected(
        Defects {
            cut_call_dispatched_with_complete_sibling: true,
            ..Defects::default()
        },
        "cut_call_not_run",
        "not-executed result without dispatch",
    )
    .await;
}

#[tokio::test]
async fn three_cut_steps_end_run_rejects_continuing_after_third_cut_by_name() {
    rejected(
        Defects {
            consecutive_cuts_ignored: true,
            ..Defects::default()
        },
        "three_cut_steps_end_run",
        "expected permanent output_limit on the third cut step",
    )
    .await;
}

#[tokio::test]
async fn runner_enforced_cap_rejects_full_uncapped_stream_by_name() {
    rejected(
        Defects {
            runner_cap_ignored: true,
            ..Defects::default()
        },
        "runner_enforced_cap",
        "at most 768 bytes",
    )
    .await;
}

#[tokio::test]
async fn step_limits_without_optional_fixtures_are_inapplicable_by_name() {
    let mut subject = subject(Defects::default());
    subject.step_limit_fixtures = false;
    let report = run(&subject).await;
    for name in [
        "cut_call_not_run",
        "three_cut_steps_end_run",
        "runner_enforced_cap",
    ] {
        assert!(
            matches!(report.outcome(name), Some(CaseOutcome::Inapplicable { .. })),
            "{name}\n{}",
            report.render()
        );
    }
    assert_eq!(
        report.outcome("step_liveness_shape"),
        Some(&CaseOutcome::Passed)
    );
    assert_none_failed_but(&report, &[]);
}
