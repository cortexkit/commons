use super::*;

const FINISH_CASES: [&str; 3] = [
    "finish_reason_last_model_step",
    "finish_reason_output_cap",
    "finish_reason_no_model_step",
];

fn subject(defects: Defects) -> FakeSubject {
    let mut subject = declaring(Capability::FinishReason, defects);
    subject.capabilities.extend([
        Capability::MaxOutputTokens,
        Capability::RunUsage,
        Capability::InputTokenCap,
        Capability::RunStatus,
    ]);
    subject.advertise_status = true;
    subject
}

#[tokio::test]
async fn finish_reason_correct_runner_passes() {
    let report = run(&subject(Defects::default())).await;
    for name in FINISH_CASES {
        assert_passed(&report, name);
    }
    assert_none_failed_but(&report, &[]);
}

#[tokio::test]
async fn finish_reason_without_status_still_passes() {
    let mut subject = subject(Defects::default());
    subject.capabilities.remove(&Capability::RunStatus);
    subject.advertise_status = false;
    let report = run(&subject).await;
    for name in FINISH_CASES {
        assert_passed(&report, name);
    }
    assert_none_failed_but(&report, &[]);
}

async fn rejected(defects: Defects, expected: &str) {
    let report = run(&subject(defects)).await;
    assert!(failed(&report, expected).contains("finish"));
    for name in FINISH_CASES {
        if name != expected {
            assert_passed(&report, name);
        }
    }
    assert_none_failed_but(&report, &[expected]);
}

#[tokio::test]
async fn finish_reason_first_step_fails_last_model_step_by_name() {
    rejected(
        Defects {
            finish_first_step: true,
            ..Defects::default()
        },
        FINISH_CASES[0],
    )
    .await;
}

#[tokio::test]
async fn finish_reason_stop_at_cap_fails_output_cap_by_name() {
    rejected(
        Defects {
            finish_length_as_stop: true,
            ..Defects::default()
        },
        FINISH_CASES[1],
    )
    .await;
}

#[tokio::test]
async fn finish_reason_invented_stop_fails_no_model_step_by_name() {
    rejected(
        Defects {
            finish_without_model_step: true,
            ..Defects::default()
        },
        FINISH_CASES[2],
    )
    .await;
}

#[tokio::test]
async fn finish_reason_missing_status_fails_last_model_step_by_name() {
    rejected(
        Defects {
            finish_status_omitted: true,
            ..Defects::default()
        },
        FINISH_CASES[0],
    )
    .await;
}

#[tokio::test]
async fn finish_reason_undeclared_cases_are_not_applicable() {
    let mut subject = subject(Defects::default());
    subject.capabilities.remove(&Capability::FinishReason);
    let report = run(&subject).await;
    for name in FINISH_CASES {
        assert!(
            matches!(report.outcome(name), Some(CaseOutcome::Inapplicable { reason }) if reason == "finish_reason is not declared")
        );
    }
    assert_none_failed_but(&report, &[]);
}

#[tokio::test]
async fn finish_reason_default_hook_leaves_emitting_cases_not_applicable() {
    let mut subject = subject(Defects::default());
    subject.finish_reason_fixture = false;
    let report = run(&subject).await;
    for name in &FINISH_CASES[..2] {
        assert!(
            matches!(report.outcome(name), Some(CaseOutcome::Inapplicable { reason }) if reason == "adapter cannot script raw model finish reasons")
        );
    }
    assert_passed(&report, FINISH_CASES[2]);
    assert_none_failed_but(&report, &[]);
}
