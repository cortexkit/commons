use super::*;

const FINISH_CASES: [&str; 4] = [
    "finish_reason_last_model_step",
    "finish_reason_output_cap",
    "finish_reason_no_model_step",
    "finish_reason_provider_limit",
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
    rejected_all(defects, &[expected]).await;
}

async fn rejected_all(defects: Defects, expected: &[&str]) {
    let report = run(&subject(defects)).await;
    for name in expected {
        assert!(failed(&report, name).contains("finish"));
    }
    for name in FINISH_CASES {
        if !expected.contains(&name) {
            assert_passed(&report, name);
        }
    }
    assert_none_failed_but(&report, expected);
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
async fn finish_reason_stop_at_any_limit_fails_both_length_cases_by_name() {
    rejected_all(
        Defects {
            finish_length_as_stop: true,
            ..Defects::default()
        },
        &[FINISH_CASES[1], FINISH_CASES[3]],
    )
    .await;
}

#[tokio::test]
async fn finish_reason_length_only_from_caller_cap_fails_provider_limit_by_name() {
    rejected(
        Defects {
            finish_length_only_with_cap: true,
            ..Defects::default()
        },
        FINISH_CASES[3],
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
    for name in [FINISH_CASES[0], FINISH_CASES[1], FINISH_CASES[3]] {
        assert!(
            matches!(report.outcome(name), Some(CaseOutcome::Inapplicable { reason }) if reason == "adapter cannot script raw model finish reasons")
        );
    }
    assert_passed(&report, FINISH_CASES[2]);
    assert_none_failed_but(&report, &[]);
}
