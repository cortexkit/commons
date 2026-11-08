use super::*;

const CASE_NAMES: &[&str] = &[
    "host_start_invalid_params_leaves_nothing",
    "host_start_exact_repeat_replays",
    "host_start_send_id_reuse_names_plan",
    "host_start_new_id_plan_drift",
];

fn subject(behavior: Option<&'static str>) -> FakeSubject {
    FakeSubject::new(Defects {
        host_start: behavior,
        ..Defects::default()
    })
}

#[tokio::test]
async fn host_start_faithful_runner_passes() {
    let report = run(&subject(Some("faithful"))).await;
    for name in CASE_NAMES {
        assert_eq!(
            report.outcome(name),
            Some(&CaseOutcome::Passed),
            "{name}: {}",
            report.render()
        );
    }
    assert_none_failed_but(&report, &[]);
}

async fn rejects_only(behavior: &'static str, name: &str) {
    let report = run(&subject(Some(behavior))).await;
    let reason = failed(&report, name);
    eprintln!("FAIL {name} ({behavior}): {reason}");
    assert_none_failed_but(&report, &[name]);
    for other in CASE_NAMES.iter().filter(|other| **other != name) {
        assert_eq!(
            report.outcome(other),
            Some(&CaseOutcome::Passed),
            "{other}: {}",
            report.render()
        );
    }
}

#[tokio::test]
async fn host_start_rejects_refusal_that_creates_a_session() {
    rejects_only("invalid_params_leaves_nothing", CASE_NAMES[0]).await;
}

#[tokio::test]
async fn host_start_rejects_changed_repeat_reply() {
    rejects_only("exact_repeat_replays", CASE_NAMES[1]).await;
}

#[tokio::test]
async fn host_start_rejects_reused_id_with_changed_plan() {
    rejects_only("send_id_reuse_names_plan", CASE_NAMES[2]).await;
}

#[tokio::test]
async fn host_start_rejects_new_id_with_changed_plan() {
    rejects_only("new_id_plan_drift", CASE_NAMES[3]).await;
}

#[tokio::test]
async fn host_start_unadvertised_is_skipped_without_sending() {
    let subject = subject(None);
    let report = run(&subject).await;
    for name in CASE_NAMES {
        assert!(
            matches!(report.outcome(name), Some(CaseOutcome::SkippedWithReason { reason, .. })
                if reason == "role.describe.extensions does not list harness.host_start"),
            "{name}: {}",
            report.render()
        );
    }
    assert!(subject
        .route_calls
        .lock()
        .unwrap()
        .iter()
        .all(|(_, method)| method != cortexkit_role_llm_runner::ops::HARNESS_HOST_START));
}
